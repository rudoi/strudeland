/**
 * The headless host (`crates/strudeland-host`) as a `JobSource`: each job is one
 * `strudeland-host plan` process, which evaluates the song in an embedded V8 and plans the job as
 * the render page does in Chromium. Build it with cargo (or take `STRUDELAND_HOST_BIN`). Every
 * process goes through an injectable `EngineProcessRunner`, so `bun test` covers this file without
 * cargo or a binary. No Playwright import.
 *
 * Outcomes: exit 0 is the job (stdout, the exact text) and its info (stderr, the last non-empty
 * line, a JSON object; any earlier lines are the console lines `STRUDELAND_HOST_LOG=1` lets
 * through, and are ignored); exit 1 (a host error: the song failed, a planner error) or 2 (bad
 * input), a signal (a timeout), or an exit 0 without an info line, without a job object or with
 * a job whose length is not the info's `jsonLength`, is `HarnessError('render-failed')` with the
 * first 2000 characters of stderr. A binary that cannot be spawned is `HarnessError('build-missing')`.
 *
 * `planRender` runs `strudeland-host plan-render` (every chunk of a render in one process, for
 * `host:parity --batch`): exit 0 is the plan on stdout (one JSON object with a `chunks` array of
 * `{ json, plan }`); any other outcome, or a stdout that is not such a plan, is
 * `HarnessError('render-failed')` with the first 2000 characters of stderr.
 */

import { mkdirSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import type { RenderRequest } from '../../src/lib/render/contract';
import { isRecord, type EngineJob } from '../../src/lib/render/engine-job';
import type { PlannedRender } from '../../src/lib/render/job-plan';
import type { PageJobInfo } from '../../src/lib/render/page-job';
import { errorMessage } from '../../src/lib/errors';
import type { JobSource } from './job-source';
import {
  bunRunner,
  buildCrateBinary,
  exitHow,
  findCrateBinary,
  firstChars,
  spawnOrMissing,
  type EngineProcessRunner,
  type ProcessResult,
} from './rust-engine';
import { HarnessError } from './transfer';
import { sha256 } from '../sha256';

const HOST_PACKAGE = 'strudeland-host';
/** One plan may take this long before it is killed. */
export const HOST_TIMEOUT_MS = 600_000;
/** Characters of stderr kept in a failure message. */
const STDERR_CHARS = 2000;

/** `cargo build --release -p strudeland-host` in `cwd` (incremental): the binary's path and
 *  `strudeland-host <crate version> (<git sha>[-dirty])`. A missing cargo or a failed build is
 *  `HarnessError('build-missing')`. */
export function buildHost(
  runner: EngineProcessRunner,
  cwd: string,
): Promise<{ path: string; version: string }> {
  return buildCrateBinary(runner, cwd, HOST_PACKAGE);
}

/** `STRUDELAND_HOST_BIN` when set and non-empty (no build; a relative path is resolved against
 *  `cwd`, the repo root), with the version `strudeland-host (external: <abs path>)`; else the
 *  freshly built binary (`buildHost`). */
export function findHostBinary(
  env: Record<string, string | undefined>,
  runner: EngineProcessRunner,
  cwd: string = process.cwd(),
): Promise<{ path: string; version: string }> {
  return findCrateBinary(env, 'STRUDELAND_HOST_BIN', HOST_PACKAGE, runner, cwd);
}

/** The host's request JSON: `req` without `code` (passed as the song file), `samples` and
 *  `mutation` (the planner reads neither; the host rejects unknown fields). */
export function hostRequestJson(req: RenderRequest): string {
  const { code: _code, samples: _samples, mutation: _mutation, ...rest } = req;
  return JSON.stringify(rest);
}

/** The `strudeland-host` arguments that plan `req` from the song file `songPath`. */
export function hostPlanArgs(req: RenderRequest, songPath: string): string[] {
  return ['plan', '--song', songPath, '--request', hostRequestJson(req)];
}

/** The `strudeland-host` arguments that plan every chunk of a render of `songPath`. */
export function hostPlanRenderArgs(songPath: string, requestJson: string): string[] {
  return ['plan-render', '--song', songPath, '--request', requestJson];
}

const failure = (how: string, stderr: string): HarnessError =>
  new HarnessError('render-failed', `strudeland-host ${how}: ${firstChars(stderr, STDERR_CHARS)}`);

/** The job and info of a finished `strudeland-host plan` run (module doc for the outcomes). The
 *  job text is only checked to be an object of the announced length here; `job` parses it on
 *  first read (once), and a text that is not JSON throws `HarnessError('render-failed')` then. */
export function parsePlanOutput(res: ProcessResult): {
  readonly job: EngineJob;
  text: string;
  info: PageJobInfo;
} {
  if (res.code !== 0) throw failure(exitHow(res), res.stderr);
  // The info is written last; console lines (STRUDELAND_HOST_LOG=1) come before it.
  const last = res.stderr
    .split('\n')
    .filter((l) => l.trim() !== '')
    .at(-1);
  let info: unknown = null;
  if (last !== undefined) {
    try {
      info = JSON.parse(last);
    } catch {
      info = null;
    }
  }
  if (!isRecord(info)) throw failure('exited 0 without an info line on stderr', res.stderr);
  const text = res.stdout;
  if (!text.startsWith('{')) {
    throw failure('exited 0 with a job that is not an object', res.stderr);
  }
  if (info.jsonLength !== text.length) {
    throw failure(
      `exited 0 with a job of ${res.stdout.length} characters, its info announced ${String(info.jsonLength)}`,
      res.stderr,
    );
  }
  let job: EngineJob | undefined;
  return {
    get job(): EngineJob {
      if (job === undefined) {
        let parsed: unknown;
        try {
          parsed = JSON.parse(text);
        } catch (e) {
          throw failure(`exited 0 with a job that is not JSON (${errorMessage(e)})`, res.stderr);
        }
        if (!isRecord(parsed))
          throw failure('exited 0 with a job that is not an object', res.stderr);
        job = parsed as unknown as EngineJob;
      }
      return job;
    },
    text,
    info: info as unknown as PageJobInfo,
  };
}

/** The plan of a finished `strudeland-host plan-render` run (module doc for the outcomes). Only
 *  the parts `host:parity --batch` reads are checked: an object whose `chunks` are `{ json:
 *  string, plan: { fromBar: number, toBar: number } }`. */
export function parsePlanRenderOutput(res: ProcessResult): PlannedRender {
  if (res.code !== 0) throw failure(exitHow(res), res.stderr);
  let parsed: unknown;
  try {
    parsed = JSON.parse(res.stdout);
  } catch (e) {
    throw failure(`exited 0 with a plan that is not JSON (${errorMessage(e)})`, res.stderr);
  }
  if (!isRecord(parsed) || !Array.isArray(parsed.chunks)) {
    throw failure('exited 0 with a plan without chunks', res.stderr);
  }
  parsed.chunks.forEach((c: unknown, i: number) => {
    const ok =
      isRecord(c) &&
      typeof c.json === 'string' &&
      isRecord(c.plan) &&
      typeof c.plan.fromBar === 'number' &&
      typeof c.plan.toBar === 'number';
    if (!ok) throw failure(`exited 0 with a malformed chunk ${i}`, res.stderr);
  });
  return parsed as unknown as PlannedRender;
}

let tmpCounter = 0;

/** Writes `code` to `path` atomically (a temporary file in the same folder, then a rename), every
 *  time: a file left truncated by an interrupted run, or edited since, is replaced, and a host
 *  process reading the path sees either the old file or the whole new one. */
export function writeSongFile(path: string, code: string): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.${process.pid}.${tmpCounter++}.tmp`;
  writeFileSync(tmp, code);
  renameSync(tmp, path);
}

export interface HostPlannerOptions {
  /** The host binary (`findHostBinary` / `buildHost`). */
  bin: string;
  /** `strudeland-host <crate version> (<sha>)`, or the external form for an override. */
  version: string;
  /** The run's `--out` folder; song files go to `<outDir>/host/<sha256(code)>.js`. */
  outDir: string;
  runner?: EngineProcessRunner;
}

export class HostPlanner implements JobSource {
  private readonly runner: EngineProcessRunner;

  constructor(private readonly o: HostPlannerOptions) {
    this.runner = o.runner ?? bunRunner;
  }

  async job(req: RenderRequest): Promise<{ job: EngineJob; text: string; info: PageJobInfo }> {
    const song = this.songFile(req.code);
    const res = await spawnOrMissing(this.runner, this.o.bin, hostPlanArgs(req, song), {
      timeoutMs: HOST_TIMEOUT_MS,
    });
    return parsePlanOutput(res);
  }

  /** Plans every chunk of a render of the song file `songPath` (`requestJson` is a
   *  `RenderPlanRequest`) in one `plan-render` process. */
  async planRender(songPath: string, requestJson: string): Promise<PlannedRender> {
    const res = await spawnOrMissing(
      this.runner,
      this.o.bin,
      hostPlanRenderArgs(songPath, requestJson),
      { timeoutMs: HOST_TIMEOUT_MS },
    );
    return parsePlanRenderOutput(res);
  }

  /** The song file `job` plans from for `code`, written (atomically) first. */
  songFile(code: string): string {
    const song = join(this.o.outDir, 'host', `${sha256(code)}.js`);
    writeSongFile(song, code);
    return song;
  }

  version(): Promise<string> {
    return Promise.resolve(this.o.version);
  }

  /** Nothing to release: each plan is its own process. Song files stay in `<outDir>/host/`. */
  async dispose(): Promise<void> {}
}

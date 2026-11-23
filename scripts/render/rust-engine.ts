/**
 * The Rust engine binary (`crates/strudeland-engine`) as the harness drives it: build it with
 * cargo (or take `STRUDELAND_ENGINE_BIN`), run one job, and assemble the `RenderResult` the
 * superdough renderer would return for the same request. Every process goes through an
 * injectable `EngineProcessRunner`, so `bun test` covers this file without cargo or a binary.
 * No Playwright import.
 *
 * Outcomes (`exitCodeToOutcome`): exit 0 rendered, exit 1 rendered with render errors (the
 * summary's), anything else (exit 2 or another code, a signal such as the SIGABRT of a release
 * panic, a timeout, a stdout that is not the summary) a render error of kind `other` carrying
 * stderr. Only a binary that cannot be spawned, or a failed build, is a harness error.
 * `signatures` asks the binary which haps it cannot play.
 */

import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { RenderError, RenderRequest, RenderResult } from '../../src/lib/render/contract';
import {
  isRecord,
  type EngineJob,
  type UnsupportedSignature,
} from '../../src/lib/render/engine-job';
import { decodeWav, type DecodedWav } from '../../src/lib/render/wav';
import { dedupeErrors } from '../../src/render/pcm';
import type { PageJobInfo } from '../../src/render/protocol';
import { errorMessage } from '../../src/lib/errors';
import { HarnessError, type HarnessErrorCode } from './transfer';

const ENGINE_PACKAGE = 'strudeland-engine';
/** One engine render may take this long before it is killed. */
export const ENGINE_TIMEOUT_MS = 600_000;
/** One signatures run may take this long before it is killed. */
export const SIGNATURES_TIMEOUT_MS = 60_000;
/** Characters of stderr kept in a crash message. */
const STDERR_CHARS = 300;

export interface EngineSummary {
  ok: boolean;
  frames: number;
  channels: number;
  errors: RenderError[];
  renderMs: number;
}

/** `code` null: killed by `signal`. */
export interface ProcessResult {
  code: number | null;
  signal: string | null;
  stdout: string;
  stderr: string;
}

/** Runs a process to completion. A spawn failure (ENOENT) rejects. */
export interface EngineProcessRunner {
  run(
    cmd: string,
    args: string[],
    opts?: { stdin?: string; cwd?: string; timeoutMs?: number },
  ): Promise<ProcessResult>;
}

/** The runner on `Bun.spawn`: a timeout kills the process (SIGKILL), reported as its signal. */
export const bunRunner: EngineProcessRunner = {
  async run(cmd, args, opts = {}) {
    const proc = Bun.spawn([cmd, ...args], {
      cwd: opts.cwd,
      stdin: opts.stdin === undefined ? 'ignore' : new TextEncoder().encode(opts.stdin),
      stdout: 'pipe',
      stderr: 'pipe',
      timeout: opts.timeoutMs,
      killSignal: 'SIGKILL',
    });
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    await proc.exited;
    return {
      code: proc.signalCode ? null : proc.exitCode,
      signal: proc.signalCode ? String(proc.signalCode) : null,
      stdout,
      stderr,
    };
  },
};

// ---- build

/** The `executable` of the cargo `compiler-artifact` message for the bin target of `pkg`, from
 *  `cargo build --message-format=json` output (one JSON message per line). */
export function binaryArtifact(cargoStdout: string, pkg: string): string | null {
  let found: string | null = null;
  for (const line of cargoStdout.split('\n')) {
    if (!line.startsWith('{')) continue;
    let msg: any;
    try {
      msg = JSON.parse(line);
    } catch {
      continue;
    }
    if (
      msg?.reason === 'compiler-artifact' &&
      msg.target?.name === pkg &&
      Array.isArray(msg.target.kind) &&
      msg.target.kind.includes('bin') &&
      typeof msg.executable === 'string'
    ) {
      found = msg.executable;
    }
  }
  return found;
}

/** The version of crate `pkg` from `cargo metadata --no-deps --format-version 1` output. */
export function crateVersion(metadataStdout: string, pkg: string): string | null {
  try {
    const meta = JSON.parse(metadataStdout);
    const found = (meta?.packages ?? []).find((p: any) => p?.name === pkg);
    return typeof found?.version === 'string' ? found.version : null;
  } catch {
    return null;
  }
}

/** The first `n` characters of `s`, trimmed: the excerpt of a process's output in a message. */
export const firstChars = (s: string, n: number = STDERR_CHARS): string => s.trim().slice(0, n);

/** How a process that did not exit 0 ended: `killed by <signal>` or `exited <code>`. */
export const exitHow = (res: ProcessResult): string =>
  res.code === null ? `killed by ${res.signal ?? 'a signal'}` : `exited ${res.code}`;

type RunOptions = Parameters<EngineProcessRunner['run']>[2];

/** Runs `cmd`; a spawn failure (the binary, cargo or git missing) becomes
 *  `HarnessError('build-missing', 'cannot run <cmd>: …')`. */
export async function spawnOrMissing(
  runner: EngineProcessRunner,
  cmd: string,
  args: string[],
  opts?: RunOptions,
): Promise<ProcessResult> {
  try {
    return await runner.run(cmd, args, opts);
  } catch (e) {
    throw new HarnessError('build-missing', `cannot run ${cmd}: ${errorMessage(e)}`);
  }
}

/** `spawnOrMissing`, and a run that does not exit 0 is `HarnessError(failure.code,
 *  '<label> <exitHow>: <first stderrChars of stderr>')`. */
export async function runChecked(
  runner: EngineProcessRunner,
  cmd: string,
  args: string[],
  opts: RunOptions,
  failure: { code: HarnessErrorCode; label: string; stderrChars?: number },
): Promise<ProcessResult> {
  const res = await spawnOrMissing(runner, cmd, args, opts);
  if (res.code !== 0) {
    throw new HarnessError(
      failure.code,
      `${failure.label} ${exitHow(res)}: ${firstChars(res.stderr, failure.stderrChars)}`,
    );
  }
  return res;
}

/** `"<pkg> <crate version> (<git short sha>[-dirty])"`; parts it cannot learn read `unknown`. */
export async function buildVersion(
  runner: EngineProcessRunner,
  cwd: string,
  pkg: string,
): Promise<string> {
  const tryRun = async (cmd: string, args: string[]) => {
    try {
      const r = await runner.run(cmd, args, { cwd });
      return r.code === 0 ? r.stdout : null;
    } catch {
      return null;
    }
  };
  const [meta, head, status] = await Promise.all([
    tryRun('cargo', ['metadata', '--no-deps', '--format-version', '1']),
    tryRun('git', ['rev-parse', '--short', 'HEAD']),
    tryRun('git', ['status', '--porcelain']),
  ]);
  const version = (meta === null ? null : crateVersion(meta, pkg)) ?? 'unknown';
  let sha = head?.trim() || 'unknown';
  if (head !== null && status !== null && status.trim() !== '') sha += '-dirty';
  return `${pkg} ${version} (${sha})`;
}

/** `cargo build --release -p strudeland-engine --message-format=json` in `cwd` (incremental,
 *  so cheap when nothing changed); the binary's path and the version string. A missing cargo or
 *  a failed build is `HarnessError('build-missing')` with cargo's stderr. */
export async function buildEngine(
  runner: EngineProcessRunner,
  cwd: string,
): Promise<{ path: string; version: string }> {
  return buildCrateBinary(runner, cwd, ENGINE_PACKAGE);
}

/** `cargo build --release -p <pkg> --message-format=json` in `cwd`: the path of the binary named
 *  like the package, and `buildVersion(runner, cwd, pkg)`. A missing cargo or a failed build is
 *  `HarnessError('build-missing')` with cargo's stderr. Shared by `buildEngine` and the host's
 *  `buildHost`. */
export async function buildCrateBinary(
  runner: EngineProcessRunner,
  cwd: string,
  pkg: string,
): Promise<{ path: string; version: string }> {
  const args = ['build', '--release', '-p', pkg, '--message-format=json'];
  const r = await runChecked(
    runner,
    'cargo',
    args,
    { cwd },
    { code: 'build-missing', label: `cargo build of ${pkg}`, stderrChars: 2000 },
  );
  const path = binaryArtifact(r.stdout, pkg);
  if (path === null) {
    throw new HarnessError('build-missing', `cargo build printed no ${pkg} binary artifact`);
  }
  return { path, version: await buildVersion(runner, cwd, pkg) };
}

/** `env[envVar]` when set and non-empty (no build; a relative path is resolved against `cwd`,
 *  the repo root), with the version `<pkg> (external: <abs path>)`, which the repo HEAD did not
 *  necessarily build; else the freshly built binary of `pkg` (`buildCrateBinary`). */
export async function findCrateBinary(
  env: Record<string, string | undefined>,
  envVar: string,
  pkg: string,
  runner: EngineProcessRunner,
  cwd: string,
): Promise<{ path: string; version: string }> {
  const bin = env[envVar];
  if (bin !== undefined && bin !== '') {
    const path = resolve(cwd, bin);
    return { path, version: `${pkg} (external: ${path})` };
  }
  return buildCrateBinary(runner, cwd, pkg);
}

/** `findCrateBinary` for the engine and `STRUDELAND_ENGINE_BIN` (resolved against `cwd`, since
 *  the engine later runs with cwd `outDir`). */
export function findEngineBinary(
  env: Record<string, string | undefined>,
  runner: EngineProcessRunner,
  cwd: string = process.cwd(),
): Promise<{ path: string; version: string }> {
  return findCrateBinary(env, 'STRUDELAND_ENGINE_BIN', ENGINE_PACKAGE, runner, cwd);
}

// ---- run

export type EngineOutcome =
  { kind: 'ok' } | { kind: 'render-errors' } | { kind: 'crashed'; message: string };

/** The binary's one-line summary, or null when stdout is not one. */
export function parseSummary(stdout: string): EngineSummary | null {
  let x: unknown;
  try {
    x = JSON.parse(stdout.trim());
  } catch {
    return null;
  }
  if (
    !isRecord(x) ||
    typeof x.ok !== 'boolean' ||
    typeof x.frames !== 'number' ||
    typeof x.channels !== 'number' ||
    typeof x.renderMs !== 'number' ||
    !Array.isArray(x.errors) ||
    !x.errors.every(
      (e) => isRecord(e) && typeof e.kind === 'string' && typeof e.message === 'string',
    )
  ) {
    return null;
  }
  return x as unknown as EngineSummary;
}

/** How the binary's run ended (module doc). */
export function exitCodeToOutcome(res: ProcessResult): EngineOutcome {
  const crashed = (how: string): EngineOutcome => ({
    kind: 'crashed',
    message: `engine ${how}: ${firstChars(res.stderr)}`,
  });
  if (res.code !== 0 && res.code !== 1) return crashed(exitHow(res));
  const summary = parseSummary(res.stdout);
  if (summary === null) return crashed(`exited ${res.code} without a summary`);
  if (res.code === 0 && (!summary.ok || summary.errors.length > 0)) {
    return crashed('exited 0 with render errors');
  }
  if (res.code === 1 && summary.errors.length === 0) {
    return crashed('exited 1 without render errors');
  }
  return res.code === 0 ? { kind: 'ok' } : { kind: 'render-errors' };
}

let counter = 0;

/** `name` with every character outside `[A-Za-z0-9._-]` replaced by `_`. */
export const safeName = (name: string): string => name.replace(/[^A-Za-z0-9._-]/g, '_');

/** A summary for a run that produced no usable output. */
const failedSummary = (message: string): EngineSummary => ({
  ok: false,
  frames: 0,
  channels: 0,
  errors: [{ kind: 'other', message }],
  renderMs: 0,
});

/** A fresh `<outDir>/engine/<name>-<n>` path stem (the folder is created). */
function tempStem(outDir: string, name: string): string {
  const dir = join(outDir, 'engine');
  mkdirSync(dir, { recursive: true });
  return join(dir, `${safeName(name)}-${counter++}`);
}

/** A job already on disk (`writeJobFile`), to render more than once without writing it again. */
export interface JobFile {
  path: string;
  sampleRate: number;
}

/** Writes `text` (a serialized job of `sampleRate`) to `<outDir>/engine/<name>-<n>.job.json`.
 *  The caller deletes it. */
export function writeJobFile(
  outDir: string,
  name: string,
  text: string,
  sampleRate: number,
): JobFile {
  const path = `${tempStem(outDir, name)}.job.json`;
  writeFileSync(path, text);
  return { path, sampleRate };
}

/** An unmutated job as fetched from the render page, with its file once written. */
export interface CachedJobText {
  readonly job: EngineJob;
  /** The page's JSON of `job`, exactly as it arrived (`SuperdoughRenderer.job`). */
  text: string;
  /** The job on disk, written by its first render. */
  file?: JobFile;
}

/** The file of `entry`'s unmutated job (of `sampleRate`): written from the page's own text on
 *  first use (never re-serialized from `entry.job`, which is not read), then reused. `onWrite`
 *  sees each new file. */
export function cachedJobFile(
  entry: CachedJobText,
  outDir: string,
  name: string,
  sampleRate: number,
  onWrite: (f: JobFile) => void = () => {},
): JobFile {
  if (!entry.file) {
    entry.file = writeJobFile(outDir, name, entry.text, sampleRate);
    onWrite(entry.file);
  }
  return entry.file;
}

const isJobFile = (job: EngineJob | JobFile): job is JobFile =>
  typeof (job as JobFile).path === 'string';

/**
 * Runs `strudeland-engine render` on `job`: writes `<outDir>/engine/<name>-<n>.job.json` (unless
 * `job` is already a `JobFile`), renders to `<outDir>/engine/<name>-<n>.wav` (cwd `outDir`), and
 * returns the summary and the decoded WAV. The temp files it wrote are deleted unless `keep` (a
 * `JobFile` is never deleted here). A crash (or an unreadable WAV) returns a failed summary with
 * one `other` error and no channels; a binary that cannot be spawned throws
 * `HarnessError('build-missing')`.
 */
export async function runEngine(
  bin: string,
  job: EngineJob | JobFile,
  outDir: string,
  name: string,
  runner: EngineProcessRunner,
  keep: boolean = false,
): Promise<{ summary: EngineSummary; wav: DecodedWav }> {
  const stem = tempStem(outDir, name);
  const ownJob = !isJobFile(job);
  const jobPath = ownJob ? `${stem}.job.json` : job.path;
  const wavPath = `${stem}.wav`;
  const noAudio: DecodedWav = { channels: [], sampleRate: job.sampleRate, format: 'f32' };
  if (ownJob) writeFileSync(jobPath, JSON.stringify(job));
  try {
    const res = await spawnOrMissing(runner, bin, ['render', '--job', jobPath, '--out', wavPath], {
      cwd: outDir,
      timeoutMs: ENGINE_TIMEOUT_MS,
    });
    const outcome = exitCodeToOutcome(res);
    if (outcome.kind === 'crashed')
      return { summary: failedSummary(outcome.message), wav: noAudio };
    const summary = parseSummary(res.stdout)!;
    try {
      return { summary, wav: decodeWav(new Uint8Array(readFileSync(wavPath))) };
    } catch (e) {
      return { summary: failedSummary(`engine wav: ${errorMessage(e)}`), wav: noAudio };
    }
  } finally {
    if (!keep) {
      if (ownJob) rmSync(jobPath, { force: true });
      rmSync(wavPath, { force: true });
    }
  }
}

// ---- signatures

const isSignature = (x: unknown): x is UnsupportedSignature =>
  isRecord(x) &&
  typeof x.s === 'string' &&
  Array.isArray(x.keys) &&
  x.keys.every((k) => typeof k === 'string') &&
  Number.isInteger(x.firstBar);

/**
 * `strudeland-engine signatures --job -` on `jobText` (a job's JSON): the distinct sounds and
 * controls the engine cannot play, in first-hap order (`[]` when it plays everything). A binary
 * that cannot be spawned throws `HarnessError('build-missing')`; a kill, a non-zero exit or a
 * stdout that is not a signature list throws `HarnessError('render-failed')`.
 */
export async function signatures(
  bin: string,
  jobText: string,
  runner: EngineProcessRunner,
): Promise<UnsupportedSignature[]> {
  const res = await runChecked(
    runner,
    bin,
    ['signatures', '--job', '-'],
    { stdin: jobText, timeoutMs: SIGNATURES_TIMEOUT_MS },
    { code: 'render-failed', label: 'engine signatures' },
  );
  let list: unknown;
  try {
    list = JSON.parse(res.stdout);
  } catch {
    list = undefined;
  }
  if (!Array.isArray(list) || !list.every(isSignature)) {
    throw new HarnessError(
      'render-failed',
      `engine signatures printed an invalid list: ${firstChars(res.stdout)}`,
    );
  }
  return list;
}

// ---- result

export interface AssembleArgs {
  req: RenderRequest;
  info: PageJobInfo;
  summary: EngineSummary;
  wav: DecodedWav;
  timing: { evalMs: number; renderMs: number; totalMs: number };
}

/**
 * The `RenderResult` for an engine render, filled from the page's job info exactly as the
 * superdough renderer fills it from the same plan. A WAV whose length or channel count does not
 * match the plan (2 for mix, `2 * stemPairs` for stems) adds an `other` error; a failed run (no
 * channels) gets silent channels of the planned shape, so `ok` false is the only signal.
 */
export function assembleResult(a: AssembleArgs): RenderResult {
  const { req, info, summary, wav } = a;
  const frames = info.plan.frames;
  const want = req.mode === 'stems' ? 2 * info.stemPairs : 2;
  const checks: RenderError[] = [];
  let channels: Float32Array[];
  if (wav.channels.length === 0) {
    if (summary.errors.length === 0) {
      checks.push({ kind: 'other', message: 'engine produced no audio' });
    }
    channels = Array.from({ length: want }, () => new Float32Array(frames));
  } else {
    channels = wav.channels;
    if (wav.channels.length !== want) {
      checks.push({
        kind: 'other',
        message: `engine wrote ${wav.channels.length} channels, expected ${want}`,
      });
    }
    const lengths = new Set(wav.channels.map((c) => c.length));
    if (lengths.size !== 1 || !lengths.has(frames) || summary.frames !== frames) {
      checks.push({
        kind: 'other',
        message:
          `engine frames ${[...lengths].join('/')} (summary ${summary.frames}), ` +
          `expected ${frames}`,
      });
    }
    if (wav.sampleRate !== req.sampleRate) {
      checks.push({
        kind: 'other',
        message: `engine sample rate ${wav.sampleRate}, expected ${req.sampleRate}`,
      });
    }
  }
  const errors = dedupeErrors([...info.errors, ...summary.errors, ...checks]);
  return {
    channels,
    sampleRate: req.sampleRate,
    frames,
    windowStartFrame: info.plan.windowStartFrame,
    ...(info.plan.contentFrames === undefined ? {} : { contentFrames: info.plan.contentFrames }),
    ...(info.plan.overlapFrames === undefined ? {} : { overlapFrames: info.plan.overlapFrames }),
    lookbackSecUsed: info.plan.lookbackSecUsed,
    bufferStartSec: info.plan.bufferStartSec,
    cps: info.cps,
    events: info.events,
    stemPairs: info.stemPairs,
    errors,
    ok: errors.length === 0,
    timing: a.timing,
    request: req,
  };
}

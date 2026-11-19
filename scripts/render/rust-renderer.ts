/**
 * The `Renderer` on the Rust engine: a `JobSource` evaluates the song and plans each chunk's
 * engine job (the render page in Chromium, a `SuperdoughRenderer`, page kind `job`; or the
 * headless host, `HostPlanner`); the `strudeland-engine` binary renders it (`rust-engine.ts`).
 * Jobs are cached per request without its mutation, so twin and mutated renders run only the
 * binary. No Playwright import: the caller brings the job source.
 */

import { rmSync } from 'node:fs';
import type {
  RenderRequest,
  RenderResult,
  Renderer,
  ScheduleResult,
} from '../../src/lib/render/contract';
import { applyMutation, type EngineJob } from '../../src/lib/render/engine-job';
import type { PageJobInfo } from '../../src/render/protocol';
import {
  assembleResult,
  bunRunner,
  cachedJobFile,
  runEngine,
  signatures,
  type CachedJobText,
  type EngineProcessRunner,
  type JobFile,
} from './rust-engine';
import type { JobSource } from './job-source';
import { sha256 } from '../sha256';

export interface RustRendererOptions {
  /** The engine binary (`findEngineBinary` / `buildEngine`). */
  bin: string;
  /** `strudeland-engine <crate version> (<sha>)`, from `buildEngine`. */
  version: string;
  /** The run's `--out` folder; temp jobs and WAVs go to `<outDir>/engine/`. */
  outDir: string;
  /** Keep the temp job and WAV files (`--keep-wav`). */
  keepWav: boolean;
  runner?: EngineProcessRunner;
  /** Where jobs are planned: a render page (`SuperdoughRenderer`, possibly shared with a caller
   *  that also renders with superdough, as `engine:refs` does) or the headless host
   *  (`HostPlanner`). Disposed with this renderer. */
  jobs: JobSource;
}

interface CachedJob extends CachedJobText {
  info: PageJobInfo;
  /** Page time of the fetch that filled the cache entry. */
  evalMs: number;
}

/** The job cache key: everything the page uses to plan a job, without the mutation.
 *  `codeSha` is `sha256(req.code)`. */
const jobKey = (req: RenderRequest, codeSha: string): string =>
  JSON.stringify([
    codeSha,
    req.layout,
    req.fromBar,
    req.toBar,
    req.overlapBars,
    req.lookbackSec,
    req.mode,
  ]);

export class RustRenderer implements Renderer {
  readonly engine = 'rust' as const;
  private readonly jobs: JobSource;
  private readonly cache = new Map<string, Promise<CachedJob>>();
  /** Every cached job file, deleted on dispose unless `keepWav`. */
  private readonly files: JobFile[] = [];
  private readonly runner: EngineProcessRunner;

  constructor(private readonly o: RustRendererOptions) {
    this.runner = o.runner ?? bunRunner;
    this.jobs = o.jobs;
  }

  /** The engine build string (decision: the report's `engineVersion`). */
  get rustVersion(): string {
    return this.o.version;
  }

  /** The job source's version: Chromium's for the render page, the host build's for the host. */
  version(): Promise<string> {
    return this.jobs.version();
  }

  /** The full window `[0, toBar + overlapBars)` as one job: its events and structure are the
   *  schedule's, and its unsupported signatures (`strudeland-engine signatures`, on the page's
   *  own job text) decide the fixture's `unsupported` status. */
  async schedule(req: RenderRequest): Promise<ScheduleResult> {
    const full: RenderRequest = { ...req, fromBar: 0, lookbackSec: 0, mode: 'mix' };
    delete full.mutation;
    const entry = await this.cached(full, jobKey(full, sha256(full.code)));
    const { info } = entry;
    return {
      events: info.events,
      cps: info.cps,
      structure: info.structure,
      errors: info.errors,
      unsupported: await signatures(this.o.bin, entry.text, this.runner),
    };
  }

  /** The unmutated job for `req` and its page info, cached (`jobKey`). */
  async jobFor(req: RenderRequest): Promise<{ job: EngineJob; info: PageJobInfo }> {
    const { job, info } = await this.cached(req, jobKey(req, sha256(req.code)));
    return { job, info };
  }

  async render(req: RenderRequest): Promise<RenderResult> {
    const t0 = performance.now();
    const codeSha = sha256(req.code);
    const key = jobKey(req, codeSha);
    const fetched = this.cache.has(key);
    const entry = await this.cached(req, key);
    const { info, evalMs } = entry;
    const name = `${codeSha.slice(0, 8)}-${req.fromBar}-${req.toBar}-${req.mode}`;
    // A mutated job is serialized per render; the unmutated one is the page's own JSON text,
    // written once per key.
    const run: EngineJob | JobFile = req.mutation
      ? applyMutation(entry.job, info.events, req.mutation)
      : cachedJobFile(entry, this.o.outDir, name, req.sampleRate, (f) => this.files.push(f));
    const { summary, wav } = await runEngine(
      this.o.bin,
      run,
      this.o.outDir,
      name,
      this.runner,
      this.o.keepWav,
    );
    return assembleResult({
      req,
      info,
      summary,
      wav,
      timing: {
        evalMs: fetched ? 0 : evalMs,
        renderMs: summary.renderMs,
        totalMs: performance.now() - t0,
      },
    });
  }

  async dispose(): Promise<void> {
    this.cache.clear();
    if (!this.o.keepWav) for (const f of this.files) rmSync(f.path, { force: true });
    this.files.length = 0;
    await this.jobs.dispose();
  }

  private cached(req: RenderRequest, key: string): Promise<CachedJob> {
    let p = this.cache.get(key);
    if (!p) {
      const t0 = performance.now();
      // `job` is read through, not copied: a source may parse it on first read.
      p = this.jobs.job(req).then((planned) => ({
        get job() {
          return planned.job;
        },
        text: planned.text,
        info: planned.info,
        evalMs: performance.now() - t0,
      }));
      // a failed fetch is not cached: the next request tries again
      p.catch(() => this.cache.delete(key));
      this.cache.set(key, p);
    }
    return p;
  }
}

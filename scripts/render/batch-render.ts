/**
 * The main window of a `render:check --planner batch` fixture, rendered by the whole Rust
 * pipeline: one `strudeland-render render` process over the fixture's exact window (loudness off,
 * a float WAV, `--jobs 1`: the harness's `--concurrency` is the parallelism), whose `--info`
 * file is cross-checked against the fixture's host-planned schedule (`batch-window.ts`). Build
 * the binary with cargo (or take `STRUDELAND_RENDER_BIN`). Every process goes through an
 * injectable `EngineProcessRunner`, so `bun test` covers this file without cargo or a binary.
 * No Playwright import.
 *
 * `PipelineRenderer` is how `render:check --planner batch` uses `BatchRenderer`: it is the
 * host-planned `RustRenderer` for every request, plus a `renderWindow` that renders the
 * fixture's main window through `BatchRenderer` (the song written to
 * `<outDir>/batch/<sha256>.js`, the outputs named `<sha8>-<from>-<to>-<layout>`).
 *
 * Outcomes: a binary that cannot be spawned is `HarnessError('build-missing')` (the run stops);
 * a non-zero exit (stderr's first 2000 characters), an unreadable info file or WAV, a failed
 * cross-check or a WAV of the wrong shape is a `RenderFailedError` (the fixture's error).
 */

import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  batchAssembled,
  batchCrossChecks,
  parseBatchInfo,
  type BatchInfo,
} from '../../src/lib/render/batch-window';
import type { AssembledWindow } from '../../src/lib/render/chunks';
import type {
  RenderRequest,
  RenderResult,
  Renderer,
  ScheduleResult,
} from '../../src/lib/render/contract';
import type { RenderEvent } from '../../src/lib/render/schedule';
import { layoutSegments, renderTiming, type RenderTiming } from '../../src/lib/render/timing';
import { decodeWav, type DecodedWav } from '../../src/lib/render/wav';
import { errorMessage } from '../../src/lib/errors';
import {
  bunRunner,
  exitHow,
  findCrateBinary,
  firstChars,
  spawnOrMissing,
  type EngineProcessRunner,
} from './rust-engine';
import { writeSongFile } from './host-planner';
import type { RustRenderer } from './rust-renderer';
import { RenderFailedError } from './transfer';
import { sha256 } from '../sha256';

const RENDER_PACKAGE = 'strudeland-render';
/** One batch render may take this long before it is killed. */
export const BATCH_TIMEOUT_MS = 1_200_000;
/** Characters of stderr kept in a failure message. */
const STDERR_CHARS = 2000;

/** `STRUDELAND_RENDER_BIN` when set and non-empty (no build), else `cargo build --release -p
 *  strudeland-render` (it needs cmake for libopus). */
export function findRenderBinary(
  env: Record<string, string | undefined>,
  runner: EngineProcessRunner,
  cwd: string = process.cwd(),
): Promise<{ path: string; version: string }> {
  return findCrateBinary(env, 'STRUDELAND_RENDER_BIN', RENDER_PACKAGE, runner, cwd);
}

/** The `strudeland-render` arguments that render `base`'s window of `songPath`. */
export function batchRenderArgs(
  songPath: string,
  base: Pick<RenderRequest, 'fromBar' | 'toBar' | 'layout'>,
  wavPath: string,
  infoPath: string,
): string[] {
  return [
    'render',
    '--song',
    songPath,
    '--from',
    String(base.fromBar),
    '--to',
    String(base.toBar),
    '--layout',
    base.layout,
    '--loudness',
    'off',
    '--format',
    'wav',
    '--bit-depth',
    '32f',
    '--jobs',
    '1',
    '--out',
    wavPath,
    '--info',
    infoPath,
  ];
}

/** What a batch render needs to know about the fixture. */
export interface BatchTarget {
  /** The stem of the output file names. */
  name: string;
  /** The song file, absolute. */
  songPath: string;
  base: RenderRequest;
  cps: number;
  events: RenderEvent[];
  timing?: RenderTiming;
}

export interface BatchRendererOptions {
  bin: string;
  /** The run's `--out` folder; files go to `<outDir>/batch/<name>.wav` and `.info.json`. */
  outDir: string;
  runner?: EngineProcessRunner;
  readFile?: (path: string) => Uint8Array;
  now?: () => number;
}

export class BatchRenderer {
  private readonly runner: EngineProcessRunner;
  private readonly readFile: (path: string) => Uint8Array;
  private readonly now: () => number;

  constructor(private readonly o: BatchRendererOptions) {
    this.runner = o.runner ?? bunRunner;
    this.readFile = o.readFile ?? ((p) => readFileSync(p));
    this.now = o.now ?? (() => performance.now());
  }

  /** The fixture's main window from the whole pipeline (module doc for the outcomes). */
  async window(t: BatchTarget): Promise<AssembledWindow> {
    const dir = join(this.o.outDir, 'batch');
    mkdirSync(dir, { recursive: true });
    const wavPath = join(dir, `${t.name}.wav`);
    const infoPath = join(dir, `${t.name}.info.json`);
    const t0 = this.now();
    const res = await spawnOrMissing(
      this.runner,
      this.o.bin,
      batchRenderArgs(t.songPath, t.base, wavPath, infoPath),
      { timeoutMs: BATCH_TIMEOUT_MS },
    );
    const ms = this.now() - t0;
    if (res.code !== 0) {
      throw new RenderFailedError([
        `batch: strudeland-render ${exitHow(res)}: ${firstChars(res.stderr, STDERR_CHARS)}`,
      ]);
    }
    let info: BatchInfo;
    let wav: DecodedWav;
    try {
      info = parseBatchInfo(new TextDecoder().decode(this.readFile(infoPath)));
    } catch (e) {
      throw new RenderFailedError([errorMessage(e)]);
    }
    try {
      wav = decodeWav(this.readFile(wavPath));
    } catch (e) {
      throw new RenderFailedError([`batch: the WAV: ${errorMessage(e)}`]);
    }
    const reasons = batchCrossChecks(info, t);
    if (reasons.length > 0) throw new RenderFailedError(reasons);
    try {
      return batchAssembled(wav, info, t.base, t.events, ms, t.timing);
    } catch (e) {
      throw new RenderFailedError([errorMessage(e)]);
    }
  }
}

/** What `PipelineRenderer` takes from the host-planned `RustRenderer`. */
export type PipelineInner = Pick<
  RustRenderer,
  'schedule' | 'render' | 'version' | 'dispose' | 'rustVersion'
>;

export interface PipelineRendererOptions {
  inner: PipelineInner;
  batch: BatchRenderer;
  /** The run's `--out` folder; song files go to `<outDir>/batch/<sha256(code)>.js`. */
  outDir: string;
  /** The `strudeland-render` build, for the run header. */
  renderVersion: string;
}

/** `--planner batch`'s renderer: the host-planned `RustRenderer` for every request, and the whole
 *  Rust pipeline (`strudeland-render render`) for a fixture's main window (`renderWindow`). */
export class PipelineRenderer implements Renderer {
  readonly engine = 'rust' as const;
  readonly renderVersion: string;

  constructor(private readonly o: PipelineRendererOptions) {
    this.renderVersion = o.renderVersion;
  }

  schedule(req: RenderRequest): Promise<ScheduleResult> {
    return this.o.inner.schedule(req);
  }

  render(req: RenderRequest): Promise<RenderResult> {
    return this.o.inner.render(req);
  }

  version(): Promise<string> {
    return this.o.inner.version();
  }

  get rustVersion(): string {
    return this.o.inner.rustVersion;
  }

  /** The window from `BatchRenderer`, cross-checked against the inner renderer's (cached)
   *  full-window schedule and the timing `render:check` derives from it. */
  async renderWindow(base: RenderRequest, events: RenderEvent[]): Promise<AssembledWindow> {
    const sched = await this.o.inner.schedule(base);
    const timing = renderTiming(
      sched.structure,
      layoutSegments(base.layout, sched.structure),
      sched.cps,
    );
    const hash = sha256(base.code);
    const songPath = join(this.o.outDir, 'batch', `${hash}.js`);
    writeSongFile(songPath, base.code);
    return this.o.batch.window({
      name: `${hash.slice(0, 8)}-${base.fromBar}-${base.toBar}-${base.layout}`,
      songPath,
      base,
      cps: sched.cps,
      events,
      timing,
    });
  }

  dispose(): Promise<void> {
    return this.o.inner.dispose();
  }
}

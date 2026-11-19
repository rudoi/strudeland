/**
 * The main window of a `render:check --planner batch` fixture, rendered by the whole Rust
 * pipeline: one `strudeland-render render` process over the fixture's exact window (loudness off,
 * a float WAV, `--jobs 1`: the harness's `--concurrency` is the parallelism), whose `--info`
 * file is cross-checked against the fixture's host-planned schedule (`batch-window.ts`). Build
 * the binary with cargo (or take `STRUDELAND_RENDER_BIN`). Every process goes through an
 * injectable `EngineProcessRunner`, so `bun test` covers this file without cargo or a binary.
 * No Playwright import.
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
import type { RenderRequest } from '../../src/lib/render/contract';
import type { RenderEvent } from '../../src/lib/render/schedule';
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
import { RenderFailedError } from './transfer';

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
  /** The fixture's name (the output file names). */
  name: string;
  /** The song file, absolute. */
  songPath: string;
  base: RenderRequest;
  cps: number;
  events: RenderEvent[];
}

export interface BatchRendererOptions {
  bin: string;
  /** The run's `--out` folder; files go to `<outDir>/batch/<fixture>.wav` and `.info.json`. */
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
      return batchAssembled(wav, info, t.base, t.events, ms);
    } catch (e) {
      throw new RenderFailedError([errorMessage(e)]);
    }
  }
}

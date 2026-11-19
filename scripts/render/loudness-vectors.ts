/**
 * `bun run render:vectors [--write]`: the vectors the render crate's Rust ports are checked
 * against (`crates/strudeland-render/tests/vectors/*.json`). `loudness.json` holds fixed signals
 * as recipes (sines and seeded noise, not samples) with `integratedLufs`
 * (`src/lib/render/loudness.ts`) of each; `tests/loudness_vectors.rs` rebuilds every signal from
 * its recipe and checks the Rust port within 1e-6 LU. `joiner.json` runs the real
 * `assembleWindow` (`src/lib/render/chunks.ts`) over synthetic chunk buffers and stores each
 * case's frame count and the FNV-1a 64-bit hash of the joined audio; `tests/joiner_vectors.rs`
 * rebuilds the same buffers and checks the Rust joiner against them. No browser: `bun test`
 * regenerates the files in memory and byte-compares them with the committed ones
 * (`tests/render-vectors.test.ts`).
 *
 * Without `--write` it prints the diff summary and exits 1 when the committed files differ; with
 * `--write` it writes the generated files and removes the ones it no longer generates. Exit 2 on
 * bad arguments.
 */

import { resolve } from 'node:path';
import { assembleWindow } from '../../src/lib/render/chunks';
import { SAMPLE_RATE, windowFrames, type RenderResult } from '../../src/lib/render/contract';
import { integratedLufs } from '../../src/lib/render/loudness';
import { mulberry32 } from 'superdough/rng.mjs';
import { runVectors, vectorsFileText } from '../vectors-files';

export { parseVectorsArgs, readVectors, type VectorsOptions } from '../vectors-files';

export const VECTORS_DIR = 'crates/strudeland-render/tests/vectors';
const GENERATOR = 'scripts/render/loudness-vectors.ts';

/** A vector file's text (`vectorsFileText`) with this generator's name. */
export const fileText = (sections: Record<string, unknown>): string =>
  vectorsFileText(GENERATOR, sections);

// ---- joiner

const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const MASK64 = (1n << 64n) - 1n;

/** FNV-1a 64 over the little-endian bytes of each channel in order, as 16 lowercase hex digits. */
export function fnv1a64(channels: Float32Array[]): string {
  let h = FNV_OFFSET;
  for (const ch of channels) {
    const bytes = new Uint8Array(ch.buffer, ch.byteOffset, ch.byteLength);
    for (const b of bytes) h = ((h ^ BigInt(b)) * FNV_PRIME) & MASK64;
  }
  return h.toString(16).padStart(16, '0');
}

/** Sample `i` of channel `c` of chunk `k`: a 16-bit numerator over 2^16, exact in f32. */
export function joinerSample(k: number, c: number, i: number): number {
  return ((k * 7919 + c * 104729 + i * 31) % 65521) / 65536;
}

interface JoinerChunk {
  fromBar: number;
  toBar: number;
  windowStartFrame: number;
  overlapBars: number;
}

interface JoinerCase {
  name: string;
  cps: number;
  window: { fromBar: number; toBar: number };
  chunks: JoinerChunk[];
}

const JOINER_CASES: JoinerCase[] = [
  {
    name: 'single-chunk',
    cps: 0.5,
    window: { fromBar: 0, toBar: 1 },
    chunks: [{ fromBar: 0, toBar: 1, windowStartFrame: 1000, overlapBars: 0 }],
  },
  {
    name: 'three-equal-chunks',
    cps: 0.5,
    window: { fromBar: 0, toBar: 3 },
    chunks: [
      { fromBar: 0, toBar: 1, windowStartFrame: 0, overlapBars: 0 },
      { fromBar: 1, toBar: 2, windowStartFrame: 22050, overlapBars: 0 },
      { fromBar: 2, toBar: 3, windowStartFrame: 44100, overlapBars: 0 },
    ],
  },
  {
    name: 'cps-7-16',
    cps: 0.4375,
    window: { fromBar: 0, toBar: 3 },
    chunks: [
      { fromBar: 0, toBar: 1, windowStartFrame: 0, overlapBars: 0 },
      { fromBar: 1, toBar: 2, windowStartFrame: 1234, overlapBars: 0 },
      { fromBar: 2, toBar: 3, windowStartFrame: 5000, overlapBars: 0 },
    ],
  },
  {
    // 97 bpm in 4/4: a bar is 109113.40... frames, so every bar boundary rounds.
    name: 'fractional-bar',
    cps: 97 / 240,
    window: { fromBar: 0, toBar: 4 },
    chunks: [
      { fromBar: 0, toBar: 1, windowStartFrame: 0, overlapBars: 0 },
      { fromBar: 1, toBar: 2, windowStartFrame: 777, overlapBars: 0 },
      { fromBar: 2, toBar: 3, windowStartFrame: 3, overlapBars: 0 },
      { fromBar: 3, toBar: 4, windowStartFrame: 60000, overlapBars: 0 },
    ],
  },
  {
    name: 'overlap-tail-dropped',
    cps: 0.5,
    window: { fromBar: 0, toBar: 2 },
    chunks: [
      { fromBar: 0, toBar: 1, windowStartFrame: 0, overlapBars: 1 },
      { fromBar: 1, toBar: 2, windowStartFrame: 4410, overlapBars: 1 },
    ],
  },
  {
    name: 'from-bar-after-zero',
    cps: 4 / 3,
    window: { fromBar: 3, toBar: 7 },
    chunks: [
      { fromBar: 3, toBar: 5, windowStartFrame: 100, overlapBars: 0 },
      { fromBar: 5, toBar: 7, windowStartFrame: 33075, overlapBars: 0 },
    ],
  },
  {
    // A bar of 25941.17... frames; chunks of 1, 7 and 3 bars, one with a 2-bar overlap.
    name: 'uneven-chunks',
    cps: 1.7,
    window: { fromBar: 1, toBar: 12 },
    chunks: [
      { fromBar: 1, toBar: 2, windowStartFrame: 11, overlapBars: 0 },
      { fromBar: 2, toBar: 9, windowStartFrame: 0, overlapBars: 2 },
      { fromBar: 9, toBar: 12, windowStartFrame: 25941, overlapBars: 0 },
    ],
  },
];

/** Chunk `k`'s render result: lookback head, content and overlap tail of synthetic samples. */
function chunkResult(k: number, chunk: JoinerChunk, cps: number): RenderResult {
  const frames =
    chunk.windowStartFrame +
    windowFrames(chunk.fromBar, chunk.toBar + chunk.overlapBars, cps, SAMPLE_RATE);
  const channels = [0, 1].map((c) => {
    const ch = new Float32Array(frames);
    for (let i = 0; i < frames; i++) ch[i] = joinerSample(k, c, i);
    return ch;
  });
  return {
    channels,
    sampleRate: SAMPLE_RATE,
    frames,
    windowStartFrame: chunk.windowStartFrame,
    lookbackSecUsed: chunk.windowStartFrame / SAMPLE_RATE,
    bufferStartSec: 0,
    cps,
    events: [],
    stemPairs: 1,
    errors: [],
    ok: true,
    timing: { evalMs: 0, renderMs: 0, totalMs: 0 },
    request: {
      code: '',
      layout: 'linear',
      fromBar: chunk.fromBar,
      toBar: chunk.toBar,
      overlapBars: chunk.overlapBars,
      lookbackSec: chunk.windowStartFrame / SAMPLE_RATE,
      mode: 'mix',
      sampleRate: SAMPLE_RATE,
    },
  };
}

function joinerCases(): object[] {
  return JOINER_CASES.map((c) => {
    const joined = assembleWindow(
      c.chunks.map((chunk, k) => chunkResult(k, chunk, c.cps)),
      [],
    );
    return { ...c, frames: joined.frames, fnv1a64: fnv1a64(joined.channels) };
  });
}

// ---- loudness

/** One additive part of a channel; `from`/`to` are frame bounds (default the whole channel). */
export type Part =
  | { kind: 'sine'; hz: number; amp: number; from?: number; to?: number }
  | { kind: 'noise'; amp: number; seed: number; from?: number; to?: number };

export interface ChannelRecipe {
  frames: number;
  parts: Part[];
}

export interface SignalRecipe {
  name: string;
  sampleRate: number;
  channels: ChannelRecipe[];
}

/** Linear amplitude of a level in dBFS. */
export const amp = (db: number): number => Math.pow(10, db / 20);

/** A channel's samples: `Math.fround` of the sum of its parts, in order. A sine part is `amp *
 *  Math.sin(((2 * Math.PI * hz) * i) / sampleRate)`; a noise part is `amp * (2 * u - 1)` with `u`
 *  the next value of its own `mulberry32(seed)`, drawn once per frame inside its bounds. A frame
 *  outside a part's `[from, to)` gets nothing from it. */
export function synthChannel(c: ChannelRecipe, sampleRate: number): Float32Array {
  const acc = new Float64Array(c.frames);
  for (const p of c.parts) {
    const from = p.from ?? 0;
    const to = Math.min(p.to ?? c.frames, c.frames);
    if (p.kind === 'sine') {
      for (let i = from; i < to; i++) {
        acc[i] = acc[i]! + p.amp * Math.sin((2 * Math.PI * p.hz * i) / sampleRate);
      }
    } else {
      const next = mulberry32(p.seed) as () => number;
      for (let i = from; i < to; i++) acc[i] = acc[i]! + p.amp * (2 * next() - 1);
    }
  }
  return Float32Array.from(acc);
}

const sine = (hz: number, db: number, from?: number, to?: number): Part => ({
  kind: 'sine',
  hz,
  amp: amp(db),
  ...(from === undefined ? {} : { from }),
  ...(to === undefined ? {} : { to }),
});
const both = (frames: number, parts: Part[]): ChannelRecipe[] => [
  { frames, parts },
  { frames, parts },
];

export const LOUDNESS_SIGNALS: SignalRecipe[] = [
  { name: 'silence-3s', sampleRate: SAMPLE_RATE, channels: both(132300, []) },
  {
    name: 'short-300ms',
    sampleRate: SAMPLE_RATE,
    channels: both(13230, [sine(1000, -20)]),
  },
  {
    name: 'sine1k-m23-stereo',
    sampleRate: SAMPLE_RATE,
    channels: both(882000, [sine(1000, -23)]),
  },
  {
    name: 'sine1k-m23-left-only',
    sampleRate: SAMPLE_RATE,
    channels: [
      { frames: 882000, parts: [sine(1000, -23)] },
      { frames: 882000, parts: [] },
    ],
  },
  {
    name: 'sine1k-m23-mono',
    sampleRate: SAMPLE_RATE,
    channels: [{ frames: 882000, parts: [sine(1000, -23)] }],
  },
  {
    name: 'sine100-m18',
    sampleRate: SAMPLE_RATE,
    channels: both(441000, [sine(100, -18)]),
  },
  {
    name: 'sine5k-m30',
    sampleRate: SAMPLE_RATE,
    channels: both(441000, [sine(5000, -30)]),
  },
  {
    name: 'noise-m20',
    sampleRate: SAMPLE_RATE,
    channels: [
      { frames: 441000, parts: [{ kind: 'noise', amp: 0.1, seed: 1 }] },
      { frames: 441000, parts: [{ kind: 'noise', amp: 0.1, seed: 2 }] },
    ],
  },
  {
    name: 'gated-burst',
    sampleRate: SAMPLE_RATE,
    channels: both(441000, [sine(1000, -10, 352800, 441000)]),
  },
  {
    name: 'relative-gate',
    sampleRate: SAMPLE_RATE,
    channels: both(1323000, [
      sine(1000, -36, 0, 441000),
      sine(1000, -23, 441000, 882000),
      sine(1000, -36, 882000, 1323000),
    ]),
  },
  {
    // The relative gate drops the quiet parts: 2 s at -45, 10 s at -23, 2 s at -45 dBFS.
    name: 'relative-gate-drops',
    sampleRate: SAMPLE_RATE,
    channels: both(617400, [
      sine(1000, -45, 0, 88200),
      sine(1000, -23, 88200, 529200),
      sine(1000, -45, 529200, 617400),
    ]),
  },
  {
    name: 'below-gate',
    sampleRate: SAMPLE_RATE,
    channels: both(220500, [sine(1000, -75)]),
  },
  {
    name: 'two-sines-stereo',
    sampleRate: SAMPLE_RATE,
    channels: [
      { frames: 441000, parts: [sine(220, -20)] },
      { frames: 441000, parts: [sine(3000, -26)] },
    ],
  },
];

/** Each signal with its `integratedLufs` (`null` for -Infinity). */
function loudnessSignals(): object[] {
  return LOUDNESS_SIGNALS.map((s) => {
    const lufs = integratedLufs(
      s.channels.map((c) => synthChannel(c, s.sampleRate)),
      s.sampleRate,
    );
    return { ...s, lufs: lufs === -Infinity ? null : lufs };
  });
}

// ---- files

export function generateVectors(): Map<string, string> {
  const files = new Map<string, string>();
  files.set('loudness.json', fileText({ signals: loudnessSignals() }));
  files.set('joiner.json', fileText({ cases: joinerCases() }));
  return files;
}

/** `bun run render:vectors [--write]`; `generate` is injectable so tests reuse one generation. */
export function main(
  argv: string[],
  root: string,
  log: (line: string) => void = console.log,
  generate: () => Map<string, string> = generateVectors,
): number {
  return runVectors({ dir: VECTORS_DIR, command: 'render:vectors' }, argv, root, log, generate);
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2), resolve(import.meta.dir, '../..')));
}

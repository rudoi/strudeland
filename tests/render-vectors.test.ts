/**
 * Staleness of the committed render vectors (`crates/strudeland-render/tests/vectors/`, written
 * only by `bun run render:vectors --write`), and the generator's own pieces.
 */

import { describe, expect, test } from 'bun:test';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { integratedLufs } from '../src/lib/render/loudness';
import {
  amp,
  fileText,
  fnv1a64,
  generateVectors,
  joinerSample,
  LOUDNESS_SIGNALS,
  main,
  parseVectorsArgs,
  readVectors,
  synthChannel,
  VECTORS_DIR,
} from '../scripts/render/loudness-vectors';

const ROOT = resolve(import.meta.dir, '..');
const REGENERATE = 'run `bun run render:vectors --write`';
// Generating hashes every joined buffer with BigInt: once for the whole file.
const GENERATED = generateVectors();

describe('render vectors', () => {
  test('the committed vectors are current', () => {
    const committed = readVectors(join(ROOT, VECTORS_DIR));
    const generated = GENERATED;
    expect([...committed.keys()].sort(), `vector files: ${REGENERATE}`).toEqual(
      [...generated.keys()].sort(),
    );
    const decoder = new TextDecoder();
    for (const [name, text] of generated) {
      expect(decoder.decode(committed.get(name)), `${name} is stale: ${REGENERATE}`).toBe(text);
    }
  });

  test('fnv1a64 over little-endian channel bytes', () => {
    expect(fnv1a64([])).toBe('cbf29ce484222325');
    // FNV-1a 64 of the single byte 0x00 and of 1.0f (00 00 80 3f).
    expect(fnv1a64([new Float32Array(0)])).toBe('cbf29ce484222325');
    const one = fnv1a64([new Float32Array([1])]);
    expect(one).toHaveLength(16);
    expect(fnv1a64([new Float32Array([1]), new Float32Array([0])])).not.toBe(one);
    expect(fnv1a64([new Float32Array([0, 1])])).toBe(
      fnv1a64([new Float32Array([0]), new Float32Array([1])]),
    );
  });

  test('joiner samples are exact in f32', () => {
    for (const [k, c, i] of [
      [0, 0, 0],
      [3, 1, 1_000_000],
      [7, 1, 65_521],
    ] as const) {
      const x = joinerSample(k, c, i);
      expect(Math.fround(x)).toBe(x);
      expect(x).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThan(1);
    }
    expect(joinerSample(0, 0, 0)).toBe(0);
    expect(joinerSample(0, 0, 1)).toBe(31 / 65536);
  });

  test('every joiner case has a hash and a frame count', () => {
    const text = GENERATED.get('joiner.json')!;
    const file = JSON.parse(text) as {
      schema: number;
      generator: string;
      cases: { name: string; frames: number; fnv1a64: string }[];
    };
    expect(file.schema).toBe(1);
    expect(file.generator).toBe('scripts/render/loudness-vectors.ts');
    expect(file.cases.map((c) => c.name)).toEqual([
      'single-chunk',
      'three-equal-chunks',
      'cps-7-16',
      'fractional-bar',
      'overlap-tail-dropped',
      'from-bar-after-zero',
      'uneven-chunks',
    ]);
    for (const c of file.cases) {
      expect(c.fnv1a64, c.name).toMatch(/^[0-9a-f]{16}$/);
      expect(Number.isInteger(c.frames), c.name).toBe(true);
    }
    expect(fileText({ cases: [] })).toBe(
      '{"schema":1,\n"generator":"scripts/render/loudness-vectors.ts",\n"cases":[]}\n',
    );
  });

  test('arguments', () => {
    expect(parseVectorsArgs([])).toEqual({ write: false, help: false });
    expect(parseVectorsArgs(['--write'])).toEqual({ write: true, help: false });
    expect(parseVectorsArgs(['-h'])).toEqual({ write: false, help: true });
    expect(parseVectorsArgs(['--nope'])).toEqual({ error: 'unknown argument --nope' });
    const lines: string[] = [];
    expect(main(['--nope'], ROOT, (l) => lines.push(l))).toBe(2);
    expect(lines).toEqual(['unknown argument --nope', 'usage: bun run render:vectors [--write]']);
    expect(main(['--help'], ROOT, () => {})).toBe(0);
  });

  test('check mode exits 0 on the committed files and 1 on a stale directory', () => {
    const lines: string[] = [];
    expect(
      main(
        [],
        ROOT,
        (l) => lines.push(l),
        () => GENERATED,
      ),
    ).toBe(0);
    expect(lines).toEqual(['vectors are current (2 files)']);
    const out: string[] = [];
    // A root without the vectors directory: every file is missing.
    expect(
      main(
        [],
        join(ROOT, 'tests/helpers'),
        (l) => out.push(l),
        () => GENERATED,
      ),
    ).toBe(1);
    expect(out.at(-1)).toBe(
      'vectors differ; run `bun run render:vectors --write` and review the diff',
    );
  });

  test('a changed byte of loudness.json fails the check', () => {
    const root = mkdtempSync(join(tmpdir(), 'render-vectors-'));
    try {
      cpSync(join(ROOT, VECTORS_DIR), join(root, VECTORS_DIR), { recursive: true });
      const file = join(root, VECTORS_DIR, 'loudness.json');
      const text = readFileSync(file, 'utf8');
      expect(
        main(
          [],
          root,
          () => {},
          () => GENERATED,
        ),
      ).toBe(0);
      writeFileSync(file, text.replace('"lufs":-22.99', '"lufs":-22.98'));
      const out: string[] = [];
      expect(
        main(
          [],
          root,
          (l) => out.push(l),
          () => GENERATED,
        ),
      ).toBe(1);
      expect(out.some((l) => l.includes('loudness.json'))).toBe(true);
      expect(
        main(
          ['--write'],
          root,
          () => {},
          () => GENERATED,
        ),
      ).toBe(0);
      expect(readFileSync(file, 'utf8')).toBe(text);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('every loudness signal has its integratedLufs', () => {
    const file = JSON.parse(GENERATED.get('loudness.json')!) as {
      schema: number;
      signals: { name: string; lufs: number | null }[];
    };
    expect(file.schema).toBe(1);
    expect(file.signals.map((s) => s.name)).toEqual([
      'silence-3s',
      'short-300ms',
      'sine1k-m23-stereo',
      'sine1k-m23-left-only',
      'sine1k-m23-mono',
      'sine100-m18',
      'sine5k-m30',
      'noise-m20',
      'gated-burst',
      'relative-gate',
      'relative-gate-drops',
      'below-gate',
      'two-sines-stereo',
    ]);
    const lufs = Object.fromEntries(file.signals.map((s) => [s.name, s.lufs]));
    for (const name of ['silence-3s', 'short-300ms', 'below-gate']) expect(lufs[name]).toBeNull();
    expect(Math.abs(lufs['sine1k-m23-stereo']! + 23)).toBeLessThan(0.1);
    expect(Math.abs(lufs['relative-gate-drops']! + 23)).toBeLessThan(0.2);
    expect(lufs['sine1k-m23-left-only']).toBe(lufs['sine1k-m23-mono']!);
    expect(LOUDNESS_SIGNALS.every((s) => s.sampleRate === 44100)).toBe(true);
  });

  test('a recipe sums its parts inside their bounds and rounds once to f32', () => {
    const x = synthChannel(
      {
        frames: 6,
        parts: [
          { kind: 'sine', hz: 1000, amp: 0.5, from: 1, to: 4 },
          { kind: 'noise', amp: 0.25, seed: 7, from: 3 },
          { kind: 'sine', hz: 1000, amp: 0.5, to: 99 },
        ],
      },
      44100,
    );
    const s = (i: number) => 0.5 * Math.sin((2 * Math.PI * 1000 * i) / 44100);
    // mulberry32(7)'s first three values, drawn for frames 3, 4 and 5.
    let a = 7;
    const next = () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const n = [0, 0, 0, next(), next(), next()];
    const want = [0, 1, 2, 3, 4, 5].map((i) =>
      Math.fround((i >= 1 && i < 4 ? s(i) : 0) + (i >= 3 ? 0.25 * (2 * n[i]! - 1) : 0) + s(i)),
    );
    expect(Array.from(x)).toEqual(want);
    expect(amp(-20)).toBeCloseTo(0.1, 15);
    expect(integratedLufs([x], 44100)).toBe(-Infinity);
  });
});

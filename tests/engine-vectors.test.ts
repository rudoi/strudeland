/**
 * The JS vectors generator (`scripts/engine/vectors.ts`): deterministic, its `stable` mirror
 * agrees with rng.mjs's own, and its command line (diff, write, exit codes). The staleness of
 * the committed files is checked in `engine-artifacts.test.ts`.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { getAudioSeed, hashSeed, setAudioSeed, voiceSeed } from 'superdough/rng.mjs';
import {
  CRACKLE_CASES,
  DUCK_SEQUENCE,
  f32Bits,
  fileText,
  fnv1aF32,
  generateVectors,
  main,
  NOISE_SEEDS,
  parseVectorsArgs,
  readVectors,
  stableMirror,
  STABLE_VALUES,
  VECTORS_DIR,
  VOICE_SEED_SEQUENCES,
} from '../scripts/engine/vectors';
import { decodeValue, type EngineValue } from '../src/lib/render/engine-job';
import { fromBitsHex as fromBits, toBitsHex as f64Bits } from '../src/render/math';
import { gainAt, type GainPoint } from '../src/lib/audio/offline';

const FILES = [
  'adsr.json',
  'duck-gain-at.json',
  'frequency.json',
  'js-hash-seed.json',
  'js-mulberry32.json',
  'js-number-to-string.json',
  'js-stable.json',
  'js-voice-seed.json',
  'noise-brown.json',
  'noise-crackle.json',
  'noise-pink.json',
  'noise-white.json',
  'reverb-ir.json',
  'waveform-n.json',
];

describe('generateVectors', () => {
  test('is deterministic and owns the vector files', () => {
    const a = generateVectors();
    const b = generateVectors();
    expect([...a.keys()].sort()).toEqual(FILES);
    expect(b).toEqual(a);
  });

  test('restores the audio seed it found', () => {
    const before = getAudioSeed();
    setAudioSeed(31337);
    generateVectors();
    expect(getAudioSeed()).toBe(31337);
    setAudioSeed(before);
  });

  test('every file is schema 1 JSON with one case per line', () => {
    for (const [name, text] of generateVectors()) {
      const parsed = JSON.parse(text);
      expect(parsed.schema, name).toBe(1);
      expect(parsed.generator, name).toBe('scripts/engine/vectors.ts');
      expect(Array.isArray(parsed.cases), name).toBe(true);
      expect(text.split('\n').length, name).toBeGreaterThan(parsed.cases.length);
      expect(text.endsWith('}\n'), name).toBe(true);
    }
  });

  test('number-to-string cases carry the exact value, its bits and String(x)', () => {
    const { cases } = JSON.parse(generateVectors().get('js-number-to-string.json')!);
    const strings = new Set(cases.map((c: { string: string }) => c.string));
    for (const s of ['0', 'NaN', 'Infinity', '-Infinity', '1e+21', '1e-7', '5e-324', '0.000001'])
      expect(strings.has(s), s).toBe(true);
    for (const c of cases) {
      const x = fromBits(c.bits);
      expect(String(x)).toBe(c.string);
      const v = decodeValue(c.value) as number;
      expect(Number.isNaN(x) ? Number.isNaN(v) : Object.is(v, x)).toBe(true);
    }
    // The exact ties (JS takes the even digit) are in.
    expect(strings.has('1125899906842624.2')).toBe(true);
  });

  test('noise files pin the head, tail and hash of every buffer', () => {
    const files = generateVectors();
    for (const type of ['white', 'pink', 'brown', 'crackle']) {
      const { cases } = JSON.parse(files.get(`noise-${type}.json`)!);
      expect(cases).toHaveLength(type === 'crackle' ? CRACKLE_CASES.length : NOISE_SEEDS.length);
      for (const c of cases) {
        expect(c.length).toBe(88200);
        expect(c.head).toHaveLength(1024);
        expect(c.tail).toHaveLength(1024);
        expect(Number.isInteger(c.fnv1a)).toBe(true);
      }
      // Different seeds give different buffers.
      expect(new Set(cases.map((c: { fnv1a: number }) => c.fnv1a)).size).toBe(cases.length);
    }
    // Crackle at density 0.02 is mostly zeros.
    const crackle = JSON.parse(files.get('noise-crackle.json')!).cases[0];
    expect(crackle.density).toBe(0.02);
    expect(crackle.head.filter((b: number) => b !== 0).length).toBeLessThan(10);
  });

  test('fnv1aF32 and f32Bits', () => {
    expect(fnv1aF32(new Float32Array(0))).toBe(2166136261);
    // One byte 0x00 four times: FNV-1a of [0, 0, 0, 0].
    let h = 2166136261;
    for (let i = 0; i < 4; i++) h = Math.imul(h, 16777619) >>> 0;
    expect(fnv1aF32(new Float32Array([0]))).toBe(h);
    expect(fnv1aF32(new Float32Array([1]))).not.toBe(h);
    expect(f32Bits(new Float32Array([1, -2, 0.5]))).toEqual([0x3f800000, 0xc0000000, 0x3f000000]);
    expect(f32Bits(new Float32Array([1, 2, 3]).subarray(1))).toEqual([0x40000000, 0x40400000]);
  });

  test('adsr covers every getADSRValues rule and the three getParamADSR shapes', () => {
    const { cases, paramAdsr } = JSON.parse(generateVectors().get('adsr.json')!);
    const results = cases.map((c: { result: unknown[] }) => JSON.stringify(c.result));
    expect(results).toContain('[0.001,0.001,1,0.01]'); // none given, no defaults
    expect(results).toContain('[0.001,0.2,0.001,0.01]'); // decay only: sustain 0.001
    expect(results).toContain('[0.1,0.001,1,0.01]'); // attack only: sustain 1
    expect(cases.some((c: { result: unknown[] }) => JSON.stringify(c.result).includes('NaN'))).toBe(
      true,
    );
    const counts = paramAdsr.map((c: { calls: unknown[] }) => c.calls.length);
    expect(counts).toContain(5); // normal
    expect(counts).toContain(4); // a + d > dur
    expect(counts).toContain(3); // a > dur
    const methods = new Set(
      paramAdsr.flatMap((c: { calls: { method: string }[] }) => c.calls.map((x) => x.method)),
    );
    expect([...methods].sort()).toEqual([
      'exponentialRampToValueAtTime',
      'linearRampToValueAtTime',
      'setValueAtTime',
    ]);
  });

  test('frequency covers note 0, freq 0, names, octave and the note error', () => {
    const { cases } = JSON.parse(generateVectors().get('frequency.json')!);
    const by = (v: object) =>
      cases.find((c: { value: unknown }) => JSON.stringify(c.value) === JSON.stringify(v));
    expect(by({ note: 0 }).freq).toBe(by({}).freq); // note 0 is falsy: the default 36
    expect(by({ note: 69 }).freq).toBe(440);
    expect(by({ freq: 330, octave: 1 }).freq).toBe(660);
    expect(by({ note: 'x9' }).error).toBe('not a note: "x9"');
    for (const c of cases) if (c.bits) expect(f64Bits(decodeValue(c.freq) as number)).toBe(c.bits);
  });

  test('mulberry32 covers seeds 0 and 2^31-1 with 64 outputs each', () => {
    const { cases, makeRandom } = JSON.parse(generateVectors().get('js-mulberry32.json')!);
    const seeds = cases.map((c: { seed: number }) => c.seed);
    expect(seeds).toContain(0);
    expect(seeds).toContain(2147483647);
    for (const c of cases) expect(c.outputs).toHaveLength(64);
    expect(makeRandom.length).toBeGreaterThan(0);
  });

  test('voice-seed sequences skip silence markers without advancing occurrence', () => {
    const { cases } = JSON.parse(generateVectors().get('js-voice-seed.json')!);
    expect(cases.map((c: { name: string }) => c.name)).toEqual(
      VOICE_SEED_SEQUENCES.map((s) => s.name),
    );
    const silence = cases.find((c: { name: string }) => c.name.startsWith('silence'))!;
    const seeds = silence.steps.map((s: { seed: number | null }) => s.seed);
    expect(seeds.filter((s: number | null) => s === null)).toHaveLength(3);
    // The sine hap's seeds are occurrence 0, 1, 2, 3: all different, no reset by the markers.
    const sine = seeds.filter((s: number | null) => s !== null);
    expect(new Set(sine).size).toBe(4);
    const repeats = cases[0].steps.map((s: { seed: number }) => s.seed);
    expect(new Set(repeats).size).toBe(repeats.length);
  });
});

describe('stableMirror', () => {
  test('agrees with rng.mjs stable through voiceSeed', () => {
    const before = getAudioSeed();
    for (const v of STABLE_VALUES) {
      setAudioSeed(undefined);
      setAudioSeed(5);
      // valueHash({ k: v }) is hashSeed('k', stable(v)).
      const vh = hashSeed('k', stableMirror(v));
      expect(voiceSeed(2, { k: v }), JSON.stringify(v)).toBe(hashSeed(5, 2, vh, 0));
    }
    setAudioSeed(undefined);
    setAudioSeed(before);
  });

  test('sorts integer-like keys first and escapes like JSON.stringify', () => {
    expect(stableMirror({ 10: 1, 2: 2, a: 3 })).toBe('{"2":2,"10":1,"a":3}');
    expect(stableMirror([NaN, -0])).toBe('[null,0]');
    expect(stableMirror('x')).toBe('x');
    expect(stableMirror(null)).toBe('null');
  });
});

describe('fileText', () => {
  test('fileText writes one element per line', () => {
    expect(fileText({ cases: [{ a: 1 }, { b: 2 }], empty: [] })).toBe(
      '{"schema":1,\n"generator":"scripts/engine/vectors.ts",\n"cases":[\n{"a":1},\n{"b":2}\n],\n"empty":[]}\n',
    );
  });
});

describe('parseVectorsArgs', () => {
  test('flags', () => {
    expect(parseVectorsArgs([])).toEqual({ write: false, help: false });
    expect(parseVectorsArgs(['--write'])).toEqual({ write: true, help: false });
    expect(parseVectorsArgs(['-h'])).toEqual({ write: false, help: true });
    expect(parseVectorsArgs(['--bogus'])).toEqual({ error: 'unknown argument --bogus' });
    expect(parseVectorsArgs(['js-stable'])).toEqual({ error: 'unknown argument js-stable' });
  });
});

describe('main', () => {
  let root: string;
  let logs: string[];
  const log = (l: string) => logs.push(l);
  const dir = () => join(root, VECTORS_DIR);

  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), 'engine-vectors-test-'));
    logs = [];
  });
  afterEach(() => rmSync(root, { recursive: true, force: true }));

  test('without --write it reports the diff, writes nothing and exits 1', () => {
    expect(main([], root, log)).toBe(1);
    expect(logs.join('\n')).toContain('14 added, 0 changed, 0 removed');
    expect(logs.join('\n')).toContain('engine:vectors --write');
    expect(readVectors(dir()).size).toBe(0);
  });

  test('--write writes the files, removes strays, then the vectors are current', () => {
    mkdirSync(dir(), { recursive: true });
    writeFileSync(join(dir(), 'js-old.json'), '{}');
    writeFileSync(join(dir(), 'js-stable.json'), '{}');
    expect(main([], root, log)).toBe(1);
    expect(logs.join('\n')).toMatch(/13 added, 1 changed, 1 removed, 0 unchanged/);
    logs = [];
    expect(main(['--write'], root, log)).toBe(0);
    expect(readdirSync(dir()).sort()).toEqual(FILES);
    expect(readFileSync(join(dir(), 'js-stable.json'), 'utf8')).toBe(
      generateVectors().get('js-stable.json')!,
    );
    logs = [];
    expect(main([], root, log)).toBe(0);
    expect(logs).toEqual(['vectors are current (14 files)']);
  });

  test('bad arguments exit 2; --help exits 0', () => {
    expect(main(['--nope'], root, log)).toBe(2);
    expect(logs).toContain('usage: bun run engine:vectors [--write]');
    logs = [];
    expect(main(['--help'], root, log)).toBe(0);
    expect(logs).toEqual(['usage: bun run engine:vectors [--write]']);
  });
});

describe('duck-gain-at.json', () => {
  const file = () => JSON.parse(generateVectors().get('duck-gain-at.json')!);
  const num = (v: unknown) => decodeValue(v as EngineValue) as number;

  test('every sample is the real gainAt of its case, exact in bits', () => {
    const { cases } = file();
    const names = cases.map((c: { name: string }) => c.name);
    for (const n of [
      'no points',
      'held point',
      'ramp',
      'ramp end not after the previous event',
      'ramp first',
      'two stacked ducks',
      'floor 0.01',
      'negative onset',
      'sequence after duck 1',
      'sequence after duck 4',
    ])
      expect(names, n).toContain(n);
    for (const c of cases) {
      const points: GainPoint[] = c.points.map(
        (p: { time: unknown; value: unknown; ramp: boolean }) => ({
          time: num(p.time),
          value: num(p.value),
          ramp: p.ramp,
        }),
      );
      // 64 evenly spaced x values plus each point time and its two neighbours.
      expect(c.samples.length, c.name).toBe(64 + 3 * points.length);
      for (const s of c.samples) {
        const x = fromBits(s.xBits);
        expect(Object.is(num(s.x), x), c.name).toBe(true);
        const y = gainAt(num(c.initial), points, x);
        expect(s.bits, `${c.name} x=${x}`).toBe(f64Bits(y));
      }
    }
  });

  test('the ramp case is seen inside, at its start and at its end', () => {
    const ramp = file().cases.find((c: { name: string }) => c.name === 'ramp');
    const at = (x: number) =>
      num(ramp.samples.find((s: { xBits: string }) => s.xBits === f64Bits(x)).value);
    expect(at(0.2)).toBe(1);
    expect(at(0.5)).toBe(0.1);
    const inside = ramp.samples.map((s: { value: unknown }) => num(s.value));
    expect(inside.some((v: number) => v > 0.1 && v < 1)).toBe(true);
  });

  test('the sequence records offlineDuck: a cancel and two ramps per duck, matching the points', () => {
    const { calls, sequence, cases } = file();
    expect(sequence.length).toBe(DUCK_SEQUENCE.length);
    expect(calls.length).toBe(3 * DUCK_SEQUENCE.length);
    DUCK_SEQUENCE.forEach((d, k) => {
      const [cancel, fall, rise] = calls.slice(3 * k, 3 * k + 3);
      expect(cancel.method).toBe('cancelAndHoldAtTime');
      expect(fromBits(cancel.args[0].bits)).toBe(d.t);
      expect([fall.method, rise.method]).toEqual([
        'exponentialRampToValueAtTime',
        'exponentialRampToValueAtTime',
      ]);
      const pts = cases.find((c: { name: string }) => c.name === `sequence after duck ${k + 1}`);
      const n = pts.points.length;
      // The rebuilt points agree with what offlineDuck scheduled.
      expect(fall.args[0].value).toEqual(pts.points[n - 2].value);
      expect(fall.args[1].value).toEqual(pts.points[n - 2].time);
      expect(rise.args[1].value).toEqual(pts.points[n - 1].time);
      expect(num(rise.args[0].value)).toBe(1);
    });
    // Duck 3 lands inside duck 2's recovery: its held value is below 1 and clamps ducked.
    const third = cases.find((c: { name: string }) => c.name === 'sequence after duck 3').points;
    const held = num(
      third.find((p: { time: unknown; ramp: boolean }) => num(p.time) === 1.05 && !p.ramp).value,
    );
    expect(held).toBeLessThan(1);
    expect(num(calls[7].args[0].value)).toBe(held);
  });
});

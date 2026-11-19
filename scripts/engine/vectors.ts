/**
 * `bun run engine:vectors [--write]`: the JS vectors the Rust engine's bit-exact ports are checked
 * against (`crates/strudeland-engine/tests/vectors/*.json`). They come from the vendored
 * superdough modules run under Bun (no browser), so `bun test` can regenerate them in memory and
 * byte-compare them with the committed files (`tests/engine-artifacts.test.ts`).
 *
 * Without `--write` it prints the diff summary and exits 1 when the committed files differ; with
 * `--write` it writes the generated files and removes the ones it no longer generates. Exit 2 on
 * bad arguments.
 *
 * Format: JSON, floats as JS's shortest round-trip decimal text (non-finite values and -0 as
 * `{ "$num": ... }`, as in engine jobs) and, where a value must be exact whatever the parser,
 * also as `bits` (the f64 bit pattern, 16 lowercase hex digits). Bun runs JavaScriptCore, not V8:
 * float buffers that depend on `Math.sin`/`pow`/... (later files) are compared within 1 ulp; the
 * `js-*` files here are pure integer arithmetic and formatting, compared exactly.
 */

import { resolve } from 'node:path';
import {
  getAudioSeed,
  hashSeed,
  makeRandom,
  mulberry32,
  setAudioSeed,
  voiceSeed,
} from 'superdough/rng.mjs';
import { getADSRValues, getFrequencyFromValue, getParamADSR } from 'superdough/helpers.mjs';
import { setLogger } from 'superdough/logger.mjs';
import { setAudioContext } from 'superdough/audioContext.mjs';
import { waveformN } from 'superdough/synth.mjs';
import { getNoiseBuffer } from 'superdough/noise.mjs';
import reverbGen from 'superdough/reverbGen.mjs';
import { encodeValue } from '../../src/lib/render/engine-job';
import { gainAt, offlineDuck, type GainPoint } from '../../src/lib/audio/offline';
import { nextDown, nextUp } from './v8-math-cases';
import { fromBitsHex as fromBits, toBitsHex as f64Bits } from '../../src/render/math';
import { runVectors, vectorsFileText } from '../vectors-files';

export { parseVectorsArgs, readVectors, type VectorsOptions } from '../vectors-files';

export const VECTORS_DIR = 'crates/strudeland-engine/tests/vectors';
const GENERATOR = 'scripts/engine/vectors.ts';

/** A vector file's text (`vectorsFileText`) with this generator's name. */
export const fileText = (sections: Record<string, unknown>): string =>
  vectorsFileText(GENERATOR, sections);

// ---- js-number-to-string

function numberToStringCases(): object[] {
  const values: number[] = [
    0,
    -0,
    NaN,
    Infinity,
    -Infinity,
    1,
    -1,
    2,
    7,
    10,
    42,
    -42,
    100,
    440,
    1000,
    15000,
    44100,
    4294967295,
    4294967296,
    2147483647,
    -2147483648,
    9007199254740991,
    9007199254740992,
    9007199254740993,
    123456789012345680000,
    1e20,
    999999999999999900000,
    1e21,
    1.5e21,
    -1e21,
    1e22,
    1e100,
    1.7976931348623157e308,
    0.1,
    0.2,
    0.3,
    0.1 + 0.2,
    0.375,
    0.5,
    0.25,
    0.05,
    0.01,
    0.001,
    0.0001,
    0.00001,
    0.000001,
    0.0000012,
    0.0000015,
    9.99e-7,
    1e-7,
    -1e-7,
    1.25e-7,
    1e-10,
    2.2250738585072014e-308,
    5e-324,
    -5e-324,
    1 / 3,
    2 / 3,
    123.456,
    -123.456,
    1.5e-3,
    Math.PI,
    Math.E,
    1 / 44100,
    128 / 44100,
    0.7071067811865476,
    261.6255653005986,
    1e-6 - 1e-22,
  ];
  // Exact ties between two shortest candidates (2^50 + n + 1/4 or 3/4, ulp 1/4): JS takes the
  // even last digit.
  for (let n = 0; n < 40; n++) values.push(2 ** 50 + n + 0.25, 2 ** 50 + n + 0.75);
  for (let e = -325; e <= 309; e += 1) values.push(Number(`1e${e}`));
  for (let e = -325; e <= 309; e += 5) values.push(Number(`7.5e${e}`));
  // Random bit patterns, decimals with few digits and plain fractions: a seeded spread.
  const rnd = mulberry32(20261111);
  const u32 = () => Math.floor(rnd() * 4294967296);
  for (let i = 0; i < 400; i++) {
    const hex = u32().toString(16).padStart(8, '0') + u32().toString(16).padStart(8, '0');
    values.push(fromBits(hex));
  }
  for (let i = 0; i < 200; i++) values.push(u32() / 10 ** Math.floor(rnd() * 14));
  for (let i = 0; i < 200; i++) values.push(rnd() * 10 ** (Math.floor(rnd() * 44) - 22));
  return values.map((x) => ({ value: encodeValue(x), bits: f64Bits(x), string: String(x) }));
}

// ---- js-hash-seed

type Part = number | string;

const partOf = (p: Part) =>
  typeof p === 'number'
    ? { type: 'number', value: encodeValue(p), bits: f64Bits(p) }
    : { type: 'string', value: p };

function hashSeedCases(): object[] {
  const lists: Part[][] = [
    [],
    [''],
    ['', ''],
    [0],
    [-0],
    [1],
    ['1'],
    [NaN],
    [Infinity, -Infinity],
    [0, 'noise', 'white', 44100],
    [0, 'noise', 'pink', 44100],
    [-1, 'noise', 'brown', 44100],
    [2147483647, 'reverb', 0.5],
    [-2147483648, 0.375, 3735928559, 2],
    [0, 1 / 3, 12345, 0],
    ['note', '60', 's', 'sine'],
    ['😀'],
    ['a😀b', 'é', '日本語'],
    ['\u0000'],
    ['a\u0000string:b'],
    ['a', 'b'],
    ['"quoted"', '\\', '\n\t'],
    ['\u{10FFFF}', '￿', ''],
    [5e-324, 1e21, 1e-7, 123456789012345680000],
    ['x'.repeat(1000)],
  ];
  return lists.map((parts) => ({ parts: parts.map(partOf), hash: hashSeed(...parts) }));
}

// ---- js-mulberry32

function mulberryCases(): Record<string, unknown> {
  const seeds = [
    0,
    1,
    2,
    42,
    123456789,
    2147483647,
    2147483648,
    3735928559,
    4294967295,
    hashSeed(0, 'noise', 'white', 44100),
  ];
  const cases = seeds.map((seed) => {
    const next = mulberry32(seed);
    return { seed, outputs: Array.from({ length: 64 }, () => next()) };
  });
  const keyed: [number, Part[]][] = [
    [0, ['noise', 'white', 44100]],
    [-1, ['noise', 'crackle', 44100]],
    [2147483647, []],
    [-2147483648, ['reverb', 2, 0.1]],
  ];
  const previous = getAudioSeed();
  const makeRandomCases = keyed.map(([seed, keyParts]) => {
    setAudioSeed(seed);
    const next = makeRandom(...keyParts);
    return { seed, keyParts, outputs: Array.from({ length: 8 }, () => next()) };
  });
  setAudioSeed(previous);
  return { cases, makeRandom: makeRandomCases };
}

// ---- js-stable

/** rng.mjs's `stable` is not exported (and `vendor/strudel/` is not patched), so this mirrors it
 *  literally (`sortKeys` + `JSON.stringify`); `tests/engine-vectors.test.ts` checks the mirror
 *  against the real one through `voiceSeed`, whose `valueHash({ k: v })` is
 *  `hashSeed('k', stable(v))`. */
function sortKeys(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sortKeys);
  if (v !== null && typeof v === 'object') {
    return Object.fromEntries(
      Object.keys(v)
        .sort()
        .map((k) => [k, sortKeys((v as Record<string, unknown>)[k])]),
    );
  }
  return v;
}

export function stableMirror(v: unknown): string {
  if (v !== null && typeof v === 'object') return JSON.stringify(sortKeys(v));
  return String(v);
}

export const STABLE_VALUES: unknown[] = [
  null,
  true,
  false,
  0,
  -0,
  NaN,
  Infinity,
  1e21,
  0.1,
  'plain',
  '',
  [],
  {},
  [1, 2, 3],
  [null, NaN, -0, Infinity, -Infinity, 1e-7, 5e-324],
  { b: 1, a: 2 },
  { 10: 1, 2: 2, a: 3 },
  { '02': 0, '-1': 1, '1.5': 2, 4294967294: 3, 4294967295: 4, 0: 5, b: 6, B: 7 },
  { z: { y: { x: [{ b: 1, a: 0 }] } }, a: [[], {}] },
  { s: 'sine', note: 60, gain: 0.5, n: [0, 4, 7] },
  { '😀': 1, '｡': 2, é: 3, e: 4 },
  ['q"\\\b\f\n\r\t\u0001\u001f\u007f  😀'],
  { 'k"\n': 'v\u0000' },
  [true, false, null, 'x', { nested: [1, { deep: 'y' }] }],
  // A Set (modulator `__ids`) has no own keys: `sortKeys` makes it `{}`.
  new Set(),
  new Set([1, 'a']),
  new Set([1, 2, 3]),
  new Set([new Set([1])]),
  { a: new Set([1]) },
  [new Set(['x'])],
  { __ids: new Set([0, 1]), 0: { control: 'gain', depth: 0.5 } },
  [new Set([3])],
];

// ---- js-voice-seed

interface Step {
  cycle: number;
  duration: number;
  value: Record<string, unknown>;
}

/** superdough.mjs:483-566 on a copy of `value`: `duration`, the silence return, the `bank`
 *  rewrite, then `voiceSeed`. `null` for a silence marker (it returns before `voiceSeed`). */
function superdoughSeed(step: Step): number | null {
  const value: Record<string, unknown> = structuredClone(step.value);
  value.duration = step.duration;
  let { s = 'triangle' } = value as { s?: unknown };
  const { bank } = value;
  if (['-', '~', '_'].includes(s as string)) return null;
  if (bank && s) {
    s = `${bank}_${s}`;
    value.s = s;
  }
  return voiceSeed(step.cycle, value) as number;
}

const SINE = { s: 'sine', note: 60 };
const SAW = { s: 'sawtooth', note: 'c3', gain: 0.5 };
const step = (cycle: number, value: Record<string, unknown>, duration = 0.5): Step => ({
  cycle,
  duration,
  value,
});

export const VOICE_SEED_SEQUENCES: { name: string; seed: number; steps: Step[] }[] = [
  {
    name: 'repeats in one cycle',
    seed: 0,
    steps: [step(0, SINE), step(0, SINE), step(0, SINE), step(0, SAW), step(0, SINE)],
  },
  {
    name: 'cycle changes reset',
    seed: 0,
    steps: [step(0, SINE), step(0, SINE), step(1, SINE), step(1, SINE), step(0, SINE)],
  },
  {
    name: 'interleaved cycles',
    seed: 42,
    steps: [step(0, SINE), step(0.5, SINE), step(0, SINE), step(0.5, SINE), step(0.5, SINE)],
  },
  {
    name: 'duration is the hap duration',
    seed: 1,
    steps: [
      step(2, SINE, 0.25),
      step(2, SINE, 0.5),
      step(2, { ...SINE, duration: 9 }, 0.25),
      step(2, { ...SINE, clip: 2 }, 0.25),
    ],
  },
  {
    name: 'bank prefixes s',
    seed: 7,
    steps: [
      step(3, { s: 'sine', bank: 'tr909' }),
      step(3, { s: 'tr909_sine', bank: 'tr909' }),
      step(3, { bank: 'tr909' }),
      step(3, { bank: 'tr909', s: 'triangle' }),
      step(3, { bank: '', s: 'sine' }),
      step(3, { bank: 0, s: 'sine' }),
      step(3, { bank: 'b', s: '' }),
      step(3, { bank: 'b', s: 0 }),
      step(3, { bank: 5, s: 2 }),
      step(3, { s: 'triangle' }),
      step(3, {}),
    ],
  },
  {
    name: 'silence markers do not advance occurrence',
    seed: -1,
    steps: [
      step(0, SINE),
      step(0, { s: '~' }),
      step(0, SINE),
      step(5, { s: '-', bank: 'x' }),
      step(0, SINE),
      step(0, { s: '_', note: 60 }),
      step(0, SINE),
    ],
  },
  {
    name: 'fractional and odd cycles',
    seed: 2147483647,
    steps: [
      step(1 / 3, SINE),
      step(1 / 3, SINE),
      step(0.1 + 0.2, SINE),
      step(0, SINE),
      step(-0, SINE),
      step(123456.875, SINE),
    ],
  },
  {
    name: 'value shapes',
    seed: -2147483648,
    steps: [
      step(0, { s: 'sine', note: [0, 4, 7], fx: { b: 1, a: [null, NaN] } }),
      step(0, { s: 'sine', note: [0, 4, 7], fx: { a: [null, NaN], b: 1 } }),
      step(0, { s: 'sine', gain: NaN, pan: -0, speed: Infinity, x: null, y: false }),
      step(0, { 10: 1, 2: 2, s: 'sine' }),
      step(0, { s: 'sine', _private: 1, _voiceSeed: 99 }),
      step(0, { s: 'sine' }),
      step(0, { s: 'sine', 'é😀': 'ü', '｡': 1 }),
      step(0, { s: 'square', note: 'c#4', vowel: 'a' }),
    ],
  },
  {
    name: 'other seed, same values',
    seed: 1234,
    steps: [step(0, SINE), step(0, SINE), step(1, SAW)],
  },
  {
    // `__ids` hashes as `{}`, so the three differ only by occurrence.
    name: 'a Set member (modulator ids)',
    seed: 3,
    steps: [
      step(0, { s: 'sine', lfo: { __ids: new Set([0]), 0: { control: 'gain', rate: 2 } } }),
      step(0, {
        s: 'sine',
        lfo: {
          __ids: new Set([0, 1]),
          0: { control: 'gain', rate: 2 },
          1: { control: 'pan' },
        },
      }),
      step(0, {
        s: 'sine',
        lfo: {
          __ids: new Set([1, 0]),
          0: { control: 'gain', rate: 2 },
          1: { control: 'pan' },
        },
      }),
    ],
  },
];

function voiceSeedCases(): object[] {
  const previous = getAudioSeed();
  const out = VOICE_SEED_SEQUENCES.map(({ name, seed, steps }) => {
    // setAudioSeed only resets the occurrence state on a change of seed.
    setAudioSeed(undefined);
    setAudioSeed(seed);
    return {
      name,
      seed,
      steps: steps.map((st) => ({
        cycle: encodeValue(st.cycle),
        duration: encodeValue(st.duration),
        value: encodeValue(st.value),
        seed: superdoughSeed(st),
      })),
    };
  });
  setAudioSeed(undefined);
  setAudioSeed(previous);
  return out;
}

// ---- adsr

type Adsr = (number | null | undefined)[];

/** `getADSRValues` inputs: every rule combination, zeros, negatives, NaN and null. */
const ADSR_VALUE_CASES: { params: Adsr; curve: string; defaults?: number[] }[] = [
  { params: [undefined, undefined, undefined, undefined], curve: 'linear' },
  {
    params: [undefined, undefined, undefined, undefined],
    curve: 'linear',
    defaults: [0.001, 0.05, 0.6, 0.01],
  },
  { params: [null, null, null, null], curve: 'exponential', defaults: [0.005, 0.14, 0, 0.1] },
  { params: [0.1, undefined, undefined, undefined], curve: 'linear' },
  { params: [undefined, 0.2, undefined, undefined], curve: 'linear' },
  { params: [undefined, undefined, 0.3, undefined], curve: 'linear' },
  { params: [undefined, undefined, undefined, 0.4], curve: 'linear' },
  { params: [0.1, 0.2, undefined, undefined], curve: 'linear' },
  { params: [0.1, undefined, 0.5, undefined], curve: 'linear' },
  { params: [undefined, 0.2, undefined, 0.4], curve: 'exponential' },
  { params: [0.1, 0.2, 0.5, 0.4], curve: 'linear', defaults: [0.001, 0.05, 0.6, 0.01] },
  { params: [0, 0, 0, 0], curve: 'linear' },
  { params: [-1, -1, -1, -1], curve: 'linear' },
  { params: [0.0001, 0.0005, 2, 0.001], curve: 'linear' },
  { params: [-0, undefined, -0, undefined], curve: 'linear' },
  { params: [NaN, undefined, undefined, undefined], curve: 'linear' },
  { params: [undefined, NaN, undefined, undefined], curve: 'linear' },
  { params: [undefined, undefined, NaN, undefined], curve: 'linear' },
  { params: [NaN, NaN, NaN, NaN], curve: 'linear' },
  { params: [null, 0.2, null, undefined], curve: 'linear' },
  { params: [1e9, 1e-9, 1e9, 1e9], curve: 'linear' },
  { params: [Infinity, -Infinity, Infinity, -Infinity], curve: 'linear' },
];

interface ParamAdsrCase {
  adsr: number[];
  min: number;
  max: number;
  begin: number;
  end: number;
  curve: 'linear' | 'exponential';
}

/** `getParamADSR` inputs: the three shapes (normal, `a + d > dur`, `a > dur`), both curves, the
 *  exponential 0 -> 0.001 mappings, swapped ranges, zero durations and NaN (nanFallback). */
const PARAM_ADSR_CASES: ParamAdsrCase[] = [
  { adsr: [0.001, 0.05, 0.6, 0.01], min: 0, max: 1, begin: 0, end: 0.25, curve: 'linear' },
  { adsr: [0.1, 0.2, 0.5, 0.3], min: 0, max: 1, begin: 0.5, end: 1.5, curve: 'linear' },
  { adsr: [0.3, 0.1, 0.5, 0.1], min: 0, max: 1, begin: 0.125, end: 0.25, curve: 'linear' },
  {
    adsr: [0.05, 0.3, 0.2, 0.1],
    min: 0,
    max: 1,
    begin: 1 / 3,
    end: 1 / 3 + 0.125,
    curve: 'linear',
  },
  { adsr: [0.1, 0.1, 0.5, 0.1], min: 0, max: 1, begin: 0.2, end: 0.2, curve: 'linear' },
  { adsr: [0, 0, 0, 0], min: 0, max: 1, begin: 0, end: 0.1, curve: 'linear' },
  { adsr: [0.005, 0.14, 0, 0.1], min: 100, max: 4000, begin: 0, end: 0.5, curve: 'exponential' },
  { adsr: [0.005, 0.14, 0, 0.1], min: 0, max: 0, begin: 0, end: 0.5, curve: 'exponential' },
  { adsr: [0.2, 0.001, 1, 0.001], min: -1200, max: 0, begin: 0.1, end: 0.3, curve: 'exponential' },
  { adsr: [0.2, 0.1, 0.5, 0.1], min: 0, max: 1200, begin: 0, end: 0.1, curve: 'exponential' },
  { adsr: [0.01, 0.5, 0, 0.1], min: 0, max: 1, begin: 0, end: 0.2, curve: 'exponential' },
  { adsr: [0.01, 0.1, 0.5, 0.1], min: 4000, max: 100, begin: 0, end: 0.5, curve: 'exponential' },
  { adsr: [NaN, NaN, NaN, NaN], min: 0, max: 1, begin: 0, end: 0.5, curve: 'linear' },
  { adsr: [0.1, NaN, 0.5, NaN], min: 0, max: 1, begin: 0, end: 0.5, curve: 'exponential' },
];

function adsrCases(): Record<string, unknown> {
  const enc = (xs: Adsr) => xs.map((x) => (x === undefined ? null : encodeValue(x)));
  const values = ADSR_VALUE_CASES.map(({ params, curve, defaults }) => ({
    params: enc(params),
    curve,
    defaults: defaults ?? null,
    result: (getADSRValues(params, curve, defaults) as number[]).map((x) => encodeValue(x)),
  }));
  // nanFallback logs a warning per NaN; keep the generator quiet, then restore the default.
  setLogger(() => {});
  const paramAdsr = PARAM_ADSR_CASES.map((c) => {
    const calls: unknown[] = [];
    const record = (method: string) => (value: number, time: number) =>
      calls.push({ method, value: encodeValue(value), time: encodeValue(time) });
    const param = {
      setValueAtTime: record('setValueAtTime'),
      linearRampToValueAtTime: record('linearRampToValueAtTime'),
      exponentialRampToValueAtTime: record('exponentialRampToValueAtTime'),
    };
    const [a, d, s, r] = c.adsr;
    getParamADSR(param, a, d, s, r, c.min, c.max, c.begin, c.end, c.curve);
    return { ...c, adsr: c.adsr.map((x) => encodeValue(x)), calls };
  });
  setLogger((msg: unknown) => console.log(msg));
  return { cases: values, paramAdsr };
}

// ---- frequency

/** `getFrequencyFromValue` inputs: note numbers and names, `freq`, `octave`, the falsy rules. */
const FREQUENCY_VALUES: Record<string, unknown>[] = [
  {},
  { note: 60 },
  { note: 69 },
  { note: 0 },
  { note: -12 },
  { note: 69.5 },
  { note: 127 },
  { note: 1e3 },
  { note: 'c3' },
  { note: 'a4' },
  { note: 'c#4' },
  { note: 'eb2' },
  { note: 'C' },
  { note: 'Bb-1' },
  { note: 'f##5' },
  { note: '' },
  { note: null },
  { note: true },
  { note: [60] },
  { note: 'x9' },
  { note: 'c-' },
  { freq: 440 },
  { freq: 0 },
  { freq: 0, note: 0 },
  { freq: 0, note: 72 },
  { freq: '220' },
  { freq: null, note: 'a4' },
  { freq: 1e30 },
  { freq: -110 },
  { freq: 440, note: 60 },
  { freq: 330, octave: 1 },
  { note: 60, octave: -2 },
  { note: 'a4', octave: 0.5 },
  { note: 60, octave: null },
  { note: 60, octave: 1e3 },
  { note: 60, octave: -1e3 },
  { note: true, freq: 0 },
  { note: [60], freq: 5 },
];

function frequencyCases(): object[] {
  return FREQUENCY_VALUES.map((value) => {
    try {
      const freq = getFrequencyFromValue(value) as number;
      return { value: encodeValue(value), freq: encodeValue(freq), bits: f64Bits(freq) };
    } catch (e) {
      return { value: encodeValue(value), error: (e as Error).message };
    }
  });
}

// ---- waveform-n

/** `waveformN(partials, phases, type)` inputs: counts, lists, coercions, phases, errors. */
const WAVEFORM_N_CASES: { partials: unknown; phases?: unknown; type: string }[] = [
  { partials: 4, type: 'sawtooth' },
  { partials: 5, type: 'square' },
  { partials: 6, type: 'triangle' },
  { partials: 3, type: 'user' },
  { partials: [1, 0.5, 0, 0.25], type: 'sawtooth' },
  { partials: [1, '0.5', null, true], type: 'user' },
  { partials: [1e-3, 1e3, -2], type: 'square' },
  { partials: 2.7, type: 'user' },
  { partials: '3', type: 'square' },
  { partials: true, type: 'triangle' },
  { partials: 4, phases: [0.25, 0.5, 0.125, 0.75], type: 'sawtooth' },
  { partials: [1, 1, 1], phases: [0, -0.1, 1.3], type: 'triangle' },
  { partials: [1, 0.5, 0.25, 0.125], phases: [0.1, 0.2, 0.3, 0.4], type: 'user' },
  { partials: 3, phases: '0.5', type: 'user' },
  { partials: 3, phases: 0.25, type: 'user' },
  { partials: 2, phases: [null, '0.25'], type: 'square' },
  { partials: 3, phases: [false, 0, -0], type: 'user' },
  { partials: 3000, type: 'sawtooth' },
  { partials: -1, type: 'user' },
  { partials: 3, type: 'sine' },
  { partials: 'abc', type: 'user' },
  { partials: {}, type: 'user' },
  { partials: [1, Infinity], type: 'triangle' },
  { partials: [1e39], type: 'user' },
  { partials: 2, phases: ['x'], type: 'user' },
];

/** The error class `createPeriodicWave` throws in Blink (`IndexSizeError` below two
 *  coefficients, `TypeError` for a non-finite float), for the stub context. */
function periodicWaveError(name: string, message: string): Error {
  const e = new Error(message);
  e.name = name;
  return e;
}

/** A stub audio context with just what `waveformN` uses; `createPeriodicWave` applies Blink's
 *  argument checks and returns the arrays. */
function waveformContext(): object {
  return {
    state: 'running',
    close() {},
    createOscillator() {
      return {
        setPeriodicWave(this: { wave?: unknown }, wave: unknown) {
          this.wave = wave;
        },
      };
    },
    createPeriodicWave(real: Float32Array, imag: Float32Array) {
      if (real.length < 2) throw periodicWaveError('IndexSizeError', 'too short');
      if ([...real, ...imag].some((x) => !Number.isFinite(x))) {
        throw periodicWaveError('TypeError', 'non-finite');
      }
      return { real, imag };
    },
  };
}

function waveformNCases(): object[] {
  setAudioContext(waveformContext());
  try {
    return WAVEFORM_N_CASES.map((c) => {
      const input = {
        partials: encodeValue(c.partials),
        ...('phases' in c ? { phases: encodeValue(c.phases) } : {}),
        type: c.type,
      };
      try {
        const osc = waveformN(c.partials, c.phases, c.type) as {
          wave: { real: Float32Array; imag: Float32Array };
        };
        return {
          ...input,
          real: [...osc.wave.real].map((x) => encodeValue(x)),
          imag: [...osc.wave.imag].map((x) => encodeValue(x)),
        };
      } catch (e) {
        return { ...input, error: (e as Error).name };
      }
    });
  } finally {
    // Back to "no context" (nothing else in a Bun process should find this stub).
    setAudioContext(undefined);
  }
}

// ---- noise

/** FNV-1a (32-bit) over the little-endian bytes of an f32 buffer. */
export function fnv1aF32(buffer: Float32Array): number {
  const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  let h = 2166136261;
  for (let i = 0; i < buffer.byteLength; i++) {
    h ^= view.getUint8(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h >>> 0;
}

/** The f32 bit patterns of `xs` as unsigned integers. */
export function f32Bits(xs: Float32Array): number[] {
  return [...new Uint32Array(xs.buffer, xs.byteOffset, xs.length)];
}

/** Audio seeds of the cached noise types; voice seeds and densities of crackle. */
export const NOISE_SEEDS = [0, -123457];
export const CRACKLE_CASES: { voiceSeed: number; density: number }[] = [
  { voiceSeed: 0, density: 0.02 },
  { voiceSeed: 0, density: 2 },
  { voiceSeed: 2654435769, density: 0.02 },
  { voiceSeed: 2654435769, density: 2 },
];
const NOISE_HEAD = 1024;

/** A stub audio context with what `getNoiseBuffer` uses: `sampleRate` and Float32Array-backed
 *  `createBuffer`. */
function noiseContext(): object {
  return {
    state: 'running',
    sampleRate: 44100,
    close() {},
    createBuffer(channels: number, length: number, sampleRate: number) {
      const data = Array.from({ length: channels }, () => new Float32Array(length));
      return { sampleRate, length, getChannelData: (c: number) => data[c] };
    },
  };
}

function noiseCase(buffer: Float32Array): object {
  return {
    length: buffer.length,
    fnv1a: fnv1aF32(buffer),
    head: f32Bits(buffer.subarray(0, NOISE_HEAD)),
    tail: f32Bits(buffer.subarray(buffer.length - NOISE_HEAD)),
  };
}

/** `getNoiseBuffer(type, ...)` for each seed (cached types) or voice seed and density
 *  (crackle): the first and last 1024 samples as f32 bits and a hash of the whole buffer. */
function noiseCases(type: 'white' | 'pink' | 'brown' | 'crackle'): object[] {
  const previous = getAudioSeed();
  setAudioContext(noiseContext());
  try {
    if (type === 'crackle') {
      setAudioSeed(undefined);
      setAudioSeed(0);
      return CRACKLE_CASES.map(({ voiceSeed, density }) => {
        const b = getNoiseBuffer(type, density, voiceSeed) as {
          getChannelData(c: number): Float32Array;
        };
        return { seed: 0, voiceSeed, density, ...noiseCase(b.getChannelData(0)) };
      });
    }
    return NOISE_SEEDS.map((seed) => {
      // A seed change clears superdough's noise cache.
      setAudioSeed(undefined);
      setAudioSeed(seed);
      const b = getNoiseBuffer(type, 0.02, 0) as { getChannelData(c: number): Float32Array };
      return { seed, ...noiseCase(b.getChannelData(0)) };
    });
  } finally {
    // Drop the stub buffers from the cache, then restore the seed and "no context".
    setAudioSeed(undefined);
    setAudioSeed(previous);
    setAudioContext(undefined);
  }
}

// ---- reverb-ir

/** One `generateReverb` request: the settings as `createReverb`'s `generate` passes them (after
 *  its defaults) and the audio seed. `lp` is always falsy: the nested lowpass needs a real
 *  `OfflineAudioContext` (it is pinned by a micro case instead). */
export interface ReverbIrCase {
  seed: number;
  d: unknown;
  fade: unknown;
  lp: unknown;
  dim: unknown;
}

/** 05's grid (d in {0.05, 0.5, 2}, fade in {0, 0.1}, two seeds) plus edge cases: an IR of one
 *  frame (`round(d * sr)` = 0, so `decayBase` is 0), of two frames, a fade longer than the IR,
 *  and non-numeric settings (a string, an array, `null`, a boolean) that JS coerces and hashes
 *  by `typeof`. */
const REVERB_IR_CASES: ReverbIrCase[] = [
  ...[0, -123457].flatMap((seed) =>
    [0.05, 0.5, 2].flatMap((d) => [0, 0.1].map((fade) => ({ seed, d, fade, lp: 0, dim: 1000 }))),
  ),
  { seed: 0, d: 0.4 / 44100, fade: 0.1, lp: 0, dim: 1000 },
  { seed: 0, d: 1 / 44100, fade: 0, lp: 0, dim: 1000 },
  { seed: 7, d: 0.0101, fade: 0.003, lp: null, dim: 0 },
  { seed: 7, d: '0.01', fade: null, lp: '', dim: 'x' },
  { seed: -1, d: [0.01], fade: true, lp: false, dim: [500] },
];
const REVERB_EDGE = 1024;

/** A stub context with what `generateReverb` uses without the lowpass: `sampleRate` and a
 *  Float32Array-backed `createBuffer` that refuses an empty buffer as Blink does. */
function reverbContext(): object {
  return {
    sampleRate: 44100,
    createBuffer(channels: number, length: number, sampleRate: number) {
      if (!(length >= 1)) throw new Error(`createBuffer length ${length}`);
      const data = Array.from({ length: channels }, () => new Float32Array(length));
      return {
        sampleRate,
        length,
        numberOfChannels: channels,
        getChannelData: (c: number) => data[c],
      };
    },
  };
}

function reverbChannel(x: Float32Array): object {
  return x.length <= 2 * REVERB_EDGE
    ? { fnv1a: fnv1aF32(x), samples: f32Bits(x) }
    : {
        fnv1a: fnv1aF32(x),
        head: f32Bits(x.subarray(0, REVERB_EDGE)),
        tail: f32Bits(x.subarray(x.length - REVERB_EDGE)),
      };
}

/** `generateReverb` for each case: the IR length and, per channel, all samples (short IRs) or
 *  the first and last 1024 as f32 bits, and an FNV-1a hash of the channel. */
function reverbIrCases(): object[] {
  const previous = getAudioSeed();
  try {
    return REVERB_IR_CASES.map((c) => {
      setAudioSeed(undefined);
      setAudioSeed(c.seed);
      let ir: { length: number; getChannelData(i: number): Float32Array } | undefined;
      reverbGen.generateReverb(
        {
          audioContext: reverbContext(),
          numChannels: 2,
          decayTime: c.d,
          fadeInTime: c.fade,
          lpFreqStart: c.lp,
          lpFreqEnd: c.dim,
        },
        (b: typeof ir) => {
          ir = b;
        },
      );
      if (!ir) throw new Error('generateReverb did not call back synchronously');
      return {
        seed: c.seed,
        d: encodeValue(c.d),
        fade: encodeValue(c.fade),
        lp: encodeValue(c.lp),
        dim: encodeValue(c.dim),
        length: ir.length,
        channels: [0, 1].map((i) => reverbChannel(ir!.getChannelData(i))),
      };
    });
  } finally {
    setAudioSeed(undefined);
    setAudioSeed(previous);
  }
}

// ---- duck-gain-at

/** `offlineDuck` calls for the sequence case: first duck, one after a completed recovery (a
 *  gap), one inside that recovery, one after it. */
export const DUCK_SEQUENCE: { t: number; onset: number; attack: number; depth: number }[] = [
  { t: 0.5, onset: 0, attack: 0.3, depth: 0.75 },
  { t: 1.0, onset: 0.02, attack: 0.05, depth: 2 },
  { t: 1.05, onset: 0.01, attack: 0.2, depth: 0.1 },
  { t: 1.5, onset: 0, attack: 0.1, depth: 1 },
];

/** Hand-made `gainAt` inputs: no points, a held point, a ramp (seen inside, at its start, at its
 *  end), a ramp whose end is not after the previous event, a ramp as the first point, two
 *  stacked ducks after the `time < t` filter, the 0.01 floor and a negative onset. */
const DUCK_GAIN_AT_CASES: { name: string; initial: number; points: GainPoint[] }[] = [
  { name: 'no points', initial: 1, points: [] },
  { name: 'held point', initial: 1, points: [{ time: 0.5, value: 0.3, ramp: false }] },
  {
    name: 'ramp',
    initial: 1,
    points: [
      { time: 0.2, value: 1, ramp: false },
      { time: 0.5, value: 0.1, ramp: true },
    ],
  },
  {
    name: 'ramp end not after the previous event',
    initial: 1,
    points: [
      { time: 0.3, value: 0.5, ramp: false },
      { time: 0.3, value: 0.2, ramp: true },
      { time: 0.6, value: 1, ramp: true },
    ],
  },
  { name: 'ramp first', initial: 0.75, points: [{ time: 0.5, value: 0.1, ramp: true }] },
  {
    name: 'two stacked ducks',
    initial: 1,
    points: [
      { time: 0.25, value: 1, ramp: false },
      { time: 0.25, value: 0.10557280900008414, ramp: true },
      { time: 0.4, value: 0.24527311878791906, ramp: false },
      { time: 0.43, value: 0.24527311878791906, ramp: true },
      { time: 0.53, value: 1, ramp: true },
    ],
  },
  {
    name: 'floor 0.01',
    initial: 1,
    points: [
      { time: 0.1, value: 1, ramp: false },
      { time: 0.12, value: 0.01, ramp: true },
      { time: 0.17, value: 1, ramp: true },
    ],
  },
  {
    name: 'negative onset',
    initial: 1,
    points: [
      { time: 0.4, value: 1, ramp: false },
      { time: 0.3, value: 0.5, ramp: true },
      { time: 0.5, value: 1, ramp: true },
    ],
  },
];

const DUCK_XS = 64;

/** The x values of a case: 64 evenly spaced over the points' span (padded by 0.1 s on each
 *  side; 0 to 1 without points), then each point time and its two neighbouring doubles. */
function duckXs(points: readonly GainPoint[]): number[] {
  const times = points.map((p) => p.time);
  const lo = times.length ? Math.min(...times) - 0.1 : 0;
  const hi = times.length ? Math.max(...times) + 0.1 : 1;
  const xs = Array.from({ length: DUCK_XS }, (_, i) => lo + ((hi - lo) * i) / (DUCK_XS - 1));
  for (const t of times) xs.push(nextDown(t), t, nextUp(t));
  return xs;
}

function duckCase(name: string, initial: number, points: readonly GainPoint[]): object {
  return {
    name,
    initial: encodeValue(initial),
    points: points.map((p) => ({
      time: encodeValue(p.time),
      value: encodeValue(p.value),
      ramp: p.ramp,
    })),
    samples: duckXs(points).map((x) => {
      const value = gainAt(initial, points, x);
      return {
        x: encodeValue(x),
        xBits: f64Bits(x),
        value: encodeValue(value),
        bits: f64Bits(value),
      };
    }),
  };
}

/** Runs `offlineDuck` through `DUCK_SEQUENCE` on a recording fake gain param (intrinsic 1, with
 *  `cancelAndHoldAtTime`, as in Chromium). Returns the recorded calls and the points after each
 *  duck, rebuilt as `offlineDuck` builds them (`held` from the real `gainAt`). */
function duckSequence(): { calls: object[]; steps: GainPoint[][] } {
  const calls: object[] = [];
  const record =
    (method: string) =>
    (...args: number[]) =>
      calls.push({ method, args: args.map((a) => ({ value: encodeValue(a), bits: f64Bits(a) })) });
  const gain = {
    value: 1,
    cancelAndHoldAtTime: record('cancelAndHoldAtTime'),
    cancelScheduledValues: record('cancelScheduledValues'),
    setValueAtTime: record('setValueAtTime'),
    exponentialRampToValueAtTime: record('exponentialRampToValueAtTime'),
  };
  const orbit = { output: { gain } };
  const steps: GainPoint[][] = [];
  let points: GainPoint[] = [];
  for (const d of DUCK_SEQUENCE) {
    offlineDuck.call(orbit, d.t, d.onset, d.attack, d.depth);
    const held = gainAt(1, points, d.t);
    const ducked = Math.min(Math.max(1 - Math.sqrt(d.depth), 0.01), held);
    const a = Math.max(d.attack, 0.002);
    points = [
      ...points.filter((p) => p.time < d.t),
      { time: d.t, value: held, ramp: false },
      { time: d.t + d.onset, value: ducked, ramp: true },
      { time: d.t + d.onset + a, value: 1, ramp: true },
    ];
    steps.push(points);
  }
  return { calls, steps };
}

function duckGainAtFile(): Record<string, unknown> {
  const { calls, steps } = duckSequence();
  const cases = [
    ...DUCK_GAIN_AT_CASES.map((c) => duckCase(c.name, c.initial, c.points)),
    ...steps.map((points, i) => duckCase(`sequence after duck ${i + 1}`, 1, points)),
  ];
  return {
    cases,
    sequence: DUCK_SEQUENCE.map((d) => ({
      t: encodeValue(d.t),
      onset: encodeValue(d.onset),
      attack: encodeValue(d.attack),
      depth: encodeValue(d.depth),
    })),
    calls,
  };
}

/** Every vector file the generator owns, file name -> text. Deterministic; restores the audio
 *  seed it found (the occurrence state of `rng.mjs` is reset as a seed change does). */
export function generateVectors(): Map<string, string> {
  const files = new Map<string, string>();
  files.set('js-number-to-string.json', fileText({ cases: numberToStringCases() }));
  files.set('js-hash-seed.json', fileText({ cases: hashSeedCases() }));
  files.set('js-mulberry32.json', fileText(mulberryCases()));
  files.set(
    'js-stable.json',
    fileText({
      cases: STABLE_VALUES.map((v) => ({ value: encodeValue(v), string: stableMirror(v) })),
    }),
  );
  files.set('js-voice-seed.json', fileText({ cases: voiceSeedCases() }));
  files.set('adsr.json', fileText(adsrCases()));
  files.set('frequency.json', fileText({ cases: frequencyCases() }));
  files.set('waveform-n.json', fileText({ cases: waveformNCases() }));
  for (const type of ['white', 'pink', 'brown', 'crackle'] as const) {
    files.set(`noise-${type}.json`, fileText({ cases: noiseCases(type) }));
  }
  files.set('reverb-ir.json', fileText({ cases: reverbIrCases() }));
  files.set('duck-gain-at.json', fileText(duckGainAtFile()));
  return files;
}

/** `bun run engine:vectors [--write]`; `generate` is injectable so tests reuse one generation. */
export function main(
  argv: string[],
  root: string,
  log: (line: string) => void = console.log,
  generate: () => Map<string, string> = generateVectors,
): number {
  return runVectors({ dir: VECTORS_DIR, command: 'engine:vectors' }, argv, root, log, generate);
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2), resolve(import.meta.dir, '../..')));
}

import { describe, expect, test } from 'bun:test';
import {
  ampToDb,
  BANDS,
  bandEnvelopeDb,
  biquadBandpass,
  clipFraction,
  ENV_FLOOR,
  firstAudibleSec,
  median,
  mixToMono,
  nearestRank,
  onsetEnvelope,
  peak,
  powerToDb,
  ratioDb,
  rms,
  rmsWindows,
  round,
  sortedAsc,
} from '../src/lib/render/dsp';
import { clickTrain, concat, silence, sine, SR } from './helpers/render-signals';

/** RMS after skipping a settling margin. */
const settledRms = (x: Float32Array, skipSec = 0.1) => rms(x.subarray(Math.round(skipSec * SR)));
const band = (name: string) => BANDS.find((b) => b.name === name)!;
const db = (r: number) => 20 * Math.log10(r);

describe('levels', () => {
  test('peak and rms of a 0.5 sine', () => {
    const x = sine(1000, 0.5, 1);
    expect(peak(x)).toBeCloseTo(0.5, 4);
    expect(Math.abs(rms(x) - 0.35355)).toBeLessThan(1e-4);
  });

  test('empty inputs', () => {
    const e = new Float32Array(0);
    expect(peak(e)).toBe(0);
    expect(rms(e)).toBe(0);
    expect(clipFraction(e)).toBe(0);
  });

  test('peak uses absolute value', () => {
    expect(peak(new Float32Array([0.1, -0.9, 0.5]))).toBeCloseTo(0.9, 6);
  });

  test('clipFraction counts strictly greater than 1', () => {
    expect(clipFraction(new Float32Array([1.0, 1.1, -1.2, 0.5]))).toBe(0.5);
  });
});

describe('firstAudibleSec', () => {
  test('silence then a -20 dBFS tone', () => {
    const x = concat(silence(0.5), sine(1000, 0.1, 0.5));
    expect(Math.abs(firstAudibleSec(x, SR) - 0.5)).toBeLessThanOrEqual(1 / SR + 1e-12);
  });

  test('silence is Infinity', () => {
    expect(firstAudibleSec(silence(1), SR)).toBe(Infinity);
  });

  test('threshold boundary is strict', () => {
    const x = new Float32Array([0.01, 0.0101]);
    expect(firstAudibleSec(x, 2, -40)).toBe(0.5);
    expect(firstAudibleSec(new Float32Array([0.5]), 1, -7)).toBe(0);
    expect(firstAudibleSec(new Float32Array([0.5]), 1, 0)).toBe(Infinity);
  });
});

describe('BANDS', () => {
  test('four feature bands', () => {
    expect(BANDS.map((b) => [b.name, b.lo, b.hi])).toEqual([
      ['sub', 20, 130],
      ['low', 130, 500],
      ['mid', 500, 2000],
      ['high', 2000, 16000],
    ]);
  });
});

describe('biquadBandpass', () => {
  for (const name of ['mid', 'high']) {
    test(`${name}: passes the centre, attenuates outside`, () => {
      const { lo, hi } = band(name);
      const pass = (f: number) =>
        settledRms(biquadBandpass(sine(f, 0.5, 1), SR, lo, hi)) / settledRms(sine(f, 0.5, 1));
      expect(pass(Math.sqrt(lo * hi))).toBeGreaterThanOrEqual(0.98);
      expect(pass(lo / 2)).toBeLessThanOrEqual(0.5);
      if (hi * 2 < SR / 2) expect(pass(hi * 2)).toBeLessThanOrEqual(0.5);
    });
  }

  test('DC and near-Nyquist are attenuated more than 20 dB', () => {
    const { lo, hi } = band('mid');
    const dc = new Float32Array(SR).fill(0.5);
    expect(db(settledRms(biquadBandpass(dc, SR, lo, hi)) / 0.5)).toBeLessThan(-20);
    const ny = sine(21000, 0.5, 1);
    expect(db(settledRms(biquadBandpass(ny, SR, lo, hi)) / rms(ny))).toBeLessThan(-20);
  });

  test('output length equals input', () => {
    expect(biquadBandpass(new Float32Array(17), SR, 500, 2000).length).toBe(17);
  });

  test('throws on an invalid band', () => {
    expect(() => biquadBandpass(new Float32Array(4), SR, 2000, SR / 2)).toThrow(RangeError);
    expect(() => biquadBandpass(new Float32Array(4), SR, 0, 100)).toThrow(RangeError);
    expect(() => biquadBandpass(new Float32Array(4), SR, 500, 500)).toThrow(RangeError);
  });
});

describe('rmsWindows', () => {
  test('window count is floor(len / win), tail dropped', () => {
    expect(rmsWindows(new Float32Array(1050), 1000, 0.1).length).toBe(10);
    expect(rmsWindows(new Float32Array(0), 1000, 0.1).length).toBe(0);
  });

  test('values', () => {
    const w = rmsWindows(new Float32Array([1, 1, 0, 0, 3]), 1, 2);
    expect(Array.from(w)).toEqual([1, 0]);
  });

  test('window is at least one frame', () => {
    expect(rmsWindows(new Float32Array(3), 1000, 0).length).toBe(3);
  });
});

describe('bandEnvelopeDb', () => {
  test('-20 dBFS 1 kHz in mid is about -23 dB per window', () => {
    const { lo, hi } = band('mid');
    const env = bandEnvelopeDb(sine(1000, 0.1, 1), SR, lo, hi);
    expect(env.length).toBe(10);
    for (const v of env.subarray(1)) expect(Math.abs(v - -23)).toBeLessThanOrEqual(1);
  });

  test('silence is exactly -100 dB', () => {
    const env = bandEnvelopeDb(silence(0.5), SR, 500, 2000);
    expect(env.length).toBe(5);
    for (const v of env) expect(v).toBe(-100);
    expect(20 * Math.log10(ENV_FLOOR)).toBe(-100);
  });
});

describe('onsetEnvelope', () => {
  test('three clicks peak at round(t/0.01)-1', () => {
    const times = [0.25, 0.75, 1.25];
    const env = onsetEnvelope(clickTrain(times, 1.5), SR);
    expect(env.length).toBe(Math.floor((1.5 * SR) / 441) - 1);
    for (const v of env) expect(v).toBeGreaterThanOrEqual(0);
    for (const t of times) {
      const k = Math.round(t / 0.01) - 1;
      const lo = Math.max(0, k - 3);
      const slice = Array.from(env.subarray(lo, k + 4));
      const at = lo + slice.indexOf(Math.max(...slice));
      expect(Math.abs(at - k)).toBeLessThanOrEqual(1);
      expect(env[at]!).toBeGreaterThan(0.1);
    }
  });

  test('steady sine has near-zero envelope after the first hop', () => {
    const env = onsetEnvelope(sine(1000, 0.5, 1), SR);
    for (const v of env) expect(v).toBeLessThan(1e-3);
  });

  test('short inputs', () => {
    expect(onsetEnvelope(new Float32Array(0), SR).length).toBe(0);
    expect(onsetEnvelope(new Float32Array(441), SR).length).toBe(0);
    expect(onsetEnvelope(new Float32Array(882), SR).length).toBe(1);
  });
});

describe('mixToMono', () => {
  test('averages channels', () => {
    const m = mixToMono([new Float32Array([1, 0, 0.5]), new Float32Array([0, 0, -0.5])]);
    expect(Array.from(m)).toEqual([0.5, 0, 0]);
  });

  test('empty list', () => {
    expect(mixToMono([]).length).toBe(0);
  });

  test('unequal lengths throw', () => {
    expect(() => mixToMono([new Float32Array(2), new Float32Array(3)])).toThrow(RangeError);
  });
});

describe('numeric helpers', () => {
  test('round matches Math.round(x * 10^d) / 10^d bit for bit', () => {
    for (const x of [0, 1.005, -2.345, 0.1234567, 1e-7, 123.4565, -0.5]) {
      expect(round(x, 2)).toBe(Math.round(x * 100) / 100);
      expect(round(x, 3)).toBe(Math.round(x * 1000) / 1000);
      expect(round(x, 6)).toBe(Math.round(x * 1e6) / 1e6);
    }
    expect(round(2.5, 0)).toBe(3);
  });

  test('dB conversions and their floors', () => {
    expect(ampToDb(1, 1e-5)).toBe(0);
    expect(ampToDb(0, 1e-5)).toBeCloseTo(-100, 10);
    expect(ampToDb(0.5, 0)).toBe(20 * Math.log10(0.5));
    expect(ratioDb(1, 1, 1e-12)).toBe(0);
    expect(ratioDb(0, 0, 1e-12)).toBe(0);
    expect(ratioDb(0.1, 1, 0)).toBeCloseTo(-20, 10);
    expect(powerToDb(100)).toBe(20);
  });

  test('nearest-rank quantiles and the median', () => {
    const s = sortedAsc([4, 0, 3, 1, 2]);
    expect(s).toEqual([0, 1, 2, 3, 4]);
    expect([0, 0.5, 0.95, 1].map((p) => nearestRank(s, p))).toEqual([0, 2, 4, 4]);
    expect(nearestRank([7], 0.95)).toBe(7);
    expect(median([])).toBe(0);
    expect(median([3, 1, 2])).toBe(2);
    expect(median([4, 1, 3, 2])).toBe(2.5);
  });
});

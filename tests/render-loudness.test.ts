import { describe, expect, test } from 'bun:test';
import { integratedLufs, kWeightingCoeffs } from '../src/lib/render/loudness';
import { concat, scale, silence, sine } from './helpers/render-signals';

describe('kWeightingCoeffs', () => {
  test('48 kHz reproduces the BS.1770-4 table', () => {
    const { shelf, highpass } = kWeightingCoeffs(48000);
    const near = (got: number[], want: number[]) =>
      got.forEach((g, i) => expect(Math.abs(g - want[i]!)).toBeLessThan(1e-6));
    near(shelf.b, [1.53512485958697, -2.69169618940638, 1.19839281085285]);
    near(shelf.a, [1, -1.69065929318241, 0.73248077421585]);
    near(highpass.b, [1, -2, 1]);
    near(highpass.a, [1, -1.99004745483398, 0.99007225036621]);
  });

  test('throws on a bad rate', () => {
    expect(() => kWeightingCoeffs(0)).toThrow(RangeError);
  });
});

describe('integratedLufs calibration tones', () => {
  for (const sr of [48000, 44100]) {
    test(`${sr} Hz`, () => {
      const tone = sine(1000, 0.1, 10, sr);
      const zero = silence(10, sr);
      expect(Math.abs(integratedLufs([tone, tone], sr) - -20)).toBeLessThanOrEqual(0.1);
      expect(Math.abs(integratedLufs([tone, zero], sr) - -23)).toBeLessThanOrEqual(0.1);
      const full = sine(1000, 1, 10, sr);
      expect(Math.abs(integratedLufs([full], sr) - -3)).toBeLessThanOrEqual(0.1);
    });
  }
});

describe('integratedLufs edges', () => {
  const sr = 44100;

  test('silence, too short, no channels are -Infinity', () => {
    expect(integratedLufs([silence(2, sr), silence(2, sr)], sr)).toBe(-Infinity);
    expect(integratedLufs([sine(1000, 0.5, 0.39, sr)], sr)).toBe(-Infinity);
    expect(integratedLufs([], sr)).toBe(-Infinity);
  });

  test('absolute gate drops very quiet blocks', () => {
    const loud = sine(1000, 0.1, 10, sr);
    const both = concat(loud, sine(1000, Math.pow(10, -85 / 20), 10, sr));
    const alone = integratedLufs([loud, loud], sr);
    // The three blocks straddling the step keep part of the loud tone and pass both gates,
    // which costs about 0.066 dB (mean square 98.5 / 100); without the gate it would be ~ -23.
    expect(Math.abs(integratedLufs([both, both], sr) - alone)).toBeLessThanOrEqual(0.1);
  });

  test('content entirely below -70 LUFS is -Infinity', () => {
    const quiet = sine(1000, Math.pow(10, -75 / 20), 5, sr);
    expect(integratedLufs([quiet, quiet], sr)).toBe(-Infinity);
  });

  test('relative gate drops blocks 10 LU below', () => {
    const x = concat(sine(1000, 0.1, 10, sr), sine(1000, Math.pow(10, -38 / 20), 10, sr));
    expect(Math.abs(integratedLufs([x, x], sr) - -20)).toBeLessThanOrEqual(0.1);
  });

  test('DC is removed by the high-pass', () => {
    // A 0.5 offset faded in over 2 s (raised cosine), so no step transient rings the filter.
    const fade = 2 * sr;
    const dc = new Float32Array(10 * sr).map((_, i) =>
      i < fade ? 0.25 * (1 - Math.cos((Math.PI * i) / fade)) : 0.5,
    );
    expect(integratedLufs([dc], sr)).toBeLessThan(-60);
  });

  test('linear in gain', () => {
    const x = sine(440, 0.05, 5, sr);
    const y = scale(x, 2);
    const d = integratedLufs([y, y], sr) - integratedLufs([x, x], sr);
    expect(Math.abs(d - 6.0206)).toBeLessThanOrEqual(0.05);
  });

  test('unequal lengths and bad rates throw', () => {
    expect(() => integratedLufs([silence(1, sr), silence(2, sr)], sr)).toThrow(RangeError);
    expect(() => integratedLufs([silence(1, sr)], 0)).toThrow(RangeError);
  });

  test('NaN propagates', () => {
    const x = sine(1000, 0.1, 2, sr);
    x[1000] = NaN;
    expect(integratedLufs([x], sr)).toBeNaN();
  });
});

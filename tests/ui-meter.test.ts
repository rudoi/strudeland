import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  CLIP_HOLD_MS,
  CLIP_LINEAR,
  createMeterModel,
  levelFraction,
  PEAK_FALL_PER_S,
} from '../src/components/ui/meter';

describe('levelFraction', () => {
  test('0 at -60 dBFS or below, 1 at full scale and above', () => {
    expect(levelFraction(0.001)).toBe(0); // -60 dBFS
    expect(levelFraction(0.0001)).toBe(0);
    expect(levelFraction(1)).toBe(1);
    expect(levelFraction(2)).toBe(1);
  });

  test('linear in dB between', () => {
    expect(levelFraction(Math.pow(10, -30 / 20))).toBeCloseTo(0.5, 12);
    expect(levelFraction(Math.pow(10, -6 / 20))).toBeCloseTo(0.9, 12);
  });

  test('monotonic', () => {
    let prev = -1;
    for (let i = 0; i <= 1000; i++) {
      const f = levelFraction(i / 1000);
      expect(f).toBeGreaterThanOrEqual(prev);
      prev = f;
    }
  });

  test('zero, negative, NaN and infinities give 0', () => {
    expect(levelFraction(0)).toBe(0);
    expect(levelFraction(-0.5)).toBe(0);
    expect(levelFraction(NaN)).toBe(0);
    expect(levelFraction(Infinity)).toBe(0);
    expect(levelFraction(-Infinity)).toBe(0);
  });
});

describe('createMeterModel', () => {
  const clock = () => {
    let t = 0;
    return { now: () => t, set: (ms: number) => (t = ms) };
  };

  test('rms and peak are the level fractions on the first push', () => {
    const c = clock();
    const m = createMeterModel(c.now);
    expect(m.push({ peak: 1, rms: Math.pow(10, -30 / 20) })).toEqual({
      rms: expect.closeTo(0.5, 12) as unknown as number,
      peak: 1,
      clip: true,
    });
  });

  test('the peak marker falls at PEAK_FALL_PER_S', () => {
    const c = clock();
    const m = createMeterModel(c.now);
    m.push({ peak: Math.pow(10, -6 / 20), rms: 0 }); // 0.9
    c.set(500);
    expect(m.push({ peak: 0, rms: 0 }).peak).toBeCloseTo(0.9 - PEAK_FALL_PER_S * 0.5, 12);
    c.set(1000);
    expect(m.push({ peak: 0, rms: 0 }).peak).toBeCloseTo(0.9 - PEAK_FALL_PER_S, 12);
    c.set(5000);
    expect(m.push({ peak: 0, rms: 0 }).peak).toBe(0);
  });

  test('a new peak above the decayed marker replaces it', () => {
    const c = clock();
    const m = createMeterModel(c.now);
    m.push({ peak: Math.pow(10, -30 / 20), rms: 0 }); // 0.5
    c.set(100);
    expect(m.push({ peak: 1, rms: 0 }).peak).toBe(1);
  });

  test('clip latches for CLIP_HOLD_MS after a peak at or above CLIP_LINEAR', () => {
    const c = clock();
    const m = createMeterModel(c.now);
    expect(m.push({ peak: 0.98, rms: 0 }).clip).toBe(false);
    c.set(10);
    expect(m.push({ peak: CLIP_LINEAR, rms: 0 }).clip).toBe(true);
    c.set(10 + CLIP_HOLD_MS - 1);
    expect(m.push({ peak: 0, rms: 0 }).clip).toBe(true);
    c.set(10 + CLIP_HOLD_MS);
    expect(m.push({ peak: 0, rms: 0 }).clip).toBe(false);
  });

  test('NaN levels read as silence and never clip', () => {
    const m = createMeterModel(() => 0);
    expect(m.push({ peak: NaN, rms: NaN })).toEqual({ rms: 0, peak: 0, clip: false });
  });

  test('reset zeroes the marker and the clip', () => {
    const c = clock();
    const m = createMeterModel(c.now);
    m.push({ peak: 1, rms: 1 });
    m.reset();
    c.set(1);
    expect(m.push({ peak: 0, rms: 0 })).toEqual({ rms: 0, peak: 0, clip: false });
  });
});

describe('Meter thin variant', () => {
  const src = readFileSync(new URL('../src/components/ui/Meter.svelte', import.meta.url), 'utf8');

  test('thin is opt-in, so the input meter is unchanged', () => {
    expect(src).toContain('thin = false');
    expect(src).toContain("class={['meter', { thin }, className]}");
  });

  test('thin is one space-1 tall with no inset well', () => {
    const rule = /\.meter\.thin \{([^}]*)\}/.exec(src)?.[1] ?? '';
    expect(rule).toContain('height: var(--space-1)');
    expect(rule).toContain('box-shadow: none');
  });
});

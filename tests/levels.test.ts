import { expect, test } from 'bun:test';
import {
  createLevelHold,
  createMasterMeter,
  createPeakReadout,
  formatPeakDb,
  LEVEL_FRESH_MS,
  mergeFrames,
  PEAK_HOLD_MS,
  type LevelFrame,
} from '../src/lib/levels/levels';
import { createMeterModel } from '../src/components/ui/meter';

const f = (p: [number, number], o: [number, number] = [0, 0], limitDb = 0): LevelFrame => ({
  peak: p,
  rms: [p[0] / 2, p[1] / 2],
  overs: o,
  limitDb,
});

test('mergeFrames: max peak and rms, summed overs, lowest limitDb', () => {
  const m = mergeFrames(f([0.2, 0.9], [1, 0], -1), f([0.5, 0.1], [2, 3], -4));
  expect(m).toEqual({ peak: [0.5, 0.9], rms: [0.25, 0.45], overs: [3, 3], limitDb: -4 });
});

test('createLevelHold: null until a frame, max peak, summed overs, lowest limitDb, reset', () => {
  const h = createLevelHold();
  expect(h.snapshot()).toBeNull();
  h.push(f([0.2, 0.9], [1, 1], -1));
  h.push(f([0.5, 0.1], [2, 0], -4));
  h.push(f([0.1, 0.1], [0, 0], 0));
  expect(h.snapshot()).toEqual({ peak: [0.5, 0.9], overs: 4, limitDb: -4 });
  h.reset();
  expect(h.snapshot()).toBeNull();
  h.push(f([0.3, 0.3]));
  expect(h.snapshot()).toEqual({ peak: [0.3, 0.3], overs: 0, limitDb: 0 });
});

test('createPeakReadout: holds, replaced when higher or after PEAK_HOLD_MS, null on 0', () => {
  let t = 0;
  const r = createPeakReadout(() => t);
  expect(r.push(f([0, 0]))).toBeNull();
  expect(r.push(f([0.5, 0.1]))).toBeCloseTo(-6.02, 1);
  t = 500;
  expect(r.push(f([0.1, 0.1]))).toBeCloseTo(-6.02, 1); // lower: held
  expect(r.push(f([0, 0]))).toBeCloseTo(-6.02, 1); // silence while held
  expect(r.push(f([0.1, 0.9]))).toBeCloseTo(-0.915, 2); // higher: replaces
  t = 500 + PEAK_HOLD_MS + 1;
  expect(r.push(f([0.1, 0.1]))).toBeCloseTo(-20, 1); // expired: lower replaces
  t += PEAK_HOLD_MS + 1;
  expect(r.push(f([0, 0]))).toBeNull();
});

test('formatPeakDb', () => {
  expect(formatPeakDb(null)).toBe('-');
  expect(formatPeakDb(-3.24)).toBe('-3.2db');
  expect(formatPeakDb(2.14)).toBe('+2.1db');
  expect(formatPeakDb(0)).toBe('+0.0db');
});

function meter() {
  let t = 0;
  const m = createMasterMeter(
    () => t,
    () => createMeterModel(() => t),
  );
  return { m, at: (x: number) => void (t = x) };
}

test('master meter: a frame is active; silence is active with an empty fill and readout -', () => {
  const { m } = meter();
  const v = m.step(f([0.5, 0.25]));
  expect(v.active).toBe(true);
  expect(v.readout).toBe('-6.0db');
  expect(v.channels[0].rms).toBeGreaterThan(0);
  const z = m.step(f([0, 0]));
  expect(z.active).toBe(true);
  expect(z.channels[0].rms).toBe(0);
  expect(z.readout).toBe('-6.0db'); // still held
});

test('master meter: silence from the start is active, readout -', () => {
  const { m } = meter();
  const z = m.step(f([0, 0]));
  expect(z).toMatchObject({ active: true, readout: '-', clip: false, limiting: false });
});

test('master meter: no frame holds the fill while fresh, then goes inactive and resets', () => {
  const { m, at } = meter();
  m.step(f([0.5, 0.5]));
  at(LEVEL_FRESH_MS - 1);
  const held = m.step(null);
  expect(held.active).toBe(true);
  expect(held.channels[0].rms).toBeGreaterThan(0);
  at(LEVEL_FRESH_MS + 1);
  const stale = m.step(null);
  expect(stale).toMatchObject({ active: false, readout: '-', clip: false });
  expect(stale.channels[0].rms).toBe(0);
  // Nothing was ever seen after the reset: still inactive.
  expect(m.step(null).active).toBe(false);
});

test('master meter: a null clock call (not live) is inactive at once', () => {
  const { m } = meter();
  m.step(f([0.5, 0.5]));
  expect(m.step(null, false).active).toBe(false);
});

test('master meter: clip, limiting, clearClip, a non-finite peak', () => {
  const { m } = meter();
  const v = m.step(f([1, 0.2], [3, 0], -2));
  expect(v.clip).toBe(true);
  expect(v.limiting).toBe(true);
  expect(m.step(f([0.1, 0.1], [0, 0], -0.05)).limiting).toBe(false);
  m.clearClip();
  expect(m.step(f([0.1, 0.1])).clip).toBe(false);
  const n = m.step(f([NaN, 0.1]));
  expect(n.clip).toBe(true);
  expect(n.channels[0].peak).toBe(1);
});

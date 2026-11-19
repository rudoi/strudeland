import { describe, expect, test } from 'bun:test';
import { AFTERGLOW, createGain, restLine, tracePoints, triggerIndex } from '../src/lib/scope/trace';
import { periodSine as sine } from './helpers/signals';

/** `tracePoints` into a fresh array of exactly `count` pairs, read back as points. */
function points(
  buf: ArrayLike<number>,
  start: number,
  count: number,
  width: number,
  height: number,
  gain: number,
): { x: number; y: number }[] {
  const out = new Float32Array(Math.max(0, count) * 2);
  const n = tracePoints(out, buf, start, count, width, height, gain);
  return Array.from({ length: n }, (_, i) => ({ x: out[2 * i]!, y: out[2 * i + 1]! }));
}

describe('triggerIndex', () => {
  test('a sine starting below zero triggers at its first rising zero crossing', () => {
    // Starts at the trough (phase -pi/2): rises through 0 a quarter period in.
    const buf = sine(64, 16, -Math.PI / 2);
    const i = triggerIndex(buf);
    expect(i).toBeGreaterThan(0);
    expect(buf[i - 1]).toBeLessThan(0);
    expect(buf[i]).toBeGreaterThanOrEqual(0);
    expect(i).toBe(4);
  });

  test('silence and DC give 0', () => {
    expect(triggerIndex(new Float32Array(64))).toBe(0);
    expect(triggerIndex(new Float32Array(64).fill(0.5))).toBe(0);
    expect(triggerIndex(new Float32Array(64).fill(-0.5))).toBe(0);
  });

  test('a falling edge does not trigger', () => {
    const buf = Float32Array.from({ length: 32 }, (_, i) => 1 - i / 16);
    expect(triggerIndex(buf)).toBe(0);
  });

  test('only searches the first half of the buffer', () => {
    const buf = new Float32Array(16).fill(-1);
    buf.fill(1, 12);
    expect(triggerIndex(buf)).toBe(0);
    buf.fill(1, 8);
    expect(triggerIndex(buf)).toBe(8);
  });

  test('honours the threshold', () => {
    const buf = Float32Array.from([0, 0.2, 0.4, 0.6, 0.8, 1, 1, 1]);
    expect(triggerIndex(buf, 0.5)).toBe(3);
  });

  test('an empty buffer gives 0', () => {
    expect(triggerIndex(new Float32Array(0))).toBe(0);
  });
});

describe('tracePoints', () => {
  test('spreads count points across the width', () => {
    const pts = points(new Float32Array(8), 0, 5, 100, 20, 1);
    expect(pts.map((p) => p.x)).toEqual([0, 25, 50, 75, 100]);
    expect(pts.every((p) => p.y === 10)).toBe(true);
  });

  test('maps +1 and -1 to a 1px margin inside the box', () => {
    const pts = points(Float32Array.from([1, -1, 0]), 0, 3, 10, 20, 1);
    expect(pts.map((p) => p.y)).toEqual([1, 19, 10]);
  });

  test('clamps: a sample of 5 stays inside the box', () => {
    const pts = points(Float32Array.from([5, -5, 0.5]), 0, 3, 10, 20, 2);
    for (const p of pts) {
      expect(p.y).toBeGreaterThanOrEqual(1);
      expect(p.y).toBeLessThanOrEqual(19);
    }
    expect(pts[0]!.y).toBe(1);
    expect(pts[1]!.y).toBe(19);
    expect(pts[2]!.y).toBe(1); // 0.5 * 2 = 1
  });

  test('reads from start and treats samples past the end as silence', () => {
    const pts = points(Float32Array.from([0, 0, 1]), 2, 3, 10, 20, 1);
    expect(pts.map((p) => p.y)).toEqual([1, 10, 10]);
  });

  test('a non-finite sample draws as silence', () => {
    const pts = points(Float32Array.from([NaN, Infinity]), 0, 2, 10, 20, 1);
    expect(pts.map((p) => p.y)).toEqual([10, 10]);
  });

  test('one point sits at x 0; zero points is empty', () => {
    expect(points(new Float32Array(4), 0, 1, 10, 20, 1)).toEqual([{ x: 0, y: 10 }]);
    expect(points(new Float32Array(4), 0, 0, 10, 20, 1)).toEqual([]);
  });

  test('writes interleaved x, y into the given array and returns the count', () => {
    const out = new Float32Array(8).fill(-7);
    expect(tracePoints(out, Float32Array.from([1, -1, 0]), 0, 3, 10, 20, 1)).toBe(3);
    expect([...out]).toEqual([0, 1, 5, 19, 10, 10, -7, -7]);
  });

  test('caps the count at the pairs the array holds, keeping the spacing of count', () => {
    const out = new Float32Array(4);
    expect(tracePoints(out, new Float32Array(8), 0, 5, 100, 20, 1)).toBe(2);
    expect([...out]).toEqual([0, 10, 25, 10]);
  });

  test('reusing the array overwrites the previous points', () => {
    const out = new Float32Array(4);
    tracePoints(out, Float32Array.from([1, 1]), 0, 2, 10, 20, 1);
    tracePoints(out, Float32Array.from([-1, -1]), 0, 2, 10, 20, 1);
    expect([...out]).toEqual([0, 19, 10, 19]);
  });
});

describe('restLine', () => {
  test('is the horizontal centre line', () => {
    expect(restLine(176, 28)).toEqual({ x1: 0, x2: 176, y: 14 });
    expect(restLine(40, 15)).toEqual({ x1: 0, x2: 40, y: 7.5 });
  });
});

describe('createGain', () => {
  test('silence gives 1', () => {
    const g = createGain();
    for (let i = 0; i < 5; i++) expect(g.next(new Float32Array(64))).toBe(1);
  });

  test('attack is fast: a loud signal is held to 0.8 of the box within a few frames', () => {
    const g = createGain();
    const loud = sine(256, 32, 0, 1);
    const first = g.next(loud); // env 0.5
    expect(first).toBeCloseTo(1.6, 5);
    let gain = first;
    for (let i = 0; i < 20; i++) gain = g.next(loud);
    expect(gain).toBeCloseTo(0.8, 3);
  });

  test('a quiet signal is lifted, at most 8x', () => {
    const g = createGain();
    const quiet = sine(256, 32, 0, 0.01);
    let gain = 0;
    for (let i = 0; i < 50; i++) gain = g.next(quiet);
    expect(gain).toBe(8);
  });

  test('a very loud signal is held to at least 0.5', () => {
    const g = createGain();
    let gain = 0;
    for (let i = 0; i < 50; i++) gain = g.next(new Float32Array(16).fill(-4));
    expect(gain).toBe(0.5);
  });

  test('release is slow: the gain rises gradually after a loud signal stops', () => {
    const g = createGain();
    const loud = sine(256, 32, 0, 1);
    for (let i = 0; i < 30; i++) g.next(loud);
    const quiet = sine(256, 32, 0, 0.1);
    const a = g.next(quiet);
    const b = g.next(quiet);
    expect(a).toBeLessThan(0.85);
    expect(b).toBeGreaterThan(a);
    expect(b).toBeLessThan(1);
  });

  test('each gain has its own follower', () => {
    const a = createGain();
    const b = createGain();
    a.next(new Float32Array(8).fill(1));
    expect(b.next(new Float32Array(8))).toBe(1);
  });
});

test('AFTERGLOW', () => {
  expect(AFTERGLOW).toBe(0.42);
});

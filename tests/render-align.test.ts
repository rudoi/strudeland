import { describe, expect, test } from 'bun:test';
import { alignOnsets, MIN_ALIGN_CORRELATION } from '../src/lib/render/align';
import { delayed, noiseSeeded } from './helpers/render-signals';

/** A sparse onset envelope: impulses at irregular hops. */
function spiky(len: number, hops: number[]): Float64Array {
  const x = new Float64Array(len);
  for (const h of hops) x[h] = 1;
  return x;
}
const A = spiky(400, [30, 47, 90, 131, 160, 222, 250, 301, 333]);

describe('alignOnsets', () => {
  test('b delayed by 25 hops', () => {
    const r = alignOnsets(A, delayed(A, 25));
    expect(r.offsetSec).toBeCloseTo(0.25, 9);
    expect(r.correlation).toBeGreaterThan(0.95);
    expect(r.trusted).toBe(true);
    expect(r.note).toBeUndefined();
  });

  test('b advanced by 25 hops', () => {
    const r = alignOnsets(delayed(A, 25), A);
    expect(r.offsetSec).toBeCloseTo(-0.25, 9);
    expect(r.trusted).toBe(true);
  });

  test('identical envelopes', () => {
    const r = alignOnsets(A, A);
    expect(r.offsetSec).toBe(0);
    expect(r.correlation).toBeCloseTo(1, 9);
    expect(r.trusted).toBe(true);
  });

  test('level-scaled copy still aligns', () => {
    const b = delayed(A, 10).map((v) => v * 0.1);
    const r = alignOnsets(A, b);
    expect(r.offsetSec).toBeCloseTo(0.1, 9);
    expect(r.trusted).toBe(true);
  });

  test('hopSec scales the offset', () => {
    expect(alignOnsets(A, delayed(A, 4), { hopSec: 0.02 }).offsetSec).toBeCloseTo(0.08, 9);
  });

  test('constant envelopes have no onset energy', () => {
    const r = alignOnsets(new Float64Array(100).fill(0.3), A);
    expect(r).toEqual({ offsetSec: 0, correlation: 0, trusted: false, note: 'no onset energy' });
  });

  test('unrelated noise is untrusted but keeps the estimate', () => {
    const a = Array.from(noiseSeeded(1, 600), Math.abs);
    const b = Array.from(noiseSeeded(2, 600), Math.abs);
    const r = alignOnsets(a, b);
    expect(r.correlation).toBeLessThan(MIN_ALIGN_CORRELATION);
    expect(r.trusted).toBe(false);
    expect(r.note).toContain('estimate');
  });

  test('a lag beyond maxLagSec is not found', () => {
    const r = alignOnsets(A, delayed(A, 60), { maxLagSec: 0.3 });
    expect(Math.abs(r.offsetSec)).toBeLessThanOrEqual(0.3 + 1e-9);
    expect(r.offsetSec).not.toBeCloseTo(0.6, 6);
  });

  test('minCorrelation option', () => {
    expect(alignOnsets(A, A, { minCorrelation: 1.5 }).trusted).toBe(false);
  });

  test('overlap shorter than 8 hops is untrusted at offset 0', () => {
    const r = alignOnsets([0, 1, 0, 0, 1], [0, 0, 1, 0, 0]);
    expect(r.offsetSec).toBe(0);
    expect(r.trusted).toBe(false);
  });

  test('empty arrays do not throw', () => {
    const r = alignOnsets([], []);
    expect(r.trusted).toBe(false);
    expect(r.offsetSec).toBe(0);
  });
});

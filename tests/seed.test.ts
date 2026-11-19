import { expect, test } from 'bun:test';
import { applySeed } from '../src/lib/strudel/seed';

const fakePattern = (log: number[]) => {
  const p = { seed: (n: number) => (log.push(n), { ...p, seeded: n }) };
  return p;
};

test('no seed returns the same pattern', () => {
  const log: number[] = [];
  const p = fakePattern(log);
  expect(applySeed(p, undefined)).toBe(p);
  expect(log).toEqual([]);
});

test('a seed calls pattern.seed with it, including 0 and negatives', () => {
  const log: number[] = [];
  const p = fakePattern(log);
  expect((applySeed(p, 5) as any).seeded).toBe(5);
  expect((applySeed(p, 0) as any).seeded).toBe(0);
  expect((applySeed(p, -2.5) as any).seeded).toBe(-2.5);
  expect(log).toEqual([5, 0, -2.5]);
});

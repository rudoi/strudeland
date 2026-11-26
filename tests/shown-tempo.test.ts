import { expect, test } from 'bun:test';
import { nextShownTempo } from '../src/lib/strudel/shown-tempo';

const meter = { num: 3, den: 8 };
const current = { cps: 4 / 3, bpm: 120, meter };

test('an unchanged sample keeps the current object', () => {
  expect(nextShownTempo(current, { cps: 4 / 3, bpm: 120, meter })).toBe(current);
});

test('a sample with any field changed replaces it', () => {
  for (const sampled of [
    { cps: 1.5, bpm: 120, meter },
    { cps: 4 / 3, bpm: 121, meter },
    { cps: 4 / 3, bpm: 120, meter: { num: 2, den: 4 } },
  ]) {
    expect(nextShownTempo(current, sampled)).toBe(sampled);
  }
});

test('the meter compares by identity', () => {
  const sampled = { cps: 4 / 3, bpm: 120, meter: { num: 3, den: 8 } };
  expect(nextShownTempo(current, sampled)).toBe(sampled);
});

import { expect, test } from 'bun:test';
import { bjorklund, pure } from '@strudel/core';

// Complete baseline sequences include the starting phase and the order of unequal gaps.
test.each<[number, number, string]>([
  [0, 0, ''],
  [0, 4, '0000'],
  [4, 4, '1111'],
  [-4, 4, '0000'],
  [1, 4, '1000'],
  [4, 5, '11110'],
  [4, 8, '10101010'],
  [2, 5, '10100'],
  [3, 5, '10101'],
  [3, 8, '10010010'],
  [5, 12, '100101001010'],
  [7, 12, '101101011010'],
  [-3, 8, '01101101'],
])('bjorklund(%i, %i) yields %s', (pulses, steps, expected) => {
  expect(bjorklund(pulses, steps)).toEqual(Array.from(expected, Number));
});

// Read the whole event spans, including legato notes that cross the query boundary.
function spans(
  pattern: {
    queryArc(
      begin: number,
      end: number,
    ): { whole: Record<'begin' | 'end', { valueOf(): number }> }[];
  },
  stepsPerCycle: number,
  cycles = 1,
) {
  return pattern
    .queryArc(0, cycles)
    .map(({ whole }): [number, number] => [
      whole.begin.valueOf() * stepsPerCycle,
      whole.end.valueOf() * stepsPerCycle,
    ])
    .sort(([a], [b]) => a - b)
    .map(([begin, end]) => `${begin}..${end}`)
    .join(' ');
}

test('euclid and bjork preserve silent, rotated and inverted steps', () => {
  const pat = pure('bd');
  expect(spans(pat.euclid(0, 8), 8)).toBe('');
  expect(spans(pat.euclid(3, 0), 8)).toBe('');
  expect(spans(pat.euclid(-3, 8), 8)).toBe('1..2 2..3 4..5 5..6 7..8');
  expect(spans(pat.euclidRot(3, 8, 1), 8)).toBe('1..2 4..5 7..8');
  expect(spans(pat.euclidRot(3, 8, -1), 8)).toBe('2..3 5..6 7..8');
  expect(spans(pat.bjork([3, 8, 1]), 8)).toBe('1..2 4..5 7..8');
  expect(spans(pat.bjork(3), 3)).toBe('0..1 1..2 2..3');
});

test('negative steps select legato with the original alignment', () => {
  const pat = pure('bd');
  expect(spans(pat.euclid(-3, -8), 8)).toBe('-1..1 1..2 2..4 4..5 5..7 7..9');
  expect(spans(pat.euclidRot(3, -8, 1), 8)).toBe('-1..1 1..4 4..7 7..9');
  expect(spans(pat.euclidLegatoRot(3, 8, 1), 8)).toBe('-1..1 1..4 4..7 7..9');
  expect(spans(pat.euclidLegato(3, -8), 8)).toBe('0..1 3..4 6..7');
});

test('pulse counts exceeding the step count span multiple cycles', () => {
  const pat = pure('bd');
  expect(spans(pat.euclid(5, 3), 3, 2)).toBe('0..1 1..2 2..3 3..4 4..5');
  expect(spans(pat.euclid(-5, 3), 3, 2)).toBe('5..6');
  expect(spans(pat.euclid(5, -3), 3, 2)).toBe('0..1 1..2 2..3 3..4 4..6');
});

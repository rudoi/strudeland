import { describe, expect, test } from 'bun:test';
import {
  adjacentStep,
  barAtOffset,
  fitPitchRange,
  gridPoint,
  layoutNoteTracks,
  pitchLabel,
  rollGeometry,
  rollOffset,
  snapGrid,
} from '../src/lib/piano-roll/grid';
import type { RollNote } from '../src/lib/piano-roll/types';

const { bars, width } = rollGeometry([
  { cycle: 2, label: '3', numerator: 4, denominator: 4 },
  { cycle: 3, label: '4', numerator: 7, denominator: 8 },
  { cycle: 4, label: '5', numerator: 6, denominator: 8 },
]);

const note = (pitch: number): RollNote => ({ id: String(pitch), start: '0', end: '1/4', pitch });

describe('piano roll geometry', () => {
  test('equivalent musical lengths have the same width across meter changes', () => {
    expect(width).toBe(10.5);
    expect(bars.map((bar) => bar.width)).toEqual([4, 3.5, 3]);
    expect(rollOffset(bars, 2.25) - rollOffset(bars, 2)).toBe(1);
    expect(rollOffset(bars, 3 + 2 / 7) - rollOffset(bars, 3)).toBeCloseTo(1);
    expect(rollOffset(bars, 1)).toBe(0);
    expect(rollOffset(bars, 6)).toBe(width);
    expect(barAtOffset(bars, 4)).toBe(bars[1]);
    expect(barAtOffset(bars, width)).toBe(bars[2]);
  });

  test('snaps triplets to exact fractions and keeps a bounded move within its bar', () => {
    const bar = bars[1]!;
    expect(gridPoint(bar, 1, 3)).toBe('64/21');
    expect(snapGrid(bar, 4.17, 3)).toBe('64/21');
    expect(snapGrid(bar, 20, 3, 1 - 2 / 7)).toBe('26/7');
    expect(snapGrid(bar, -4, 3)).toBe('3');
  });

  test('keyboard steps surround off-grid notes and pass through exact subdivision boundaries', () => {
    const bar = bars[1]!;
    expect(adjacentStep(bar, 3 + 2 / 21, 3, 1)).toBe(3);
    expect(adjacentStep(bar, 3 + 2 / 21, 3, -1)).toBe(1);
    expect(adjacentStep(bar, 3 + 1 / 8, 3, 1)).toBe(3);
    expect(adjacentStep(bar, 3 + 1 / 8, 3, -1)).toBe(2);
  });

  test('fits the visible pitches and retains padding against the MIDI limits', () => {
    expect(fitPitchRange([])).toEqual({ low: 57, high: 65 });
    expect(fitPitchRange([note(71)])).toEqual({ low: 67, high: 75 });
    expect(fitPitchRange([note(127)])).toEqual({ low: 119, high: 127 });
    expect(fitPitchRange([note(60), note(76.5)])).toEqual({ low: 58, high: 79 });
    expect(pitchLabel(0)).toBe('C-1');
    expect(pitchLabel(76.5)).toBe('MIDI 76.5');
  });

  test('separates sustained voices and duplicates, while touching notes reuse a track', () => {
    const voice = (id: string, start: string, end: string): RollNote => ({
      ...note(60),
      id,
      start,
      end,
    });
    const result = layoutNoteTracks(
      [
        voice('first', '0', '1/2'),
        voice('sustain', '0', '1'),
        voice('duplicate', '0', '1/2'),
        voice('second', '1/2', '1'),
        voice('later', '1', '2'),
      ],
      0,
      2,
    );
    expect(result.map(({ note, track, tracks }) => [note.id, track, tracks])).toEqual([
      ['sustain', 0, 3],
      ['first', 1, 3],
      ['duplicate', 2, 3],
      ['second', 1, 3],
      ['later', 0, 1],
    ]);
  });

  test('assigns tracks using clipped fragments rather than whole spans', () => {
    const sustained = { ...note(60), id: 'sustain', end: '1', visibleEnd: '1/2' };
    const next = { ...note(60), id: 'next', start: '1/2', end: '1' };
    expect(layoutNoteTracks([sustained, next], 0, 1)).toEqual([
      { note: sustained, track: 0, tracks: 1 },
      { note: next, track: 0, tracks: 1 },
    ]);
  });
});

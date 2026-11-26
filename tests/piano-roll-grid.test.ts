import { describe, expect, test } from 'bun:test';
import {
  adjacentStep,
  barAtOffset,
  fitPitchRange,
  gridPoint,
  layoutNoteTracks,
  noteMarkerKey,
  pitchLabel,
  rollGeometry,
  rollOffset,
  shiftRollEdge,
  snapGrid,
} from '../src/lib/piano-roll/grid';
import type { RollNote } from '../src/lib/piano-roll/types';
import {
  sectionMoveConflict,
  resolveRollMove,
  rollMoveTime,
  spansUseSameCps,
} from '../src/lib/piano-roll/move';
import { buildStructure } from '../src/lib/strudel/structure';

const { bars, width } = rollGeometry([
  { cycle: 2, label: '3', numerator: 4, denominator: 4 },
  { cycle: 3, label: '4', numerator: 7, denominator: 8 },
  { cycle: 4, label: '5', numerator: 6, denominator: 8 },
]);

const note = (pitch: number): RollNote => ({ id: String(pitch), start: '0', end: '1/4', pitch });

describe('musical move intervals', () => {
  const structure = buildStructure({
    meter: '4/4',
    meterChanges: [
      { bar: 2, meter: '7/8' },
      { bar: 3, meter: '6/8' },
    ],
    sections: [
      { name: 'verse', bars: 3 },
      { name: 'tail', bars: 2 },
    ],
  });
  const time = rollMoveTime(structure, 3, 0);

  test('preserves a quarter note across 4/4, 7/8 and 6/8, including in reverse', () => {
    const source = { ...note(64), start: '1/4', end: '1/2' };
    const moved = resolveRollMove(source, '1/4', '10/7', time);
    expect(moved).toMatchObject({ start: '10/7', end: '12/7' });
    const next = resolveRollMove({ ...source, ...moved! }, '10/7', '2', time);
    expect(next).toMatchObject({ start: '2', end: '7/3' });
    expect(resolveRollMove({ ...source, ...next! }, '2', '1/4', time)).toMatchObject({
      start: source.start,
      end: source.end,
    });
  });

  test('measures both halves of a carry-in note and fits exact boundaries', () => {
    const source = { ...note(64), start: '3/4', end: '5/4' };
    expect(resolveRollMove(source, '1', '2', time)).toMatchObject({
      start: '12/7',
      end: '55/24',
    });
    expect(resolveRollMove(source, '1', '0', time)).toBeNull();
    expect(resolveRollMove(source, '1', '23/8', time)).toBeNull();
    expect(resolveRollMove(note(60), '0', '8/3', time)).toMatchObject({ end: '3' });
    expect(resolveRollMove(note(60), '0', '0', time)).toBeUndefined();
  });

  test('crosses a long constant-meter run without accumulating a rounding error', () => {
    const time = rollMoveTime(null, 640_000);
    const source = { ...note(60), start: '1/3', end: '900001/3' };
    expect(resolveRollMove(source, '1/3', '2/3', time)).toMatchObject({
      start: '2/3',
      end: '900002/3',
    });
  });

  test('checks every affected phase in a section and every shared section', () => {
    const a = { start: '1/4', end: '1/2' };
    const b = { start: '10/7', end: '12/7' };
    expect(sectionMoveConflict(structure, ['verse'], a, b)).toBeUndefined();
    expect(sectionMoveConflict(structure, ['verse'], a, b, 2)).toBe('placement');
    expect(sectionMoveConflict(structure, ['verse'], a, b, 3)).toBeUndefined();
    expect(sectionMoveConflict(structure, ['verse', 'tail'], a, b, 3)).toBe('placement');
    expect(sectionMoveConflict(structure, ['verse'], b, a, 2)).toBe('placement');
    expect(sectionMoveConflict(structure, ['tail'], { start: '1', end: '3' }, a)).toBe('placement');
    expect(spansUseSameCps(time, a, b)).toBe(false);
    expect(spansUseSameCps(time, { start: '1/2', end: '1' })).toBe(true);
    expect(spansUseSameCps(time, { start: '1/2', end: '5/4' })).toBe(false);
    const acrossChange = { start: '1/2', end: '5/4' };
    const equalLength = { start: '2', end: '71/24' };
    expect(sectionMoveConflict(structure, ['verse'], acrossChange, equalLength)).toBeUndefined();
    expect(
      sectionMoveConflict(structure, ['verse'], acrossChange, equalLength, undefined, acrossChange),
    ).toBe('meter');
  });
});

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
    const tripletSteps = bar.numerator * 3;
    expect(gridPoint(bar, 1, tripletSteps)).toBe('64/21');
    expect(snapGrid(bar, 4.17, tripletSteps)).toBe('64/21');
    expect(snapGrid(bar, 20, tripletSteps, 1 - 2 / 7)).toBe('26/7');
    expect(snapGrid(bar, -4, tripletSteps)).toBe('3');
  });

  test('keyboard steps surround off-grid notes and pass through exact subdivision boundaries', () => {
    const bar = bars[1]!;
    const tripletSteps = bar.numerator * 3;
    expect(adjacentStep(bar, 3 + 2 / 21, tripletSteps, 1)).toBe(3);
    expect(adjacentStep(bar, 3 + 2 / 21, tripletSteps, -1)).toBe(1);
    expect(adjacentStep(bar, 3 + 1 / 8, tripletSteps, 1)).toBe(3);
    expect(adjacentStep(bar, 3 + 1 / 8, tripletSteps, -1)).toBe(2);
  });

  test('snaps written slots independently of a mixed-meter bar ruler', () => {
    const bar = bars[1]!;
    const slots = 16;
    expect(bar.numerator).toBe(7);
    expect(gridPoint(bar, 1, slots)).toBe('49/16');
    const offset = bar.offset + bar.width * 0.063;
    expect(snapGrid(bar, offset, slots)).toBe('49/16');
    expect(adjacentStep(bar, 3 + 1 / 16, slots, 1)).toBe(2);
    expect(adjacentStep(bar, 3 + 1 / 16, slots, -1)).toBe(0);
    expect(snapGrid(bar, bar.offset + bar.width, slots, 1 - 1 / slots)).toBe('63/16');
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

  test('measures a carry-in edit relative to the visible portion without jumping its onset', () => {
    expect(shiftRollEdge('0', '3', '49/16')).toBe('1/16');
    expect(shiftRollEdge('3', '5/2', '11/4')).toBe('13/4');
    expect(shiftRollEdge('1/3', '1/3', '1/2')).toBe('1/2');
  });

  test('keys disconnected fragments of the same note separately across inspections', () => {
    const first = { ...note(60), id: 'first', end: '1', visibleEnd: '1/4' };
    const later = { ...first, id: 'later', visibleStart: '1/2', visibleEnd: '3/4' };
    const markers = layoutNoteTracks([first, later], 0, 1);
    expect(markers.map(({ track }) => track)).toEqual([0, 0]);
    expect(noteMarkerKey(first, 0)).not.toBe(noteMarkerKey(later, 0));
    expect(noteMarkerKey({ ...first, id: 'next inspection' }, 0)).toBe(noteMarkerKey(first, 0));
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

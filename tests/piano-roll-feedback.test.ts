import { describe, expect, test } from 'bun:test';
import {
  notePosition,
  projectRollEdit,
  reconcileRollNotes,
  sameRollPosition,
  sourceEditFeedback,
} from '../src/lib/piano-roll/feedback';
import type { RollNote } from '../src/lib/piano-roll/types';

const note: RollNote = {
  id: 'old',
  pitch: 64,
  start: '3/4',
  end: '5/4',
  visibleEnd: '1',
  event: { pitch: 64, from: ['3/4', '5/4'] },
};

describe('piano roll feedback', () => {
  test('a move carries exact timing and the original identity past regeneration', () => {
    const { edited, notes } = projectRollEdit(
      [note],
      { kind: 'move', id: 'old', start: '7/8', end: '9/8' },
      false,
    );
    expect(notes).toEqual(edited);
    expect(edited[0]).toMatchObject({
      start: '7/8',
      end: '9/8',
      visibleEnd: '1',
      event: note.event,
    });
    expect(sameRollPosition({ ...edited[0]!, id: 'fresh-host-id' }, notePosition(edited[0]!))).toBe(
      true,
    );
  });

  test('reclips a moved carry at the viewport and keeps mask edges inside the page', () => {
    const left = { ...note, start: '3', end: '5', visibleStart: '4', visibleEnd: '9/2' };
    const right = { ...note, start: '7', end: '9', visibleStart: '15/2', visibleEnd: '8' };
    expect(
      projectRollEdit([left], { kind: 'move', id: left.id, start: '25/8', end: '41/8' }, false, {
        from: 4,
        to: 8,
      }).edited[0],
    ).toMatchObject({ start: '25/8', visibleStart: undefined, visibleEnd: '37/8' });
    expect(
      projectRollEdit([right], { kind: 'move', id: right.id, start: '55/8', end: '71/8' }, false, {
        from: 4,
        to: 8,
      }).edited[0],
    ).toMatchObject({ visibleStart: '59/8', visibleEnd: undefined, end: '71/8' });
  });

  test('moves masked fragments together and refreshes their individual parts without moving a unison', () => {
    const event = {
      from: ['0', '1'] as [string, string],
      pitch: 64,
      voice: 'sine',
      count: 2,
      index: 0,
    };
    const left = {
      ...note,
      id: 'left',
      start: '0',
      end: '1',
      event,
      visibleStart: '0',
      visibleEnd: '1/4',
    };
    const right = { ...left, id: 'right', visibleStart: '1/2', visibleEnd: '3/4' };
    const unison = { ...right, id: 'unison', event: { ...event, index: 1 } };
    const nextCycle = {
      ...left,
      id: 'repeat',
      start: '2',
      end: '3',
      visibleStart: '2',
      visibleEnd: '9/4',
    };
    const projected = projectRollEdit(
      [left, right, unison, nextCycle],
      { kind: 'move', id: 'right', start: '10/7', end: '12/7' },
      false,
    );
    expect(projected.edited.map((n) => [n.id, n.visibleStart, n.visibleEnd])).toEqual([
      ['left', '10/7', '3/2'],
      ['right', '11/7', '23/14'],
    ]);
    expect(projected.notes.slice(2)).toEqual([unison, nextCycle]);
    const refreshed = projected.edited
      .map((n, index) => ({
        ...n,
        id: String(index),
        event: { ...event, entry: 'roll1' },
      }))
      .reverse();
    const reconciled = reconcileRollNotes(projected.notes, refreshed, new Set(['left', 'right']));
    expect(reconciled.map((n) => [n.id, n.visibleStart, n.visibleEnd, n.event?.entry])).toEqual([
      ['left', '10/7', '3/2', 'roll1'],
      ['right', '11/7', '23/14', 'roll1'],
    ]);
  });

  test('direct retuning flashes notes bound to one source token', () => {
    const origin = { from: 10, to: 12 };
    const notes = [
      { ...note, origin },
      { ...note, id: 'repeat', origin, start: '7/4', end: '9/4' },
    ];
    const edit = { kind: 'pitch', id: 'old', pitch: 65 } as const;
    expect(projectRollEdit(notes, edit, true).edited.map(({ id }) => id)).toEqual([
      'old',
      'repeat',
    ]);
    expect(projectRollEdit(notes, edit, false).edited.map(({ id }) => id)).toEqual(['old']);
  });

  test('keeps an edited triangle distinct from the overlapping sine through a host refresh', () => {
    const sine = { ...note, id: '0', pitch: 61, sound: 'sine' };
    const triangle = { ...note, id: '1', pitch: 60, sound: 'triangle' };
    const first = projectRollEdit([sine, triangle], { kind: 'pitch', id: '1', pitch: 61 }, false);
    const edited = new Set(first.edited.map(({ id }) => id));
    const refreshed = reconcileRollNotes(
      first.notes,
      [
        { ...triangle, id: '0', pitch: 61, event: { ...note.event!, entry: 'roll1' } },
        { ...sine, id: '1' },
      ],
      edited,
    );
    expect(refreshed.map(({ id, sound }) => [id, sound])).toEqual([
      ['0', 'sine'],
      ['1', 'triangle'],
    ]);
    expect(refreshed.filter(({ id }) => edited.has(id)).map(({ sound }) => sound)).toEqual([
      'triangle',
    ]);
    expect(
      projectRollEdit(refreshed, { kind: 'pitch', id: '1', pitch: 62 }, false).notes.map(
        ({ sound, pitch }) => [sound, pitch],
      ),
    ).toEqual([
      ['sine', 61],
      ['triangle', 62],
    ]);
  });

  test('flashes the retuned grid note and the following glide whose bend changed', () => {
    const glide = { ...note, id: 'glide', pitch: 67, start: '5/4', end: '2', glideBend: -3 };
    const unaffected = { ...glide, id: 'later', start: '2', end: '3' };
    const preview = projectRollEdit(
      [note, glide, unaffected],
      { kind: 'pitch', id: 'old', pitch: 65 },
      true,
    );
    const changed = [
      { ...note, id: 'retuned', pitch: 65 },
      { ...glide, id: 'changed-glide', glideBend: -2 },
      unaffected,
    ];
    expect(sourceEditFeedback(preview.notes, changed, new Set(['old']))).toEqual(
      new Set(['retuned', 'changed-glide']),
    );
  });
});

import { Fraction } from '@strudel/core';
import type { RollEdit, RollNote } from './types';

export function notePosition({ pitch, start, end }: RollNote) {
  return { pitch, start, end };
}

export function sameRollPosition(note: RollNote, position: ReturnType<typeof notePosition>) {
  return (
    Math.abs(note.pitch - position.pitch) < 1e-8 &&
    Fraction(note.start).equals(Fraction(position.start)) &&
    Fraction(note.end).equals(Fraction(position.end))
  );
}

/** Predict changes in displayed coordinates while source and host inspection catch up. */
export function projectRollEdit(notes: readonly RollNote[], change: RollEdit, direct: boolean) {
  const edited: RollNote[] = [];
  if (change.kind === 'insert') {
    const { start, end, pitch } = change;
    edited.push({ id: 'inserted', start, end, pitch });
    return { notes, edited };
  }
  const selected = notes.find((note) => note.id === change.id);
  const projected = notes.flatMap((note) => {
    const linked =
      direct &&
      change.kind === 'pitch' &&
      selected?.origin &&
      note.origin?.from === selected.origin.from &&
      note.origin.to === selected.origin.to;
    if (note !== selected && !linked) return [note];
    if (change.kind === 'delete') return [];
    let next: RollNote;
    if (change.kind === 'pitch') {
      next = { ...note, pitch: change.pitch };
    } else if (change.kind === 'move') {
      const length = Fraction(note.end).sub(Fraction(note.start));
      const end = Fraction(change.start).add(length).toFraction();
      next = { ...note, start: change.start, end, visibleStart: undefined, visibleEnd: undefined };
    } else {
      next = { ...note, end: change.end, visibleEnd: undefined };
    }
    edited.push(next);
    return [next];
  });
  return { notes: projected, edited };
}

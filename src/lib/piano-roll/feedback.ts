import { Fraction } from '@strudel/core';
import type { PianoRollCommit } from '../project/session';
import { resolveRollTarget } from './source';
import type { RollEdit, RollNote } from './types';

export function resolvePostEditTarget(
  { snapshot, target, targetCaret }: PianoRollCommit,
  wrappedTarget: boolean,
) {
  if (!target) return null;
  const exact = resolveRollTarget(snapshot.code, target);
  if (exact || !wrappedTarget || targetCaret === undefined) return exact;
  // A wrapper's leading semicolon or shorthand property key is outside the expression.
  const inner = resolveRollTarget(snapshot.code, { from: targetCaret, to: targetCaret });
  return inner &&
    inner.expression.from >= target.from &&
    inner.expression.from < targetCaret &&
    inner.expression.to === target.to
    ? inner
    : null;
}

export function notePosition({ pitch, start, end }: RollNote) {
  return { pitch, start, end };
}

export function sameRollPosition(note: RollNote, position: ReturnType<typeof notePosition>) {
  return (
    Math.abs(note.pitch - position.pitch) < 1e-8 &&
    (note.start === position.start || Fraction(note.start).equals(Fraction(position.start))) &&
    (note.end === position.end || Fraction(note.end).equals(Fraction(position.end)))
  );
}

function matchRollNotes(
  projected: readonly RollNote[],
  current: readonly RollNote[],
  edited: ReadonlySet<string>,
) {
  const remaining = new Set(current);
  const matches = new Map<RollNote, RollNote>();
  for (const note of projected) {
    let best: RollNote | undefined;
    let bestScore = -1;
    for (const candidate of remaining) {
      if (
        candidate.start !== note.start ||
        candidate.end !== note.end ||
        !sameRollPosition(candidate, note) ||
        (note.sound !== undefined && candidate.sound !== note.sound) ||
        (note.event?.entry && candidate.event?.entry !== note.event.entry)
      )
        continue;
      const score =
        4 * Number(Boolean(candidate.event?.entry) === edited.has(note.id)) +
        2 * Number(candidate.event?.voice === note.event?.voice) +
        Number(candidate.event?.index === note.event?.index);
      if (score > bestScore) {
        best = candidate;
        bestScore = score;
      }
    }
    if (!best) continue;
    matches.set(note, best);
    remaining.delete(best);
  }
  return { matches, remaining };
}

/** Keep the roll's IDs and voice order while taking fresh host selectors and source locations. */
export function reconcileRollNotes(
  projected: readonly RollNote[],
  inspected: readonly RollNote[],
  edited: ReadonlySet<string>,
): RollNote[] {
  const { matches, remaining } = matchRollNotes(projected, inspected, edited);
  const reserved = new Set(projected.map(({ id }) => id));
  const notes = Array.from(matches, ([previous, current]) => ({ ...current, id: previous.id }));
  for (const note of remaining) {
    let id = note.id;
    while (reserved.has(id)) id = 'fresh:' + id;
    reserved.add(id);
    notes.push({ ...note, id });
  }
  return notes;
}

export function sourceEditFeedback(
  projected: readonly RollNote[],
  current: readonly RollNote[],
  edited: ReadonlySet<string>,
): ReadonlySet<string> {
  const { matches } = matchRollNotes(projected, current, edited);
  return new Set(
    Array.from(matches)
      .filter(([before, after]) => edited.has(before.id) || before.glideBend !== after.glideBend)
      .map(([, after]) => after.id),
  );
}

/** Predict changes in displayed coordinates while source and host inspection catch up. */
export function projectRollEdit(notes: readonly RollNote[], change: RollEdit, direct: boolean) {
  const edited: RollNote[] = [];
  if (change.kind === 'insert') {
    const { start, end, pitch } = change;
    edited.push({ id: 'inserted', start, end, pitch });
    return { notes: [...notes, ...edited], edited };
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

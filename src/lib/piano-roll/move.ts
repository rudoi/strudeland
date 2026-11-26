import { Fraction } from '@strudel/core';
import { beatsPerBar, DEFAULT_METER, type Meter, type SongStructure } from '../strudel/structure';
import type { RollEdit, RollNote } from './types';

type Edge = ReturnType<typeof Fraction>;
type Span = Pick<RollNote, 'start' | 'end'>;
type Move = Extract<RollEdit, { kind: 'move' }>;

const ceil = (edge: Edge): number => -Number(edge.mul(-1).floor());

export interface RollMoveTime {
  end: number;
  runAt(bar: number): { from: number; to: number; meter: Meter };
}

export function rollClockChangesCps(structure: SongStructure): boolean {
  const reference = beatsPerBar(structure.meter);
  return structure.meterChanges?.some(({ meter }) => beatsPerBar(meter) !== reference) ?? false;
}

export function rollMoveTime(
  structure: SongStructure | null,
  end: number,
  writtenStart?: number,
): RollMoveTime {
  const base = structure?.meter ?? DEFAULT_METER;
  const changes = structure?.meterChanges;
  if (!changes?.length || writtenStart === undefined) {
    const run = { from: 0, to: end, meter: base };
    return { end, runAt: () => run };
  }
  return {
    end,
    runAt(bar) {
      const written = writtenStart + bar;
      let first = 0;
      let last = changes.length;
      while (first < last) {
        const middle = Math.floor((first + last) / 2);
        if (changes[middle]!.bar <= written) first = middle + 1;
        else last = middle;
      }
      const change = changes[first - 1];
      const next = changes[first];
      return {
        from: change ? Math.max(0, change.bar - writtenStart) : 0,
        to: next ? Math.min(end, next.bar - writtenStart) : end,
        meter: change?.meter ?? base,
      };
    },
  };
}

function quarterLength(start: Edge, end: Edge, time: RollMoveTime): Edge {
  let length = Fraction(0);
  for (let at = start; at.lt(end);) {
    const run = time.runAt(Number(at.floor()));
    const next = end.min(Fraction(run.to));
    length = length.add(next.sub(at).mul(beatsPerBar(run.meter)));
    at = next;
  }
  return length;
}

function atDistance(
  start: Edge,
  quarters: Edge,
  direction: 1 | -1,
  time: RollMoveTime,
): Edge | null {
  let at = start;
  while (quarters.gt(0)) {
    if (direction === 1 ? at.gte(time.end) : at.lte(0)) return null;
    const bar = direction === 1 ? Number(at.floor()) : ceil(at) - 1;
    const run = time.runAt(bar);
    const boundary = Fraction(direction === 1 ? run.to : run.from);
    const width = beatsPerBar(run.meter);
    const available = boundary.sub(at).mul(direction).mul(width);
    if (quarters.lte(available)) return at.add(quarters.div(width).mul(direction));
    quarters = quarters.sub(available);
    at = boundary;
  }
  return at;
}

/** Reposition the grabbed edge and retain the whole note's length in quarter notes. */
export function resolveRollMove(
  note: RollNote,
  anchor: string,
  snapped: string,
  time: RollMoveTime,
): Move | null | undefined {
  const start = Fraction(note.start);
  const end = Fraction(note.end);
  const grabbed = Fraction(anchor);
  const dest = Fraction(snapped);
  if (dest.eq(grabbed)) return;
  if (
    start.lt(0) ||
    end.gt(time.end) ||
    end.lte(start) ||
    grabbed.lt(start) ||
    grabbed.gte(end) ||
    dest.lt(0) ||
    dest.gte(time.end)
  )
    return null;
  const movedStart = atDistance(dest, quarterLength(start, grabbed, time), -1, time);
  if (!movedStart) return null;
  const movedEnd = atDistance(dest, quarterLength(grabbed, end, time), 1, time);
  if (!movedEnd) return null;
  return { kind: 'move', id: note.id, start: movedStart.toFraction(), end: movedEnd.toFraction() };
}

export function crossesRollBar(note: Span, move: Span): boolean {
  const bar = Fraction(note.start).floor();
  return (
    !Fraction(move.start).floor().eq(bar) ||
    Fraction(note.end).gt(bar.add(1)) ||
    Fraction(move.end).gt(bar.add(1))
  );
}

type ExactSpan = readonly [start: Edge, end: Edge];

function sameCps(time: RollMoveTime, first: ExactSpan, ...others: ExactSpan[]): boolean {
  const width = beatsPerBar(time.runAt(Number(first[0].floor())).meter);
  return [first, ...others].every(([start, end]) => {
    if (start.lt(0) || end.gt(time.end)) return false;
    const until = ceil(end);
    for (let bar = Number(start.floor()); bar < until;) {
      const run = time.runAt(bar);
      if (beatsPerBar(run.meter) !== width) return false;
      bar = run.to;
    }
    return true;
  });
}

/** Different time signatures with the same quarter-note length use the same CPS. */
export function spansUseSameCps(time: RollMoveTime, first: Span, ...others: Span[]): boolean {
  const exact = ({ start, end }: Span): ExactSpan => [Fraction(start), Fraction(end)];
  return sameCps(time, exact(first), ...others.map(exact));
}

/** A phrase restarts at cycle zero at each section, and may repeat inside it. */
export function sectionMoveConflict(
  structure: SongStructure,
  names: readonly string[],
  note: Span,
  move: Span,
  period?: number,
  queryOrigin?: Span,
): 'placement' | 'meter' | undefined {
  const sourceStart = Fraction(note.start);
  const sourceEnd = Fraction(note.end);
  const destStart = Fraction(move.start);
  const destEnd = Fraction(move.end);
  const origin = queryOrigin && [Fraction(queryOrigin.start), Fraction(queryOrigin.end)];
  for (const name of names) {
    const section = structure.sections.find((item) => item.name === name);
    if (!section) return 'placement';
    const time = rollMoveTime(structure, section.bars, section.start);
    const earliest = sourceStart.min(destStart);
    const latest = sourceEnd.max(destEnd);
    const first = period ? Number(latest.mul(-1).div(period).floor()) + 1 : 0;
    const last = period ? ceil(Fraction(time.end).sub(earliest).div(period)) - 1 : 0;
    for (let index = first; index <= last; index++) {
      const offset = index * (period ?? 0);
      const a = sourceStart.add(offset),
        b = sourceEnd.add(offset),
        c = destStart.add(offset),
        d = destEnd.add(offset);
      const audible = (start: Edge, end: Edge) => end.gt(0) && start.lt(time.end);
      if (!audible(a, b) && !audible(c, d)) continue;
      if (a.lt(0) || b.gt(time.end) || c.lt(0) || d.gt(time.end)) return 'placement';
      if (!quarterLength(a, b, time).eq(quarterLength(c, d, time))) return 'placement';
      if (origin && !sameCps(time, [origin[0].add(offset), origin[1].add(offset)], [c, d]))
        return 'meter';
    }
  }
}

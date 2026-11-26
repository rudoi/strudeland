import { Fraction } from '@strudel/core';
import { beatsPerBar, meterAtBar, type Meter, type SongStructure } from '../strudel/structure';
import type { RollEdit, RollNote } from './types';

type Edge = ReturnType<typeof Fraction>;
type Span = Pick<RollNote, 'start' | 'end'>;
type Move = Extract<RollEdit, { kind: 'move' }>;

const ceil = (edge: Edge): number => -Number(edge.mul(-1).floor());

export interface RollMoveTime {
  end: number;
  meterAt(cycle: number): Meter;
}

function quarterLength(start: Edge, end: Edge, { meterAt }: RollMoveTime): Edge {
  let length = Fraction(0);
  for (let at = start; at.lt(end);) {
    const bar = Number(at.floor());
    const next = end.min(Fraction(bar + 1));
    length = length.add(next.sub(at).mul(beatsPerBar(meterAt(bar))));
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
    const boundary = Fraction(direction === 1 ? bar + 1 : bar);
    const width = beatsPerBar(time.meterAt(bar));
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
  const movedEnd = atDistance(movedStart, quarterLength(start, end, time), 1, time);
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

/** Different time signatures with the same quarter-note length use the same CPS. */
export function spansUseSameCps(time: RollMoveTime, ...spans: Span[]): boolean {
  const width = beatsPerBar(time.meterAt(Number(Fraction(spans[0]!.start).floor())));
  return spans.every(({ start, end }) => {
    const until = ceil(Fraction(end));
    for (let bar = Number(Fraction(start).floor()); bar < until; bar++) {
      if (beatsPerBar(time.meterAt(bar)) !== width) return false;
    }
    return true;
  });
}

/** A phrase restarts at cycle zero at each section, and may repeat inside it. */
export function moveFitsSections(
  structure: SongStructure,
  names: readonly string[],
  note: Span,
  move: Span,
  period?: number,
): boolean {
  const sourceStart = Fraction(note.start);
  const sourceEnd = Fraction(note.end);
  const destStart = Fraction(move.start);
  const destEnd = Fraction(move.end);
  return names.every((name) => {
    const section = structure.sections.find((item) => item.name === name);
    if (!section) return false;
    const time = {
      end: section.bars,
      meterAt: (cycle: number) => meterAtBar(structure, section.start + cycle),
    };
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
      if (a.lt(0) || b.gt(time.end) || c.lt(0) || d.gt(time.end)) return false;
      if (!quarterLength(a, b, time).eq(quarterLength(c, d, time))) return false;
    }
    return true;
  });
}

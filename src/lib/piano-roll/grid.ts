import { Fraction } from '@strudel/core';
import type { RollBar, RollNote } from './types';

export interface GridBar extends RollBar {
  offset: number;
  width: number;
}

export interface PitchRange {
  low: number;
  high: number;
}

export function rollGeometry(bars: readonly RollBar[]): { bars: GridBar[]; width: number } {
  let width = 0;
  return {
    bars: bars.map((bar) => {
      const offset = width;
      const length = (bar.numerator * 4) / bar.denominator;
      width += length;
      return { ...bar, offset, width: length };
    }),
    width,
  };
}

export const cycleNumber = (position: string): number => Number(Fraction(position));

/** Logical time to quarter-note width from the start of this window. */
export function rollOffset(bars: readonly GridBar[], cycle: number): number {
  const bar = bars.find((bar) => cycle < bar.cycle + 1) ?? bars.at(-1);
  if (!bar) return 0;
  return bar.offset + Math.max(0, Math.min(1, cycle - bar.cycle)) * bar.width;
}

export function barAtOffset(bars: readonly GridBar[], offset: number): GridBar | undefined {
  return bars.find((bar) => offset < bar.offset + bar.width) ?? bars.at(-1);
}

/** Return an exact edge on a bar's beat subdivisions or authored slots. */
export function gridPoint(bar: GridBar, step: number, steps: number): string {
  return Fraction(bar.cycle).add(Fraction(step).div(steps)).toFraction();
}

export function snapGrid(bar: GridBar, offset: number, steps: number, upper = 1): string {
  const local = (offset - bar.offset) / bar.width;
  const last = Math.max(0, Math.floor(upper * steps + Number.EPSILON * steps));
  return gridPoint(bar, Math.max(0, Math.min(last, Math.round(local * steps))), steps);
}

export function adjacentStep(bar: GridBar, cycle: number, steps: number, delta: number) {
  const actual = (cycle - bar.cycle) * steps;
  const epsilon = Number.EPSILON * Math.max(1, Math.abs(cycle)) * steps;
  return delta > 0 ? Math.floor(actual + epsilon) + 1 : Math.ceil(actual - epsilon) - 1;
}

export const keyPitch = (pitch: number): number => Math.max(0, Math.min(127, pitch));

export function shiftRollEdge(edge: string, anchor: string, destination: string): string {
  return Fraction(edge).add(Fraction(destination).sub(anchor)).toFraction();
}

/** Identify the marker across inspections that regenerate IDs. A single event may have gaps. */
export function noteMarkerKey(note: RollNote, track: number): string {
  return [
    note.pitch,
    note.start,
    note.end,
    note.visibleStart ?? note.start,
    note.visibleEnd ?? note.end,
    track,
  ].join(':');
}

/** Split overlapping visible notes within a pitch lane. Touching intervals can reuse a track. */
export function layoutNoteTracks(notes: readonly RollNote[], from: number, to: number) {
  type Interval = { note: RollNote; start: number; end: number };
  type Assignment = { note: RollNote; track: number };
  const lanes = new Map<number, Interval[]>();
  for (const note of notes) {
    const pitch = keyPitch(note.pitch);
    const lane = lanes.get(pitch) ?? [];
    lane.push({
      note,
      start: Math.max(from, cycleNumber(note.visibleStart ?? note.start)),
      end: Math.min(to, cycleNumber(note.visibleEnd ?? note.end)),
    });
    lanes.set(pitch, lane);
  }

  const result: (Assignment & { tracks: number })[] = [];
  for (const lane of lanes.values()) {
    lane.sort((a, b) => a.start - b.start || b.end - a.end);
    let group: Assignment[] = [];
    let occupied: Interval[] = [];
    function flush() {
      for (const assignment of group) result.push({ ...assignment, tracks: occupied.length });
      group = [];
      occupied = [];
    }

    for (const interval of lane) {
      const free = ({ start, end }: Interval) =>
        end < interval.start || (end === interval.start && start < end);
      if (occupied.every(free)) flush();
      let track = occupied.findIndex(free);
      if (track === -1) track = occupied.length;
      occupied[track] = interval;
      group.push({ note: interval.note, track });
    }
    flush();
  }
  return result;
}

export function fitPitchRange(notes: readonly RollNote[]): PitchRange {
  if (notes.length === 0) return { low: 57, high: 65 };
  let low = 127;
  let high = 0;
  for (const { pitch } of notes) {
    if (!Number.isFinite(pitch)) continue;
    low = Math.min(low, Math.floor(keyPitch(pitch)));
    high = Math.max(high, Math.ceil(keyPitch(pitch)));
  }
  if (low > high) return { low: 57, high: 65 };
  const pad = Math.max(2, Math.ceil((8 - (high - low)) / 2));
  low -= pad;
  high += pad;
  if (low < 0) return { low: 0, high: Math.min(127, high - low) };
  if (high > 127) return { low: Math.max(0, low - (high - 127)), high: 127 };
  return { low, high };
}

const pitchNames = ['C', 'C♯', 'D', 'D♯', 'E', 'F', 'F♯', 'G', 'G♯', 'A', 'A♯', 'B'];

export function pitchLabel(pitch: number): string {
  if (!Number.isInteger(pitch) || pitch < 0 || pitch > 127) return `MIDI ${pitch}`;
  return `${pitchNames[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
}

export function isBlackKey(pitch: number): boolean {
  return pitchNames[pitch % 12]?.endsWith('♯') ?? false;
}

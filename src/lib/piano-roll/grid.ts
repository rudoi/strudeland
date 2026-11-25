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

/** One grid step is a denominator beat subdivision. A bar end is an exact admissible edge. */
export function gridPoint(bar: GridBar, step: number, subdivision: number): string {
  return Fraction(bar.cycle)
    .add(Fraction(step).div(bar.numerator * subdivision))
    .toFraction();
}

export function snapGrid(bar: GridBar, offset: number, subdivision: number, upper = 1): string {
  const steps = bar.numerator * subdivision;
  const local = (offset - bar.offset) / bar.width;
  const last = Math.max(0, Math.floor(upper * steps + Number.EPSILON * steps));
  return gridPoint(bar, Math.max(0, Math.min(last, Math.round(local * steps))), subdivision);
}

export function adjacentStep(bar: GridBar, cycle: number, subdivision: number, delta: number) {
  const actual = (cycle - bar.cycle) * bar.numerator * subdivision;
  const epsilon = Number.EPSILON * Math.max(1, Math.abs(cycle)) * bar.numerator * subdivision;
  return delta > 0 ? Math.floor(actual + epsilon) + 1 : Math.ceil(actual - epsilon) - 1;
}

export const keyPitch = (pitch: number): number => Math.max(0, Math.min(127, pitch));

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

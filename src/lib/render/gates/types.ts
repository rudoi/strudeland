/**
 * The shape every render gate returns. A fixture fails if any gate it runs is `fail`;
 * `unverifiable` never fails a fixture on its own, but the report lists it, and a fixture whose
 * manifest requires a gate fails if that gate is `unverifiable`. Pure.
 */

import { round } from '../dsp';
import type { RenderEvent } from '../schedule';

/** Fixed-meter calibration rounds near a downbeat. In mixed meter the schedule supplies the bar. */
export function scoreBar(event: RenderEvent, fixedBarSec?: number): number {
  return fixedBarSec === undefined ? event.bar : Math.floor(event.tSec / fixedBarSec + 1e-6);
}

export const GATE_NAMES = ['seam', 'twin', 'stems', 'bass', 'mix'] as const;
export type GateName = (typeof GATE_NAMES)[number];
export type GateVerdict = 'pass' | 'fail' | 'unverifiable';
export type Json = number | string | boolean | null | Json[] | { [k: string]: Json };

export interface GateResult {
  gate: GateName;
  verdict: GateVerdict;
  /** Numbers rounded to 2 decimals (dB) or 3 decimals (seconds). Always finite. */
  measurements: { [k: string]: Json };
  /** One line per failure or unverifiable cause; empty on a clean pass. */
  reasons: string[];
  /** Reported, never change the verdict (clip, shallow bass misses, aliased pairs). */
  warnings: string[];
}

/** dB measurement rounding: 2 decimals. */
export const roundDb = (x: number): number => round(x, 2);
/** Seconds measurement rounding: 3 decimals. */
export const roundSec = (x: number): number => round(x, 3);

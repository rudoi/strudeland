/**
 * The seam gate: chunk k rendered past its content end (`tail`) against chunk k+1's first
 * content bars (`head`), same audio time. A band window where the head is quieter than the tail
 * names material lost at the seam (lookback too short). One-sided on purpose: the head being
 * louder is not a loss. Pure (ported from the earlier render harness's seam check).
 */

import { biquadBandpass, mixToMono, ratioDb, rmsWindows } from '../dsp';
import {
  SEAM_BANDS,
  SEAM_FAIL_DB,
  SEAM_FLOOR,
  SEAM_TOLERANCE_DB,
  SEAM_WINDOW_SEC,
} from '../thresholds';
import { roundDb, roundSec, type GateResult } from './types';

export interface SeamDeficit {
  /** seamSec + window start. */
  tSec: number;
  band: string;
  deficitDb: number;
}

export interface SeamConsistency {
  seamSec: number;
  /** Band windows compared (silent ones skipped). */
  windows: number;
  /** Largest head-quieter deficit, >= 0, 2 decimals. */
  worstDeficitDb: number;
  deficits: SeamDeficit[];
  /** Set only on escalation: the worst two-way deficit between two renders of the head. */
  twinFloorDb?: number;
}

export interface SeamOptions {
  toleranceDb: number;
  /** Default SEAM_WINDOW_SEC. */
  windowSec?: number;
  /** Linear RMS, default SEAM_FLOOR. */
  floor?: number;
}

/** Compares the mono mean of `tail` and `head` per SEAM_BANDS band and window. `seamSec` only
 *  labels the result. Deficits over `toleranceDb` are listed. */
export function seamConsistency(
  tail: Float32Array[],
  head: Float32Array[],
  sampleRate: number,
  seamSec: number,
  opts: SeamOptions,
): SeamConsistency {
  const windowSec = opts.windowSec ?? SEAM_WINDOW_SEC;
  const floor = opts.floor ?? SEAM_FLOOR;
  const w = Math.max(1, Math.round(windowSec * sampleRate));
  const tm = mixToMono(tail);
  const hm = mixToMono(head);
  const n = Math.min(tm.length, hm.length);
  const deficits: SeamDeficit[] = [];
  let worst = 0;
  let windows = 0;
  const bandRms = (x: Float32Array, lo: number, hi: number) =>
    rmsWindows(biquadBandpass(x.subarray(0, n), sampleRate, lo, hi), sampleRate, w / sampleRate);
  for (const [band, lo, hi] of SEAM_BANDS) {
    const et = bandRms(tm, lo, hi);
    const eh = bandRms(hm, lo, hi);
    for (let k = 0; k < et.length; k++) {
      const a = et[k]!;
      const b = eh[k]!;
      if (Math.max(a, b) < floor) continue;
      windows++;
      const d = ratioDb(a, b, floor);
      if (d > worst) worst = d;
      if (d > opts.toleranceDb) {
        deficits.push({ tSec: seamSec + (k * w) / sampleRate, band, deficitDb: roundDb(d) });
      }
    }
  }
  return { seamSec, windows, worstDeficitDb: roundDb(worst), deficits };
}

/** Verdict for one seam: fails over SEAM_FAIL_DB plus the twin floor, if escalated. */
export function judgeSeam(sc: SeamConsistency): GateResult {
  const floorDb = sc.twinFloorDb ?? 0;
  const failAt = SEAM_FAIL_DB + floorDb;
  const measurements = {
    seamSec: roundSec(sc.seamSec),
    windows: sc.windows,
    worstDeficitDb: sc.worstDeficitDb,
    deficitWindows: sc.deficits.length,
    twinFloorDb: sc.twinFloorDb === undefined ? null : roundDb(sc.twinFloorDb),
    bands: [...new Set(sc.deficits.map((d) => d.band))],
  };
  const reasons: string[] = [];
  const warnings: string[] = [];
  let verdict: GateResult['verdict'];
  if (sc.windows === 0) {
    verdict = 'unverifiable';
    reasons.push('overlap is silent in every band');
  } else if (sc.worstDeficitDb > failAt) {
    verdict = 'fail';
    reasons.push(
      `seam at ${sc.seamSec.toFixed(2)}s lost material: worst band deficit ${sc.worstDeficitDb} dB ` +
        `(> ${SEAM_FAIL_DB} dB + twin floor ${roundDb(floorDb)} dB) in ` +
        `${sc.deficits.length}/${sc.windows} windows`,
    );
  } else {
    verdict = 'pass';
    if (sc.deficits.length > 0) {
      warnings.push(
        `${sc.deficits.length} windows over ${SEAM_TOLERANCE_DB} dB, worst ${sc.worstDeficitDb} dB`,
      );
    }
  }
  return { gate: 'seam', verdict, measurements, reasons, warnings };
}

/** The seam gate with escalation: over SEAM_FAIL_DB, the head is rendered again and the
 *  two-way deficit between the two heads (the renderer's own noise) raises the fail line. A
 *  lost voice is missing from both heads, so its floor stays near 0. `rerenderHead` returns the
 *  re-rendered head, trimmed like `head`; it is only called on escalation. */
export async function seamWithTwinEscalation(
  tail: Float32Array[],
  head: Float32Array[],
  sampleRate: number,
  seamSec: number,
  rerenderHead: () => Promise<Float32Array[]>,
): Promise<GateResult> {
  const sc = seamConsistency(tail, head, sampleRate, seamSec, { toleranceDb: SEAM_TOLERANCE_DB });
  if (sc.worstDeficitDb <= SEAM_FAIL_DB) return judgeSeam(sc);
  sc.twinFloorDb = twinFloorDb(head, await rerenderHead(), sampleRate, seamSec);
  return judgeSeam(sc);
}

/** The renderer's own noise at a seam: the worst two-way deficit between two renders of the
 *  same head. */
export function twinFloorDb(
  head: Float32Array[],
  twin: Float32Array[],
  sampleRate: number,
  seamSec: number,
): number {
  const opts: SeamOptions = { toleranceDb: SEAM_TOLERANCE_DB };
  return Math.max(
    seamConsistency(head, twin, sampleRate, seamSec, opts).worstDeficitDb,
    seamConsistency(twin, head, sampleRate, seamSec, opts).worstDeficitDb,
  );
}

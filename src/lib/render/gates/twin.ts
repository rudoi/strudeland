/**
 * The twin gate: two renders of the same request must agree. `exact` mode demands bit-equal
 * floats (the fork seeds every non-kabelsalat voice); `tolerance` mode bounds the 100 ms band
 * envelope differences and the whole-signal residual, for stochastic fixtures. Pure (ported
 * from the earlier render harness's twin-null probe).
 */

import {
  ampToDb,
  biquadBandpass,
  firstNonFinite,
  mixToMono,
  nearestRank,
  ratioDb,
  rmsWindows,
  sortedAsc,
} from '../dsp';
import {
  SEAM_BANDS,
  TWIN_FLOOR,
  TWIN_MAX_DB,
  TWIN_P95_DB,
  TWIN_RESIDUAL_DB,
  TWIN_WINDOW_SEC,
} from '../thresholds';
import { roundDb, type GateResult, type Json } from './types';

export type TwinMode = 'exact' | 'tolerance';

export interface TwinOptions {
  sampleRate: number;
  mode: TwinMode;
}

/** |dB| differences of the 100 ms band envelopes (silent cells skipped). */
export interface BandQuantiles {
  cells: number;
  p50: number;
  p95: number;
  max: number;
}

export interface TwinMetrics {
  /** 20 log10 of the residual over the signal, clamped to [-200, 200]; -200 for equal. */
  residualDb: number;
  /** Samples, over all channels, whose float bit patterns differ. */
  differingFrames: number;
  /** First differing sample, or null. */
  firstDiff: { frame: number; channel: number } | null;
  bands: Record<string, BandQuantiles>;
}

/** Nearest-rank quantiles: q(p) = sorted[round(p * (n - 1))]; all 0 for an empty list. */
export function bandQuantiles(values: number[]): BandQuantiles {
  const s = sortedAsc(values);
  if (s.length === 0) return { cells: 0, p50: 0, p95: 0, max: 0 };
  return { cells: s.length, p50: nearestRank(s, 0.5), p95: nearestRank(s, 0.95), max: s.at(-1)! };
}

export function twinMetrics(a: Float32Array[], b: Float32Array[], sampleRate: number): TwinMetrics {
  let differingFrames = 0;
  let firstDiff: TwinMetrics['firstDiff'] = null;
  for (let c = 0; c < Math.min(a.length, b.length); c++) {
    const ua = new Uint32Array(a[c]!.buffer, a[c]!.byteOffset, a[c]!.length);
    const ub = new Uint32Array(b[c]!.buffer, b[c]!.byteOffset, b[c]!.length);
    const n = Math.min(ua.length, ub.length);
    for (let i = 0; i < n; i++) {
      if (ua[i] !== ub[i]) {
        differingFrames++;
        if (firstDiff === null || i < firstDiff.frame) firstDiff = { frame: i, channel: c };
      }
    }
  }

  const ma = mixToMono(a);
  const mb = mixToMono(b);
  const n = Math.min(ma.length, mb.length);
  let diff = 0;
  let ref = 0;
  for (let i = 0; i < n; i++) {
    const d = ma[i]! - mb[i]!;
    diff += d * d;
    ref += mb[i]! * mb[i]!;
  }
  const raw = diff === 0 ? -Infinity : ampToDb(Math.sqrt(diff / Math.max(ref, 1e-30)), 0);
  const residualDb = Math.min(200, Math.max(-200, raw));

  const bands: Record<string, BandQuantiles> = {};
  const bandRms = (x: Float32Array, lo: number, hi: number) =>
    rmsWindows(biquadBandpass(x.subarray(0, n), sampleRate, lo, hi), sampleRate, TWIN_WINDOW_SEC);
  for (const [name, lo, hi] of SEAM_BANDS) {
    const ea = bandRms(ma, lo, hi);
    const eb = bandRms(mb, lo, hi);
    const deltas: number[] = [];
    for (let k = 0; k < ea.length; k++) {
      if (Math.max(ea[k]!, eb[k]!) < TWIN_FLOOR) continue;
      deltas.push(Math.abs(ratioDb(ea[k]!, eb[k]!, TWIN_FLOOR)));
    }
    bands[name] = bandQuantiles(deltas);
  }
  return { residualDb, differingFrames, firstDiff, bands };
}

const lengthOf = (x: Float32Array[]): number | null => {
  const n = x[0]?.length ?? 0;
  return x.every((ch) => ch.length === n) ? n : null;
};

/** The twin verdict for renders `a` and `b` of the same request. */
export function twinNull(a: Float32Array[], b: Float32Array[], opts: TwinOptions): GateResult {
  const fa = lengthOf(a);
  const fb = lengthOf(b);
  const base = { mode: opts.mode, frames: fa ?? a[0]?.length ?? 0, channels: a.length };
  const broken = (reasons: string[]): GateResult => ({
    gate: 'twin',
    verdict: 'fail',
    measurements: { ...base, differingFrames: null, residualDb: null, bands: null },
    reasons,
    warnings: [],
  });

  const shape: string[] = [];
  if (a.length !== b.length) shape.push(`channel count differs: ${a.length} vs ${b.length}`);
  if (fa === null || fb === null) shape.push('channels of one render have unequal lengths');
  else if (fa !== fb) shape.push(`frame count differs: ${fa} vs ${fb}`);
  if (shape.length > 0) return broken(shape);

  const bad: string[] = [];
  for (const [label, x] of [
    ['a', a],
    ['b', b],
  ] as const) {
    const nf = firstNonFinite(x);
    if (nf.count > 0) {
      bad.push(
        `${nf.count} non-finite samples in ${label}, first at frame ${nf.frame} ch ${nf.channel}`,
      );
    }
  }
  if (bad.length > 0) return broken(bad);

  const m = twinMetrics(a, b, opts.sampleRate);
  const bands: { [k: string]: Json } = {};
  for (const [name, q] of Object.entries(m.bands)) {
    bands[name] = { cells: q.cells, p50: roundDb(q.p50), p95: roundDb(q.p95), max: roundDb(q.max) };
  }
  const measurements = {
    ...base,
    differingFrames: m.differingFrames,
    residualDb: roundDb(m.residualDb),
    bands,
  };
  const reasons: string[] = [];
  let verdict: GateResult['verdict'] = 'pass';

  if (opts.mode === 'exact') {
    if (m.differingFrames > 0) {
      verdict = 'fail';
      reasons.push(
        `${m.differingFrames} samples differ, first at frame ${m.firstDiff!.frame} ch ${m.firstDiff!.channel}`,
      );
    }
  } else if (Object.values(m.bands).every((q) => q.cells === 0)) {
    verdict = 'unverifiable';
    reasons.push('both renders are silent in every band');
  } else {
    for (const [name, q] of Object.entries(m.bands)) {
      if (q.p95 > TWIN_P95_DB) {
        reasons.push(`${name} band p95 ${roundDb(q.p95)} dB > ${TWIN_P95_DB} dB`);
      }
      if (q.max > TWIN_MAX_DB) {
        reasons.push(`${name} band max ${roundDb(q.max)} dB > ${TWIN_MAX_DB} dB`);
      }
    }
    if (m.residualDb > TWIN_RESIDUAL_DB) {
      reasons.push(`residual ${roundDb(m.residualDb)} dB > ${TWIN_RESIDUAL_DB} dB`);
    }
    if (reasons.length > 0) verdict = 'fail';
  }
  return { gate: 'twin', verdict, measurements, reasons, warnings: [] };
}

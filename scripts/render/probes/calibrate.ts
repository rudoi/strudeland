/**
 * Pure calibration math shared by the probes (`*.probe.ts`): quantiles, rounding up to a step,
 * the twin-null decisions, the sweep rows and the threshold choice rules of 04 section 6. No
 * Playwright, no I/O; tested under `bun test`.
 */

import { nearestRank, round, sortedAsc } from '../../../src/lib/render/dsp';
import type { BassRule } from '../../../src/lib/render/gates/bass';

export interface Quantiles {
  n: number;
  p50: number;
  p95: number;
  p99: number;
  max: number;
}

/** Nearest-rank quantiles, q(p) = sorted[round(p * (n - 1))]; all 0 for an empty list. */
export function quantiles(values: number[]): Quantiles {
  const s = sortedAsc(values);
  if (s.length === 0) return { n: 0, p50: 0, p95: 0, p99: 0, max: 0 };
  const q = (p: number) => nearestRank(s, p);
  return { n: s.length, p50: q(0.5), p95: q(0.95), p99: q(0.99), max: s.at(-1)! };
}

/** Rounds `x` up to a multiple of `step`, never below one step (a calibrated tolerance of 0
 *  would fail on the next float-level wobble). The result is rounded to the step's decimals. */
export function ceilTo(x: number, step: number): number {
  return stepMultiple(step, Math.max(1, Math.ceil(x / step - 1e-9)));
}

/** Like ceilTo but for signed values (a residual in dB): plain ceiling to the step. */
export function ceilSigned(x: number, step: number): number {
  return stepMultiple(step, Math.ceil(x / step - 1e-9));
}

/** `k * step`, rounded to the step's decimals. */
function stepMultiple(step: number, k: number): number {
  if (!(step > 0)) throw new RangeError(`step must be > 0 (got ${step})`);
  const decimals = Math.max(0, Math.ceil(-Math.log10(step) - 1e-9));
  return Number((k * step).toFixed(decimals));
}

export interface BandStats {
  p50: number;
  p95: number;
  max: number;
}

export interface TwinFixtureStats {
  name: string;
  stochastic: boolean;
  bitEqual: boolean;
  /** Worst pair, dB (-200 for bit-equal). */
  residualDb: number;
  bands: Record<string, BandStats>;
  lufsDelta: number;
  peakDeltaDb: number;
}

export interface TwinDecision {
  TWIN_BITEXACT_HOLDS: boolean;
  TWIN_P95_DB: number;
  TWIN_MAX_DB: number;
  TWIN_RESIDUAL_DB: number;
  worst: {
    p95: number;
    bandMax: number;
    residualDb: number;
    lufsDelta: number;
    peakDeltaDb: number;
  };
  self: { bandMeanDb: number; bandP95Db: number; lufsDb: number; peakDb: number };
  bitEqual: string[];
  notBitEqual: string[];
}

/** 04 section 6 probe 1 decisions over every fixture's twin statistics. */
export function twinDecision(stats: TwinFixtureStats[]): TwinDecision {
  if (stats.length === 0) throw new RangeError('no twin statistics');
  const bandsOf = (s: TwinFixtureStats) => Object.values(s.bands);
  const p95 = Math.max(0, ...stats.flatMap((s) => bandsOf(s).map((b) => b.p95)));
  const bandMax = Math.max(0, ...stats.flatMap((s) => bandsOf(s).map((b) => b.max)));
  const residualDb = Math.max(...stats.map((s) => s.residualDb));
  const lufsDelta = Math.max(0, ...stats.map((s) => Math.abs(s.lufsDelta)));
  const peakDeltaDb = Math.max(0, ...stats.map((s) => Math.abs(s.peakDeltaDb)));
  return {
    TWIN_BITEXACT_HOLDS: stats.filter((s) => !s.stochastic).every((s) => s.bitEqual),
    TWIN_P95_DB: ceilTo(1.25 * p95, 0.05),
    TWIN_MAX_DB: ceilTo(1.25 * bandMax, 0.1),
    TWIN_RESIDUAL_DB: ceilSigned(residualDb + 3, 1),
    worst: { p95, bandMax, residualDb, lufsDelta, peakDeltaDb },
    self: {
      bandMeanDb: Math.max(0.1, 2 * p95),
      bandP95Db: Math.max(0.5, 3 * p95),
      lufsDb: Math.max(0.05, 2 * lufsDelta),
      peakDb: Math.max(0.1, 2 * peakDeltaDb),
    },
    bitEqual: stats.filter((s) => s.bitEqual).map((s) => s.name),
    notBitEqual: stats.filter((s) => !s.bitEqual).map((s) => s.name),
  };
}

/** One threshold of a sweep. A unit is a seam, an orbit-bar or a bass note. */
export interface SweepRow {
  db: number;
  recall: number;
  falseAlarms: number;
  cleanUnits: number;
  mutatedUnits: number;
}

export function sweepRow(
  db: number,
  caught: number,
  mutatedUnits: number,
  falseAlarms: number,
  cleanUnits: number,
): SweepRow {
  return {
    db,
    recall: mutatedUnits > 0 ? round(caught / mutatedUnits, 3) : 0,
    falseAlarms,
    cleanUnits,
    mutatedUnits,
  };
}

export interface Choice {
  db: number;
  rule: 'clean' | 'fallback';
  reason: string;
}

/** The smallest threshold with no false alarm and recall >= 0.9; else the one with maximal
 *  recall and at most one false alarm (smallest on ties); null when neither exists. */
export function chooseThreshold(rows: SweepRow[]): Choice | null {
  const sorted = [...rows].sort((a, b) => a.db - b.db);
  const clean = sorted.find((r) => r.falseAlarms === 0 && r.recall >= 0.9);
  if (clean) {
    return {
      db: clean.db,
      rule: 'clean',
      reason: `smallest threshold with 0 false alarms and recall >= 0.9 (recall ${clean.recall})`,
    };
  }
  const ok = sorted.filter((r) => r.falseAlarms <= 1);
  if (ok.length === 0) return null;
  const best = ok.reduce((b, r) => (r.recall > b.recall ? r : b));
  return {
    db: best.db,
    rule: 'fallback',
    reason:
      `no threshold has 0 false alarms with recall >= 0.9; maximal recall ${best.recall} with ` +
      `${best.falseAlarms} false alarm(s)`,
  };
}

/** Seam fail verdict at threshold `t`, mirroring judgeSeam with escalation: the twin floor is
 *  only added once the worst deficit is over the line. */
export function seamFlagged(
  worstDeficitDb: number,
  twinFloorDb: number | null,
  t: number,
): boolean {
  if (worstDeficitDb <= t) return false;
  return worstDeficitDb > t + (twinFloorDb ?? 0);
}

export type RuleKey = Exclude<keyof BassRule, 'fractionMinNotes' | 'minNotes'>;

/** Every combination of the swept bass rule fields. */
export function ruleGrid(axes: Record<RuleKey, number[]>): Pick<BassRule, RuleKey>[] {
  const out: Pick<BassRule, RuleKey>[] = [];
  for (const clusterMin of axes.clusterMin) {
    for (const clusterSpanBars of axes.clusterSpanBars) {
      for (const clusterDeepDb of axes.clusterDeepDb) {
        for (const fractionMax of axes.fractionMax) {
          out.push({ clusterMin, clusterSpanBars, clusterDeepDb, fractionMax });
        }
      }
    }
  }
  return out;
}

/** The valid rule closest to `start` (sum of per-axis distances in axis steps), or null. Ties
 *  go to the earlier rule in `valid`. */
export function closestRule(
  valid: Pick<BassRule, RuleKey>[],
  start: Pick<BassRule, RuleKey>,
  steps: Record<RuleKey, number>,
): Pick<BassRule, RuleKey> | null {
  let best: Pick<BassRule, RuleKey> | null = null;
  let bestD = Infinity;
  for (const r of valid) {
    let d = 0;
    for (const k of Object.keys(steps) as RuleKey[]) d += Math.abs(r[k] - start[k]) / steps[k];
    if (d < bestD - 1e-9) {
      best = r;
      bestD = d;
    }
  }
  return best;
}

/** `[from, from + step, …]` up to `to` inclusive, without float drift. */
export function range(from: number, to: number, step: number): number[] {
  const out: number[] = [];
  const n = Math.floor((to - from) / step + 1e-9);
  for (let i = 0; i <= n; i++) out.push(Number((from + i * step).toFixed(6)));
  return out;
}

// ---- engine-null (the Rust engine against superdough)

/** `20 log10(rms(ref - cand) / rms(ref))` over every sample of every channel, as the engine's
 *  micro test computes it (`ref` = Chromium): -Infinity when bit-equal, +Infinity when `ref` is
 *  silent and `cand` is not, null when the shapes differ. */
export function nullResidualDb(ref: Float32Array[], cand: Float32Array[]): number | null {
  if (ref.length !== cand.length) return null;
  let diff = 0;
  let power = 0;
  for (let c = 0; c < ref.length; c++) {
    const a = ref[c]!;
    const b = cand[c]!;
    if (a.length !== b.length) return null;
    for (let i = 0; i < a.length; i++) {
      const d = a[i]! - b[i]!;
      diff += d * d;
      power += a[i]! * a[i]!;
    }
  }
  if (Number.isNaN(diff) || Number.isNaN(power)) return null;
  if (diff === 0) return -Infinity;
  if (power === 0) return Infinity;
  return 10 * Math.log10(diff / power);
}

export interface WorstWindow {
  /** Start of the window, seconds from frame 0. */
  startSec: number;
  /** Window residual relative to the reference window, dB (as nullResidualDb). */
  residualDb: number;
  /** RMS of the difference, dBFS. */
  diffDbfs: number;
  /** RMS of the reference window, dBFS. */
  refDbfs: number;
}

/** The window (`windowSec`, all channels) with the largest residual among those whose
 *  reference RMS is at least `floorDb` dBFS; null when none is, or the shapes differ. Ties keep
 *  the earlier window. */
export function worstWindow(
  ref: Float32Array[],
  cand: Float32Array[],
  sampleRate: number,
  windowSec = 0.1,
  floorDb = -60,
): WorstWindow | null {
  if (ref.length !== cand.length || ref.length === 0) return null;
  const n = ref[0]!.length;
  if (ref.some((c) => c.length !== n) || cand.some((c) => c.length !== n)) return null;
  const w = Math.max(1, Math.round(windowSec * sampleRate));
  let best: WorstWindow | null = null;
  for (let start = 0; start < n; start += w) {
    const end = Math.min(n, start + w);
    let diff = 0;
    let power = 0;
    for (let c = 0; c < ref.length; c++) {
      for (let i = start; i < end; i++) {
        const d = ref[c]![i]! - cand[c]![i]!;
        diff += d * d;
        power += ref[c]![i]! * ref[c]![i]!;
      }
    }
    const count = (end - start) * ref.length;
    const refDbfs = 10 * Math.log10(power / count);
    if (!(refDbfs >= floorDb)) continue;
    const residualDb = diff === 0 ? -Infinity : 10 * Math.log10(diff / power);
    if (best === null || residualDb > best.residualDb) {
      best = {
        startSec: start / sampleRate,
        residualDb,
        diffDbfs: diff === 0 ? -Infinity : 10 * Math.log10(diff / count),
        refDbfs,
      };
    }
  }
  return best;
}

/** A dB value for evidence JSON: rounded to 0.01, infinities as "-inf"/"inf". */
export function dbJson(x: number | null): number | string | null {
  if (x === null || Number.isNaN(x)) return null;
  if (x === -Infinity) return '-inf';
  if (x === Infinity) return 'inf';
  return round(x, 2);
}

/** The null-residual target of a micro case: delay and reverb cases (the local ones of an FX
 *  chain too) -40 dB, every other case (oscillators, noise, envelopes, filters, pan, levels)
 *  -60 dB. */
export function microTargetDb(name: string): number {
  return /^(delay-|reverb-|measure-feedback|fx-chain-(delay|room))/.test(name) ? -40 : -60;
}

/** The verdict columns of one micro case row of `engine-null.probe.ts`. `meetsThreshold` is
 *  `null` when the case's `platform` does not apply on `host` (it is rendered and reported but
 *  left out of `microThresholdsMet`), else whether the residual is at
 *  most `maxResidualDb`. A floor case (`floorDb` from `index.json`) also gets `meetsFloorRule`:
 *  the threshold lies above the floor and the residual is within the threshold. */
export interface MicroVerdict {
  meetsThreshold: boolean | null;
  floorDb?: number;
  meetsFloorRule?: boolean;
}

export function microVerdict(args: {
  residual: number | null;
  maxResidualDb: number | undefined;
  platform?: string;
  host: string;
  floorDb?: number;
}): MicroVerdict {
  const { residual, maxResidualDb: max, platform, host, floorDb } = args;
  const applies = platform === undefined || platform === host;
  const within = residual !== null && max !== undefined && residual <= max;
  const v: MicroVerdict = { meetsThreshold: applies ? within : null };
  if (floorDb !== undefined) {
    v.floorDb = floorDb;
    v.meetsFloorRule = max !== undefined && max > floorDb && within;
  }
  return v;
}

/** `process.platform` as `thresholds.json` names it (`darwin` is `"macos"`). */
export function platformName(nodePlatform: string): string {
  return nodePlatform === 'darwin' ? 'macos' : nodePlatform;
}

/** One supported fixture's deltas: its Rust render against the superdough golden
 *  (`compareFeatures`, as `render:check --engine rust` computes them) and the superdough twin
 *  floor (two superdough renders of chunk 0). */
export interface EngineFixtureStats {
  name: string;
  parity: {
    frameDelta: number;
    offsetSec: number;
    peakDeltaDb: number;
    lufsDeltaDb: number | null;
    bandMeanDeltaDb: Record<string, number>;
    bandP95DeltaDb: Record<string, number>;
  };
  twin: { peakDeltaDb: number; lufsDelta: number; bandP95: number };
}

export interface EngineDecision {
  engine: {
    name: 'engine';
    frameTolerance: number;
    maxOffsetSec: number;
    peakDb: number;
    lufsDb: number;
    bandMeanDb: number;
    bandP95Db: number;
    hashMustMatch: false;
  };
  worst: {
    frameDelta: number;
    offsetSec: number;
    peakDb: number;
    lufsDb: number;
    bandMeanDb: number;
    bandP95Db: number;
    /** The fixture behind each worst value. */
    from: Record<string, string>;
  };
  rules: Record<string, string>;
}

/** `PROFILES.engine` from the fixtures' parity deltas against the goldens and the superdough
 *  twin floor (the golden's own noise), with the margins and floors `twinDecision` gives
 *  `PROFILES.self`: no tolerance below the floor that the self profile needs for superdough's
 *  float-level wobble. */
export function engineDecision(stats: EngineFixtureStats[]): EngineDecision {
  if (stats.length === 0) throw new RangeError('no engine statistics');
  const from: Record<string, string> = {};
  const worstOf = (key: string, value: (s: EngineFixtureStats) => number): number => {
    let w = 0;
    for (const s of stats) {
      const v = Math.abs(value(s));
      if (v > w) {
        w = v;
        from[key] = s.name;
      }
    }
    return w;
  };
  const maxAbs = (r: Record<string, number>) => Math.max(0, ...Object.values(r).map(Math.abs));
  const frameDelta = worstOf('frameDelta', (s) => s.parity.frameDelta);
  const offsetSec = worstOf('offsetSec', (s) => s.parity.offsetSec);
  const peakDb = worstOf('peakDb', (s) =>
    Math.max(Math.abs(s.parity.peakDeltaDb), s.twin.peakDeltaDb),
  );
  const lufsDb = worstOf('lufsDb', (s) =>
    Math.max(Math.abs(s.parity.lufsDeltaDb ?? 0), s.twin.lufsDelta),
  );
  const bandMeanDb = worstOf('bandMeanDb', (s) =>
    Math.max(maxAbs(s.parity.bandMeanDeltaDb), s.twin.bandP95),
  );
  const bandP95Db = worstOf('bandP95Db', (s) =>
    Math.max(maxAbs(s.parity.bandP95DeltaDb), s.twin.bandP95),
  );
  return {
    engine: {
      name: 'engine',
      frameTolerance: frameDelta,
      maxOffsetSec: Math.max(0.01, ceilTo(2 * offsetSec, 0.01)),
      peakDb: Math.max(0.1, ceilTo(2 * peakDb, 0.01)),
      lufsDb: Math.max(0.05, ceilTo(2 * lufsDb, 0.01)),
      bandMeanDb: Math.max(0.1, ceilTo(2 * bandMeanDb, 0.01)),
      bandP95Db: Math.max(0.5, ceilTo(3 * bandP95Db, 0.05)),
      hashMustMatch: false,
    },
    worst: { frameDelta, offsetSec, peakDb, lufsDb, bandMeanDb, bandP95Db, from },
    rules: {
      frameTolerance: 'worst |frameDelta| (frames are planned identically, so 0)',
      maxOffsetSec: 'max(0.01, 2 x worst |offsetSec|), as self',
      peakDb: 'max(0.1, 2 x worst |peak delta|, golden or twin), as self',
      lufsDb: 'max(0.05, 2 x worst |LUFS delta|, golden or twin), as self',
      bandMeanDb: 'max(0.1, 2 x worst |band mean delta| or twin band p95), as self',
      bandP95Db: 'max(0.5, 3 x worst |band p95 delta| or twin band p95), as self',
    },
  };
}

/** Per-chunk render times of one fixture, in milliseconds (`RenderResult.timing.renderMs`). */
export interface FixtureTiming {
  rustRenderMs: number[];
  superdoughRenderMs: number[];
}

/** The timing row of one fixture: each chunk's `timing.renderMs`, rounded to 1 ms, in chunk
 *  order. Throws when the engines rendered a different number of chunks or a time is not a
 *  finite non-negative number. */
export function fixtureTiming(
  superdough: readonly { timing: { renderMs: number } }[],
  rust: readonly { timing: { renderMs: number } }[],
): FixtureTiming {
  if (superdough.length !== rust.length) {
    throw new RangeError(
      `chunk counts differ: superdough ${superdough.length}, rust ${rust.length}`,
    );
  }
  const ms = (r: { timing: { renderMs: number } }) => {
    const v = r.timing.renderMs;
    if (!Number.isFinite(v) || v < 0) throw new RangeError(`bad render time ${v}`);
    return Math.round(v);
  };
  return { rustRenderMs: rust.map(ms), superdoughRenderMs: superdough.map(ms) };
}

/** `decision.renderTimesSec`: each fixture's Rust render time, the sum of its chunk times in
 *  seconds rounded to 0.01 s, keyed by fixture name in name order. */
export function renderTimesSec(
  rows: readonly { name: string; timing: FixtureTiming }[],
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const r of [...rows].sort((a, b) => a.name.localeCompare(b.name))) {
    if (r.name in out) throw new RangeError(`duplicate fixture ${r.name}`);
    const sum = r.timing.rustRenderMs.reduce((a, b) => a + b, 0);
    out[r.name] = Math.round(sum / 10) / 100;
  }
  return out;
}

/**
 * Parity: compare candidate features against a golden under a tolerance profile. Pure.
 */

import { alignOnsets } from './align';
import { powerToDb, round } from './dsp';
import { ENGINE_TRAITS, type EngineName } from './engines';
import type { Features } from './features';

export interface ParityProfile {
  name: 'self' | 'engine';
  /** |candidate.frames - golden.frames| allowed. */
  frameTolerance: number;
  /** Allowed |alignment offset| in seconds; beyond it the verdict fails. */
  maxOffsetSec: number;
  /** |peak delta|, dB. */
  peakDb: number;
  /** |LUFS delta|, LU. */
  lufsDb: number;
  /** |level-matched band mean delta|, dB, per band. */
  bandMeanDb: number;
  /** p95 of |per-window band delta| after level matching, per band. */
  bandP95Db: number;
  /** When true, an equal pcmSha256 passes every check immediately (self profile only). */
  hashMustMatch: boolean;
}

// `self` is calibrated from the twin-null probe; `engine` (Rust renders judged against the
// superdough goldens) from the engine-null probe.
export const PROFILES: { self: ParityProfile; engine: ParityProfile } = {
  self: {
    name: 'self',
    frameTolerance: 0,
    maxOffsetSec: 0.01,
    // calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.self.peakDb
    // (floor 0.1: worst peak delta 0)
    peakDb: 0.1,
    // calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.self.lufsDb
    // (floor 0.05: worst lufsDelta 0; was 0.08)
    lufsDb: 0.05,
    // calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.self.bandMeanDb
    // (floor 0.1: worst p95 9e-7; was 0.53)
    bandMeanDb: 0.1,
    // calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.self.bandP95Db
    // (floor 0.5: worst p95 9e-7; was 0.79)
    bandP95Db: 0.5,
    hashMustMatch: false,
  },
  engine: {
    name: 'engine',
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.frameTolerance
    // (worst frame delta 0)
    frameTolerance: 0,
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.maxOffsetSec
    // (floor 0.01: worst offset 0)
    maxOffsetSec: 0.01,
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.peakDb
    // (floor 0.1: worst peak delta 0.01, due to superdough cutting voice tails on disconnect)
    peakDb: 0.1,
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.lufsDb
    // (floor 0.05: worst LUFS delta 0)
    lufsDb: 0.05,
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.bandMeanDb
    // (floor 0.1: worst band mean delta 0.02, in the high band from drift in the golden)
    bandMeanDb: 0.1,
    // calibrated: see plans/0010-rust-engine-core/evidence/engine-null-final.json decision.engine.bandP95Db
    // (floor 0.5: worst band p95 delta 0.13, in the high band from drift in the golden)
    bandP95Db: 0.5,
    hashMustMatch: false,
  },
};

/** The parity profile for renders of `engine`, judged against the superdough goldens. */
export function profileFor(engine: EngineName): ParityProfile {
  return PROFILES[ENGINE_TRAITS[engine].parityProfile];
}

export interface ParityResult {
  verdict: 'pass' | 'fail' | 'unverifiable';
  profile: string;
  frameDelta: number;
  offsetSec: number;
  correlation: number;
  aligned: boolean;
  peakDeltaDb: number;
  lufsDeltaDb: number | null;
  bandMeanDeltaDb: Record<string, number>;
  bandP95DeltaDb: Record<string, number>;
  hashEqual: boolean;
  /** Non-failing findings. */
  warnings: string[];
  /** One line per failed or skipped check, with measured vs allowed. */
  reasons: string[];
}

const MIN_COMMON_WINDOWS = 10;
const BAND_WINDOW_SEC = 0.1;
const SKIP_DB = -90;

const r2 = (v: number) => round(v, 2);

function meanPowerDb(a: number[], from: number, n: number): number {
  let s = 0;
  for (let i = 0; i < n; i++) s += 10 ** (a[from + i]! / 10);
  return powerToDb(s / n);
}

export function compareFeatures(
  golden: Features,
  candidate: Features,
  profile: ParityProfile,
): ParityResult {
  const res: ParityResult = {
    verdict: 'pass',
    profile: profile.name,
    frameDelta: candidate.frames - golden.frames,
    offsetSec: 0,
    correlation: 0,
    aligned: false,
    peakDeltaDb: r2(candidate.peakDb - golden.peakDb),
    lufsDeltaDb: null,
    bandMeanDeltaDb: {},
    bandP95DeltaDb: {},
    hashEqual: golden.pcmSha256 === candidate.pcmSha256,
    warnings: [],
    reasons: [],
  };
  const unverifiable = (reason: string): ParityResult => {
    res.verdict = 'unverifiable';
    res.reasons.push(reason);
    return res;
  };

  if (golden.schema !== candidate.schema) return unverifiable('schema differs');
  if (golden.sampleRate !== candidate.sampleRate) {
    return unverifiable(`sample rate differs: ${golden.sampleRate} vs ${candidate.sampleRate}`);
  }

  if (res.hashEqual && profile.hashMustMatch) {
    res.aligned = true;
    res.correlation = 1;
    res.peakDeltaDb = 0;
    res.lufsDeltaDb = golden.lufs === null ? null : 0;
    for (const b of Object.keys(golden.bandsDb)) {
      res.bandMeanDeltaDb[b] = 0;
      res.bandP95DeltaDb[b] = 0;
    }
    return res;
  }
  let failed = false;
  const fail = (reason: string) => {
    failed = true;
    res.reasons.push(reason);
  };
  if (!res.hashEqual && profile.hashMustMatch) fail('pcm hash differs');

  if (Math.abs(res.frameDelta) > profile.frameTolerance) {
    fail(`frames differ by ${res.frameDelta} (allowed ${profile.frameTolerance})`);
  }

  const al = alignOnsets(golden.onset, candidate.onset, { hopSec: 0.01, maxLagSec: 3 });
  let shift = 0;
  if (al.trusted) {
    res.aligned = true;
    res.offsetSec = r2(al.offsetSec);
    res.correlation = r2(al.correlation);
    shift = Math.round(al.offsetSec / BAND_WINDOW_SEC);
    if (Math.abs(al.offsetSec) > profile.maxOffsetSec) {
      fail(`offset ${r2(al.offsetSec)} s exceeds ${profile.maxOffsetSec} s`);
    }
  } else {
    res.correlation = r2(al.correlation);
    res.reasons.push(`not aligned: ${al.note ?? 'onsets not trusted'}`);
  }

  // Candidate index = golden index + shift.
  const names = Object.keys(golden.bandsDb) as (keyof Features['bandsDb'])[];
  const gLen = golden.bandsDb.sub.length;
  const cLen = candidate.bandsDb.sub.length;
  const gFrom = Math.max(0, -shift);
  const common = Math.min(gLen - gFrom, cLen - (gFrom + shift));
  if (common < MIN_COMMON_WINDOWS) {
    return unverifiable(
      `only ${Math.max(0, common)} common band windows (need ${MIN_COMMON_WINDOWS})`,
    );
  }
  const cFrom = gFrom + shift;

  if (Math.abs(res.peakDeltaDb) > profile.peakDb) {
    fail(`peak delta ${res.peakDeltaDb} dB exceeds ${profile.peakDb} dB`);
  }
  if (golden.lufs !== null && candidate.lufs !== null) {
    res.lufsDeltaDb = r2(candidate.lufs - golden.lufs);
    if (Math.abs(res.lufsDeltaDb) > profile.lufsDb) {
      fail(`LUFS delta ${res.lufsDeltaDb} LU exceeds ${profile.lufsDb} LU`);
    }
  } else {
    res.reasons.push('LUFS check skipped: loudness is null');
  }
  const level = res.lufsDeltaDb ?? 0;

  for (const name of names) {
    const g = golden.bandsDb[name];
    const c = candidate.bandsDb[name];
    const gMean = meanPowerDb(g, gFrom, common);
    if (gMean < SKIP_DB) {
      res.reasons.push(`band ${name} skipped: golden mean ${r2(gMean)} dB is below ${SKIP_DB} dB`);
      continue;
    }
    const cMean = meanPowerDb(c, cFrom, common);
    const meanDelta = cMean - gMean - level;
    res.bandMeanDeltaDb[name] = r2(meanDelta);
    const diffs: number[] = [];
    for (let i = 0; i < common; i++) {
      const gv = g[gFrom + i]!;
      const cv = c[cFrom + i]!;
      if (gv > SKIP_DB && cv > SKIP_DB) diffs.push(Math.abs(cv - gv - level));
    }
    diffs.sort((a, b) => a - b);
    const p95 = diffs.length === 0 ? 0 : diffs[Math.ceil(0.95 * diffs.length) - 1]!;
    res.bandP95DeltaDb[name] = r2(p95);
    if (Math.abs(res.bandMeanDeltaDb[name]!) > profile.bandMeanDb) {
      fail(
        `band ${name} mean delta ${res.bandMeanDeltaDb[name]} dB exceeds ${profile.bandMeanDb} dB`,
      );
    }
    if (res.bandP95DeltaDb[name]! > profile.bandP95Db) {
      fail(`band ${name} p95 delta ${res.bandP95DeltaDb[name]} dB exceeds ${profile.bandP95Db} dB`);
    }
  }

  if (!res.hashEqual && !profile.hashMustMatch) res.warnings.push('pcm hash differs (advisory)');
  res.verdict = failed ? 'fail' : 'pass';
  return res;
}

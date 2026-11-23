/**
 * The bass gate: energy at each low note's own fundamental right after its onset, gain-normalized
 * and compared to the song's median. Runs on the stereo mix, so it needs no stems. Pure. The
 * earlier render harness supplies the event and scoring logic. This gate adds the verdict rule.
 *
 * In earlier calibration, a 12 dB threshold caught 41% of removed notes with 2 false alarms
 * in 5 clean windows (phase cancellation at -12.6 to -13.6 dB). A dropped 3-of-8-bars passage
 * of about 8-20 notes puts 3 or more deep misses in one orbit within
 * 4 bars with high probability; a lone false alarm does not. Whether the 15 dB depth filter leaves
 * enough recall is what the calibration bass mutation sweep decides; if not, lower
 * BASS_CLUSTER_DEEP_DB toward BASS_MISS_DB and rely on the fraction and cluster counts.
 */

import { biquadBandpass, mixToMono, ratioDb, rms, round } from '../dsp';
import type { RenderEvent } from '../schedule';
import {
  BASS_BAND_RATIO,
  BASS_CLUSTER_DEEP_DB,
  BASS_CLUSTER_MIN,
  BASS_CLUSTER_SPAN_BARS,
  BASS_FRACTION_MAX,
  BASS_FRACTION_MIN_NOTES,
  BASS_GAIN_FLOOR,
  BASS_MAX_ATTACK_SEC,
  BASS_MAX_HZ,
  BASS_MIN_GAIN,
  BASS_MIN_HZ,
  BASS_MIN_NOTES,
  BASS_MISS_DB,
  BASS_PAD_SEC,
  BASS_WINDOW_SEC,
} from '../thresholds';
import { roundDb, scoreBar, type GateResult } from './types';

export interface BassEvent extends RenderEvent {
  freq: number;
}

export interface BassMiss {
  tSec: number;
  /** Absolute layout bar of the scheduled note. */
  bar: number;
  hz: number;
  orbit: number;
  s: string;
  /** Score relative to the median score, dB (2 decimals). */
  relDb: number;
}

export interface BassPresence {
  checked: number;
  misses: BassMiss[];
  /** Median gain-normalized band RMS; 0 when nothing was checked. */
  referenceRms: number;
}

export interface BassOptions {
  missDb: number;
  /** Layout seconds of frame 0 of the buffer; default 0. */
  offsetSec?: number;
  /** Constant bar duration for a fixed-meter fixture; omit to use the scheduled bars. */
  barSec?: number;
}

export interface BassRule {
  clusterMin: number;
  clusterSpanBars: number;
  clusterDeepDb: number;
  fractionMax: number;
  fractionMinNotes: number;
  minNotes: number;
}

const NOISE = /^(white|pink|brown|crackle)$/;

/** Low notes the schedule says are there: pitched, no pitch envelope (a kick's sweep defeats an
 *  f0 test), not noise, fast attack, audible, BASS_MIN_HZ..maxHz. */
export function bassEvents(
  events: RenderEvent[],
  opts: { maxHz?: number; minGain?: number } = {},
): BassEvent[] {
  const maxHz = opts.maxHz ?? BASS_MAX_HZ;
  const minGain = opts.minGain ?? BASS_MIN_GAIN;
  const out: BassEvent[] = [];
  for (const e of events) {
    if (NOISE.test(e.s) || e.penv !== 0) continue;
    if (e.attack >= BASS_MAX_ATTACK_SEC || e.gain < minGain) continue;
    const hz = e.freq;
    if (hz === null || !Number.isFinite(hz) || hz < BASS_MIN_HZ || hz > maxHz) continue;
    out.push({ ...e, freq: hz });
  }
  return out;
}

/** Scores every bass note fully inside the buffer on the mono mix; a note more than `missDb`
 *  below the median score is a miss. Song-relative on purpose: accents and filters vary level. */
export function bassPresence(
  channels: Float32Array[],
  sampleRate: number,
  events: RenderEvent[],
  opts: BassOptions,
): BassPresence {
  const x = mixToMono(channels);
  const offset = opts.offsetSec ?? 0;
  const win = Math.round(BASS_WINDOW_SEC * sampleRate);
  const pad = Math.round(BASS_PAD_SEC * sampleRate);
  const scored: { e: BassEvent; score: number }[] = [];
  for (const e of bassEvents(events)) {
    const i0 = Math.round((e.tSec - offset) * sampleRate);
    const i1 = i0 + win;
    if (i0 < 0 || i1 > x.length) continue;
    // About +/- 1 semitone around f0, padded in front so the filter settles.
    const start = Math.max(0, i0 - pad);
    const band = biquadBandpass(
      x.subarray(start, i1),
      sampleRate,
      e.freq / BASS_BAND_RATIO,
      e.freq * BASS_BAND_RATIO,
    );
    const energy = rms(band.subarray(i0 - start));
    scored.push({ e, score: energy / Math.max(e.gain, BASS_GAIN_FLOOR) });
  }
  if (scored.length === 0) return { checked: 0, misses: [], referenceRms: 0 };
  const sorted = scored.map((s) => s.score).sort((a, b) => a - b);
  const ref = sorted[Math.floor(sorted.length / 2)]!;
  const misses: BassMiss[] = [];
  for (const { e, score } of scored) {
    const relDb = ratioDb(score, ref, 1e-12);
    if (relDb < -Math.abs(opts.missDb)) {
      misses.push({
        tSec: e.tSec,
        bar: scoreBar(e, opts.barSec),
        hz: round(e.freq, 1),
        orbit: e.orbit,
        s: e.s,
        relDb: roundDb(relDb),
      });
    }
  }
  return { checked: scored.length, misses, referenceRms: ref };
}

interface Cluster {
  orbit: number;
  fromBar: number;
  toBar: number;
  count: number;
}

/** Per orbit, merged bar ranges where `min` deep misses span at most `spanBars - 1` bars. */
function findClusters(deep: BassMiss[], min: number, spanBars: number): Cluster[] {
  const byOrbit = new Map<number, number[]>();
  for (const m of deep) byOrbit.set(m.orbit, [...(byOrbit.get(m.orbit) ?? []), m.bar]);
  const out: Cluster[] = [];
  for (const [orbit, bars] of [...byOrbit.entries()].sort((a, b) => a[0] - b[0])) {
    const b = bars.sort((p, q) => p - q);
    const ranges: [number, number][] = [];
    for (let i = 0; i + min - 1 < b.length; i++) {
      if (b[i + min - 1]! - b[i]! > spanBars - 1) continue;
      const last = ranges[ranges.length - 1];
      if (last && b[i]! <= last[1]) last[1] = Math.max(last[1], b[i + min - 1]!);
      else ranges.push([b[i]!, b[i + min - 1]!]);
    }
    for (const [fromBar, toBar] of ranges) {
      out.push({
        orbit,
        fromBar,
        toBar,
        count: b.filter((x) => x >= fromBar && x <= toBar).length,
      });
    }
  }
  return out;
}

const MAX_LISTED = 8;

/** The bass verdict. `rule` overrides the BASS_* constants for the calibration sweep only. */
export function bassGate(
  p: BassPresence,
  barTime: number | ((bar: number) => number),
  rule: Partial<BassRule> = {},
): GateResult {
  const r: BassRule = {
    clusterMin: rule.clusterMin ?? BASS_CLUSTER_MIN,
    clusterSpanBars: rule.clusterSpanBars ?? BASS_CLUSTER_SPAN_BARS,
    clusterDeepDb: rule.clusterDeepDb ?? BASS_CLUSTER_DEEP_DB,
    fractionMax: rule.fractionMax ?? BASS_FRACTION_MAX,
    fractionMinNotes: rule.fractionMinNotes ?? BASS_FRACTION_MIN_NOTES,
    minNotes: rule.minNotes ?? BASS_MIN_NOTES,
  };
  const deep = p.misses.filter((m) => m.relDb <= -r.clusterDeepDb);
  const clusters = findClusters(deep, r.clusterMin, r.clusterSpanBars);
  const fraction = p.checked > 0 ? p.misses.length / p.checked : 0;
  const worst = p.misses.reduce((w, m) => Math.min(w, m.relDb), Infinity);
  const measurements = {
    checked: p.checked,
    misses: p.misses.length,
    deepMisses: deep.length,
    fraction: round(fraction, 3),
    referenceRms: round(p.referenceRms, 6),
    worstRelDb: Number.isFinite(worst) ? roundDb(worst) : null,
    clusters: clusters.map((c) => ({ ...c })),
  };

  if (p.checked < r.minNotes) {
    return {
      gate: 'bass',
      verdict: 'unverifiable',
      measurements,
      reasons: [`${p.checked} eligible bass notes`],
      warnings: [],
    };
  }

  const secondsAt = typeof barTime === 'number' ? (bar: number) => bar * barTime : barTime;
  const reasons: string[] = [];
  for (const c of clusters) {
    reasons.push(
      `orbit ${c.orbit}: ${c.count} bass notes missing (<= -${r.clusterDeepDb} dB) in bars ` +
        `${c.fromBar}-${c.toBar} (${secondsAt(c.fromBar).toFixed(2)}s-` +
        `${secondsAt(c.toBar + 1).toFixed(2)}s)`,
    );
  }
  if (p.checked >= r.fractionMinNotes && fraction > r.fractionMax) {
    reasons.push(
      `${p.misses.length}/${p.checked} bass notes missing (${Math.round(fraction * 1000) / 10}% > ` +
        `${r.fractionMax * 100}%)`,
    );
  }

  const inCluster = (m: BassMiss) =>
    m.relDb <= -r.clusterDeepDb &&
    clusters.some((c) => c.orbit === m.orbit && m.bar >= c.fromBar && m.bar <= c.toBar);
  const isolated = p.misses.filter((m) => !inCluster(m));
  const warnings = isolated.slice(0, MAX_LISTED).map((m) => {
    const shallow = m.relDb > -r.clusterDeepDb && m.relDb < -BASS_MISS_DB;
    return (
      `isolated bass miss at bar ${m.bar}, orbit ${m.orbit}, ${m.relDb} dB` +
      (shallow ? ' (shallow, likely phase interaction)' : '')
    );
  });
  if (isolated.length > MAX_LISTED) warnings.push(`+${isolated.length - MAX_LISTED}`);

  return {
    gate: 'bass',
    verdict: reasons.length > 0 ? 'fail' : 'pass',
    measurements,
    reasons,
    warnings,
  };
}

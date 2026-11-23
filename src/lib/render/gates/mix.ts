/**
 * The mix gate on the assembled float window: finite samples, the exact frame count, the opening
 * (when the window starts at bar 0), clicks at chunk seams, and level (a warning only: nothing
 * normalizes in strudeland). Pure (ported from the earlier render harness's opening, mix and
 * seam checks).
 */

import {
  ampToDb,
  clipFraction,
  firstAudibleSec,
  firstNonFinite,
  median,
  peak,
  round,
} from '../dsp';
import type { RenderEvent } from '../schedule';
import type { RenderTiming } from '../timing';
import {
  CLICK_MAX_RATIO,
  CLICK_NEIGHBOR_BARS,
  CLICK_NEIGHBOR_FACTOR,
  CLICK_WINDOW_SEC,
  CLIP_LIMIT,
  FRAME_TOL,
  OPEN_LATE_TOL_SEC,
  OPEN_PERCUSSIVE_HEADROOM,
  OPEN_THRESHOLD_DB,
  OPEN_TOL_SEC,
} from '../thresholds';
import { roundDb, roundSec, type GateResult } from './types';

export interface OpeningExpectation {
  expectedFirstAudibleSec: number;
  hasPercussiveOpener: boolean;
  /** False when no event is loud enough to cross the threshold: a silent opening is then by
   *  schedule. Absent counts as true (a silent opening fails). */
  hasAudibleVoice?: boolean;
}

/** What the schedule predicts for the opening. A linear attack from 0 to `gain` crosses the
 *  threshold `attack * thr / gain` after onset; a fast voice at t = 0 at least
 *  OPEN_PERCUSSIVE_HEADROOM x the threshold makes the opening strict. A 0.015-gain square under
 *  slow pads falls below that cutoff. Event times are layout seconds. */
export function deriveOpening(
  events: RenderEvent[],
  thresholdDb: number = OPEN_THRESHOLD_DB,
): OpeningExpectation {
  const thr = Math.pow(10, thresholdDb / 20);
  let earliest = Infinity;
  let percussive = false;
  for (const e of events) {
    if (!(e.gain > thr)) continue;
    const toThreshold = e.attack > 0 ? e.attack * (thr / e.gain) : 0;
    earliest = Math.min(earliest, e.tSec + toThreshold);
    if (e.tSec < 0.01 && e.attack < 0.02 && e.gain >= thr * OPEN_PERCUSSIVE_HEADROOM) {
      percussive = true;
    }
  }
  return {
    expectedFirstAudibleSec: Number.isFinite(earliest) ? earliest : 0,
    hasPercussiveOpener: percussive,
    hasAudibleVoice: Number.isFinite(earliest),
  };
}

export interface MixInput {
  sampleRate: number;
  /** Exact: windowFrames(fromBar, toBar, cps, sampleRate) of the whole window. */
  expectedFrames: number;
  barSec: number;
  /** The mixed-meter window whose rounded frames are indexed by seamSecs. */
  meteredWindow?: { timing: RenderTiming; fromBar: number };
  /** Chunk seams, seconds from frame 0 of `channels` (AssembledWindow.seamSecs). */
  seamSecs: number[];
  /** Only when fromBar is 0; omitted = opening not checked. */
  opening?: OpeningExpectation;
}

/** Median of |x[i] - x[i - 10]| for i = 10, 20, ... */
function globalMedianStep(x: Float32Array): number {
  const steps: number[] = [];
  for (let i = 10; i < x.length; i += 10) steps.push(Math.abs(x[i]! - x[i - 10]!));
  return median(steps);
}

/** Largest one-sample step within +/- CLICK_WINDOW_SEC of `sec`. */
function localMaxStep(x: Float32Array, sampleRate: number, sec: number): number {
  const i = Math.round(sec * sampleRate);
  const w = Math.round(CLICK_WINDOW_SEC * sampleRate);
  let m = 0;
  for (let j = Math.max(1, i - w); j < Math.min(x.length, i + w); j++) {
    const d = Math.abs(x[j]! - x[j - 1]!);
    if (d > m) m = d;
  }
  return m;
}

interface SeamClick {
  seamSec: number;
  ratio: number;
  click: boolean;
}

/** A seam is a click on a channel when its step stands out from the whole channel AND from the
 *  bar lines around it that are not seams (hard downbeats read high everywhere). */
function seamClicks(
  ch: Float32Array,
  { sampleRate, barSec, seamSecs, meteredWindow }: MixInput,
): SeamClick[] {
  const duration = ch.length / sampleRate;
  const originFrame = meteredWindow?.timing.frameAt(meteredWindow.fromBar, sampleRate) ?? 0;
  const glob = Math.max(globalMedianStep(ch), 1e-6);
  const isSeam = (t: number) => seamSecs.some((s) => Math.abs(s - t) < 1e-3);
  const out: SeamClick[] = [];
  for (const seamSec of seamSecs) {
    if (!(seamSec > 0 && seamSec < duration)) continue;
    const step = localMaxStep(ch, sampleRate, seamSec);
    const ratio = step / glob;
    let click = ratio > CLICK_MAX_RATIO;
    if (click && barSec > 0) {
      // Seam positions are frame-rounded offsets in this buffer. Recover the exact
      // musical boundary before walking adjacent bars, even for a window late in the song.
      const seamBar = meteredWindow
        ? Math.round(meteredWindow.timing.cycleAt(seamSec + originFrame / sampleRate))
        : 0;
      const near: number[] = [];
      for (let k = -CLICK_NEIGHBOR_BARS; k <= CLICK_NEIGHBOR_BARS; k++) {
        const t = meteredWindow
          ? (meteredWindow.timing.frameAt(seamBar + k, sampleRate) - originFrame) / sampleRate
          : seamSec + k * barSec;
        if (k !== 0 && t > 0 && t < duration - 0.01 && !isSeam(t)) near.push(t);
      }
      if (near.length > 0) {
        const neighbor = Math.max(...near.map((t) => localMaxStep(ch, sampleRate, t)));
        click = step > CLICK_NEIGHBOR_FACTOR * neighbor;
      }
    }
    out.push({ seamSec, ratio, click });
  }
  return out;
}

/** The mix verdict. Never `unverifiable`. Throws RangeError for zero channels. */
export function verifyMix(channels: Float32Array[], input: MixInput): GateResult {
  if (channels.length === 0) throw new RangeError('verifyMix needs at least one channel');
  const sr = input.sampleRate;
  const frames = channels[0]!.length;
  const reasons: string[] = [];
  const warnings: string[] = [];

  const nf = firstNonFinite(channels);
  const finite = nf.count === 0;
  if (!finite) {
    reasons.push(`${nf.count} non-finite samples, first at frame ${nf.frame} ch ${nf.channel}`);
  }

  const lengths = channels.map((c) => c.length);
  const driftMs = ((frames - input.expectedFrames) / sr) * 1000;
  if (lengths.some((n) => n !== frames)) {
    reasons.push(`channel lengths differ: ${lengths.join(', ')}`);
  } else if (Math.abs(frames - input.expectedFrames) > FRAME_TOL) {
    reasons.push(
      `frame count ${frames} != expected ${input.expectedFrames} (drift ${roundSec(driftMs)} ms)`,
    );
  }

  const open = Math.min(...channels.map((c) => firstAudibleSec(c, sr, OPEN_THRESHOLD_DB)));
  const o = input.opening;
  if (o) {
    if (open === Infinity) {
      if (o.hasAudibleVoice === false) warnings.push('schedule has no audible voice');
      else reasons.push('silent opening');
    } else if (o.hasPercussiveOpener) {
      if (!(open < OPEN_TOL_SEC)) {
        reasons.push(`opening at ${roundSec(open)}s, percussive opener needs < ${OPEN_TOL_SEC}s`);
      }
    } else if (!(open <= o.expectedFirstAudibleSec + OPEN_LATE_TOL_SEC)) {
      reasons.push(
        `opening at ${roundSec(open)}s, later than expected ` +
          `${roundSec(o.expectedFirstAudibleSec)}s + ${OPEN_LATE_TOL_SEC}s`,
      );
    }
  }

  let worstSeamRatio: number | null = null;
  let clicks: number | null = null;
  let pk: number | null = null;
  let pkDb: number | null = null;
  let clip: number | null = null;
  if (finite) {
    const bySeam = new Map<number, SeamClick>();
    for (const ch of channels) {
      for (const s of seamClicks(ch, input)) {
        const cur = bySeam.get(s.seamSec);
        bySeam.set(s.seamSec, {
          seamSec: s.seamSec,
          ratio: Math.max(cur?.ratio ?? 0, s.ratio),
          click: (cur?.click ?? false) || s.click,
        });
      }
    }
    const seams = [...bySeam.values()];
    worstSeamRatio = seams.reduce((m, s) => Math.max(m, s.ratio), 0);
    const clicked = seams.filter((s) => s.click);
    clicks = clicked.length;
    if (clicked.length > 0) {
      reasons.push(
        `${clicked.length} seam clicks at ${clicked.map((s) => `${roundSec(s.seamSec)}s`).join(', ')}` +
          ` (worst ratio ${roundDb(Math.max(...clicked.map((s) => s.ratio)))} > ${CLICK_MAX_RATIO})`,
      );
    }

    pk = Math.max(...channels.map(peak));
    pkDb = pk === 0 ? null : roundDb(ampToDb(pk, 0));
    clip = Math.max(...channels.map(clipFraction));
    if (pk > CLIP_LIMIT) {
      warnings.push(
        `peak ${roundDb(pk)} (${pkDb} dBFS) over full scale; ` +
          `clip fraction ${roundDb(clip * 100)}%`,
      );
    }
  }

  return {
    gate: 'mix',
    verdict: reasons.length > 0 ? 'fail' : 'pass',
    measurements: {
      frames,
      expectedFrames: input.expectedFrames,
      driftMs: roundSec(driftMs),
      peak: pk === null ? null : round(pk, 4),
      peakDb: pkDb,
      clipFraction: clip === null ? null : round(clip, 6),
      firstAudibleSec: open === Infinity ? null : roundSec(open),
      expectedFirstAudibleSec: o ? roundSec(o.expectedFirstAudibleSec) : null,
      worstSeamRatio: worstSeamRatio === null ? null : roundDb(worstSeamRatio),
      clicks,
    },
    reasons,
    warnings,
  };
}

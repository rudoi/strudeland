/**
 * How much lookback a chunk needs, from the song's own schedule. A chunk only schedules haps whose
 * onset lies in its render window, so a voice that started earlier is silent in it; rendering
 * from earlier and trimming the lead-in fixes that, and this module says how much earlier.
 *
 * Additive on purpose: a voice feeds its orbit's effects until onset + duration + its full
 * release, and the effect THEN rings for its own tail, so per orbit the need is
 * voice + max(reverb, delay). A voice's own need is superdough's `endWithRelease` (duration plus
 * max(release, FXrelease)) plus the compressor's pre-delay; local (`FX` entry) delay and reverb
 * tails are cut at `endWithRelease` and add nothing (superdough's later main-thread cut is inside
 * the margin). A bus reader also needs its longest-lived feeder rendered, through chains of buses.
 * Pure. Ported from the earlier render harness's lookback calculation with unlimited lookback.
 */

import type { RenderEvent } from './schedule';
import {
  CHUNK_LOOKBACK_RATIO,
  DELAY_FEEDBACK_CEIL,
  LOOKBACK_K,
  LOOKBACK_MARGIN_SEC,
} from './thresholds';

export interface LookbackOptions {
  /** Multiplier on room size (1.0 = the -60 dB point). Default LOOKBACK_K. */
  k?: number;
  /** Added after the additive sum. Default LOOKBACK_MARGIN_SEC. */
  marginSec?: number;
}

export interface LookbackOrbit {
  orbit: number;
  voiceSec: number;
  reverbSec: number;
  delaySec: number;
  needSec: number;
}

export interface LookbackResult {
  lookbackSec: number;
  /** Sorted by orbit. */
  byOrbit: LookbackOrbit[];
}

/** Seconds for a feedback delay to decay to -60 dB; feedback is clamped at DELAY_FEEDBACK_CEIL. */
export function delayTailSec(delaytimeSec: number, feedback: number): number {
  if (!(delaytimeSec > 0) || !(feedback > 0)) return 0;
  const fb = Math.min(feedback, DELAY_FEEDBACK_CEIL);
  return (delaytimeSec * Math.log(1000)) / Math.log(1 / fb);
}

const finiteOr0 = (x: number): number => (Number.isFinite(x) ? x : 0);

/** Blink's compressor pre-delay, `(unsigned)(0.006f * 44100.0f)` = 264 frames at the render's
 *  44100 Hz: the node's output lags its input by this much after the source stops. An engine
 *  constant, not a gate threshold, so it lives here (hashed into the refs' provenance). */
export const COMPRESSOR_PREDELAY_SEC = 264 / 44100;

/** One voice's own need: duration + max(release, FXrelease) (superdough's `fullRelease`, so a
 *  negative release counts as 0), plus the compressor pre-delay. */
function ownSec(e: RenderEvent): number {
  return (
    finiteOr0(e.durSec) +
    Math.max(finiteOr0(e.release), finiteOr0(e.fxRelease)) +
    (e.compressor ? COMPRESSOR_PREDELAY_SEC : 0)
  );
}

/** Max over `reads` of `feed` (0 for none or an unfed bus). */
function maxFeed(reads: readonly string[], feed: ReadonlyMap<string, number>): number {
  let m = 0;
  for (const b of reads) m = Math.max(m, feed.get(b) ?? 0);
  return m;
}

/**
 * Per bus, the largest need of a feeder including what that feeder itself reads: `F_{k+1}(b) =
 * max over feeders f of b of (own(f) + max over f's reads c of F_k(c))`, from `F_0 = 0`, for
 * exactly as many rounds as there are distinct buses. An acyclic chain settles exactly; a cycle
 * (which the engine refuses and superdough cannot reproduce) still ends with a finite value.
 */
function busFeedSec(events: readonly RenderEvent[], own: readonly number[]): Map<string, number> {
  const buses = new Set<string>();
  for (const e of events) {
    for (const b of e.busSends) buses.add(b);
    for (const b of e.busReads) buses.add(b);
  }
  let feed = new Map<string, number>();
  for (let round = 0; round < buses.size; round++) {
    const next = new Map<string, number>();
    events.forEach((f, i) => {
      if (f.busSends.length === 0) return;
      const need = own[i]! + maxFeed(f.busReads, feed);
      for (const b of f.busSends) next.set(b, Math.max(next.get(b) ?? 0, need));
    });
    feed = next;
  }
  return feed;
}

export function deriveLookback(events: RenderEvent[], opts: LookbackOptions = {}): LookbackResult {
  const k = opts.k ?? LOOKBACK_K;
  const marginSec = opts.marginSec ?? LOOKBACK_MARGIN_SEC;
  if (!(k > 0)) throw new RangeError(`k must be > 0 (got ${k})`);

  const own = events.map(ownSec);
  const feed = busFeedSec(events, own);
  const acc = new Map<number, { voiceSec: number; maxSize: number; delaySec: number }>();
  for (const [i, e] of events.entries()) {
    let o = acc.get(e.orbit);
    if (!o) {
      o = { voiceSec: 0, maxSize: 0, delaySec: 0 };
      acc.set(e.orbit, o);
    }
    o.voiceSec = Math.max(o.voiceSec, own[i]! + maxFeed(e.busReads, feed));
    if (e.room !== null) o.maxSize = Math.max(o.maxSize, finiteOr0(e.room.size));
    if (e.delay !== null) {
      o.delaySec = Math.max(o.delaySec, delayTailSec(e.delay.timeSec, e.delay.feedback));
    }
  }

  const byOrbit = [...acc.entries()]
    .sort(([a], [b]) => a - b)
    .map(([orbit, o]): LookbackOrbit => {
      const reverbSec = k * o.maxSize;
      return {
        orbit,
        voiceSec: o.voiceSec,
        reverbSec,
        delaySec: o.delaySec,
        needSec: o.voiceSec + Math.max(reverbSec, o.delaySec),
      };
    });
  const need = byOrbit.reduce((m, o) => Math.max(m, o.needSec), 0);
  return { lookbackSec: need + marginSec, byOrbit };
}

export function lookbackSeconds(events: RenderEvent[], opts?: LookbackOptions): number {
  return deriveLookback(events, opts).lookbackSec;
}

/** Lookback rounded up to whole bars (chunks are planned in bars). Throws if barSec <= 0. */
export function lookbackBars(lookbackSec: number, barSec: number): number {
  if (!(barSec > 0)) throw new RangeError(`barSec must be > 0 (got ${barSec})`);
  // Math.max pins a -0 from ceil() of a tiny negative back to 0
  return Math.max(0, Math.ceil(lookbackSec / barSec - 1e-9));
}

/** Chunk size must scale with lookback or the lookback dominates the render cost. */
export function scaleChunkBars(
  requestedChunkBars: number,
  lookbackBarsN: number,
  ratio = CHUNK_LOOKBACK_RATIO,
): number {
  if (!Number.isInteger(requestedChunkBars) || requestedChunkBars < 1) {
    throw new RangeError(
      `requestedChunkBars must be a positive integer (got ${requestedChunkBars})`,
    );
  }
  if (!Number.isInteger(lookbackBarsN) || lookbackBarsN < 0) {
    throw new RangeError(`lookbackBars must be a non-negative integer (got ${lookbackBarsN})`);
  }
  return Math.max(requestedChunkBars, Math.ceil(ratio * lookbackBarsN));
}

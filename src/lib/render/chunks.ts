/**
 * Chunk planning and slicing. A long window is rendered as bounded bar chunks, each with
 * lookback (extra leading bars, trimmed) and an overlap (extra trailing bars, compared with the
 * next chunk's head by the seam gate). The slicing helpers turn chunk results into the buffers
 * the gates take, so no caller invents the trim. Pure (`planChunks` ports the earlier render
 * harness's planner over [fromBar, toBar) with overlap added).
 */

import { windowFrames, type RenderRequest, type RenderResult } from './contract';
import { lookbackBars, lookbackSeconds, scaleChunkBars } from './lookback';
import type { RenderEvent } from './schedule';
import { CHUNK_MIN_BARS, SEAM_OVERLAP_BARS, SEAM_SEARCH_BARS } from './thresholds';
import { historyStart, renderWindowFrames, type RenderTiming } from './timing';

export interface PlanInput {
  /** Content range [fromBar, toBar): integers, fromBar >= 0, toBar > fromBar. */
  fromBar: number;
  toBar: number;
  /** Kept bars per chunk, int >= 1 (usually scaleChunkBars(CHUNK_MIN_BARS, lookbackBars)). */
  chunkBars: number;
  /** Int >= 0. */
  lookbackBars: number;
  /** > 0. */
  barSec: number;
  /** bar -> stochastic voices crossing that bar line (stochasticCrossings). */
  avoidSeamBars?: Record<number, number>;
  /** Default SEAM_SEARCH_BARS. */
  seamSearchBars?: number;
  /** Default SEAM_OVERLAP_BARS; 0 disables the seam overlap. */
  overlapBars?: number;
  timing?: RenderTiming;
  /** Seconds of warmup requested before rounding to a musical bar line. */
  lookbackSec?: number;
}

export interface ChunkPlan {
  index: number;
  /** [contentStartBar, contentEndBar), exclusive end. */
  contentStartBar: number;
  contentEndBar: number;
  /** max(0, contentStartBar - lookbackBars): clamps at the layout start, not at fromBar. */
  winBeginBar: number;
  /** (contentStartBar - winBeginBar) * barSec: seconds trimmed from the render head. */
  lookbackSec: number;
  contentSec: number;
  /** Bars rendered after contentEndBar: 0 for the last chunk. */
  overlapBars: number;
  /** Stochastic voices crossing this chunk's START seam after nudging (0 for chunk 0). */
  seamCrossings: number;
}

const isNonNegInt = (x: number): boolean => Number.isInteger(x) && x >= 0;

export function planChunks(input: PlanInput): ChunkPlan[] {
  const { fromBar, toBar, chunkBars, lookbackBars, barSec, avoidSeamBars, timing } = input;
  const seamSearchBars = input.seamSearchBars ?? SEAM_SEARCH_BARS;
  const overlap = input.overlapBars ?? SEAM_OVERLAP_BARS;
  if (!isNonNegInt(fromBar))
    throw new Error(`fromBar must be a non-negative integer (got ${fromBar})`);
  if (!Number.isInteger(toBar) || toBar - fromBar < 1) {
    throw new Error(`toBar must be an integer > fromBar (got ${fromBar}..${toBar})`);
  }
  if (!Number.isInteger(chunkBars) || chunkBars < 1) {
    throw new Error(`chunkBars must be a positive integer (got ${chunkBars})`);
  }
  if (!isNonNegInt(lookbackBars)) {
    throw new Error(`lookbackBars must be a non-negative integer (got ${lookbackBars})`);
  }
  if (!Number.isFinite(barSec) || barSec <= 0)
    throw new Error(`barSec must be > 0 (got ${barSec})`);
  if (!isNonNegInt(seamSearchBars)) {
    throw new Error(`seamSearchBars must be a non-negative integer (got ${seamSearchBars})`);
  }
  if (!isNonNegInt(overlap)) {
    throw new Error(`overlapBars must be a non-negative integer (got ${overlap})`);
  }

  /** The boundary at or near `target` with the fewest stochastic crossings, ties to the bar
   *  closest to `target` and then the later one. Stays within (cs, toBar]. */
  const chooseSeam = (cs: number, target: number): number => {
    if (!avoidSeamBars || target >= toBar || seamSearchBars === 0) return target;
    let best = target;
    let bestCross = avoidSeamBars[target] ?? 0;
    if (bestCross === 0) return target;
    for (let d = 1; d <= seamSearchBars; d++) {
      // later bar first on a tie: keeps chunks from systematically shrinking
      for (const cand of [target + d, target - d]) {
        if (cand <= cs || cand > toBar) continue;
        const cross = avoidSeamBars[cand] ?? 0;
        if (cross < bestCross) {
          best = cand;
          bestCross = cross;
        }
      }
      if (bestCross === 0) break;
    }
    return best;
  };

  const plans: ChunkPlan[] = [];
  let cs = fromBar;
  for (let k = 0; cs < toBar; k++) {
    const contentStartBar = cs;
    const contentEndBar = chooseSeam(cs, Math.min(cs + chunkBars, toBar));
    const winBeginBar = timing
      ? historyStart(contentStartBar, input.lookbackSec ?? 0, timing)
      : Math.max(0, contentStartBar - lookbackBars);
    plans.push({
      index: k,
      contentStartBar,
      contentEndBar,
      winBeginBar,
      lookbackSec: timing
        ? timing.secondsAt(contentStartBar) - timing.secondsAt(winBeginBar)
        : (contentStartBar - winBeginBar) * barSec,
      contentSec: timing
        ? timing.secondsAt(contentEndBar) - timing.secondsAt(contentStartBar)
        : (contentEndBar - contentStartBar) * barSec,
      overlapBars: 0,
      seamCrossings: k === 0 ? 0 : (avoidSeamBars?.[contentStartBar] ?? 0),
    });
    cs = contentEndBar;
  }
  for (let k = 0; k < plans.length - 1; k++) {
    plans[k]!.overlapBars = Math.min(overlap, toBar - plans[k]!.contentEndBar);
  }
  return plans;
}

/** Total kept content seconds (equals (toBar - fromBar) * barSec). */
export function totalContentSec(plans: ChunkPlan[]): number {
  return plans.reduce((s, p) => s + p.contentSec, 0);
}

/** bar line -> stochastic voices sounding across it (onset before, end after, 1e-9 slack). */
export function stochasticCrossings(
  events: RenderEvent[],
  barSec: number,
  toBar = Infinity,
  timing?: RenderTiming,
): Record<number, number> {
  if (!(barSec > 0)) throw new RangeError(`barSec must be > 0 (got ${barSec})`);
  const out: Record<number, number> = {};
  for (const e of events) {
    if (!e.stochastic) continue;
    const end = e.tSec + e.durSec + e.release;
    for (
      let b = Math.max(0, Math.floor(timing ? timing.cycleAt(e.tSec) : e.tSec / barSec));
      b <= toBar;
      b++
    ) {
      const t = timing ? timing.secondsAt(b) : b * barSec;
      if (!(t + 1e-9 < end)) break;
      if (e.tSec < t - 1e-9) out[b] = (out[b] ?? 0) + 1;
    }
  }
  return out;
}

export interface RenderChunkInput {
  fromBar: number;
  toBar: number;
  /** The unmutated events of the schedule [0, toBar): `eventsFromValues` output. */
  events: RenderEvent[];
  cps: number;
  /** Override the normal lookback when an export uses the full engine decay. */
  lookbackSec?: number;
  /** Seam overlap bars: 0 for a joined render, the default SEAM_OVERLAP_BARS for the seam gate. */
  overlapBars?: number;
  timing?: RenderTiming;
}

/** The chunk plan of a window from its schedule: the lookback of `events` in whole bars, chunks
 *  scaled to it, seams nudged away from stochastic crossings. The one recipe the harness and the
 *  batch planner share. Throws as `planChunks` and the lookback helpers do. */
export function renderChunkPlans(input: RenderChunkInput): ChunkPlan[] {
  const { timing } = input;
  const barSec = timing?.minBarSec ?? 1 / input.cps;
  const lookbackSec = input.lookbackSec ?? lookbackSeconds(input.events);
  const lb = lookbackBars(lookbackSec, barSec);
  return planChunks({
    fromBar: input.fromBar,
    toBar: input.toBar,
    chunkBars: scaleChunkBars(CHUNK_MIN_BARS, lb),
    lookbackBars: lb,
    barSec,
    avoidSeamBars: stochasticCrossings(input.events, barSec, input.toBar, timing),
    ...(timing ? { timing, lookbackSec } : {}),
    ...(input.overlapBars === undefined ? {} : { overlapBars: input.overlapBars }),
  });
}

// ---- request and slicing helpers

/** The render request for one chunk: `base` with the window, overlap and lookback of `plan`. */
export function chunkRequest(base: RenderRequest, plan: ChunkPlan): RenderRequest {
  return {
    ...base,
    fromBar: plan.contentStartBar,
    toBar: plan.contentEndBar,
    overlapBars: plan.overlapBars,
    lookbackSec: plan.lookbackSec,
  };
}

function slice(r: RenderResult, start: number, len: number, what: string): Float32Array[] {
  for (const [c, ch] of r.channels.entries()) {
    if (ch.length < start + len) {
      throw new RangeError(
        `${what}: channel ${c} has ${ch.length} frames, needs ${start + len} (${start} + ${len})`,
      );
    }
  }
  return r.channels.map((ch) => ch.subarray(start, start + len));
}

function contentFrames(r: RenderResult): number {
  return r.contentFrames ?? windowFrames(r.request.fromBar, r.request.toBar, r.cps, r.sampleRate);
}

/** The kept content [fromBar, toBar): lookback and overlap removed. Throws on a short buffer. */
export function contentOf(r: RenderResult): Float32Array[] {
  return slice(r, r.windowStartFrame, contentFrames(r), 'contentOf');
}

/** The `request.overlapBars` bars rendered after the content: the seam gate's `tail`. */
export function overlapOf(r: RenderResult): Float32Array[] {
  const { toBar, overlapBars } = r.request;
  const m = r.overlapFrames ?? windowFrames(toBar, toBar + overlapBars, r.cps, r.sampleRate);
  return slice(r, r.windowStartFrame + contentFrames(r), m, 'overlapOf');
}

/** The first `bars` bars of the content: the seam gate's `head` (bars = the previous chunk's
 *  overlapBars). Throws RangeError when `bars` exceeds the content. */
export function headOf(r: RenderResult, bars: number, timing?: RenderTiming): Float32Array[] {
  const { fromBar, toBar } = r.request;
  if (!isNonNegInt(bars) || fromBar + bars > toBar) {
    throw new RangeError(`headOf: ${bars} bars outside content [${fromBar}, ${toBar})`);
  }
  const h = renderWindowFrames(fromBar, fromBar + bars, r.cps, r.sampleRate, timing);
  return contentOf(r).map((ch) => ch.subarray(0, h));
}

export interface AssembledWindow extends RenderResult {
  /** Seams in seconds from frame 0 of `channels`: seamSecs[i] is the start of chunk i + 1. */
  seamSecs: number[];
}

/** Concatenates the content of contiguous chunk results into one result for the whole window.
 *  `events` is the harness schedule. Throws RangeError on no results, a gap or overlap between
 *  chunks, or a channel count mismatch. */
export function assembleWindow(results: RenderResult[], events: RenderEvent[]): AssembledWindow {
  const first = results[0];
  if (!first) throw new RangeError('assembleWindow: no results');
  const last = results[results.length - 1]!;
  const nch = first.channels.length;
  for (let i = 0; i < results.length; i++) {
    const r = results[i]!;
    if (r.channels.length !== nch) {
      throw new RangeError(
        `assembleWindow: chunk ${i} has ${r.channels.length} channels, chunk 0 has ${nch}`,
      );
    }
    const next = results[i + 1];
    if (next && r.request.toBar !== next.request.fromBar) {
      throw new RangeError(
        `assembleWindow: chunk ${i} ends at bar ${r.request.toBar}, chunk ${i + 1} starts at bar ${next.request.fromBar}`,
      );
    }
  }

  const contents = results.map(contentOf);
  const sizes = results.map(contentFrames);
  const frames = sizes.reduce((n, s) => n + s, 0);
  const channels = Array.from({ length: nch }, () => new Float32Array(frames));
  const seamSecs: number[] = [];
  let off = 0;
  for (const [i, content] of contents.entries()) {
    if (i > 0) seamSecs.push(off / first.sampleRate);
    for (let c = 0; c < nch; c++) channels[c]!.set(content[c]!, off);
    off += sizes[i]!;
  }

  return {
    channels,
    sampleRate: first.sampleRate,
    frames,
    windowStartFrame: 0,
    lookbackSecUsed: 0,
    bufferStartSec:
      first.contentFrames === undefined
        ? first.request.fromBar / first.cps
        : first.bufferStartSec + first.lookbackSecUsed,
    ...(first.contentFrames === undefined ? {} : { contentFrames: frames }),
    cps: first.cps,
    events,
    stemPairs: first.stemPairs,
    errors: results.flatMap((r) => r.errors),
    ok: results.every((r) => r.ok),
    timing: results.reduce(
      (t, r) => ({
        evalMs: t.evalMs + r.timing.evalMs,
        renderMs: t.renderMs + r.timing.renderMs,
        totalMs: t.totalMs + r.timing.totalMs,
      }),
      { evalMs: 0, renderMs: 0, totalMs: 0 },
    ),
    request: { ...first.request, toBar: last.request.toBar, overlapBars: 0, lookbackSec: 0 },
    seamSecs,
  };
}

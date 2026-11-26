/**
 * Render planning shared by every renderer: which haps play, the render window, the output
 * channel layout, the pause schedule of the bar-by-bar loop, the per-bar buckets and the order
 * orbits are created in. Lifted out of `src/lib/audio/offline.ts` (which imports superdough) so
 * pure code, the engine job planner and `bun test` can use it; `offline.ts` imports what it
 * needs from here. Pure: no superdough, DOM, Tauri or Playwright imports.
 */

import { errorMessage } from '../errors';
import {
  expectedFrames,
  type HapMutation,
  type RenderError,
  type RenderMode,
  type RenderRequest,
} from './contract';
import type { EventSource, RenderEvent } from './schedule';
import { orbitPair } from './gates/stems';
import { STEMS_MAX_PAIRS } from './thresholds';
import { historyStart, renderWindowFrames, type RenderTiming } from './timing';

/** Pause this long before each bar line to schedule the next bar (see `pauseSchedule`). */
export const LEAD_SEC = 0.05;
/** Largest render buffer accepted, bytes (Float32 channels). */
export const MAX_BUFFER_BYTES = 1_000_000_000;
/** Suspend times quantize to a render quantum (128 frames); the lead must exceed two. */
export const MIN_LEAD_FRAMES = 256;
/** The render quantum: Chromium quantizes a suspend to a multiple of it. */
export const RENDER_QUANTUM = 128;

/** The frame Chromium suspends at for `suspendSec` (Blink `OfflineAudioContext.suspend`):
 *  truncate to a frame, then round up to a multiple of the render quantum. */
export const pauseFrameOf = (suspendSec: number, sampleRate: number): number =>
  Math.ceil(Math.trunc(suspendSec * sampleRate) / RENDER_QUANTUM) * RENDER_QUANTUM;
const MAX_MESSAGE = 300;

// ---- hap selection

/** True when the hap goes to the default (superdough) output: `repl.mjs` `getSchedule` skips it
 *  only when it has both `onSchedule` and `dominantSchedule`. */
export function routesToDefault(hap: any): boolean {
  const ctx = hap?.context ?? {};
  return !(ctx.onSchedule && ctx.dominantSchedule);
}

export interface Selected {
  hap: any;
  value: any;
  /** `hap.whole.begin`, layout cycles. */
  onsetCycle: number;
  /** Position in `selected`. */
  index: number;
}

/** The haps superdough would play, ascending by onset (ties keep query order). */
export function selectHaps(haps: any[]): { selected: Selected[]; errors: RenderError[] } {
  const errors: RenderError[] = [];
  const kept = haps
    .filter((h) => h != null && h.whole != null && h.hasOnset() && routesToDefault(h))
    .map((hap, i) => ({ hap, i, onset: hap.whole.begin.valueOf() as number }))
    .sort((a, b) => a.onset - b.onset || a.i - b.i);
  const selected: Selected[] = [];
  for (const { hap, onset } of kept) {
    const bar = Math.floor(onset);
    try {
      hap.ensureObjectValue();
    } catch (e) {
      // The fork's ensureObjectValue throws on a non-object value; superdough would throw too.
      errors.push(classifyError(`hap at cycle ${onset}: ${errorMessage(e)}`, bar));
      continue;
    }
    const value = hap.value;
    if (value.cps !== undefined) {
      errors.push({
        kind: 'other',
        message: 'unsupported: hap sets cps (control cps is not supported by the harness)',
        bar,
      });
    }
    selected.push({ hap, value, onsetCycle: onset, index: selected.length });
  }
  return { selected, errors };
}

/** Event sources for `eventsFromValues`, in selected order. */
export function eventSources(selected: Selected[]): EventSource[] {
  return selected.map((s) => ({
    value: s.value,
    wholeBeginCycles: s.onsetCycle,
    wholeDurCycles: s.hap.whole.end.valueOf() - s.hap.whole.begin.valueOf(),
  }));
}

// ---- window and channels

export interface WindowPlan {
  /** First rendered layout cycle (a bar line). */
  c0: number;
  /** End layout cycle (exclusive): toBar + overlapBars. */
  c1: number;
  lookbackSecUsed: number;
  windowStartFrame: number;
  /** Layout seconds of the first frame. */
  bufferStartSec: number;
  frames: number;
  /** Mapped musical content frames in the request's range, excluding lookback and overlap; every
   *  planner sets it. */
  contentFrames: number;
  /** Mapped frames after content, excluding any export tail; present exactly when
   *  `overlapBars > 0`. */
  overlapFrames?: number;
}

/** Channels the buffer-size bound assumes for a mode (stems at its widest). */
export const maxChannels = (mode: RenderMode): number =>
  mode === 'stems' ? 2 * STEMS_MAX_PAIRS : 2;

/** Apply the existing scratch-buffer ceiling before allocating a render or joined export. */
export function checkBufferFrames(frames: number, mode: RenderMode): void {
  const bytes = frames * maxChannels(mode) * 4;
  if (!Number.isSafeInteger(frames) || frames < 0 || bytes > MAX_BUFFER_BYTES) {
    throw new RangeError(`render buffer too large: ${bytes} bytes (max ${MAX_BUFFER_BYTES})`);
  }
}

/** The render window: whole-bar lead-in covering `lookbackSec` (clamped at bar 0), the request's
 *  bars and its overlap. Throws on an empty window or a buffer over `MAX_BUFFER_BYTES`. */
export function windowPlan(
  req: RenderRequest,
  cps: number,
  lookbackSec: number,
  tailFrames = 0,
  timing?: RenderTiming,
  scheduleStart = 0,
): WindowPlan {
  if (!Number.isFinite(cps) || cps <= 0) throw new RangeError(`invalid cps ${cps}`);
  if (!Number.isFinite(lookbackSec) || lookbackSec < 0) {
    throw new RangeError(`invalid lookback ${lookbackSec}`);
  }
  const c0 = timing
    ? Math.max(scheduleStart, historyStart(req.fromBar, lookbackSec, timing))
    : Math.max(0, req.fromBar - Math.ceil(lookbackSec * cps - 1e-9));
  const c1 = req.toBar + req.overlapBars;
  if (c1 <= c0) throw new RangeError(`empty render window [${c0}, ${c1})`);
  const contentFrames = renderWindowFrames(req.fromBar, req.toBar, cps, req.sampleRate, timing);
  const overlapFrames = req.overlapBars
    ? renderWindowFrames(req.toBar, c1, cps, req.sampleRate, timing)
    : undefined;
  if (timing) {
    const start = timing.frameAt(c0, req.sampleRate);
    const from = timing.frameAt(req.fromBar, req.sampleRate);
    const end = timing.frameAt(c1, req.sampleRate);
    const frames = end - start + tailFrames;
    checkBufferFrames(frames, req.mode);
    return {
      c0,
      c1,
      lookbackSecUsed: (from - start) / req.sampleRate,
      windowStartFrame: from - start,
      bufferStartSec: start / req.sampleRate,
      frames,
      contentFrames,
      ...(overlapFrames === undefined ? {} : { overlapFrames }),
    };
  }
  const lookbackSecUsed = (req.fromBar - c0) / cps;
  const frames = expectedFrames(req, cps, lookbackSecUsed) + tailFrames;
  checkBufferFrames(frames, req.mode);
  return {
    c0,
    c1,
    lookbackSecUsed,
    windowStartFrame: Math.round(lookbackSecUsed * req.sampleRate),
    bufferStartSec: c0 / cps,
    frames,
    contentFrames,
    ...(overlapFrames === undefined ? {} : { overlapFrames }),
  };
}

export interface ChannelPlan {
  pairs: number;
  channels: number;
  multiChannelOrbits: boolean;
}

/** The numeric orbit of a hap value: `Number(value.orbit ?? 1)`, 1 when not finite. */
export const orbitNumber = (value: any): number => {
  const n = Number(value?.orbit ?? 1);
  return Number.isFinite(n) ? n : 1;
};

/** Output channel layout: one stereo pair for a mix, one pair per orbit (up to 16, aliasing as
 *  superdough's `% channelCount` does) for stems. */
export function channelPlan(values: any[], mode: RenderMode): ChannelPlan {
  if (mode === 'mix') return { pairs: 1, channels: 2, multiChannelOrbits: false };
  let maxPair = 0;
  for (const v of values) {
    let pair: number;
    if (v?.channels != null) {
      const chans = [v.channels].flat().map(Number).filter(Number.isFinite);
      pair = chans.length === 0 ? 0 : Math.ceil(Math.max(...chans) / 2) - 1;
    } else {
      // Unwrapped (orbit k on pair k - 1): the clamp below plus superdough's `% channelCount`
      // then put orbit k on pair (k - 1) % pairs, which is `orbitPair(k, pairs)` exactly.
      pair = orbitPair(orbitNumber(v), Number.POSITIVE_INFINITY);
    }
    maxPair = Math.max(maxPair, pair);
  }
  const pairs = Math.min(Math.max(maxPair + 1, 1), STEMS_MAX_PAIRS);
  return { pairs, channels: 2 * pairs, multiChannelOrbits: true };
}

/** The 0-based output channels superdough connects an orbit to (`superdough.mjs:528-533`). */
export function orbitChannels(value: any, multiChannelOrbits: boolean): number[] {
  if (value?.channels != null) return [value.channels].flat().map((c: unknown) => Number(c) - 1);
  const orbit = value?.orbit === undefined ? 1 : value.orbit;
  return multiChannelOrbits && orbit > 0 ? [orbit * 2 - 2, orbit * 2 - 1] : [0, 1];
}

// ---- pause schedule, buckets and orbit order

/** The pause lead for a bar of `barSec` seconds: `LEAD_SEC`, at most a quarter bar. */
export const pauseLeadSec = (barSec: number): number => Math.min(LEAD_SEC, barSec / 4);

/** When the bar-by-bar loop suspends: `suspendSec[j]` is the context time it pauses at to
 *  schedule bar j (`j * barSec - leadSec`); bar 0 is scheduled before rendering starts, so
 *  `suspendSec[0] = 0`. Length `max(nBars, 1)`. Throws when the lead is too short for a
 *  render-quantum-quantized suspend and there is more than one bar. */
export function pauseSchedule(
  nBars: number,
  cps: number,
  sampleRate: number,
  timing?: RenderTiming,
  c0 = 0,
): { leadSec: number; suspendSec: number[] } {
  const barSec = 1 / cps;
  const leadSec = pauseLeadSec(timing?.minBarSec ?? barSec);
  if (nBars > 1 && leadSec < MIN_LEAD_FRAMES / sampleRate) {
    throw new Error('bar too short for the pause lead');
  }
  const suspendSec = [0];
  const origin = timing ? timing.frameAt(c0, sampleRate) / sampleRate : 0;
  for (let j = 1; j < nBars; j++) {
    suspendSec.push((timing ? timing.secondsAt(c0 + j) - origin : j * barSec) - leadSec);
  }
  return { leadSec, suspendSec };
}

/** The selected haps per window bar j (`floor(onsetCycle) - c0`), in selected order; haps
 *  outside `[0, nBars)` and those in `dropped` are left out. Length `max(nBars, 0)`. */
export function bucketByBar(
  selected: Selected[],
  c0: number,
  nBars: number,
  dropped: ReadonlySet<number> = new Set<number>(),
): Selected[][] {
  const buckets: Selected[][] = Array.from({ length: Math.max(nBars, 0) }, () => []);
  for (const s of selected) {
    if (dropped.has(s.index)) continue;
    const j = Math.floor(s.onsetCycle) - c0;
    if (j >= 0 && j < nBars) buckets[j]!.push(s);
  }
  return buckets;
}

export interface OrbitCreation {
  /** The hap's raw orbit value (1 when absent), as superdough's `getOrbit` receives it. */
  orbit: unknown;
  channels: number[];
  /** Onset (layout cycles) of the orbit's first hap. */
  onset: number;
}

/** Every orbit `full` and `chunk` use, keyed by `String(orbit)`, with the channels of its
 *  earliest hap (ties: `full` first, then insertion order), sorted by that onset (stable). */
export function orbitCreationOrder(
  chunk: Selected[],
  full: Selected[],
  multiChannelOrbits: boolean,
): OrbitCreation[] {
  const firstUse = new Map<string, OrbitCreation>();
  for (const s of [...full, ...chunk]) {
    const orbit = s.value.orbit === undefined ? 1 : s.value.orbit;
    const key = String(orbit);
    const prev = firstUse.get(key);
    if (prev && prev.onset <= s.onsetCycle) continue;
    firstUse.set(key, {
      orbit,
      channels: orbitChannels(s.value, multiChannelOrbits),
      onset: s.onsetCycle,
    });
  }
  return [...firstUse.values()].sort((a, b) => a.onset - b.onset);
}

// ---- mutations and errors

/** Whether the mutation drops this (pre-mutation) schedule event. */
export function matchesMutation(m: HapMutation, e: RenderEvent): boolean {
  if (m.kind === 'dropOrbit') return e.orbit === m.orbit && m.bars.includes(e.bar);
  return e.freq !== null && e.freq < m.hz && (m.bars === undefined || m.bars.includes(e.bar));
}

/** A `RenderError` from a message, its kind inferred from the text (first rule that matches). */
export function classifyError(msg: string, bar?: number): RenderError {
  const kind: RenderError['kind'] = msg.startsWith('hap ')
    ? 'hap'
    : msg.startsWith('IR renders did not settle')
      ? 'ir'
      : /worklet|processor/i.test(msg)
        ? 'worklet'
        : /not found|is it loaded|sample/i.test(msg)
          ? 'sample'
          : 'other';
  const err: RenderError = { kind, message: msg.slice(0, MAX_MESSAGE) };
  if (bar !== undefined) err.bar = bar;
  return err;
}

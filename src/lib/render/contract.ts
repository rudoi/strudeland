/**
 * The render contract: what the harness asks a renderer for and what it gets back. Plain JSON
 * shapes (requests cross the Playwright boundary) plus the frame-count rules every renderer and
 * gate shares. Pure: no imports but types.
 */

import type { InstrumentSnapshot } from '../strudel/instruments';
import type { SongStructure } from '../strudel/structure';
import type { AssembledWindow } from './chunks';
import type { UnsupportedSignature } from './engine-job';
import type { EngineName } from './engines';
import type { RenderPlanRequest } from './job-plan';
import type { RenderEvent } from './schedule';
import type { RenderTiming } from './timing';

/** The only supported sample rate. */
export const SAMPLE_RATE = 44100;

export type RenderMode = 'mix' | 'stems';
export type SegmentLayout = 'linear' | 'clip';

/** Plain JSON, crosses the Playwright boundary. Bars are layout bars (global cycles of the
 *  request's layout: `floor(hap.whole.begin)`), 0-based. One mutation per request. */
export type HapMutation =
  | { kind: 'dropOrbit'; orbit: number; bars: number[] }
  | { kind: 'dropNotesBelowHz'; hz: number; bars?: number[] }; // bars omitted = every bar

export interface RenderRequest {
  /** Song source, evaluated exactly as on disk. */
  code: string;
  layout: SegmentLayout;
  /** Window [fromBar, toBar) in layout bars; integers, 0 <= fromBar < toBar. */
  fromBar: number;
  toBar: number;
  /** Extra bars rendered after toBar, kept in the buffer (seam comparison). Integer >= 0. */
  overlapBars: number;
  /** Seconds rendered before fromBar and kept at the head of the buffer. 'auto' = the renderer
   *  computes the lookback from the schedule; a number (>= 0) overrides it (0 is the seam
   *  mutation). */
  lookbackSec: number | 'auto';
  mode: RenderMode;
  sampleRate: typeof SAMPLE_RATE;
  /** Calibration/mutation probes only. Applied to audio, NOT to the returned schedule. */
  mutation?: HapMutation;
  /** Sample-pack base URL or manifest path forwarded to the renderer; opaque to gates. */
  samples?: Record<string, string>;
}

/** Whole-render wire input. Evaluation captures pins before shared planning begins. */
export interface RenderPlanInput extends RenderPlanRequest {
  instruments?: InstrumentSnapshot;
}

/** `unsupported`: a hap uses a sound or control the renderer does not implement (the Rust
 *  engine only; superdough never reports it). */
export type RenderErrorKind =
  'eval' | 'hap' | 'worklet' | 'sample' | 'ir' | 'unsupported' | 'other';

export interface RenderError {
  kind: RenderErrorKind;
  message: string;
  bar?: number;
}

export interface RenderResult {
  /** mix: 2 arrays. stems: 2 * stemPairs arrays. All length `frames`. */
  channels: Float32Array[];
  sampleRate: number;
  frames: number;
  /** Frame index of fromBar inside `channels`. */
  windowStartFrame: number;
  /** Mapped frames between fromBar and toBar, excluding lookback and overlap (the plan's
   *  `contentFrames`). */
  contentFrames: number;
  /** Mapped frames after content; present exactly when `request.overlapBars > 0`. */
  overlapFrames?: number;
  /** Lookback actually rendered, seconds, never negative. */
  lookbackSecUsed: number;
  /** Layout seconds of the first frame. */
  bufferStartSec: number;
  cps: number;
  /** One entry per rendered hap with an onset, BEFORE any mutation, sorted by tSec. */
  events: RenderEvent[];
  /** Number of stereo pairs in `channels` (1 for mix). */
  stemPairs: number;
  errors: RenderError[];
  /** errors.length === 0 is required for any gate to run; a failed render is a harness fail. */
  ok: boolean;
  timing: { evalMs: number; renderMs: number; totalMs: number };
  /** Echo of the request, so reports and gates need not carry it separately. */
  request: RenderRequest;
}

/** Query-only result (no audio): everything the harness needs before it plans chunks. */
export interface ScheduleResult {
  /** Every rendered-hap event with onset in layout bars [0, toBar + overlapBars), sorted by tSec. */
  events: RenderEvent[];
  cps: number;
  structure: SongStructure;
  errors: RenderError[];
  /** The distinct sound/control signatures the renderer cannot play, in first-use order. Only a
   *  renderer that can refuse haps (the Rust engine) sets it; absent means none. */
  unsupported?: UnsupportedSignature[];
}

export interface Renderer {
  /** Which engine renders. */
  readonly engine: EngineName;
  /** Evaluate and query only. Uses `code`, `layout`, `toBar` and `overlapBars` of the request;
   *  `fromBar`, `lookbackSec`, `mode` and `mutation` are ignored. */
  schedule(req: RenderRequest): Promise<ScheduleResult>;
  render(req: RenderRequest): Promise<RenderResult>;
  /** Renders `[base.fromBar, base.toBar)` of `base.code` as one window through the renderer's
   *  own production path. When present, the harness takes the fixture's main window from it
   *  instead of joining chunk renders. `events` is the harness schedule, attached as
   *  `assembleWindow` attaches it; `song` is the cps and (mixed meter) timing the harness
   *  derived from that schedule. */
  renderWindow?(
    base: RenderRequest,
    events: RenderEvent[],
    song: { cps: number; timing?: RenderTiming },
  ): Promise<AssembledWindow>;
  dispose(): Promise<void>;
}

/** Frames of the bar window [fromBar, toBar): round(toBar * sr / cps) - round(fromBar * sr / cps).
 *  Rounding the two edges (not the difference) makes adjacent windows telescope exactly: the
 *  frames of chunks [a, b) and [b, c) add up to the frames of [a, c). */
export function windowFrames(
  fromBar: number,
  toBar: number,
  cps: number,
  sampleRate: number,
): number {
  return Math.round((toBar * sampleRate) / cps) - Math.round((fromBar * sampleRate) / cps);
}

/** Exact buffer length of a render: round(lookbackSecUsed * sampleRate) +
 *  windowFrames(fromBar, toBar + overlapBars, cps, sampleRate). */
export function expectedFrames(req: RenderRequest, cps: number, lookbackSecUsed: number): number {
  return (
    Math.round(lookbackSecUsed * req.sampleRate) +
    windowFrames(req.fromBar, req.toBar + req.overlapBars, cps, req.sampleRate)
  );
}

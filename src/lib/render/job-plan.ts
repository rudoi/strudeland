/**
 * The engine-job planner shared by the render page (`src/render/main.ts`) and the headless host:
 * evaluate a song through a `PlannerRuntime`, query it once, and plan every chunk of a render in
 * one call (`planRender`), or exactly one chunk for the harness (`chunk`). Strudel- and DOM-free:
 * the runtime comes in through the adapter.
 */

import { errorMessage } from '../errors';
import { readSongHeader, type Meta, type SongHeader } from '../project/song-header';
import type { EvaluateOptions } from '../strudel/runtime';
import {
  authoredLoop,
  MAX_LOOPS,
  StructureError,
  withLoops,
  type SongStructure,
} from '../strudel/structure';
import { locate, type Segment } from '../strudel/transport';
import {
  SAMPLE_RATE,
  type RenderError,
  type RenderMode,
  type RenderRequest,
  type SegmentLayout,
} from './contract';
import { chunkRequest, renderChunkPlans } from './chunks';
import { isRecord, planEngineJob } from './engine-job';
import { exportDecay, lookbackSeconds } from './lookback';
import { pageJob, dedupeErrors, type PageJobInfo } from './page-job';
import {
  checkBufferFrames,
  eventSources,
  selectHaps,
  windowPlan,
  type Selected,
  type WindowPlan,
} from './plan';
import { eventsFromValues, exportTimingEvent, type RenderEvent } from './schedule';
import { layoutSegments, renderTiming, renderWindowFrames, type RenderTiming } from './timing';

/** The part of a Strudel runtime the planner uses (the page and the host adapt theirs). */
export interface PlannerRuntime {
  /** Resolves true iff the code evaluated without error. */
  evaluate(code: string, opts: EvaluateOptions): Promise<boolean>;
  /** The last evaluate's error message; null when it succeeded. */
  evalError(): string | null;
  state(): { structure: SongStructure | null; cps: number; seed: number | undefined };
  /** Haps of [from, to) on `segments`; null before a successful evaluate. */
  query(from: number, to: number, segments: readonly Segment[]): any[] | null;
}

export type PlanJobPhase = 'evaluating' | 'querying';

/** Why planning failed: the `meta` header, the song's evaluation, or the query and plan.
 *  `flag` names the command-line flag a length request error is about (`--length`,
 *  `--section`, `--loops`, `--preset`); only `planRender` sets it, and only with kind 'meta'. */
export class PlanJobError extends Error {
  declare public readonly flag?: string;

  constructor(
    public readonly kind: 'meta' | 'eval' | 'plan',
    message: string,
    flag?: string,
  ) {
    super(message);
    this.name = 'PlanJobError';
    if (flag !== undefined) this.flag = flag;
  }
}

/** A song evaluated for planning or querying: its `meta`, structure, tempo, seed and the
 *  segments of the requested layout. */
export interface EvaluatedSong {
  meta: Meta;
  structure: SongStructure;
  cps: number;
  seed: number;
  segments: readonly Segment[];
  timing?: RenderTiming;
}

/** Wraps any failure of `body` that is not already a `PlanJobError` as kind 'plan'. */
function asPlanError<T>(body: () => T): T {
  try {
    return body();
  } catch (e) {
    if (e instanceof PlanJobError) throw e;
    throw new PlanJobError('plan', errorMessage(e));
  }
}

/** The song header, a header error as the planner's `meta` error. */
function headerOrThrow(code: string): Extract<SongHeader, { ok: true }> {
  const header = readSongHeader(code);
  if (!header.ok) throw new PlanJobError('meta', header.error.reported);
  return header;
}

/** `evaluateSong` before the segments: parse, evaluate, read the state. */
async function evaluateState(
  rt: PlannerRuntime,
  code: string,
  onPhase?: (p: PlanJobPhase) => void,
): Promise<Omit<EvaluatedSong, 'segments' | 'timing'>> {
  onPhase?.('evaluating');
  const header = headerOrThrow(code);
  if (!(await rt.evaluate(code, header.options))) {
    throw new PlanJobError('eval', rt.evalError() ?? 'evaluate failed');
  }
  onPhase?.('querying');
  return asPlanError(() => {
    const state = rt.state();
    const structure = state.structure ?? header.structure;
    return { meta: header.meta, structure, cps: state.cps, seed: state.seed ?? 0 };
  });
}

/**
 * Parses `code`'s `meta` header, evaluates it through `rt` and reads the song's state: the
 * steps every planner and query path shares. The structure is the evaluated one (what `song()`
 * registered, which playback follows), else the header's; the clip layout's segments are its
 * clip arrangement. Throws `PlanJobError` (meta, eval, or plan for a failure reading the state);
 * an `evaluate` that rejects propagates unchanged.
 */
export async function evaluateSong(
  rt: PlannerRuntime,
  code: string,
  layout: SegmentLayout,
  onPhase?: (p: PlanJobPhase) => void,
): Promise<EvaluatedSong> {
  const song = await evaluateState(rt, code, onPhase);
  return asPlanError(() => {
    const segments = layoutSegments(layout, song.structure);
    const timing = renderTiming(song.structure, segments, song.cps);
    return { ...song, segments, ...(timing ? { timing } : {}) };
  });
}

/** `rt.query` on `segments`, a null result (no successful evaluate) as a `PlanJobError`. */
export function querySong(
  rt: PlannerRuntime,
  segments: readonly Segment[],
): (from: number, to: number) => any[] {
  return (from, to) => {
    const haps = rt.query(from, to, segments);
    if (haps === null) throw new PlanJobError('plan', 'query before a successful evaluate');
    return haps;
  };
}

/** What `planWindow` reads besides the request: the song's query, tempo, seed and structure. */
interface WindowContext {
  query: (from: number, to: number) => any[];
  cps: number;
  seed: number;
  structure: SongStructure;
  timing?: RenderTiming;
  /** Loop passes all use the same authored events. */
  frozenSchedule?: boolean;
}

/** A `selectHaps` result for a range of the schedule. */
interface ScheduleSelection {
  selected: Selected[];
  errors: RenderError[];
}

/** One window's engine job from the schedule selection up to its end: the plan, the chunk's own
 *  selection, the job and its info. `lookbackSec` is already resolved. */
function planWindow(
  ctx: WindowContext,
  request: RenderRequest,
  lookbackSec: number,
  schedule: ScheduleSelection,
  captured: RenderError[],
  scheduleStart = 0,
  tailFrames = 0,
): { info: PageJobInfo; json: string } {
  const plan = windowPlan(request, ctx.cps, lookbackSec, tailFrames, ctx.timing, scheduleStart);
  // The incoming schedule already ends at c1. For loop exports the chunk uses those same
  // authored onsets and whole durations, with indices local to the chunk's event list.
  // Standard windows keep the render page's query at c0, which also observes query fragments.
  const chunk =
    plan.c0 === scheduleStart
      ? schedule
      : ctx.frozenSchedule
        ? { selected: onsetsFrom(schedule.selected, plan.c0), errors: [] }
        : selectHaps(ctx.query(plan.c0, plan.c1));
  const planned = planEngineJob(
    chunk.selected,
    schedule.selected,
    plan,
    request,
    ctx.cps,
    ctx.seed,
    ctx.timing,
  );
  const errors = dedupeErrors([...schedule.errors, ...chunk.errors, ...captured]);
  return pageJob(planned, { errors, plan, cps: ctx.cps, structure: ctx.structure });
}

// ---- planRender: every chunk of a render, or one chunk for the harness, in one call

/** What to render. `window` is for the harness and the dev binary; the command line never
 *  sends it. */
export type LengthRequest =
  | { kind: 'clip'; loops?: number }
  | { kind: 'section'; name: string }
  | { kind: 'length'; bars: number }
  | { kind: 'window'; fromBar: number; toBar: number; layout: SegmentLayout };

export type ExportPreset = 'tagged-loop' | 'intro-loop' | 'folded-loop' | 'one-shot';

/** A harness request for one chunk: exactly the job `[fromBar, toBar)` plus overlap, lookback
 *  and mode would get, planned by the pass code every production chunk goes through. */
export interface SingleChunk {
  overlapBars: number;
  lookbackSec: number | 'auto';
  mode: RenderMode;
}

export interface RenderPlanRequest {
  preset?: ExportPreset;
  length: LengthRequest;
  /** Also return the schedule's events. Default false. */
  includeEvents?: boolean;
  /** Plan exactly one chunk over `length` (a window), for the harness. */
  chunk?: SingleChunk;
}

/** The single-chunk request for a harness `RenderRequest`: its window, overlap, lookback and
 *  mode (`sampleRate` is always `SAMPLE_RATE`; `code` travels beside the request). */
export function singleChunkRequest(req: RenderRequest): RenderPlanRequest {
  return {
    length: { kind: 'window', fromBar: req.fromBar, toBar: req.toBar, layout: req.layout },
    chunk: { overlapBars: req.overlapBars, lookbackSec: req.lookbackSec, mode: req.mode },
  };
}

/** A chunk's `WindowPlan` plus its content window and the exact lookback its request carries
 *  (`lookbackSecUsed` is not always equal to it in floating point). */
export type ChunkWindowPlan = WindowPlan & { fromBar: number; toBar: number; lookbackSec: number };

/** What a single-chunk plan adds to its chunk: the job's events and stem pairs (`PageJobInfo`). */
export interface ChunkInfo {
  events: RenderEvent[];
  stemPairs: number;
}

/** Layout bars `[fromBar, toBar)` that play in the section `name`. */
export interface PlannedSection {
  fromBar: number;
  toBar: number;
  name: string;
}

interface RenderPass {
  /** `info` is present only for a `chunk` request. */
  chunks: { json: string; plan: ChunkWindowPlan; info?: ChunkInfo }[];
  tailFrames: number;
  /** Mapped musical frames of the pass, excluding the lookback and the tail. */
  musicFrames: number;
}

export interface PlannedRender {
  /** Extra frames on the last job and the isolated body when the opening includes an intro. */
  export?: {
    loopStartBar?: number;
    /** Mapped musical frame offset from the opening's start; present exactly when
     *  `loopStartBar` is. */
    loopStartFrame?: number;
    tailFrames: number;
    /** The body from `loopStartBar` to `window.toBar`, with its own effect state. */
    repeat?: RenderPass;
  };
  window: { fromBar: number; toBar: number; layout: SegmentLayout };
  /** Mapped musical frames in the opening or standard render. */
  musicFrames: number;
  cps: number;
  structure: SongStructure;
  /** The section each layout bar in `[0, c1)` plays in, where `c1` is `window.toBar` plus a
   *  single chunk's overlap: runs in bar order from `locate` on the segments the schedule query
   *  used, with a bar in no section in no run. The runs start at bar 0, so they cover the
   *  lookback before `window.fromBar` and a loop preset's repeat pass. The engine's refusal
   *  names a first bar's section from them. */
  sections: PlannedSection[];
  /** `lookbackSeconds(events)` of the full schedule (the 'auto' lookback). */
  lookbackSec: number;
  /** Captured and selection errors of every chunk, deduped, first occurrence kept. */
  errors: RenderError[];
  /** Present only with `includeEvents`; the opening's schedule for a loop export. */
  events?: RenderEvent[];
  /** Every chunk of the opening or standard render; for a `chunk` request, exactly one, with
   *  its `info`. */
  chunks: RenderPass['chunks'];
}

/** The widest render window, in bars (the host's request limit). */
export const MAX_RENDER_BARS = 100_000;

const badRequest = (message: string): PlanJobError =>
  new PlanJobError('plan', `bad request: ${message}`);

const isIntIn = (v: unknown, min: number, max: number): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max;

/** Checks the request's shape, its length and its `chunk`: the one length validator. */
function validateRenderRequest(request: unknown): asserts request is RenderPlanRequest {
  if (!isRecord(request)) throw badRequest('not a JSON object');
  const { includeEvents, length, chunk } = request;
  if (includeEvents !== undefined && typeof includeEvents !== 'boolean') {
    throw badRequest('includeEvents must be true or false');
  }
  if (
    request.preset !== undefined &&
    (typeof request.preset !== 'string' ||
      !['tagged-loop', 'intro-loop', 'folded-loop', 'one-shot'].includes(request.preset))
  )
    throw badRequest(`unknown preset ${String(request.preset)}`);
  if (!isRecord(length)) throw badRequest('length must be an object');
  if (chunk !== undefined) {
    if (!isRecord(chunk)) throw badRequest('chunk must be an object');
    if (length.kind !== 'window') throw badRequest('chunk needs a window length');
    if (request.preset !== undefined) throw badRequest('chunk cannot be combined with a preset');
  }
  if (
    request.preset !== undefined &&
    request.preset !== 'one-shot' &&
    (length.kind !== 'clip' || length.loops !== undefined)
  ) {
    throw new PlanJobError(
      'meta',
      'loop presets cannot be combined with --section, --length or --loops',
      '--preset',
    );
  }
  switch (length.kind) {
    case 'clip':
      if (length.loops !== undefined && !isIntIn(length.loops, 1, MAX_LOOPS)) {
        throw badRequest(`--loops must be an integer from 1 to ${MAX_LOOPS}`);
      }
      break;
    case 'section':
      if (typeof length.name !== 'string') throw badRequest('length.name must be a string');
      break;
    case 'length':
      if (!isIntIn(length.bars, 1, MAX_RENDER_BARS)) {
        throw badRequest(`length.bars must be an integer from 1 to ${MAX_RENDER_BARS}`);
      }
      break;
    case 'window': {
      const { fromBar, toBar, layout } = length;
      if (
        !isIntIn(fromBar, 0, MAX_RENDER_BARS) ||
        !isIntIn(toBar, 1, MAX_RENDER_BARS) ||
        fromBar >= toBar
      ) {
        throw badRequest(
          `length window must be integers with 0 <= fromBar < toBar <= ${MAX_RENDER_BARS}`,
        );
      }
      if (layout !== 'clip' && layout !== 'linear') {
        throw badRequest(`unknown layout ${String(layout)}`);
      }
      break;
    }
    default:
      throw badRequest(`unknown length kind ${String(length.kind)}`);
  }
  if (isRecord(chunk)) validateChunk(chunk, length.toBar as number);
}

/** The single chunk's ranges, once its window is known to be valid (`toBar` in range). */
function validateChunk(chunk: Record<string, unknown>, toBar: number): void {
  const { overlapBars, lookbackSec, mode } = chunk;
  if (!isIntIn(overlapBars, 0, MAX_RENDER_BARS - toBar)) {
    throw badRequest(`chunk.overlapBars must be an integer from 0 to ${MAX_RENDER_BARS - toBar}`);
  }
  if (
    lookbackSec !== 'auto' &&
    !(typeof lookbackSec === 'number' && Number.isFinite(lookbackSec) && lookbackSec >= 0)
  ) {
    throw badRequest("chunk.lookbackSec must be 'auto' or a number of seconds >= 0");
  }
  if (mode !== 'mix' && mode !== 'stems') throw badRequest(`unknown chunk mode ${String(mode)}`);
}

/** `structure` with the `--loops` override (unchanged without one), as a flagged `meta` error
 *  when the song has no loop section. */
function loopsApplied(structure: SongStructure, loops: number | undefined): SongStructure {
  if (loops === undefined) return structure;
  if (authoredLoop(structure) === null) {
    throw new PlanJobError(
      'meta',
      '--loops needs meta.loop or a section with loop: true in meta.sections',
      '--loops',
    );
  }
  try {
    return withLoops(structure, loops);
  } catch (e) {
    if (e instanceof StructureError) throw new PlanJobError('meta', e.message, '--loops');
    throw e;
  }
}

/** The render window and layout a length request asks for, from the song's structure (with any
 *  `loops` override applied). A window ending past `MAX_RENDER_BARS` (a clip of many or long
 *  loops, a late section) is refused like an explicit window would be. Throws the flagged `meta`
 *  errors. */
function resolveWindow(
  length: LengthRequest,
  structure: SongStructure,
): { fromBar: number; toBar: number; layout: SegmentLayout } {
  const sections = structure.sections;
  switch (length.kind) {
    case 'window':
      return { fromBar: length.fromBar, toBar: length.toBar, layout: length.layout };
    case 'clip': {
      if (sections.length === 0) {
        throw new PlanJobError(
          'meta',
          'this song has no sections; pass --length N to say how many bars to render',
          '--length',
        );
      }
      const { clipBars } = structure;
      if (clipBars > MAX_RENDER_BARS) {
        const limit = `past the render limit of ${MAX_RENDER_BARS} bars`;
        if (length.loops !== undefined) {
          throw new PlanJobError(
            'meta',
            `--loops ${length.loops} makes the clip ${clipBars} bars, ${limit}`,
            '--loops',
          );
        }
        const advice = authoredLoop(structure) === null ? '--length N' : '--length N or --loops N';
        throw new PlanJobError(
          'meta',
          `the song's clip is ${clipBars} bars, ${limit}; pass ${advice}`,
          '--length',
        );
      }
      return { fromBar: 0, toBar: clipBars, layout: 'clip' };
    }
    case 'section': {
      if (sections.length === 0) {
        throw new PlanJobError(
          'meta',
          'this song has no sections; --section needs meta.sections',
          '--section',
        );
      }
      const section = sections.find((s) => s.name === length.name);
      if (!section) {
        throw new PlanJobError(
          'meta',
          `unknown section '${length.name}' (sections: ${sections.map((s) => s.name).join(', ')})`,
          '--section',
        );
      }
      const toBar = section.start + section.bars;
      if (toBar > MAX_RENDER_BARS) {
        throw new PlanJobError(
          'meta',
          `section '${section.name}' ends at bar ${toBar}, past the render limit of ${MAX_RENDER_BARS} bars`,
          '--section',
        );
      }
      return { fromBar: section.start, toBar, layout: 'linear' };
    }
    case 'length':
      if (sections.length === 0) return { fromBar: 0, toBar: length.bars, layout: 'linear' };
      if (length.bars > structure.clipBars) {
        throw new PlanJobError(
          'meta',
          `--length ${length.bars} is past the end of the song (${structure.clipBars} bars)`,
          '--length',
        );
      }
      return { fromBar: 0, toBar: length.bars, layout: 'clip' };
  }
}

/** Loop exports use a single linear pass through the evaluated song, ending at the authored
 *  loop boundary even when meta.loops repeats it or the author wrote an outro. */
function exportWindow(request: RenderPlanRequest, structure: SongStructure) {
  if (!request.preset || request.preset === 'one-shot') {
    return { window: resolveWindow(request.length, structure), loopStartBar: undefined };
  }
  const loop = authoredLoop(structure);
  if (!loop) {
    throw new PlanJobError(
      'meta',
      '--preset needs meta.loop or a section with loop: true in meta.sections',
      '--preset',
    );
  }
  if (loop.first === loop.last) {
    // Preserve the legacy section-window bounds and diagnostics for a single marked section.
    const window = resolveWindow(
      { kind: 'section', name: structure.sections[loop.first]!.name },
      structure,
    );
    if (request.preset !== 'folded-loop') window.fromBar = 0;
    return { window, loopStartBar: loop.start };
  }
  if (loop.end > MAX_RENDER_BARS) {
    throw new PlanJobError(
      'meta',
      `the loop ends at bar ${loop.end}, past the render limit of ${MAX_RENDER_BARS} bars`,
      '--preset',
    );
  }
  const window = {
    fromBar: request.preset === 'folded-loop' ? loop.start : 0,
    toBar: loop.end,
    layout: 'linear' as const,
  };
  return { window, loopStartBar: loop.start };
}

/** The first event at or after `bar` in an onset-sorted selection. */
function onsetIndex(selected: Selected[], bar: number): number {
  let lo = 0;
  let hi = selected.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (selected[mid]!.onsetCycle < bar) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** The runs of `[0, c1)` that `locate` places in one section, merged while the section
 *  index stays the same on consecutive bars. */
function sectionRuns(
  structure: SongStructure,
  segments: readonly Segment[],
  c1: number,
): PlannedSection[] {
  const runs: PlannedSection[] = [];
  let open = -1; // the section index of runs.at(-1) when it ends at `bar`, else -1
  for (let bar = 0; bar < c1; bar++) {
    const at = locate(bar, structure, segments);
    if (!at) {
      open = -1;
      continue;
    }
    if (at.section === open) runs.at(-1)!.toBar = bar + 1;
    else runs.push({ fromBar: bar, toBar: bar + 1, name: structure.sections[at.section]!.name });
    open = at.section;
  }
  return runs;
}

/** Onsets from `bar` onward, numbered from the start of this selection. */
function onsetsFrom(selected: Selected[], bar: number): Selected[] {
  const first = onsetIndex(selected, bar);
  return first === 0 ? selected : selected.slice(first).map((s, index) => ({ ...s, index }));
}

/** The schedule selection up to `c1`: haps with original indices and errors before the cut. */
export function cutSchedule(full: ScheduleSelection, c1: number): ScheduleSelection {
  return {
    selected: full.selected.slice(0, onsetIndex(full.selected, c1)),
    errors: full.errors.filter((e) => e.bar === undefined || e.bar < c1),
  };
}

/**
 * Plans every chunk of a render in one call: evaluates once, resolves the length request against
 * the evaluated structure (the one playback follows; `--loops` replaces its loop count), queries
 * the schedule once, and plans each chunk's engine job (`renderChunkPlans`, no seam overlap).
 * Each chunk's selection is the schedule cut at the chunk's end; loop passes share the selected
 * schedule. A `chunk` request plans exactly one chunk over its window, with the chunk's overlap,
 * lookback ('auto': the schedule's) and mode, and carries its `info`; its job is byte-equal to
 * the matching chunk of a full plan. It names the section of every layout bar it schedules
 * (`sections`). `captured` is read as each chunk is planned. Throws
 * `PlanJobError` (length errors are kind 'meta' with a `flag`; a header or evaluation error comes
 * first).
 */
export async function planRender(
  rt: PlannerRuntime,
  code: string,
  request: RenderPlanRequest,
  captured: RenderError[],
  onPhase?: (p: PlanJobPhase) => void,
): Promise<PlannedRender> {
  validateRenderRequest(request);
  const loops = request.length.kind === 'clip' ? request.length.loops : undefined;
  const song = await evaluateState(rt, code, onPhase);
  const { cps, seed } = song;
  return asPlanError(() => {
    // The window, the clip segments and the reported structure all come from one structure.
    const structure = loopsApplied(song.structure, loops);
    const { window, loopStartBar } = exportWindow(request, structure);
    const { fromBar, toBar, layout } = window;
    const segments = layoutSegments(layout, structure);
    const timing = renderTiming(structure, segments, cps);
    const secondsAt = (bar: number) => (timing ? timing.secondsAt(bar) : bar / cps);
    const framesBetween = (from: number, to: number) =>
      renderWindowFrames(from, to, cps, SAMPLE_RATE, timing);
    const musicalFrames = framesBetween(fromBar, toBar);
    const bodyFrames = loopStartBar === undefined ? 0 : framesBetween(loopStartBar, toBar);
    const programFrames = musicalFrames + bodyFrames;
    if (request.preset) checkBufferFrames(programFrames, 'mix');
    const query = querySong(rt, segments);
    const frozenSchedule = loopStartBar !== undefined;
    const ctx: WindowContext = {
      query,
      cps,
      seed,
      structure: song.structure,
      frozenSchedule,
      timing,
    };
    const base: RenderRequest = {
      code,
      layout,
      fromBar,
      toBar,
      overlapBars: 0,
      lookbackSec: 0,
      mode: request.chunk?.mode ?? 'mix',
      sampleRate: SAMPLE_RATE,
    };
    const single = request.chunk;
    const errors: RenderError[] = [];
    let lookbackSec = 0;

    function planPass(
      full: ScheduleSelection,
      start: number,
      scheduleStart: number,
      keepTail: boolean,
    ) {
      const chunks: RenderPass['chunks'] = [];
      const events =
        !request.preset || (request.includeEvents && start === fromBar)
          ? eventsFromValues(eventSources(full.selected), cps, timing)
          : [];
      const exportEvents = request.preset
        ? full.selected.map((s) =>
            exportTimingEvent(s.value, s.onsetCycle, Number(s.hap.duration), cps, timing),
          )
        : undefined;
      const decay = exportEvents ? exportDecay(exportEvents, secondsAt(toBar)) : undefined;
      const history = decay?.lookbackSec ?? lookbackSeconds(events);
      lookbackSec = Math.max(lookbackSec, history);
      const tailFrames = keepTail ? (decay?.tailFrames ?? 0) : 0;
      if (keepTail) checkBufferFrames(programFrames + tailFrames, 'mix');
      // Each request's lookback is resolved: the chunk plan's, or the single chunk's ('auto' is
      // the schedule's history).
      const requests: (RenderRequest & { lookbackSec: number })[] = single
        ? [
            {
              ...base,
              overlapBars: single.overlapBars,
              lookbackSec: single.lookbackSec === 'auto' ? history : single.lookbackSec,
            },
          ]
        : renderChunkPlans({
            fromBar: start,
            toBar,
            events: exportEvents ?? events,
            cps,
            overlapBars: 0,
            lookbackSec: history,
            timing,
          }).map((p) => ({ ...chunkRequest(base, p), lookbackSec: p.lookbackSec }));
      for (const req of requests) {
        const chunkLookback =
          scheduleStart === 0
            ? req.lookbackSec
            : Math.min(
                req.lookbackSec,
                timing
                  ? secondsAt(req.fromBar) - secondsAt(scheduleStart)
                  : (req.fromBar - scheduleStart) / cps,
              );
        const schedule = cutSchedule(full, req.toBar + req.overlapBars);
        const { info, json } = planWindow(
          ctx,
          req,
          chunkLookback,
          schedule,
          captured,
          scheduleStart,
          req.toBar === toBar ? tailFrames : 0,
        );
        errors.push(...info.errors);
        chunks.push({
          json,
          plan: {
            ...info.plan,
            fromBar: req.fromBar,
            toBar: req.toBar,
            lookbackSec: chunkLookback,
          },
          ...(single ? { info: { events: info.events, stemPairs: info.stemPairs } } : {}),
        });
      }
      return { chunks, events, tailFrames };
    }

    // The opening keeps intro effects. The repeat starts at the authored body and adds only its
    // own decay after the final bar. When there is no intro to render, one pass supplies both.
    const separateRepeat = loopStartBar !== undefined && loopStartBar > fromBar;
    const scheduleStart = loopStartBar === fromBar ? fromBar : 0;
    const c1 = toBar + (single?.overlapBars ?? 0);
    const sections = sectionRuns(structure, segments, c1);
    const full = selectHaps(query(scheduleStart, c1));
    const opening = planPass(full, fromBar, scheduleStart, !separateRepeat && !!request.preset);
    const repeat = separateRepeat
      ? planPass(
          {
            selected: onsetsFrom(full.selected, loopStartBar),
            errors: full.errors.filter((e) => e.bar === undefined || e.bar >= loopStartBar),
          },
          loopStartBar,
          loopStartBar,
          true,
        )
      : undefined;
    return {
      window,
      musicFrames: musicalFrames,
      cps,
      structure,
      sections,
      lookbackSec,
      errors: dedupeErrors(errors),
      ...(request.includeEvents === true ? { events: opening.events } : {}),
      ...(request.preset
        ? {
            export: {
              ...(loopStartBar === undefined
                ? {}
                : { loopStartBar, loopStartFrame: framesBetween(fromBar, loopStartBar) }),
              tailFrames: opening.tailFrames,
              ...(repeat
                ? {
                    repeat: {
                      chunks: repeat.chunks,
                      tailFrames: repeat.tailFrames,
                      musicFrames: bodyFrames,
                    },
                  }
                : {}),
            },
          }
        : {}),
      chunks: opening.chunks,
    };
  });
}

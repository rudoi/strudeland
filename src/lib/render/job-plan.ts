/**
 * The engine-job planner shared by the render page's `job` kind (`src/render/main.ts`) and the
 * headless host: evaluate a song through a `PlannerRuntime`, query it, and plan the chunk's
 * engine job (`planJob`), or every chunk of a render in one call (`planRender`). Strudel- and
 * DOM-free: the runtime comes in through the adapter.
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
import type { Segment } from '../strudel/transport';
import { SAMPLE_RATE, type RenderError, type RenderRequest, type SegmentLayout } from './contract';
import { chunkRequest, renderChunkPlans } from './chunks';
import { planEngineJob } from './engine-job';
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

/**
 * Plans `request`'s engine job: the same plan the page's render path gives `renderOffline`,
 * without a context. `captured` (errors captured while the song ran) is read once, last.
 * Throws `PlanJobError`; its message is the text the render page reports.
 */
export async function planJob(
  rt: PlannerRuntime,
  request: RenderRequest,
  captured: RenderError[],
  onPhase?: (p: PlanJobPhase) => void,
): Promise<{ info: PageJobInfo; json: string }> {
  const { structure, cps, seed, segments, timing } = await evaluateSong(
    rt,
    request.code,
    request.layout,
    onPhase,
  );
  return asPlanError(() => {
    const c1 = request.toBar + request.overlapBars;
    const query = querySong(rt, segments);

    // The full schedule [0, c1), selected once: the 'auto' lookback and every orbit the render
    // must create up front.
    const schedule = selectHaps(query(0, c1));
    const events = eventsFromValues(eventSources(schedule.selected), cps, timing);
    const lookbackSec =
      request.lookbackSec === 'auto' ? lookbackSeconds(events) : request.lookbackSec;
    return planWindow(
      { query, cps, seed, structure, timing },
      request,
      lookbackSec,
      schedule,
      captured,
    );
  });
}

// ---- planRender: every chunk of a render in one call

/** What to render. `window` is for the harness and the dev binary; the command line never
 *  sends it. */
export type LengthRequest =
  | { kind: 'clip'; loops?: number }
  | { kind: 'section'; name: string }
  | { kind: 'length'; bars: number }
  | { kind: 'window'; fromBar: number; toBar: number; layout: SegmentLayout };

export type ExportPreset = 'tagged-loop' | 'intro-loop' | 'folded-loop' | 'one-shot';

export interface RenderPlanRequest {
  preset?: ExportPreset;
  length: LengthRequest;
  /** 'stems' is rejected: a render is the mix. */
  mode: 'mix';
  sampleRate: 44100;
  /** Also return the schedule's events. Default false. */
  includeEvents?: boolean;
}

/** A chunk's `WindowPlan` plus its content window and the exact lookback its request carries
 *  (`lookbackSecUsed` is not always equal to it in floating point). */
export type ChunkWindowPlan = WindowPlan & { fromBar: number; toBar: number; lookbackSec: number };

interface RenderPass {
  chunks: { json: string; plan: ChunkWindowPlan }[];
  tailFrames: number;
  musicFrames?: number;
}

export interface PlannedRender {
  /** Extra frames on the last job and the isolated body when the opening includes an intro. */
  export?: {
    loopStartBar?: number;
    /** Mapped musical frame offset from the opening's start. */
    loopStartFrame?: number;
    tailFrames: number;
    /** The body from `loopStartBar` to `window.toBar`, with its own effect state. */
    repeat?: RenderPass;
  };
  window: { fromBar: number; toBar: number; layout: SegmentLayout };
  /** Mapped musical frames in the opening or standard render. */
  musicFrames?: number;
  cps: number;
  structure: SongStructure;
  /** `lookbackSeconds(events)` of the full schedule (the 'auto' lookback). */
  lookbackSec: number;
  /** Captured and selection errors of every chunk, deduped, first occurrence kept. */
  errors: RenderError[];
  /** Present only with `includeEvents`; the opening's schedule for a loop export. */
  events?: RenderEvent[];
  chunks: RenderPass['chunks'];
}

/** The widest render window, in bars (the host's request limit). */
export const MAX_RENDER_BARS = 100_000;

/**
 * True: each chunk's schedule selection is the full schedule's cut at the chunk's end (haps with
 * onset before it, original `index` kept). False: each chunk queries and selects [0, c1) itself,
 * as `planJob` does (quadratic, equal by construction). Kept `true` while the batch parity check
 * finds the jobs byte-equal to the page's.
 */
export const BATCH_REUSE_SCHEDULE = true;

const badRequest = (message: string): PlanJobError =>
  new PlanJobError('plan', `bad request: ${message}`);

const LOOPS_MESSAGE = `--loops must be an integer from 1 to ${MAX_LOOPS}`;

const isIntIn = (v: unknown, min: number, max: number): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max;

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** Checks the request's shape (the host's request type checked the same rules first). */
function validateRenderRequest(request: unknown): asserts request is RenderPlanRequest {
  if (!isObject(request)) throw badRequest('not a JSON object');
  const { mode, sampleRate, includeEvents, length } = request;
  if (mode === 'stems') throw badRequest('mode: stems is not supported yet; v1 renders the mix');
  if (mode !== 'mix') throw badRequest(`unknown mode ${String(mode)}`);
  if (sampleRate !== SAMPLE_RATE) throw badRequest(`sampleRate must be ${SAMPLE_RATE}`);
  if (includeEvents !== undefined && typeof includeEvents !== 'boolean') {
    throw badRequest('includeEvents must be true or false');
  }
  if (
    request.preset !== undefined &&
    (typeof request.preset !== 'string' ||
      !['tagged-loop', 'intro-loop', 'folded-loop', 'one-shot'].includes(request.preset))
  )
    throw badRequest(`unknown preset ${String(request.preset)}`);
  if (!isObject(length)) throw badRequest('length must be an object');
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
        throw new PlanJobError('meta', LOOPS_MESSAGE, '--loops');
      }
      return;
    case 'section':
      if (typeof length.name !== 'string') throw badRequest('length.name must be a string');
      return;
    case 'length':
      if (!isIntIn(length.bars, 1, MAX_RENDER_BARS)) {
        throw badRequest(`length.bars must be an integer from 1 to ${MAX_RENDER_BARS}`);
      }
      return;
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
      return;
    }
    default:
      throw badRequest(`unknown length kind ${String(length.kind)}`);
  }
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
 * Loop passes share the selected schedule. Standard jobs are byte-equal to `planJob` of the
 * same chunk request. `captured` is read as each chunk is planned. Throws `PlanJobError` (length
 * errors are kind 'meta' with a `flag`; a header or evaluation error comes first).
 */
export async function planRender(
  rt: PlannerRuntime,
  code: string,
  request: RenderPlanRequest,
  captured: RenderError[],
  onPhase?: (p: PlanJobPhase) => void,
  /** Tests only: the schedule rule to use instead of `BATCH_REUSE_SCHEDULE`. */
  reuseSchedule: boolean = BATCH_REUSE_SCHEDULE,
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
      mode: 'mix',
      sampleRate: SAMPLE_RATE,
    };
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
      const plans = renderChunkPlans({
        fromBar: start,
        toBar,
        events: exportEvents ?? events,
        cps,
        overlapBars: 0,
        lookbackSec: history,
        timing,
      });
      for (const plan of plans) {
        const req = chunkRequest(base, plan);
        const chunkLookback =
          scheduleStart === 0
            ? plan.lookbackSec
            : Math.min(
                plan.lookbackSec,
                timing
                  ? secondsAt(req.fromBar) - secondsAt(scheduleStart)
                  : (req.fromBar - scheduleStart) / cps,
              );
        const schedule =
          frozenSchedule || reuseSchedule
            ? cutSchedule(full, req.toBar)
            : selectHaps(query(scheduleStart, req.toBar));
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
        });
      }
      return { chunks, events, tailFrames };
    }

    // The opening keeps intro effects. The repeat starts at the authored body and adds only its
    // own decay after the final bar. When there is no intro to render, one pass supplies both.
    const separateRepeat = loopStartBar !== undefined && loopStartBar > fromBar;
    const scheduleStart = loopStartBar === fromBar ? fromBar : 0;
    const full = selectHaps(query(scheduleStart, toBar));
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
      ...(timing ? { musicFrames: musicalFrames } : {}),
      cps,
      structure,
      lookbackSec,
      errors: dedupeErrors(errors),
      ...(request.includeEvents === true ? { events: opening.events } : {}),
      ...(request.preset
        ? {
            export: {
              ...(loopStartBar === undefined ? {} : { loopStartBar }),
              ...(timing && loopStartBar !== undefined
                ? { loopStartFrame: framesBetween(fromBar, loopStartBar) }
                : {}),
              tailFrames: opening.tailFrames,
              ...(repeat
                ? {
                    repeat: {
                      chunks: repeat.chunks,
                      tailFrames: repeat.tailFrames,
                      ...(timing ? { musicFrames: bodyFrames } : {}),
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

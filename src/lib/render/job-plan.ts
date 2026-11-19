/**
 * The engine-job planner shared by the render page's `job` kind (`src/render/main.ts`) and the
 * headless host: evaluate a song through a `PlannerRuntime`, query it, and plan the chunk's
 * engine job (`planJob`), or every chunk of a render in one call (`planRender`). Strudel- and
 * DOM-free: the runtime comes in through the adapter.
 */

import { errorMessage } from '../errors';
import { evaluateOptions, staticStructure } from '../project/evaluate-options';
import { parseMeta, type Meta } from '../project/meta';
import type { EvaluateOptions } from '../strudel/runtime';
import { MAX_LOOPS, StructureError, withLoops, type SongStructure } from '../strudel/structure';
import { clipSegments, LINEAR_SEGMENTS, type Segment } from '../strudel/transport';
import { SAMPLE_RATE, type RenderError, type RenderRequest, type SegmentLayout } from './contract';
import { chunkRequest, renderChunkPlans } from './chunks';
import { planEngineJob } from './engine-job';
import { lookbackSeconds } from './lookback';
import { pageJob, dedupeErrors, type PageJobInfo } from './page-job';
import { eventSources, selectHaps, windowPlan, type Selected, type WindowPlan } from './plan';
import { eventsFromValues, type RenderEvent } from './schedule';

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
 *  `--section`, `--loops`); only `planRender` sets it, and only with kind 'meta'. */
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

/** `parseMeta`, a header error as the planner's `meta` error. */
function parsedMeta(code: string): Meta {
  const { meta, error: metaError } = parseMeta(code);
  if (metaError) {
    throw new PlanJobError(
      'meta',
      `meta: ${metaError.message} (${metaError.line}:${metaError.column + 1})`,
    );
  }
  return meta;
}

/** The segments of `layout`: the clip arrangement of `structure`, or the linear timeline. */
function layoutSegments(layout: SegmentLayout, structure: SongStructure): readonly Segment[] {
  return layout === 'clip' ? clipSegments(structure) : LINEAR_SEGMENTS;
}

/** `evaluateSong` before the segments: parse, evaluate, read the state. */
async function evaluateState(
  rt: PlannerRuntime,
  code: string,
  onPhase?: (p: PlanJobPhase) => void,
): Promise<Omit<EvaluatedSong, 'segments'>> {
  onPhase?.('evaluating');
  const meta = parsedMeta(code);
  if (!(await rt.evaluate(code, evaluateOptions(meta)))) {
    throw new PlanJobError('eval', rt.evalError() ?? 'evaluate failed');
  }
  onPhase?.('querying');
  return asPlanError(() => {
    const state = rt.state();
    const structure = state.structure ?? staticStructure(meta);
    return { meta, structure, cps: state.cps, seed: state.seed ?? 0 };
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
  return asPlanError(() => ({ ...song, segments: layoutSegments(layout, song.structure) }));
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
}

/** A `selectHaps` result over the schedule [0, c1). */
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
): { info: PageJobInfo; json: string } {
  const plan = windowPlan(request, ctx.cps, lookbackSec);
  // The chunk window ends at c1 = toBar + overlapBars (`windowPlan`'s definition); when it also
  // starts at 0 it is the schedule's window, and querying it again would return the same haps in
  // the same order (queries are pure), so the schedule's selection is reused. A window starting
  // later is queried: a hap crossing c0 comes back as a fragment there, which filtering the
  // schedule's haps would not reproduce.
  const chunk = plan.c0 === 0 ? schedule : selectHaps(ctx.query(plan.c0, plan.c1));
  const planned = planEngineJob(
    chunk.selected,
    schedule.selected,
    plan,
    request,
    ctx.cps,
    ctx.seed,
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
  const { structure, cps, seed, segments } = await evaluateSong(
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
    const events = eventsFromValues(eventSources(schedule.selected), cps);
    const lookbackSec =
      request.lookbackSec === 'auto' ? lookbackSeconds(events) : request.lookbackSec;
    return planWindow({ query, cps, seed, structure }, request, lookbackSec, schedule, captured);
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

export interface RenderPlanRequest {
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

export interface PlannedRender {
  window: { fromBar: number; toBar: number; layout: SegmentLayout };
  cps: number;
  structure: SongStructure;
  /** `lookbackSeconds(events)` of the full schedule (the 'auto' lookback). */
  lookbackSec: number;
  /** Captured and selection errors of every chunk, deduped, first occurrence kept. */
  errors: RenderError[];
  /** Present only with `includeEvents`; equals `ScheduleResult.events` for [0, toBar). */
  events?: RenderEvent[];
  chunks: { json: string; plan: ChunkWindowPlan }[];
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
  if (!isObject(length)) throw badRequest('length must be an object');
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
  if (structure.loopIndex === null) {
    throw new PlanJobError(
      'meta',
      '--loops needs a section with loop: true in meta.sections',
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
        const advice = structure.loopIndex === null ? '--length N' : '--length N or --loops N';
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

/** The schedule selection up to `c1` (an integer bar): the haps with onset before it, in order,
 *  with their original `index`, and the errors of the bars before it. The selection is sorted by
 *  onset, so the haps are a prefix. */
export function cutSchedule(full: ScheduleSelection, c1: number): ScheduleSelection {
  let lo = 0;
  let hi = full.selected.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (full.selected[mid]!.onsetCycle < c1) lo = mid + 1;
    else hi = mid;
  }
  return {
    selected: full.selected.slice(0, lo),
    errors: full.errors.filter((e) => e.bar === undefined || e.bar < c1),
  };
}

/**
 * Plans every chunk of a render in one call: evaluates once, resolves the length request against
 * the evaluated structure (the one playback follows; `--loops` replaces its loop count), queries
 * the schedule once, plans the chunks (`renderChunkPlans`, no seam overlap) and each chunk's
 * engine job, byte-equal to `planJob` of the same chunk request. `captured` is shared by the
 * chunks and read as each one is planned. Throws `PlanJobError` (length errors are kind 'meta'
 * with a `flag`; a header or evaluation error comes first).
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
    const window = resolveWindow(request.length, structure);
    const { fromBar, toBar, layout } = window;
    const query = querySong(rt, layoutSegments(layout, structure));
    const full = selectHaps(query(0, toBar));
    const events = eventsFromValues(eventSources(full.selected), cps);
    const lookbackSec = lookbackSeconds(events);
    const plans = renderChunkPlans({ fromBar, toBar, events, cps, overlapBars: 0 });
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
    // The chunk window's structure is the song's own, as `planJob` reports it.
    const ctx: WindowContext = { query, cps, seed, structure: song.structure };
    const chunks: PlannedRender['chunks'] = [];
    const errors: RenderError[] = [];
    for (const plan of plans) {
      const req = chunkRequest(base, plan);
      const c1 = req.toBar + req.overlapBars;
      const schedule = reuseSchedule ? cutSchedule(full, c1) : selectHaps(query(0, c1));
      const { info, json } = planWindow(ctx, req, plan.lookbackSec, schedule, captured);
      errors.push(...info.errors);
      chunks.push({
        json,
        plan: {
          ...info.plan,
          fromBar: req.fromBar,
          toBar: req.toBar,
          lookbackSec: plan.lookbackSec,
        },
      });
    }
    return {
      window,
      cps,
      structure,
      lookbackSec,
      errors: dedupeErrors(errors),
      ...(request.includeEvents === true ? { events } : {}),
      chunks,
    };
  });
}

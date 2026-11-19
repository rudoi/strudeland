import { describe, expect, test } from 'bun:test';
import { evaluateOptions, staticStructure } from '../src/lib/project/evaluate-options';
import { parseMeta } from '../src/lib/project/meta';
import type { RenderError, RenderRequest } from '../src/lib/render/contract';
import { planEngineJob } from '../src/lib/render/engine-job';
import { chunkRequest, renderChunkPlans } from '../src/lib/render/chunks';
import {
  cutSchedule,
  evaluateSong,
  planJob,
  PlanJobError,
  planRender,
  querySong,
  type LengthRequest,
  type PlanJobPhase,
  type PlannerRuntime,
  type RenderPlanRequest,
} from '../src/lib/render/job-plan';
import { lookbackSeconds } from '../src/lib/render/lookback';
import { dedupeErrors, pageJob } from '../src/lib/render/page-job';
import { eventSources, selectHaps, windowPlan } from '../src/lib/render/plan';
import { eventsFromValues } from '../src/lib/render/schedule';
import type { EvaluateOptions } from '../src/lib/strudel/runtime';
import { buildStructure, withLoops, type SongStructure } from '../src/lib/strudel/structure';
import { clipSegments, LINEAR_SEGMENTS, type Segment } from '../src/lib/strudel/transport';
import { hap, req } from './helpers/fake-offline';

/** Haps on [0, 4): two notes per bar, one with a non-object value (a select error) in bar 1. */
function songHaps(from: number, to: number): any[] {
  const out: any[] = [];
  for (let b = Math.floor(from); b < to; b++) {
    out.push(hap(b, b + 0.5, { s: 'sine', note: 60 + b }));
    out.push(hap(b + 0.5, b + 1, { s: 'square', note: 48 + b, orbit: 2 }));
    if (b === 1) out.push(hap(b + 0.25, b + 0.5, 'not an object'));
  }
  return out;
}

interface FakeOptions {
  evaluateResult?: boolean;
  evaluateThrows?: unknown;
  evalError?: string | null;
  structure?: SongStructure | null;
  cps?: number;
  seed?: number | undefined;
  /** Replaces the query; `null` result means "before a successful evaluate". */
  query?: (from: number, to: number, segments: readonly Segment[]) => any[] | null;
  stateThrows?: unknown;
}

/** A fake runtime that records every call, in order. */
function fakeRuntime(o: FakeOptions = {}) {
  const calls: string[] = [];
  const evaluated: { code: string; opts: EvaluateOptions }[] = [];
  const queries: { from: number; to: number; segments: readonly Segment[] }[] = [];
  const rt: PlannerRuntime = {
    async evaluate(code, opts) {
      calls.push('evaluate');
      evaluated.push({ code, opts });
      if (o.evaluateThrows !== undefined) throw o.evaluateThrows;
      return o.evaluateResult ?? true;
    },
    evalError() {
      calls.push('evalError');
      return o.evalError === undefined ? null : o.evalError;
    },
    state() {
      calls.push('state');
      if (o.stateThrows !== undefined) throw o.stateThrows;
      return {
        structure: o.structure === undefined ? null : o.structure,
        cps: o.cps ?? 0.5,
        seed: 'seed' in o ? o.seed : 7,
      };
    },
    query(from, to, segments) {
      calls.push(`query ${from} ${to}`);
      queries.push({ from, to, segments });
      return o.query ? o.query(from, to, segments) : songHaps(from, to);
    },
  };
  return { rt, calls, evaluated, queries };
}

const CODE = "const meta = { bpm: 120, meter: '4/4' };\nnote('c').s('sine')";

/** What the page produced before the shared planner, computed by hand from the same haps. */
function expected(
  request: RenderRequest,
  cps: number,
  seed: number,
  structure: SongStructure,
  captured: RenderError[] = [],
) {
  const c1 = request.toBar + request.overlapBars;
  const { selected, errors: selectErrors } = selectHaps(songHaps(0, c1));
  const events = eventsFromValues(eventSources(selected), cps);
  const lookbackSec =
    request.lookbackSec === 'auto' ? lookbackSeconds(events) : request.lookbackSec;
  const plan = windowPlan(request, cps, lookbackSec);
  const chunk = selectHaps(songHaps(plan.c0, plan.c1));
  const planned = planEngineJob(chunk.selected, selected, plan, request, cps, seed);
  const errors = dedupeErrors([...selectErrors, ...chunk.errors, ...captured]);
  return pageJob(planned, { errors, plan, cps, structure });
}

async function planError(p: Promise<unknown>): Promise<PlanJobError> {
  try {
    await p;
  } catch (e) {
    expect(e).toBeInstanceOf(PlanJobError);
    return e as PlanJobError;
  }
  throw new Error('planJob resolved; expected a PlanJobError');
}

describe('PlanJobError', () => {
  test('carries kind, message and name', () => {
    const e = new PlanJobError('eval', 'boom');
    expect(e).toBeInstanceOf(Error);
    expect(e.kind).toBe('eval');
    expect(e.message).toBe('boom');
    expect(e.name).toBe('PlanJobError');
  });
});

describe('planJob success', () => {
  test('equals pageJob(planEngineJob(...)) for the same haps (fixed lookback)', async () => {
    const request = req({ code: CODE, toBar: 3, overlapBars: 1, mode: 'stems' });
    const { rt, evaluated, queries } = fakeRuntime();
    const got = await planJob(rt, request, []);
    const meta = parseMeta(CODE).meta;
    const want = expected(request, 0.5, 7, staticStructure(meta));
    expect(got.json).toBe(want.json);
    expect(got.info).toEqual(want.info);
    expect(evaluated).toEqual([{ code: CODE, opts: evaluateOptions(meta) }]);
    // The chunk window [0, 4) is the schedule's: queried once, its selection reused.
    expect(queries.map((q) => [q.from, q.to])).toEqual([[0, 4]]);
  });

  test("'auto' lookback queries the chunk from the derived window", async () => {
    const request = req({ code: CODE, fromBar: 2, toBar: 4, lookbackSec: 'auto' });
    const { rt, queries } = fakeRuntime();
    const got = await planJob(rt, request, []);
    const want = expected(request, 0.5, 7, staticStructure(parseMeta(CODE).meta));
    expect(got.json).toBe(want.json);
    expect(got.info).toEqual(want.info);
    expect(queries[1]!.from).toBe(want.info.plan.c0);
    expect(queries[1]!.to).toBe(4);
  });

  test('a numeric lookback moves c0 back by whole bars', async () => {
    const request = req({ code: CODE, fromBar: 2, toBar: 3, lookbackSec: 2.5 });
    const { rt, queries } = fakeRuntime();
    const got = await planJob(rt, request, []);
    expect(got.info.plan.c0).toBe(0); // ceil(2.5 s * 0.5 cps) = 2 bars
    // The window reaches bar 0, so it is the schedule's window and is not queried again.
    expect(queries.map((q) => [q.from, q.to])).toEqual([[0, 3]]);
  });

  test('onPhase reports evaluating then querying', async () => {
    const phases: PlanJobPhase[] = [];
    const { rt, calls } = fakeRuntime();
    const order: string[] = [];
    rt.evaluate = ((orig) => async (c: string, o: EvaluateOptions) => {
      order.push(`evaluate during ${phases.at(-1)}`);
      return orig(c, o);
    })(rt.evaluate.bind(rt));
    await planJob(rt, req({ code: CODE }), [], (p) => phases.push(p));
    expect(phases).toEqual(['evaluating', 'querying']);
    expect(order).toEqual(['evaluate during evaluating']);
    expect(calls).toEqual(['evaluate', 'state', 'query 0 2']);
  });

  test('without onPhase it still plans', async () => {
    const { rt } = fakeRuntime();
    await expect(planJob(rt, req({ code: CODE }), [])).resolves.toBeDefined();
  });

  test('captured errors come after the select errors, deduped, read after the queries', async () => {
    const captured: RenderError[] = [
      { kind: 'worklet', message: 'w' },
      { kind: 'worklet', message: 'w' },
    ];
    const request = req({ code: CODE, toBar: 3 });
    const { rt } = fakeRuntime({
      query: (from, to) => {
        // The page appends while the song runs; the planner must see these.
        captured.push({ kind: 'sample', message: `during ${from}-${to}` });
        return songHaps(from, to);
      },
    });
    const got = await planJob(rt, request, captured);
    const selectMsg = got.info.errors[0]!;
    expect(selectMsg.kind).not.toBe('worklet');
    expect(selectMsg.message).toContain('hap at cycle 1.25');
    expect(selectMsg.bar).toBe(1);
    // The same select error in both selections is kept once.
    expect(got.info.errors.slice(1)).toEqual([
      { kind: 'worklet', message: 'w' },
      { kind: 'sample', message: 'during 0-3' },
    ]);
    // Appending after planJob returned does not change the result.
    captured.push({ kind: 'other', message: 'late' });
    expect(got.info.errors).toHaveLength(3);
  });

  test('seed defaults to 0 when the state has none', async () => {
    const request = req({ code: CODE });
    const structure = staticStructure(parseMeta(CODE).meta);
    const none = await planJob(fakeRuntime({ seed: undefined }).rt, request, []);
    expect(none.json).toBe(expected(request, 0.5, 0, structure).json);
    expect(JSON.parse(none.json).seed).toBe(0);
    const seven = await planJob(fakeRuntime({ seed: -7 }).rt, request, []);
    expect(JSON.parse(seven.json).seed).toBe(-7);
    expect(seven.json).toBe(expected(request, 0.5, -7, structure).json);
  });

  test("the runtime's structure wins over the header's", async () => {
    const structure = buildStructure({
      bpm: 90,
      meter: '3/4',
      sections: [{ name: 'a', bars: 8 }],
    });
    const got = await planJob(fakeRuntime({ structure }).rt, req({ code: CODE }), []);
    expect(got.info.structure).toBe(structure);
    expect(got.info.cps).toBe(0.5); // cps comes from the state, never the structure
  });

  test('clip layout queries clip segments of the structure; linear the linear ones', async () => {
    const structure = buildStructure({
      bpm: 120,
      meter: '4/4',
      loops: 2,
      sections: [
        { name: 'intro', bars: 1 },
        { name: 'loop', bars: 1, loop: true },
        { name: 'tail', bars: 1 },
      ],
    });
    const clip = fakeRuntime({ structure });
    await planJob(clip.rt, req({ code: CODE, layout: 'clip' }), []);
    expect(clip.queries.map((q) => q.segments)).toEqual([clipSegments(structure)]);
    const clipChunk = fakeRuntime({ structure });
    await planJob(clipChunk.rt, req({ code: CODE, layout: 'clip', fromBar: 1, toBar: 2 }), []);
    expect(clipChunk.queries.map((q) => q.segments)).toEqual([
      clipSegments(structure),
      clipSegments(structure),
    ]);
    expect(clipSegments(structure)).not.toEqual(LINEAR_SEGMENTS);
    const linear = fakeRuntime({ structure });
    await planJob(linear.rt, req({ code: CODE, layout: 'linear' }), []);
    for (const q of linear.queries) expect(q.segments).toBe(LINEAR_SEGMENTS);
  });

  test('clip layout without a runtime structure uses the header structure', async () => {
    const code =
      "const meta = { bpm: 120, loops: 3, sections: [{ name: 'a', bars: 1 }, { name: 'b', bars: 2, loop: true }] };\nx";
    const structure = staticStructure(parseMeta(code).meta);
    expect(structure.loopIndex).toBe(1);
    const f = fakeRuntime();
    await planJob(f.rt, req({ code, layout: 'clip' }), []);
    expect(f.queries[0]!.segments).toEqual(clipSegments(structure));
  });
});

describe('planJob query reuse', () => {
  const shapes: [string, Partial<RenderRequest>][] = [
    ['full window, mix', { toBar: 4 }],
    ['full window, stems', { toBar: 3, overlapBars: 1, mode: 'stems' }],
    ['full window, auto lookback', { toBar: 4, lookbackSec: 'auto' }],
    ['full window, clip layout', { toBar: 4, layout: 'clip' }],
    ['chunk from bar 0', { fromBar: 0, toBar: 2, overlapBars: 1 }],
    ['chunk, lookback reaching bar 0', { fromBar: 2, toBar: 3, lookbackSec: 4 }],
    ['chunk, lookback inside the song', { fromBar: 3, toBar: 4, lookbackSec: 2 }],
    ['chunk, no lookback', { fromBar: 2, toBar: 4, lookbackSec: 0 }],
    ['chunk, stems, auto lookback', { fromBar: 1, toBar: 3, mode: 'stems', lookbackSec: 'auto' }],
    ['chunk, clip layout', { fromBar: 2, toBar: 4, layout: 'clip' }],
  ];

  test.each(shapes)('%s: the job equals the two-query plan', async (_name, shape) => {
    const request = req({ code: CODE, ...shape });
    const captured: RenderError[] = [{ kind: 'worklet', message: 'w' }];
    const f = fakeRuntime();
    const got = await planJob(f.rt, request, captured);
    const want = expected(request, 0.5, 7, staticStructure(parseMeta(CODE).meta), captured);
    expect(got.json).toBe(want.json);
    expect(got.info).toEqual(want.info);
    const c1 = request.toBar + request.overlapBars;
    const windows = f.queries.map((q) => [q.from, q.to]);
    // One query when the chunk window is the schedule's [0, c1); two otherwise.
    expect(windows).toEqual(
      want.info.plan.c0 === 0
        ? [[0, c1]]
        : [
            [0, c1],
            [want.info.plan.c0, c1],
          ],
    );
  });

  test('a later window is queried, not filtered from the schedule: fragments differ', async () => {
    // A hap on bars 0-2 comes back whole from [0, 3) but as a fragment (part from bar 1, no
    // onset) from [1, 3): filtering the schedule's haps would wrongly play it in the chunk.
    const query = (from: number, to: number) =>
      [
        { b: 0, e: 2, note: 40 },
        { b: 2, e: 3, note: 50 },
      ]
        .filter((x) => x.b < to && x.e > from)
        .map((x) => hap(x.b, x.e, { s: 'sine', note: x.note }, { partBegin: Math.max(x.b, from) }));
    const request = req({ code: CODE, fromBar: 1, toBar: 3, lookbackSec: 0 });
    const f = fakeRuntime({ query });
    const got = await planJob(f.rt, request, []);
    expect(f.queries.map((q) => [q.from, q.to])).toEqual([
      [0, 3],
      [1, 3],
    ]);
    const job = JSON.parse(got.json);
    const cycles = job.pauses.flatMap((p: any) => p.haps.map((h: any) => h.cycle));
    expect(cycles).toEqual([2]);
  });

  test('a reused selection still reports its select errors once', async () => {
    const f = fakeRuntime();
    const got = await planJob(f.rt, req({ code: CODE, toBar: 3 }), []);
    expect(f.queries).toHaveLength(1);
    expect(got.info.errors.filter((e) => e.message.includes('hap at cycle 1.25'))).toHaveLength(1);
  });
});

describe('planJob failures', () => {
  test("a meta error is kind 'meta' with the page's message; nothing is evaluated", async () => {
    const code = "const meta = { seed: 'a' };\nnote('c')";
    const metaError = parseMeta(code).error!;
    expect(metaError).not.toBeNull();
    const phases: PlanJobPhase[] = [];
    const f = fakeRuntime();
    const e = await planError(planJob(f.rt, req({ code }), [], (p) => phases.push(p)));
    expect(e.kind).toBe('meta');
    expect(e.message).toBe(
      `meta: ${metaError.message} (${metaError.line}:${metaError.column + 1})`,
    );
    expect(e.message).toMatch(/^meta: .+ \(1:\d+\)$/);
    expect(f.calls).toEqual([]);
    expect(phases).toEqual(['evaluating']);
  });

  test("a failed evaluate is kind 'eval' with the runtime's message", async () => {
    const phases: PlanJobPhase[] = [];
    const f = fakeRuntime({ evaluateResult: false, evalError: 'ReferenceError: x' });
    const e = await planError(planJob(f.rt, req({ code: CODE }), [], (p) => phases.push(p)));
    expect(e.kind).toBe('eval');
    expect(e.message).toBe('ReferenceError: x');
    expect(f.calls).toEqual(['evaluate', 'evalError']);
    expect(phases).toEqual(['evaluating']);
  });

  test("a failed evaluate without a message is 'evaluate failed'", async () => {
    const f = fakeRuntime({ evaluateResult: false, evalError: null });
    const e = await planError(planJob(f.rt, req({ code: CODE }), []));
    expect(e.kind).toBe('eval');
    expect(e.message).toBe('evaluate failed');
  });

  test('an empty eval message is kept, not replaced by the fallback', async () => {
    const f = fakeRuntime({ evaluateResult: false, evalError: '' });
    const e = await planError(planJob(f.rt, req({ code: CODE }), []));
    expect(e.message).toBe('');
  });

  test('an evaluate that throws propagates unchanged (the runtime never throws)', async () => {
    const thrown = new TypeError('adapter broke');
    const f = fakeRuntime({ evaluateThrows: thrown });
    await expect(planJob(f.rt, req({ code: CODE }), [])).rejects.toBe(thrown);
  });

  test("a null query is kind 'plan': 'query before a successful evaluate'", async () => {
    const f = fakeRuntime({ query: () => null });
    const e = await planError(planJob(f.rt, req({ code: CODE }), []));
    expect(e.kind).toBe('plan');
    expect(e.message).toBe('query before a successful evaluate');
    expect(f.queries).toHaveLength(1);
  });

  test('a null chunk query after a good schedule query is the same error', async () => {
    let n = 0;
    const f = fakeRuntime({ query: (a, b) => (n++ === 0 ? songHaps(a, b) : null) });
    const e = await planError(planJob(f.rt, req({ code: CODE, fromBar: 1, toBar: 2 }), []));
    expect(f.queries.map((q) => [q.from, q.to])).toEqual([
      [0, 2],
      [1, 2],
    ]);
    expect(e).toMatchObject({ kind: 'plan', message: 'query before a successful evaluate' });
  });

  test("a pattern error thrown by query is wrapped as kind 'plan'", async () => {
    const f = fakeRuntime({
      query: () => {
        throw new Error('pattern exploded');
      },
    });
    const e = await planError(planJob(f.rt, req({ code: CODE }), []));
    expect(e.kind).toBe('plan');
    expect(e.message).toBe('pattern exploded');
  });

  test('a non-Error thrown value is wrapped with its string form', async () => {
    const f = fakeRuntime({ stateThrows: 'state gone' });
    const e = await planError(planJob(f.rt, req({ code: CODE }), []));
    expect(e).toMatchObject({ kind: 'plan', message: 'state gone' });
  });

  test('a PlanJobError thrown inside is rethrown unchanged', async () => {
    const inner = new PlanJobError('eval', 'inner');
    const f = fakeRuntime({
      query: () => {
        throw inner;
      },
    });
    await expect(planJob(f.rt, req({ code: CODE }), [])).rejects.toBe(inner);
  });

  test.each([
    ['NaN cps', { cps: NaN }, {}, 'invalid cps NaN'],
    ['zero cps', { cps: 0 }, {}, 'invalid cps 0'],
    ['negative cps', { cps: -1 }, {}, 'invalid cps -1'],
    ['negative lookback', {}, { lookbackSec: -1 }, 'invalid lookback -1'],
    ['NaN lookback', {}, { lookbackSec: NaN }, 'invalid lookback NaN'],
    ['empty window', {}, { fromBar: 0, toBar: 0 }, 'empty render window [0, 0)'],
  ] as const)(
    "%s: windowPlan's RangeError is kind 'plan'",
    async (_name, state, request, message) => {
      const f = fakeRuntime({ ...state, query: () => [] });
      const e = await planError(planJob(f.rt, req({ code: CODE, ...request }), []));
      expect(e.kind).toBe('plan');
      expect(e.message).toBe(message);
    },
  );

  test("a huge window's buffer bound is kind 'plan'", async () => {
    const f = fakeRuntime({ query: () => [] });
    const e = await planError(planJob(f.rt, req({ code: CODE, toBar: 1e7 }), []));
    expect(e.kind).toBe('plan');
    expect(e.message).toMatch(/^render buffer too large: \d+ bytes \(max 1000000000\)$/);
  });
});

describe('evaluateSong', () => {
  test('returns the static structure, cps, seed and the layout segments', async () => {
    const f = fakeRuntime({ cps: 0.75, seed: undefined });
    const meta = parseMeta(CODE).meta;
    const linear = await evaluateSong(f.rt, CODE, 'linear');
    expect(linear).toEqual({
      meta,
      structure: staticStructure(meta),
      cps: 0.75,
      seed: 0,
      segments: LINEAR_SEGMENTS,
    });
    const clip = await evaluateSong(fakeRuntime().rt, CODE, 'clip');
    expect(clip.segments).toEqual(clipSegments(staticStructure(meta)));
    expect(f.evaluated[0].opts).toEqual(evaluateOptions(meta));
  });

  test("reports 'evaluating' then 'querying', and a state failure as kind 'plan'", async () => {
    const phases: PlanJobPhase[] = [];
    const f = fakeRuntime({ stateThrows: new Error('state gone') });
    const e = await planError(evaluateSong(f.rt, CODE, 'linear', (p) => phases.push(p)));
    expect(e).toMatchObject({ kind: 'plan', message: 'state gone' });
    expect(phases).toEqual(['evaluating', 'querying']);
  });

  test('querySong turns a null query into the planner error', () => {
    const f = fakeRuntime({ query: () => null });
    expect(() => querySong(f.rt, LINEAR_SEGMENTS)(0, 1)).toThrow(
      new PlanJobError('plan', 'query before a successful evaluate'),
    );
  });
});

// ---- planRender

const SECTIONS_CODE =
  "const meta = { bpm: 120, loops: 2, sections: [{ name: 'intro', bars: 4 }, { name: 'main', bars: 8, loop: true }, { name: 'outro', bars: 4 }] };\nx";
const NO_LOOP_CODE =
  "const meta = { bpm: 120, sections: [{ name: 'a', bars: 3 }, { name: 'b', bars: 5 }] };\nx";

const renderReq = (
  length: LengthRequest,
  o: Partial<RenderPlanRequest> = {},
): RenderPlanRequest => ({
  length,
  mode: 'mix',
  sampleRate: 44100,
  ...o,
});

/** `songHaps` plus a long hap from each bar's last quarter into the bar after next (it crosses
 *  chunk ends) and a second hap sharing each bar's first onset (a sort tie). */
function crossingHaps(from: number, to: number): any[] {
  const out = songHaps(from, to);
  for (let b = Math.floor(from) - 2; b < to; b++) {
    const begin = b + 0.75;
    const end = b + 2.5;
    if (b < 0 || end <= from || begin >= to) continue;
    out.push(
      hap(begin, end, { s: 'triangle', note: 30 + (b % 5) }, { partBegin: Math.max(begin, from) }),
    );
  }
  for (let b = Math.floor(from); b < to; b++) out.push(hap(b, b + 0.25, { s: 'sine', note: 90 }));
  return out;
}

const sectionsMeta = parseMeta(SECTIONS_CODE).meta;
const LONG_SECTION_CODE =
  "const meta = { bpm: 120, sections: [{ name: 'intro', bars: 4 }, { name: 'main', bars: 20, loop: true }, { name: 'outro', bars: 4 }] };\nx";

describe('planRender length resolution', () => {
  test.each([
    ['sections, clip', SECTIONS_CODE, { kind: 'clip' }, { fromBar: 0, toBar: 24, layout: 'clip' }],
    [
      'sections, section',
      SECTIONS_CODE,
      { kind: 'section', name: 'main' },
      { fromBar: 4, toBar: 12, layout: 'linear' },
    ],
    [
      'sections, last section',
      SECTIONS_CODE,
      { kind: 'section', name: 'outro' },
      { fromBar: 12, toBar: 16, layout: 'linear' },
    ],
    [
      'sections, length',
      SECTIONS_CODE,
      { kind: 'length', bars: 10 },
      { fromBar: 0, toBar: 10, layout: 'clip' },
    ],
    [
      'sections, length of the whole clip',
      SECTIONS_CODE,
      { kind: 'length', bars: 24 },
      { fromBar: 0, toBar: 24, layout: 'clip' },
    ],
    [
      'no sections, length',
      CODE,
      { kind: 'length', bars: 10 },
      { fromBar: 0, toBar: 10, layout: 'linear' },
    ],
    [
      'no sections, window',
      CODE,
      { kind: 'window', fromBar: 3, toBar: 7, layout: 'clip' },
      { fromBar: 3, toBar: 7, layout: 'clip' },
    ],
    [
      'sections, window past the clip',
      SECTIONS_CODE,
      { kind: 'window', fromBar: 0, toBar: 40, layout: 'linear' },
      { fromBar: 0, toBar: 40, layout: 'linear' },
    ],
  ] as const)('%s', async (_name, code, length, window) => {
    const f = fakeRuntime();
    const got = await planRender(f.rt, code, renderReq(length as LengthRequest), []);
    expect(got.window).toEqual(window);
    const structure = staticStructure(parseMeta(code).meta);
    const segments = window.layout === 'clip' ? clipSegments(structure) : LINEAR_SEGMENTS;
    for (const q of f.queries) expect(q.segments).toEqual(segments);
    expect(f.queries[0]).toMatchObject({ from: 0, to: window.toBar });
    expect(got.chunks[0]!.plan.fromBar).toBe(window.fromBar);
    expect(got.chunks.at(-1)!.plan.toBar).toBe(window.toBar);
    for (let i = 1; i < got.chunks.length; i++) {
      expect(got.chunks[i]!.plan.fromBar).toBe(got.chunks[i - 1]!.plan.toBar);
    }
    expect(f.evaluated).toHaveLength(1);
    expect(got.structure).toEqual(structure);
    expect(got.cps).toBe(0.5);
  });

  test('loops override moves the clip, its structure and the clip segments', async () => {
    const f = fakeRuntime();
    const got = await planRender(f.rt, SECTIONS_CODE, renderReq({ kind: 'clip', loops: 3 }), []);
    const overridden = buildStructure({ ...sectionsMeta, loops: 3 });
    expect(overridden.clipBars).toBe(32);
    expect(got.window).toEqual({ fromBar: 0, toBar: 32, layout: 'clip' });
    expect(got.structure).toEqual(overridden);
    expect(got.structure.loops).toBe(3);
    expect(clipSegments(overridden)).not.toEqual(clipSegments(staticStructure(sectionsMeta)));
    for (const q of f.queries) expect(q.segments).toEqual(clipSegments(overridden));
  });

  test('loops 1 shortens the clip to one pass of the loop section', async () => {
    const got = await planRender(
      fakeRuntime().rt,
      SECTIONS_CODE,
      renderReq({ kind: 'clip', loops: 1 }),
      [],
    );
    expect(got.window.toBar).toBe(16);
    expect(got.structure.clipBars).toBe(16);
  });

  test("a clip without loops reports the runtime's structure", async () => {
    const structure = buildStructure({ ...sectionsMeta, bpm: 90 });
    const got = await planRender(
      fakeRuntime({ structure }).rt,
      SECTIONS_CODE,
      renderReq({ kind: 'clip' }),
      [],
    );
    expect(got.structure).toBe(structure);
  });

  test('the largest length is planned', async () => {
    const f = fakeRuntime({ query: () => [] });
    const got = await planRender(f.rt, CODE, renderReq({ kind: 'length', bars: 100000 }), []);
    expect(got.window).toEqual({ fromBar: 0, toBar: 100000, layout: 'linear' });
    expect(got.chunks.at(-1)!.plan.toBar).toBe(100000);
  });
});

describe('planRender length errors', () => {
  test.each([
    [
      'clip on a song without sections',
      CODE,
      { kind: 'clip' },
      '--length',
      'this song has no sections; pass --length N to say how many bars to render',
    ],
    [
      'section on a song without sections',
      CODE,
      { kind: 'section', name: 'main' },
      '--section',
      'this song has no sections; --section needs meta.sections',
    ],
    [
      'an unknown section name',
      SECTIONS_CODE,
      { kind: 'section', name: 'bridge' },
      '--section',
      "unknown section 'bridge' (sections: intro, main, outro)",
    ],
    [
      'a length past the clip',
      SECTIONS_CODE,
      { kind: 'length', bars: 25 },
      '--length',
      '--length 25 is past the end of the song (24 bars)',
    ],
    [
      'the largest length past a short clip',
      SECTIONS_CODE,
      { kind: 'length', bars: 100000 },
      '--length',
      '--length 100000 is past the end of the song (24 bars)',
    ],
    [
      'loops on a song without a loop section',
      NO_LOOP_CODE,
      { kind: 'clip', loops: 2 },
      '--loops',
      '--loops needs a section with loop: true in meta.sections',
    ],
    [
      'loops on a song without sections',
      CODE,
      { kind: 'clip', loops: 2 },
      '--loops',
      '--loops needs a section with loop: true in meta.sections',
    ],
    [
      'loops 0',
      SECTIONS_CODE,
      { kind: 'clip', loops: 0 },
      '--loops',
      '--loops must be an integer from 1 to 1000',
    ],
    [
      'loops 1001',
      SECTIONS_CODE,
      { kind: 'clip', loops: 1001 },
      '--loops',
      '--loops must be an integer from 1 to 1000',
    ],
    [
      'fractional loops',
      SECTIONS_CODE,
      { kind: 'clip', loops: 1.5 },
      '--loops',
      '--loops must be an integer from 1 to 1000',
    ],
    [
      'NaN loops',
      SECTIONS_CODE,
      { kind: 'clip', loops: NaN },
      '--loops',
      '--loops must be an integer from 1 to 1000',
    ],
  ] as const)('%s', async (_name, code, length, flag, message) => {
    const f = fakeRuntime();
    const e = await planError(planRender(f.rt, code, renderReq(length as LengthRequest), []));
    expect(e.kind).toBe('meta');
    expect(e.flag).toBe(flag);
    expect(e.message).toBe(message);
    // Length errors come before anything is queried; a loop count out of range before the song
    // is evaluated.
    expect(f.calls.filter((c) => c.startsWith('query'))).toEqual([]);
    if (message.startsWith('--loops must')) expect(f.calls).toEqual([]);
  });

  test('a header error is the plain meta error, without a flag', async () => {
    const code = "const meta = { seed: 'a' };\nnote('c')";
    const metaError = parseMeta(code).error!;
    const f = fakeRuntime();
    const e = await planError(planRender(f.rt, code, renderReq({ kind: 'length', bars: 2 }), []));
    expect(e.kind).toBe('meta');
    expect(e.flag).toBeUndefined();
    expect('flag' in e).toBe(false);
    expect(e.message).toBe(
      `meta: ${metaError.message} (${metaError.line}:${metaError.column + 1})`,
    );
    expect(f.calls).toEqual([]);
  });

  test('a failed evaluate is kind eval, without a flag', async () => {
    const f = fakeRuntime({ evaluateResult: false, evalError: 'ReferenceError: y' });
    const e = await planError(planRender(f.rt, CODE, renderReq({ kind: 'length', bars: 2 }), []));
    expect(e).toMatchObject({ kind: 'eval', message: 'ReferenceError: y' });
    expect(e.flag).toBeUndefined();
  });

  test('a failing query is kind plan', async () => {
    const f = fakeRuntime({ query: () => null });
    const e = await planError(planRender(f.rt, CODE, renderReq({ kind: 'length', bars: 2 }), []));
    expect(e).toMatchObject({ kind: 'plan', message: 'query before a successful evaluate' });
  });

  test('NaN cps is kind plan', async () => {
    const f = fakeRuntime({ cps: NaN, query: () => [] });
    const e = await planError(planRender(f.rt, CODE, renderReq({ kind: 'length', bars: 2 }), []));
    expect(e.kind).toBe('plan');
    expect(e.flag).toBeUndefined();
  });
});

describe('planRender request validation', () => {
  const ok = renderReq({ kind: 'length', bars: 4 });
  test.each([
    ['not an object', null, 'bad request: not a JSON object'],
    ['an array', [], 'bad request: not a JSON object'],
    [
      'stems',
      { ...ok, mode: 'stems' },
      'bad request: mode: stems is not supported yet; v1 renders the mix',
    ],
    ['an unknown mode', { ...ok, mode: 'surround' }, 'bad request: unknown mode surround'],
    ['no mode', { ...ok, mode: undefined }, 'bad request: unknown mode undefined'],
    ['another sample rate', { ...ok, sampleRate: 48000 }, 'bad request: sampleRate must be 44100'],
    [
      'a non-boolean includeEvents',
      { ...ok, includeEvents: 'yes' },
      'bad request: includeEvents must be true or false',
    ],
    ['no length', { ...ok, length: undefined }, 'bad request: length must be an object'],
    [
      'an unknown kind',
      { ...ok, length: { kind: 'bars' } },
      'bad request: unknown length kind bars',
    ],
    [
      'zero bars',
      { ...ok, length: { kind: 'length', bars: 0 } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'NaN bars',
      { ...ok, length: { kind: 'length', bars: NaN } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'negative bars',
      { ...ok, length: { kind: 'length', bars: -4 } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'fractional bars',
      { ...ok, length: { kind: 'length', bars: 2.5 } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'too many bars',
      { ...ok, length: { kind: 'length', bars: 100001 } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'bars as a string',
      { ...ok, length: { kind: 'length', bars: '4' } },
      'bad request: length.bars must be an integer from 1 to 100000',
    ],
    [
      'a section without a name',
      { ...ok, length: { kind: 'section' } },
      'bad request: length.name must be a string',
    ],
    [
      'an empty window',
      { ...ok, length: { kind: 'window', fromBar: 4, toBar: 4, layout: 'linear' } },
      'bad request: length window must be integers with 0 <= fromBar < toBar <= 100000',
    ],
    [
      'a negative window start',
      { ...ok, length: { kind: 'window', fromBar: -1, toBar: 4, layout: 'linear' } },
      'bad request: length window must be integers with 0 <= fromBar < toBar <= 100000',
    ],
    [
      'a window past the limit',
      { ...ok, length: { kind: 'window', fromBar: 0, toBar: 100001, layout: 'linear' } },
      'bad request: length window must be integers with 0 <= fromBar < toBar <= 100000',
    ],
    [
      'a NaN window',
      { ...ok, length: { kind: 'window', fromBar: NaN, toBar: 4, layout: 'linear' } },
      'bad request: length window must be integers with 0 <= fromBar < toBar <= 100000',
    ],
    [
      'a window with an unknown layout',
      { ...ok, length: { kind: 'window', fromBar: 0, toBar: 4, layout: 'loop' } },
      'bad request: unknown layout loop',
    ],
  ] as const)('%s is kind plan without a flag', async (_name, request, message) => {
    const f = fakeRuntime();
    const e = await planError(planRender(f.rt, CODE, request as unknown as RenderPlanRequest, []));
    expect(e.kind).toBe('plan');
    expect(e.flag).toBeUndefined();
    expect(e.message).toBe(message);
    expect(f.calls).toEqual([]);
  });
});

describe('planRender chunks', () => {
  const cases: [string, string, LengthRequest, typeof songHaps][] = [
    ['linear, 20 bars', CODE, { kind: 'length', bars: 20 }, songHaps],
    ['linear, crossing haps', CODE, { kind: 'length', bars: 30 }, crossingHaps],
    ['clip of a sectioned song', SECTIONS_CODE, { kind: 'clip' }, crossingHaps],
    [
      'a window from a later bar',
      CODE,
      { kind: 'window', fromBar: 5, toBar: 27, layout: 'linear' },
      crossingHaps,
    ],
    ['a section in the middle', LONG_SECTION_CODE, { kind: 'section', name: 'main' }, crossingHaps],
  ];

  test.each(cases)(
    "%s: every chunk's job equals planJob of its chunk request",
    async (_name, code, length, haps) => {
      const captured: RenderError[] = [{ kind: 'worklet', message: 'w' }];
      const got = await planRender(
        fakeRuntime({ query: haps }).rt,
        code,
        renderReq(length),
        captured,
      );
      const events = eventsFromValues(
        eventSources(selectHaps(haps(0, got.window.toBar)).selected),
        0.5,
      );
      const plans = renderChunkPlans({
        ...got.window,
        events,
        cps: 0.5,
        overlapBars: 0,
      });
      expect(got.chunks).toHaveLength(plans.length);
      expect(plans.length).toBeGreaterThan(1);
      const base: RenderRequest = {
        code,
        layout: got.window.layout,
        fromBar: got.window.fromBar,
        toBar: got.window.toBar,
        overlapBars: 0,
        lookbackSec: 0,
        mode: 'mix',
        sampleRate: 44100,
      };
      for (const [i, plan] of plans.entries()) {
        const request = chunkRequest(base, plan);
        const want = await planJob(fakeRuntime({ query: haps }).rt, request, captured);
        expect(got.chunks[i]!.json).toBe(want.json);
        expect(got.chunks[i]!.plan).toEqual({
          ...want.info.plan,
          fromBar: plan.contentStartBar,
          toBar: plan.contentEndBar,
          lookbackSec: plan.lookbackSec,
        });
        expect(Object.is(got.chunks[i]!.plan.lookbackSec, request.lookbackSec)).toBe(true);
      }
      expect(got.lookbackSec).toBe(lookbackSeconds(events));
    },
  );

  test.each(cases)(
    '%s: the per-chunk schedule fallback gives the same jobs',
    async (_n, code, length, haps) => {
      const reuse = await planRender(fakeRuntime({ query: haps }).rt, code, renderReq(length), []);
      const f = fakeRuntime({ query: haps });
      const fallback = await planRender(f.rt, code, renderReq(length), [], undefined, false);
      expect(fallback).toEqual(reuse);
      // The fallback queries [0, c1) for every chunk.
      for (const c of reuse.chunks) {
        expect(f.queries.some((q) => q.from === 0 && q.to === c.plan.toBar)).toBe(true);
      }
    },
  );

  test('evaluates once and queries the schedule once, then each later chunk window', async () => {
    const f = fakeRuntime();
    const got = await planRender(f.rt, CODE, renderReq({ kind: 'length', bars: 20 }), []);
    expect(f.calls.filter((c) => c === 'evaluate')).toHaveLength(1);
    expect(f.queries.map((q) => [q.from, q.to])).toEqual([
      [0, 20],
      ...got.chunks.filter((c) => c.plan.c0 > 0).map((c) => [c.plan.c0, c.plan.c1]),
    ]);
    // The first chunk starts at bar 0: it reuses the cut schedule.
    expect(got.chunks[0]!.plan.c0).toBe(0);
  });

  test('chunk plans carry no overlap and the exact lookback of the chunk request', async () => {
    const got = await planRender(
      fakeRuntime().rt,
      CODE,
      renderReq({ kind: 'length', bars: 20 }),
      [],
    );
    for (const c of got.chunks) {
      expect(c.plan.c1).toBe(c.plan.toBar);
      expect(Object.keys(c.plan).sort()).toEqual(
        [
          'bufferStartSec',
          'c0',
          'c1',
          'frames',
          'fromBar',
          'lookbackSec',
          'lookbackSecUsed',
          'toBar',
          'windowStartFrame',
        ].sort(),
      );
    }
    expect(got.chunks[0]!.plan.lookbackSec).toBe(0);
    expect(got.chunks[1]!.plan.lookbackSec).toBeGreaterThan(0);
  });

  test('includeEvents adds the schedule events; without it there is no events key', async () => {
    const on = await planRender(
      fakeRuntime().rt,
      CODE,
      renderReq({ kind: 'length', bars: 6 }, { includeEvents: true }),
      [],
    );
    expect(on.events).toEqual(
      eventsFromValues(eventSources(selectHaps(songHaps(0, 6)).selected), 0.5),
    );
    for (const o of [{}, { includeEvents: false }]) {
      const off = await planRender(
        fakeRuntime().rt,
        CODE,
        renderReq({ kind: 'length', bars: 6 }, o),
        [],
      );
      expect('events' in off).toBe(false);
    }
  });

  test('errors are the chunks’ select and captured errors, deduped', async () => {
    const captured: RenderError[] = [
      { kind: 'worklet', message: 'w' },
      { kind: 'worklet', message: 'w' },
    ];
    const got = await planRender(
      fakeRuntime().rt,
      CODE,
      renderReq({ kind: 'length', bars: 20 }),
      captured,
    );
    expect(got.errors).toHaveLength(2);
    expect(got.errors[0]!.message).toContain('hap at cycle 1.25');
    expect(got.errors[1]).toEqual({ kind: 'worklet', message: 'w' });
  });

  test('onPhase reports evaluating then querying', async () => {
    const phases: PlanJobPhase[] = [];
    await planRender(fakeRuntime().rt, CODE, renderReq({ kind: 'length', bars: 2 }), [], (p) =>
      phases.push(p),
    );
    expect(phases).toEqual(['evaluating', 'querying']);
  });
});

describe('cutSchedule', () => {
  test('keeps the haps with onset before c1, the same objects with their original index', () => {
    const full = selectHaps(crossingHaps(0, 12));
    expect(cutSchedule(full, 13)).toEqual(full);
    for (const c1 of [0, 1, 2, 5, 12]) {
      const cut = cutSchedule(full, c1);
      const own = selectHaps(crossingHaps(0, c1));
      expect(cut.selected.length).toBe(own.selected.length);
      cut.selected.forEach((s, i) => {
        expect(s).toBe(full.selected[i]!);
        expect(s.index).toBe(i);
        expect(s.onsetCycle).toBeLessThan(c1);
        expect(s.index).toBe(own.selected[i]!.index);
        expect(s.onsetCycle).toBe(own.selected[i]!.onsetCycle);
        expect(s.value).toEqual(own.selected[i]!.value);
      });
      expect(cut.errors).toEqual(own.errors);
    }
  });

  test('an index past a dropped hap is not renumbered', () => {
    const full = {
      selected: [
        { hap: null, value: {}, onsetCycle: 0, index: 0 },
        { hap: null, value: {}, onsetCycle: 1.5, index: 3 },
        { hap: null, value: {}, onsetCycle: 2, index: 4 },
      ],
      errors: [
        { kind: 'other' as const, message: 'a', bar: 1 },
        { kind: 'other' as const, message: 'b', bar: 2 },
        { kind: 'other' as const, message: 'c' },
      ],
    };
    const cut = cutSchedule(full, 2);
    expect(cut.selected.map((s) => s.index)).toEqual([0, 3]);
    expect(cut.errors.map((e) => e.message)).toEqual(['a', 'c']);
  });
});

describe('PlanJobError flag', () => {
  test('is set only when given', () => {
    const flagged = new PlanJobError('meta', 'm', '--length');
    expect(flagged.flag).toBe('--length');
    expect(flagged.kind).toBe('meta');
    const plain = new PlanJobError('meta', 'm');
    expect('flag' in plain).toBe(false);
  });
});

describe('planRender follows the evaluated structure', () => {
  // The header says intro 4, main 8 (loop) x 2, outro 4: 24 bars. The song's `song()` call
  // registers another structure: main is 20 bars and played 3 times.
  const evaluated = buildStructure({
    bpm: 120,
    loops: 3,
    sections: [
      { name: 'intro', bars: 4 },
      { name: 'main', bars: 20, loop: true },
      { name: 'outro', bars: 4 },
      { name: 'coda', bars: 2 },
    ],
  });

  test('the default clip is the evaluated clip, on its segments', async () => {
    const f = fakeRuntime({ structure: evaluated });
    const got = await planRender(f.rt, SECTIONS_CODE, renderReq({ kind: 'clip' }), []);
    expect(evaluated.clipBars).toBe(70);
    expect(got.window).toEqual({ fromBar: 0, toBar: 70, layout: 'clip' });
    expect(got.structure).toBe(evaluated);
    for (const q of f.queries) expect(q.segments).toEqual(clipSegments(evaluated));
  });

  test('--loops replaces the evaluated loop count, not the header one', async () => {
    const f = fakeRuntime({ structure: evaluated });
    const got = await planRender(f.rt, SECTIONS_CODE, renderReq({ kind: 'clip', loops: 2 }), []);
    const want = withLoops(evaluated, 2);
    expect(want.clipBars).toBe(50);
    expect(got.window).toEqual({ fromBar: 0, toBar: 50, layout: 'clip' });
    expect(got.structure).toEqual(want);
    for (const q of f.queries) expect(q.segments).toEqual(clipSegments(want));
  });

  test('sections, lengths and their errors read the evaluated sections', async () => {
    const section = await planRender(
      fakeRuntime({ structure: evaluated }).rt,
      SECTIONS_CODE,
      renderReq({ kind: 'section', name: 'coda' }),
      [],
    );
    expect(section.window).toEqual({ fromBar: 28, toBar: 30, layout: 'linear' });
    const length = await planRender(
      fakeRuntime({ structure: evaluated }).rt,
      SECTIONS_CODE,
      renderReq({ kind: 'length', bars: 60 }),
      [],
    );
    expect(length.window).toEqual({ fromBar: 0, toBar: 60, layout: 'clip' });
    const e = await planError(
      planRender(
        fakeRuntime({ structure: evaluated }).rt,
        SECTIONS_CODE,
        renderReq({ kind: 'length', bars: 71 }),
        [],
      ),
    );
    expect(e).toMatchObject({
      kind: 'meta',
      flag: '--length',
      message: '--length 71 is past the end of the song (70 bars)',
    });
    // A header without sections whose song registers some has a clip.
    const clip = await planRender(
      fakeRuntime({ structure: evaluated }).rt,
      CODE,
      renderReq({ kind: 'clip' }),
      [],
    );
    expect(clip.window.toBar).toBe(70);
  });
});

describe('planRender render limit', () => {
  /** A header of `n` sections of `bars` bars, the second one looping. */
  const longCode = (n: number, bars: number, loops?: number): string => {
    const sections = Array.from(
      { length: n },
      (_, i) => `{ name: 's${i}', bars: ${bars}${i === 1 ? ', loop: true' : ''} }`,
    );
    const l = loops === undefined ? '' : `loops: ${loops}, `;
    return `const meta = { bpm: 120, ${l}sections: [${sections.join(', ')}] };\nx`;
  };

  test.each([
    [
      'a clip of long sections',
      longCode(11, 10000),
      { kind: 'clip' },
      '--length',
      "the song's clip is 110000 bars, past the render limit of 100000 bars; pass --length N or --loops N",
    ],
    [
      'a clip of meta.loops passes',
      longCode(2, 10000, 10),
      { kind: 'clip' },
      '--length',
      "the song's clip is 110000 bars, past the render limit of 100000 bars; pass --length N or --loops N",
    ],
    [
      'a clip of --loops passes',
      longCode(2, 10000),
      { kind: 'clip', loops: 1000 },
      '--loops',
      '--loops 1000 makes the clip 10010000 bars, past the render limit of 100000 bars',
    ],
    [
      'a section ending past the limit',
      longCode(11, 10000),
      { kind: 'section', name: 's10' },
      '--section',
      "section 's10' ends at bar 110000, past the render limit of 100000 bars",
    ],
  ] as const)('%s is a flagged meta error', async (_name, code, length, flag, message) => {
    const f = fakeRuntime({ query: () => [] });
    const e = await planError(planRender(f.rt, code, renderReq(length as LengthRequest), []));
    expect(e).toMatchObject({ kind: 'meta', flag, message });
    expect(f.calls.filter((c) => c.startsWith('query'))).toEqual([]);
  });

  test('a clip without a loop section names only --length', async () => {
    const code =
      'const meta = { bpm: 120, sections: [' +
      Array.from({ length: 11 }, (_, i) => `{ name: 's${i}', bars: 10000 }`).join(', ') +
      '] };\nx';
    const e = await planError(planRender(fakeRuntime().rt, code, renderReq({ kind: 'clip' }), []));
    expect(e.message).toBe(
      "the song's clip is 110000 bars, past the render limit of 100000 bars; pass --length N",
    );
  });

  test('a clip and a section ending at the limit are planned', async () => {
    const f = fakeRuntime({ query: () => [] });
    const got = await planRender(f.rt, longCode(10, 10000), renderReq({ kind: 'clip' }), []);
    expect(got.window.toBar).toBe(100000);
    const section = await planRender(
      fakeRuntime({ query: () => [] }).rt,
      longCode(10, 10000),
      renderReq({ kind: 'section', name: 's9' }),
      [],
    );
    expect(section.window).toEqual({ fromBar: 90000, toBar: 100000, layout: 'linear' });
  });
});

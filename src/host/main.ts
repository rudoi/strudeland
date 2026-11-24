/**
 * The headless host's entry: `globalThis.strudelandHost = { planJob, planRender, query }`,
 * bundled into `crates/strudeland-host/js/host.js` by `bun run host:build` and evaluated in an
 * embedded JS runtime with no DOM. The functions return at once and report through the result
 * slot (`globalThis.__strudelandResult`; for `planJob` the job text in
 * `globalThis.__strudelandJob` and for `planRender` the plan's ASCII JSON in
 * `globalThis.__strudelandRender`, each set first); the embedder drains microtasks and reads them. Each
 * call creates its own runtime: the embedder gives every request a fresh context, so nothing
 * here is cached or reset.
 */

import type { Readable } from 'svelte/store';
import { errorMessage } from '../lib/errors';
import type { RenderError, RenderRequest, SegmentLayout } from '../lib/render/contract';
import { loggedError, loggerMessage } from '../lib/render/logged-errors';
import {
  evaluateSong,
  planJob,
  PlanJobError,
  planRender,
  querySong,
  type PlannerRuntime,
  type RenderPlanRequest,
} from '../lib/render/job-plan';
import { asciiJson } from '../lib/render/page-job';
import { classifyError, eventSources, selectHaps } from '../lib/render/plan';
import { eventsFromValues } from '../lib/render/schedule';
import type { InstrumentSnapshot } from '../lib/strudel/instruments';
import { createRuntime } from '../lib/strudel/runtime';
import { createHostBackend } from './backend';
import { hostPrebake } from './prebake';

export type HostResult =
  | { ok: true; value: string }
  | { ok: false; kind: 'meta' | 'eval' | 'plan'; error: string; flag?: string };

export type Layout = SegmentLayout;

export interface StrudelandHost {
  planJob(code: string, requestJson: string): void;
  planRender(code: string, requestJson: string): void;
  query(code: string, from: number, to: number, layout: Layout): void;
}

/** The widest `query` window, in layout cycles. */
export const MAX_QUERY_CYCLES = 4096;

const g = globalThis as any;

/** A store's current value (`svelte/store`'s `get`, without bundling svelte). */
export function current<T>(store: Readable<T>): T {
  let value!: T;
  store.subscribe((v) => (value = v))();
  return value;
}

/** A phase mark for `strudeland-host bench --profile`: the embedder's `__strudelandMark` reads a
 *  monotonic clock on its side and returns nothing; it is a no-op unless profiling, and absent
 *  outside the host. */
const mark = (label: string): void => {
  if (typeof g.__strudelandMark === 'function') g.__strudelandMark(label);
};

const setResult = (r: HostResult): void => {
  g.__strudelandResult = r;
  mark('settled');
};

const failure = (e: unknown): HostResult =>
  e instanceof PlanJobError
    ? {
        ok: false,
        kind: e.kind,
        error: e.message,
        ...(e.flag === undefined ? {} : { flag: e.flag }),
      }
    : { ok: false, kind: 'plan', error: errorMessage(e) };

/** A runtime that never schedules: inert backend and timers. */
function hostRuntime() {
  return createRuntime({
    backends: { superdough: createHostBackend() },
    prebake: hostPrebake,
    setInterval: () => 0,
    clearInterval: () => {},
  });
}

/** The host adapter (`createRuntime` result to `PlannerRuntime`), the page adapter's four
 *  members. `evaluate` never rejects: a rejection (a failed prebake) becomes a failed evaluate
 *  whose message `evalError` reports, since `planJob` does not wrap `evaluate`. */
export function hostAdapter(
  runtime: ReturnType<typeof createRuntime>,
  instruments?: InstrumentSnapshot,
): PlannerRuntime {
  let rejected: string | null = null;
  return {
    evaluate: async (code, opts) => {
      rejected = null;
      try {
        return await runtime.evaluate(code, { ...opts, instruments });
      } catch (e) {
        rejected = errorMessage(e);
        return false;
      }
    },
    evalError: () => rejected ?? current(runtime.store).evalError,
    state: () => {
      const s = current(runtime.store);
      return { structure: s.structure, cps: s.cps, seed: s.seed };
    },
    query: (from, to, segments) => runtime.query(from, to, segments),
  };
}

/** Replaces `console.error`/`console.warn` with versions that push onto `captured` and then
 *  call the originals, and `console.log` with one that pushes the failures core's `logger`
 *  reports (`loggedError`, the render page's rule for its `strudel.log` events) and then calls
 *  the original; any other `console.log` is only forwarded. Returns the restore. */
export function captureConsole(captured: RenderError[]): () => void {
  const { error, warn, log } = console;
  const push = (args: unknown[]) => captured.push(classifyError(args.map(errorMessage).join(' ')));
  console.error = (...args: unknown[]) => {
    push(args);
    error.apply(console, args);
  };
  console.warn = (...args: unknown[]) => {
    push(args);
    warn.apply(console, args);
  };
  console.log = (...args: unknown[]) => {
    const message = loggerMessage(args);
    const err = message === null ? null : loggedError(message, undefined);
    if (err) captured.push(err);
    log.apply(console, args);
  };
  return () => {
    console.error = error;
    console.warn = warn;
    console.log = log;
  };
}

/** Runs `body` and stores its settled result; never throws synchronously. */
function settle(body: () => Promise<string>): void {
  g.__strudelandResult = undefined;
  try {
    body().then(
      (value) => setResult({ ok: true, value }),
      (e) => setResult(failure(e)),
    );
  } catch (e) {
    setResult(failure(e));
  }
}

/** `requestJson` parsed; not JSON or not an object is a `bad request` plan error. */
function parseRequest(requestJson: string): object {
  let parsed: unknown;
  try {
    parsed = JSON.parse(requestJson);
  } catch (e) {
    throw new PlanJobError('plan', `bad request: ${errorMessage(e)}`);
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new PlanJobError('plan', 'bad request: not a JSON object');
  }
  return parsed;
}

async function runPlanJob(code: string, requestJson: string): Promise<string> {
  const request = { ...parseRequest(requestJson), code } as RenderRequest;
  const captured: RenderError[] = [];
  const restore = captureConsole(captured);
  try {
    const { info, json } = await planJob(hostAdapter(hostRuntime()), request, captured, mark);
    g.__strudelandJob = json;
    return JSON.stringify(info);
  } finally {
    restore();
  }
}

/** Plans every chunk (`planRender` validates the request's fields) and stores the plan as one
 *  ASCII JSON text in `__strudelandRender`; the slot's value is unused. */
async function runPlanRender(code: string, requestJson: string): Promise<string> {
  const request = parseRequest(requestJson) as RenderPlanRequest;
  const captured: RenderError[] = [];
  const restore = captureConsole(captured);
  try {
    const planned = await planRender(
      hostAdapter(hostRuntime(), request.instruments),
      code,
      request,
      captured,
      mark,
    );
    g.__strudelandRender = asciiJson(planned);
    return 'ok';
  } finally {
    restore();
  }
}

/** True when `[from, to)` is a window `query` accepts. */
export function isQueryWindow(from: unknown, to: unknown): boolean {
  return (
    typeof from === 'number' &&
    typeof to === 'number' &&
    Number.isFinite(from) &&
    Number.isFinite(to) &&
    from < to &&
    to - from <= MAX_QUERY_CYCLES
  );
}

async function runQuery(code: string, from: number, to: number, layout: Layout): Promise<string> {
  if (!isQueryWindow(from, to)) throw new PlanJobError('plan', 'bad window');
  if (layout !== 'clip' && layout !== 'linear') {
    throw new PlanJobError('plan', `bad request: unknown layout ${String(layout)}`);
  }
  // No console capture: `query`'s output is the events only, captured and select errors are not
  // part of it.
  const rt = hostAdapter(hostRuntime());
  const { cps, segments, timing } = await evaluateSong(rt, code, layout);
  // A failure here that is not a PlanJobError settles as kind 'plan' (`failure`).
  const { selected } = selectHaps(querySong(rt, segments)(from, to));
  return JSON.stringify(eventsFromValues(eventSources(selected), cps, timing));
}

export const strudelandHost: StrudelandHost = {
  planJob(code, requestJson) {
    settle(() => runPlanJob(code, requestJson));
  },
  planRender(code, requestJson) {
    settle(() => runPlanRender(code, requestJson));
  },
  query(code, from, to, layout) {
    settle(() => runQuery(code, from, to, layout));
  },
};

g.strudelandHost = strudelandHost;

/**
 * A hand-rolled `Runtime` stub (no Strudel, no audio) recording what the session asks of it. Its
 * transport is the runtime's own model (`createTransportModel`) over a plain segment list, with a
 * horizon and an audible cycle the test sets.
 * `live: true` makes a successful evaluate start it and apply its tempo and `activeCode`, and a
 * failed one set `evalError`; otherwise evaluate leaves the store alone and tests drive it
 * directly. `stop` always stops, as the runtime's does.
 *
 * As in the runtime, the transport model's `started` notifies on every toggle, also one that
 * repeats the value (each stop rewinds the transport), and never on other store changes.
 */
import type { LevelHoldSnapshot } from '../../src/lib/levels/levels';
import { get, writable, type Readable } from 'svelte/store';
import type {
  EvaluateOptions,
  Runtime,
  RuntimeState,
  StartOptions,
} from '../../src/lib/strudel/runtime';
import { tempoAtBar } from '../../src/lib/strudel/structure';
import {
  createTransportModel,
  INITIAL_SEGMENTS,
  prune,
  type Located,
  type Segment,
} from '../../src/lib/strudel/transport';

/** What `query` answers: fixed haps (null: nothing evaluated) or a function that may throw. */
type QueryAnswer = any[] | null | ((from: number, to: number) => any[] | null);

export function stubRuntime(opts: { audio?: RuntimeState['audio']; live?: boolean } = {}) {
  const store = writable<RuntimeState>({
    started: false,
    cps: 0.5,
    meter: { num: 4, den: 4 },
    structure: null,
    tempoOverride: false,
    seed: undefined,
    evalError: null,
    outputError: null,
    audio: opts.audio ?? 'suspended',
    backend: 'fake',
    engine: 'superdough',
    live: null,
    engineWarnings: [],
    activeCode: null,
    limiter: false,
    limiterAvailable: true,
  });
  const evaluated: string[] = [];
  const limiterCalls: boolean[] = [];
  const options: (EvaluateOptions | undefined)[] = [];
  let levelSnap: LevelHoldSnapshot | null = null;
  let result = true;
  let hold: Promise<void> | undefined;
  let stops = 0;
  let answer: QueryAnswer = null;
  const queries: [from: number, to: number][] = [];
  let pos: { cycle: number; located: Located | null } = { cycle: 0, located: null };
  const starts: (StartOptions | undefined)[] = [];
  let startResult: () => boolean | Promise<boolean> = () => true;
  let cycleTimeOf: (cycle: number) => number | null = () => null;
  const cycleTimes: number[] = [];
  let segments: readonly Segment[] = INITIAL_SEGMENTS;
  let horizon = 0;
  const startedRuns = new Set<(started: boolean) => void>();
  const started: Readable<boolean> = {
    subscribe(run) {
      startedRuns.add(run);
      run(get(store).started);
      return () => startedRuns.delete(run);
    },
  };
  /** The runtime's `onToggle`: the store first, then the transport model. */
  const toggle = (p: Partial<RuntimeState> & { started: boolean }) => {
    store.update((s) => ({ ...s, ...p }));
    startedRuns.forEach((fn) => fn(p.started));
  };
  const model = createTransportModel({
    started,
    structure: () => get(store).structure,
    boundary: {
      horizon: () => horizon,
      segments: () => segments,
      install: (next) => void (segments = next),
      // No look-behind: nothing in the stub queries behind the playhead.
      prune: (cycle) => void (segments = prune(segments, cycle)),
    },
    now: () => pos.cycle,
  });
  const runtime: Runtime = {
    scheduler: {},
    drawingScheduler: {},
    store: { subscribe: store.subscribe },
    async evaluate(code, o) {
      evaluated.push(code);
      options.push(o);
      await hold;
      if (opts.live) {
        if (result) {
          const s = get(store);
          // `autostart: false` installs without starting (and leaves a playing runtime playing).
          const next = o?.autostart === false ? s.started : s.audio === 'running';
          store.update((s) => ({
            ...s,
            evalError: null,
            cps: o?.cps ?? s.cps,
            meter: o?.meter ?? s.meter,
            seed: o?.seed,
            activeCode: code,
          }));
          // An autostart toggles only a stopped scheduler.
          if (next !== s.started) toggle({ started: next });
        } else {
          store.update((s) => ({ ...s, evalError: 'boom is not defined' }));
        }
      }
      return result;
    },
    stop() {
      stops++;
      toggle({ started: false });
    },
    resumeAudio: async () => true,
    canAudition: () => !get(store).started,
    prepareAudition: async () => true,
    async start(o) {
      starts.push(o);
      const ok = await startResult();
      if (ok && opts.live) toggle({ started: true });
      return ok;
    },
    cycleTime(cycle) {
      cycleTimes.push(cycle);
      return cycleTimeOf(cycle);
    },
    transport: model,
    position: () => pos,
    tempoNow: () => {
      const s = get(store);
      const bar = s.started && s.structure?.meterChanges ? (pos.located?.songBar ?? null) : null;
      return tempoAtBar(s.cps, s.meter, s.structure, bar);
    },
    query(from, to) {
      queries.push([from, to]);
      return typeof answer === 'function' ? answer(from, to) : answer;
    },
    readScope: () => false,
    drainLevels: () => null,
    levels: () => levelSnap,
    setLimiter: (on) => {
      limiterCalls.push(on);
      store.update((s) => ({ ...s, limiter: on }));
    },
  };
  return {
    runtime,
    evaluated,
    options,
    /** What the next evaluates resolve to. */
    setResult: (ok: boolean) => void (result = ok),
    /** Evaluates wait for this before resolving. */
    hold: (p: Promise<void> | undefined) => void (hold = p),
    stops: () => stops,
    /** What `runtime.levels()` returns next. */
    setLevels: (l: LevelHoldSnapshot | null) => void (levelSnap = l),
    /** Every `setLimiter(on)` call, in order. */
    limiterCalls: () => limiterCalls,
    /** The scheduler toggles (notified even when `started` repeats). */
    setStarted: (started: boolean) => toggle({ started }),
    /** Change runtime store fields (evalError, audio, …); `started` among them toggles. */
    patch: (p: Partial<RuntimeState>) =>
      'started' in p && p.started !== undefined
        ? toggle({ ...p, started: p.started })
        : store.update((s) => ({ ...s, ...p })),
    /** What `query` answers from now on (null by default: nothing evaluated). */
    setQuery: (a: QueryAnswer) => void (answer = a),
    /** Every `query(from, to)` call, in order. */
    queries: () => queries,
    /** The transport's live segment list. */
    segments: () => segments,
    /** The scheduler's query horizon (a layout cycle) from now on: 0 by default. */
    setHorizon: (cycle: number) => void (horizon = cycle),
    /** What `position()` returns from now on; its cycle is the transport's audible cycle. */
    setPosition: (p: { cycle: number; located: Located | null }) => void (pos = p),
    /** Every `start(opts)` call's options, in order. */
    starts: () => starts,
    /** What `start` resolves to from now on (true by default; `live` also sets `started`). */
    setStart: (fn: () => boolean | Promise<boolean>) => void (startResult = fn),
    /** Every `cycleTime(cycle)` call's cycle, in order. */
    cycleTimes: () => cycleTimes,
    /** What `cycleTime` answers from now on (null by default). */
    setCycleTime: (fn: (cycle: number) => number | null) => void (cycleTimeOf = fn),
  };
}

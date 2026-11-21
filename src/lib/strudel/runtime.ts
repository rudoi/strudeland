import { getRNGMode, repl, State, TimeSpan, useRNG } from '@strudel/core';
import { transpiler } from '@strudel/transpiler';
import type { AudioBackend } from '../audio/backend';
import type { LiveStats } from '../audio/playback-api';
import type { EngineWarning } from '../audio/backend';
import {
  createLevelHold,
  mergeFrames,
  type LevelFrame,
  type LevelHoldSnapshot,
} from '../levels/levels';
import type { Readable } from 'svelte/store';
import type { Hap, ScheduleRoute } from './types';
import { applySeed } from './seed';
import { beginEvaluate, registeredStructure, setSegmentSource } from './song';
import { cpsFor, DEFAULT_BPM, DEFAULT_METER, type Meter, type SongStructure } from './structure';
import {
  canPlayEnding,
  inCountIn,
  INITIAL_SEGMENTS,
  LINEAR_SEGMENTS,
  locate,
  nextSafeBoundary,
  prune,
  withLoop,
  withRelease,
  withCountIn,
  withStartFrom,
  restartAt,
  type Located,
  type Segment,
  type Transport,
  type TransportState,
  type View,
} from './transport';
import { errorMessage } from '../errors';

/** Tempo for one evaluate. Omitted values use the defaults (120 quarter-note bpm, 4/4). */
export interface EvaluateOptions {
  cps?: number;
  meter?: Meter;
  /** Seeds the whole pattern (an inner `.seed()` still wins); omitted: unseeded. */
  seed?: number;
  /**
   * Default true. False installs the pattern without starting the scheduler (`start()` starts it
   * later). While playing it changes nothing: an evaluate always replaces the live pattern.
   */
  autostart?: boolean;
  /**
   * The live engine the song names (`meta.engine`; default superdough). Stopped, the runtime
   * switches at once; playing, at the next bar line, continuing from that bar. Without a rust
   * backend (a plain browser, the host, the render page) `rust` means superdough.
   */
  engine?: EngineId;
}

/** The live engines: the keys of `RuntimeOptions.backends`. */
export type EngineId = 'superdough' | 'rust';

/** Options for `Runtime.start`. */
export interface StartOptions {
  /**
   * One bar of silence at global cycle 0 before the song (or the armed section) starts, so bar 1
   * is global cycle 1. Code without `song()` is delayed by one cycle instead.
   */
  countIn?: boolean;
  /**
   * The engine for this start only; it wins over the song's `engine`. The next stop returns to the
   * song's. Recording takes pass `'superdough'`. Without a rust backend `rust` means superdough.
   */
  backend?: EngineId;
}

/** cps a code `setcps` must differ by to count as overriding meta. */
const TEMPO_EPSILON = 1e-9;
/**
 * How long a replaced segment is kept after it stops governing, in cycles. Painters and the
 * highlighter query a look-behind window, so the segments that governed it must still be there.
 * Well above the drawer's look-behind (`PAINTER_DRAW_TIME` reaches 2 cycles back) and a few
 * cycles of a painter's own window.
 */
export const HISTORY_CYCLES = 16;
/** How often the runtime checks whether the song has ended. */
const AUTO_STOP_MS = 100;
/**
 * An engine left by a switch while playing keeps its session until the haps it was sent have
 * played: until the last one ends, plus this for its release, at most `DRAIN_MAX_SEC` after.
 */
export const DRAIN_RELEASE_SEC = 1;
export const DRAIN_MAX_SEC = 16;
/**
 * The scheduler's fixed hap lead (`cyclist.latency`) in seconds, the same on both engines. The
 * Rust engine's share of it covers the invoke and the engine's render-ahead; raise only by the
 * latency gate.
 */
export const HAP_LATENCY_SEC = 0.1;

export interface RuntimeState {
  started: boolean;
  cps: number;
  /** The meter of the last successful evaluate (4/4 until then). */
  meter: Meter;
  /** What the last successful evaluate's `song()` calls registered; null without `song()`. */
  structure: SongStructure | null;
  /** True when the last successful evaluate's code changed cps away from meta's. */
  tempoOverride: boolean;
  /** The last successful evaluate's seed; undefined without one. */
  seed: number | undefined;
  evalError: string | null;
  outputError: string | null;
  audio: 'running' | 'suspended';
  backend: string;
  /** The active engine: the key of the backend haps go to. */
  engine: EngineId;
  /** The Rust engine's latest stats while it is active and has reported; else null. */
  live: LiveStats | null;
  /** Sounds the Rust engine skipped this play, one per signature; cleared at each start. */
  engineWarnings: EngineWarning[];
  activeCode: string | null;
  /** The master limiter preference (applied to both backends). */
  limiter: boolean;
  /** Whether the active backend can limit (superdough: once its master worklet is attached). */
  limiterAvailable: boolean;
}

export interface RuntimeHooks {
  beforeEval?: () => void | Promise<void>;
  afterEval?: (r: { code: string; pattern: any; meta: any }) => void;
  onToggle?: (started: boolean) => void;
  /** Called synchronously when evaluation, playback or stop is requested. */
  onPlaybackRequest?: () => void;
  /** Called for every scheduled hap, before it is routed. Synchronous; a throw is logged only. */
  onSchedule?: (hap: Hap, t: number, cps: number, route: ScheduleRoute) => void;
}

export interface Runtime {
  readonly scheduler: any;
  readonly store: Readable<RuntimeState>;
  /**
   * Resolves true iff the code evaluated without error. Never throws. Sets cps from `opts` (or the
   * default) together with the new pattern (while stopped, before the code runs), unless the code
   * calls `setcps`/`setcpm`, which wins. A failed evaluate leaves cps as it was. Calls run one at
   * a time.
   */
  evaluate(code: string, opts?: EvaluateOptions): Promise<boolean>;
  stop(): void;
  /**
   * Starts the scheduler on the last successfully evaluated pattern, from cycle 0. Call from a
   * user gesture: it resumes audio synchronously first. Resolves false, changing nothing, unless a
   * successful evaluate has run, no evaluate is running, the scheduler is stopped and audio runs;
   * a failure restores what it touched and resolves false. Never throws.
   */
  start(opts?: StartOptions): Promise<boolean>;
  /**
   * The context time (`backend.getTime()`'s clock, latency included) at which global cycle `cycle`
   * sounds at the current cps, or null unless the scheduler runs with a fresh checkpoint (a cps
   * change makes it stale until the next tick).
   */
  cycleTime(cycle: number): number | null;
  /** Call from a user gesture. Resolves true iff audio is running. Never throws. */
  resumeAudio(): Promise<boolean>;
  /** Whether a stopped editor may audition a note on its own audio graph. */
  canAudition(): boolean;
  /** Prepares the preview graph and shared synth registry. Call from a user gesture. */
  prepareAudition(): Promise<boolean>;
  /** Start-from, loop and release for `song()`; reset to the start on every stop. */
  readonly transport: Transport;
  /** The scheduler's cycle (clamped >= 0) and where the evaluated song is at it. */
  position(): { cycle: number; located: Located | null };
  /**
   * The evaluated pattern's haps over cycles `[fromCycle, toCycle)` of the linear song timeline
   * (`LINEAR_SEGMENTS`: cycle N is song bar N, independent of loop, armed section and playback),
   * or null before the first successful evaluate. Pattern errors throw. `segments` defaults to
   * the linear layout; pass `clipSegments(structure)` for the clip layout.
   */
  query(fromCycle: number, toCycle: number, segments?: readonly Segment[]): any[] | null;
  /** Fills `buf` with the latest master output samples; false when the backend has none. */
  readScope(buf: Float32Array): boolean;
  /**
   * The master level frames that arrived since the last call, merged (`mergeFrames`), or null
   * when none did. Meant for one reader (the meter).
   */
  drainLevels(): LevelFrame | null;
  /**
   * What the master levels held since the last play start or successful evaluate (a stop keeps
   * them), or null until a frame arrives.
   */
  levels(): LevelHoldSnapshot | null;
  /** Sets the limiter preference and applies it to both backends. */
  setLimiter(on: boolean): void;
}

export interface RuntimeOptions {
  /** The engines; superdough is active at first. Without `rust`, every song plays on superdough. */
  backends: { superdough: AudioBackend; rust?: AudioBackend };
  prebake: () => Promise<void>;
  hooks?: RuntimeHooks;
  /**
   * How long to wait for backend.resume() before treating audio as suspended, for a backend that
   * declares no `resumeTimeoutMs` of its own. Default 1000.
   */
  resumeTimeoutMs?: number;
  /** Test seams for the scheduler clock (also used for the auto-stop check). */
  setInterval?: (fn: () => void, ms: number) => unknown;
  clearInterval?: (id: unknown) => void;
  /** The initial limiter preference (default off). */
  limiter?: boolean;
}

const isInitial = (seg: readonly Segment[]): boolean =>
  seg.length === 1 && seg[0]!.at === 0 && seg[0]!.songBar === 0 && seg[0]!.view.kind === 'song';

const sameView = (a: View, b: View): boolean =>
  a.kind === b.kind && (a.kind !== 'loop' || a.section === (b as typeof a).section);

export function createRuntime(opts: RuntimeOptions): Runtime {
  const { backends, hooks } = opts;
  /** The backend haps, the clock, the seed and the scope go to. */
  let activeId: EngineId = 'superdough';
  const backendOf = (id: EngineId): AudioBackend | undefined => backends[id];
  /** `id`, or superdough when there is no backend for it: without rust every song plays there. */
  const resolveEngine = (id: EngineId): EngineId => (backendOf(id) ? id : 'superdough');
  // `swapTo` only makes an engine active when its backend exists.
  const active = (): AudioBackend => backendOf(activeId)!;
  /** The engine the last (successful or running) evaluate's song names. */
  let desired: EngineId = 'superdough';
  /** Numbers each evaluate; `desiredBy` is the one that last set `desired`. */
  let evaluates = 0;
  let desiredBy = 0;
  /** A switch asked for while playing: it happens once the playhead reaches `atCycle`. */
  let pendingSwap: { to: EngineId; atCycle: number } | null = null;
  const defaultCps = cpsFor(DEFAULT_BPM, DEFAULT_METER);
  let state: RuntimeState = {
    started: false,
    cps: defaultCps,
    meter: DEFAULT_METER,
    structure: null,
    tempoOverride: false,
    seed: undefined,
    evalError: null,
    outputError: null,
    audio: 'suspended', // the context does not exist until the first gesture
    backend: active().id,
    engine: 'superdough',
    live: null,
    engineWarnings: [],
    activeCode: null,
    limiter: opts.limiter ?? false,
    limiterAvailable: true,
  };
  const levelHold = createLevelHold();
  /** Frames merged since the last `drainLevels`. */
  let pendingLevels: LevelFrame | null = null;
  /** A new play or evaluate: frames merged before it (while stopped, the meter does not drain)
   *  must not reach the meter or the state. */
  const restartLevels = () => {
    levelHold.reset();
    pendingLevels = null;
  };
  const subscribers = new Set<(s: RuntimeState) => void>();
  const patch = (p: Partial<RuntimeState>) => {
    const keys = Object.keys(p) as (keyof RuntimeState)[];
    if (keys.every((k) => state[k] === p[k])) return; // nothing changed: don't wake subscribers
    state = { ...state, ...p };
    subscribers.forEach((fn) => fn(state));
  };

  let prebaked: Promise<void> | undefined;
  // A failed prebake is not cached, so the next evaluate retries it.
  const ensurePrebaked = () =>
    (prebaked ??= opts.prebake().catch((e) => {
      prebaked = undefined;
      throw e;
    }));

  /** Per engine: when (on its clock) the last hap it was sent ends; reset when it becomes active. */
  const soundUntil: Record<EngineId, number> = { superdough: 0, rust: 0 };

  const output: AudioBackend['output'] = async (hap, t, cps) => {
    const end = t + Number((hap as any).duration) / cps;
    if (end > soundUntil[activeId]) soundUntil[activeId] = end;
    try {
      await active().output(hap, t, cps);
    } catch (e) {
      // A failing sound fails on every hap; report each distinct message once.
      if (state.outputError === errorMessage(e)) return;
      console.error(e);
      patch({ outputError: errorMessage(e) });
    }
  };

  /**
   * The tempo of the evaluate in progress. `repl()`'s hooks are created once, so they read it from
   * here; evaluates are serialized (`queue`), so no other call can change it meanwhile.
   */
  let pending: { cps: number; meter: Meter; seed: number | undefined } = {
    cps: defaultCps,
    meter: DEFAULT_METER,
    seed: undefined,
  };
  let evaluating = false;
  /** True while an evaluate's code runs (from the end of beforeEval to afterEval/onEvalError). */
  let codeRunning = false;
  /** Whether the running evaluate's code called setcps/setcpm. */
  let codeSetCps = false;

  // ---- transport ----

  /** The one mutable segment list; `song()` reads it at every query. */
  let segments: readonly Segment[] = INITIAL_SEGMENTS;
  setSegmentSource(() => segments);
  let transportState: TransportState = {
    view: { kind: 'song' },
    pendingStart: false,
    armedSection: null,
  };
  const transportSubscribers = new Set<(s: TransportState) => void>();

  /** `structure` is the one the change was made against; it names the armed section. */
  function setSegments(
    next: readonly Segment[],
    started = state.started,
    structure: SongStructure | null = null,
  ): void {
    const last = next[next.length - 1]!;
    const view = last.view;
    // Only `start()` installs a count-in, always followed by the real segments.
    if (view.kind === 'count-in') throw new Error('transport: a count-in cannot be last');
    segments = next;
    const pendingStart = !started && !isInitial(next);
    const armedSection =
      pendingStart && structure ? (locate(last.at, structure, next)?.section ?? null) : null;
    if (
      sameView(view, transportState.view) &&
      pendingStart === transportState.pendingStart &&
      armedSection === transportState.armedSection
    ) {
      return;
    }
    transportState = { view, pendingStart, armedSection };
    transportSubscribers.forEach((fn) => fn(transportState));
  }

  /** The last successfully evaluated pattern (a failed evaluate leaves it); never exposed. */
  let evaluatedPattern: any = null;

  const r = repl({
    onSchedule: output,
    onScheduleTap: hooks?.onSchedule,
    getTime: () => active().getTime(),
    transpiler,
    // Runs only after the code evaluated, right before the scheduler gets the pattern: starting
    // from stopped schedules the first haps synchronously, before afterEval, so the audio seed is
    // set here. Every successful evaluate sets it; a song without one gets 0.
    editPattern: (pattern: any) => {
      const seeded = applySeed(pattern, pending.seed);
      active().setSeed(pending.seed ?? 0);
      return seeded;
    },
    setInterval: opts.setInterval,
    clearInterval: opts.clearInterval,
    beforeEval: async () => {
      await ensurePrebaked();
      await hooks?.beforeEval?.();
      // useRNG changes a module global; keep one song's choice from leaking into the next.
      useRNG('legacy');
      beginEvaluate();
      // While playing, meta's tempo waits for afterEval, so the old pattern keeps its tempo while
      // the code runs. Stopped, nothing plays: set it now so the first start runs at it.
      if (!r.scheduler.started) setCps(pending.cps);
      codeSetCps = false;
      codeRunning = true;
    },
    afterEval: (args: { code: string; pattern: any; meta: any }) => {
      codeRunning = false;
      // The new pattern is installed; switch the tempo with it unless the code chose its own.
      // setCps keeps the cycle continuous.
      if (!codeSetCps) setCps(pending.cps);
      const cps: number = r.scheduler.cps;
      // Before patch: a subscriber reacting to the new activeCode queries the new pattern.
      evaluatedPattern = args.pattern;
      patch({
        evalError: null,
        outputError: null,
        cps,
        meter: pending.meter,
        structure: registeredStructure(),
        tempoOverride: codeSetCps && Math.abs(cps - pending.cps) > TEMPO_EPSILON,
        seed: pending.seed,
        activeCode: args.code,
      });
      // The mix is about to change: earlier clips no longer describe it.
      restartLevels();
      // The pattern is already installed and playing; a visuals failure must not turn
      // a successful evaluate into an eval error.
      try {
        hooks?.afterEval?.(args);
      } catch (e) {
        console.error(e);
      }
    },
    onEvalError: (e: unknown) => {
      codeRunning = false;
      patch({ evalError: errorMessage(e) });
    },
    onToggle: (started: boolean) => {
      // Every start (a plain one, an autostarting evaluate, an engine switch) begins a new play.
      patch(started ? { started, engineWarnings: [] } : { started });
      // A play from stopped is a new hold; an engine switch (swapping) and a stop are not.
      if (started && !swapping) restartLevels();
      // Every stop (Stop, auto-stop, the scheduler's own) rewinds the transport; a start keeps
      // what was armed while stopped.
      setSegments(started ? segments : INITIAL_SEGMENTS, started);
      hooks?.onToggle?.(started);
    },
  });

  // The code's `setcps`/`setcpm` call the scheduler's setCps; note that they did, so afterEval
  // knows whether meta's tempo applies. The runtime's own calls go through the original.
  const rawSetCps: (cps: number) => void = r.scheduler.setCps.bind(r.scheduler);
  /**
   * The checkpoint a `setCps` left stale: the cyclist rebases it on its next tick, so until then
   * it no longer matches the tempo. A hap's `cps` control rebases it exactly in the tick, so only
   * this object (not the cyclist's tick counter) tells whether `cycleTime` can read it.
   */
  let staleCheckpoint: unknown = null;
  const setCps = (cps: number): void => {
    const changed = r.scheduler.cps !== cps;
    rawSetCps(cps);
    if (changed) staleCheckpoint = r.scheduler.checkpoint;
  };
  r.scheduler.setCps = (cps: number) => {
    if (codeRunning) codeSetCps = true;
    setCps(cps);
  };

  // Single-flight: every gesture while suspended calls this; share one attempt at a time.
  // Cheap when audio already runs, so the window can call it on every gesture: that also
  // recovers a context WebKit suspended behind our back (sleep, device change).
  let resuming: Promise<boolean> | undefined;
  const resumeAudio = (): Promise<boolean> => {
    if (active().isRunning()) {
      patch({ audio: 'running' });
      return Promise.resolve(true);
    }
    return (resuming ??= doResume().finally(() => (resuming = undefined)));
  };

  /** Resume attempts per backend: a late success of an older attempt is not the one in use. */
  const resumeAttempts = new Map<AudioBackend, number>();

  /**
   * `b.resume()`, called synchronously (WebKit only honours it inside the gesture), raced against
   * the backend's own timeout (else the option's), then `b.init()`. A resume outside a gesture can
   * stay pending forever. One that completes after its timeout opened a session nothing uses: it
   * is ended, unless a newer attempt is under way or the backend is playing. Throws what they throw.
   */
  const resumeBackend = async (b: AudioBackend): Promise<boolean> => {
    const attempt = (resumeAttempts.get(b) ?? 0) + 1;
    resumeAttempts.set(b, attempt);
    const resumed = b.resume();
    let timedOut = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<boolean>((resolve) => {
      timer = setTimeout(
        () => {
          timedOut = true;
          resolve(false);
        },
        b.resumeTimeoutMs ?? opts.resumeTimeoutMs ?? 1000,
      );
    });
    try {
      const ok = await Promise.race([resumed, timeout]);
      if (timedOut) {
        resumed.then(
          (late) => {
            const inUse = resumeAttempts.get(b) !== attempt || (b === active() && state.started);
            if (late && !inUse) void b.stop?.();
          },
          () => {},
        );
      }
      if (ok) await b.init();
      return ok;
    } finally {
      clearTimeout(timer);
    }
  };

  const doResume = async () => {
    let ok = false;
    try {
      ok = await resumeBackend(active());
    } catch (e) {
      ok = false; // a context without its worklets is not usable audio
      console.error(e);
      patch({ outputError: errorMessage(e) });
    }
    patch({ audio: ok ? 'running' : 'suspended' });
    return ok;
  };

  r.scheduler.latency = HAP_LATENCY_SEC;

  /** Engines left by a switch while playing, playing out what they were sent: their `stop` timer. */
  const draining = new Map<EngineId, ReturnType<typeof setTimeout>>();
  const endDrain = (id: EngineId): boolean => {
    const timer = draining.get(id);
    if (timer === undefined) return false;
    clearTimeout(timer);
    draining.delete(id);
    return true;
  };

  /**
   * Makes `id` the active engine; only while stopped. False when there is no backend for it.
   * Leaving a backend ends its session (`stop`): at once, or with `drain` (a switch while playing)
   * once the haps it was sent have played, `drain` seconds from now at least.
   */
  function swapTo(id: EngineId, drain: number | null = null): boolean {
    if (id === activeId) return true;
    const next = backendOf(id);
    if (!next) return false;
    const prev = active();
    const prevId = activeId;
    activeId = id;
    // Its session, if still draining, is the one in use again.
    endDrain(id);
    soundUntil[id] = 0;
    // The seed of the pattern in force: the evaluate that set it seeded the previous backend.
    next.setSeed(state.seed ?? 0);
    patch({ backend: next.id, engine: id, live: null, engineWarnings: [] });
    if (drain === null) void prev.stop?.();
    else if (prev.stop) {
      const queued = soundUntil[prevId] - prev.getTime();
      const sec = Math.min(Math.max(drain, queued, 0), DRAIN_MAX_SEC) + DRAIN_RELEASE_SEC;
      endDrain(prevId);
      const timer = setTimeout(() => {
        draining.delete(prevId);
        if (activeId !== prevId) void prev.stop?.();
      }, sec * 1000);
      draining.set(prevId, timer);
    }
    return true;
  }

  const ENGINES = ['superdough', 'rust'] as const;
  for (const id of ENGINES) {
    backendOf(id)?.onWarning?.((w) => {
      if (activeId !== id) return;
      if (state.engineWarnings.some((x) => x.signature === w.signature)) return;
      patch({ engineWarnings: [...state.engineWarnings, w] });
    });
  }

  for (const id of ENGINES) {
    const b = backendOf(id);
    // Both engines, active or not: a draining engine and the metronome's superdough still count.
    b?.onLevels?.((f) => {
      levelHold.push(f);
      pendingLevels = pendingLevels === null ? f : mergeFrames(pendingLevels, f);
    });
    b?.setLimiter?.(state.limiter);
  }

  let stops = 0;

  /**
   * Stops, then makes `to` (the song's engine by default) active; false when it has no backend.
   * `drain` (a switch while playing): the old engine plays out the haps it was sent, for at least
   * that many seconds, instead of being cut.
   */
  const stop = (to: EngineId = desired, drain: number | null = null): boolean => {
    const was = active();
    stops++;
    r.stop();
    pendingSwap = null;
    patch({ outputError: null });
    if (drain === null) {
      // Ends the session, cutting every voice already scheduled, also on an engine still playing
      // out a switch.
      for (const id of [...draining.keys()]) if (endDrain(id)) void backendOf(id)?.stop?.();
      void was.stop?.();
    }
    // A start's own engine (a take's superdough) lasts one play.
    return swapTo(to, drain);
  };

  const now = (): number => Math.max(0, r.scheduler.now());

  /** Drop segments that stopped governing more than `HISTORY_CYCLES` before `cycle`. */
  const pruneAt = (cycle: number) => {
    segments = prune(segments, cycle - HISTORY_CYCLES);
  };

  function position(): { cycle: number; located: Located | null } {
    const cycle = now();
    pruneAt(cycle);
    const s = state.structure;
    return { cycle, located: s ? locate(cycle, s, segments) : null };
  }

  /**
   * While stopped: the change at cycle 0 against the click's `structure` (it replaces the list).
   * While playing: append at the next safe bar boundary, against the evaluated structure (a no-op
   * without `song()`).
   */
  function change(
    structure: SongStructure,
    apply: (seg: readonly Segment[], s: SongStructure, at: number) => Segment[],
  ): void {
    if (!state.started) {
      setSegments(apply(segments, structure, 0), false, structure);
      return;
    }
    const s = state.structure;
    if (!s) return;
    pruneAt(now());
    setSegments(apply(segments, s, nextSafeBoundary(r.scheduler.lastEnd, s, segments, 'bar')!));
  }

  const transport: Transport = {
    store: {
      subscribe(run) {
        transportSubscribers.add(run);
        run(transportState);
        return () => transportSubscribers.delete(run);
      },
    },
    startFrom(section, structure) {
      change(structure, (seg, s, at) => withStartFrom(seg, s, section, at));
    },
    loopSection(section, structure) {
      if (section === null && !state.started) {
        transport.reset();
        return;
      }
      change(structure, (seg, s, at) => withLoop(seg, s, section, at));
    },
    playEnding() {
      const s = state.structure;
      if (!canPlayEnding(state.started, s, transportState.view, position().located)) return;
      const at = nextSafeBoundary(r.scheduler.lastEnd, s!, segments, 'loopPass');
      if (at === null) return;
      setSegments(withRelease(segments, s!, at));
    },
    reset() {
      setSegments(INITIAL_SEGMENTS);
    },
  };

  function query(
    fromCycle: number,
    toCycle: number,
    segments: readonly Segment[] = LINEAR_SEGMENTS,
  ): any[] | null {
    if (evaluatedPattern === null) return null;
    // `song()` reads its layout from the controls, which every nested query state inherits.
    const controls = { _cps: r.scheduler.cps, cyclist: 'cyclist', _segments: segments };
    const span = new State(new TimeSpan(fromCycle, toCycle), controls);
    // `query`, not `queryArc`: the latter swallows errors and answers [].
    return evaluatedPattern.query(span);
  }

  // Auto-stop: once the song has played past its end. One interval for the window's lifetime.
  // Also the engine switch at a bar line and the Rust engine's stats.
  (opts.setInterval ?? globalThis.setInterval)(() => {
    // A reference compare: subscribers wake once per stats event.
    const a = active();
    if (a.liveStats) patch({ live: a.liveStats() });
    patch({ limiterAvailable: a.limiterAvailable?.() ?? false });
    if (!state.started || evaluating) return;
    if (pendingSwap !== null && !swapping && queued === 0 && now() >= pendingSwap.atCycle) {
      void performSwap(pendingSwap);
      return;
    }
    const s = state.structure;
    if (!s || s.sections.length === 0) return;
    const { cycle, located } = position();
    // A count-in has no song bar either, but the song has yet to start.
    if (located === null && !inCountIn(cycle, segments)) stop();
  }, AUTO_STOP_MS);

  let swapping = false;
  /**
   * The switch while playing, once the playhead reached the bar line `swap.atCycle`. The old
   * engine keeps every hap the scheduler already sent it (up to `lastEnd`) and plays them out; the
   * new one starts on the same segments with the haps from `lastEnd` on, so each hap sounds on
   * exactly one engine and the song continues rather than restarting. Its scheduler starts early
   * enough (onsets before `lastEnd` filtered out) that `lastEnd` sounds when the old engine would
   * have played it.
   */
  async function performSwap(swap: { to: EngineId; atCycle: number }): Promise<void> {
    swapping = true;
    try {
      const sch = r.scheduler;
      const handoff: number = sch.lastEnd;
      const prev = active();
      // Seconds until the hand-off cycle sounds on the old engine (0 when its time is unknown).
      const at = cycleTime(handoff);
      const lead = at === null ? 0 : Math.max(0, at - prev.getTime());
      // A fresh start sounds its first cycle after the clock's minimum latency plus the hap lead.
      const firstSound = (sch.clock?.minLatency ?? 0) + sch.latency;
      const early = Math.max(0, lead - firstSound) * sch.cps;
      const from = { ...restartAt(segments, handoff - early), onsetsFrom: handoff };
      if (!stop(swap.to, lead)) return;
      const ok = await start(undefined, from);
      if (!ok && !state.started) {
        patch({ outputError: `could not restart on the ${swap.to} engine` });
      }
    } finally {
      swapping = false;
    }
  }

  /**
   * `from`: begin at that global cycle with those segments (an engine switch), not at 0, and with
   * `onsetsFrom` play no onset before that cycle (the old engine has played them).
   */
  async function start(
    startOpts?: StartOptions,
    from?: { cycle: number; segments: readonly Segment[]; onsetsFrom?: number },
  ): Promise<boolean> {
    const ready = () => !state.started && !evaluating && evaluatedPattern !== null;
    if (!ready()) return false;
    swapTo(resolveEngine(startOpts?.backend ?? desired));
    const generation = stops;
    // Synchronously, inside the caller's gesture: WebKit only honours resume() there.
    const resumed = resumeAudio();
    if (!(await resumed) || !ready() || generation !== stops) {
      // This start's own engine did not get to play: back to the song's.
      if (!state.started && generation === stops) swapTo(desired);
      return false;
    }
    const countIn = startOpts?.countIn === true;
    const savedSegments = segments;
    const savedPattern = r.scheduler.pattern;
    // Without `song()` no segment gates the pattern: delay the pattern itself by one cycle and
    // silence the count-in (a cyclic pattern shifted by a cycle would still sound there, with its
    // cycle -1). `evaluatedPattern` (what `query` reads) stays unshifted; a plain start puts it back.
    try {
      let pattern =
        countIn && state.structure === null
          ? evaluatedPattern.late(1).filterHaps((h: any) => (h.whole ?? h.part).begin.gte(1))
          : evaluatedPattern;
      const onsetsFrom = from?.onsetsFrom;
      if (onsetsFrom !== undefined) {
        pattern = pattern.filterHaps((h: any) => (h.whole ?? h.part).begin.gte(onsetsFrom));
      }
      if (r.scheduler.pattern !== pattern) await r.scheduler.setPattern(pattern, false);
      // Directly, not through setSegments: the transport state is the same with a count-in.
      if (countIn && state.structure !== null) segments = withCountIn(segments);
      if (from) {
        segments = from.segments;
        // The cyclist's first tick queries from `lastEnd` (a stop sets it to 0).
        r.scheduler.lastEnd = from.cycle;
      }
      await r.scheduler.start();
      return true;
    } catch (e) {
      console.error(e);
      // A stop rewinds the transport, as every stop does; otherwise put the segments back.
      if (r.scheduler.started) r.stop();
      else {
        segments = savedSegments;
        r.scheduler.lastEnd = 0;
      }
      if (r.scheduler.pattern !== savedPattern) await r.scheduler.setPattern(savedPattern, false);
      return false;
    }
  }

  function cycleTime(cycle: number): number | null {
    const sch = r.scheduler;
    // `start()` ticks the clock at once, so a started scheduler has a checkpoint; a tick since
    // the last `setCps` (or a `cps` hap's exact rebase) makes it current.
    if (!sch.started) return null;
    if (sch.num_ticks_since_checkpoint === 0 && sch.checkpoint === staleCheckpoint) return null;
    // cyclist.mjs's hap target time plus its fixed latency.
    const t = (cycle - sch.checkpoint.cycles) / sch.cps + sch.checkpoint.seconds + sch.latency;
    return Number.isFinite(t) ? t : null;
  }

  /** Evaluates run one at a time, so each one's `pending` tempo is the one its hooks read. */
  let queue: Promise<unknown> = Promise.resolve();
  /** Evaluates called and not finished: an engine switch waits for them (one may revert it). */
  let queued = 0;
  let starts = 0;
  let auditionResume: Promise<boolean> | undefined;
  const canAudition = () =>
    !state.started && !swapping && draining.size === 0 && queued === 0 && starts === 0;

  return {
    scheduler: r.scheduler,
    store: {
      subscribe(run) {
        subscribers.add(run);
        run(state);
        return () => subscribers.delete(run);
      },
    },
    evaluate(code, evalOpts) {
      if (!code.trim()) {
        patch({ evalError: 'Nothing to evaluate: the editor is empty.' });
        return Promise.resolve(false);
      }
      opts.hooks?.onPlaybackRequest?.();
      const tempo = {
        cps: evalOpts?.cps ?? defaultCps,
        meter: evalOpts?.meter ?? DEFAULT_METER,
        seed: evalOpts?.seed,
      };
      const autostart = evalOpts?.autostart ?? true;
      // Synchronously, before resumeAudio, so this gesture resumes the song's engine.
      const target = resolveEngine(evalOpts?.engine ?? 'superdough');
      const prevDesired = desired;
      const prevSwap = pendingSwap;
      const id = ++evaluates;
      desired = target;
      desiredBy = id;
      if (!state.started) swapTo(target);
      else if (target === activeId) {
        // Back to the playing engine: a switch's target resumed for it is not needed.
        // (One still playing out an earlier switch keeps its drain.)
        const unused = pendingSwap?.to;
        if (unused && unused !== activeId && !draining.has(unused)) {
          void backendOf(unused)?.stop?.();
        }
        pendingSwap = null;
      } else {
        pendingSwap = { to: target, atCycle: Math.floor(position().cycle) + 1 };
        // The switch starts the target from the interval, outside any gesture, where WebKit would
        // not honour its resume(): resume it now, inside this one.
        resumeBackend(backendOf(target)!).catch((e) => console.error(e));
      }
      const generation = stops;
      // Synchronously, inside the caller's gesture: WebKit only honours resume() there.
      const resumed = resumeAudio();
      queued++;
      const run = queue.then(async () => {
        const running = await resumed;
        pending = tempo;
        evaluating = true;
        const cpsBefore: number = r.scheduler.cps;
        const rngBefore: string = getRNGMode();
        let ok = false;
        try {
          // A stop() pressed after this call wins: evaluate, but don't autostart.
          ok = (await r.evaluate(code, autostart && running && generation === stops)) !== undefined;
          return ok;
        } finally {
          codeRunning = false;
          // A failed evaluate changes nothing: undo a tempo its code (or, stopped, meta) set.
          if (!ok && r.scheduler.cps !== cpsBefore) setCps(cpsBefore);
          // Also the RNG mode beforeEval reset: the pattern still playing uses it.
          if (!ok) useRNG(rngBefore);
          // And the engine switch it asked for, unless a later evaluate has asked for its own
          // (even for the same engine: that one may succeed).
          if (!ok && desiredBy === id) {
            desired = prevDesired;
            pendingSwap = prevSwap;
            if (!state.started) swapTo(prevDesired);
          }
          evaluating = false;
          queued--;
        }
      });
      queue = run.catch(() => {});
      return run;
    },
    stop() {
      opts.hooks?.onPlaybackRequest?.();
      stop();
    },
    start(startOpts) {
      opts.hooks?.onPlaybackRequest?.();
      starts++;
      return start(startOpts).finally(() => starts--);
    },
    cycleTime,
    resumeAudio,
    canAudition,
    prepareAudition() {
      if (!canAudition()) return Promise.resolve(false);
      const backend = opts.backends.superdough;
      return (auditionResume ??= (async () => {
        if (backend.isRunning()) {
          await backend.init();
        } else if (!(await (active() === backend ? resumeAudio() : resumeBackend(backend)))) {
          return false;
        }
        await ensurePrebaked();
        return true;
      })()
        .catch((error) => {
          console.error(error);
          return false;
        })
        .finally(() => (auditionResume = undefined)));
    },
    transport,
    position,
    query,
    readScope: (buf) => active().readScope?.(buf) ?? false,
    drainLevels() {
      const f = pendingLevels;
      pendingLevels = null;
      return f;
    },
    levels: () => levelHold.snapshot(),
    setLimiter(on) {
      patch({ limiter: on });
      for (const id of ENGINES) backendOf(id)?.setLimiter?.(on);
    },
  };
}

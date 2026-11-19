/**
 * The Rust live backend: haps go to the native engine (`strudeland-live`) through `PlaybackApi`,
 * and the engine's anchors give the clock. Values are interned per session (each distinct encoded
 * control map crosses once), haps are batched per run of equal cps and flushed on a microtask, so
 * a scheduler tick costs one invoke. No `AudioContext`, no `graph()`: the audio never touches the
 * webview. Only the `PlaybackApi` type is known here; the Tauri calls live in `playback-api.ts`.
 */
import { SAMPLE_RATE } from '../render/contract';
import { nearestRank, sortedAsc } from '../render/dsp';
import { canonicalJson, encodeControlMap, type EngineValue } from '../render/engine-job';
import { orbitChannels, orbitNumber } from '../render/plan';
import { measureClockOffset } from '../recording/sync';
import type { AudioBackend, EngineWarning } from './backend';
import type { LevelFrame } from '../levels/levels';
import { SCOPE_FFT_SIZE } from './scope-tap';
import {
  PlaybackError,
  type LiveStats,
  type OrbitSpec,
  type PlaybackApi,
  type PlaybackBatch,
  type PlaybackEvent,
} from './playback-api';

/** Distinct values one session may intern (engine-side eviction is a follow-up). */
export const MAX_VALUES = 16384;
/** How long `resume()` waits for the first anchor. */
export const ANCHOR_TIMEOUT_MS = 2000;
/**
 * How long the runtime waits for `resume()`: the native start (opening the CoreAudio device on a
 * blocking thread, `START_TIMEOUT` = 5 s in `crates/strudeland-live/src/player.rs`), then the
 * first anchor (`ANCHOR_TIMEOUT_MS`), with a second to spare.
 */
export const RESUME_TIMEOUT_MS = 8000;
/** An anchor arriving this long after the last clock-offset measurement re-measures it. */
export const REMEASURE_AFTER_MS = 60_000;
/** A scope frame older than this is no signal. */
const SCOPE_FRESH_MS = 500;
/** Schedule round trips kept for `invokeRttMs`. */
const RTT_WINDOW = 256;
/** Samples in one scope frame. */
const SCOPE_SAMPLES = 1024;

export const TOO_MANY_VALUES = `rust engine: too many distinct values in one play (${MAX_VALUES}); stop and play again`;

export type { EngineWarning } from './backend';

/** The Rust backend: every optional live hook is present. */
export type RustBackend = AudioBackend &
  Required<
    Pick<
      AudioBackend,
      'liveStats' | 'onWarning' | 'stop' | 'onLevels' | 'setLimiter' | 'limiterAvailable'
    >
  >;

type ValueEntry = PlaybackBatch['values'][number];
type HapEntry = PlaybackBatch['haps'][number];
interface Run {
  cps: number;
  values: ValueEntry[];
  haps: HapEntry[];
}

/** `{ p50, p95, max }` of `samples` (`nearestRank` quantiles; zeros when empty). */
export function rttSummary(samples: readonly number[]): LiveStats['invokeRttMs'] {
  if (samples.length === 0) return { p50: 0, p95: 0, max: 0 };
  const sorted = sortedAsc(samples);
  return {
    p50: nearestRank(sorted, 0.5),
    p95: nearestRank(sorted, 0.95),
    max: sorted[sorted.length - 1]!,
  };
}

/** Fills `out` with `frame` stretched across it, linearly interpolated. Zero padding would leave
 *  the scope's trigger search (the first half) on silence and draw a flat line. */
function stretch(frame: Float32Array, out: Float32Array): void {
  const len = out.length;
  const last = frame.length - 1;
  const step = last / (len - 1);
  for (let i = 0; i < len; i++) {
    const x = i * step;
    const j = Math.min(Math.floor(x), last - 1);
    const f = x - j;
    out[i] = frame[j]! * (1 - f) + frame[j + 1]! * f;
  }
}

const asError = (e: unknown): Error => (e instanceof Error ? e : new Error(String(e)));
const isStale = (e: unknown): boolean => e instanceof PlaybackError && e.kind === 'stale';

export function createRustBackend(
  api: PlaybackApi,
  deps: { nowMs?: () => number } = {},
): RustBackend {
  const nowMs = deps.nowMs ?? performance.now.bind(performance);

  let session: number | null = null;
  /** Bumped by `stop()`: events, starts and rejections of an older generation are dropped. */
  let generation = 0;
  let seed = 0;
  /** The latest anchor, its host time in ms (a double holds it to well under a nanosecond). */
  let anchor: { frame: number; hostMs: number } | null = null;
  /** Host clock minus `nowMs()`, in ms. */
  let offsetMs: number | null = null;
  let offsetAtMs: number | null = null;
  let remeasuring = false;
  let initPromise: Promise<void> | null = null;
  let starting: Promise<boolean> | null = null;
  const anchorWaiters = new Set<(ok: boolean) => void>();

  const values = new Map<string, number>();
  const orbits = new Set<number>();
  let runs: Run[] = [];
  let microtaskQueued = false;
  const rtt: number[] = [];
  let stats: LiveStats | null = null;
  const scope = new Float32Array(SCOPE_SAMPLES);
  /** `scope` stretched across the scope clock's buffer, computed once per frame. */
  const scopeView = new Float32Array(SCOPE_FFT_SIZE);
  let scopeAtMs: number | null = null;
  let pendingError: Error | null = null;
  const warningListeners = new Set<(w: EngineWarning) => void>();
  const levelListeners = new Set<(f: LevelFrame) => void>();
  /** The limiter preference; it outlives sessions and is re-sent when one opens. */
  let limiter = false;

  const measure = async (): Promise<number> =>
    Number((await measureClockOffset({ hostNowNs: () => api.hostNowNs(), nowMs })).offsetNs) / 1e6;

  /** A rejection of a call made in generation `gen`: `stale` and older generations are ignored. */
  const failed = (gen: number) => (e: unknown) => {
    if (gen !== generation || isStale(e)) return;
    pendingError = asError(e);
  };

  const resolveAnchorWaiters = (ok: boolean) => {
    for (const w of [...anchorWaiters]) w(ok);
  };

  const onEvent = (e: PlaybackEvent) => {
    switch (e.kind) {
      case 'anchor': {
        anchor = { frame: e.frame, hostMs: Number(BigInt(e.hostNs)) / 1e6 };
        if (!remeasuring && offsetAtMs !== null && nowMs() - offsetAtMs > REMEASURE_AFTER_MS) {
          remeasuring = true;
          offsetAtMs = nowMs();
          measure().then(
            (o) => {
              offsetMs = o;
              offsetAtMs = nowMs();
              remeasuring = false;
            },
            () => (remeasuring = false),
          );
        }
        resolveAnchorWaiters(true);
        break;
      }
      case 'warning': {
        const w = { signature: e.signature, keys: e.keys, firstCycle: e.firstCycle };
        for (const cb of [...warningListeners]) cb(w);
        break;
      }
      case 'stats': {
        const { kind: _kind, ...rest } = e;
        stats = { ...rest, invokeRttMs: rttSummary(rtt) };
        break;
      }
      case 'scope': {
        const src = e.samples;
        const n = Math.min(src.length, SCOPE_SAMPLES);
        const at = SCOPE_SAMPLES - n;
        const from = src.length - n;
        scope.fill(0, 0, at);
        for (let i = 0; i < n; i++) scope[at + i] = src[from + i]!;
        stretch(scope, scopeView);
        scopeAtMs = nowMs();
        break;
      }
      case 'levels': {
        const { kind: _kind, ...frame } = e;
        for (const cb of [...levelListeners]) cb(frame);
        break;
      }
      case 'error':
        pendingError = new Error(e.message);
        break;
    }
  };

  const waitForAnchor = (): Promise<boolean> => {
    if (anchor !== null) return Promise.resolve(true);
    return new Promise<boolean>((resolve) => {
      const done = (ok: boolean) => {
        clearTimeout(timer);
        anchorWaiters.delete(done);
        resolve(ok);
      };
      const timer = setTimeout(() => done(false), ANCHOR_TIMEOUT_MS);
      anchorWaiters.add(done);
    });
  };

  const open = async (): Promise<boolean> => {
    if (session === null) {
      const gen = generation;
      const s = await api.start(seed, (e) => {
        if (gen === generation) onEvent(e);
      });
      if (gen !== generation) {
        // Stopped while starting: end the session that just opened.
        api.stop(s).catch(() => {});
        return false;
      }
      session = s;
      if (limiter) api.setLimiter(s, true).catch(failed(gen));
    }
    const opened = session;
    const valid = await waitForAnchor();
    if (!valid && session === opened) await stop();
    return valid;
  };

  const flush = () => {
    microtaskQueued = false;
    const s = session;
    const queued = runs;
    runs = [];
    if (s === null) return;
    const gen = generation;
    for (const run of queued) {
      if (run.values.length === 0 && run.haps.length === 0) continue;
      const t0 = nowMs();
      const batch: PlaybackBatch = { values: run.values, haps: run.haps, cps: run.cps };
      let p: Promise<void>;
      try {
        p = api.schedule(s, batch);
      } catch (e) {
        failed(gen)(e);
        continue;
      }
      p.then(() => {
        rtt.push(nowMs() - t0);
        if (rtt.length > RTT_WINDOW) rtt.splice(0, rtt.length - RTT_WINDOW);
      }, failed(gen));
    }
  };

  const stop = async (): Promise<void> => {
    generation++;
    resolveAnchorWaiters(false);
    if (session === null) return;
    const s = session;
    session = null;
    anchor = null;
    values.clear();
    orbits.clear();
    runs = [];
    stats = null;
    pendingError = null;
    scopeAtMs = null;
    try {
      await api.stop(s);
    } catch {
      // The session is gone either way.
    }
  };

  const backend: RustBackend = {
    id: 'rust',
    resumeTimeoutMs: RESUME_TIMEOUT_MS,

    init() {
      initPromise ??= (async () => {
        offsetMs = await measure();
        offsetAtMs = nowMs();
      })().catch((e) => {
        initPromise = null;
        throw e;
      });
      return initPromise;
    },

    resume() {
      if (session !== null && anchor !== null) return Promise.resolve(true);
      starting ??= open().finally(() => (starting = null));
      return starting;
    },

    isRunning: () => session !== null && anchor !== null && offsetMs !== null,

    getTime() {
      if (session === null || anchor === null || offsetMs === null) return 0;
      return anchor.frame / SAMPLE_RATE + (offsetMs + nowMs() - anchor.hostMs) / 1000;
    },

    setSeed(next) {
      // Every evaluate re-sends the seed; the same one again changes nothing.
      if (next === seed) return;
      seed = next;
      if (session !== null) {
        const gen = generation;
        api.setSeed(session, next).catch(failed(gen));
      }
    },

    output(hap: any, t, cps) {
      if (pendingError) {
        const e = pendingError;
        pendingError = null;
        throw e;
      }
      if (session === null) return;
      hap.ensureObjectValue();
      if (!hap.whole) return;
      const value = hap.value as Record<string, unknown>;
      const duration = Number(hap.duration) / cps;
      const cycle = Number(hap.whole.begin.valueOf());
      if (!Number.isFinite(t) || !Number.isFinite(duration) || !Number.isFinite(cycle)) return;

      let run = runs[runs.length - 1];
      if (!run || run.cps !== cps) {
        run = { cps, values: [], haps: [] };
        runs.push(run);
      }
      const encoded: Record<string, EngineValue> = encodeControlMap(value);
      const key = canonicalJson(encoded);
      let index = values.get(key);
      if (index === undefined) {
        if (values.size >= MAX_VALUES) {
          // Past the cap the session ends; the runtime shows the error.
          void stop();
          throw new Error(TOO_MANY_VALUES);
        }
        index = values.size;
        values.set(key, index);
        const orbit = orbitNumber(value);
        let spec: OrbitSpec | null = null;
        if (!orbits.has(orbit)) {
          orbits.add(orbit);
          spec = { orbit, channels: orbitChannels(value, false) };
        }
        run.values.push([index, encoded, spec]);
      }
      run.haps.push([t, duration, cycle, index]);
      if (!microtaskQueued) {
        microtaskQueued = true;
        queueMicrotask(flush);
      }
    },

    readScope(buf) {
      if (scopeAtMs === null || nowMs() - scopeAtMs > SCOPE_FRESH_MS) return false;
      const len = buf.length;
      if (len <= SCOPE_SAMPLES) buf.set(scope.subarray(SCOPE_SAMPLES - len));
      else if (len === SCOPE_FFT_SIZE) buf.set(scopeView);
      else stretch(scope, buf);
      return true;
    },

    liveStats: () => stats,

    onLevels(cb) {
      levelListeners.add(cb);
      return () => void levelListeners.delete(cb);
    },

    setLimiter(on) {
      limiter = on;
      if (session !== null) api.setLimiter(session, on).catch(failed(generation));
    },

    limiterAvailable: () => true,

    onWarning(cb) {
      warningListeners.add(cb);
      return () => void warningListeners.delete(cb);
    },

    stop,
  };
  return backend;
}

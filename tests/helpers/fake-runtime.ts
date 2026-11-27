/** A runtime on a fake audio backend, with the scheduler clock and timers under the test's control. */
import { createRuntime, type RuntimeHooks } from '../../src/lib/strudel/runtime';
import type { AudioBackend, AudioGraph } from '../../src/lib/audio/backend';
import type { LevelFrame } from '../../src/lib/levels/levels';
import type { LiveStats } from '../../src/lib/audio/playback-api';
import type { EngineWarning, RustBackend } from '../../src/lib/audio/rust';

export function fakeRuntime(
  opts: {
    running?: boolean;
    failOutput?: boolean;
    hang?: boolean;
    resume?: () => Promise<boolean>;
    init?: () => Promise<void>;
    prebake?: () => Promise<void>;
    hooks?: RuntimeHooks;
    /** The backend's `graph()` (e.g. `fakeAudioGraph().graph`); absent: the backend has none. */
    graph?: AudioGraph | (() => AudioGraph | null);
    /** Also a fake rust backend (`rb`), on the same fake clock. */
    rust?: boolean;
    /** What the fake rust backend's `resume()` resolves to (default true). */
    rustResume?: () => Promise<boolean>;
    /** The fake rust backend's own `resumeTimeoutMs` (absent: none declared). */
    rustResumeTimeoutMs?: number;
    /** The runtime's `resumeTimeoutMs` option. */
    resumeTimeoutMs?: number;
    /** The runtime's initial applied limiter. */
    limiter?: boolean;
  } = {},
) {
  const b = {
    now: 0,
    running: opts.running ?? true,
    calls: [] as any[],
    resumes: 0,
    seeds: [] as number[],
    /** setSeed and output calls in call order: `seed <n>` and `output`. */
    order: [] as string[],
    levelListeners: new Set<(f: LevelFrame) => void>(),
    limiters: [] as boolean[],
    /** What the superdough backend's `limiterAvailable()` answers. */
    limiterOk: true,
    /** Sends a `levels` frame to every listener. */
    level(f: LevelFrame) {
      this.levelListeners.forEach((cb) => cb(f));
    },
  };
  // Scheduler clock and auto-stop share the seam; each interval is cleared by its own id.
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const backend: AudioBackend = {
    id: 'superdough',
    init: opts.init ?? (async () => {}),
    resume: () => {
      b.resumes++;
      if (opts.resume) return opts.resume();
      return opts.hang ? new Promise<boolean>(() => {}) : Promise.resolve(b.running);
    },
    // Like the real backend: nothing runs until a resume has succeeded.
    isRunning: () => b.running && b.resumes > 0,
    getTime: () => b.now,
    setSeed: (seed) => {
      b.seeds.push(seed);
      b.order.push(`seed ${seed}`);
    },
    onLevels: (cb) => {
      b.levelListeners.add(cb);
      return () => b.levelListeners.delete(cb);
    },
    setLimiter: (on) => void b.limiters.push(on),
    limiterAvailable: () => b.limiterOk,
    output: (...args) => {
      if (opts.failOutput) throw new Error('sound nope not found! Is it loaded?');
      b.calls.push(args);
      b.order.push('output');
    },
  };
  const graph = opts.graph;
  if (graph) {
    // Like the real backend: no graph until audio runs.
    backend.graph = () =>
      backend.isRunning() ? (typeof graph === 'function' ? graph() : graph) : null;
  }
  const rb = {
    running: false,
    calls: [] as any[],
    resumes: 0,
    seeds: [] as number[],
    stops: 0,
    stats: null as LiveStats | null,
    listeners: new Set<(w: EngineWarning) => void>(),
    scopes: 0,
    levelListeners: new Set<(f: LevelFrame) => void>(),
    limiters: [] as boolean[],
    /** Sends a `levels` frame to every listener. */
    level(f: LevelFrame) {
      this.levelListeners.forEach((cb) => cb(f));
    },
    /** Sends a `warning` event to every listener. */
    warn(w: EngineWarning) {
      this.listeners.forEach((cb) => cb(w));
    },
  };
  const rust: RustBackend | undefined = opts.rust
    ? {
        id: 'rust',
        resumeTimeoutMs: opts.rustResumeTimeoutMs,
        init: async () => {},
        resume: async () => {
          rb.resumes++;
          rb.running = opts.rustResume ? await opts.rustResume() : true;
          return rb.running;
        },
        isRunning: () => rb.running,
        getTime: () => b.now,
        setSeed: (seed) => void rb.seeds.push(seed),
        output: (...args) => void rb.calls.push(args),
        readScope: () => (rb.scopes++, true),
        liveStats: () => rb.stats,
        onWarning: (cb) => {
          rb.listeners.add(cb);
          return () => rb.listeners.delete(cb);
        },
        onLevels: (cb) => {
          rb.levelListeners.add(cb);
          return () => rb.levelListeners.delete(cb);
        },
        setLimiter: (on) => void rb.limiters.push(on),
        limiterAvailable: () => true,
        stop: async () => {
          rb.stops++;
          rb.running = false;
        },
      }
    : undefined;
  const tick = (n = 40) => {
    for (let i = 0; i < n; i++) {
      b.now += 0.1;
      // A timer cleared by an earlier one in this round (a stop) does not fire, as in a browser.
      for (const [id, f] of [...timers]) if (timers.has(id)) f();
    }
  };
  const rt = createRuntime({
    backends: { superdough: backend, rust },
    prebake: opts.prebake ?? (async () => {}),
    hooks: opts.hooks,
    resumeTimeoutMs: opts.resumeTimeoutMs,
    limiter: opts.limiter,
    setInterval: (fn) => {
      timers.set(++nextTimer, fn);
      return nextTimer;
    },
    clearInterval: (id) => {
      timers.delete(id as number);
    },
  });
  return { b, rb, rt, tick, timers, backend, rust };
}

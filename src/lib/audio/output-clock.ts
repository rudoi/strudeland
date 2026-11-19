/**
 * Reads when the context's audio leaves the speakers: `getOutputTimestamp()` pairs a context time
 * with the `performance.now()` at which it is heard. The recording sync maps that to host time.
 * Only reads the graph the backend exposes; never creates a context.
 */
import type { AudioGraph } from './backend';

/** One `getOutputTimestamp()` reading: `contextTime` is heard at `performanceMs`. */
export interface OutputStamp {
  contextTime: number;
  performanceMs: number;
}

/** The context's latencies in seconds, recorded with a take (never used in the maths). */
export interface OutputLatency {
  baseLatency: number;
  /** Null when the context does not report one (WebKit) or reports 0. */
  outputLatency: number | null;
}

export interface OutputClock {
  /** The current output stamp, or null when there is none yet (no graph, no method, a 0 stamp). */
  read(): OutputStamp | null;
  latency(): OutputLatency;
  /** The context's `currentTime`, or null without a running graph. */
  now(): number | null;
}

const positive = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v) && v > 0;

/** `graph` is the backend's `graph()`; it is called on every read, never kept. Never throws. */
export function createOutputClock(deps: { graph: () => AudioGraph | null }): OutputClock {
  const context = (): AudioContext | null => {
    try {
      return deps.graph()?.context ?? null;
    } catch {
      return null;
    }
  };
  return {
    read() {
      const ctx = context();
      if (!ctx || typeof ctx.getOutputTimestamp !== 'function') return null;
      let stamp: AudioTimestamp;
      try {
        stamp = ctx.getOutputTimestamp();
      } catch {
        return null;
      }
      // WebKit answers 0 for both until the first render quantum has played.
      const { contextTime, performanceTime } = stamp ?? {};
      if (!positive(contextTime) || !positive(performanceTime)) return null;
      return { contextTime, performanceMs: performanceTime };
    },
    latency() {
      const ctx = context();
      const base = ctx?.baseLatency;
      const output = ctx?.outputLatency;
      return {
        baseLatency: typeof base === 'number' && Number.isFinite(base) && base >= 0 ? base : 0,
        outputLatency: positive(output) ? output : null,
      };
    },
    now() {
      const t = context()?.currentTime;
      return typeof t === 'number' && Number.isFinite(t) ? t : null;
    },
  };
}

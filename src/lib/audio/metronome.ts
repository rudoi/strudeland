/**
 * A click track on the live audio graph, independent of the song: it never touches Strudel and
 * plays with nothing evaluated. Clicks are scheduled ahead on the context clock by a short timer,
 * each at a time computed from its index (never accumulated), so nothing drifts.
 */
import type { AudioGraph } from './backend';

export interface MetronomeGrid {
  /** Context time of a downbeat, seconds. */
  T: number;
  cps: number;
  /** Meter numerator: clicks per bar. */
  num: number;
  /** Integer click index (<= 0) of the first click, relative to `T`. */
  from: number;
  /** Context time, exclusive; absent: until `stop()`. */
  until?: number;
}

export interface Metronome {
  /** False, starting nothing, without a graph or with an invalid grid. Replaces a running grid. */
  start(grid: MetronomeGrid): boolean;
  /** Idempotent. Clicks already scheduled (at most `LOOKAHEAD_S` ahead) play out. */
  stop(): void;
  readonly running: boolean;
}

export interface MetronomeDeps {
  /** The backend's `graph()`; called on every tick, never kept. */
  graph: () => AudioGraph | null;
  setInterval?: (fn: () => void, ms: number) => unknown;
  clearInterval?: (id: unknown) => void;
}

/** How often the timer schedules. */
export const TICK_MS = 25;
/** How far ahead of `currentTime` a tick schedules clicks. */
export const LOOKAHEAD_S = 0.1;

const ACCENT = { frequency: 1200, peak: 0.5 };
const BEAT = { frequency: 800, peak: 0.3 };
const ATTACK_S = 0.002;
const DECAY_END_S = 0.05;
const STOP_S = 0.06;
const FLOOR = 0.0001;

/** Seconds between clicks: one click per meter numerator unit (4/4: 4 a bar, 6/8: 6). */
export const clickInterval = (cps: number, num: number): number => 1 / (cps * num);

/** Mathematical modulo: `-1 mod 4` is 3. */
const mod = (a: number, n: number): number => ((a % n) + n) % n;

/** Click `i` is the bar's accent iff `i mod num = 0`. */
export const isAccent = (i: number, num: number): boolean => mod(i, num) === 0;

export function validGrid(g: MetronomeGrid): boolean {
  return (
    Number.isFinite(g.T) &&
    Number.isFinite(g.cps) &&
    g.cps > 0 &&
    Number.isInteger(g.num) &&
    g.num >= 1 &&
    Number.isInteger(g.from) &&
    g.from <= 0 &&
    (g.until === undefined || !Number.isNaN(g.until))
  );
}

function click(graph: AudioGraph, t: number, accent: boolean): void {
  const ctx = graph.context;
  const sound = accent ? ACCENT : BEAT;
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = 'sine';
  osc.frequency.value = sound.frequency;
  gain.gain.setValueAtTime(0, t);
  gain.gain.linearRampToValueAtTime(sound.peak, t + ATTACK_S);
  gain.gain.exponentialRampToValueAtTime(FLOOR, t + DECAY_END_S);
  osc.connect(gain);
  gain.connect(graph.destination);
  osc.onended = () => {
    osc.disconnect();
    gain.disconnect();
  };
  osc.start(t);
  osc.stop(t + STOP_S);
}

export function createMetronome(deps: MetronomeDeps): Metronome {
  const setTimer = deps.setInterval ?? ((fn, ms) => globalThis.setInterval(fn, ms));
  const clearTimer =
    deps.clearInterval ??
    ((id) => globalThis.clearInterval(id as ReturnType<typeof globalThis.setInterval>));
  let grid: MetronomeGrid | null = null;
  /** Index of the next click not yet scheduled or skipped. */
  let next = 0;
  let timer: unknown = undefined;
  let hasTimer = false;

  const graphOrNull = (): AudioGraph | null => {
    try {
      return deps.graph();
    } catch {
      return null;
    }
  };

  function stop(): void {
    if (hasTimer) clearTimer(timer);
    hasTimer = false;
    timer = undefined;
    grid = null;
  }

  function tick(): void {
    const g = grid;
    if (!g) return;
    const graph = graphOrNull();
    if (!graph) return; // the output is being rebuilt: the next tick catches up
    const now = graph.context.currentTime;
    const horizon = now + LOOKAHEAD_S;
    const interval = clickInterval(g.cps, g.num);
    // Jump over clicks long past (they are skipped, never played late); the loop skips the rest.
    next = Math.max(next, Math.ceil((now - g.T) / interval) - 1);
    for (;;) {
      const t = g.T + next * interval;
      if (g.until !== undefined && t >= g.until) {
        stop();
        return;
      }
      if (t >= horizon) return;
      if (t >= now) {
        try {
          click(graph, t, isAccent(next, g.num));
        } catch (e) {
          console.error(e);
        }
      }
      next++;
    }
  }

  return {
    start(g) {
      stop();
      if (!validGrid(g) || !graphOrNull()) return false;
      grid = { ...g };
      next = g.from;
      tick();
      if (grid) {
        timer = setTimer(tick, TICK_MS);
        hasTimer = true;
      }
      return true;
    },
    stop,
    get running() {
      return grid !== null;
    },
  };
}

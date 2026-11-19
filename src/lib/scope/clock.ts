/**
 * One shared animation loop for every scope: it reads the output once per frame and fans the
 * buffer out to the subscribers. One clock per window (built by `createApp`).
 */

import { SCOPE_FFT_SIZE } from '../audio/scope-tap';

/** The samples read per frame: the tap's analyser size, so one read fills the buffer exactly. */
export const SCOPE_BUFFER_SIZE = SCOPE_FFT_SIZE;

/** The Svelte context key `App.svelte` provides the clock under. */
export const SCOPE_CLOCK_KEY = Symbol('scope-clock');

/**
 * Called with the latest samples each frame, or `null` when there is nothing to show. `running` is
 * true for a call from the running loop (even with no samples) and false for the one rest call
 * when the loop stops (stopped, or the window hidden).
 */
export type ScopeSubscriber = (buf: Float32Array | null, running: boolean) => void;

export interface ScopeClock {
  /** Not called on subscribe; returns the unsubscribe function. */
  subscribe(fn: ScopeSubscriber): () => void;
  setRunning(on: boolean): void;
  /**
   * Stops the loop for good: removes the `visibilitychange` listener, cancels the pending frame and
   * drops every subscriber (none is called). Later calls do nothing. Idempotent.
   */
  dispose(): void;
}

type Raf = (cb: (t: number) => void) => number;
type Caf = (handle: number) => void;

// Looked up when called, not when created, so building a clock needs no DOM.
const defaultRaf: Raf = (cb) => requestAnimationFrame(cb);
const defaultCaf: Caf = (h) => cancelAnimationFrame(h);

const pageHidden = () => typeof document !== 'undefined' && document.hidden;

export function createScopeClock(
  read: (buf: Float32Array) => boolean,
  raf: Raf = defaultRaf,
  caf: Caf = defaultCaf,
): ScopeClock {
  const buf = new Float32Array(SCOPE_BUFFER_SIZE);
  const subscribers = new Set<ScopeSubscriber>();
  /**
   * `subscribers` as an array, replaced (never mutated) on subscribe and unsubscribe: a frame
   * allocates nothing, and a change made during a notify takes effect from the next one.
   */
  let snapshot: ScopeSubscriber[] = [];
  let running = false;
  /** Whether the loop is on; `handle` is the pending frame (null inside a frame). */
  let active = false;
  let handle: number | null = null;

  let disposed = false;

  const shouldRun = () => !disposed && running && subscribers.size > 0 && !pageHidden();
  const notify = (b: Float32Array | null, live: boolean) => {
    const fns = snapshot;
    for (let i = 0; i < fns.length; i++) fns[i]!(b, live);
  };

  const frame = () => {
    handle = null;
    if (!active) return;
    notify(read(buf) ? buf : null, true);
    if (active && handle === null) handle = raf(frame);
  };

  /** Start or stop the loop to match the state; a stop with subscribers left rests them once. */
  const update = () => {
    if (shouldRun()) {
      if (active) return;
      active = true;
      handle = raf(frame);
    } else if (active) {
      active = false;
      if (handle !== null) caf(handle);
      handle = null;
      if (subscribers.size > 0) notify(null, false);
    }
  };

  const doc = typeof document !== 'undefined' ? document : null;
  doc?.addEventListener('visibilitychange', update);

  return {
    subscribe(fn) {
      if (disposed) return () => {};
      subscribers.add(fn);
      snapshot = [...subscribers];
      update();
      return () => {
        if (!subscribers.delete(fn)) return;
        snapshot = [...subscribers];
        update();
      };
    },
    setRunning(on) {
      running = on;
      update();
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      doc?.removeEventListener('visibilitychange', update);
      subscribers.clear();
      snapshot = [];
      active = false;
      if (handle !== null) caf(handle);
      handle = null;
    },
  };
}

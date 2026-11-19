/**
 * Offline rendering on superdough: a backend that never plays live, the IR wait and duck patches
 * for an `OfflineAudioContext`, and `renderOffline`, the bar-by-bar suspend/schedule/resume loop.
 * The pure planning helpers (hap selection, render window, channels, pauses) live in
 * `src/lib/render/plan.ts`.
 *
 * Everything that touches WebAudio or superdough's module state goes through `OfflineDeps`, so
 * `bun test` drives the loop with fakes. Nothing here creates an audio context or installs a
 * patch at import time: `defaultDeps` only reaches the WebAudio globals when its functions run.
 */

import {
  getSampleInfo,
  getSound,
  initAudio,
  loadBuffer,
  samples,
  setAudioContext,
  setSuperdoughAudioController,
  superdough,
} from 'superdough';
import { setNodePooling } from 'superdough/nodePools.mjs';
import { setAudioSeed } from 'superdough/rng.mjs';
import { Orbit, SuperdoughAudioController } from 'superdough/superdoughoutput.mjs';
import type { AudioBackend } from './backend';
import type { RenderError, RenderRequest } from '../render/contract';
import { eventsFromValues, type RenderEvent } from '../render/schedule';
import {
  bucketByBar,
  channelPlan,
  classifyError,
  eventSources,
  matchesMutation,
  orbitCreationOrder,
  pauseSchedule,
  selectHaps,
  type Selected,
  type WindowPlan,
} from '../render/plan';
import { errorMessage } from '../errors';

/** Polyphony for offline renders: explicit, since `initAudio` turns `undefined` into NaN. */
export const OFFLINE_MAX_POLYPHONY = 100_000;
const LATE_EPS_SEC = 1e-6;

// ---- backend

/** An `AudioBackend` whose clock never runs: `createRuntime.evaluate` evaluates without starting
 *  the scheduler, so the page can only `query`. */
export function createOfflineBackend(): AudioBackend {
  return {
    id: 'superdough',
    init: () => Promise.reject(new Error('offline backend: init must not be called')),
    resume: () => Promise.resolve(false),
    isRunning: () => false,
    getTime: () => 0,
    setSeed: () => {},
    output: () => {},
  };
}

// ---- IR tracker and offline patches

export interface IrTracker {
  install(): void;
  /** Waits for nested renders (and their `oncomplete`); returns how many are still pending. */
  settle(): Promise<number>;
  uninstall(): void;
  /** Nested renders not yet settled. */
  readonly pending: number;
}

export interface IrTrackerOptions {
  maxRounds?: number;
  timeoutMs?: number;
  yieldFn?: () => Promise<void>;
}

const macrotask = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** Tracks every `startRendering` on a context other than `getMain()`: superdough's nested IR
 *  renders (`reverbGen.mjs` `applyGradualLowpass`), whose `oncomplete` sets the convolver
 *  buffer. `proto` defaults to `OfflineAudioContext.prototype`, read only when called. */
export function createIrTracker(
  proto: { startRendering: (...args: any[]) => any } = (globalThis as any).OfflineAudioContext
    .prototype,
  getMain: () => unknown,
  opts: IrTrackerOptions = {},
): IrTracker {
  const maxRounds = opts.maxRounds ?? 20;
  const timeoutMs = opts.timeoutMs ?? 60_000;
  const yieldFn = opts.yieldFn ?? macrotask;
  const pending = new Set<Promise<void>>();
  let original: ((...args: any[]) => any) | null = null;

  return {
    install() {
      if (original) return;
      const orig = proto.startRendering;
      original = orig;
      proto.startRendering = function (this: unknown, ...args: any[]) {
        const p = orig.apply(this, args);
        if (this !== getMain()) {
          const tracked: Promise<void> = Promise.resolve(p).then(
            () => {},
            () => {},
          );
          pending.add(tracked);
          void tracked.then(() => pending.delete(tracked));
        }
        return p;
      };
    },
    async settle() {
      let timer: ReturnType<typeof setTimeout> | undefined;
      let timedOut = false;
      const timeout = new Promise<void>((r) => {
        timer = setTimeout(() => {
          timedOut = true;
          r();
        }, timeoutMs);
      });
      try {
        for (let i = 0; i < maxRounds && pending.size > 0 && !timedOut; i++) {
          await Promise.race([Promise.all([...pending]), timeout]);
          await yieldFn();
        }
        await yieldFn();
      } finally {
        clearTimeout(timer);
      }
      return pending.size;
    },
    uninstall() {
      if (!original) return;
      proto.startRendering = original;
      original = null;
    },
    get pending() {
      return pending.size;
    },
  };
}

interface GainParam {
  /** The intrinsic value: read once, before the first duck schedules anything. */
  value?: number;
  cancelAndHoldAtTime?: (t: number) => unknown;
  cancelScheduledValues(t: number): unknown;
  setValueAtTime(v: number, t: number): unknown;
  exponentialRampToValueAtTime(v: number, t: number): unknown;
}

/** One automation event `offlineDuck` scheduled: a held value, or the end of an exponential
 *  ramp from the event before it. */
export interface GainPoint {
  time: number;
  value: number;
  ramp: boolean;
}

/** The value of `initial` followed by `points` (ascending times) at time `x`, as WebAudio
 *  computes it: an exponential ramp runs from the previous event to its own; a ramp whose end
 *  is not after the previous event jumps to its value. */
export function gainAt(initial: number, points: readonly GainPoint[], x: number): number {
  let prev: GainPoint | null = null;
  for (const p of points) {
    if (p.time <= x) {
      prev = p;
      continue;
    }
    if (!p.ramp || prev === null) break;
    const span = p.time - prev.time;
    return prev.value * Math.pow(p.value / prev.value, (x - prev.time) / span);
  }
  return prev?.value ?? initial;
}

/** Per gain param, what `offlineDuck` has scheduled on it: the value `Orbit.duck` reads as
 *  `gainParam.value` lies in the future offline, so it is computed from this. */
const duckCurves = new WeakMap<object, { initial: number; points: GainPoint[] }>();

/** `Orbit.duck` scheduled directly at `t`. The original defers through `webAudioTimeout`, which
 *  lands late by a varying amount offline, and holds the gain it reads then; here the hold is at
 *  `t` and the held value is the scheduled curve's value at `t`. Otherwise the same curve:
 *  `clamp(1 - sqrt(depth), 0.01, held)`, then back to 1 over the attack. */
export function offlineDuck(
  this: { output: { gain: GainParam } },
  t: number,
  onset = 0,
  attack = 0.1,
  depth = 1,
): void {
  const g = this.output.gain;
  let curve = duckCurves.get(g);
  if (!curve) {
    curve = { initial: g.value ?? 1, points: [] };
    duckCurves.set(g, curve);
  }
  const held = gainAt(curve.initial, curve.points, t);
  const a = Math.max(attack, 0.002);
  const ducked = Math.min(Math.max(1 - Math.sqrt(depth), 0.01), held);
  if (g.cancelAndHoldAtTime) {
    g.cancelAndHoldAtTime(t);
  } else {
    // As the original does on browsers without cancelAndHoldAtTime.
    g.cancelScheduledValues(t);
    g.setValueAtTime(held, t);
  }
  g.exponentialRampToValueAtTime(ducked, t + onset);
  g.exponentialRampToValueAtTime(1, t + onset + a);
  curve.points = [
    ...curve.points.filter((p) => p.time < t),
    { time: t, value: held, ramp: false },
    { time: t + onset, value: ducked, ramp: true },
    { time: t + onset + a, value: 1, ramp: true },
  ];
}

/** Replaces `proto.duck` with `offlineDuck`; returns the restore function. */
export function installDuckPatch(proto: { duck: (...args: any[]) => any }): () => void {
  const original = proto.duck;
  proto.duck = offlineDuck;
  return () => {
    proto.duck = original;
  };
}

export interface OfflinePatches {
  /** The IR tracker's `settle`. */
  settle(): Promise<number>;
  /** Restores every patched prototype. */
  uninstall(): void;
}

/** The IR tracker (main context `ctx`) and the offline duck. Nothing else is patched. */
export function installOfflinePatches(
  ctx: unknown,
  contextProto: { startRendering: (...args: any[]) => any },
  orbitProto: { duck: (...args: any[]) => any },
  opts?: IrTrackerOptions,
): OfflinePatches {
  const tracker = createIrTracker(contextProto, () => ctx, opts);
  tracker.install();
  const restoreDuck = installDuckPatch(orbitProto);
  return {
    settle: () => tracker.settle(),
    uninstall() {
      tracker.uninstall();
      restoreDuck();
    },
  };
}

// ---- samples

/** Registers corpus samples (`{ stem: file }`) under `baseUrl` (ending in `/`), so `s("stem")`
 *  plays `baseUrl + file`; returns the full URLs. Registers sources only: nothing is fetched. */
export async function registerCorpusSamples(
  map: Record<string, string>,
  baseUrl: string,
  register: (sampleMap: Record<string, string[]>, baseUrl: string) => unknown = samples,
): Promise<string[]> {
  const entries = Object.entries(map);
  if (entries.length === 0) return [];
  if (!baseUrl.endsWith('/')) throw new Error(`sample base URL must end with /: ${baseUrl}`);
  await register(Object.fromEntries(entries.map(([k, f]) => [k, [f]])), baseUrl);
  return entries.map(([, f]) => baseUrl + f);
}

// ---- renderOffline

export interface RenderedBuffer {
  numberOfChannels: number;
  length: number;
  getChannelData(i: number): Float32Array;
}

/** The subset of `OfflineAudioContext` the loop drives. */
export interface OfflineContextLike {
  readonly currentTime: number;
  suspend(t: number): Promise<void>;
  resume(): Promise<void>;
  startRendering(): Promise<RenderedBuffer>;
}

export interface OrbitControllerLike {
  getOrbit(orbit: unknown, channels: number[]): unknown;
}

/** The URL superdough's sampler would load for this (unmutated) hap value, or null when it does
 *  not play a sample. Mirrors `superdough()` (`bank` prefix, `getSound`) and `getSampleBuffer`
 *  (`getSampleInfo`, no `resolveUrl`), so a preload fills the cache entry playback reads. */
export function sampleUrlOf(
  value: any,
  lookup: (s: string) => any = getSound,
  info: (value: any, bank: any) => { url: string } = getSampleInfo,
): string | null {
  if (value == null || typeof value !== 'object' || value.source) return null;
  let s = value.s;
  if (typeof s !== 'string' || s === '') return null;
  if (value.bank) s = `${value.bank}_${s}`;
  const sound = lookup(s);
  if (sound?.data?.type !== 'sample' || sound.data.samples == null) return null;
  const url = info({ ...value, s }, sound.data.samples).url;
  return typeof url === 'string' ? url : null;
}

/** The distinct sample URLs of `values`, in first-use order. */
export function sampleUrlsOf(values: any[], urlOf: (value: any) => string | null): string[] {
  const urls = new Set<string>();
  for (const v of values) {
    const url = urlOf(v);
    if (url !== null) urls.add(url);
  }
  return [...urls];
}

export interface OfflineDeps {
  createContext(channels: number, frames: number, sampleRate: number): OfflineContextLike;
  setContext(ctx: OfflineContextLike): void;
  createController(ctx: OfflineContextLike): OrbitControllerLike;
  setController(controller: OrbitControllerLike): void;
  initAudio(opts: { maxPolyphony: number; multiChannelOrbits: boolean }): Promise<void>;
  setSeed(seed: number): void;
  /** Turns superdough's node pool on or off; returns the previous setting. */
  setPooling(enabled: boolean): boolean;
  installPatches(ctx: OfflineContextLike): OfflinePatches;
  preloadSamples(ctx: OfflineContextLike, urls: string[]): Promise<void>;
  /** `sampleUrlOf` with superdough's sound map. */
  sampleUrl(value: any): string | null;
  superdough(value: any, t: number, hapDuration: number, cps: number, cycle: number): unknown;
  now(): number;
}

export const defaultDeps: OfflineDeps = {
  createContext: (channels, frames, sampleRate) =>
    new (globalThis as any).OfflineAudioContext(channels, frames, sampleRate),
  setContext: (ctx) => setAudioContext(ctx),
  createController: (ctx) => new SuperdoughAudioController(ctx),
  setController: (c) => setSuperdoughAudioController(c),
  initAudio: (opts) => initAudio(opts),
  setSeed: (seed) => setAudioSeed(seed),
  setPooling: (enabled) => setNodePooling(enabled),
  installPatches: (ctx) =>
    installOfflinePatches(ctx, (globalThis as any).OfflineAudioContext.prototype, Orbit.prototype),
  preloadSamples: async (ctx, urls) => {
    await Promise.all(urls.map((url) => loadBuffer(url, ctx)));
  },
  sampleUrl: (value) => sampleUrlOf(value),
  superdough: (value, t, hapDuration, cps, cycle) => superdough(value, t, hapDuration, cps, cycle),
  now: () => performance.now(),
};

export interface OfflineJob {
  /** The chunk's haps: `runtime.query(plan.c0, plan.c1, segments)`. */
  haps: any[];
  cps: number;
  seed: number;
  /** Supplies mode, sampleRate and mutation. */
  req: RenderRequest;
  plan: WindowPlan;
  /** `selectHaps` of the full schedule (`query(0, c1, segments)`); every orbit they use is
   *  created up front so a `duckorbit` target exists before its own first hap. Defaults to the
   *  chunk's own haps. */
  scheduleSelected?: Selected[];
  /** Told when the render moves on: preloading samples, then rendering. */
  onPhase?: (phase: 'loading-samples' | 'rendering') => void;
}

export interface OfflineOutput {
  buffer: RenderedBuffer;
  /** Pre-mutation schedule events of the selected haps, layout seconds, selected order. */
  events: RenderEvent[];
  errors: RenderError[];
  channels: number;
  stemPairs: number;
  /** Context creation to the end of the render. */
  renderMs: number;
}

const used = new WeakSet<OfflineDeps>();

/** Renders one chunk. Single use per deps set (one page per chunk with `defaultDeps`). */
export async function renderOffline(
  job: OfflineJob,
  deps: OfflineDeps = defaultDeps,
): Promise<OfflineOutput> {
  if (used.has(deps)) throw new Error('render page already used');
  used.add(deps);
  const { cps, req, plan } = job;
  const { c0, c1, frames } = plan;
  const sampleRate = req.sampleRate;
  const nBars = c1 - c0;
  const { suspendSec } = pauseSchedule(nBars, cps, sampleRate);

  // 1. select
  const { selected, errors } = selectHaps(job.haps);
  const layout = channelPlan(
    selected.map((s) => s.value),
    req.mode,
  );

  // 2. events (before mutation and before superdough mutates values)
  const events = eventsFromValues(eventSources(selected), cps);

  // 3. mutate
  const mutation = req.mutation;
  const dropped = new Set<number>();
  if (mutation) {
    for (const s of selected) if (matchesMutation(mutation, events[s.index]!)) dropped.add(s.index);
  }
  const buckets = bucketByBar(selected, c0, nBars, dropped);

  // 4. context
  const t0 = deps.now();
  const ctx = deps.createContext(layout.channels, frames, sampleRate);
  deps.setContext(ctx);
  const controller = deps.createController(ctx);
  deps.setController(controller);
  await deps.initAudio({
    maxPolyphony: OFFLINE_MAX_POLYPHONY,
    multiChannelOrbits: layout.multiChannelOrbits,
  });
  deps.setSeed(job.seed);
  const patches = deps.installPatches(ctx);
  // Pooled nodes come back from `ended` callbacks, whose timing against the render thread varies
  // run to run; fresh nodes for every voice keep two renders of one window bit-equal.
  const pooling = deps.setPooling(false);
  try {
    createOrbits(controller, job, selected, layout.multiChannelOrbits);

    // 5. samples: the URLs playback will load, so the preload fills superdough's cache
    job.onPhase?.('loading-samples');
    const played = buckets.flat().map((s) => s.value);
    try {
      await deps.preloadSamples(ctx, sampleUrlsOf(played, deps.sampleUrl));
    } catch (e) {
      errors.push(classifyError(`sample preload failed: ${errorMessage(e)}`));
    }
    job.onPhase?.('rendering');

    // 6. schedule and render, bar by bar
    const settleIR = async () => {
      if ((await patches.settle()) !== 0) errors.push(classifyError('IR renders did not settle'));
    };
    const scheduleBar = async (j: number) => {
      const calls: Promise<unknown>[] = [];
      const meta: { t: number; s: Selected }[] = [];
      for (const s of buckets[j]!) {
        let t = (s.onsetCycle - c0) / cps;
        const now = ctx.currentTime;
        if (t < now) {
          if (now - t > LATE_EPS_SEC) {
            errors.push(
              classifyError(`hap ${t}s scheduled late (now ${now})`, Math.floor(s.onsetCycle)),
            );
          }
          t = now;
        }
        meta.push({ t, s });
        try {
          calls.push(
            Promise.resolve(
              deps.superdough(s.value, t, Number(s.hap.duration) / cps, cps, s.onsetCycle),
            ),
          );
        } catch (e) {
          calls.push(Promise.reject(e));
        }
      }
      const results = await Promise.allSettled(calls);
      results.forEach((r, i) => {
        if (r.status !== 'rejected') return;
        const { t, s } = meta[i]!;
        errors.push(
          classifyError(
            `hap ${t.toFixed(3)}s ${s.value.s ?? ''}: ${errorMessage(r.reason)}`,
            Math.floor(s.onsetCycle),
          ),
        );
      });
    };

    await scheduleBar(0);
    await settleIR();
    let rendering: Promise<RenderedBuffer> | null = null;
    for (let j = 1; j < nBars; j++) {
      const at = suspendSec[j]!;
      const paused = ctx.suspend(at);
      if (rendering === null) {
        rendering = ctx.startRendering();
        rendering.catch(() => {}); // awaited below; keep an early failure from going unhandled
      } else {
        await ctx.resume();
      }
      await untilPaused(paused, rendering, at);
      await scheduleBar(j);
      await settleIR();
    }
    if (rendering === null) rendering = ctx.startRendering();
    else await ctx.resume();
    const buffer = await rendering;

    // 7. finish
    if (buffer.length !== frames || buffer.numberOfChannels !== layout.channels) {
      errors.push(
        classifyError(
          `rendered ${buffer.numberOfChannels}x${buffer.length}, expected ${layout.channels}x${frames}`,
        ),
      );
    }
    return {
      buffer,
      events,
      errors,
      channels: layout.channels,
      stemPairs: layout.pairs,
      renderMs: deps.now() - t0,
    };
  } finally {
    deps.setPooling(pooling);
    patches.uninstall();
  }
}

/** Waits for the suspend at `at`, or fails as soon as the render does (a rejected
 *  `startRendering` never reaches the suspend) or finishes without pausing there. */
async function untilPaused(
  paused: Promise<void>,
  rendering: Promise<RenderedBuffer>,
  at: number,
): Promise<void> {
  await Promise.race([
    paused,
    rendering.then(() => {
      throw new Error(`render finished before the pause at ${at.toFixed(3)}s`);
    }),
  ]);
}

/** Creates every orbit the full schedule uses, in time order, with the channels superdough
 *  would connect it to (00-module-map decision 14). */
function createOrbits(
  controller: OrbitControllerLike,
  job: OfflineJob,
  chunk: Selected[],
  multiChannelOrbits: boolean,
): void {
  orbitCreationOrder(chunk, job.scheduleSelected ?? [], multiChannelOrbits).forEach((o) =>
    controller.getOrbit(o.orbit, o.channels),
  );
}

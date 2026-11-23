import { describe, expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import {
  createIrTracker,
  createOfflineBackend,
  gainAt,
  installDuckPatch,
  installOfflinePatches,
  offlineDuck,
  OFFLINE_MAX_POLYPHONY,
  registerCorpusSamples,
  renderOffline,
  sampleUrlOf,
  sampleUrlsOf,
  type OfflineDeps,
  type OfflineJob,
  type RenderedBuffer,
} from '../src/lib/audio/offline';
import {
  channelPlan,
  classifyError,
  eventSources,
  LEAD_SEC,
  MAX_BUFFER_BYTES,
  matchesMutation,
  orbitChannels,
  routesToDefault,
  selectHaps,
  windowPlan,
} from '../src/lib/render/plan';
import { expectedFrames, type HapMutation, type RenderRequest } from '../src/lib/render/contract';
import { orbitPair } from '../src/lib/render/gates/stems';
import { eventsFromValues } from '../src/lib/render/schedule';
import { renderTiming } from '../src/lib/render/timing';
import { createRuntime } from '../src/lib/strudel/runtime';
import { buildStructure, cpsFor } from '../src/lib/strudel/structure';
import { LINEAR_SEGMENTS } from '../src/lib/strudel/transport';
import { ev } from './helpers/render-events';

await core.evalScope(core, mini);

const { Fraction, Hap, TimeSpan } = core as any;

/** A real Hap with whole [begin, end) and, unless given, part = whole. */
function hap(
  begin: number,
  end: number,
  value: unknown,
  opts: { partBegin?: number; context?: object; noWhole?: boolean } = {},
): any {
  const whole = opts.noWhole ? undefined : new TimeSpan(Fraction(begin), Fraction(end));
  const part = new TimeSpan(Fraction(opts.partBegin ?? begin), Fraction(end));
  return new Hap(whole, part, value, opts.context ?? {});
}

const req = (o: Partial<RenderRequest> = {}): RenderRequest => ({
  code: '',
  layout: 'linear',
  fromBar: 0,
  toBar: 2,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: 44100,
  ...o,
});

function offlineRuntime() {
  const timers = new Map<number, () => void>();
  let next = 0;
  const backend = createOfflineBackend();
  const outputs: unknown[] = [];
  let inits = 0;
  const wrapped = {
    ...backend,
    init: () => (inits++, backend.init()),
    output: (...args: Parameters<typeof backend.output>) => {
      outputs.push(args);
      return backend.output(...args);
    },
  };
  const rt = createRuntime({
    backends: { superdough: wrapped },
    prebake: async () => {},
    setInterval: (fn) => (timers.set(++next, fn), next),
    clearInterval: (id) => void timers.delete(id as number),
  });
  const tick = (n = 40) => {
    for (let i = 0; i < n; i++) [...timers.values()].forEach((f) => f());
  };
  return { rt, outputs, tick, inits: () => inits };
}

const snap = (rt: ReturnType<typeof createRuntime>) => {
  let s: any;
  rt.store.subscribe((v) => (s = v))();
  return s;
};

// ---- backend

describe('createOfflineBackend', () => {
  test('never runs', async () => {
    const b = createOfflineBackend();
    expect(b.id).toBe('superdough');
    expect(b.isRunning()).toBe(false);
    expect(await b.resume()).toBe(false);
    expect(b.getTime()).toBe(0);
    expect(() => b.setSeed(3)).not.toThrow();
    expect(() => b.output({} as any, 0, 0.5)).not.toThrow();
    await expect(b.init()).rejects.toThrow('offline backend: init must not be called');
  });

  test('a runtime on it evaluates and queries without starting', async () => {
    const { rt, outputs, tick, inits } = offlineRuntime();
    expect(await rt.evaluate('s("a")')).toBe(true);
    expect(snap(rt).started).toBe(false);
    tick();
    expect(inits()).toBe(0);
    expect(outputs).toEqual([]);
    const haps = rt.query(0, 2)!;
    expect(haps.length).toBe(2);
    expect(haps[0].value).toEqual({ s: 'a' });
  });
});

// ---- routing and selection

describe('routesToDefault', () => {
  const f = () => {};
  test('rules', () => {
    expect(routesToDefault({ context: {} })).toBe(true);
    expect(routesToDefault({ context: { onSchedule: f } })).toBe(true);
    expect(routesToDefault({ context: { onSchedule: f, dominantSchedule: true } })).toBe(false);
    expect(routesToDefault({ context: { dominantSchedule: true } })).toBe(true);
    expect(routesToDefault({})).toBe(true);
  });
});

describe('selectHaps', () => {
  test('drops continuations, whole-less and dominant-schedule haps', () => {
    const f = () => {};
    const { selected, errors } = selectHaps([
      hap(0, 1, { s: 'a' }),
      hap(0, 1, { s: 'cont' }, { partBegin: 0.5 }),
      hap(0, 1, { s: 'sig' }, { noWhole: true }),
      hap(0, 1, { s: 'dom' }, { context: { onSchedule: f, dominantSchedule: true } }),
      hap(0, 1, { s: 'both' }, { context: { onSchedule: f } }),
    ]);
    expect(selected.map((s) => s.value.s)).toEqual(['a', 'both']);
    expect(errors).toEqual([]);
  });

  test('sorts by onset, ties in query order, and numbers the result', () => {
    const { selected } = selectHaps([
      hap(1, 2, { s: 'c' }),
      hap(0, 1, { s: 'a' }),
      hap(1, 2, { s: 'd' }),
      hap(0, 0.5, { s: 'b' }),
    ]);
    expect(selected.map((s) => s.value.s)).toEqual(['a', 'b', 'c', 'd']);
    expect(selected.map((s) => s.index)).toEqual([0, 1, 2, 3]);
    expect(selected.map((s) => s.onsetCycle)).toEqual([0, 0, 1, 1]);
  });

  test('a non-object value is an error and is not selected (the fork ensureObjectValue throws)', () => {
    const { selected, errors } = selectHaps([hap(2.5, 3, 'x'), hap(0, 1, { s: 'a' })]);
    expect(selected.map((s) => s.value)).toEqual([{ s: 'a' }]);
    expect(errors).toHaveLength(1);
    expect(errors[0]!.kind).toBe('hap');
    expect(errors[0]!.bar).toBe(2);
    expect(errors[0]!.message).toContain('expected hap.value to be an object');
  });

  test('a cps control is an unsupported error with its bar', () => {
    const { selected, errors } = selectHaps([hap(3.25, 4, { s: 'a', cps: 1 })]);
    expect(selected).toHaveLength(1);
    expect(errors).toEqual([
      {
        kind: 'other',
        message: 'unsupported: hap sets cps (control cps is not supported by the harness)',
        bar: 3,
      },
    ]);
  });

  test('real haps of an overlapping stack', async () => {
    const { rt } = offlineRuntime();
    expect(await rt.evaluate('stack(s("a b"), s("c*4"), note("e").s("x").late(0.25))')).toBe(true);
    const { selected } = selectHaps(rt.query(0, 2)!);
    const onsets = selected.map((s) => s.onsetCycle);
    expect(onsets).toEqual([...onsets].sort((a, b) => a - b));
    // 2 + 4 per cycle, plus `e` from cycle 0.25 and 1.25 (the one from -0.75 is a continuation).
    expect(selected).toHaveLength(14);
    expect(selected.slice(0, 2).map((s) => s.value.s)).toEqual(['a', 'c']);
    expect(selected.filter((s) => s.value.s === 'x').map((s) => s.onsetCycle)).toEqual([
      0.25, 1.25,
    ]);
  });
});

describe('eventSources and schedule events', () => {
  test('cycles in, layout seconds out, selected order', () => {
    const cps = 0.5;
    const { selected } = selectHaps([hap(21, 21.5, { s: 'b' }), hap(20, 22, { s: 'a', note: 60 })]);
    const src = eventSources(selected);
    expect(src).toEqual([
      { value: { s: 'a', note: 60 }, wholeBeginCycles: 20, wholeDurCycles: 2 },
      { value: { s: 'b' }, wholeBeginCycles: 21, wholeDurCycles: 0.5 },
    ]);
    const events = eventsFromValues(src, cps);
    expect(events.map((e) => e.tSec)).toEqual([40, 42]);
    expect(events.map((e) => e.durSec)).toEqual([4, 1]);
    expect(events.map((e) => e.bar)).toEqual([20, 21]);
    expect(events.map((e) => e.s)).toEqual(selected.map((s) => s.value.s));
  });
});

// ---- window plan

describe('windowPlan', () => {
  const cps = 0.5;
  test('whole-bar lead-in; an exact multiple does not round up', () => {
    const p = windowPlan(req({ fromBar: 8, toBar: 10 }), cps, 4 / cps);
    expect(p.c0).toBe(4);
    expect(p.c1).toBe(10);
    expect(p.lookbackSecUsed).toBe(8);
    expect(p.bufferStartSec).toBe(8);
    expect(p.windowStartFrame).toBe(8 * 44100);
  });
  test('a partial bar of lookback rounds up to a whole bar', () => {
    const p = windowPlan(req({ fromBar: 8, toBar: 10 }), cps, 2.5);
    expect(p.c0).toBe(6);
    expect(p.lookbackSecUsed).toBe(4);
  });
  test('clamped at bar 0', () => {
    const p = windowPlan(req({ fromBar: 1, toBar: 3 }), cps, 100);
    expect(p.c0).toBe(0);
    expect(p.lookbackSecUsed).toBe(2);
    const z = windowPlan(req({ fromBar: 0, toBar: 3 }), cps, 100);
    expect(z.c0).toBe(0);
    expect(z.lookbackSecUsed).toBe(0);
    expect(z.windowStartFrame).toBe(0);
  });
  test('lookback 0 starts at fromBar', () => {
    const p = windowPlan(req({ fromBar: 5, toBar: 7 }), cps, 0);
    expect(p.c0).toBe(5);
    expect(p.lookbackSecUsed).toBe(0);
    expect(p.frames).toBe(2 * 2 * 44100);
  });
  test('frames and windowStartFrame at 137 bpm 7/8', () => {
    const c = cpsFor(137, { num: 7, den: 8 });
    const r = req({ fromBar: 5, toBar: 9, overlapBars: 1 });
    const p = windowPlan(r, c, 1.3);
    expect(p.c0).toBe(4);
    expect(p.lookbackSecUsed).toBeCloseTo(1 / c, 12);
    expect(p.frames).toBe(expectedFrames(r, c, p.lookbackSecUsed));
    expect(p.windowStartFrame).toBe(Math.round(p.lookbackSecUsed * 44100));
    expect(p.bufferStartSec).toBe(4 / c);
  });
  test('overlap extends frames and c1', () => {
    const a = windowPlan(req({ fromBar: 2, toBar: 4 }), cps, 0);
    const b = windowPlan(req({ fromBar: 2, toBar: 4, overlapBars: 2 }), cps, 0);
    expect(b.c1).toBe(6);
    expect(b.frames - a.frames).toBe(2 * 2 * 44100);
  });
  test('clip layout bars plan the same way', () => {
    expect(windowPlan(req({ layout: 'clip', fromBar: 3, toBar: 5 }), cps, 2)).toEqual(
      windowPlan(req({ fromBar: 3, toBar: 5 }), cps, 2),
    );
  });
  test('rejects an empty window, an oversized buffer and bad inputs', () => {
    expect(() => windowPlan(req({ fromBar: 2, toBar: 2 }), cps, 0)).toThrow('empty render window');
    // stems at 32 channels: > 1e9 bytes needs > 7.8M frames (177 s).
    expect(() => windowPlan(req({ mode: 'stems', toBar: 100 }), cps, 0)).toThrow(
      'render buffer too large',
    );
    expect(windowPlan(req({ mode: 'mix', toBar: 100 }), cps, 0).frames * 2 * 4).toBeLessThan(
      MAX_BUFFER_BYTES,
    );
    expect(() => windowPlan(req(), 0, 0)).toThrow('invalid cps');
    expect(() => windowPlan(req(), cps, -1)).toThrow('invalid lookback');
  });
});

// ---- errors

describe('classifyError', () => {
  test('kind rules in order', () => {
    expect(classifyError('hap 1.000s bd: sample not found').kind).toBe('hap');
    expect(classifyError('IR renders did not settle').kind).toBe('ir');
    expect(classifyError('could not load AudioWorklet sample').kind).toBe('worklet');
    expect(classifyError('ladder-processor crashed').kind).toBe('worklet');
    expect(classifyError('sound bd not found! Is it loaded?').kind).toBe('sample');
    expect(classifyError('sample decode failed').kind).toBe('sample');
    expect(classifyError('something else').kind).toBe('other');
  });
  test('bar copied when given, cut to 300 characters', () => {
    expect(classifyError('x', 4)).toEqual({ kind: 'other', message: 'x', bar: 4 });
    expect('bar' in classifyError('x')).toBe(false);
    expect(classifyError('y'.repeat(500)).message).toHaveLength(300);
  });
});

// ---- channels

describe('channelPlan', () => {
  const vals = (...orbits: (number | undefined)[]) => orbits.map((orbit) => ({ s: 'a', orbit }));
  test('mix is one stereo pair', () => {
    expect(channelPlan(vals(1, 2, 9), 'mix')).toEqual({
      pairs: 1,
      channels: 2,
      multiChannelOrbits: false,
    });
  });
  test('stems: a pair per orbit up to the highest', () => {
    expect(channelPlan(vals(1), 'stems')).toEqual({
      pairs: 1,
      channels: 2,
      multiChannelOrbits: true,
    });
    expect(channelPlan(vals(undefined), 'stems').channels).toBe(2);
    expect(channelPlan(vals(1, 2, 5), 'stems').channels).toBe(10);
    expect(channelPlan(vals(0, 1), 'stems').channels).toBe(2);
    expect(channelPlan([], 'stems').channels).toBe(2);
    expect(channelPlan([{ orbit: 'nope' }], 'stems').channels).toBe(2);
  });
  test('orbit 20 clamps to 16 pairs; superdough wraps it onto orbitPair', () => {
    const plan = channelPlan(vals(20), 'stems');
    expect(plan.channels).toBe(32);
    const [l] = orbitChannels({ orbit: 20 }, true);
    expect(Math.floor((l! % plan.channels) / 2)).toBe(orbitPair(20, plan.pairs));
  });
  test('explicit channels', () => {
    expect(channelPlan([{ orbit: 1, channels: [5, 6] }], 'stems').channels).toBe(6);
    expect(channelPlan([{ channels: 3 }], 'stems').channels).toBe(4);
  });
});

describe('orbitChannels', () => {
  test('mapping as superdough', () => {
    expect(orbitChannels({}, false)).toEqual([0, 1]);
    expect(orbitChannels({ orbit: 3 }, false)).toEqual([0, 1]);
    expect(orbitChannels({ orbit: 3 }, true)).toEqual([4, 5]);
    expect(orbitChannels({ orbit: 0 }, true)).toEqual([0, 1]);
    expect(orbitChannels({}, true)).toEqual([0, 1]);
    expect(orbitChannels({ orbit: 3, channels: [5, 6] }, true)).toEqual([4, 5]);
  });
});

// ---- mutations

describe('matchesMutation', () => {
  test('dropOrbit on orbit and bar', () => {
    const m: HapMutation = { kind: 'dropOrbit', orbit: 2, bars: [3, 4] };
    expect(matchesMutation(m, ev({ orbit: 2, bar: 3 }))).toBe(true);
    expect(matchesMutation(m, ev({ orbit: 1, bar: 3 }))).toBe(false);
    expect(matchesMutation(m, ev({ orbit: 2, bar: 5 }))).toBe(false);
  });
  test('dropNotesBelowHz with and without bars', () => {
    const all: HapMutation = { kind: 'dropNotesBelowHz', hz: 100 };
    expect(matchesMutation(all, ev({ freq: 50, bar: 9 }))).toBe(true);
    expect(matchesMutation(all, ev({ freq: 100 }))).toBe(false);
    expect(matchesMutation(all, ev({ freq: null }))).toBe(false);
    const some: HapMutation = { kind: 'dropNotesBelowHz', hz: 100, bars: [1] };
    expect(matchesMutation(some, ev({ freq: 50, bar: 1 }))).toBe(true);
    expect(matchesMutation(some, ev({ freq: 50, bar: 2 }))).toBe(false);
  });
});

// ---- IR tracker

const deferred = <T = void>() => {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
};
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe('createIrTracker', () => {
  function proto() {
    const results: ReturnType<typeof deferred<string>>[] = [];
    const p = {
      startRendering(this: unknown) {
        const d = deferred<string>();
        results.push(d);
        return d.promise;
      },
    };
    return { p, results, original: p.startRendering };
  }

  test('the main context is not tracked; others are, until they settle', async () => {
    const { p, results } = proto();
    const main = {};
    let yields = 0;
    const t = createIrTracker(p, () => main, {
      yieldFn: async () => {
        yields++;
      },
    });
    t.install();
    void p.startRendering.call(main);
    expect(t.pending).toBe(0);
    const nested = p.startRendering.call({});
    expect(t.pending).toBe(1);
    const settled = t.settle();
    results[1]!.resolve('ir');
    expect(await settled).toBe(0);
    expect(await nested).toBe('ir');
    // One yield after the round's await, one more at the end.
    expect(yields).toBe(2);
  });

  test('a rejected nested render counts as settled', async () => {
    const { p, results } = proto();
    const t = createIrTracker(p, () => null, { yieldFn: async () => {} });
    t.install();
    p.startRendering.call({}).catch(() => {});
    results[0]!.reject(new Error('x'));
    expect(await t.settle()).toBe(0);
  });

  test('waits for a chain: settling one starts another', async () => {
    const { p, results } = proto();
    const t = createIrTracker(p, () => null);
    t.install();
    void p.startRendering.call({}).then(() => {
      // like oncomplete regenerating: starts the next one
      void p.startRendering.call({});
    });
    const settled = t.settle();
    results[0]!.resolve('a');
    await tick();
    expect(results).toHaveLength(2);
    results[1]!.resolve('b');
    expect(await settled).toBe(0);
  });

  test('returns the count left after the timeout', async () => {
    const { p } = proto();
    // Never settles: the round bound cannot help, so the timeout must.
    const t = createIrTracker(p, () => null, { maxRounds: 3, timeoutMs: 10 });
    t.install();
    void p.startRendering.call({});
    void p.startRendering.call({});
    expect(await t.settle()).toBe(2);
    t.uninstall();
  });

  test('maxRounds bounds a chain that never ends', async () => {
    const { p, results } = proto();
    const t = createIrTracker(p, () => null, { maxRounds: 2, yieldFn: tick });
    t.install();
    const chain = () =>
      void p.startRendering.call({}).then(() => {
        chain();
      });
    chain();
    const settled = t.settle();
    // Resolve whichever render is newest, as fast as they come.
    const pump = setInterval(() => results.at(-1)!.resolve('x'), 1);
    const left = await settled;
    clearInterval(pump);
    expect(left).toBeGreaterThan(0);
    t.uninstall();
  });

  test('uninstall restores the original; install is idempotent', () => {
    const { p, original } = proto();
    const t = createIrTracker(p, () => null);
    t.install();
    const wrapped = p.startRendering;
    t.install();
    expect(p.startRendering).toBe(wrapped);
    expect(p.startRendering).not.toBe(original);
    t.uninstall();
    expect(p.startRendering).toBe(original);
    t.uninstall();
    expect(p.startRendering).toBe(original);
  });

  test('the default yield is a macrotask, so oncomplete-style callbacks have run', async () => {
    const { p, results } = proto();
    const t = createIrTracker(p, () => null);
    t.install();
    let assigned = false;
    void p.startRendering.call({}).then(() => setTimeout(() => (assigned = true), 0));
    const settled = t.settle();
    results[0]!.resolve('x');
    await settled;
    expect(assigned).toBe(true);
    t.uninstall();
  });
});

// ---- duck patch

function recordingOrbit(withHold = true) {
  const calls: [string, ...number[]][] = [];
  const gain: any = {
    value: 1,
    cancelScheduledValues: (t: number) => calls.push(['cancel', t]),
    setValueAtTime: (v: number, t: number) => calls.push(['set', v, t]),
    exponentialRampToValueAtTime: (v: number, t: number) => calls.push(['ramp', v, t]),
  };
  if (withHold) gain.cancelAndHoldAtTime = (t: number) => calls.push(['hold', t]);
  return { orbit: { output: { gain } }, calls };
}

describe('offline duck', () => {
  test('schedules at t: hold, ramp to ducked, ramp back over the attack', () => {
    const { orbit, calls } = recordingOrbit();
    offlineDuck.call(orbit, 2, 0.1, 0.2, 0.64);
    expect(calls).toEqual([
      ['hold', 2],
      ['ramp', 1 - 0.8, 2.1],
      ['ramp', 1, 2.1 + 0.2],
    ]);
  });
  test('defaults, the minimum attack and the 0.01 floor', () => {
    const { orbit, calls } = recordingOrbit();
    offlineDuck.call(orbit, 1, undefined, 0);
    expect(calls).toEqual([
      ['hold', 1],
      ['ramp', 0.01, 1],
      ['ramp', 1, 1.002],
    ]);
  });
  test('falls back to cancelScheduledValues plus setValueAtTime, as the original does', () => {
    const { orbit, calls } = recordingOrbit(false);
    offlineDuck.call(orbit, 3, 0, 0.1, 0.25);
    expect(calls[0]).toEqual(['cancel', 3]);
    expect(calls[1]).toEqual(['set', 1, 3]);
    expect(calls[2]).toEqual(['ramp', 0.5, 3]);
  });
  test('a duck inside an earlier recovery is clamped to the held gain (Orbit.duck clamp)', () => {
    const { orbit, calls } = recordingOrbit(false);
    // First duck: hold 1 at t=0, to 0.2 at 0.1, back to 1 at 0.3.
    offlineDuck.call(orbit, 0, 0.1, 0.2, 0.64);
    calls.length = 0;
    // At t=0.15 the recovery is at 0.2 * 5^(0.05 / 0.2); depth 0.25 wants 0.5, above that.
    const held = 0.2 * Math.pow(5, 0.25);
    offlineDuck.call(orbit, 0.15, 0, 0.1, 0.25);
    expect(calls[1]![0]).toBe('set');
    expect(calls[1]![1]).toBeCloseTo(held, 12);
    expect(calls[2]![1]).toBeCloseTo(held, 12);
    expect(calls[3]).toEqual(['ramp', 1, 0.15 + 0.1]);
  });
  test('a duck after the recovery ends is not clamped', () => {
    const { orbit, calls } = recordingOrbit();
    offlineDuck.call(orbit, 0, 0, 0.2, 0.64);
    calls.length = 0;
    offlineDuck.call(orbit, 1, 0, 0.2, 0.64);
    expect(calls[1]).toEqual(['ramp', 1 - 0.8, 1]);
  });
  test('gainAt follows holds, exponential ramps and jumps', () => {
    const pts = [
      { time: 1, value: 1, ramp: false },
      { time: 1, value: 0.25, ramp: true },
      { time: 3, value: 1, ramp: true },
    ];
    expect(gainAt(0.5, pts, 0)).toBe(0.5);
    expect(gainAt(0.5, pts, 1)).toBe(0.25);
    expect(gainAt(0.5, pts, 2)).toBeCloseTo(0.5, 12);
    expect(gainAt(0.5, pts, 5)).toBe(1);
    expect(gainAt(1, [], 7)).toBe(1);
  });
  test('installDuckPatch replaces and restores', () => {
    const original = () => {};
    const proto = { duck: original as (...a: any[]) => any };
    const restore = installDuckPatch(proto);
    expect(proto.duck).toBe(offlineDuck);
    restore();
    expect(proto.duck).toBe(original);
  });
  test('installOfflinePatches installs both and one call restores both', async () => {
    const start = () => Promise.resolve();
    const ctxProto = { startRendering: start as (...a: any[]) => any };
    const duck = () => {};
    const orbitProto = { duck: duck as (...a: any[]) => any };
    const patches = installOfflinePatches({}, ctxProto, orbitProto, { yieldFn: async () => {} });
    expect(ctxProto.startRendering).not.toBe(start);
    expect(orbitProto.duck).toBe(offlineDuck);
    expect(await patches.settle()).toBe(0);
    patches.uninstall();
    expect(ctxProto.startRendering).toBe(start);
    expect(orbitProto.duck).toBe(duck);
  });
});

// ---- samples

describe('registerCorpusSamples', () => {
  test('registers one-file banks under the base URL and returns the URLs', async () => {
    const calls: unknown[] = [];
    const urls = await registerCorpusSamples(
      { kick: 'kick.wav', hat: 'hat.wav' },
      'http://127.0.0.1:1/samples/',
      (m, b) => calls.push([m, b]),
    );
    expect(urls).toEqual([
      'http://127.0.0.1:1/samples/kick.wav',
      'http://127.0.0.1:1/samples/hat.wav',
    ]);
    expect(calls).toEqual([
      [{ kick: ['kick.wav'], hat: ['hat.wav'] }, 'http://127.0.0.1:1/samples/'],
    ]);
  });
  test('an empty map registers nothing', async () => {
    let n = 0;
    expect(await registerCorpusSamples({}, 'x', () => n++)).toEqual([]);
    expect(n).toBe(0);
  });
  test('the base URL must end with /', async () => {
    await expect(registerCorpusSamples({ a: 'a.wav' }, '/samples', () => {})).rejects.toThrow(
      'must end with /',
    );
  });
});

describe('sample URLs', () => {
  const sounds: Record<string, any> = {
    kick: { data: { type: 'sample', samples: ['/samples/kick.wav'] } },
    tr909_bd: { data: { type: 'sample', samples: ['/x/bd0.wav', '/x/bd1.wav'] } },
    sine: { data: { type: 'synth' } },
  };
  const lookup = (s: string) => sounds[s.toLowerCase()];

  test('the URL the sampler loads: bank prefix, n index, synths and sources skipped', () => {
    expect(sampleUrlOf({ s: 'kick' }, lookup)).toBe('/samples/kick.wav');
    expect(sampleUrlOf({ s: 'bd', bank: 'tr909', n: 1 }, lookup)).toBe('/x/bd1.wav');
    expect(sampleUrlOf({ s: 'sine' }, lookup)).toBeNull();
    expect(sampleUrlOf({ s: 'nope' }, lookup)).toBeNull();
    expect(sampleUrlOf({ note: 40 }, lookup)).toBeNull();
    expect(sampleUrlOf({ s: 'kick', source: () => {} }, lookup)).toBeNull();
  });

  test('with superdough: the URL registered by samples(map, base), exactly', async () => {
    const { samples } = await import('superdough');
    await samples({ rh_preload_probe: 'probe.wav' }, '/samples/');
    expect(sampleUrlOf({ s: 'rh_preload_probe' })).toBe('/samples/probe.wav');
  });

  test('sampleUrlsOf dedupes in first-use order and skips nulls', () => {
    const urlOf = (v: any) => (v.s === 'x' ? null : `u/${v.s}`);
    expect(sampleUrlsOf([{ s: 'b' }, { s: 'x' }, { s: 'a' }, { s: 'b' }], urlOf)).toEqual([
      'u/b',
      'u/a',
    ]);
  });
});

// ---- renderOffline with a fake OfflineAudioContext

interface FakeOpts {
  /** Seconds the fake overshoots each pause (a late render thread). */
  overshoot?: number;
  /** Rendered buffer shape override. */
  bufferLength?: number;
  superdough?: (value: any, t: number) => unknown;
  settleLeft?: number;
  preload?: () => Promise<void>;
  /** startRendering rejects with this (after a macrotask) instead of rendering. */
  renderRejects?: Error;
  /** The node pooling setting before the render (default on). */
  pooling?: boolean;
}

function fakeDeps(o: FakeOpts = {}) {
  const log: string[] = [];
  const sd: { value: any; t: number; dur: number; cps: number; cycle: number; now: number }[] = [];
  const orbits: [unknown, number[]][] = [];
  const suspends: number[] = [];
  let ctx: any;
  let clock = 0;

  function createContext(channels: number, frames: number, sampleRate: number) {
    const pauses = new Map<number, () => void>();
    let finish!: (b: RenderedBuffer) => void;
    const length = o.bufferLength ?? frames;
    const buffer: RenderedBuffer = {
      numberOfChannels: channels,
      length,
      getChannelData: () => new Float32Array(length),
    };
    const advance = () => {
      const next = [...pauses.keys()].filter((t) => t >= ctx.currentTime).sort((a, b) => a - b)[0];
      if (next !== undefined) {
        const resolve = pauses.get(next)!;
        pauses.delete(next);
        ctx.currentTime = next + (o.overshoot ?? 0);
        log.push(`paused ${next}`);
        resolve();
      } else {
        ctx.currentTime = frames / sampleRate;
        log.push('finished');
        finish(buffer);
      }
    };
    ctx = {
      currentTime: 0,
      channels,
      frames,
      sampleRate,
      suspend(t: number) {
        suspends.push(t);
        log.push(`suspend ${t}`);
        return new Promise<void>((r) => pauses.set(t, r));
      },
      startRendering() {
        log.push('start');
        if (o.renderRejects) {
          const err = o.renderRejects;
          return new Promise<RenderedBuffer>((_, reject) => setTimeout(() => reject(err), 0));
        }
        const p = new Promise<RenderedBuffer>((r) => (finish = r));
        setTimeout(advance, 0);
        return p;
      },
      resume() {
        log.push('resume');
        setTimeout(advance, 0);
        return Promise.resolve();
      },
    };
    log.push(`context ${channels}x${frames}@${sampleRate}`);
    return ctx;
  }

  let pooling = o.pooling ?? true;
  const deps: OfflineDeps = {
    createContext,
    setContext: (c) => log.push(c === ctx ? 'setContext' : 'setContext ?'),
    createController: () => ({
      getOrbit: (n, ch) => {
        orbits.push([n, ch]);
        log.push(`orbit ${n}`);
      },
    }),
    setController: () => log.push('setController'),
    initAudio: async (opts) => {
      log.push(`initAudio ${opts.maxPolyphony} ${opts.multiChannelOrbits}`);
    },
    setSeed: (s) => log.push(`seed ${s}`),
    setPooling: (enabled) => {
      log.push(`pooling ${enabled}`);
      const previous = pooling;
      pooling = enabled;
      return previous;
    },
    installPatches: () => {
      log.push('install');
      return {
        settle: async () => {
          log.push('settle');
          return o.settleLeft ?? 0;
        },
        uninstall: () => log.push('uninstall'),
      };
    },
    preloadSamples: async (_c, urls) => {
      log.push(`preload ${urls.join(',')}`);
      await o.preload?.();
    },
    sampleUrl: (v) => (typeof v.s === 'string' && v.s.startsWith('b0') ? `u/${v.s}.wav` : null),
    superdough: (value, t, dur, cps, cycle) => {
      sd.push({ value, t, dur, cps, cycle, now: ctx.currentTime });
      log.push(`sd ${value.s}`);
      return o.superdough ? o.superdough(value, t) : Promise.resolve();
    },
    now: () => (clock += 10),
  };
  return { deps, log, sd, orbits, suspends, ctx: () => ctx, pooling: () => pooling };
}

const CPS = 0.5; // 2 s bars
const BAR = 1 / CPS;
const LEAD = Math.min(LEAD_SEC, BAR / 4);

function job(
  o: Partial<RenderRequest> & {
    haps: any[];
    lookbackSec?: number;
    scheduleHaps?: any[];
    onPhase?: OfflineJob['onPhase'];
  },
): OfflineJob {
  const { haps, scheduleHaps, onPhase, ...r } = o;
  const request = req({ lookbackSec: 0, ...r });
  return {
    haps,
    cps: CPS,
    seed: 7,
    req: request,
    plan: windowPlan(request, CPS, request.lookbackSec as number),
    scheduleSelected: scheduleHaps ? selectHaps(scheduleHaps).selected : undefined,
    onPhase,
  };
}

/** Haps for bars [from, to): `s` names b<bar>.<k>, one at the bar line, one half a bar in. */
const barHaps = (from: number, to: number, extra: (bar: number) => object = () => ({})) =>
  Array.from({ length: to - from }, (_, i) => from + i).flatMap((bar) => [
    hap(bar, bar + 0.5, { s: `b${bar}.0`, ...extra(bar) }),
    hap(bar + 0.5, bar + 1, { s: `b${bar}.1`, ...extra(bar) }),
  ]);

describe('renderOffline: loop mechanics', () => {
  test('setup order, one settle per pause, nBars - 1 suspends', async () => {
    const f = fakeDeps();
    const out = await renderOffline(job({ toBar: 3, haps: barHaps(0, 3) }), f.deps);
    const i = (s: string) => f.log.indexOf(s);
    expect(f.log[0]).toBe(`context 2x${3 * BAR * 44100}@44100`);
    expect(i('setContext')).toBeLessThan(i('setController'));
    expect(i('setController')).toBeLessThan(i(`initAudio ${OFFLINE_MAX_POLYPHONY} false`));
    expect(i(`initAudio ${OFFLINE_MAX_POLYPHONY} false`)).toBeLessThan(i('seed 7'));
    expect(i('seed 7')).toBeLessThan(i('install'));
    expect(i('install')).toBeLessThan(i('pooling false'));
    expect(i('pooling false')).toBeLessThan(i('orbit 1'));
    const preload = 'preload u/b0.0.wav,u/b0.1.wav';
    expect(i('orbit 1')).toBeLessThan(i(preload));
    expect(i(preload)).toBeLessThan(f.log.findIndex((l) => l.startsWith('sd ')));
    expect(f.suspends).toEqual([BAR - LEAD, 2 * BAR - LEAD]);
    expect(f.log.filter((l) => l === 'settle')).toHaveLength(3);
    expect(i('settle')).toBeLessThan(i('start'));
    expect(f.log.filter((l) => l === 'start')).toHaveLength(1);
    expect(f.log.filter((l) => l === 'resume')).toHaveLength(2);
    expect(f.log.at(-1)).toBe('uninstall');
    expect(out.errors).toEqual([]);
    expect(out.channels).toBe(2);
    expect(out.stemPairs).toBe(1);
    expect(out.buffer.length).toBe(3 * BAR * 44100);
    expect(out.renderMs).toBe(10);
  });

  test('one bar never suspends', async () => {
    const f = fakeDeps();
    const out = await renderOffline(job({ toBar: 1, haps: barHaps(0, 1) }), f.deps);
    expect(f.suspends).toEqual([]);
    expect(f.log.filter((l) => l === 'settle')).toHaveLength(1);
    expect(f.log.indexOf('settle')).toBeLessThan(f.log.indexOf('start'));
    expect(f.sd).toHaveLength(2);
    expect(out.errors).toEqual([]);
  });

  test('stems: multiChannelOrbits and the channel count', async () => {
    const f = fakeDeps();
    const haps = [hap(0, 1, { s: 'a', orbit: 1 }), hap(0, 1, { s: 'b', orbit: 3 })];
    const out = await renderOffline(job({ toBar: 1, mode: 'stems', haps }), f.deps);
    expect(f.log).toContain(`initAudio ${OFFLINE_MAX_POLYPHONY} true`);
    expect(f.log[0]).toBe(`context 6x${BAR * 44100}@44100`);
    expect(out.channels).toBe(6);
    expect(out.stemPairs).toBe(3);
    expect(f.orbits).toEqual([
      [1, [0, 1]],
      [3, [4, 5]],
    ]);
  });

  test('a second call on the same deps throws', async () => {
    const f = fakeDeps();
    await renderOffline(job({ toBar: 1, haps: [] }), f.deps);
    await expect(renderOffline(job({ toBar: 1, haps: [] }), f.deps)).rejects.toThrow(
      'render page already used',
    );
  });

  test('a rejecting or throwing superdough adds one hap error each; the render continues', async () => {
    const f = fakeDeps({
      superdough: (v) => {
        if (v.s === 'b0.1') return Promise.reject(new Error('sound b0.1 not found! Is it loaded?'));
        if (v.s === 'b1.0') throw new Error('sync boom');
        return undefined;
      },
    });
    const out = await renderOffline(job({ toBar: 2, haps: barHaps(0, 2) }), f.deps);
    expect(f.sd).toHaveLength(4);
    expect(out.errors).toEqual([
      { kind: 'hap', message: 'hap 1.000s b0.1: sound b0.1 not found! Is it loaded?', bar: 0 },
      { kind: 'hap', message: 'hap 2.000s b1.0: sync boom', bar: 1 },
    ]);
    expect(f.log).toContain('finished');
  });

  test('the short-bar guard throws before any context', async () => {
    const f = fakeDeps();
    const fast = 44100 / 256 / 0.25 + 1; // barSec / 4 < 256 frames
    const request = req({ toBar: 2 });
    const j: OfflineJob = {
      haps: [],
      cps: fast,
      seed: 0,
      req: request,
      plan: windowPlan(request, fast, 0),
    };
    await expect(renderOffline(j, f.deps)).rejects.toThrow('bar too short for the pause lead');
    expect(f.log).toEqual([]);
  });

  test('a buffer of the wrong length is an error', async () => {
    const f = fakeDeps({ bufferLength: 10 });
    const out = await renderOffline(job({ toBar: 1, haps: [] }), f.deps);
    expect(out.errors).toHaveLength(1);
    expect(out.errors[0]!.kind).toBe('other');
    expect(out.errors[0]!.message).toContain('expected 2x');
  });

  test('unsettled IR renders add an ir error per pause', async () => {
    const f = fakeDeps({ settleLeft: 1 });
    const out = await renderOffline(job({ toBar: 2, haps: [] }), f.deps);
    expect(out.errors).toEqual([
      { kind: 'ir', message: 'IR renders did not settle' },
      { kind: 'ir', message: 'IR renders did not settle' },
    ]);
  });

  test('a failed preload is a sample error, not a throw', async () => {
    const f = fakeDeps({ preload: () => Promise.reject(new Error('404')) });
    const out = await renderOffline(job({ toBar: 1, haps: [] }), f.deps);
    expect(out.errors).toEqual([{ kind: 'sample', message: 'sample preload failed: 404' }]);
  });

  test('patches are uninstalled when the render throws', async () => {
    const f = fakeDeps();
    f.deps.createController = () => ({
      getOrbit: () => {
        throw new Error('boom');
      },
    });
    await expect(renderOffline(job({ toBar: 1, haps: barHaps(0, 1) }), f.deps)).rejects.toThrow(
      'boom',
    );
    expect(f.log.at(-1)).toBe('uninstall');
  });

  test('node pooling is off for the render and restored after it', async () => {
    const f = fakeDeps();
    await renderOffline(job({ toBar: 2, haps: barHaps(0, 2) }), f.deps);
    const i = (s: string) => f.log.indexOf(s);
    expect(i('pooling false')).toBeLessThan(f.log.findIndex((l) => l.startsWith('sd ')));
    expect(f.log.slice(-2)).toEqual(['pooling true', 'uninstall']);
    expect(f.pooling()).toBe(true);
  });

  test('a render that started with pooling off leaves it off', async () => {
    const f = fakeDeps({ pooling: false });
    await renderOffline(job({ toBar: 2, haps: barHaps(0, 2) }), f.deps);
    expect(f.pooling()).toBe(false);
  });

  test('a failed render restores node pooling', async () => {
    const f = fakeDeps({ renderRejects: new Error('render boom') });
    await expect(renderOffline(job({ toBar: 3, haps: barHaps(0, 3) }), f.deps)).rejects.toThrow(
      'render boom',
    );
    expect(f.pooling()).toBe(true);
    expect(f.log.at(-2)).toBe('pooling true');
  });

  test('a rejecting startRendering fails the render at once, not at the next pause', async () => {
    const f = fakeDeps({ renderRejects: new Error('render boom') });
    await expect(renderOffline(job({ toBar: 3, haps: barHaps(0, 3) }), f.deps)).rejects.toThrow(
      'render boom',
    );
    expect(f.log.filter((l) => l.startsWith('sd b1'))).toEqual([]);
    expect(f.log.at(-1)).toBe('uninstall');
  });

  test('a render that finishes before a pause is an error', async () => {
    const f = fakeDeps();
    const j = job({ toBar: 2, haps: [] });
    const orig = f.deps.createContext;
    f.deps.createContext = (...a) => {
      const c = orig(...a);
      c.suspend = () => new Promise<void>(() => {}); // the pause never happens
      return c;
    };
    await expect(renderOffline(j, f.deps)).rejects.toThrow('render finished before the pause');
  });

  test('onPhase: loading-samples before the preload, rendering after it', async () => {
    const f = fakeDeps();
    await renderOffline(
      job({ toBar: 2, haps: barHaps(0, 2), onPhase: (p) => f.log.push(`phase ${p}`) }),
      f.deps,
    );
    const i = (s: string) => f.log.findIndex((l) => l.startsWith(s));
    expect(i('phase loading-samples')).toBeLessThan(i('preload '));
    expect(i('preload ')).toBeLessThan(i('phase rendering'));
    expect(i('phase rendering')).toBeLessThan(i('sd '));
  });

  test('preloads only the samples of haps that play, once each', async () => {
    const f = fakeDeps();
    const haps = [
      hap(0, 0.5, { s: 'b0.x', orbit: 2 }),
      hap(0.5, 1, { s: 'b0.x', orbit: 2 }),
      hap(0.5, 1, { s: 'b0.y', orbit: 1 }),
      hap(0.5, 1, { s: 'synth', orbit: 1 }),
    ];
    await renderOffline(
      job({ toBar: 1, haps, mutation: { kind: 'dropOrbit', orbit: 1, bars: [0] } }),
      f.deps,
    );
    expect(f.log).toContain('preload u/b0.x.wav');
  });

  test('selection errors are carried into the output', async () => {
    const f = fakeDeps();
    const out = await renderOffline(
      job({ toBar: 1, haps: [hap(0, 1, { s: 'a', cps: 2 })] }),
      f.deps,
    );
    expect(out.errors.map((e) => e.kind)).toEqual(['other']);
  });
});

describe('renderOffline: orbits created up front', () => {
  test('orbits only in the full schedule are created first, in time order', async () => {
    const f = fakeDeps();
    const chunk = [hap(4, 5, { s: 'kick', orbit: 1, duckorbit: 2 })];
    const full = [
      hap(0, 1, { s: 'pad', orbit: 3 }),
      hap(1, 2, { s: 'pad', orbit: 2 }),
      hap(6, 7, { s: 'late', orbit: 5 }),
      ...chunk.map((h) => hap(4, 5, { ...h.value })),
    ];
    await renderOffline(
      job({ fromBar: 4, toBar: 5, mode: 'stems', haps: chunk, scheduleHaps: full }),
      f.deps,
    );
    expect(f.orbits.map(([n]) => n)).toEqual([3, 2, 1, 5]);
    expect(f.orbits[0]).toEqual([3, [4, 5]]);
    const firstSd = f.log.findIndex((l) => l.startsWith('sd '));
    expect(f.log.lastIndexOf('orbit 5')).toBeLessThan(firstSd);
  });

  test('default orbit 1 and mix channels; without scheduleSelected the chunk orbits only', async () => {
    const f = fakeDeps();
    await renderOffline(
      job({ toBar: 1, haps: [hap(0, 1, { s: 'a' }), hap(0, 1, { s: 'b', orbit: 4 })] }),
      f.deps,
    );
    expect(f.orbits).toEqual([
      [1, [0, 1]],
      [4, [0, 1]],
    ]);
  });
});

describe('renderOffline: mutations', () => {
  test('a dropped hap is absent from superdough calls but present in events', async () => {
    const f = fakeDeps();
    const haps = [
      hap(0, 1, { s: 'a', orbit: 1 }),
      hap(0, 1, { s: 'b', orbit: 2 }),
      hap(1, 2, { s: 'c', orbit: 2 }),
    ];
    const out = await renderOffline(
      job({ toBar: 2, haps, mutation: { kind: 'dropOrbit', orbit: 2, bars: [0] } }),
      f.deps,
    );
    expect(f.sd.map((c) => c.value.s)).toEqual(['a', 'c']);
    expect(out.events.map((e) => e.s)).toEqual(['a', 'b', 'c']);
  });

  test('dropNotesBelowHz drops low pitched notes only', async () => {
    const f = fakeDeps();
    const haps = [
      hap(0, 1, { s: 'sine', note: 24 }),
      hap(0, 1, { s: 'sine', note: 72 }),
      hap(0, 1, { s: 'bd' }),
    ];
    await renderOffline(
      job({ toBar: 1, haps, mutation: { kind: 'dropNotesBelowHz', hz: 100 } }),
      f.deps,
    );
    expect(f.sd.map((c) => c.value.note ?? c.value.s)).toEqual([72, 'bd']);
  });

  test('no mutation drops nothing; events follow selected order', async () => {
    const f = fakeDeps();
    const haps = barHaps(0, 2).reverse();
    const out = await renderOffline(job({ toBar: 2, haps }), f.deps);
    expect(f.sd).toHaveLength(4);
    expect(out.events.map((e) => e.s)).toEqual(['b0.0', 'b0.1', 'b1.0', 'b1.1']);
  });
});

describe('renderOffline: cycle and time arguments', () => {
  test('buffer-relative t, duration / cps, cps and the layout onset cycle', async () => {
    const f = fakeDeps();
    const haps = [hap(20, 20.5, { s: 'a' }), hap(21.25, 22, { s: 'b' })];
    const out = await renderOffline(job({ fromBar: 20, toBar: 22, haps }), f.deps);
    expect(f.sd.map(({ t, dur, cps, cycle }) => [t, dur, cps, cycle])).toEqual([
      [0, 0.5 / CPS, CPS, 20],
      [1.25 / CPS, 0.75 / CPS, CPS, 21.25],
    ]);
    expect(f.sd[0]!.value).toBe(haps[0].value);
    expect(out.events.map((e) => e.tSec)).toEqual([20 / CPS, 21.25 / CPS]);
  });

  test('with lookback, t counts from c0', async () => {
    const f = fakeDeps();
    const haps = [hap(3, 4, { s: 'a' }), hap(4, 5, { s: 'b' })];
    await renderOffline(job({ fromBar: 4, toBar: 5, lookbackSec: BAR, haps }), f.deps);
    expect(f.sd.map((c) => c.t)).toEqual([0, BAR]);
  });

  test('haps outside the window are not scheduled', async () => {
    const f = fakeDeps();
    const haps = [hap(1, 2, { s: 'in' }), hap(5, 6, { s: 'out' })];
    await renderOffline(job({ fromBar: 1, toBar: 2, haps }), f.deps);
    expect(f.sd.map((c) => c.value.s)).toEqual(['in']);
  });
});

describe('renderOffline: order and t bounds', () => {
  test('mixed-meter render schedules with both timeline seconds and local CPS', async () => {
    const structure = buildStructure({
      sections: [{ name: 'body', bars: 3 }],
      meterChanges: [{ bar: 2, meter: '3/8' }],
    });
    const timing = renderTiming(structure, LINEAR_SEGMENTS, structure.cps)!;
    const request = req({ fromBar: 1, toBar: 3, layout: 'linear' });
    const f = fakeDeps();
    const calls: unknown[][] = [];
    f.deps.superdough = (_value, ...time) => calls.push(time);
    const result = await renderOffline(
      {
        haps: [hap(0, 2, { s: 'sine' }), hap(1, 2, { s: 'sine' }), hap(2, 3, { s: 'sine' })],
        cps: structure.cps,
        seed: 0,
        req: request,
        plan: windowPlan(request, structure.cps, 0.25, 0, timing),
        timing,
      },
      f.deps,
    );
    expect(calls).toEqual([
      [0, 2.75, 0.5, 0, 0],
      [2, 0.75, 4 / 3, 1, 2],
      [2.75, 0.75, 4 / 3, 2, 2.75],
    ]);
    expect(result.buffer.length).toBe(154350);
    expect(f.suspends).toEqual([1.95, 2.7]);
  });
  test('calls ascend; each bar is scheduled at its pause; t is never in the past', async () => {
    const f = fakeDeps();
    await renderOffline(job({ toBar: 3, haps: barHaps(0, 3).reverse() }), f.deps);
    const ts = f.sd.map((c) => c.t);
    expect(ts).toEqual([...ts].sort((a, b) => a - b));
    for (const c of f.sd) {
      const bar = Math.floor(c.cycle);
      expect(c.now).toBe(bar === 0 ? 0 : bar * BAR - LEAD);
      expect(c.t).toBeGreaterThanOrEqual(c.now);
    }
  });

  test('a hap behind the clock is clamped and reported late', async () => {
    const f = fakeDeps({ overshoot: 0.1 }); // the fake runs 0.05 s past the bar line
    const out = await renderOffline(job({ toBar: 2, haps: barHaps(0, 2) }), f.deps);
    const late = f.sd.find((c) => c.value.s === 'b1.0')!;
    expect(late.t).toBe(late.now);
    expect(late.now).toBeCloseTo(BAR + 0.05, 12);
    expect(out.errors).toHaveLength(1);
    expect(out.errors[0]!.kind).toBe('hap');
    expect(out.errors[0]!.bar).toBe(1);
    expect(out.errors[0]!.message).toMatch(/^hap 2s scheduled late \(now 2\.05/);
    // the half-bar hap is still ahead of the clock
    expect(f.sd.find((c) => c.value.s === 'b1.1')!.t).toBe(1.5 * BAR);
  });

  test('a gap within 1e-6 s clamps silently', async () => {
    const f = fakeDeps({ overshoot: LEAD + 5e-7 });
    const out = await renderOffline(job({ toBar: 2, haps: barHaps(1, 2) }), f.deps);
    expect(out.errors).toEqual([]);
    const c = f.sd[0]!;
    expect(c.t).toBe(c.now);
  });
});

describe('module load', () => {
  test('importing offline.ts creates no audio context and patches nothing', async () => {
    expect((globalThis as any).OfflineAudioContext).toBeUndefined();
    const mod = await import('../src/lib/audio/offline');
    expect(typeof mod.defaultDeps.createContext).toBe('function');
  });
});

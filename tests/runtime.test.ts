import { expect, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import { createRuntime, HISTORY_CYCLES } from '../src/lib/strudel/runtime';
import { song } from '../src/lib/strudel/song';
import { bpmFromCps, buildStructure, cpsFor, DEFAULT_METER } from '../src/lib/strudel/structure';
import { clipSegments, LINEAR_SEGMENTS, type TransportState } from '../src/lib/strudel/transport';
import { fakeRuntime as fake } from './helpers/fake-runtime';

await core.evalScope(core, mini, { song });

const snap = (rt: ReturnType<typeof createRuntime>) => {
  let s: any;
  rt.store.subscribe((v) => (s = v))();
  return s;
};
const settle = () => new Promise((r) => setTimeout(r, 20));

test('delivers haps then stops', async () => {
  const { b, rt, tick } = fake();
  expect(await rt.evaluate('note("c e g").s("sine")')).toBe(true);
  expect(snap(rt).started).toBe(true);
  tick();
  await settle();
  expect(b.calls.length).toBeGreaterThan(0);
  expect(b.calls[0][0].value.note).toBe('c');
  rt.stop();
  expect(snap(rt).started).toBe(false);
  const n = b.calls.length;
  tick();
  await settle();
  expect(b.calls.length).toBe(n);
});
test('onSchedule delivers t as the absolute target time, spaced by the hap onsets over cps', async () => {
  const { b, rt, tick } = fake();
  expect(await rt.evaluate('setcps(0.5); note("c e g").s("sine")')).toBe(true);
  tick();
  await settle();
  rt.stop();
  expect(b.calls.length).toBeGreaterThanOrEqual(3);
  const [a, c] = [b.calls[0], b.calls[1]];
  expect(typeof a[1]).toBe('number');
  expect(c[1]).toBeGreaterThan(a[1]);
  const onsets = (c[0].whole.begin.valueOf() - a[0].whole.begin.valueOf()) / 0.5;
  expect(c[1] - a[1]).toBeCloseTo(onsets);
  // Latency is included: the first hap is due after the clock time it was scheduled at.
  expect(a[1]).toBeGreaterThan(0);
});
test('eval error does not throw', async () => {
  const { rt } = fake();
  expect(await rt.evaluate('s("bd").nonexistent()')).toBe(false);
  expect(snap(rt).evalError).toContain('nonexistent');
  expect(await rt.evaluate('note("c")')).toBe(true);
  expect(snap(rt).evalError).toBe(null);
  expect(snap(rt).activeCode).toContain('note');
});
test('empty code rejected', async () => {
  const { rt } = fake();
  expect(await rt.evaluate('  \n')).toBe(false);
  expect(snap(rt).evalError).toContain('empty');
});
test('suspended audio: evaluates, does not start', async () => {
  const { rt } = fake({ running: false });
  expect(await rt.evaluate('note("c")')).toBe(true);
  expect(snap(rt).started).toBe(false);
  expect(snap(rt).audio).toBe('suspended');
});
test('output errors surface (unknown sound)', async () => {
  const { rt, tick } = fake({ failOutput: true });
  await rt.evaluate('note("c").s("nope")');
  tick();
  await settle();
  expect(snap(rt).outputError).toContain('not found');
});
test('a code setcpm sets cps; bpm reads it through the meter', async () => {
  const { rt } = fake();
  await rt.evaluate('setcpm(120/4)\nnote("c")');
  expect(snap(rt).cps).toBe(0.5);
  expect(bpmFromCps(snap(rt).cps, snap(rt).meter)).toBe(120);
  await rt.evaluate('setcpm(140/4)\nnote("c")');
  expect(bpmFromCps(snap(rt).cps, snap(rt).meter)).toBeCloseTo(140);
});
test('concurrent resumeAudio calls share one attempt', async () => {
  const { rt, b } = fake();
  await Promise.all([rt.resumeAudio(), rt.resumeAudio(), rt.resumeAudio()]);
  expect(b.resumes).toBe(1);
});
test('a repeating output error notifies subscribers once', async () => {
  const { rt, tick } = fake({ failOutput: true });
  await rt.evaluate('note("c c c c").s("nope")');
  let n = 0;
  const unsub = rt.store.subscribe(() => n++);
  tick();
  tick();
  await settle();
  unsub();
  expect(n).toBe(2); // initial call + the first error only
});
test('a resume() that never settles does not hang evaluate', async () => {
  const { rt } = fake({ hang: true });
  expect(await rt.evaluate('note("c")')).toBe(true);
  expect(snap(rt).started).toBe(false);
  expect(snap(rt).audio).toBe('suspended');
});

test('resumeAudio is a cheap no-op once audio runs', async () => {
  const { rt, b } = fake();
  await rt.resumeAudio();
  await rt.resumeAudio();
  expect(b.resumes).toBe(1);
  expect(snap(rt).audio).toBe('running');
});
test('a failed prebake is retried by the next evaluate', async () => {
  let attempts = 0;
  const { rt } = fake({
    prebake: async () => {
      if (++attempts === 1) throw new Error('prebake boom');
    },
  });
  expect(await rt.evaluate('note("c")')).toBe(false);
  expect(snap(rt).evalError).toContain('prebake boom');
  expect(await rt.evaluate('note("c")')).toBe(true);
  expect(attempts).toBe(2);
});
test('stop() pressed while audio is starting prevents autostart', async () => {
  let release!: (v: boolean) => void;
  const { rt } = fake({ resume: () => new Promise<boolean>((r) => (release = r)) });
  const pending = rt.evaluate('note("c")');
  rt.stop();
  release(true);
  expect(await pending).toBe(true);
  expect(snap(rt).started).toBe(false);
});
test('init failure after resume leaves audio suspended', async () => {
  const { rt } = fake({
    init: async () => {
      throw new Error('worklets failed');
    },
  });
  expect(await rt.resumeAudio()).toBe(false);
  expect(snap(rt).audio).toBe('suspended');
  expect(snap(rt).outputError).toContain('worklets failed');
});
test('a throwing afterEval hook does not turn a good evaluate into an error', async () => {
  const { rt } = fake({
    hooks: {
      afterEval: () => {
        throw new Error('widget boom');
      },
    },
  });
  expect(await rt.evaluate('note("c")')).toBe(true);
  expect(snap(rt).evalError).toBe(null);
  expect(snap(rt).started).toBe(true);
});

// ---------------------------------------------------------------- tempo

const seen = () => ((globalThis as any).seenCps ??= []) as number[];
/** Code that records the cps it sees while it runs. */
const recordCps = (tail = 'note("c")') => `seenCps.push(getCps());\n${tail}`;

test('stopped: cps from the options is applied before the code runs, and the meter recorded', async () => {
  seen().length = 0;
  const { rt } = fake();
  const meter = { num: 6, den: 8 };
  expect(await rt.evaluate(recordCps(), { cps: cpsFor(120, meter), meter })).toBe(true);
  // Nothing played yet, so the first start already runs at the new tempo.
  expect(seen()).toEqual([cpsFor(120, meter)]);
  expect(snap(rt).cps).toBe(cpsFor(120, meter));
  expect(snap(rt).meter).toEqual(meter);
  expect(snap(rt).tempoOverride).toBe(false);
  expect(bpmFromCps(snap(rt).cps, snap(rt).meter)).toBeCloseTo(120, 9);
});

test('no options: the default 0.5 cps and 4/4, never the previous tempo', async () => {
  const { rt } = fake();
  expect(snap(rt).cps).toBe(0.5);
  expect(snap(rt).meter).toEqual(DEFAULT_METER);
  await rt.evaluate('note("c")', { cps: 1.25, meter: { num: 3, den: 4 } });
  expect(snap(rt).cps).toBe(1.25);
  await rt.evaluate('note("c")');
  expect(snap(rt).cps).toBe(0.5);
  expect(snap(rt).meter).toEqual(DEFAULT_METER);
});

test('playing: meta cps is applied together with the new pattern, after the code ran', async () => {
  seen().length = 0;
  const after: number[] = [];
  const { rt } = fake({ hooks: { afterEval: () => void after.push(rt.scheduler.cps) } });
  await rt.evaluate('note("c")', { cps: 1 });
  expect(snap(rt).started).toBe(true);
  expect(await rt.evaluate(recordCps(), { cps: 2 })).toBe(true);
  expect(seen()).toEqual([1]); // the code ran while the old pattern played at the old tempo
  expect(after).toEqual([1, 2]);
  expect(rt.scheduler.cps).toBe(2);
  expect(snap(rt).cps).toBe(2);
  expect(snap(rt).tempoOverride).toBe(false);
});

test('playing: the old pattern keeps the old cps while an async evaluate is in flight', async () => {
  const { rt } = fake();
  await rt.evaluate('note("c")', { cps: 1 });
  let release!: () => void;
  (globalThis as any).evalGate = new Promise<void>((r) => (release = r));
  const run = rt.evaluate('await evalGate\nnote("d")', { cps: 2 });
  await settle();
  expect(rt.scheduler.cps).toBe(1);
  release();
  expect(await run).toBe(true);
  expect(rt.scheduler.cps).toBe(2);
});

test('a code setcpm wins and sets tempoOverride; the next evaluate without it clears it', async () => {
  const { rt } = fake();
  await rt.evaluate('setcpm(30)\nnote("c")', { cps: 1 });
  expect(snap(rt).cps).toBe(0.5);
  expect(snap(rt).tempoOverride).toBe(true);
  await rt.evaluate('note("c")', { cps: 1 });
  expect(snap(rt).cps).toBe(1);
  expect(snap(rt).tempoOverride).toBe(false);
  // Playing: the code's tempo still wins over meta's.
  await rt.evaluate('setcpm(90)\nnote("c")', { cps: 0.5 });
  expect(rt.scheduler.cps).toBe(1.5);
  expect(snap(rt).tempoOverride).toBe(true);
});

test('a setcps equal to meta is no override', async () => {
  const { rt } = fake();
  await rt.evaluate('setcps(0.75)\nnote("c")', { cps: 0.75 });
  expect(snap(rt).tempoOverride).toBe(false);
  // Playing at 0.75: a code setcps to exactly the new meta value.
  await rt.evaluate('setcps(1.5)\nnote("c")', { cps: 1.5 });
  expect(rt.scheduler.cps).toBe(1.5);
  expect(snap(rt).tempoOverride).toBe(false);
});

test('a failed evaluate leaves the tempo and tempoOverride as they were', async () => {
  const { rt } = fake();
  await rt.evaluate('setcpm(30)\nnote("c")', { cps: 1 });
  expect(await rt.evaluate('note("c").nonexistent()', { cps: 2 })).toBe(false);
  expect(rt.scheduler.cps).toBe(0.5); // the playing pattern keeps its tempo
  expect(snap(rt).cps).toBe(0.5);
  expect(snap(rt).tempoOverride).toBe(true);
  expect(snap(rt).meter).toEqual(DEFAULT_METER);
  // A code setcps before the failure is undone too.
  expect(await rt.evaluate('setcps(3)\nnote("c").nonexistent()', { cps: 2 })).toBe(false);
  expect(rt.scheduler.cps).toBe(0.5);
});

test('stopped: a failed evaluate leaves the tempo as it was', async () => {
  const { rt } = fake({ running: false });
  expect(await rt.evaluate('note("c").nonexistent()', { cps: 2 })).toBe(false);
  expect(rt.scheduler.cps).toBe(0.5);
});

test('a failed prebake does not change the tempo', async () => {
  let attempts = 0;
  const { rt } = fake({
    prebake: async () => {
      if (++attempts === 1) throw new Error('prebake boom');
    },
  });
  expect(await rt.evaluate('note("c")', { cps: 2 })).toBe(false);
  expect(rt.scheduler.cps).toBe(0.5);
});

test('concurrent evaluates run in order, each with its own cps', async () => {
  const after: [string, number][] = [];
  const { rt } = fake({
    hooks: { afterEval: ({ code }) => void after.push([code, rt.scheduler.cps]) },
  });
  const a = rt.evaluate('note("a")', { cps: 1 });
  const b = rt.evaluate('note("b")', { cps: 0.25, meter: { num: 7, den: 8 } });
  expect(await Promise.all([a, b])).toEqual([true, true]);
  expect(after).toEqual([
    ['note("a")', 1],
    ['note("b")', 0.25],
  ]);
  expect(snap(rt).meter).toEqual({ num: 7, den: 8 });
});

test('an evaluate queued behind a failing one still runs', async () => {
  const { rt } = fake();
  const a = rt.evaluate('note("c").nonexistent()', { cps: 2 });
  const b = rt.evaluate('note("c")', { cps: 0.75 });
  expect(await a).toBe(false);
  expect(await b).toBe(true);
  expect(snap(rt).cps).toBe(0.75);
});

// ---------------------------------------------------------------- structure and transport

// intro 2 bars (song bars 0-1), body 4 looping (2-5), tail 2 (6-7).
const META = {
  bpm: 120,
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
};
const STRUCTURE = buildStructure(META);
/** A JS literal with single-quoted strings (the transpiler turns double quotes into mini). */
const literal = (v: unknown) => JSON.stringify(v).replace(/"/g, "'");
const SONG = `const meta = ${literal(META)};
$: song(meta, { intro: s("<i0 i1>"), body: s("<b0 b1 b2 b3>"), tail: s("<t0 t1>") })
`;
const STRAIGHT_META = {
  sections: [
    { name: 'a', bars: 1 },
    { name: 'b', bars: 1 },
  ],
};
const STRAIGHT = `const meta = ${literal(STRAIGHT_META)};
$: song(meta, { a: s("a"), b: s("b") })
`;

const tstate = (rt: ReturnType<typeof createRuntime>): TransportState => {
  let s!: TransportState;
  rt.transport.store.subscribe((v) => (s = v))();
  return s;
};

/** A started runtime playing SONG, with the scheduler clock under the test's control. */
async function playing(opts: Parameters<typeof fake>[0] = {}) {
  let pattern: any;
  const f = fake({ ...opts, hooks: { afterEval: (a) => void (pattern = a.pattern) } });
  expect(await f.rt.evaluate(SONG)).toBe(true);
  expect(snap(f.rt).started).toBe(true);
  let now = 0;
  f.rt.scheduler.now = () => now;
  /** Place the playhead at `cycle` with the scheduler having queried up to `lastEnd`. */
  const at = (cycle: number, lastEnd = cycle) => {
    now = cycle;
    f.rt.scheduler.lastEnd = lastEnd;
  };
  /** The song bar at a global cycle, read through the pattern (what sounds). */
  const sounds = (cycle: number): string[] =>
    pattern
      .queryArc(cycle, cycle + 1)
      .filter((h: any) => h.hasOnset())
      .map((h: any) => h.value.s);
  const bar = (cycle: number) => {
    at(cycle);
    return f.rt.position().located?.songBar ?? null;
  };
  return { ...f, at, sounds, bar };
}

test('structure comes from song(), and is null without it', async () => {
  const { rt } = fake();
  expect(snap(rt).structure).toBeNull();
  await rt.evaluate(SONG);
  expect(snap(rt).structure).toEqual(STRUCTURE);
  await rt.evaluate('note("c")');
  expect(snap(rt).structure).toBeNull();
});

test('a failing song() keeps the previous structure', async () => {
  const { rt } = fake();
  await rt.evaluate(SONG);
  expect(await rt.evaluate('$: song({ sections: [] }, {})')).toBe(false);
  expect(snap(rt).evalError).toContain('song:');
  expect(snap(rt).structure).toEqual(STRUCTURE);
});

test('the initial transport state: song view, nothing armed', () => {
  const { rt } = fake();
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
  expect(rt.position()).toEqual({ cycle: 0, located: null });
});

test('stopped startFrom arms a start; the evaluate that starts keeps it', async () => {
  const { rt } = fake();
  rt.transport.startFrom(2, STRUCTURE);
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: true, armedSection: 2 });
  await rt.evaluate(SONG);
  expect(snap(rt).started).toBe(true);
  expect(tstate(rt).pendingStart).toBe(false);
  expect(rt.position().located).toMatchObject({ section: 2, songBar: 6 });
});

test('stopped loopSection arms a loop; loopSection(null) disarms it', () => {
  const { rt } = fake();
  rt.transport.loopSection(0, STRUCTURE);
  expect(tstate(rt)).toEqual({
    view: { kind: 'loop', section: 0 },
    pendingStart: true,
    armedSection: 0,
  });
  rt.transport.loopSection(null, STRUCTURE);
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
});

test('stopped: an armed change replaces the previous one; out of range is a no-op', () => {
  const { rt } = fake();
  rt.transport.loopSection(1, STRUCTURE);
  rt.transport.startFrom(9, STRUCTURE);
  rt.transport.loopSection(9, STRUCTURE);
  expect(tstate(rt)).toEqual({
    view: { kind: 'loop', section: 1 },
    pendingStart: true,
    armedSection: 1,
  });
  rt.transport.startFrom(0, STRUCTURE); // the default list again
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
});

test('armedSection names the armed section and notifies when only it changes', () => {
  const { rt } = fake();
  const seen: (number | null)[] = [];
  const unsubscribe = rt.transport.store.subscribe((v) => void seen.push(v.armedSection));
  rt.transport.startFrom(1, STRUCTURE);
  rt.transport.startFrom(2, STRUCTURE); // same view and pendingStart: still a new state
  rt.transport.startFrom(2, STRUCTURE); // unchanged: no notification
  rt.transport.loopSection(1, STRUCTURE);
  rt.transport.loopSection(null, STRUCTURE);
  unsubscribe();
  expect(seen).toEqual([null, 1, 2, 1, null]);
});

test('armedSection clears when the armed start plays and on Stop', async () => {
  const { rt } = fake();
  rt.transport.loopSection(1, STRUCTURE);
  expect(tstate(rt).armedSection).toBe(1);
  await rt.evaluate(SONG);
  expect(snap(rt).started).toBe(true);
  expect(tstate(rt).armedSection).toBeNull();
  rt.stop();
  rt.transport.startFrom(2, STRUCTURE);
  expect(tstate(rt).armedSection).toBe(2);
  rt.stop();
  expect(tstate(rt).armedSection).toBeNull();
});

test('stopped playEnding is a no-op', () => {
  const { rt } = fake();
  rt.transport.playEnding();
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
});

test('an armed start survives an evaluate that does not start (suspended audio)', async () => {
  const { rt } = fake({ running: false });
  rt.transport.startFrom(1, STRUCTURE);
  expect(await rt.evaluate(SONG)).toBe(true);
  expect(snap(rt).started).toBe(false);
  expect(tstate(rt).pendingStart).toBe(true);
  expect(tstate(rt).armedSection).toBe(1);
});

test('playing: the song plays intro once, then loops the body', async () => {
  const { sounds, bar } = await playing();
  expect([0, 1, 2, 5, 6, 9, 10].map(bar)).toEqual([0, 1, 2, 5, 2, 5, 2]);
  expect(sounds(0)).toEqual(['i0']);
  expect(sounds(6)).toEqual(['b0']);
});

test('playing loopSection appends at the next bar after what the scheduler queried', async () => {
  const { rt, at, bar, sounds } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE);
  expect(tstate(rt)).toEqual({
    view: { kind: 'loop', section: 0 },
    pendingStart: false,
    armedSection: null,
  });
  expect(sounds(3)).toEqual(['b1']);
  expect(sounds(4)).toEqual(['i0']);
  expect([3, 4, 5, 6, 7].map(bar)).toEqual([3, 0, 1, 0, 1]);
});

test('a replaced segment is kept for look-behind queries, then pruned', async () => {
  const { rt, at, sounds } = await playing();
  at(7.2, 7.4);
  rt.transport.loopSection(0, STRUCTURE); // from cycle 8
  at(10);
  rt.position(); // prunes, as every frame does
  expect(sounds(7)).toEqual(['b1']); // painters still see what played at 7
  expect(sounds(10)).toEqual(['i0']);
  at(7 + 1 + HISTORY_CYCLES + 1);
  rt.position();
  // Long gone: the first remaining segment is extended backwards.
  expect(sounds(7)).toEqual(['i1']);
});

test('playing loopSection(null) returns to the song from the bar playing at the boundary', async () => {
  const { rt, at, bar } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE); // bars 0,1,0,1… from cycle 4
  at(6.5, 6.6);
  rt.transport.loopSection(null, STRUCTURE); // at 7 the loop plays bar 1: the song goes on from it
  expect(tstate(rt).view).toEqual({ kind: 'song' });
  expect([7, 8, 9].map(bar)).toEqual([1, 2, 3]);
});

test('playing startFrom jumps at the next bar boundary (quantized to lastEnd, not now)', async () => {
  const { rt, at, bar } = await playing();
  at(1.1, 2.3); // the scheduler has already queried into cycle 2
  rt.transport.startFrom(2, STRUCTURE);
  expect([1, 2, 3, 4].map(bar)).toEqual([1, 2, 6, 7]);
  expect(bar(5)).toBeNull(); // the tail plays once, then the song is over
});

test('playing: the argument structure is ignored, the evaluated one is used', async () => {
  const { rt, at, bar } = await playing();
  const other = buildStructure({
    sections: [
      { name: 'x', bars: 3 },
      { name: 'y', bars: 5 },
    ],
  });
  at(0.2, 0.5);
  rt.transport.startFrom(1, other); // section 1 of the evaluated song is the body, at bar 2
  expect(bar(1)).toBe(2);
});

test('playing: an index out of range for the evaluated structure is a no-op', async () => {
  const { rt, at, bar } = await playing();
  const big = buildStructure({
    sections: Array.from({ length: 5 }, (_, i) => ({ name: `s${i}`, bars: 1 })),
  });
  at(0.2, 0.5);
  rt.transport.startFrom(4, big);
  rt.transport.loopSection(4, big);
  expect(tstate(rt).view).toEqual({ kind: 'song' });
  expect([1, 2].map(bar)).toEqual([1, 2]);
});

test('playEnding releases at the next loop pass and plays the tail once', async () => {
  const { rt, at, bar } = await playing();
  at(3.2, 3.4); // body bar 1 of 4; the pass ends at cycle 6
  rt.transport.playEnding();
  expect(tstate(rt).view).toEqual({ kind: 'release' });
  expect([5, 6, 7, 8].map(bar)).toEqual([5, 6, 7, null]);
});

test('playEnding is a no-op in the intro, when already released, and without song()', async () => {
  const { rt, at, bar } = await playing();
  at(0.5, 0.6);
  rt.transport.playEnding();
  expect(tstate(rt).view).toEqual({ kind: 'song' });
  at(2.5, 2.6);
  rt.transport.playEnding(); // release at 6
  at(3.5, 3.6);
  rt.transport.playEnding();
  expect([6, 7, 8].map(bar)).toEqual([6, 7, null]);

  const plain = fake();
  await plain.rt.evaluate('note("c")');
  plain.rt.transport.playEnding();
  plain.rt.transport.startFrom(1, STRUCTURE);
  plain.rt.transport.loopSection(1, STRUCTURE);
  expect(tstate(plain.rt)).toEqual({
    view: { kind: 'song' },
    pendingStart: false,
    armedSection: null,
  });
});

test('playEnding loopPass boundary on a pass start takes effect right there', async () => {
  const { rt, at, bar } = await playing();
  at(5.5, 5.9); // the next unqueried bar, 6, starts a pass
  rt.transport.playEnding();
  expect([6, 7, 8].map(bar)).toEqual([6, 7, null]);
});

test('a later change supersedes one armed at the same boundary', async () => {
  const { rt, at, bar } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE);
  rt.transport.startFrom(2, STRUCTURE);
  expect(tstate(rt).view).toEqual({ kind: 'song' });
  expect([4, 5, 6].map(bar)).toEqual([6, 7, null]);
});

test('Stop resets the transport to the start', async () => {
  const { rt, at } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE);
  rt.stop();
  expect(snap(rt).started).toBe(false);
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
  // The next start plays from the top.
  await rt.evaluate(SONG);
  rt.scheduler.now = () => 4;
  expect(rt.position().located?.songBar).toBe(4);
});

test('Stop also disarms a start armed while stopped', () => {
  const { rt } = fake();
  rt.transport.loopSection(2, STRUCTURE);
  rt.stop();
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
});

test('reset disarms a stopped start and notifies', () => {
  const { rt } = fake();
  rt.transport.startFrom(2, STRUCTURE);
  const seen: TransportState[] = [];
  const unsub = rt.transport.store.subscribe((v) => void seen.push(v));
  rt.transport.reset();
  unsub();
  expect(seen.at(-1)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
  expect(seen).toHaveLength(2);
});

test('playing: reset drops a loop and plays on in song view from the global cycle', async () => {
  const { rt, at, bar } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE);
  expect(tstate(rt).view).toEqual({ kind: 'loop', section: 0 });
  rt.transport.reset();
  expect(tstate(rt).view).toEqual({ kind: 'song' });
  expect([4, 5].map(bar)).toEqual([4, 5]);
});

test('a re-evaluate keeps the segments and swaps the structure', async () => {
  const { rt, at, bar } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE);
  const shorter = SONG.replace("'bars':2", "'bars':1"); // intro now 1 bar
  expect(await rt.evaluate(shorter)).toBe(true);
  expect(snap(rt).structure?.sections[0]!.bars).toBe(1);
  expect(tstate(rt).view).toEqual({ kind: 'loop', section: 0 });
  expect([4, 5].map(bar)).toEqual([0, 0]);
});

// ---------------------------------------------------------------- position and auto-stop

test('position clamps a negative cycle to 0 and is null without a structure', async () => {
  const { rt } = fake();
  await rt.evaluate('note("c")');
  rt.scheduler.now = () => -0.05;
  expect(rt.position()).toEqual({ cycle: 0, located: null });
  await rt.evaluate(SONG);
  rt.scheduler.now = () => -0.05;
  expect(rt.position()).toEqual({
    cycle: 0,
    located: { section: 0, localCycle: 0, songBar: 0 },
  });
});

test('auto-stop: a song with no loop section stops once it has played through', async () => {
  const { rt, tick } = fake();
  expect(await rt.evaluate(STRAIGHT, { cps: 2 })).toBe(true); // 2 bars = 1 s
  tick(5);
  expect(snap(rt).started).toBe(true);
  tick(10);
  expect(snap(rt).started).toBe(false);
  expect(rt.position().cycle).toBe(0);
});

test('auto-stop: a looping song, or code without song(), plays on', async () => {
  const looping = fake();
  await looping.rt.evaluate(SONG, { cps: 4 });
  looping.tick(40); // 16 bars
  expect(snap(looping.rt).started).toBe(true);
  const plain = fake();
  await plain.rt.evaluate('note("c")', { cps: 4 });
  plain.tick(40);
  expect(snap(plain.rt).started).toBe(true);
});

test('auto-stop after playEnding: the tail plays, then the scheduler stops', async () => {
  const { rt, tick } = fake();
  await rt.evaluate(SONG, { cps: 2 });
  tick(12); // about cycle 2.3: inside the body
  rt.transport.playEnding();
  expect(tstate(rt).view).toEqual({ kind: 'release' }); // released at 6: the tail is bars 6-7
  tick(25); // about cycle 7.3
  expect(snap(rt).started).toBe(true);
  tick(10);
  expect(snap(rt).started).toBe(false);
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
});

test('auto-stop leaves a stopped runtime alone', async () => {
  const { rt, tick, timers } = fake();
  expect(timers.size).toBe(1); // the auto-stop interval, created once
  rt.transport.startFrom(2, STRUCTURE);
  tick(5);
  expect(tstate(rt).pendingStart).toBe(true);
});

// ---------------------------------------------------------------- query

/** Onset haps as [whole.begin, whole.end, value], sorted by begin. */
const onsetsOf = (haps: any[] | null) =>
  haps!
    .filter((h) => h.hasOnset())
    .map((h) => [h.whole.begin.valueOf(), h.whole.end.valueOf(), h.value])
    .sort((x, y) => x[0] - y[0]);
/** SONG on the linear timeline: every bar once, the tail included. */
const LINEAR_SONG = ['i0', 'i1', 'b0', 'b1', 'b2', 'b3', 't0', 't1'].map((s, i) => [
  i,
  i + 1,
  { s },
]);

test('query is null before any evaluate, and after a failed first one', async () => {
  const { rt } = fake({ running: false });
  expect(rt.query(0, 4)).toBeNull();
  expect(await rt.evaluate('s("a").nonexistent()')).toBe(false);
  expect(rt.query(0, 4)).toBeNull();
});

test('a failed evaluate keeps the previously evaluated pattern for query', async () => {
  const { rt } = fake({ running: false });
  expect(await rt.evaluate('s("<a b>")')).toBe(true);
  expect(await rt.evaluate('s("c").nonexistent()')).toBe(false);
  expect(onsetsOf(rt.query(0, 2))).toEqual([
    [0, 1, { s: 'a' }],
    [1, 2, { s: 'b' }],
  ]);
});

test('query lays song() out linearly: every bar once, the tail included', async () => {
  const { rt } = fake({ running: false });
  expect(await rt.evaluate(SONG)).toBe(true);
  expect(onsetsOf(rt.query(0, STRUCTURE.totalBars))).toEqual(LINEAR_SONG);
  // Part of the range: the tail's last bar alone, and nothing past the end.
  expect(onsetsOf(rt.query(7, 8))).toEqual([LINEAR_SONG[7]]);
  expect(rt.query(8, 12)).toEqual([]);
});

test('query is independent of playback, loop and armed section', async () => {
  const stopped = fake({ running: false });
  await stopped.rt.evaluate(SONG);
  const reference = onsetsOf(stopped.rt.query(0, 8));
  expect(reference).toEqual(LINEAR_SONG);

  // Armed while stopped: a start from the tail, then a loop of the intro.
  stopped.rt.transport.startFrom(2, STRUCTURE);
  expect(tstate(stopped.rt).armedSection).toBe(2);
  expect(onsetsOf(stopped.rt.query(0, 8))).toEqual(reference);
  stopped.rt.transport.loopSection(0, STRUCTURE);
  expect(tstate(stopped.rt).view).toEqual({ kind: 'loop', section: 0 });
  expect(onsetsOf(stopped.rt.query(0, 8))).toEqual(reference);

  // Playing, mid-song, then looping the intro.
  const { rt, at } = await playing();
  at(5.2, 5.4);
  expect(onsetsOf(rt.query(0, 8))).toEqual(reference);
  rt.transport.loopSection(0, STRUCTURE);
  expect(tstate(rt).view).toEqual({ kind: 'loop', section: 0 });
  expect(onsetsOf(rt.query(0, 8))).toEqual(reference);
});

test('query with clip segments repeats the loop section and plays the tail once', async () => {
  const { rt } = fake({ running: false });
  const meta = { ...META, loops: 2 };
  const structure = buildStructure(meta);
  expect(
    await rt.evaluate(`const meta = ${literal(meta)};\n` + SONG.split('\n').slice(1).join('\n')),
  ).toBe(true);
  const ids = (haps: any[] | null) => onsetsOf(haps).map((h) => h[2].s);
  expect(ids(rt.query(0, structure.clipBars, clipSegments(structure)))).toEqual([
    'i0',
    'i1',
    'b0',
    'b1',
    'b2',
    'b3',
    'b0',
    'b1',
    'b2',
    'b3',
    't0',
    't1',
  ]);
  // The default is still the linear layout.
  expect(onsetsOf(rt.query(0, 8))).toEqual(LINEAR_SONG);
  // One loop: both layouts agree.
  const one = fake({ running: false });
  await one.rt.evaluate(SONG);
  expect(onsetsOf(one.rt.query(0, 8, clipSegments(STRUCTURE)))).toEqual(
    onsetsOf(one.rt.query(0, 8)),
  );
});

test('query leaves the live segment view untouched', async () => {
  const { rt, at, sounds } = await playing();
  at(3.2, 3.4);
  rt.transport.loopSection(0, STRUCTURE); // from cycle 4: bars 0,1,0,1…
  expect(sounds(6)).toEqual(['i0']);
  rt.query(0, 8);
  expect(sounds(6)).toEqual(['i0']); // still the loop, not the linear t0
  expect(sounds(2)).toEqual(['b0']);
});

test('a pattern error surfaces from query, and the segment view is untouched', async () => {
  let pattern: any;
  const { rt } = fake({ running: false, hooks: { afterEval: (a) => void (pattern = a.pattern) } });
  // The tail throws; in song view the body loops forever, so only the linear timeline reaches it.
  const code = `const meta = ${literal(META)};
song(meta, { intro: s("i"), body: s("b"), tail: new Pattern(() => { throw new Error('tail boom') }) })
`;
  expect(await rt.evaluate(code)).toBe(true);
  rt.transport.loopSection(0, STRUCTURE);
  expect(() => rt.query(0, 8)).toThrow('tail boom');
  expect(pattern.queryArc(6, 7).map((h: any) => h.value.s)).toEqual(['i']); // looped intro
  // The bars before the tail still answer.
  expect(onsetsOf(rt.query(0, 6)).map((o) => o[2].s)).toEqual(['i', 'i', 'b', 'b', 'b', 'b']);
});

test('a throwing $: block is isolated: its bars are silent and the other blocks still answer', async () => {
  const { rt } = fake({ running: false });
  // `$:` blocks are stacked, and stack logs a failing layer and leaves it silent.
  const code = `const meta = ${literal(META)};
$: song(meta, { intro: s("i"), body: s("b"), tail: new Pattern(() => { throw new Error('tail boom') }) })
$: s("x")
`;
  expect(await rt.evaluate(code)).toBe(true);
  const values = rt
    .query(0, 8)!
    .filter((h) => h.hasOnset())
    .map((h) => h.value.s);
  expect(values.filter((v: string) => v === 'x')).toHaveLength(8);
  expect(values.filter((v: string) => v !== 'x')).toEqual([]);
  expect(onsetsOf(rt.query(0, 6)).map((o) => o[2].s)).toEqual([
    'i',
    'x',
    'i',
    'x',
    'b',
    'x',
    'b',
    'x',
    'b',
    'x',
    'b',
    'x',
  ]);
});

test('query lays out a wrapped song() linearly, whatever the loop and armed state', async () => {
  const { rt } = fake({ running: false });
  const code = `const meta = ${literal(META)};
$: song(meta, { intro: s("i"), body: s("b"), tail: s("t") }).gain(0.5)
$: stack(song(meta, { intro: s("x"), body: s("x"), tail: s("x") }).fast(1)).s("x")
`;
  expect(await rt.evaluate(code)).toBe(true);
  const gained = () =>
    rt
      .query(0, 8)!
      .filter((h) => h.value.gain === 0.5)
      .map((h) => [Number(h.whole.begin), h.value.s]);
  const reference = gained();
  expect(reference.length).toBe(8);
  expect(reference.map((r) => r[1])).toEqual(['i', 'i', 'b', 'b', 'b', 'b', 't', 't']);
  rt.transport.startFrom(2, STRUCTURE);
  rt.transport.loopSection(0, STRUCTURE);
  expect(gained()).toEqual(reference);
});

test('query carries the transpiler locations of double-quoted strings, not single-quoted', async () => {
  const { rt } = fake({ running: false });
  const code = `$: s("bd hh")\n$: s('cp sd')`;
  expect(await rt.evaluate(code)).toBe(true);
  const haps = rt.query(0, 1)!;
  const located = haps
    .filter((h) => h.context.locations?.length)
    .map((h) => [h.value.s, h.context.locations.map((l: any) => code.slice(l.start, l.end))]);
  expect(located).toEqual([
    ['bd', ['bd']],
    ['hh', ['hh']],
  ]);
  const bd = haps.find((h) => h.value.s === 'bd');
  expect(bd.context.locations).toEqual([
    { start: code.indexOf('bd'), end: code.indexOf('bd') + 2 },
  ]);
  const single = haps.find((h) => h.value.s === 'cp sd');
  expect(single.context.locations ?? []).toEqual([]);
});

test('without song(), bar N is cycle N-1', async () => {
  const { rt } = fake({ running: false });
  expect(await rt.evaluate('s("<a b c>")')).toBe(true);
  expect(onsetsOf(rt.query(0, 1))).toEqual([[0, 1, { s: 'a' }]]);
  expect(onsetsOf(rt.query(2, 3))).toEqual([[2, 3, { s: 'c' }]]);
});

test('LINEAR_SEGMENTS is one frozen release segment from cycle 0', () => {
  expect(LINEAR_SEGMENTS).toEqual([{ at: 0, songBar: 0, view: { kind: 'release' } }]);
  expect(Object.isFrozen(LINEAR_SEGMENTS)).toBe(true);
  expect(Object.isFrozen(LINEAR_SEGMENTS[0])).toBe(true);
});

// ---- hap tap ----
type Tap = { hap: any; t: number; cps: number; route: string };
/** Evaluates `code`, plays a few ticks and stops. `own` collects haps seen by a pattern's onSchedule. */
const playTapped = async (
  code: string,
  onSchedule?: (hap: any, t: number, cps: number, route: any) => void,
) => {
  const taps: Tap[] = [];
  const own: any[] = [];
  (globalThis as any).__own = own;
  const f = fake({
    hooks: onSchedule
      ? {
          onSchedule: (hap, t, cps, route) => {
            taps.push({ hap, t, cps, route });
            onSchedule(hap, t, cps, route);
          },
        }
      : undefined,
  });
  expect(await f.rt.evaluate(code)).toBe(true);
  f.tick();
  await settle();
  f.rt.stop();
  return { ...f, taps, own };
};

test('tap: a plain pattern is tapped once per hap, route default, with the same args as output', async () => {
  const { taps, b } = await playTapped('note("c e g").s("sine")', () => {});
  expect(taps.length).toBeGreaterThan(0);
  expect(taps.length).toBe(b.calls.length);
  expect(taps.every((x) => x.route === 'default')).toBe(true);
  expect(taps[0].hap).toBe(b.calls[0][0]);
  expect(taps[0].t).toBe(b.calls[0][1]);
  expect(typeof taps[0].cps).toBe('number');
});
test('tap: without a hook, haps still reach output', async () => {
  const f = fake();
  expect(await f.rt.evaluate('note("c e g").s("sine")')).toBe(true);
  f.tick();
  await settle();
  f.rt.stop();
  expect(f.b.calls.length).toBeGreaterThan(0);
});

test('tap: runs before backend.output', async () => {
  const seen: string[] = [];
  const f = fake({ hooks: { onSchedule: () => void seen.push('tap') } });
  const push = f.b.calls.push.bind(f.b.calls);
  f.b.calls.push = (...a: any[]) => (seen.push('output'), push(...a));
  await f.rt.evaluate('note("c").s("sine")');
  f.tick();
  await settle();
  f.rt.stop();
  expect(seen.slice(0, 2)).toEqual(['tap', 'output']);
});
test('tap: .onSchedule(fn) gives route pattern and skips backend.output', async () => {
  const { taps, b, own } = await playTapped(
    'note("c e").s("sine").onSchedule((h) => globalThis.__own.push(h))',
    () => {},
  );
  expect(taps.length).toBeGreaterThan(0);
  expect(taps.every((x) => x.route === 'pattern')).toBe(true);
  expect(own.length).toBe(taps.length);
  expect(b.calls.length).toBe(0);
});
test('tap: .onSchedule(fn, false) gives route both and also calls backend.output', async () => {
  const { taps, b, own } = await playTapped(
    'note("c e").s("sine").onSchedule((h) => globalThis.__own.push(h), false)',
    () => {},
  );
  expect(taps.length).toBeGreaterThan(0);
  expect(taps.every((x) => x.route === 'both')).toBe(true);
  expect(own.length).toBe(taps.length);
  expect(b.calls.length).toBe(taps.length);
});
test('tap: a throwing tap does not block output or the pattern onSchedule', async () => {
  const err = console.error;
  console.error = () => {};
  try {
    const plain = await playTapped('note("c e").s("sine")', () => {
      throw new Error('tap boom');
    });
    expect(plain.b.calls.length).toBe(plain.taps.length);
    expect(plain.b.calls.length).toBeGreaterThan(0);
    const both = await playTapped(
      'note("c e").s("sine").onSchedule((h) => globalThis.__own.push(h), false)',
      () => {
        throw new Error('tap boom');
      },
    );
    expect(both.b.calls.length).toBeGreaterThan(0);
    expect(both.own.length).toBe(both.b.calls.length);
  } finally {
    console.error = err;
  }
});
test('tap: no hook behaves as before', async () => {
  const { b, own } = await playTapped('note("c e g").s("sine")');
  expect(b.calls.length).toBeGreaterThan(0);
  expect(own.length).toBe(0);
  expect(b.calls[0][0].value.note).toBe('c');
});

// ---- seed ----

const RAND_CODE = 's("a*8").degradeBy(0.5).pan(rand)';
const view = (rt: ReturnType<typeof createRuntime>, to = 4) =>
  onsetsOf(rt.query(0, to)).map((o) => JSON.stringify(o));

test('seed: the same seed gives identical haps on every evaluate, different seeds differ', async () => {
  const { rt } = fake({ running: false });
  await rt.evaluate(RAND_CODE, { seed: 5 });
  const first = view(rt);
  await rt.evaluate(RAND_CODE, { seed: 5 });
  expect(view(rt)).toEqual(first);
  await rt.evaluate(RAND_CODE, { seed: 6 });
  expect(view(rt)).not.toEqual(first);
  await rt.evaluate(RAND_CODE);
  expect(view(rt)).not.toEqual(first);
});

test('seed: evaluate with a seed equals core.seed applied by hand', async () => {
  const { rt } = fake({ running: false });
  await rt.evaluate(RAND_CODE, { seed: 5 });
  const seeded = view(rt);
  await rt.evaluate(`(${RAND_CODE}).seed(5)`);
  expect(view(rt)).toEqual(seeded);
});

test('seed: no seed and seed 0 give the same haps', async () => {
  const { rt } = fake({ running: false });
  await rt.evaluate(RAND_CODE);
  const none = view(rt);
  await rt.evaluate(RAND_CODE, { seed: 0 });
  expect(view(rt)).toEqual(none);
});

test('seed: an inner .seed() wins over the option', async () => {
  const { rt } = fake({ running: false });
  await rt.evaluate(`(${RAND_CODE}).seed(1)`, { seed: 5 });
  const inner = view(rt);
  await rt.evaluate(`(${RAND_CODE}).seed(1)`, { seed: 9 });
  expect(view(rt)).toEqual(inner);
  await rt.evaluate(RAND_CODE, { seed: 1 });
  expect(view(rt)).toEqual(inner);
});

test('seed: the song() segments control survives the seeding', async () => {
  const { rt } = fake({ running: false });
  const code = `const meta = {
  bpm: 120, meter: '4/4',
  sections: [{ name: 'a', bars: 1 }, { name: 'b', bars: 2 }],
}
$: song(meta, { a: s('i'), b: s('b') })`;
  await rt.evaluate(code, { seed: 3 });
  const seeded = onsetsOf(rt.query(0, 3)).map((o) => o[2].s);
  expect(seeded).toEqual(['i', 'b', 'b']);
});

test('seed: RuntimeState.seed follows successful evaluates only', async () => {
  const { rt } = fake({ running: false });
  expect(snap(rt).seed).toBeUndefined();
  await rt.evaluate('s("a")', { seed: 5 });
  expect(snap(rt).seed).toBe(5);
  expect(await rt.evaluate('s("a").nonexistent()', { seed: 8 })).toBe(false);
  expect(snap(rt).seed).toBe(5);
  await rt.evaluate('s("a")', { seed: 0 });
  expect(snap(rt).seed).toBe(0);
  await rt.evaluate('s("a")');
  expect(snap(rt).seed).toBeUndefined();
});

test('audio seed: set before afterEval on every successful evaluate, 0 without a seed', async () => {
  const seenAtHook: number[][] = [];
  const { rt, b } = fake({
    running: false,
    hooks: { afterEval: () => void seenAtHook.push([...b.seeds]) },
  });
  await rt.evaluate('s("a")', { seed: 5 });
  // Set at the commit point, before the afterEval hook.
  expect(seenAtHook).toEqual([[5]]);
  await rt.evaluate('s("a")');
  expect(b.seeds).toEqual([5, 0]);
  await rt.evaluate('s("a")', { seed: 0 });
  expect(b.seeds).toEqual([5, 0, 0]);
});

test('audio seed: set before the first hap of an evaluate that starts playback', async () => {
  const seenAtHook: string[][] = [];
  const { rt, b } = fake({ hooks: { afterEval: () => void seenAtHook.push([...b.order]) } });
  await rt.evaluate('s("a*16")', { seed: 5 });
  expect(snap(rt).started).toBe(true);
  // Starting schedules the first haps synchronously, before afterEval.
  expect(seenAtHook[0]).toContain('output');
  expect(b.order[0]).toBe('seed 5');
  rt.stop();
});

test('audio seed: the code runs before the seed changes', async () => {
  const { rt, b } = fake({ running: false });
  (globalThis as any).__seeds = b.seeds;
  await rt.evaluate('globalThis.__seen = [...globalThis.__seeds]; s("a")', { seed: 7 });
  expect((globalThis as any).__seen).toEqual([]);
  expect(b.seeds).toEqual([7]);
});

test('audio seed: a failed evaluate never touches the backend seed', async () => {
  const { rt, b } = fake({ running: false });
  expect(await rt.evaluate('s("a").nonexistent()', { seed: 8 })).toBe(false);
  expect(b.seeds).toEqual([]);
  await rt.evaluate('s("a")', { seed: 5 });
  expect(await rt.evaluate('s("a").nonexistent()', { seed: 8 })).toBe(false);
  expect(await rt.evaluate('s("a"', { seed: 9 })).toBe(false);
  expect(b.seeds).toEqual([5]);
});

test("useRNG: a failed evaluate keeps the playing song's RNG mode", async () => {
  const { rt } = fake({ running: false });
  await rt.evaluate("useRNG('precise'); s('a')");
  expect(core.getRNGMode()).toBe('precise');
  expect(await rt.evaluate("s('a').nonexistent()")).toBe(false);
  expect(core.getRNGMode()).toBe('precise');
  expect(await rt.evaluate("s('a'")).toBe(false);
  expect(core.getRNGMode()).toBe('precise');
  // A failing song's own useRNG call is undone as well.
  expect(await rt.evaluate("useRNG('legacy'); s('a').nonexistent()")).toBe(false);
  expect(core.getRNGMode()).toBe('precise');
  await rt.evaluate("s('a')");
  expect(core.getRNGMode()).toBe('legacy');
});

test('useRNG: a song that switches the RNG does not leak into the next evaluate', async () => {
  const { rt } = fake({ running: false });
  const legacy = core.getRandsAtTime(1.25, 3, 0);
  await rt.evaluate("useRNG('precise'); s('a')");
  await rt.evaluate("s('a')");
  expect(core.getRandsAtTime(1.25, 3, 0)).toEqual(legacy);
  core.useRNG('precise');
  expect(core.getRandsAtTime(1.25, 3, 0)).not.toEqual(legacy);
  await rt.evaluate("s('a')");
  expect(core.getRandsAtTime(1.25, 3, 0)).toEqual(legacy);
});

// ---------------------------------------------------------------- start, count-in and cycleTime

/** Onset haps the backend received, as [whole.begin, value.s, t]. */
const delivered = (b: { calls: any[] }) =>
  b.calls.map(([h, t]) => [h.whole.begin.valueOf(), h.value.s, t] as [number, string, number]);

test('evaluate with autostart false installs the pattern without starting', async () => {
  const { rt, b, tick } = fake();
  expect(await rt.evaluate(STRAIGHT, { cps: 2, autostart: false })).toBe(true);
  expect(snap(rt).started).toBe(false);
  expect(snap(rt).structure?.sections.map((s: any) => s.name)).toEqual(['a', 'b']);
  expect(snap(rt).cps).toBe(2);
  tick(5);
  expect(b.calls).toEqual([]);
  expect(rt.query(0, 2)!.map((h) => h.value.s)).toEqual(['a', 'b']);
});

test('evaluate with autostart false while playing replaces the live pattern and keeps playing', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate('s("x")', { cps: 2 });
  tick(5);
  expect(await rt.evaluate('s("y")', { cps: 2, autostart: false })).toBe(true);
  expect(snap(rt).started).toBe(true);
  b.calls.length = 0;
  tick(5);
  expect(new Set(delivered(b).map((d) => d[1]))).toEqual(new Set(['y']));
});

test('start: plays the evaluated song from cycle 0, at the times cycleTime gives', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  expect(rt.cycleTime(0)).toBeNull(); // not started
  expect(await rt.start()).toBe(true);
  expect(snap(rt).started).toBe(true);
  // Started at clock 0: the checkpoint is at 0.01 (zyklus's minLatency), plus 0.1 latency.
  expect(rt.cycleTime(0)).toBe(0 / 2 + 0.01 + 0.1);
  expect(rt.cycleTime(1)).toBe(1 / 2 + 0.01 + 0.1);
  expect(rt.cycleTime(-1)).toBe(-1 / 2 + 0.01 + 0.1);
  tick(5);
  expect(delivered(b)).toEqual([
    [0, 'a', rt.cycleTime(0)!],
    [1, 'b', rt.cycleTime(1)!],
  ]);
});

test('start: false, changing nothing, before an evaluate, while playing, or without audio', async () => {
  const none = fake();
  expect(await none.rt.start({ countIn: true })).toBe(false);
  expect(none.b.resumes).toBe(0);
  expect(snap(none.rt).started).toBe(false);

  const failed = fake();
  expect(await failed.rt.evaluate('s("a"', { autostart: false })).toBe(false);
  expect(await failed.rt.start()).toBe(false);
  expect(snap(failed.rt).started).toBe(false);

  const playing = fake();
  await playing.rt.evaluate(STRAIGHT, { cps: 2 });
  expect(await playing.rt.start({ countIn: true })).toBe(false);
  playing.tick(3); // no count-in was installed: bar 0 plays at once
  expect(playing.rt.position().located).not.toBeNull();

  const silent = fake({ running: false });
  await silent.rt.evaluate(STRAIGHT, { autostart: false });
  expect(await silent.rt.start()).toBe(false);
  expect(snap(silent.rt).started).toBe(false);
  expect(snap(silent.rt).audio).toBe('suspended');
});

test('start: a stop pressed while audio resumes wins', async () => {
  let release!: (v: boolean) => void;
  const { rt, b } = fake({
    running: false,
    resume: () => new Promise<boolean>((r) => (release = r)),
  });
  const ev = rt.evaluate(STRAIGHT, { autostart: false });
  release(false);
  expect(await ev).toBe(true);
  const st = rt.start();
  rt.stop();
  b.running = true;
  release(true);
  expect(await st).toBe(false);
  expect(snap(rt).started).toBe(false);
});

test('start with a count-in: one silent bar, then bar 0 at cycle 1', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  expect(await rt.start({ countIn: true })).toBe(true);
  expect(rt.position()).toEqual({ cycle: 0, located: null });
  tick(10);
  expect(delivered(b)).toEqual([
    [1, 'a', rt.cycleTime(1)!],
    [2, 'b', rt.cycleTime(2)!],
  ]);
});

test('the count-in wakes no transport subscriber and keeps an armed section', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate(SONG, { cps: 2, autostart: false });
  rt.transport.startFrom(2, STRUCTURE); // the tail, bars 6-7
  const seen: TransportState[] = [];
  const unsub = rt.transport.store.subscribe((v) => void seen.push(v));
  expect(await rt.start({ countIn: true })).toBe(true);
  // The one emission is the armed start becoming a plain start, as without a count-in.
  expect(seen).toEqual([
    { view: { kind: 'song' }, pendingStart: true, armedSection: 2 },
    { view: { kind: 'song' }, pendingStart: false, armedSection: null },
  ]);
  tick(10);
  unsub();
  expect(seen).toHaveLength(2);
  expect(delivered(b).map((d) => d.slice(0, 2))).toEqual([
    [1, 't0'],
    [2, 't1'],
  ]);
});

test('a stop drops the count-in: the next plain start plays from cycle 0', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  await rt.start({ countIn: true });
  rt.stop();
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
  b.calls.length = 0;
  expect(await rt.start()).toBe(true);
  tick(3);
  expect(delivered(b)[0]!.slice(0, 2)).toEqual([0, 'a']);
});

test('a failed start restores the segments and the pattern', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  rt.transport.startFrom(1, buildStructure(STRAIGHT_META));
  const start = rt.scheduler.start;
  rt.scheduler.start = async () => {
    throw new Error('clock nope');
  };
  const err = console.error;
  console.error = () => {};
  try {
    expect(await rt.start({ countIn: true })).toBe(false);
  } finally {
    console.error = err;
    rt.scheduler.start = start;
  }
  expect(snap(rt).started).toBe(false);
  expect(tstate(rt)).toEqual({ view: { kind: 'song' }, pendingStart: true, armedSection: 1 });
  expect(await rt.start()).toBe(true);
  tick(3);
  expect(delivered(b)[0]!.slice(0, 2)).toEqual([0, 'b']); // the armed section, no count-in
});

test('a failed plain-script count-in start puts the unshifted pattern back', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate('s("p")', { cps: 2, autostart: false });
  const pattern = rt.scheduler.pattern;
  const start = rt.scheduler.start;
  rt.scheduler.start = async () => {
    throw new Error('clock nope');
  };
  const err = console.error;
  console.error = () => {};
  try {
    expect(await rt.start({ countIn: true })).toBe(false);
  } finally {
    console.error = err;
    rt.scheduler.start = start;
  }
  expect(rt.scheduler.pattern).toBe(pattern);
  await rt.start();
  tick(3);
  expect(delivered(b)[0]!.slice(0, 2)).toEqual([0, 'p']);
});

test('auto-stop skips the count-in, then fires after the song', async () => {
  const { rt, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false }); // 2 bars = 1 s, plus 0.5 s count-in
  await rt.start({ countIn: true });
  tick(3); // about cycle 0.5: inside the count-in
  expect(rt.position().located).toBeNull();
  expect(snap(rt).started).toBe(true);
  tick(9); // about cycle 2.3: bar 1 of the song
  expect(snap(rt).started).toBe(true);
  tick(10);
  expect(snap(rt).started).toBe(false);
});

test('plain script: a count-in delays the pattern by one cycle, query stays unshifted', async () => {
  const { rt, b, tick } = fake();
  await rt.evaluate('s("p")', { cps: 2, autostart: false });
  expect(await rt.start({ countIn: true })).toBe(true);
  tick(9);
  expect(delivered(b)).toEqual([
    [1, 'p', rt.cycleTime(1)!],
    [2, 'p', rt.cycleTime(2)!],
  ]);
  expect(rt.query(0, 1)!.map((h) => h.whole.begin.valueOf())).toEqual([0]);
  // Twice in a row: delayed by one cycle, not two.
  rt.stop();
  b.calls.length = 0;
  await rt.start({ countIn: true });
  tick(5);
  expect(delivered(b)[0]!.slice(0, 2)).toEqual([1, 'p']);
  // A plain start puts the unshifted pattern back.
  rt.stop();
  b.calls.length = 0;
  await rt.start();
  tick(3);
  expect(delivered(b)[0]!.slice(0, 2)).toEqual([0, 'p']);
});

test('cycleTime: null after a cps change until the next tick, then on the new tempo; null after stop', async () => {
  const { rt, tick } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  await rt.start();
  tick(2);
  rt.scheduler.setCps(1);
  expect(rt.cycleTime(1)).toBeNull();
  tick(1);
  const cp = rt.scheduler.checkpoint;
  expect(cp.cycles).toBeGreaterThan(0);
  expect(rt.cycleTime(5)).toBe((5 - cp.cycles) / 1 + cp.seconds + 0.1);
  rt.stop();
  expect(rt.cycleTime(5)).toBeNull();
});

test('cycleTime: a cps hap rebases the checkpoint exactly, so the time is known in the same tick', async () => {
  const { rt, b, tick } = fake();
  // Twenty haps a cycle at cps 1; the fourth (cycle 0.15, inside start's last synchronous tick)
  // doubles the tempo.
  await rt.evaluate('s("a*20").cps("1!3 2!17")', { cps: 1, autostart: false });
  expect(await rt.start()).toBe(true);
  // The hap reset the cyclist's tick counter inside that tick; its checkpoint is the hap.
  expect(rt.scheduler.num_ticks_since_checkpoint).toBe(0);
  expect(rt.scheduler.checkpoint.cycles.valueOf()).toBe(0.15);
  const T = rt.cycleTime(1);
  expect(T).toBeCloseTo((1 - 0.15) / 2 + 0.16 + 0.1, 9);
  tick(15);
  // Bar 1 sounds when cycleTime said it would.
  expect(delivered(b).find((d) => d[0] === 1)![2]).toBeCloseTo(T!, 9);
});

test('cycleTime: a non-finite result is null', async () => {
  const { rt } = fake();
  await rt.evaluate(STRAIGHT, { cps: 2, autostart: false });
  await rt.start();
  expect(rt.cycleTime(Number.NaN)).toBeNull();
  expect(rt.cycleTime(Infinity)).toBeNull();
});

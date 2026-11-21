import { expect, jest, test } from 'bun:test';
import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import type { LiveStats } from '../src/lib/audio/playback-api';
import {
  createRuntime,
  DRAIN_MAX_SEC,
  DRAIN_RELEASE_SEC,
  HAP_LATENCY_SEC,
  type RuntimeState,
} from '../src/lib/strudel/runtime';
import { song } from '../src/lib/strudel/song';
import { fakeRuntime as fake } from './helpers/fake-runtime';

await core.evalScope(core, mini, { song });

const snap = (rt: ReturnType<typeof createRuntime>): RuntimeState => {
  let s!: RuntimeState;
  rt.store.subscribe((v) => (s = v))();
  return s;
};
const settle = () => new Promise((r) => setTimeout(r, 20));

// intro 2 bars, body 2 bars; at 120 bpm 4/4 a bar is 2 s, 20 fake ticks.
const SONG = `const meta = { bpm: 120, sections: [{ name: 'intro', bars: 2 }, { name: 'body', bars: 2 }] };
$: song(meta, { intro: s("<i0 i1>"), body: s("<b0 b1>") })
`;
const PLAIN = 'note("c e g").s("sine")';

const stats = (underruns: number): LiveStats => ({
  underruns,
  lateHaps: 0,
  haps: 10,
  batches: 2,
  skippedHaps: 0,
  renderHeadroomMs: 30,
  batchDecodeUs: { p50: 5, max: 9 },
  invokeRttMs: { p50: 1, p95: 2, max: 3 },
});

test('the hap latency is 0.1 s on both engines and stays across the swap', async () => {
  expect(HAP_LATENCY_SEC).toBe(0.1);
  const { rt } = fake({ rust: true });
  expect(rt.scheduler.latency).toBe(HAP_LATENCY_SEC);
  expect(await rt.evaluate(PLAIN, { engine: 'rust', autostart: false })).toBe(true);
  expect(rt.scheduler.latency).toBe(HAP_LATENCY_SEC);
});

test('superdough is active at first; a rust song swaps at once while stopped', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(snap(rt)).toMatchObject({
    engine: 'superdough',
    backend: 'superdough',
    live: null,
    engineWarnings: [],
  });
  expect(await rt.evaluate(PLAIN, { engine: 'rust', seed: 5 })).toBe(true);
  expect(snap(rt)).toMatchObject({ engine: 'rust', backend: 'rust', started: true });
  // The gesture resumed the rust engine, and the seed went to it.
  expect(rb.resumes).toBe(1);
  expect(rb.seeds.at(-1)).toBe(5);
  tick(10);
  await settle();
  expect(rb.calls.length).toBeGreaterThan(0);
  expect(b.calls.length).toBe(0);
  // The clock and the scope follow the active backend.
  expect(rt.readScope(new Float32Array(4))).toBe(true);
  expect(rb.scopes).toBe(1);
  rt.stop();
});

test('a failed evaluate puts the previous engine back', async () => {
  const { rt } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  expect(await rt.evaluate('note("c").nonexistent()', { engine: 'rust', autostart: false })).toBe(
    false,
  );
  expect(snap(rt).engine).toBe('superdough');
  // And start plays the song's engine, superdough.
  expect(await rt.start()).toBe(true);
  expect(snap(rt).engine).toBe('superdough');
  rt.stop();
});

test('a failed evaluate does not undo a later queued evaluate for the same engine', async () => {
  const { rt } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  // Both queued at once: the first fails, the second (same engine) succeeds.
  const bad = rt.evaluate('note("c").nonexistent()', { engine: 'rust', autostart: false });
  const good = rt.evaluate(PLAIN, { engine: 'rust', autostart: false });
  expect(await bad).toBe(false);
  expect(await good).toBe(true);
  expect(snap(rt).engine).toBe('rust');
  expect(await rt.start()).toBe(true);
  expect(snap(rt).engine).toBe('rust');
  rt.stop();
});

test('start({ backend }) overrides the song for one play; stop returns to the song engine', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { engine: 'rust', autostart: false })).toBe(true);
  expect(snap(rt).engine).toBe('rust');
  expect(await rt.start({ backend: 'superdough' })).toBe(true);
  expect(snap(rt).engine).toBe('superdough');
  // Leaving rust ended its session.
  expect(rb.stops).toBe(1);
  tick(10);
  await settle();
  expect(b.calls.length).toBeGreaterThan(0);
  expect(rb.calls.length).toBe(0);
  rt.stop();
  expect(snap(rt).engine).toBe('rust');
});

test('stop() ends the rust session', async () => {
  const { rb, rt } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  expect(rb.stops).toBe(0);
  rt.stop();
  expect(rb.stops).toBe(1);
  expect(snap(rt).engine).toBe('rust');
  // The next start opens a new session.
  expect(await rt.start()).toBe(true);
  expect(rb.resumes).toBe(2);
  rt.stop();
});

const begins = (calls: any[]): number[] => calls.map(([h]) => h.whole.begin.valueOf());

/** `n` ticks, one at a time, letting a switch's async start run between them (as in the app). */
async function play(tick: (n?: number) => void, n: number): Promise<void> {
  for (let i = 0; i < n; i++) {
    tick(1);
    await new Promise((r) => setTimeout(r, 0));
  }
}

test('a switch while playing waits for the next bar line, then continues from that bar', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(10); // half a bar
  await settle();
  expect(await rt.evaluate(SONG, { engine: 'rust' })).toBe(true);
  // Still superdough: the old engine plays the new pattern until the bar line.
  expect(snap(rt).engine).toBe('superdough');
  tick(5);
  await settle();
  expect(snap(rt).engine).toBe('superdough');
  expect(rb.calls.length).toBe(0);
  await play(tick, 10); // past cycle 1
  expect(snap(rt)).toMatchObject({ engine: 'rust', started: true, outputError: null });
  // Superdough had already been sent bar 1 (intro bar 1); it plays it, and nothing after.
  expect(begins(b.calls)).toEqual([0, 1]);
  expect(b.calls[1][0].value.s).toBe('i1');
  await play(tick, 40);
  expect(begins(b.calls)).toEqual([0, 1]);
  // Rust continues from the song position, not from the start: bar 2 is the body.
  expect(begins(rb.calls)).toEqual([2, 3]);
  expect(rb.calls.map(([h]) => h.value.s)).toEqual(['b0', 'b1']);
  expect(rt.position().cycle).toBeGreaterThanOrEqual(3);
  expect(Math.floor(rt.position().located!.songBar)).toBe(3);
  rt.stop();
  expect(snap(rt).engine).toBe('rust');
});

test('a switch without song() continues the pattern where the old engine stopped', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate('s("<a b c>")')).toBe(true);
  tick(10);
  await settle();
  expect(await rt.evaluate('s("<a b c>")', { engine: 'rust' })).toBe(true);
  await play(tick, 35);
  expect(snap(rt).engine).toBe('rust');
  expect(b.calls.map(([h]) => h.value.s)).toEqual(['a', 'b']);
  expect(rb.calls[0][0].whole.begin.valueOf()).toBe(2);
  expect(rb.calls[0][0].value.s).toBe('c');
  rt.stop();
});

/** Every onset of `s("x*8")` from 0 up to `last`, in eighths of a cycle. */
const eighths = (last: number) => Array.from({ length: last * 8 + 1 }, (_, k) => k / 8);

test('a switch sends each hap to exactly one engine, both ways', async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  const DENSE = 's("x*8")';
  const toRust = { engine: 'rust' as const };
  // superdough -> rust
  expect(await rt.evaluate(DENSE)).toBe(true);
  tick(10);
  await settle();
  expect(await rt.evaluate(DENSE, toRust)).toBe(true);
  await play(tick, 30);
  expect(snap(rt).engine).toBe('rust');
  const sd = begins(b.calls);
  const rs = begins(rb.calls);
  // Superdough kept everything it was sent, up to the bar line and past it; rust took over after.
  expect(Math.max(...sd)).toBeGreaterThanOrEqual(1);
  expect(Math.min(...rs)).toBeGreaterThan(Math.max(...sd));
  expect([...sd, ...rs]).toEqual(eighths(Math.max(...rs)));
  // The hand-off hap sounds when superdough would have played it: an eighth (0.25 s) after its last.
  expect(rb.calls[0][1] - b.calls.at(-1)[1]).toBeCloseTo(0.25, 6);
  rt.stop();

  // rust -> superdough
  b.calls.length = 0;
  rb.calls.length = 0;
  const stopsBefore = rb.stops;
  expect(await rt.evaluate(DENSE, toRust)).toBe(true);
  tick(10);
  await settle();
  expect(await rt.evaluate(DENSE)).toBe(true);
  await play(tick, 30);
  expect(snap(rt).engine).toBe('superdough');
  const rs2 = begins(rb.calls);
  const sd2 = begins(b.calls);
  expect(Math.min(...sd2)).toBeGreaterThan(Math.max(...rs2));
  expect([...rs2, ...sd2]).toEqual(eighths(Math.max(...sd2)));
  expect(b.calls[0][1] - rb.calls.at(-1)[1]).toBeCloseTo(0.25, 6);
  // The rust session plays out what it was sent: not stopped at the switch.
  expect(rb.stops).toBe(stopsBefore);
  rt.stop();
  // A stop cuts it at once.
  expect(rb.stops).toBe(stopsBefore + 1);
});

test('the engine left by a switch is stopped once its haps have played', async () => {
  const { rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  tick(10);
  await settle();
  expect(await rt.evaluate(PLAIN)).toBe(true);
  jest.useFakeTimers();
  try {
    tick(15); // past the bar line: the switch
    expect(snap(rt).engine).toBe('superdough');
    expect(rb.stops).toBe(0);
    jest.advanceTimersByTime(DRAIN_RELEASE_SEC * 1000);
    expect(rb.stops).toBe(0);
    jest.advanceTimersByTime((DRAIN_MAX_SEC + DRAIN_RELEASE_SEC) * 1000);
    expect(rb.stops).toBe(1);
  } finally {
    jest.useRealTimers();
  }
  await settle();
  rt.stop();
});

test('stop() before the bar line cancels the switch; the song engine applies at the next start', async () => {
  const { rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(5);
  expect(await rt.evaluate(SONG, { engine: 'rust' })).toBe(true);
  rt.stop();
  // Stopped: the song's engine is active at once, nothing restarted.
  expect(snap(rt)).toMatchObject({ engine: 'rust', started: false });
  tick(30);
  await settle();
  expect(snap(rt).started).toBe(false);
  // Only the evaluate's gesture resumed rust; nothing started on it.
  expect(rb.resumes).toBe(1);
  expect(rb.calls.length).toBe(0);
});

test('switching back before the bar line cancels the switch', async () => {
  const { rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(5);
  expect(await rt.evaluate(SONG, { engine: 'rust' })).toBe(true);
  expect(await rt.evaluate(SONG, { engine: 'superdough' })).toBe(true);
  tick(30);
  await settle();
  expect(snap(rt)).toMatchObject({ engine: 'superdough', started: true });
  // The rust session the first evaluate resumed is ended, unused.
  expect(rb.resumes).toBe(1);
  expect(rb.stops).toBe(1);
  expect(rb.calls.length).toBe(0);
  rt.stop();
});

test("a switch while playing resumes the target inside the evaluate's gesture", async () => {
  const { b, rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(5);
  const toRust = rt.evaluate(SONG, { engine: 'rust' });
  // Synchronously, during the call: not later, from the interval.
  expect(rb.resumes).toBe(1);
  expect(await toRust).toBe(true);
  tick(20);
  await settle();
  expect(snap(rt)).toMatchObject({ engine: 'rust', started: true });
  expect(rb.resumes).toBe(1);
  // And back: superdough is resumed by the evaluate too.
  const resumes = b.resumes;
  b.running = false; // its context was suspended meanwhile
  const toSuperdough = rt.evaluate(SONG);
  expect(b.resumes).toBe(resumes + 1);
  b.running = true;
  expect(await toSuperdough).toBe(true);
  tick(25);
  await settle();
  expect(snap(rt)).toMatchObject({ engine: 'superdough', started: true });
  expect(b.resumes).toBe(resumes + 1);
  rt.stop();
});

const delayed = (ms: number) => () => new Promise<boolean>((r) => setTimeout(() => r(true), ms));

test("the backend's own resume timeout wins over the runtime's", async () => {
  const { rt } = fake({
    rust: true,
    resumeTimeoutMs: 20,
    rustResumeTimeoutMs: 1000,
    rustResume: delayed(60),
  });
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  expect(snap(rt)).toMatchObject({ engine: 'rust', started: true, audio: 'running' });
  rt.stop();
});

test('a resume that completes after its timeout has its session ended', async () => {
  const { rb, rt } = fake({ rust: true, rustResumeTimeoutMs: 20, rustResume: delayed(60) });
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  expect(snap(rt)).toMatchObject({ started: false, audio: 'suspended' });
  expect(rb.stops).toBe(0);
  await new Promise((r) => setTimeout(r, 100));
  expect(rb.resumes).toBe(1);
  expect(rb.stops).toBe(1);
  expect(rb.running).toBe(false);
});

test('a switch whose new engine does not come up stops with a notice', async () => {
  const { rt, tick } = fake({ rust: true, rustResume: async () => false });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(5);
  expect(await rt.evaluate(SONG, { engine: 'rust' })).toBe(true);
  tick(20);
  await settle();
  expect(snap(rt)).toMatchObject({
    started: false,
    engine: 'rust',
    outputError: 'could not restart on the rust engine',
  });
});

test('audition stays unavailable through a delayed engine handoff', async () => {
  const ready: ((running: boolean) => void)[] = [];
  const { rt, tick } = fake({
    rust: true,
    rustResume: () => new Promise<boolean>((resolve) => ready.push(resolve)),
  });
  expect(await rt.evaluate(SONG)).toBe(true);
  tick(5);
  expect(await rt.evaluate(SONG, { engine: 'rust' })).toBe(true);
  tick(20);
  expect(snap(rt)).toMatchObject({ started: false, engine: 'rust' });
  expect(rt.canAudition()).toBe(false);
  expect(await rt.prepareAudition()).toBe(false);
  for (const resume of ready) resume(true);
  await settle();
  expect(snap(rt)).toMatchObject({ started: true, engine: 'rust' });
  rt.stop();
  expect(rt.canAudition()).toBe(true);
});

test('with Rust selected, audition resumes only the preview graph and reuses it', async () => {
  const { rt, b, rb } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { engine: 'rust', autostart: false })).toBe(true);
  const rustResumes = rb.resumes;
  expect(await rt.prepareAudition()).toBe(true);
  expect(await rt.prepareAudition()).toBe(true);
  expect(b.resumes).toBe(1);
  expect(rb.resumes).toBe(rustResumes);
  b.running = false;
  expect(await rt.prepareAudition()).toBe(false);
  expect(b.resumes).toBe(2);
  expect(rb.resumes).toBe(rustResumes);
  rt.stop();
});

test('without a rust backend a rust song plays on superdough, from evaluate and from start', async () => {
  const { b, rt, tick } = fake();
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  // An autostarting evaluate plays on superdough (a plain browser, the host, the render page).
  expect(snap(rt)).toMatchObject({ engine: 'superdough', started: true });
  tick(5);
  await settle();
  expect(b.calls.length).toBeGreaterThan(0);
  rt.stop();
  // So does a plain start, and a start that asks for rust.
  expect(await rt.start()).toBe(true);
  expect(snap(rt)).toMatchObject({ engine: 'superdough', started: true, outputError: null });
  rt.stop();
  expect(await rt.start({ backend: 'rust' })).toBe(true);
  expect(snap(rt)).toMatchObject({ engine: 'superdough', started: true, outputError: null });
  rt.stop();
});

test('warnings dedupe by signature, clear at each start and are ignored on superdough', async () => {
  const { rb, rt } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { autostart: false })).toBe(true);
  rb.warn({ signature: 's=foo', keys: ['vowel'], firstCycle: 0 });
  expect(snap(rt).engineWarnings).toEqual([]);
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  const w1 = { signature: 's=foo', keys: ['vowel'], firstCycle: 1 };
  const w2 = { signature: 's=bar', keys: [], firstCycle: 2 };
  rb.warn(w1);
  rb.warn({ ...w1, firstCycle: 3 });
  rb.warn(w2);
  expect(snap(rt).engineWarnings).toEqual([w1, w2]);
  rt.stop();
  // Kept after the play, for the user to read; a new play starts clean.
  expect(snap(rt).engineWarnings).toEqual([w1, w2]);
  expect(await rt.start()).toBe(true);
  expect(snap(rt).engineWarnings).toEqual([]);
  rt.stop();
});

test('live mirrors the rust stats once per change and is null after a swap', async () => {
  const { rb, rt, tick } = fake({ rust: true });
  expect(await rt.evaluate(PLAIN, { engine: 'rust' })).toBe(true);
  let wakes = 0;
  let last: LiveStats | null = null;
  const off = rt.store.subscribe((s) => {
    if (s.live !== last) wakes++;
    last = s.live;
  });
  wakes = 0;
  tick(3);
  expect(snap(rt).live).toBeNull();
  const a = stats(0);
  rb.stats = a;
  tick(5);
  expect(snap(rt).live).toBe(a);
  expect(wakes).toBe(1);
  const c = stats(1);
  rb.stats = c;
  tick(5);
  expect(snap(rt).live).toBe(c);
  expect(wakes).toBe(2);
  rt.stop();
  expect(await rt.start({ backend: 'superdough' })).toBe(true);
  expect(snap(rt).live).toBeNull();
  tick(5);
  expect(snap(rt).live).toBeNull();
  off();
  rt.stop();
});

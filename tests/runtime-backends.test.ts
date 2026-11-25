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
import { layoutClock } from '../src/lib/strudel/timing';
import { INITIAL_SEGMENTS } from '../src/lib/strudel/transport';
import { parseMeta } from '../src/lib/project/meta';
import { evaluateOptions } from '../src/lib/project/evaluate-options';
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

const MIXED_HEADER = `const meta = {
  meter: '4/4', sections: [{ name: 'phrase', bars: 3 }],
  meterChanges: [{ bar: 2, meter: '3/8' }, { bar: 3, meter: '4/4' }]
};`;
const MIXED_SONG = `${MIXED_HEADER}
stack(song(meta, {phrase: s('held').slow(2)}), s('outside').loopAt(1))`;

test.each(['superdough', 'rust'] as const)(
  '%s schedules the final mixed-meter pattern with musical seeds, elapsed phase and local CPS',
  async (engine) => {
    const { rt, b, rb, tick } = fake({ rust: true });
    expect(await rt.evaluate(MIXED_SONG, { engine })).toBe(true);
    const downbeat = rt.cycleTime(0)!;
    expect(rt.cycleTime(2)! - downbeat).toBeCloseTo(2.75);
    const haps = rt.query(0, 3)!.filter((h) => h.hasOnset() && h.value.s === 'outside');
    expect(haps.map((h) => Number(h.whole.begin))).toEqual([0, 1, 2]);
    expect(haps.map((h) => h.value.speed)).toEqual([0.5, 4 / 3, 0.5]);
    tick(33);
    const calls = (engine === 'rust' ? rb : b).calls;
    const outside = calls.filter(([h]) => h.value.s === 'outside');
    expect(outside.map(([h]) => Number(h.whole.begin))).toEqual([0, 1, 2]);
    [0, 2, 2.75].forEach((seconds, i) =>
      expect(outside[i]![1] - downbeat).toBeCloseTo(seconds, 10),
    );
    expect(outside.map(([, , cps, timing]) => [cps, timing])).toEqual([
      [0.5, { phaseSec: 0, durationSec: 2 }],
      [4 / 3, { phaseSec: 2, durationSec: 0.75 }],
      [0.5, { phaseSec: 2.75, durationSec: 2 }],
    ]);
    const held = calls.filter(([h]) => h.value.s === 'held');
    expect(held.map(([h]) => Number(h.whole.begin))).toEqual([0, 2]);
    expect(held[0][3].durationSec).toBe(2.75);
    expect(rt.drawingScheduler.now()).toBeCloseTo(rt.position().cycle);
    rt.stop();
  },
);

test('mixed-meter CPS changes produce a diagnostic and preserve the reference clock', async () => {
  const { rt } = fake();
  expect(await rt.evaluate(MIXED_SONG, { autostart: false })).toBe(true);
  expect(await rt.evaluate(`${MIXED_HEADER}\nsetcps(2); song(meta, {phrase:s('click')})`)).toBe(
    false,
  );
  expect(snap(rt).evalError).toContain('cannot change CPS');
  expect(rt.query(0, 1)!.some((h) => h.value.s === 'held')).toBe(true);
  expect(await rt.evaluate('s("bd")', { expectedStructure: snap(rt).structure! })).toBe(false);
  expect(snap(rt).evalError).toContain('requires song(');
  expect(await rt.evaluate(`${MIXED_HEADER}\nsong(meta, {phrase:s('click').cps(3)})`)).toBe(true);
  expect(snap(rt).outputError).toContain('cannot change CPS');
  expect(rt.scheduler.cps).toBe(0.5);
  expect(() => rt.query(0, 1)).toThrow('cannot change CPS');
  rt.stop();
});

test('count-in and a manual range start at the armed section’s meter', async () => {
  const code = `const meta = {
    sections: [{ name: 'intro', bars: 1 }, { name: 'a', bars: 1 }, { name: 'b', bars: 1 }],
    meterChanges: [{ bar: 2, meter: '3/8' }, { bar: 3, meter: '4/4' }]
  }; song(meta, {intro: s('intro'), a: s('a'), b: s('b')})`;
  const { rt, b, tick } = fake();
  expect(await rt.evaluate(code, { autostart: false })).toBe(true);
  rt.transport.loopRange(1, 2, snap(rt).structure!);
  expect(await rt.start({ countIn: true })).toBe(true);
  const at = rt.cycleTime(0)!;
  expect(rt.cycleTime(1)! - at).toBeCloseTo(0.75);
  tick(44);
  expect(b.calls.map(([h]) => h.value.s)).toEqual(['a', 'b', 'a', 'b']);
  [0.75, 1.5, 3.5, 4.25].forEach((seconds, i) =>
    expect(b.calls[i]![1] - at).toBeCloseTo(seconds, 10),
  );
  rt.stop();
});

test('mixed-meter backend handoff keeps logical onsets continuous after a shortened bar', async () => {
  const { rt, b, rb, tick } = fake({ rust: true });
  const code = `${MIXED_HEADER}\nsong(meta, {phrase: s('click')}).fast(4)`;
  // Keep the arrangement running past the first handoff.
  const looping = code.replace("name: 'phrase',", "name: 'phrase', loop: true,");
  expect(await rt.evaluate(looping)).toBe(true);
  await play(tick, 24);
  expect(await rt.evaluate(looping, { engine: 'rust' })).toBe(true);
  await play(tick, 26);
  const calls = [...b.calls, ...rb.calls];
  expect(rb.calls.length).toBeGreaterThan(0);
  expect(calls.map(([hap]) => Number(hap.whole.begin))).toEqual(calls.map((_, i) => i / 4));
  rt.stop();
});

test('editing a mixed meter preserves the play position at the scheduler handoff', async () => {
  const { rt, b, tick } = fake();
  expect(await rt.evaluate(MIXED_SONG)).toBe(true);
  tick(25);
  const last = rt.scheduler.lastEnd;
  const anchor = rt.position().cycle;
  const replacement = MIXED_SONG.replace("meter: '3/8'", "meter: '2/4'");
  expect(await rt.evaluate(replacement)).toBe(true);
  expect(rt.scheduler.lastEnd).toBe(last);
  expect(rt.position().cycle).toBeCloseTo(anchor, 10);
  tick(12);
  const outside = b.calls.filter(([h]) => h.value.s === 'outside');
  expect(outside.map(([h]) => Number(h.whole.begin))).toEqual([0, 1, 2]);
  rt.stop();
});

test('repeated live tempo changes take effect as their scheduled mixed-meter audio reaches the playhead', async () => {
  const { rt, b, tick } = fake();
  expect(await rt.evaluate(MIXED_SONG)).toBe(true);
  tick(26);
  const clock = layoutClock(snap(rt).structure!, INITIAL_SEGMENTS);
  const start = b.now;
  const baseline = rt.scheduler.now();
  const first = rt.scheduler.lastEnd;
  const firstSec = (first - baseline) / 0.5;
  const checkReference = (reference: number) => {
    expect(rt.position().cycle).toBeCloseTo(clock.toCycle(reference), 10);
    expect(rt.drawingScheduler.now()).toBeCloseTo(rt.position().cycle, 10);
  };
  expect(await rt.evaluate(MIXED_SONG, { cps: 1.5 })).toBe(true);
  checkReference(baseline);
  tick(1);
  checkReference(baseline + (b.now - start) * 0.5);
  const second = rt.scheduler.lastEnd;
  const secondSec = firstSec + (second - first) / 1.5;
  expect(await rt.evaluate(MIXED_SONG, { cps: 0.25 })).toBe(true);
  checkReference(baseline + (b.now - start) * 0.5);
  for (let i = 0; i < 7; i++) {
    tick(1);
    const elapsed = b.now - start;
    const reference =
      baseline +
      Math.min(elapsed, firstSec) * 0.5 +
      Math.max(0, Math.min(elapsed, secondSec) - firstSec) * 1.5 +
      Math.max(0, elapsed - secondSec) * 0.25;
    checkReference(reference);
  }
  rt.stop();
});

test('getCps follows the audible meter and tempo across a change to both', async () => {
  const { rt, tick } = fake();
  expect(await rt.evaluate(MIXED_SONG)).toBe(true);
  expect(core.getCps()).toBe(0.5);
  tick(22);
  expect(core.getCps()).toBeCloseTo(4 / 3);
  const changed = MIXED_SONG.replace("meter: '4/4', sections", "meter: '2/4', sections");
  expect(await rt.evaluate(changed, { cps: 1.5, meter: { num: 2, den: 4 } })).toBe(true);
  expect(core.getCps()).toBeCloseTo(4 / 3);
  tick(3);
  expect(core.getCps()).toBeCloseTo(2);
  tick(6);
  expect(core.getCps()).toBeCloseTo(0.75);
  rt.stop();
  expect(core.getCps()).toBe(1.5);
});

test('a drawing window preserves painter order and repetitions across meter boundaries', async () => {
  await import('@strudel/draw');
  const paintA = () => {};
  const paintB = () => {};
  await core.evalScope({ paintA, paintB });
  const { rt } = fake();
  const code = `const meta = {
    sections: [{ name: 'one', bars: 1 }, { name: 'two', bars: 2 }],
    meterChanges: [{ bar: 2, meter: '3/8' }, { bar: 3, meter: '4/4' }]
  };
  song(meta, {one: s('click').onPaint(paintA), two: s('click').onPaint(paintB).onPaint(paintA)}).onPaint(paintA)`;
  expect(await rt.evaluate(code, { autostart: false })).toBe(true);
  const painters: unknown[] = [];
  rt.drawingScheduler.pattern.queryArc(0, 3, { painters });
  expect(painters).toEqual([paintA, paintA, paintB]);
});

test('a fixed static header allows a dynamic legacy song but rejects dynamic mixed meter', async () => {
  const { rt } = fake();
  const header = "const meta = { sections: [{ name: 'part', bars: 3 }] };";
  const options = { ...evaluateOptions(parseMeta(header).meta), autostart: false };
  expect(
    await rt.evaluate(`${header}\nsong({...meta, meter: '3/8'}, {part: s('click')})`, options),
  ).toBe(true);
  const changed = `${header}\nsong({...meta, meter: '3/8', meterChanges: [{bar: 2, meter: '4/4'}]}, {part: s('click')})`;
  expect(await rt.evaluate(changed, options)).toBe(false);
  expect(snap(rt).evalError).toContain('arrangement in the meta header');
});

test.each([
  ['.cps(2)', 'cannot change CPS'],
  ['.cpm(60)', 'do not support .cpm()'],
])(
  'a live edit with an imminent %s conflict preserves the playing pattern',
  async (op, message) => {
    const { rt, b, tick } = fake();
    expect(await rt.evaluate(MIXED_SONG)).toBe(true);
    tick(12);
    const pattern = rt.scheduler.pattern;
    const position = rt.position().cycle;
    const replacement = `${MIXED_HEADER}\nsong(meta, {phrase: s('replacement')${op}})`;
    expect(await rt.evaluate(replacement, { cps: 1.5 })).toBe(false);
    expect(snap(rt)).toMatchObject({
      activeCode: MIXED_SONG,
      cps: 0.5,
      outputError: null,
      started: true,
    });
    expect(snap(rt).evalError).toContain(message);
    expect(rt.scheduler.pattern).toBe(pattern);
    expect(rt.position().cycle).toBeCloseTo(position, 10);
    tick(20);
    expect(b.calls.filter(([h]) => h.value.s === 'outside')).toHaveLength(3);
    rt.stop();
  },
);

test('prepared mixed-meter queries diagnose .cpm() while fixed-meter patterns retain its absolute rate', async () => {
  const { rt } = fake();
  expect(
    await rt.evaluate(`${MIXED_HEADER}\nsong(meta, {phrase: s('click')}).cpm(60)`, {
      autostart: false,
    }),
  ).toBe(true);
  expect(() => rt.query(1, 2)).toThrow('do not support .cpm()');
  expect(await rt.evaluate("s('click').cpm(60)", { autostart: false, cps: 0.5 })).toBe(true);
  expect(rt.query(0, 1)!.map((hap) => Number(hap.whole.begin))).toEqual([0, 0.5]);
});

test('a failed mixed-meter autostart leaves the last accepted pattern and its clock installed', async () => {
  const { rt } = fake();
  expect(await rt.evaluate(MIXED_SONG, { autostart: false })).toBe(true);
  const start = rt.scheduler.start;
  rt.scheduler.start = async () => {
    throw new Error('clock failed');
  };
  try {
    expect(await rt.evaluate('s("replacement")')).toBe(false);
    expect(rt.query(0, 1)!.some((h) => h.value.s === 'held')).toBe(true);
  } finally {
    rt.scheduler.start = start;
  }
  expect(await rt.start()).toBe(true);
  expect(rt.cycleTime(2)! - rt.cycleTime(0)!).toBeCloseTo(2.75);
  rt.stop();
});

test('a dominant custom mixed-meter output receives the logical onset and measured gate', async () => {
  const { rt, b, tick } = fake();
  const calls: number[][] = [];
  await core.evalScope({
    captureMeteredGate: (h: any, t: number, cps: number) =>
      calls.push([Number(h.whole.begin), t, Number(h.duration) / cps, cps]),
  });
  expect(
    await rt.evaluate(`${MIXED_HEADER}
song(meta, {phrase: s('held').slow(2)}).onSchedule(captureMeteredGate)`),
  ).toBe(true);
  const at = rt.cycleTime(0)!;
  tick(30);
  expect(calls.map(([onset]) => onset)).toEqual([0, 2]);
  expect(calls[0]![2]).toBeCloseTo(2.75);
  expect(calls[1]![1] - at).toBeCloseTo(2.75);
  expect(b.calls).toHaveLength(0);
  rt.stop();
});

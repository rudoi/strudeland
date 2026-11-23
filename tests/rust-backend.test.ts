import { afterEach, describe, expect, jest, test } from 'bun:test';
import { Fraction, Hap, TimeSpan } from '@strudel/core';
import {
  ANCHOR_TIMEOUT_MS,
  RESUME_TIMEOUT_MS,
  createRustBackend,
  MAX_VALUES,
  REMEASURE_AFTER_MS,
  rttSummary,
  TOO_MANY_VALUES,
} from '../src/lib/audio/rust';
import { SAMPLE_RATE } from '../src/lib/render/contract';
import { canonicalJson, encodeControlMap } from '../src/lib/render/engine-job';
import { fakePlaybackApi } from './helpers/fake-playback-api';
import { SCOPE_BUFFER_SIZE } from '../src/lib/scope/clock';
import { triggerIndex } from '../src/lib/scope/trace';

/** `hostNowNs` minus `performance.now()` on the fake clocks. */
const OFFSET_NS = 5_000_000_000_000n;

/** A fake `performance.now()` and a host clock `OFFSET_NS` ahead of it. */
function setup() {
  const clock = { ms: 1000 };
  const nowMs = () => clock.ms;
  const { api, fake } = fakePlaybackApi({
    hostNs: () => OFFSET_NS + BigInt(Math.round(clock.ms * 1e6)),
  });
  const backend = createRustBackend(api, { nowMs });
  return { clock, api, fake, backend };
}

/** A hap whose whole is `[begin, end)` cycles. */
const hap = (begin: number, end: number, value: unknown, noWhole = false): any =>
  new Hap(
    noWhole ? undefined : new TimeSpan(Fraction(begin), Fraction(end)),
    new TimeSpan(Fraction(begin), Fraction(end)),
    value,
    {},
  );

/** Lets the queued flush (a microtask) and the fake's promises run. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

/** Opens a session with an anchor and a measured offset. */
async function running(s: ReturnType<typeof setup>, frame = 44100) {
  const resumed = s.backend.resume();
  await settle();
  s.fake.emit({ kind: 'anchor', frame, hostNs: String(OFFSET_NS + BigInt(s.clock.ms * 1e6)) });
  expect(await resumed).toBe(true);
  await s.backend.init();
}

afterEach(() => jest.useRealTimers());

describe('rttSummary', () => {
  test('dsp nearestRank quantiles; zeros when empty', () => {
    expect(rttSummary([])).toEqual({ p50: 0, p95: 0, max: 0 });
    expect(rttSummary([3])).toEqual({ p50: 3, p95: 3, max: 3 });
    const hundred = Array.from({ length: 100 }, (_, i) => 100 - i); // 100..1
    // sorted[round(0.5 * 99)] and sorted[round(0.95 * 99)]
    expect(rttSummary(hundred)).toEqual({ p50: 51, p95: 95, max: 100 });
  });
});

describe('rust backend: batching', () => {
  test('id is rust, no graph', () => {
    const { backend } = setup();
    expect(backend.id).toBe('rust');
    expect('graph' in backend).toBe(false);
  });

  test('output before a session sends nothing', async () => {
    const s = setup();
    s.backend.output(hap(0, 1, { s: 'sine' }), 0.1, 1);
    await settle();
    expect(s.fake.batches).toEqual([]);
  });

  test('many haps in one tick: one invoke; a repeated value crosses once', async () => {
    const s = setup();
    await running(s);
    const v = { s: 'sine', note: 60 };
    for (let i = 0; i < 4; i++) s.backend.output(hap(i / 4, (i + 1) / 4, { ...v }), 1 + i / 4, 1);
    expect(s.fake.batches).toEqual([]); // not before the microtask
    await settle();
    expect(s.fake.batches).toHaveLength(1);
    const [b] = s.fake.batches;
    expect(b!.values).toHaveLength(1);
    expect(b!.haps.map((h) => h[3])).toEqual([0, 0, 0, 0]);
    // The next tick: the same value is not sent again.
    s.backend.output(hap(1, 1.25, { ...v }), 2, 1);
    await settle();
    expect(s.fake.batches).toHaveLength(2);
    expect(s.fake.batches[1]!.values).toEqual([]);
    expect(s.fake.batches[1]!.haps).toEqual([[2, 0.25, 1, 0]]);
    expect(s.fake.of('schedule').every((a) => a[0] === 1)).toBe(true);
  });

  test('the batch shape: three-element values, orbit spec once per orbit, hap tuples', async () => {
    const s = setup();
    await running(s);
    const a = { s: 'sine', gain: 0.5 };
    const b = { s: 'saw' }; // orbit 1 again
    const c = { s: 'square', orbit: 2 };
    const d = { s: 'tri', orbit: 2 };
    s.backend.output(hap(0, 0.5, a), 1, 0.5);
    s.backend.output(hap(0.5, 1, b), 2, 0.5);
    s.backend.output(hap(1, 2, c), 3, 0.5);
    s.backend.output(hap(2, 3, d), 5, 0.5);
    await settle();
    const [batch] = s.fake.batches;
    expect(JSON.parse(JSON.stringify(batch))).toEqual({
      values: [
        [0, encodeControlMap(a), { orbit: 1, channels: [0, 1] }],
        [1, encodeControlMap(b), null],
        [2, encodeControlMap(c), { orbit: 2, channels: [0, 1] }],
        [3, encodeControlMap(d), null],
      ],
      haps: [
        [1, 0.5 / 0.5, 0, 0],
        [2, 0.5 / 0.5, 0.5, 1],
        [3, 1 / 0.5, 1, 2],
        [5, 1 / 0.5, 2, 3],
      ],
      cps: 0.5,
    });
    for (const entry of batch!.values) expect(entry).toHaveLength(3);
  });

  test('values are keyed by canonical JSON (key order does not matter)', async () => {
    const s = setup();
    await running(s);
    s.backend.output(hap(0, 1, { s: 'sine', note: 60 }), 1, 1);
    s.backend.output(hap(1, 2, { note: 60, s: 'sine' }), 2, 1);
    await settle();
    expect(canonicalJson(encodeControlMap({ s: 'sine', note: 60 }))).toBe(
      canonicalJson(encodeControlMap({ note: 60, s: 'sine' })),
    );
    expect(s.fake.batches[0]!.values).toHaveLength(1);
  });

  test('a cps change inside one tick gives two invokes, in order', async () => {
    const s = setup();
    await running(s);
    s.backend.output(hap(0, 1, { s: 'a' }), 1, 1);
    s.backend.output(hap(1, 2, { s: 'b' }), 2, 2);
    s.backend.output(hap(2, 3, { s: 'a' }), 3, 2);
    await settle();
    const bs = s.fake.batches;
    expect(bs.map((b) => b.cps)).toEqual([1, 2]);
    expect(bs[0]!.haps).toEqual([[1, 1, 0, 0]]);
    expect(bs[1]!.values.map((v) => v[0])).toEqual([1]);
    expect(bs[1]!.haps).toEqual([
      [2, 0.5, 1, 1],
      [3, 0.5, 2, 0],
    ]);
  });

  test('a mixed-meter hap carries its measured gate, logical onset and elapsed phase', async () => {
    const s = setup();
    await running(s);
    s.backend.output(hap(2, 4, { s: 'sine' }), 9, 4 / 3, { durationSec: 2.75, phaseSec: 3.5 });
    await settle();
    expect(s.fake.batches[0]!.cps).toBe(4 / 3);
    expect(s.fake.batches[0]!.haps).toEqual([[9, 2.75, 2, 0, 3.5]]);
  });

  test('a non-finite t, a hap without whole and a non-finite cycle are skipped', async () => {
    const s = setup();
    await running(s);
    s.backend.output(hap(0, 1, { s: 'a' }), Number.NaN, 1);
    s.backend.output(hap(0, 1, { s: 'a' }), Infinity, 1);
    s.backend.output(hap(0, 1, { s: 'a' }, true), 1, 1);
    s.backend.output(hap(0, 1, { s: 'a' }), 1, 0); // duration Infinity
    await settle();
    expect(s.fake.batches).toEqual([]);
  });

  test('a non-object value throws (ensureObjectValue)', async () => {
    const s = setup();
    await running(s);
    expect(() => s.backend.output(hap(0, 1, 'x'), 1, 1)).toThrow();
  });

  test(`the ${MAX_VALUES} value cap throws, drops the hap and ends the session`, async () => {
    const s = setup();
    await running(s);
    for (let i = 0; i < MAX_VALUES; i++) s.backend.output(hap(0, 1, { s: 'a', n: i }), 1, 1);
    expect(() => s.backend.output(hap(0, 1, { s: 'a', n: -1 }), 1, 1)).toThrow(TOO_MANY_VALUES);
    await settle();
    expect(s.fake.of('stop')).toEqual([[1]]);
    expect(s.backend.isRunning()).toBe(false);
    // The haps before the cap were still queued when the session ended: nothing crosses.
    expect(s.fake.batches).toEqual([]);
  });
});

describe('rust backend: errors', () => {
  test('a stale schedule rejection is ignored; another kind throws on the next output, once', async () => {
    const s = setup();
    await running(s);
    s.fake.failNext('stale', 'schedule');
    s.backend.output(hap(0, 1, { s: 'a' }), 1, 1);
    await settle();
    expect(() => s.backend.output(hap(1, 2, { s: 'a' }), 2, 1)).not.toThrow();
    await settle();
    s.fake.failNext('badBatch', 'schedule');
    s.backend.output(hap(2, 3, { s: 'a' }), 3, 1);
    await settle();
    expect(() => s.backend.output(hap(3, 4, { s: 'a' }), 4, 1)).toThrow('fake schedule failed');
    expect(() => s.backend.output(hap(4, 5, { s: 'a' }), 5, 1)).not.toThrow();
  });

  test('an error event makes the next output throw once', async () => {
    const s = setup();
    await running(s);
    s.fake.emit({ kind: 'error', message: 'unknown value index 9' });
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).toThrow('unknown value index 9');
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).not.toThrow();
  });
});

describe('rust backend: resume, init and the clock', () => {
  test('resume resolves true on the first anchor; single-flight; isRunning needs anchor and offset', async () => {
    const s = setup();
    const a = s.backend.resume();
    const b = s.backend.resume();
    expect(a).toBe(b);
    await settle();
    expect(s.fake.of('start')).toEqual([[0]]);
    expect(s.backend.isRunning()).toBe(false);
    s.fake.emit({ kind: 'anchor', frame: 0, hostNs: '1' });
    expect(await a).toBe(true);
    expect(s.backend.isRunning()).toBe(false); // no offset yet
    await s.backend.init();
    expect(s.backend.isRunning()).toBe(true);
    expect(await s.backend.resume()).toBe(true);
    expect(s.fake.of('start')).toHaveLength(1);
  });

  test('resume resolves false after 2 s without an anchor; the next resume opens a fresh device stream', async () => {
    jest.useFakeTimers();
    const s = setup();
    const first = s.backend.resume();
    await settle();
    jest.advanceTimersByTime(ANCHOR_TIMEOUT_MS - 1);
    let done = false;
    void first.then(() => (done = true));
    await settle();
    expect(done).toBe(false);
    jest.advanceTimersByTime(1);
    expect(await first).toBe(false);
    const second = s.backend.resume();
    await settle();
    s.fake.emit({ kind: 'anchor', frame: 10, hostNs: '1' });
    expect(await second).toBe(true);
    expect(s.fake.of('start')).toHaveLength(2);
  });

  test('a rejected start propagates', async () => {
    const s = setup();
    s.fake.failNext('output', 'start');
    await expect(s.backend.resume()).rejects.toThrow('fake start failed');
  });

  test('init is idempotent', async () => {
    const s = setup();
    await Promise.all([s.backend.init(), s.backend.init()]);
    const pings = s.fake.pings;
    expect(pings).toBeGreaterThan(0);
    await s.backend.init();
    expect(s.fake.pings).toBe(pings);
  });

  test('getTime: 0 before an anchor, frame/44100 + elapsed after, follows a later anchor', async () => {
    const s = setup();
    await s.backend.init();
    expect(s.backend.getTime()).toBe(0);
    const resumed = s.backend.resume();
    await settle();
    expect(s.backend.getTime()).toBe(0);
    // Frame 88200 is heard at the host time of performance.now() = 1000 ms.
    const heardAt = OFFSET_NS + 1_000_000_000n;
    s.fake.emit({ kind: 'anchor', frame: 88200, hostNs: String(heardAt) });
    await resumed;
    expect(s.backend.getTime()).toBeCloseTo(2, 9);
    s.clock.ms = 1250;
    expect(s.backend.getTime()).toBeCloseTo(2.25, 9);
    // A later anchor: frame 100000 heard at 1200 ms.
    s.fake.emit({ kind: 'anchor', frame: 100000, hostNs: String(OFFSET_NS + 1_200_000_000n) });
    expect(s.backend.getTime()).toBeCloseTo(100000 / SAMPLE_RATE + 0.05, 9);
  });

  test('an anchor more than 60 s after the last measurement re-measures the offset', async () => {
    const s = setup();
    await running(s);
    const pings = s.fake.pings;
    s.clock.ms += REMEASURE_AFTER_MS - 1;
    s.fake.emit({ kind: 'anchor', frame: 0, hostNs: String(OFFSET_NS) });
    await settle();
    expect(s.fake.pings).toBe(pings);
    s.clock.ms += 2;
    // The host clock moved 3 ms ahead of performance.now(): the new offset follows it.
    const old = s.fake.hostNs;
    s.fake.hostNs = () => old() + 3_000_000n;
    const anchorHost = OFFSET_NS + 3_000_000n + BigInt(s.clock.ms * 1e6);
    s.fake.emit({ kind: 'anchor', frame: 44100, hostNs: String(anchorHost) });
    for (let i = 0; i < 20; i++) await settle();
    expect(s.fake.pings).toBeGreaterThan(pings);
    expect(s.backend.getTime()).toBeCloseTo(1, 9);
  });
});

test('the resume timeout covers the native start and the first anchor', () => {
  // START_TIMEOUT in crates/strudeland-live/src/player.rs
  const NATIVE_START_MS = 5000;
  expect(setup().backend.resumeTimeoutMs).toBe(RESUME_TIMEOUT_MS);
  expect(RESUME_TIMEOUT_MS).toBeGreaterThan(NATIVE_START_MS + ANCHOR_TIMEOUT_MS);
});

describe('rust backend: seed', () => {
  test('before resume the seed goes to start; after it to setSeed(session, seed)', async () => {
    const s = setup();
    s.backend.setSeed(42);
    expect(s.fake.of('setSeed')).toEqual([]);
    await running(s);
    expect(s.fake.of('start')).toEqual([[42]]);
    s.backend.setSeed(-3);
    expect(s.fake.of('setSeed')).toEqual([[1, -3]]);
  });

  test('the same seed again is not sent', async () => {
    const s = setup();
    await running(s);
    s.backend.setSeed(0);
    s.backend.setSeed(5);
    s.backend.setSeed(5);
    expect(s.fake.of('setSeed')).toEqual([[1, 5]]);
  });

  test('a setSeed failure other than stale throws on the next output', async () => {
    const s = setup();
    await running(s);
    s.fake.failNext('stale', 'setSeed');
    s.backend.setSeed(1);
    await settle();
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).not.toThrow();
    s.fake.failNext('engine', 'setSeed');
    s.backend.setSeed(2);
    await settle();
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).toThrow('fake setSeed failed');
  });
});

describe('rust backend: scope', () => {
  for (const len of [512, 1024]) {
    test(`readScope with a ${len}-sample buffer: false before a frame, tail copy after`, async () => {
      const s = setup();
      await running(s);
      const buf = new Float32Array(len).fill(9);
      expect(s.backend.readScope!(buf)).toBe(false);
      const samples = Array.from({ length: 1024 }, (_, i) => i / 1024);
      s.fake.emit({ kind: 'scope', samples });
      expect(s.backend.readScope!(buf)).toBe(true);
      expect(Array.from(buf)).toEqual(Array.from(Float32Array.from(samples.slice(1024 - len))));
    });
  }

  test('readScope with the scope clock’s 2048-sample buffer stretches the frame across all of it', async () => {
    const s = setup();
    await running(s);
    const buf = new Float32Array(SCOPE_BUFFER_SIZE).fill(9);
    const samples = Array.from({ length: 1024 }, (_, i) => Math.sin((2 * Math.PI * i) / 100));
    s.fake.emit({ kind: 'scope', samples });
    expect(s.backend.readScope!(buf)).toBe(true);
    // Both ends are the frame's ends and nothing is left as padding.
    expect(buf[0]).toBeCloseTo(samples[0]!, 6);
    expect(buf[buf.length - 1]).toBeCloseTo(samples[1023]!, 6);
    expect(buf.every((v) => v !== 9)).toBe(true);
    // The scope's trigger search (the first half) finds a rising edge, so the trace moves.
    expect(triggerIndex(buf)).toBeGreaterThan(0);
  });

  test('a frame older than 500 ms is no signal', async () => {
    const s = setup();
    await running(s);
    s.fake.emit({ kind: 'scope', samples: new Array(1024).fill(0.5) });
    s.clock.ms += 500;
    expect(s.backend.readScope!(new Float32Array(16))).toBe(true);
    s.clock.ms += 1;
    expect(s.backend.readScope!(new Float32Array(16))).toBe(false);
  });
});

describe('rust backend: levels and the limiter', () => {
  const frame = {
    peak: [0.5, 1.25] as [number, number],
    rms: [0.25, 0.125] as [number, number],
    overs: [0, 3] as [number, number],
    limitDb: -2.5,
  };

  test('a levels event reaches listeners without its kind; the remover works', async () => {
    const s = setup();
    await running(s);
    const got: unknown[] = [];
    const remove = s.backend.onLevels((f) => got.push(f));
    s.fake.emit({ kind: 'levels', ...frame });
    expect(got).toEqual([frame]);
    expect(got[0]).not.toHaveProperty('kind');
    remove();
    s.fake.emit({ kind: 'levels', ...frame });
    expect(got).toHaveLength(1);
  });

  test('setLimiter reaches the api only with a session', async () => {
    const s = setup();
    s.backend.setLimiter(true);
    await settle();
    expect(s.fake.of('setLimiter')).toEqual([]);
    await running(s);
    expect(s.fake.of('setLimiter')).toEqual([[1, true]]);
    s.backend.setLimiter(false);
    await settle();
    expect(s.fake.of('setLimiter')).toEqual([
      [1, true],
      [1, false],
    ]);
    expect(s.backend.limiterAvailable()).toBe(true);
  });

  test('open sends the stored preference after start, and again for a later session', async () => {
    const s = setup();
    s.backend.setLimiter(true);
    await running(s);
    expect(s.fake.calls.map((c) => c.method).filter((m) => m !== 'schedule')).toEqual([
      'start',
      'setLimiter',
    ]);
    expect(s.fake.of('setLimiter')).toEqual([[1, true]]);
    await s.backend.stop();
    await running(s);
    expect(s.fake.of('setLimiter')).toEqual([
      [1, true],
      [2, true],
    ]);
  });

  test('an off preference sends nothing at open', async () => {
    const s = setup();
    await running(s);
    expect(s.fake.of('setLimiter')).toEqual([]);
  });

  test('a rejected setLimiter surfaces at the next output, a stale one is ignored', async () => {
    const s = setup();
    await running(s);
    s.fake.failNext('stale', 'setLimiter');
    s.backend.setLimiter(true);
    await settle();
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).not.toThrow();
    s.fake.failNext('engine', 'setLimiter');
    s.backend.setLimiter(false);
    await settle();
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).toThrow();
  });
});

describe('rust backend: warnings and stats', () => {
  test('warning events reach listeners; the remover works', async () => {
    const s = setup();
    await running(s);
    const got: unknown[] = [];
    const remove = s.backend.onWarning((w) => got.push(w));
    const w = { signature: 's:sample', keys: ['s'], firstCycle: 3 };
    s.fake.emit({ kind: 'warning', ...w });
    expect(got).toEqual([w]);
    remove();
    s.fake.emit({ kind: 'warning', ...w });
    expect(got).toHaveLength(1);
  });

  test('stats: invokeRttMs from the schedule round trips, a stable reference between events', async () => {
    const s = setup();
    await running(s);
    expect(s.backend.liveStats()).toBeNull();
    s.fake.holdSchedules = true;
    for (const ms of [4, 2, 8]) {
      s.backend.output(hap(0, 1, { s: 'a' }), 1, 1);
      await settle(); // the flush invoked schedule
      s.clock.ms += ms;
      s.fake.release();
      await settle();
    }
    const engine = {
      underruns: 0,
      lateHaps: 1,
      haps: 3,
      batches: 3,
      skippedHaps: 0,
      renderHeadroomMs: 38.5,
      batchDecodeUs: { p50: 12, max: 40 },
    };
    s.fake.emit({ kind: 'stats', ...engine });
    const first = s.backend.liveStats();
    expect(first).toEqual({ ...engine, invokeRttMs: { p50: 4, p95: 8, max: 8 } });
    expect(s.backend.liveStats()).toBe(first!);
    s.fake.emit({ kind: 'stats', ...engine, haps: 4 });
    expect(s.backend.liveStats()).not.toBe(first!);
    expect(s.backend.liveStats()!.haps).toBe(4);
  });
});

describe('rust backend: stop', () => {
  test('stop ends the session and clears the interning; a value is re-sent after resume', async () => {
    const s = setup();
    await running(s);
    s.backend.output(hap(0, 1, { s: 'a' }), 1, 1);
    await settle();
    s.fake.emit({
      kind: 'stats',
      underruns: 0,
      lateHaps: 0,
      haps: 1,
      batches: 1,
      skippedHaps: 0,
      renderHeadroomMs: 40,
      batchDecodeUs: { p50: 1, max: 1 },
    });
    await s.backend.stop();
    expect(s.fake.of('stop')).toEqual([[1]]);
    expect(s.backend.isRunning()).toBe(false);
    expect(s.backend.getTime()).toBe(0);
    expect(s.backend.liveStats()).toBeNull();
    s.backend.output(hap(1, 2, { s: 'a' }), 2, 1); // no session: dropped
    await settle();
    expect(s.fake.batches).toHaveLength(1);
    await running(s);
    expect(s.fake.of('start')).toHaveLength(2);
    s.backend.output(hap(2, 3, { s: 'a' }), 3, 1);
    await settle();
    const last = s.fake.batches.at(-1)!;
    expect(s.fake.of('schedule').at(-1)![0]).toBe(2);
    expect(last.values.map((v) => [v[0], v[2]])).toEqual([[0, { orbit: 1, channels: [0, 1] }]]);
  });

  test('stop swallows an api error and is a no-op without a session', async () => {
    const s = setup();
    await s.backend.stop();
    expect(s.fake.of('stop')).toEqual([]);
    await running(s);
    s.fake.failNext('engine', 'stop');
    await s.backend.stop();
    expect(s.backend.isRunning()).toBe(false);
  });

  test('events of a stopped session are dropped', async () => {
    const s = setup();
    await running(s);
    const old = s.fake.lastCallback!;
    await s.backend.stop();
    old({ kind: 'error', message: 'late' });
    old({ kind: 'anchor', frame: 1, hostNs: '1' });
    expect(s.backend.isRunning()).toBe(false);
    await running(s);
    expect(() => s.backend.output(hap(0, 1, { s: 'a' }), 1, 1)).not.toThrow();
  });

  test('stop while waiting for the anchor resolves resume false', async () => {
    const s = setup();
    const resumed = s.backend.resume();
    await settle();
    await s.backend.stop();
    expect(await resumed).toBe(false);
  });
});

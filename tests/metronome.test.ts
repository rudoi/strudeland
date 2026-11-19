import { describe, expect, test } from 'bun:test';
import {
  clickInterval,
  createMetronome,
  isAccent,
  LOOKAHEAD_S,
  TICK_MS,
  validGrid,
  type MetronomeGrid,
} from '../src/lib/audio/metronome';
import type { AudioGraph } from '../src/lib/audio/backend';
import { fakeAudioGraph } from './helpers/fake-audio-graph';

/** A metronome on a fake graph with timers under the test's control. */
function setup(opts: { graph?: () => AudioGraph | null; currentTime?: number } = {}) {
  const g = fakeAudioGraph({ currentTime: opts.currentTime ?? 0 });
  let time = opts.currentTime ?? 0;
  const timers = new Map<number, () => void>();
  let nextId = 0;
  let graphCalls = 0;
  const m = createMetronome({
    graph: () => {
      graphCalls++;
      return opts.graph ? opts.graph() : g.graph;
    },
    setInterval: (fn, ms) => {
      expect(ms).toBe(TICK_MS);
      timers.set(++nextId, fn);
      return nextId;
    },
    clearInterval: (id) => void timers.delete(id as number),
  });
  /** Advance the context clock to `t` in `step`-second timer ticks. */
  const runTo = (t: number, step = TICK_MS / 1000) => {
    while (time + step <= t + 1e-12) {
      time += step;
      g.setTime(time);
      [...timers.values()].forEach((f) => f());
    }
  };
  const jump = (t: number) => {
    time = t;
    g.setTime(t);
    [...timers.values()].forEach((f) => f());
  };
  const clicks = () => g.oscillators().map((o) => [o.starts[0]!, o.frequency.value] as const);
  return { g, m, timers, runTo, jump, clicks, graphCalls: () => graphCalls };
}

/** Expected click times `T + i·interval` for i in [from, from + n). */
const times = (T: number, cps: number, num: number, from: number, n: number) =>
  Array.from({ length: n }, (_, k) => T + (from + k) * clickInterval(cps, num));

describe('clickInterval and isAccent', () => {
  test('one click per numerator unit', () => {
    expect(clickInterval(0.5, 4)).toBe(0.5); // 4/4 at 120 bpm
    expect(clickInterval(1 / 3, 6)).toBe(0.5); // 6/8 at 120 bpm (3 quarters a bar)
    expect(clickInterval(0.5, 7)).toBe(1 / 3.5); // 7/8
    expect(clickInterval(2 / 3, 3)).toBe(0.5); // 3/4 at 120 bpm
  });

  test('the accent is i mod num = 0, negative i included', () => {
    expect([-8, -5, -4, -3, -1, 0, 1, 3, 4, 8].map((i) => isAccent(i, 4))).toEqual([
      true,
      false,
      true,
      false,
      false,
      true,
      false,
      false,
      true,
      true,
    ]);
    expect([-3, -2, -1, 0].map((i) => isAccent(i, 3))).toEqual([true, false, false, true]);
    expect(isAccent(5, 1)).toBe(true);
  });

  test('validGrid', () => {
    const ok: MetronomeGrid = { T: 1, cps: 0.5, num: 4, from: -4 };
    expect(validGrid(ok)).toBe(true);
    expect(validGrid({ ...ok, from: 0, until: 9 })).toBe(true);
    expect(validGrid({ ...ok, until: Infinity })).toBe(true);
    for (const bad of [
      { T: Number.NaN },
      { T: Infinity },
      { cps: 0 },
      { cps: -1 },
      { cps: Number.NaN },
      { num: 0 },
      { num: 2.5 },
      { from: 1 },
      { from: -0.5 },
      { until: Number.NaN },
    ]) {
      expect([bad, validGrid({ ...ok, ...bad })]).toEqual([bad, false]);
    }
  });
});

describe('metronome', () => {
  test('4/4: a bar of count-in then a bar, accents on the downbeats, until exclusive', () => {
    const { m, timers, runTo, clicks } = setup();
    expect(m.start({ T: 2, cps: 0.5, num: 4, from: -4, until: 4 })).toBe(true);
    expect(m.running).toBe(true);
    expect(timers.size).toBe(1);
    runTo(4.5);
    expect(clicks()).toEqual(
      times(2, 0.5, 4, -4, 8).map((t, k) => [t, k % 4 === 0 ? 1200 : 800] as const),
    );
    expect(clicks()[0]![0]).toBe(0);
    expect(clicks()[4]![0]).toBe(2);
    expect(m.running).toBe(false);
    expect(timers.size).toBe(0);
  });

  test.each([
    ['6/8', 1 / 3, 6],
    ['7/8', 0.5, 7],
    ['3/4', 2 / 3, 3],
    ['4/4', 0.5, 4],
  ])('%s: one bar has num clicks and one accent', (_name, cps, num) => {
    const { m, runTo, clicks } = setup();
    const T = 0.5;
    m.start({ T, cps, num, from: 0, until: T + 1 / cps });
    runTo(T + 1 / cps + 0.5);
    expect(clicks().map((c) => c[0])).toEqual(times(T, cps, num, 0, num));
    expect(clicks().map((c) => c[1])).toEqual([1200, ...Array(num - 1).fill(800)]);
  });

  test('negative indexes: the accent falls on i mod num = 0', () => {
    const { m, runTo, clicks } = setup();
    m.start({ T: 2.5, cps: 2 / 3, num: 3, from: -5, until: 2.6 });
    runTo(3);
    // i = -5..0: -3 and 0 are accents.
    expect(clicks()).toEqual(
      times(2.5, 2 / 3, 3, -5, 6).map((t, k) => [t, [2, 5].includes(k) ? 1200 : 800] as const),
    );
  });

  test('no gaps and no duplicates across uneven ticks', () => {
    const { m, g, jump, clicks } = setup();
    m.start({ T: 0, cps: 2, num: 4, from: 0 }); // a click every 0.125 s
    let t = 0;
    for (const step of [0.007, 0.04, 0.09, 0.001, 0.05, 0.08, 0.025, 0.099, 0.03]) {
      for (let k = 0; k < 6; k++) jump((t += step));
    }
    m.stop();
    const starts = clicks().map((c) => c[0]);
    expect(new Set(starts).size).toBe(starts.length);
    const last = starts.at(-1)!;
    const n = Math.round(last / 0.125) + 1;
    expect(starts).toEqual(times(0, 2, 4, 0, n));
    expect(last).toBeLessThan(t + LOOKAHEAD_S);
    expect(last + 0.125).toBeGreaterThanOrEqual(t + LOOKAHEAD_S); // scheduled up to the horizon
    expect(g.oscillators().every((o) => o.starts.length === 1)).toBe(true);
  });

  test('clicks in the past are skipped, never played late', () => {
    const { m, jump, clicks } = setup({ currentTime: 5 });
    m.start({ T: 5, cps: 1, num: 4, from: -4 }); // clicks 4, 4.25, 4.5, 4.75 are past
    expect(clicks().map((c) => c[0])).toEqual([5]);
    jump(6.0); // a stalled main thread: 5.25 to 5.75 passed unscheduled
    expect(clicks().map((c) => c[0])).toEqual([5, 6]);
    jump(6.05);
    expect(clicks().map((c) => c[0])).toEqual([5, 6]);
    jump(6.2);
    expect(clicks().map((c) => c[0])).toEqual([5, 6, 6.25]);
  });

  test('a click exactly on currentTime plays; one exactly on the horizon waits', () => {
    const { m, jump, clicks } = setup({ currentTime: 1 });
    m.start({ T: 1, cps: 2.5, num: 4, from: 0 }); // every 0.1 s: 1 plays, 1.1 is the horizon
    expect(clicks().map((c) => c[0])).toEqual([1]);
    jump(1.0000001);
    expect(clicks().map((c) => c[0])).toEqual([1, 1.1]);
  });

  test('the sound: a sine through its own gain, the envelope, start and stop', () => {
    const { m, g } = setup();
    m.start({ T: 0.05, cps: 0.5, num: 4, from: 0, until: 0.06 });
    expect(g.nodes.map((n) => n.kind)).toEqual(['oscillator', 'gain']);
    const [osc] = g.oscillators();
    const [gain] = g.gains();
    expect(osc!.type).toBe('sine');
    expect(osc!.frequency.value).toBe(1200);
    expect(osc!.starts).toEqual([0.05]);
    expect(osc!.stops).toEqual([0.05 + 0.06]);
    expect(gain!.automation).toEqual([
      ['setValueAtTime', 0, 0.05],
      ['linearRampToValueAtTime', 0.5, 0.05 + 0.002],
      ['exponentialRampToValueAtTime', 0.0001, 0.05 + 0.05],
    ]);
    expect(osc!.connects).toEqual([gain]);
    expect(gain!.connects).toEqual([g.graph.destination]);
    expect(g.destination.connects).toEqual([gain]);
    osc!.end();
    expect([osc!.disconnected, gain!.disconnected]).toEqual([true, true]);
  });

  test('a beat click is 800 Hz at 0.3', () => {
    const { m, g } = setup();
    m.start({ T: 0.55, cps: 0.5, num: 4, from: -1, until: 0.1 }); // one click, i = -1 at 0.05
    expect(g.oscillators().map((o) => [o.starts[0], o.frequency.value])).toEqual([
      [0.55 + -1 * 0.5, 800],
    ]);
    expect(g.gains()[0]!.automation[1]).toEqual([
      'linearRampToValueAtTime',
      0.3,
      0.55 + -1 * 0.5 + 0.002,
    ]);
    expect(m.running).toBe(false);
  });

  test('without a graph start returns false and starts nothing', () => {
    const { m, timers } = setup({ graph: () => null });
    expect(m.start({ T: 0, cps: 0.5, num: 4, from: 0 })).toBe(false);
    expect(m.running).toBe(false);
    expect(timers.size).toBe(0);
  });

  test('an invalid grid returns false and starts nothing', () => {
    const { m, timers, g } = setup();
    expect(m.start({ T: 0, cps: 0, num: 4, from: 0 })).toBe(false);
    expect(m.running).toBe(false);
    expect(timers.size).toBe(0);
    expect(g.nodes).toEqual([]);
  });

  test('stop mid-lookahead: scheduled clicks play out, no new ones, the timer is cleared', () => {
    const { m, runTo, timers, clicks } = setup();
    m.start({ T: 0, cps: 4, num: 4, from: 0 }); // every 0.0625 s
    expect(clicks().map((c) => c[0])).toEqual([0, 0.0625]);
    m.stop();
    expect(m.running).toBe(false);
    expect(timers.size).toBe(0);
    runTo(1);
    expect(clicks().map((c) => c[0])).toEqual([0, 0.0625]);
    m.stop(); // idempotent
    expect(m.running).toBe(false);
  });

  test('start while running replaces the grid', () => {
    const { m, runTo, timers, clicks } = setup();
    m.start({ T: 0, cps: 0.5, num: 4, from: 0 });
    m.start({ T: 0.3, cps: 1, num: 3, from: 0, until: 0.31 });
    expect(timers.size).toBe(1); // the first grid's timer is cleared
    runTo(2);
    expect(timers.size).toBe(0);
    expect(clicks()).toEqual([
      [0, 1200],
      [0.3, 1200],
    ]);
  });

  test('graph() is called on every tick; a null tick is skipped and the next catches up', () => {
    let graph: AudioGraph | null = null;
    const s = setup({ graph: () => graph });
    graph = s.g.graph;
    s.m.start({ T: 0, cps: 1, num: 4, from: 0 }); // every 0.25 s
    const calls = s.graphCalls();
    graph = null;
    s.runTo(0.4);
    expect(s.graphCalls() - calls).toBe(16);
    expect(s.clicks().map((c) => c[0])).toEqual([0]);
    graph = s.g.graph;
    s.runTo(0.425);
    expect(s.clicks().map((c) => c[0])).toEqual([0, 0.5]); // 0.25 is past: skipped
    expect(s.m.running).toBe(true);
  });

  test('a throwing graph() counts as none', () => {
    const { m } = setup({
      graph: () => {
        throw new Error('no context');
      },
    });
    expect(m.start({ T: 0, cps: 1, num: 4, from: 0 })).toBe(false);
  });

  test('a click that fails to build is logged and the grid goes on', () => {
    const { m, g, runTo, clicks } = setup();
    const create = g.graph.context.createOscillator.bind(g.graph.context);
    let fail = true;
    (g.graph.context as any).createOscillator = () => {
      if (fail) {
        fail = false;
        throw new Error('NotSupportedError');
      }
      return create();
    };
    const err = console.error;
    const logged: unknown[] = [];
    console.error = (e: unknown) => void logged.push(e);
    try {
      m.start({ T: 0, cps: 1, num: 4, from: 0, until: 0.6 });
      runTo(1);
    } finally {
      console.error = err;
    }
    expect(logged).toHaveLength(1);
    expect(clicks().map((c) => c[0])).toEqual([0.25, 0.5]);
  });
});

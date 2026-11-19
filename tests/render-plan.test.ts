import { describe, expect, test } from 'bun:test';
import { renderOffline } from '../src/lib/audio/offline';
import type { RenderRequest } from '../src/lib/render/contract';
import { encodeControlMap, planEngineJob } from '../src/lib/render/engine-job';
import {
  bucketByBar,
  LEAD_SEC,
  MIN_LEAD_FRAMES,
  orbitCreationOrder,
  orbitNumber,
  pauseLeadSec,
  pauseSchedule,
  selectHaps,
  windowPlan,
  type Selected,
} from '../src/lib/render/plan';
import { hap, recordingDeps, req } from './helpers/fake-offline';

/** A Selected with only what the bucket and orbit helpers read. */
const sel = (onsetCycle: number, index: number, value: object = {}): Selected => ({
  hap: null,
  value,
  onsetCycle,
  index,
});

describe('pauseLeadSec', () => {
  test('LEAD_SEC, at most a quarter bar', () => {
    expect(pauseLeadSec(2)).toBe(LEAD_SEC);
    expect(pauseLeadSec(0.2)).toBe(LEAD_SEC);
    expect(pauseLeadSec(0.1)).toBe(0.025);
    expect(pauseLeadSec(0)).toBe(0);
  });
});

describe('pauseSchedule', () => {
  test('bar 0 at 0, then a lead before every bar line', () => {
    expect(pauseSchedule(3, 0.5, 44100)).toEqual({ leadSec: 0.05, suspendSec: [0, 1.95, 3.95] });
  });

  test('one bar, or none, still has the bar-0 entry', () => {
    expect(pauseSchedule(1, 0.5, 44100).suspendSec).toEqual([0]);
    expect(pauseSchedule(0, 0.5, 44100).suspendSec).toEqual([0]);
    expect(pauseSchedule(-2, 0.5, 44100).suspendSec).toEqual([0]);
  });

  test('a short bar takes a quarter-bar lead', () => {
    // 0.16 s bars: lead 0.04 s (1764 frames, above the guard).
    const { leadSec, suspendSec } = pauseSchedule(3, 6.25, 44100);
    expect(leadSec).toBeCloseTo(0.04, 15);
    expect(suspendSec[1]).toBe(1 * 0.16 - leadSec);
    expect(suspendSec[2]).toBe(2 * 0.16 - leadSec);
  });

  test('suspendSec[j] = j * barSec - leadSec exactly as renderOffline computed it', () => {
    const cps = 137 / 60 / 3.5; // 137 bpm, 7/8
    const barSec = 1 / cps;
    const lead = Math.min(LEAD_SEC, barSec / 4);
    const { suspendSec } = pauseSchedule(5, cps, 44100);
    for (let j = 1; j < 5; j++) expect(suspendSec[j]).toBe(j * barSec - lead);
  });

  test('the guard: a lead under MIN_LEAD_FRAMES throws for more than one bar', () => {
    const fast = 44100 / MIN_LEAD_FRAMES / 4 + 1; // barSec / 4 < 256 frames
    expect(() => pauseSchedule(2, fast, 44100)).toThrow('bar too short for the pause lead');
    expect(pauseSchedule(1, fast, 44100).suspendSec).toEqual([0]);
    // Exactly MIN_LEAD_FRAMES is enough.
    const edge = 44100 / MIN_LEAD_FRAMES / 4;
    expect(pauseSchedule(2, edge, 44100).leadSec).toBe(MIN_LEAD_FRAMES / 44100);
  });
});

describe('bucketByBar', () => {
  const s = [sel(1.9, 0), sel(2, 1), sel(2.5, 2), sel(3.99, 3), sel(4, 4), sel(2.25, 5)];

  test('window edges: bars c0 .. c0 + nBars - 1, others dropped', () => {
    const b = bucketByBar(s, 2, 2);
    expect(b.map((x) => x.map((y) => y.index))).toEqual([[1, 2, 5], [3]]);
  });

  test('keeps the selected order inside a bucket (not onset order)', () => {
    const b = bucketByBar([sel(2.5, 0), sel(2, 1)], 2, 1);
    expect(b[0]!.map((x) => x.index)).toEqual([0, 1]);
  });

  test('skips dropped indices', () => {
    const b = bucketByBar(s, 2, 2, new Set([2, 3]));
    expect(b.map((x) => x.map((y) => y.index))).toEqual([[1, 5], []]);
  });

  test('no bars: an empty list', () => {
    expect(bucketByBar(s, 2, 0)).toEqual([]);
    expect(bucketByBar(s, 2, -1)).toEqual([]);
    expect(bucketByBar([], 0, 2)).toEqual([[], []]);
  });

  test('bars starting at 0 and negative onsets', () => {
    const b = bucketByBar([sel(-0.5, 0), sel(0, 1), sel(0.99, 2)], 0, 1);
    expect(b[0]!.map((x) => x.index)).toEqual([1, 2]);
  });
});

describe('orbitCreationOrder', () => {
  test('sorted by first onset, full schedule and chunk together', () => {
    const full = [sel(0, 0, { orbit: 2 }), sel(1, 1, { orbit: 3 })];
    const chunk = [sel(0.5, 0, { orbit: 4 })];
    expect(orbitCreationOrder(chunk, full, true)).toEqual([
      { orbit: 2, channels: [2, 3], onset: 0 },
      { orbit: 4, channels: [6, 7], onset: 0.5 },
      { orbit: 3, channels: [4, 5], onset: 1 },
    ]);
  });

  test('the earliest hap of an orbit wins, with its channels', () => {
    const full = [sel(2, 0, { orbit: 2, channels: [5, 6] })];
    const chunk = [sel(1, 0, { orbit: 2 })];
    expect(orbitCreationOrder(chunk, full, true)).toEqual([
      { orbit: 2, channels: [2, 3], onset: 1 },
    ]);
  });

  test('an equal onset keeps the first inserted (full before chunk)', () => {
    const full = [sel(1, 0, { orbit: 2, channels: [3, 4] })];
    const chunk = [sel(1, 0, { orbit: 2 })];
    expect(orbitCreationOrder(chunk, full, true)[0]!.channels).toEqual([2, 3]);
  });

  test('ties between orbits keep insertion order', () => {
    const full = [sel(0, 0, { orbit: 3 }), sel(0, 1, { orbit: 1 }), sel(0, 2, { orbit: 2 })];
    expect(orbitCreationOrder([], full, false).map((o) => o.orbit)).toEqual([3, 1, 2]);
  });

  test('a replaced orbit keeps its first insertion slot for ties', () => {
    // orbit 5 first seen at 1 (slot 0), then at 0; orbit 6 at 0 (slot 1). Both at 0: 5 first.
    const full = [sel(1, 0, { orbit: 5 }), sel(0, 1, { orbit: 6 }), sel(0, 2, { orbit: 5 })];
    expect(orbitCreationOrder([], full, false).map((o) => o.orbit)).toEqual([5, 6]);
  });

  test('a missing orbit is 1; the key is String(orbit), the raw value kept', () => {
    const full = [sel(0, 0, {}), sel(0, 1, { orbit: '2' }), sel(1, 2, { orbit: 2 })];
    expect(orbitCreationOrder([], full, false)).toEqual([
      { orbit: 1, channels: [0, 1], onset: 0 },
      { orbit: '2', channels: [0, 1], onset: 0 },
    ]);
  });

  test('nothing selected: no orbits', () => {
    expect(orbitCreationOrder([], [], true)).toEqual([]);
  });
});

describe('orbitNumber', () => {
  test('Number(orbit ?? 1), 1 when not finite', () => {
    expect(orbitNumber({})).toBe(1);
    expect(orbitNumber(undefined)).toBe(1);
    expect(orbitNumber({ orbit: 3 })).toBe(3);
    expect(orbitNumber({ orbit: '4' })).toBe(4);
    expect(orbitNumber({ orbit: 'x' })).toBe(1);
    expect(orbitNumber({ orbit: Infinity })).toBe(1);
    expect(orbitNumber({ orbit: 0 })).toBe(0);
    expect(orbitNumber({ orbit: null })).toBe(1);
  });
});

// ---- renderOffline and planEngineJob agree

const CPS = 0.5;

/** Two haps per bar on alternating orbits, plus a clipped note. */
function song(from: number, to: number): any[] {
  const out: any[] = [];
  for (let bar = from; bar < to; bar++) {
    out.push(hap(bar, bar + 0.5, { s: 'sine', note: 40 + bar, orbit: 1 + (bar % 3) }));
    out.push(hap(bar + 0.25, bar + 1, { s: 'square', freq: 220, orbit: 2, clip: 0.5 }));
    // a continuation: never selected
    out.push(hap(bar - 0.5, bar + 0.25, { s: 'sine' }, { partBegin: bar }));
  }
  return out;
}

async function bothPlans(r: RenderRequest, lookbackSec: number) {
  const p = windowPlan(r, CPS, lookbackSec);
  const haps = song(p.c0, p.c1);
  const scheduleHaps = song(0, p.c1);
  const f = recordingDeps();
  const out = await renderOffline(
    {
      haps,
      cps: CPS,
      seed: 9,
      req: r,
      plan: p,
      scheduleSelected: selectHaps(scheduleHaps).selected,
    },
    f.deps,
  );
  const planned = planEngineJob(
    selectHaps(haps).selected,
    selectHaps(scheduleHaps).selected,
    p,
    r,
    CPS,
    9,
  );
  return { f, out, planned, p };
}

describe('renderOffline and planEngineJob derive the same plan', () => {
  for (const [name, r, lookback] of [
    ['mix, from bar 0', req({ toBar: 3 }), 0],
    [
      'stems with a lookback bar and overlap',
      req({ fromBar: 2, toBar: 4, overlapBars: 1, mode: 'stems' }),
      1,
    ],
  ] as const) {
    test(name, async () => {
      const { f, out, planned, p } = await bothPlans(r, lookback);
      const { job } = planned;
      // pause times
      expect(f.suspends).toEqual(job.pauses.slice(1).map((x) => x.suspendSec));
      // buckets and call order, with times, durations, cycles and values
      const haps = job.pauses.flatMap((x) => x.haps);
      expect(haps).toHaveLength(f.calls.length);
      haps.forEach((h, i) => {
        const c = f.calls[i]!;
        expect(h.t).toBe(c.t);
        expect(h.duration).toBe(c.dur);
        expect(h.cycle).toBe(c.cycle);
        expect(job.values[h.v]).toEqual(encodeControlMap(c.value));
      });
      // each hap sits in the pause of its bar
      job.pauses.forEach((x, j) =>
        x.haps.forEach((h) => expect(Math.floor(h.cycle) - p.c0).toBe(j)),
      );
      // channel plan and orbit order
      expect(f.contexts).toEqual([{ channels: job.channels.count, frames: job.frames }]);
      expect(out.channels).toBe(job.channels.count);
      expect(out.stemPairs).toBe(planned.stemPairs);
      expect(f.orbits).toEqual(job.orbits.map((o) => [o.orbit, o.channels]));
      // events, seed
      expect(planned.events).toEqual(out.events);
      expect(f.seed()).toBe(job.seed);
    });
  }
});

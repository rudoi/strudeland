import { describe, expect, test } from 'bun:test';
import {
  COMPRESSOR_PREDELAY_SEC,
  delayTailSec,
  deriveLookback,
  lookbackBars,
  lookbackSeconds,
  scaleChunkBars,
} from '../src/lib/render/lookback';
import { ev, room } from './helpers/render-events';

describe('delayTailSec', () => {
  test('zero without a delay time or without feedback', () => {
    expect(delayTailSec(0, 0.5)).toBe(0);
    expect(delayTailSec(0.25, 0)).toBe(0);
    expect(delayTailSec(-1, 0.5)).toBe(0);
    expect(delayTailSec(NaN, 0.5)).toBe(0);
  });

  test('feedback 0.5 takes about 10 repeats to reach -60 dB', () => {
    expect(delayTailSec(0.25, 0.5) / 0.25).toBeCloseTo(9.97, 1);
  });

  test('feedback 1 is clamped to 0.98 and still terminates', () => {
    expect(Number.isFinite(delayTailSec(0.25, 1))).toBe(true);
    expect(delayTailSec(0.25, 1)).toBe(delayTailSec(0.25, 0.98));
    expect(delayTailSec(0.25, 5)).toBe(delayTailSec(0.25, 0.98));
  });
});

describe('deriveLookback', () => {
  test('empty list gives just the margin', () => {
    expect(lookbackSeconds([])).toBe(0.5);
    expect(deriveLookback([]).byOrbit).toEqual([]);
    expect(lookbackSeconds([], { marginSec: 0 })).toBe(0);
  });

  test('a voice is duration plus release, plus margin', () => {
    expect(lookbackSeconds([ev({ durSec: 1, release: 0.3 })])).toBeCloseTo(1.8, 9);
  });

  test('room size adds to the voice; null room adds nothing', () => {
    const v = { durSec: 1, release: 0.3 };
    expect(lookbackSeconds([ev({ ...v, room: room(2) })])).toBeCloseTo(3.8, 9);
    expect(lookbackSeconds([ev({ ...v, room: room(20) })])).toBeCloseTo(21.8, 9);
    expect(lookbackSeconds([ev({ ...v, room: null })])).toBeCloseTo(1.8, 9);
  });

  test('the largest room on an orbit counts', () => {
    const r = deriveLookback([ev({ durSec: 0, room: room(2) }), ev({ durSec: 0, room: room(8) })], {
      marginSec: 0,
    });
    expect(r.lookbackSec).toBeCloseTo(8, 9);
    expect(r.byOrbit[0]!.reverbSec).toBe(8);
  });

  test('a delay adds its -60 dB tail', () => {
    const d = { amount: 0.5, timeSec: 0.375, feedback: 0.5 };
    const s = lookbackSeconds([ev({ durSec: 0, delay: d })], { marginSec: 0 });
    expect(s).toBeCloseTo((0.375 * Math.log(1000)) / Math.log(2), 9);
    expect(s).toBeCloseTo(3.737, 3);
  });

  test('feedback 1 is clamped in the derivation', () => {
    const r = deriveLookback(
      [ev({ durSec: 0, delay: { amount: 1, timeSec: 0.25, feedback: 1 } })],
      {
        marginSec: 0,
      },
    );
    expect(r.lookbackSec).toBe(delayTailSec(0.25, 0.98));
  });

  test('reverb and delay on one orbit take the larger, not the sum', () => {
    const r = deriveLookback(
      [ev({ durSec: 2, room: room(4), delay: { amount: 0.5, timeSec: 0.5, feedback: 0.5 } })],
      { marginSec: 0 },
    );
    expect(r.lookbackSec).toBeCloseTo(2 + delayTailSec(0.5, 0.5), 9);
    expect(r.byOrbit[0]).toMatchObject({ orbit: 1, voiceSec: 2, reverbSec: 4 });
  });

  test('additive within an orbit: a 16 s pad into a size-8 room needs 24 s', () => {
    expect(lookbackSeconds([ev({ durSec: 16, room: room(8) })], { marginSec: 0 })).toBeCloseTo(
      24,
      9,
    );
  });

  test('voice and tail need not come from the same event on an orbit', () => {
    const r = deriveLookback([ev({ durSec: 6 }), ev({ durSec: 0.1, room: room(3) })], {
      marginSec: 0,
    });
    expect(r.lookbackSec).toBeCloseTo(9, 9);
  });

  test('per orbit: a long pad and another orbit big room do not add', () => {
    const r = deriveLookback(
      [ev({ durSec: 20, orbit: 1 }), ev({ durSec: 1, orbit: 2, room: room(16) })],
      { marginSec: 0 },
    );
    expect(r.lookbackSec).toBeCloseTo(20, 9);
    expect(r.byOrbit.map((o) => o.orbit)).toEqual([1, 2]);
    expect(r.byOrbit[1]!.needSec).toBeCloseTo(17, 9);
  });

  test('byOrbit is sorted by orbit', () => {
    const r = deriveLookback([ev({ orbit: 5 }), ev({ orbit: 0 }), ev({ orbit: 3 })]);
    expect(r.byOrbit.map((o) => o.orbit)).toEqual([0, 3, 5]);
  });

  test('k scales the reverb tail', () => {
    const e = [ev({ durSec: 0, room: room(12) })];
    expect(lookbackSeconds(e, { k: 1, marginSec: 0 })).toBeCloseTo(12, 9);
    expect(lookbackSeconds(e, { k: 1.5, marginSec: 0 })).toBeCloseTo(18, 9);
    expect(lookbackSeconds(e, { k: 2 / 3, marginSec: 0 })).toBeCloseTo(8, 9);
  });

  test('non-finite duration or release counts as 0', () => {
    expect(lookbackSeconds([ev({ durSec: Infinity, release: NaN })], { marginSec: 0 })).toBe(0);
  });

  test('throws on k <= 0', () => {
    expect(() => deriveLookback([], { k: 0 })).toThrow(/k must be/);
    expect(() => deriveLookback([], { k: -1 })).toThrow(/k must be/);
  });
});

describe('deriveLookback: FXrelease, compressor and buses', () => {
  const z = { marginSec: 0 };

  test('COMPRESSOR_PREDELAY_SEC is 264 frames at 44100 Hz', () => {
    expect(COMPRESSOR_PREDELAY_SEC).toBe(264 / 44100);
    expect(Math.round(COMPRESSOR_PREDELAY_SEC * 44100)).toBe(264);
    expect(COMPRESSOR_PREDELAY_SEC).toBeCloseTo(0.0059864, 7);
  });

  test('FXrelease longer than release replaces it', () => {
    expect(lookbackSeconds([ev({ durSec: 1, release: 0.3, fxRelease: 2 })])).toBe(3.5);
    expect(lookbackSeconds([ev({ durSec: 1, release: 0.3, fxRelease: 2 })], z)).toBe(3);
  });

  test('FXrelease shorter than release changes nothing', () => {
    const with_ = lookbackSeconds([ev({ durSec: 1, release: 0.3, fxRelease: 0.1 })]);
    expect(with_).toBe(lookbackSeconds([ev({ durSec: 1, release: 0.3 })]));
    expect(with_).toBeCloseTo(1.8, 12);
  });

  test('a compressor adds its pre-delay', () => {
    const r = deriveLookback([ev({ durSec: 1, release: 0.3, compressor: true })]);
    expect(r.lookbackSec).toBe(1 + 0.3 + COMPRESSOR_PREDELAY_SEC + 0.5);
    expect(r.lookbackSec).toBeCloseTo(1.8059864, 7);
    expect(r.byOrbit[0]!.voiceSec).toBe(1 + 0.3 + COMPRESSOR_PREDELAY_SEC);
  });

  test('a compressor with FXrelease adds both', () => {
    expect(lookbackSeconds([ev({ durSec: 1, fxRelease: 2, compressor: true })], z)).toBe(
      3 + COMPRESSOR_PREDELAY_SEC,
    );
  });

  test('a negative release clamps to 0 (superdough fullRelease)', () => {
    expect(lookbackSeconds([ev({ durSec: 1, release: -0.5 })], z)).toBe(1);
    // both negative: superdough's max(release, FXrelease) is still negative, mirrored as is
    expect(lookbackSeconds([ev({ durSec: 1, release: -0.5, fxRelease: -2 })], z)).toBe(0.5);
  });

  test('a negative FXrelease adds nothing', () => {
    expect(lookbackSeconds([ev({ durSec: 1, release: 0.25, fxRelease: -3 })], z)).toBe(1.25);
  });

  test('a non-finite FXrelease counts as 0', () => {
    for (const fxRelease of [NaN, Infinity, -Infinity]) {
      expect(lookbackSeconds([ev({ durSec: 1, release: 0.25, fxRelease })], z)).toBe(1.25);
    }
  });

  test('a reader adds its feeder: the per-orbit split', () => {
    const r = deriveLookback([
      ev({ durSec: 2, release: 0.5, busSends: ['1'], orbit: 2 }),
      ev({ durSec: 1, release: 0.5, busReads: ['1'], orbit: 3 }),
    ]);
    expect(r.byOrbit).toEqual([
      { orbit: 2, voiceSec: 2.5, reverbSec: 0, delaySec: 0, needSec: 2.5 },
      { orbit: 3, voiceSec: 4, reverbSec: 0, delaySec: 0, needSec: 4 },
    ]);
    expect(r.lookbackSec).toBe(4.5);
  });

  test('two feeders of one bus: the largest counts, not the sum', () => {
    const r = deriveLookback(
      [
        ev({ durSec: 2, busSends: ['a'], orbit: 1 }),
        ev({ durSec: 5, busSends: ['a'], orbit: 1 }),
        ev({ durSec: 1, busReads: ['a'], orbit: 2 }),
      ],
      z,
    );
    expect(r.byOrbit[1]!.voiceSec).toBe(6);
    expect(r.lookbackSec).toBe(6);
  });

  test('a chain of buses settles exactly', () => {
    const r = deriveLookback(
      [
        ev({ busSends: ['a'], orbit: 1 }),
        ev({ busReads: ['a'], busSends: ['b'], orbit: 2 }),
        ev({ busReads: ['b'], orbit: 3 }),
      ],
      z,
    );
    expect(r.byOrbit.map((o) => o.voiceSec)).toEqual([1, 2, 3]);
    expect(r.lookbackSec).toBe(3);
  });

  test('a chain longer than the old answer needs every round (4 buses, events reversed)', () => {
    const chain = [
      ev({ busSends: ['a'], orbit: 1 }),
      ev({ busReads: ['a'], busSends: ['b'], orbit: 2 }),
      ev({ busReads: ['b'], busSends: ['c'], orbit: 3 }),
      ev({ busReads: ['c'], busSends: ['d'], orbit: 4 }),
      ev({ busReads: ['d'], orbit: 5 }),
    ];
    const r = deriveLookback([...chain].reverse(), z);
    expect(r.byOrbit.map((o) => o.voiceSec)).toEqual([1, 2, 3, 4, 5]);
  });

  test('a two-bus cycle is bounded and finite', () => {
    const r = deriveLookback(
      [
        ev({ busReads: ['b'], busSends: ['a'], orbit: 1 }),
        ev({ busReads: ['a'], busSends: ['b'], orbit: 2 }),
      ],
      z,
    );
    // n = 2 rounds: F_1 = {a: 1, b: 1}, F_2 = {a: 2, b: 2}; eff = 1 + 2
    expect(r.byOrbit.map((o) => o.voiceSec)).toEqual([3, 3]);
    expect(r.lookbackSec).toBe(3);
  });

  test('a self loop is bounded by one round', () => {
    const r = deriveLookback([ev({ busReads: ['x'], busSends: ['x'] })], z);
    expect(r.lookbackSec).toBe(2);
  });

  test('a cycle with a long feeder stays finite', () => {
    const r = deriveLookback(
      [
        ev({ durSec: 1e6, busReads: ['b'], busSends: ['a'] }),
        ev({ busReads: ['a'], busSends: ['b'] }),
      ],
      z,
    );
    expect(Number.isFinite(r.lookbackSec)).toBe(true);
  });

  test('a feeder nobody reads and a reader nobody feeds add nothing beyond own', () => {
    const r = deriveLookback(
      [
        ev({ durSec: 7, release: 0.5, busSends: ['1'], orbit: 1 }),
        ev({ durSec: 1, release: 0.5, busReads: ['2'], orbit: 2 }),
      ],
      z,
    );
    expect(r.byOrbit.map((o) => o.voiceSec)).toEqual([7.5, 1.5]);
  });

  test('bus keys are compared as strings', () => {
    const r = deriveLookback(
      [ev({ durSec: 3, busSends: ['1'] }), ev({ busReads: ['01'], orbit: 2 })],
      z,
    );
    expect(r.byOrbit[1]!.voiceSec).toBe(1);
  });

  test('the feeder need includes its FXrelease and compressor', () => {
    const r = deriveLookback(
      [
        ev({ durSec: 1, fxRelease: 3, compressor: true, busSends: ['1'] }),
        ev({ durSec: 1, busReads: ['1'], orbit: 2 }),
      ],
      z,
    );
    expect(r.byOrbit[1]!.voiceSec).toBe(1 + (1 + 3 + COMPRESSOR_PREDELAY_SEC));
  });

  test('the bus need adds to the reader orbit tails as before', () => {
    const r = deriveLookback(
      [
        ev({ durSec: 2, busSends: ['1'] }),
        ev({ durSec: 1, busReads: ['1'], orbit: 2, room: room(4) }),
      ],
      z,
    );
    expect(r.byOrbit[1]).toEqual({ orbit: 2, voiceSec: 3, reverbSec: 4, delaySec: 0, needSec: 7 });
  });

  test('event order does not change the result', () => {
    const events = [
      ev({ durSec: 2, busSends: ['a'], orbit: 1, release: 0.25 }),
      ev({ durSec: 0.5, busReads: ['a'], busSends: ['b'], orbit: 2, compressor: true }),
      ev({ durSec: 1, busReads: ['b', 'a'], orbit: 3, fxRelease: 1.5, room: room(2) }),
      ev({ durSec: 4, busReads: ['b'], busSends: ['a'], orbit: 1 }),
    ];
    const want = deriveLookback(events);
    const perms = [
      [...events].reverse(),
      [events[2]!, events[0]!, events[3]!, events[1]!],
      [events[1]!, events[3]!, events[2]!, events[0]!],
    ];
    for (const p of perms) expect(deriveLookback(p)).toEqual(want);
  });

  test('byOrbit keeps its shape', () => {
    const r = deriveLookback([ev({ fxRelease: 1, compressor: true, busSends: ['1'] })]);
    expect(Object.keys(r).sort()).toEqual(['byOrbit', 'lookbackSec']);
    expect(Object.keys(r.byOrbit[0]!).sort()).toEqual([
      'delaySec',
      'needSec',
      'orbit',
      'reverbSec',
      'voiceSec',
    ]);
  });

  test('an event without the new features is unchanged: duration + release', () => {
    const e = ev({ durSec: 1.25, release: 0.375, room: room(3) });
    expect(deriveLookback([e], z).byOrbit[0]!.voiceSec).toBe(1.25 + 0.375);
  });
});

describe('lookbackBars', () => {
  test('rounds up to whole bars', () => {
    expect(lookbackBars(3.6, 2)).toBe(2);
    expect(lookbackBars(4.1, 2)).toBe(3);
  });

  test('an exact multiple is not rounded up', () => {
    expect(lookbackBars(2.4, 1.2)).toBe(2);
  });

  test('zero is zero, never -0', () => {
    expect(Object.is(lookbackBars(0, 2), 0)).toBe(true);
  });

  test('throws on barSec <= 0', () => {
    expect(() => lookbackBars(1, 0)).toThrow(/barSec/);
    expect(() => lookbackBars(1, -2)).toThrow(/barSec/);
  });
});

describe('scaleChunkBars', () => {
  test('keeps the requested size when lookback is small', () => {
    expect(scaleChunkBars(8, 2)).toBe(8);
    expect(scaleChunkBars(8, 0)).toBe(8);
  });

  test('grows to 1.5x a large lookback', () => {
    expect(scaleChunkBars(8, 16)).toBe(24);
    expect(scaleChunkBars(8, 7)).toBe(11);
  });

  test('ratio override', () => {
    expect(scaleChunkBars(8, 16, 2)).toBe(32);
  });

  test('throws on non-integer or out-of-range input', () => {
    expect(() => scaleChunkBars(0, 2)).toThrow(/requestedChunkBars/);
    expect(() => scaleChunkBars(8.5, 2)).toThrow(/requestedChunkBars/);
    expect(() => scaleChunkBars(8, -1)).toThrow(/lookbackBars/);
    expect(() => scaleChunkBars(8, 1.5)).toThrow(/lookbackBars/);
  });
});

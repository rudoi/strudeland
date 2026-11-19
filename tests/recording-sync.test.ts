import { describe, expect, test } from 'bun:test';
import { clickInterval as metronomeClickInterval } from '../src/lib/audio/metronome';
import {
  CLOCK_PINGS,
  clickInterval,
  downbeatHostNs,
  measureClockOffset,
  MIN_CLOCK_SAMPLES,
  offsetToSidecar,
  pickClockOffset,
  stampToSidecar,
  type ClockSample,
} from '../src/lib/recording/sync';

describe('pickClockOffset', () => {
  test('takes the sample with the smallest round trip, at its midpoint', () => {
    const samples: ClockSample[] = [
      { beforeMs: 100, afterMs: 103, hostNs: 5_000_000_000n },
      { beforeMs: 200, afterMs: 200.5, hostNs: 5_100_000_000n },
      { beforeMs: 300, afterMs: 302, hostNs: 5_200_000_000n },
    ];
    // midpoint 200.25 ms = 200_250_000 ns
    expect(pickClockOffset(samples)).toEqual({ offsetNs: 4_899_750_000n, rttNs: 500_000n });
  });

  test('ties go to the later sample', () => {
    const samples: ClockSample[] = [
      { beforeMs: 10, afterMs: 11, hostNs: 1_000_000_000n },
      { beforeMs: 20, afterMs: 21, hostNs: 2_000_000_000n },
      { beforeMs: 30, afterMs: 32, hostNs: 3_000_000_000n },
    ];
    expect(pickClockOffset(samples)).toEqual({ offsetNs: 1_979_500_000n, rttNs: 1_000_000n });
  });

  test('the offset is signed: a host clock behind performance.now() gives a negative offset', () => {
    const o = pickClockOffset([{ beforeMs: 1000, afterMs: 1000, hostNs: 999_999_988n }]);
    expect(o).toEqual({ offsetNs: -12n, rttNs: 0n });
    expect(offsetToSidecar(o)).toEqual({ offsetNs: '-12', rttNs: '0' });
  });

  test('a zero round trip is exact', () => {
    expect(pickClockOffset([{ beforeMs: 0.001, afterMs: 0.001, hostNs: 7n }])).toEqual({
      offsetNs: 7n - 1000n,
      rttNs: 0n,
    });
  });

  test('host times above 2^53 keep every nanosecond', () => {
    const hostNs = 2n ** 63n - 1n;
    const o = pickClockOffset([{ beforeMs: 1, afterMs: 3, hostNs }]);
    expect(o.offsetNs).toBe(hostNs - 2_000_000n);
    expect(offsetToSidecar(o)).toEqual({ offsetNs: '9223372036852775807', rttNs: '2000000' });
  });

  test('an empty list throws', () => {
    expect(() => pickClockOffset([])).toThrow('no clock samples');
  });

  test('NaN, infinite and negative round trips are ignored', () => {
    const bad: ClockSample[] = [
      { beforeMs: NaN, afterMs: 1, hostNs: 1n },
      { beforeMs: 1, afterMs: Infinity, hostNs: 1n },
      { beforeMs: 5, afterMs: 4, hostNs: 1n },
    ];
    expect(() => pickClockOffset(bad)).toThrow('no clock samples');
    expect(pickClockOffset([...bad, { beforeMs: 2, afterMs: 4, hostNs: 10_000_000n }])).toEqual({
      offsetNs: 7_000_000n,
      rttNs: 2_000_000n,
    });
  });
});

describe('measureClockOffset', () => {
  /** A ms clock advancing by `step` per read and a host clock at `offset` + now, each answer
   *  taking `rtt(i)` ms (the clock moves while the host answers). */
  function clocks(opts: { rtt: (i: number) => number; fail?: (i: number) => boolean }) {
    let now = 1000;
    let i = 0;
    const offset = 5_000_000_000n;
    return {
      nowMs: () => now,
      hostNowNs: async () => {
        const k = i++;
        if (opts.fail?.(k)) throw new Error('ipc');
        const half = opts.rtt(k) / 2;
        now += half;
        const t = offset + BigInt(Math.round(now * 1e6));
        now += half;
        return t;
      },
      pings: () => i,
    };
  }

  test('sends 20 pings by default and keeps the tightest', async () => {
    const c = clocks({ rtt: (i) => (i === 7 ? 0.2 : 1) });
    const o = await measureClockOffset(c);
    expect(c.pings()).toBe(CLOCK_PINGS);
    expect(CLOCK_PINGS).toBe(20);
    expect(o).toEqual({ offsetNs: 5_000_000_000n, rttNs: 200_000n });
  });

  test('skips failed pings', async () => {
    const c = clocks({ rtt: (i) => (i === 3 ? 0.4 : 1), fail: (i) => i % 2 === 0 });
    const o = await measureClockOffset(c, 10);
    expect(c.pings()).toBe(10);
    expect(o).toEqual({ offsetNs: 5_000_000_000n, rttNs: 400_000n });
  });

  test('exactly five answers are enough', async () => {
    const c = clocks({ rtt: () => 1, fail: (i) => i >= MIN_CLOCK_SAMPLES });
    expect(MIN_CLOCK_SAMPLES).toBe(5);
    expect((await measureClockOffset(c, 8)).rttNs).toBe(1_000_000n);
  });

  test('fewer than five answers throw', async () => {
    const c = clocks({ rtt: () => 1, fail: (i) => i >= 4 });
    await expect(measureClockOffset(c, 20)).rejects.toThrow(
      'the host clock answered 4 of 20 pings',
    );
  });

  test('every ping failing throws', async () => {
    const c = clocks({ rtt: () => 1, fail: () => true });
    await expect(measureClockOffset(c)).rejects.toThrow('the host clock answered 0 of 20 pings');
  });
});

describe('downbeatHostNs', () => {
  const stamp = { contextTime: 10, performanceMs: 50_000 };

  test('T after the stamp', () => {
    // 50_000 ms + 2.5 s = 52_500 ms
    expect(downbeatHostNs(1_000n, stamp, 12.5)).toBe(1_000n + 52_500_000_000n);
  });

  test('T before the stamp', () => {
    expect(downbeatHostNs(1_000n, stamp, 9.75)).toBe(1_000n + 49_750_000_000n);
  });

  test('T at the stamp is its performance time', () => {
    expect(downbeatHostNs(0n, stamp, 10)).toBe(50_000_000_000n);
  });

  test('a negative offset', () => {
    expect(downbeatHostNs(-50_000_000_000n, stamp, 10.000001)).toBe(1_000n);
  });

  test('extraDelaySec shifts bar 1 later by that many ms; the default is no shift', () => {
    expect(downbeatHostNs(1_000n, stamp, 12.5, 0.0015)).toBe(1_000n + 52_501_500_000n);
    expect(downbeatHostNs(1_000n, stamp, 12.5, 0)).toBe(downbeatHostNs(1_000n, stamp, 12.5));
    expect(downbeatHostNs(0n, stamp, 10, 72 / 48000)).toBe(50_001_500_000n);
  });

  test('non-finite inputs throw rather than give a wrong time', () => {
    expect(() => downbeatHostNs(0n, stamp, NaN)).toThrow();
    expect(() => downbeatHostNs(0n, stamp, 10, NaN)).toThrow();
    expect(() => downbeatHostNs(0n, { contextTime: 1, performanceMs: Infinity }, 1)).toThrow();
  });
});

describe('stampToSidecar', () => {
  test('the context time and the host time as a decimal string', () => {
    expect(stampToSidecar(123n, { contextTime: 1.25, performanceMs: 2000.5 })).toEqual([
      1.25,
      '2000500123',
    ]);
  });

  test('a negative offset and a large host time', () => {
    expect(stampToSidecar(-500_000n, { contextTime: 0.5, performanceMs: 1 })).toEqual([
      0.5,
      '500000',
    ]);
    expect(stampToSidecar(2n ** 62n, { contextTime: 3, performanceMs: 0 })).toEqual([
      3,
      '4611686018427387904',
    ]);
  });
});

describe('clickInterval', () => {
  test('is the metronome one', () => {
    expect(clickInterval).toBe(metronomeClickInterval);
  });

  test.each([
    // [meter, bpm (quarter notes), cps, clicks per bar, seconds per click]
    ['4/4', 120, 0.5, 4, 0.5],
    ['6/8', 120, 120 / 60 / 3, 6, 0.25],
    ['7/8', 120, 120 / 60 / 3.5, 7, 0.25],
    ['3/4', 90, 90 / 60 / 3, 3, 2 / 3],
  ] as const)('%s at %d bpm', (_m, _bpm, cps, num, seconds) => {
    expect(clickInterval(cps, num)).toBeCloseTo(seconds, 12);
    expect(clickInterval(cps, num) * num).toBeCloseTo(1 / cps, 12);
  });
});

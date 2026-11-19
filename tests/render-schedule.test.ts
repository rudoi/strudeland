import { describe, expect, test } from 'bun:test';
import { eventsFromValues, type EventSource } from '../src/lib/render/schedule';

const src = (value: unknown, begin = 0, dur = 1): EventSource => ({
  value: value as Record<string, unknown>,
  wholeBeginCycles: begin,
  wholeDurCycles: dur,
});
const one = (value: unknown, cps = 0.5) => eventsFromValues([src(value)], cps)[0]!;

describe('eventsFromValues', () => {
  test('superdough defaults on {}', () => {
    const e = one({});
    expect(e).toMatchObject({
      s: '',
      orbit: 1,
      gain: 0.8,
      attack: 0,
      decay: 0,
      sustain: null,
      release: 0.01,
      penv: 0,
      cutoff: null,
      room: null,
      delay: null,
      freq: null,
      note: null,
      stochastic: false,
      fxRelease: 0,
      compressor: false,
      busSends: [],
      busReads: [],
    });
  });

  test('timing in layout seconds and bars', () => {
    const e = eventsFromValues([src({}, 2.5, 0.25)], 0.5)[0]!;
    expect(e.tSec).toBe(5);
    expect(e.durSec).toBe(0.5);
    expect(e.bar).toBe(2);
  });

  test('gain multiplies by velocity', () => {
    expect(one({ gain: 0.5, velocity: 0.5 }).gain).toBe(0.25);
    expect(one({ velocity: 0.5 }).gain).toBeCloseTo(0.4, 12);
  });

  test('explicit fields', () => {
    const e = one({
      s: 'sawtooth',
      orbit: 3,
      attack: 0.1,
      decay: 0.2,
      sustain: 0.5,
      release: 0.3,
      penv: 12,
      cutoff: 800,
    });
    expect(e).toMatchObject({
      s: 'sawtooth',
      orbit: 3,
      attack: 0.1,
      decay: 0.2,
      sustain: 0.5,
      release: 0.3,
      penv: 12,
      cutoff: 800,
    });
  });

  test('NaN and non-finite numbers fall back to defaults', () => {
    const e = one({ gain: NaN, orbit: Infinity, release: 'x', attack: null });
    expect(e.gain).toBe(0.8);
    expect(e.orbit).toBe(1);
    expect(e.release).toBe(0.01);
    expect(e.attack).toBe(0);
  });

  test('freq from note name, MIDI number and freq', () => {
    expect(one({ note: 'c2' }).freq!).toBeCloseTo(65.406, 2);
    expect(one({ note: 36 }).freq!).toBeCloseTo(65.406, 2);
    expect(one({ note: 'a4' }).freq!).toBeCloseTo(440, 9);
    expect(one({ note: 'c#3' }).freq!).toBeCloseTo(one({ note: 49 }).freq!, 9);
    expect(one({ note: 'cs3' }).freq!).toBeCloseTo(one({ note: 49 }).freq!, 9);
    expect(one({ note: 'eb3' }).freq!).toBeCloseTo(one({ note: 51 }).freq!, 9);
    expect(one({ note: 'C' }).freq!).toBeCloseTo(one({ note: 48 }).freq!, 9);
    expect(one({ note: 'c-1' }).freq!).toBeCloseTo(one({ note: 0 }).freq!, 9);
    expect(one({ freq: 100, note: 'c2' }).freq).toBe(100);
  });

  test('n is a sample index, not a note', () => {
    const e = one({ s: 'bd', n: 3 });
    expect(e.note).toBeNull();
    expect(e.freq).toBeNull();
  });

  test('unparseable note string gives freq null, keeps raw note', () => {
    const e = one({ note: 'h7' });
    expect(e.freq).toBeNull();
    expect(e.note).toBe('h7');
  });

  test('room defaults and room 0', () => {
    expect(one({ room: 0.3 }).room).toEqual({
      amount: 0.3,
      size: 2,
      fade: 0.1,
      lp: 15000,
      dim: 1000,
    });
    expect(one({ room: 0.3, size: 4 }).room!.size).toBe(4);
    expect(one({ room: 0.3, roomsize: 5, size: 4 }).room!.size).toBe(5);
    expect(one({ room: 0 }).room).toBeNull();
    expect(one({ room: -1 }).room).toBeNull();
  });

  test('delay time from delaysync and cps; explicit delaytime wins', () => {
    expect(one({ delay: 0.4 }, 0.5).delay).toEqual({ amount: 0.4, timeSec: 0.375, feedback: 0.5 });
    expect(one({ delay: 0.4, delaysync: 0.25 }, 0.5).delay!.timeSec).toBe(0.5);
    expect(one({ delay: 0.4, delaytime: 0.2, delayfeedback: 0.7 }, 0.5).delay).toEqual({
      amount: 0.4,
      timeSec: 0.2,
      feedback: 0.7,
    });
    expect(one({ delay: 0 }).delay).toBeNull();
  });

  test('stochastic is exactly the kabelsalat path', () => {
    expect(one({ workletSrc: 'sine(440)' }).stochastic).toBe(true);
    expect(one({ s: 'white' }).stochastic).toBe(false);
    expect(one({ s: 'crackle' }).stochastic).toBe(false);
    expect(one({ s: 'supersaw' }).stochastic).toBe(false);
  });

  test('non-object values', () => {
    expect(one('bd').s).toBe('bd');
    expect(one(42)).toMatchObject({ s: '', gain: 0.8 });
    expect(one(null).orbit).toBe(1);
  });

  test('sorted by tSec, stable on ties', () => {
    const evs = eventsFromValues(
      [src({ s: 'c' }, 1), src({ s: 'a' }, 0), src({ s: 'b' }, 1), src({ s: 'd' }, 0.5)],
      1,
    );
    expect(evs.map((e) => e.s)).toEqual(['a', 'd', 'c', 'b']);
  });

  test('empty input', () => {
    expect(eventsFromValues([], 1)).toEqual([]);
  });

  test('invalid cps throws', () => {
    expect(() => eventsFromValues([], 0)).toThrow(RangeError);
    expect(() => eventsFromValues([], -1)).toThrow(RangeError);
    expect(() => eventsFromValues([], NaN)).toThrow(RangeError);
    expect(() => eventsFromValues([], Infinity)).toThrow(RangeError);
  });
});

/** A `bmod` value as the page builds it: `__ids` plus one config per id. */
const bmod = (ids: Iterable<string>, configs: Record<string, unknown>) => ({
  __ids: ids,
  ...configs,
});

describe('eventsFromValues: fxRelease, compressor, busSends, busReads', () => {
  test('fxRelease from FXrelease: number, numeric string, absent, junk', () => {
    expect(one({ FXrelease: 2 }).fxRelease).toBe(2);
    expect(one({ FXrelease: '0.5' }).fxRelease).toBe(0.5);
    expect(one({}).fxRelease).toBe(0);
    expect(one({ FXrelease: 'x' }).fxRelease).toBe(0);
    expect(one({ FXrelease: NaN }).fxRelease).toBe(0);
    expect(one({ FXrelease: -1 }).fxRelease).toBe(-1);
  });

  test('fxRelease is top level only, and only the canonical key', () => {
    expect(one({ FX: [{ FXrelease: 3 }] }).fxRelease).toBe(0);
    expect(one({ fxr: 3, FXrel: 3, FXr: 3 }).fxRelease).toBe(0);
  });

  test('compressor at top level, inside an FX entry, as 0 and null, absent', () => {
    expect(one({ compressor: -20 }).compressor).toBe(true);
    expect(one({ FX: [{ shape: 0.5 }, { compressor: -10 }] }).compressor).toBe(true);
    expect(one({ compressor: 0 }).compressor).toBe(true);
    expect(one({ compressor: null }).compressor).toBe(true);
    expect(one({ FX: [{ compressor: null }] }).compressor).toBe(true);
    expect(one({}).compressor).toBe(false);
    expect(one({ compressor: undefined }).compressor).toBe(false);
    expect(one({ FX: [{ shape: 0.5 }] }).compressor).toBe(false);
  });

  test('busSends: bus 1 and bus "1" are one bus; null and absent send nowhere', () => {
    expect(one({ bus: 1 }).busSends).toEqual(['1']);
    expect(one({ bus: '1' }).busSends).toEqual(['1']);
    expect(one({ bus: 0 }).busSends).toEqual(['0']);
    expect(one({ bus: null }).busSends).toEqual([]);
    expect(one({}).busSends).toEqual([]);
    expect(one({ FX: [{ bus: 2 }] }).busSends).toEqual([]);
  });

  test('busReads of the bus sound: n ?? 0', () => {
    expect(one({ s: 'bus' }).busReads).toEqual(['0']);
    expect(one({ s: 'bus', n: 2 }).busReads).toEqual(['2']);
    expect(one({ s: 'bus', n: '2' }).busReads).toEqual(['2']);
    expect(one({ s: 'bus', n: null }).busReads).toEqual(['0']);
    expect(one({ s: 'sine', n: 2 }).busReads).toEqual([]);
  });

  test('busReads of bmod with a Set and with an array __ids', () => {
    expect(one({ bmod: bmod(new Set(['m1']), { m1: { bus: 1 } }) }).busReads).toEqual(['1']);
    expect(one({ bmod: bmod(['m1'], { m1: { bus: '3' } }) }).busReads).toEqual(['3']);
  });

  test('two ids on one bus are de-duplicated, order kept', () => {
    const v = {
      s: 'bus',
      n: 4,
      bmod: bmod(new Set(['a', 'b', 'c']), { a: { bus: 2 }, b: { bus: '4' }, c: { bus: 2 } }),
    };
    expect(one(v).busReads).toEqual(['4', '2']);
  });

  test('a bmod inside an FX entry is read after the hap value', () => {
    const v = {
      bmod: bmod(['m'], { m: { bus: 5 } }),
      FX: [{ bmod: bmod(['f'], { f: { bus: 7 } }) }, { bmod: bmod(['g'], { g: { bus: 5 } }) }],
    };
    expect(one(v).busReads).toEqual(['5', '7']);
  });

  test('a bmod config without bus reads "undefined" (superdough getBus(undefined))', () => {
    expect(one({ bmod: bmod(['m'], { m: { depth: 1 } }) }).busReads).toEqual(['undefined']);
  });

  test('malformed inputs give empty arrays and do not throw', () => {
    const cases: unknown[] = [
      { bmod: 5 },
      { bmod: null },
      { bmod: { __ids: 5, 5: { bus: 1 } } },
      { bmod: { __ids: 'ab', a: { bus: 1 } } },
      { bmod: { __ids: {}, m: { bus: 1 } } },
      { bmod: { m: { bus: 1 } } },
      { bmod: bmod(['m'], { m: 'x' }) },
      { bmod: bmod(['m'], { m: null }) },
      { bmod: bmod(['m'], {}) },
      { bmod: bmod([{}] as unknown as string[], {}) },
      { FX: 'x', bus: null },
      { FX: [null, 3, 'y'] },
      { FX: { compressor: 1, bmod: bmod(['m'], { m: { bus: 1 } }) } },
      {
        bmod: {
          __ids: {
            [Symbol.iterator]() {
              throw new Error('boom');
            },
          },
        },
      },
    ];
    for (const v of cases) {
      const e = one(v);
      expect(e.busReads).toEqual([]);
      expect(e.busSends).toEqual([]);
      expect(e.compressor).toBe(false);
    }
  });

  test('a bus key whose String() throws is skipped, not thrown', () => {
    const bad = Object.create(null) as object;
    expect(one({ bus: bad }).busSends).toEqual([]);
    expect(one({ s: 'bus', n: bad }).busReads).toEqual([]);
    expect(one({ bmod: bmod(['m'], { m: { bus: bad } }) }).busReads).toEqual([]);
  });

  test('non-object values carry the defaults', () => {
    for (const v of ['bd', 42, null]) {
      expect(one(v)).toMatchObject({ fxRelease: 0, compressor: false, busSends: [], busReads: [] });
    }
  });
});

import { describe, expect, test } from 'bun:test';
import { createLiveClock, type ClockPort, type LiveClock } from '../src/lib/strudel/live-clock';
import { buildStructure, type Meter, type SongStructure } from '../src/lib/strudel/structure';
import { INITIAL_SEGMENTS } from '../src/lib/strudel/transport';

// The tick is 0.1 s, as in the fake runtime. Expected values below are worked out by hand from
// seconds and cps; none comes from the clock's own functions.
const TICK = 0.1;
const FOUR_FOUR: Meter = { num: 4, den: 4 };
const TWO_FOUR: Meter = { num: 2, den: 4 };

const sections = [{ name: 'phrase', bars: 3 }];
const meterChanges = [
  { bar: 2, meter: '3/8' },
  { bar: 3, meter: '4/4' },
];
/** 4/4, then 3/8 in bar 2, then 4/4 in bar 3: bars of 1, 0.375 and 1 reference cycles. */
const MIXED = buildStructure({ meter: '4/4', sections, meterChanges });
/** The same at 2/4: bars of 1, 0.75 and 2 reference cycles. */
const MIXED_2_4 = buildStructure({ meter: '2/4', sections, meterChanges });
/** Bar 2 at 2/4 instead of 3/8: bars of 1, 0.5 and 1 reference cycles. */
const MIXED_BAR_2_IN_2_4 = buildStructure({
  meter: '4/4',
  sections,
  meterChanges: [
    { bar: 2, meter: '2/4' },
    { bar: 3, meter: '4/4' },
  ],
});
const CONSTANT = buildStructure({ meter: '4/4', sections });

/** MIXED's layout cycle at reference clock `r` (bar 1: [0, 1), bar 2: [1, 1.375), bar 3 on). */
const mixedCycle = (r: number) => (r < 1 ? r : r < 1.375 ? 1 + (r - 1) / 0.375 : 2 + (r - 1.375));

/**
 * A fake Cyclist: `now()` follows `Cyclist.now` (0 while stopped, else `lastBegin + (getTime() -
 * lastTick - tickDuration) * cps`), and `tick()` advances it as Cyclist's clock callback does at
 * constant cps, the time stepping with `lastTick` (zyklus's phase).
 */
function fakePort() {
  const p = {
    started: false,
    lastBegin: 0,
    lastEnd: 0,
    lastTick: 0,
    tickDuration: TICK,
    time: 0,
    cps: 0.5,
  };
  const port: ClockPort = {
    get started() {
      return p.started;
    },
    now: () => (p.started ? p.lastBegin + (p.time - p.lastTick - p.tickDuration) * p.cps : 0),
    get lastBegin() {
      return p.lastBegin;
    },
    get lastEnd() {
      return p.lastEnd;
    },
    get lastTick() {
      return p.lastTick;
    },
    get tickDuration() {
      return p.tickDuration;
    },
    getTime: () => p.time,
    get cps() {
      return p.cps;
    },
  };
  const tick = (n = 1) => {
    for (let i = 0; i < n; i++) {
      p.lastBegin = p.lastEnd;
      p.lastEnd += p.tickDuration * p.cps;
      p.lastTick += p.tickDuration;
      p.time = p.lastTick;
    }
  };
  return { p, port, tick };
}

/** A clock with `structure` installed while stopped (as the first evaluate does), then started. */
function playing(structure: SongStructure | null, meter: Meter = FOUR_FOUR) {
  const f = fakePort();
  const clock = createLiveClock(f.port);
  clock.edit({
    structure,
    meter,
    nextCps: f.p.cps,
    codeSetCps: false,
    segments: INITIAL_SEGMENTS,
  });
  f.p.started = true;
  return { ...f, clock };
}

/** A live evaluate: the clock's edit, then the scheduler's tempo as afterEval sets it. */
function edit(
  f: { p: { cps: number }; clock: LiveClock },
  structure: SongStructure | null,
  nextCps: number,
  meter: Meter = FOUR_FOUR,
  codeSetCps = false,
) {
  const anchor = f.clock.edit({
    structure,
    meter,
    nextCps,
    codeSetCps,
    segments: INITIAL_SEGMENTS,
  });
  if (!codeSetCps) f.p.cps = nextCps;
  return anchor;
}

describe('createLiveClock', () => {
  test('reads nothing from the port while constructing', () => {
    const port = new Proxy({} as ClockPort, {
      get: () => {
        throw new Error('read');
      },
    });
    expect(() => createLiveClock(port)).not.toThrow();
  });

  test('stopped and without a map: identity maths, the scheduler’s now and cps', () => {
    const { p, port, tick } = fakePort();
    const clock = createLiveClock(port);
    expect(clock.toCycle(1.25)).toBe(1.25);
    expect(clock.toClock(1.25)).toBe(1.25);
    expect(clock.now()).toBe(0);
    expect(clock.cpsNow()).toBe(0.5);
    p.cps = 0.75;
    expect(clock.cpsNow()).toBe(0.75);
    // Started, still without a map or history: the scheduler's own now, clamped at 0.
    p.started = true;
    tick(1);
    // lastBegin 0, one tick behind: 0 - 0.1 s * 0.75.
    expect(port.now()).toBeCloseTo(-0.075, 10);
    expect(clock.now()).toBe(0);
    tick(4);
    // lastBegin 0.3, minus one tick at 0.75 cps.
    expect(clock.now()).toBeCloseTo(0.225, 10);
    expect(clock.cpsNow()).toBe(0.75);
  });

  test('a constant-meter tempo edit while playing rebases the phase', () => {
    const f = playing(CONSTANT);
    f.tick(10);
    // The anchor is lastEnd: 10 ticks of 0.1 s at 0.5 cps.
    const anchor = edit(f, CONSTANT, 1.5);
    expect(anchor!.clock).toBeCloseTo(0.5, 10);
    expect(anchor!.cycle).toBeCloseTo(0.5, 10);
    // 0.5 / 0.5 - 0.5 / 1.5 seconds.
    expect(f.clock.phaseOffsetSec).toBeCloseTo(2 / 3, 10);
    expect(f.clock.map).toBeNull();
    expect(f.clock.snapshot().history).toEqual([]);

    const coded = playing(CONSTANT);
    coded.tick(10);
    edit(coded, CONSTANT, 1.5, FOUR_FOUR, true);
    expect(coded.clock.phaseOffsetSec).toBe(0);

    const stopped = createLiveClock(fakePort().port);
    expect(
      stopped.edit({
        structure: CONSTANT,
        meter: FOUR_FOUR,
        nextCps: 1.5,
        codeSetCps: false,
        segments: INITIAL_SEGMENTS,
      }),
    ).toBeUndefined();
    expect(stopped.phaseOffsetSec).toBe(0);
    expect(stopped.structure).toBe(CONSTANT);
  });

  test('repeated live tempo edits follow each old tempo until that edit sounds', () => {
    const f = playing(MIXED);
    f.tick(26);
    // The audible reference trails lastEnd (1.3) by two ticks at 0.5 cps: 1.2.
    const baseline = 1.2;
    const start = f.p.time;
    expect(f.clock.now()).toBeCloseTo(mixedCycle(baseline), 10);
    edit(f, MIXED, 1.5);
    // The first edit sounds once the audio reaches its anchor, lastEnd 1.3: 0.2 s at 0.5 cps.
    const firstSec = (1.3 - baseline) / 0.5;
    expect(f.clock.now()).toBeCloseTo(mixedCycle(baseline), 10);
    f.tick(1);
    expect(f.clock.now()).toBeCloseTo(mixedCycle(baseline + 0.1 * 0.5), 10);
    // One tick at 1.5 cps put lastEnd at 1.45: the second edit sounds 0.1 s after the first.
    const secondSec = firstSec + (1.45 - 1.3) / 1.5;
    edit(f, MIXED, 0.25);
    expect(f.clock.now()).toBeCloseTo(mixedCycle(baseline + 0.1 * 0.5), 10);
    for (let i = 0; i < 7; i++) {
      f.tick(1);
      const elapsed = f.p.time - start;
      const reference =
        baseline +
        Math.min(elapsed, firstSec) * 0.5 +
        Math.max(0, Math.min(elapsed, secondSec) - firstSec) * 1.5 +
        Math.max(0, elapsed - secondSec) * 0.25;
      expect(f.clock.now()).toBeCloseTo(mixedCycle(reference), 10);
    }
    // The last step: 0.8 s in, 0.2 s at 0.5, 0.1 s at 1.5 and 0.5 s at 0.25 cps.
    expect(f.clock.now()).toBeCloseTo(mixedCycle(1.575), 10);
  });

  test('cpsNow follows the audible meter and tempo across an edit of both', () => {
    const f = playing(MIXED);
    f.tick(23);
    // Reference 1.05, in the 3/8 bar: 120 bpm over 1.5 quarters.
    expect(f.clock.cpsNow()).toBeCloseTo(4 / 3, 10);
    edit(f, MIXED_2_4, 1.5, TWO_FOUR);
    // The anchor (lastEnd 1.15) sounds 0.2 s later; until then the old tempo and meter.
    expect(f.clock.cpsNow()).toBeCloseTo(4 / 3, 10);
    f.tick(1);
    expect(f.clock.cpsNow()).toBeCloseTo(4 / 3, 10);
    f.tick(1);
    // At the anchor: 1.5 cps at 2/4 is 180 bpm, over the 3/8 bar's 1.5 quarters.
    expect(f.clock.cpsNow()).toBeCloseTo(2, 10);
    // The anchor is 0.4 into the new 3/8 bar of 0.75, so the bar ends 0.45 later, at 1.6; the
    // reference rises 0.15 per tick from 1.15.
    f.tick(2);
    expect(f.clock.cpsNow()).toBeCloseTo(2, 10);
    f.tick(2);
    // Reference 1.75, in bar 3: 180 bpm over 4 quarters.
    expect(f.clock.cpsNow()).toBeCloseTo(0.75, 10);
  });

  test('a meter edit keeps the playhead at the anchor', () => {
    const f = playing(MIXED);
    f.tick(25);
    // Reference 1.15, 0.15 into the 0.375 bar: cycle 1.4.
    expect(f.clock.now()).toBeCloseTo(1.4, 10);
    const anchor = edit(f, MIXED_BAR_2_IN_2_4, 0.5);
    // lastEnd 1.25, 0.25 into the 0.375 bar.
    expect(anchor!.cycle).toBeCloseTo(1 + 0.25 / 0.375, 10);
    expect(f.clock.now()).toBeCloseTo(1.4, 10);
    expect(f.clock.phaseOffsetSec).toBe(0);
    f.tick(3);
    // Reference 1.3, 0.05 past the anchor in a 2/4 bar of 0.5 reference cycles.
    expect(f.clock.now()).toBeCloseTo(1 + 0.25 / 0.375 + 0.05 / 0.5, 10);
  });

  test('history drops what sounded, never pushes one anchor twice, records the old state', () => {
    const f = playing(MIXED);
    f.tick(26);
    const firstMap = f.clock.map;
    const a = edit(f, MIXED, 1.5);
    let history = f.clock.snapshot().history;
    expect(history).toHaveLength(1);
    expect(history[0]!.end).toEqual(a!);
    expect(history[0]!.end.clock).toBeCloseTo(1.3, 10);
    expect(history[0]!.map).toBe(firstMap);
    expect(history[0]!.cps).toBe(0.5);
    expect(history[0]!.referenceMeter).toEqual(FOUR_FOUR);
    // Same anchor clock (no tick): not pushed again.
    edit(f, MIXED, 1.5);
    expect(f.clock.snapshot().history).toEqual(history);
    // Five ticks at 1.5 cps: the audio is well past 1.3, so the next edit drops that entry.
    f.tick(5);
    const secondMap = f.clock.map;
    const b = edit(f, MIXED_2_4, 1, TWO_FOUR);
    history = f.clock.snapshot().history;
    expect(history).toHaveLength(1);
    expect(history[0]!.end).toEqual(b!);
    expect(history[0]!.map).toBe(secondMap);
    expect(history[0]!.cps).toBe(1.5);
    expect(history[0]!.referenceMeter).toEqual(FOUR_FOUR);
    // One tick later the previous edit is still in the lookahead: both stay.
    f.tick(1);
    edit(f, MIXED_2_4, 0.5, TWO_FOUR);
    history = f.clock.snapshot().history;
    expect(history).toHaveLength(2);
    expect(history[1]!.cps).toBe(1);
    expect(history[1]!.referenceMeter).toEqual(TWO_FOUR);
  });

  test('code without song(): an offset anchor builds a structure at the evaluate’s meter', () => {
    const f = playing(MIXED);
    f.tick(25);
    // lastEnd 1.25 is cycle 1 + 0.25 / 0.375 on the old map.
    const anchor = edit(f, null, 0.5, { num: 3, den: 4 });
    expect(anchor!.cycle).toBeCloseTo(1 + 0.25 / 0.375, 10);
    expect(f.clock.structure?.meter).toEqual({ num: 3, den: 4 });
    expect(f.clock.structure?.meterChanges).toBeUndefined();
    // A constant offset through the anchor.
    expect(f.clock.toClock(3)).toBeCloseTo(3 + 1.25 - (1 + 0.25 / 0.375), 10);

    const plain = playing(null);
    plain.tick(10);
    const equal = edit(plain, null, 1, { num: 3, den: 4 });
    expect(equal!.cycle).toBe(equal!.clock);
    expect(plain.clock.structure).toBeNull();
    expect(plain.clock.map).toBeNull();
  });

  test('setLayout builds a map only for meter changes or an offset anchor', () => {
    const { port } = fakePort();
    const clock = createLiveClock(port);
    clock.setLayout(INITIAL_SEGMENTS, { cycle: 1, clock: 1.5 });
    expect(clock.map).toBeNull();
    clock.restore({ structure: CONSTANT, map: null, history: [], phaseOffsetSec: 0 });
    clock.setLayout(INITIAL_SEGMENTS);
    expect(clock.map).toBeNull();
    clock.setLayout(INITIAL_SEGMENTS, { cycle: 2, clock: 2 });
    expect(clock.map).toBeNull();
    clock.setLayout(INITIAL_SEGMENTS, { cycle: 1, clock: 1.5 });
    expect(clock.toClock(3)).toBeCloseTo(3.5, 10);
    expect(clock.toCycle(2)).toBeCloseTo(1.5, 10);
    clock.restore({ structure: MIXED, map: null, history: [], phaseOffsetSec: 0 });
    clock.setLayout(INITIAL_SEGMENTS);
    expect(clock.toClock(2)).toBeCloseTo(1.375, 10);
    expect(clock.toCycle(1.1875)).toBeCloseTo(1.5, 10);
  });

  test('restoring a snapshot gives back the clock it was taken from', () => {
    const f = playing(MIXED);
    f.tick(26);
    edit(f, MIXED, 1.5);
    f.tick(1);
    const held = f.clock.snapshot();
    const heldHistory = [...held.history];
    const before = {
      toCycle: f.clock.toCycle(1.4),
      toClock: f.clock.toClock(1.9),
      now: f.clock.now(),
      cps: f.clock.cpsNow(),
      phase: f.clock.phaseOffsetSec,
      structure: f.clock.structure,
      map: f.clock.map,
    };
    // A failed evaluate: the clock moved, the scheduler's tempo did not.
    f.clock.edit({
      structure: MIXED_2_4,
      meter: TWO_FOUR,
      nextCps: 0.25,
      codeSetCps: false,
      segments: INITIAL_SEGMENTS,
    });
    expect(f.clock.structure).toBe(MIXED_2_4);
    expect(held.history).toEqual(heldHistory);
    f.clock.restore({ ...held, extra: true } as typeof held);
    expect(Object.keys(f.clock.snapshot()).sort()).toEqual(
      ['history', 'map', 'phaseOffsetSec', 'structure'].sort(),
    );
    expect({
      toCycle: f.clock.toCycle(1.4),
      toClock: f.clock.toClock(1.9),
      now: f.clock.now(),
      cps: f.clock.cpsNow(),
      phase: f.clock.phaseOffsetSec,
      structure: f.clock.structure,
      map: f.clock.map,
    }).toEqual(before);
    expect(f.clock.map).toBe(before.map);
    // Later edits publish new arrays; the held one stays as it was.
    f.tick(1);
    edit(f, MIXED, 0.5);
    f.tick(10);
    edit(f, MIXED, 1);
    expect(held.history).toEqual(heldHistory);
    expect(f.clock.snapshot().history).not.toBe(held.history);
  });

  test('resume assigns map, history and phase, keeping the live structure', () => {
    const f = playing(MIXED);
    f.tick(26);
    edit(f, MIXED, 1.5);
    f.tick(1);
    const held = f.clock.snapshot();
    f.clock.edit({
      structure: MIXED_2_4,
      meter: TWO_FOUR,
      nextCps: 0.25,
      codeSetCps: false,
      segments: INITIAL_SEGMENTS,
    });
    const live = f.clock.structure;
    expect(live).toBe(MIXED_2_4);
    f.clock.resume({ ...held, structure: CONSTANT } as typeof held);
    const after = f.clock.snapshot();
    expect(after.structure).toBe(live);
    expect(after.map).toBe(held.map);
    expect(after.history).toBe(held.history);
    expect(after.phaseOffsetSec).toBe(held.phaseOffsetSec);
  });

  test('resume then setLayout equals restore with the live structure then setLayout', () => {
    const run = (handover: (c: LiveClock, s: ReturnType<LiveClock['snapshot']>) => void) => {
      const f = playing(MIXED);
      f.tick(26);
      edit(f, MIXED, 1.5);
      f.tick(1);
      const held = f.clock.snapshot();
      const anchor = f.clock.anchor();
      f.clock.reset();
      f.clock.setLayout(INITIAL_SEGMENTS);
      handover(f.clock, held);
      f.clock.setLayout(INITIAL_SEGMENTS, anchor);
      const s = f.clock.snapshot();
      return {
        structure: s.structure,
        history: s.history.map((h) => [h.end, h.cps, h.referenceMeter, h.map !== null]),
        phaseOffsetSec: s.phaseOffsetSec,
        map: s.map !== null,
        toCycle: f.clock.toCycle(1.4),
        toClock: f.clock.toClock(1.9),
        now: f.clock.now(),
        cps: f.clock.cpsNow(),
      };
    };
    const viaRestore = run((c, s) => c.restore({ ...s, structure: c.structure }));
    const viaResume = run((c, s) => c.resume(s));
    expect(viaResume).toEqual(viaRestore);
  });

  test('audibleCycle is toCycle(audibleReference()), unclamped; now clamps it at 0', () => {
    const f = playing(MIXED);
    expect(f.port.now()).toBeLessThan(0);
    expect(f.clock.audibleCycle()).toBe(f.clock.toCycle(f.clock.audibleReference()));
    expect(f.clock.audibleCycle()).toBeLessThan(0);
    expect(f.clock.now()).toBe(0);
    f.tick(26);
    edit(f, MIXED, 1.5);
    f.tick(1);
    expect(f.clock.audibleCycle()).toBe(f.clock.toCycle(f.clock.audibleReference()));
    expect(f.clock.now()).toBe(f.clock.audibleCycle());
  });

  test('reset empties the history and zeroes the phase, keeping structure and map', () => {
    const f = playing(MIXED);
    f.tick(26);
    edit(f, MIXED, 1.5);
    const { structure, map } = f.clock.snapshot();
    expect(f.clock.phaseOffsetSec).not.toBe(0);
    f.clock.reset();
    expect(f.clock.snapshot()).toEqual({ structure, map, history: [], phaseOffsetSec: 0 });
    expect(f.clock.structure).toBe(structure);
    expect(f.clock.map).toBe(map);
  });

  test('reads change nothing', () => {
    const f = playing(MIXED);
    f.tick(26);
    edit(f, MIXED, 1.5);
    f.tick(1);
    const before = f.clock.snapshot();
    f.clock.now();
    f.clock.cpsNow();
    f.clock.audibleReference();
    f.clock.toCycle(1.4);
    f.clock.toClock(1.9);
    f.clock.anchor();
    const after = f.clock.snapshot();
    expect(after.structure).toBe(before.structure);
    expect(after.map).toBe(before.map);
    expect(after.history).toBe(before.history);
    expect(after.phaseOffsetSec).toBe(before.phaseOffsetSec);
  });
});

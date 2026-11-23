import { describe, expect, test } from 'bun:test';
import { hapTiming, renderTiming } from '../src/lib/render/timing';
import { buildStructure, withLoops } from '../src/lib/strudel/structure';
import { layoutClock } from '../src/lib/strudel/timing';
import {
  canPlayEnding,
  clipSegments,
  INITIAL_SEGMENTS,
  LINEAR_SEGMENTS,
  locate,
  nextSafeBoundary,
  prune,
  withCountIn,
  withLoop,
  withRelease,
  withStartFrom,
} from '../src/lib/strudel/transport';

const structure = buildStructure({
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'opening', bars: 1 },
    { name: 'F1', bars: 3 },
    { name: 'F2', bars: 3 },
    { name: 'E', bars: 4 },
    { name: 'ending', bars: 1 },
  ],
  loop: { from: 'F1', through: 'E' },
  meterChanges: [
    { bar: 4, meter: '3/8' },
    { bar: 5, meter: '4/4' },
    { bar: 7, meter: '3/8' },
    { bar: 8, meter: '4/4' },
  ],
  loops: 2,
});

describe('meter-aware layout clock', () => {
  test('repeated signatures keep a fixed grid while equivalent beat lengths may change meter', () => {
    const sections = [{ name: 'body', bars: 4 }];
    expect(
      buildStructure({ sections, meterChanges: [{ bar: 2, meter: '4/4' }] }).meterChanges,
    ).toBeUndefined();
    const s = buildStructure({
      meter: '3/4',
      sections,
      meterChanges: [
        { bar: 2, meter: '6/8' },
        { bar: 3, meter: '6/8' },
      ],
    });
    expect(s.meterChanges).toEqual([{ bar: 1, meter: { num: 6, den: 8 } }]);
    expect(layoutClock(s, LINEAR_SEGMENTS).toClock(4)).toBe(4);
  });

  test('one loop pass is 35 quarter notes in both linear and clip layouts', () => {
    const linear = layoutClock(structure, LINEAR_SEGMENTS);
    const clip = layoutClock(structure, clipSegments(structure));
    expect(linear.toClock(11) - linear.toClock(1)).toBe(8.75);
    expect(clip.toClock(11) - clip.toClock(1)).toBe(8.75);
    expect(clip.toClock(21) - clip.toClock(11)).toBe(8.75);
    expect(clip.toClock(22) / structure.cps).toBe(39);
    expect(structure.clipBars).toBe(22);
    expect(structure.clipSeconds).toBe(39);
    expect(withLoops(structure, 3).clipSeconds).toBe(56.5);
    expect(linear.toClock(-0.5)).toBe(-0.5);
    expect(linear.toClock(13) - linear.toClock(12)).toBe(1);
  });

  test('fractional beat positions round-trip, even after many passes', () => {
    const live = layoutClock(structure, INITIAL_SEGMENTS);
    for (const cycle of [-1.25, 0, 2.5, 3, 3.5, 4, 11, 11.75, 500003.5]) {
      expect(live.toCycle(live.toClock(cycle))).toBe(cycle);
    }
    expect(live.toClock(500001) - live.toClock(1)).toBe(50000 * 8.75);
    expect(live.meterAt(13.9)).toEqual({ num: 3, den: 8 });
    expect(live.nextMeterBoundary(13.9)).toBe(14);
  });

  test('a pruned transport can retain its clock anchor and count-in uses the armed meter', () => {
    const s = buildStructure({
      sections: [
        { name: 'initial', bars: 1 },
        { name: 'short', bars: 2 },
      ],
      meterChanges: [{ bar: 2, meter: '7/8' }],
    });
    const start = withStartFrom(INITIAL_SEGMENTS, s, 1, 0);
    const segments = withCountIn(start);
    const clock = layoutClock(s, segments);
    expect(clock.toClock(1)).toBe(7 / 8);
    const anchor = { cycle: 1, clock: clock.toClock(1) };
    expect(layoutClock(s, prune(segments, 1), anchor).toClock(2)).toBe(clock.toClock(2));
    expect(clock.toCycle(clock.toClock(2))).toBe(2);
    expect(clock.meterAt(0)).toEqual({ num: 7, den: 8 });
  });

  test('the first retained manual loop has consistent meter and time before its anchor', () => {
    const s = buildStructure({
      sections: [
        { name: 'intro', bars: 1 },
        { name: 'body', bars: 2 },
      ],
      meterChanges: [{ bar: 3, meter: '3/8' }],
    });
    const segments = prune(withLoop(INITIAL_SEGMENTS, s, 1, 12), 12);
    const clock = layoutClock(s, segments, { cycle: 12, clock: 10 });
    expect(clock.meterAt(11.5)).toEqual({ num: 3, den: 8 });
    expect(clock.toClock(12) - clock.toClock(11.5)).toBe(3 / 16);
    expect(clock.toCycle(clock.toClock(11.5))).toBe(11.5);
  });

  test('an onset in the rounded opening sample of a render chunk stays on frame zero', () => {
    const s = buildStructure({
      bpm: 121,
      sections: [{ name: 'body', bars: 3 }],
      meterChanges: [{ bar: 2, meter: '3/8' }],
    });
    const timing = renderTiming(s, LINEAR_SEGMENTS, s.cps)!;
    const origin = timing.frameAt(2, 44100) / 44100;
    expect(timing.secondsAt(2.000001)).toBeLessThan(origin);
    expect(hapTiming(2.000001, 0.5, 2, s.cps, timing, origin).t).toBe(0);
  });

  test('absolute frame edges round once from integer subdivisions, including a half sample', () => {
    const s = buildStructure({
      bpm: 120,
      meter: '5/4',
      sections: [{ name: 'body', bars: 2, loop: true }],
      meterChanges: [{ bar: 2, meter: '3/16' }],
    });
    const timing = renderTiming(s, LINEAR_SEGMENTS, s.cps)!;
    expect(timing.secondsAt(2)).toBe(2.875);
    expect(timing.frameAt(2, 44100)).toBe(126788);
  });

  test('temporary ranges wrap, and release applies at the authored loop end', () => {
    const audition = withLoop(INITIAL_SEGMENTS, structure, 2, 0, 3);
    expect(locate(7, structure, audition)?.songBar).toBe(4);
    expect(layoutClock(structure, audition).toClock(7)).toBe(6.375);
    expect(
      canPlayEnding(
        true,
        structure,
        { kind: 'loop', section: 2, through: 3 },
        locate(1, structure, audition),
      ),
    ).toBe(false);
    for (const cycle of [2, 5, 8]) {
      expect(
        canPlayEnding(
          true,
          structure,
          { kind: 'song' },
          locate(cycle, structure, INITIAL_SEGMENTS),
        ),
      ).toBe(true);
      expect(nextSafeBoundary(cycle, structure, INITIAL_SEGMENTS, 'loopPass')).toBe(11);
    }
    expect(locate(11, structure, withRelease(INITIAL_SEGMENTS, structure, 11))?.songBar).toBe(11);
    expect(locate(0, structure, withStartFrom(INITIAL_SEGMENTS, structure, 4, 0))?.songBar).toBe(
      11,
    );
  });
});

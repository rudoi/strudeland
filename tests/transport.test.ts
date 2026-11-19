import { describe, expect, test } from 'bun:test';
import { buildStructure, cpsFor } from '../src/lib/strudel/structure';
import {
  canPlayEnding,
  clipSegments,
  inCountIn,
  INITIAL_SEGMENTS,
  LINEAR_SEGMENTS,
  locate,
  nextSafeBoundary,
  pieceEnd,
  prune,
  restartAt,
  withCountIn,
  withLoop,
  withRelease,
  withStartFrom,
  type Located,
  type Segment,
  type View,
} from '../src/lib/strudel/transport';
import { loopView, RELEASE, seg, SONG } from './helpers/song-fixtures';

// intro 2 (bars 0-1), body 4 looping (2-5), tail 2 (6-7); total 8.
const S = buildStructure({
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
});
// a 1, b 2, c 3 with no loop section; total 6.
const NO_LOOP = buildStructure({
  sections: [
    { name: 'a', bars: 1 },
    { name: 'b', bars: 2 },
    { name: 'c', bars: 3 },
  ],
});
const EMPTY = buildStructure({});

/** Song bars for global cycles 0..n-1 (null past the end). */
const bars = (s: typeof S, segments: readonly Segment[], n: number) =>
  Array.from({ length: n }, (_, g) => locate(g, s, segments)?.songBar ?? null);

describe('locate: song view', () => {
  test('intro once, then the loop section wraps forever', () => {
    expect(bars(S, INITIAL_SEGMENTS, 16)).toEqual([0, 1, 2, 3, 4, 5, 2, 3, 4, 5, 2, 3, 4, 5, 2, 3]);
    expect(locate(1000, S, INITIAL_SEGMENTS)).toEqual({ section: 1, localCycle: 2, songBar: 4 });
  });

  test('section and local cycle, including fractional cycles', () => {
    expect(locate(0, S, INITIAL_SEGMENTS)).toEqual({ section: 0, localCycle: 0, songBar: 0 });
    expect(locate(1.5, S, INITIAL_SEGMENTS)).toEqual({
      section: 0,
      localCycle: 1.5,
      songBar: 1.5,
    });
    expect(locate(2, S, INITIAL_SEGMENTS)).toEqual({ section: 1, localCycle: 0, songBar: 2 });
    expect(locate(6.25, S, INITIAL_SEGMENTS)).toEqual({
      section: 1,
      localCycle: 0.25,
      songBar: 2.25,
    });
  });

  test('negative cycles clamp to 0', () => {
    expect(locate(-0.01, S, INITIAL_SEGMENTS)).toEqual({ section: 0, localCycle: 0, songBar: 0 });
    expect(locate(-5, S, INITIAL_SEGMENTS)).toEqual(locate(0, S, INITIAL_SEGMENTS));
  });

  test('start in the intro plays on into the loop', () => {
    expect(bars(S, [seg(0, 1, SONG)], 8)).toEqual([1, 2, 3, 4, 5, 2, 3, 4]);
  });

  test('start in the loop section wraps in it', () => {
    expect(bars(S, [seg(0, 4, SONG)], 6)).toEqual([4, 5, 2, 3, 4, 5]);
  });

  test('start in the tail plays the tail once and ends', () => {
    expect(bars(S, [seg(0, 6, SONG)], 4)).toEqual([6, 7, null, null]);
  });

  test('no loop section plays through once and ends', () => {
    expect(bars(NO_LOOP, INITIAL_SEGMENTS, 8)).toEqual([0, 1, 2, 3, 4, 5, null, null]);
    expect(locate(5.999, NO_LOOP, INITIAL_SEGMENTS)?.section).toBe(2);
    expect(locate(6, NO_LOOP, INITIAL_SEGMENTS)).toBeNull();
  });

  test('a later segment takes over at its cycle', () => {
    const segments = [seg(0, 0, SONG), seg(3, 6, SONG)];
    expect(bars(S, segments, 6)).toEqual([0, 1, 2, 6, 7, null]);
  });

  test('before the first segment, it extends backwards', () => {
    expect(bars(S, [seg(4, 2, SONG)], 6)).toEqual([null, null, 0, 1, 2, 3]);
    expect(locate(1, S, [seg(4, 2, loopView(1))])?.songBar).toBe(3);
    expect(locate(0, S, [seg(4, 4, SONG)])?.songBar).toBe(0);
    expect(locate(0, S, [seg(4, 1, SONG)])).toBeNull();
  });
});

describe('locate: loop view', () => {
  test('loops an intro section', () => {
    expect(bars(S, [seg(0, 0, loopView(0))], 5)).toEqual([0, 1, 0, 1, 0]);
  });

  test('loops the loop section', () => {
    expect(bars(S, [seg(0, 2, loopView(1))], 6)).toEqual([2, 3, 4, 5, 2, 3]);
  });

  test('loops a tail section instead of ending', () => {
    expect(bars(S, [seg(0, 6, loopView(2))], 5)).toEqual([6, 7, 6, 7, 6]);
  });

  test('loops a section of a song with no loop section', () => {
    expect(bars(NO_LOOP, [seg(0, 1, loopView(1))], 5)).toEqual([1, 2, 1, 2, 1]);
  });

  test('takes over mid-song', () => {
    expect(bars(S, [seg(0, 0, SONG), seg(3, 0, loopView(0))], 7)).toEqual([0, 1, 2, 0, 1, 0, 1]);
  });

  test('an out-of-range section locates to null', () => {
    expect(locate(0, S, [seg(0, 0, loopView(3))])).toBeNull();
    expect(locate(0, NO_LOOP, [seg(0, 0, loopView(-1))])).toBeNull();
  });
});

describe('locate: release', () => {
  test('released at a pass start plays the tail and ends', () => {
    const segments = [seg(0, 0, SONG), seg(6, 6, RELEASE)];
    expect(bars(S, segments, 10)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, null, null]);
  });

  test('released mid-pass is still taken at its segment cycle', () => {
    // Armed at the next pass boundary (cycle 10) while cycle 7 played.
    const segments = [seg(0, 0, SONG), seg(10, 6, RELEASE)];
    expect(bars(S, segments, 13)).toEqual([0, 1, 2, 3, 4, 5, 2, 3, 4, 5, 6, 7, null]);
  });

  test('release never wraps', () => {
    expect(bars(S, [seg(0, 2, RELEASE)], 8)).toEqual([2, 3, 4, 5, 6, 7, null, null]);
  });
});

describe('locate: degenerate input', () => {
  test('no sections is always null', () => {
    expect(locate(0, EMPTY, INITIAL_SEGMENTS)).toBeNull();
    expect(locate(10, EMPTY, [seg(0, 0, loopView(0))])).toBeNull();
  });

  test('no segments is null', () => {
    expect(locate(0, S, [])).toBeNull();
  });

  test('a structure shortened below the playhead', () => {
    const shorter = buildStructure({ sections: [{ name: 'a', bars: 2 }] });
    expect(locate(3, shorter, [seg(0, 0, SONG)])).toBeNull();
    // A song view whose loop section was removed plays straight through.
    expect(bars(shorter, [seg(0, 0, SONG)], 3)).toEqual([0, 1, null]);
  });
});

describe('pieceEnd', () => {
  test('section ends and loop wrap points', () => {
    expect(pieceEnd(0, S, INITIAL_SEGMENTS)).toBe(2);
    expect(pieceEnd(1.5, S, INITIAL_SEGMENTS)).toBe(2);
    expect(pieceEnd(2, S, INITIAL_SEGMENTS)).toBe(6);
    expect(pieceEnd(5.9, S, INITIAL_SEGMENTS)).toBe(6);
    expect(pieceEnd(6, S, INITIAL_SEGMENTS)).toBe(10);
    expect(pieceEnd(13, S, INITIAL_SEGMENTS)).toBe(14);
  });

  test('the next segment comes first', () => {
    const segments = [seg(0, 0, SONG), seg(3, 6, SONG)];
    expect(pieceEnd(2, S, segments)).toBe(3);
    expect(pieceEnd(3, S, segments)).toBe(5);
  });

  test('past the end: the next segment or Infinity', () => {
    expect(pieceEnd(7, NO_LOOP, INITIAL_SEGMENTS)).toBe(Infinity);
    expect(pieceEnd(6, NO_LOOP, [seg(0, 0, SONG), seg(9, 0, SONG)])).toBe(9);
    expect(pieceEnd(0, EMPTY, INITIAL_SEGMENTS)).toBe(Infinity);
  });

  test('a precomputed locate gives the same result', () => {
    const segments = [seg(0, 0, SONG), seg(3, 6, SONG), seg(9, 2, loopView(1))];
    for (const g of [0, 1.5, 2, 3, 5, 7, 8, 9, 12, 20]) {
      expect(pieceEnd(g, S, segments, locate(g, S, segments))).toBe(pieceEnd(g, S, segments));
    }
    expect(pieceEnd(-1, S, segments, null)).toBe(0);
  });

  test('negative cycles end at 0', () => {
    expect(pieceEnd(-0.5, S, INITIAL_SEGMENTS)).toBe(0);
    expect(pieceEnd(-3, EMPTY, INITIAL_SEGMENTS)).toBe(0);
  });

  test('pieces tile the timeline with a constant mapping inside each', () => {
    const segments = [seg(0, 0, SONG), seg(7, 4, loopView(1)), seg(12, 6, RELEASE)];
    let g = 0;
    const ends: number[] = [];
    while (g < 20) {
      const end = pieceEnd(g, S, segments);
      const start = locate(g, S, segments);
      for (let t = g; t < Math.min(end, 20); t += 0.5) {
        const here = locate(t, S, segments);
        if (start === null) expect(here).toBeNull();
        else
          expect(here).toEqual({
            ...start,
            localCycle: start.localCycle + t - g,
            songBar: start.songBar + t - g,
          });
      }
      ends.push(end);
      g = end;
    }
    expect(ends).toEqual([2, 6, 7, 9, 12, 14, Infinity]);
  });
});

describe('nextSafeBoundary', () => {
  test("'bar' is the first integer cycle not yet queried", () => {
    expect(nextSafeBoundary(0, S, INITIAL_SEGMENTS, 'bar')).toBe(0);
    expect(nextSafeBoundary(3, S, INITIAL_SEGMENTS, 'bar')).toBe(3);
    expect(nextSafeBoundary(3.0001, S, INITIAL_SEGMENTS, 'bar')).toBe(4);
    expect(nextSafeBoundary(3.9, S, INITIAL_SEGMENTS, 'bar')).toBe(4);
    expect(nextSafeBoundary(-0.1, S, INITIAL_SEGMENTS, 'bar')).toBe(0);
    expect(nextSafeBoundary(1.5, EMPTY, INITIAL_SEGMENTS, 'bar')).toBe(2);
  });

  test("'bar' with bars shorter than the scheduler's lookahead", () => {
    // 400 bpm in 1/16: a bar lasts 0.0375 s, so a 0.2 s lookahead queries ~5.3 bars ahead.
    const fast = buildStructure({ bpm: 400, meter: '1/16', sections: [{ name: 'a', bars: 64 }] });
    const now = 10;
    const lastEnd = now + 0.2 * cpsFor(400, { num: 1, den: 16 });
    const b = nextSafeBoundary(lastEnd, fast, INITIAL_SEGMENTS, 'bar')!;
    expect(b).toBe(16);
    expect(b).toBeGreaterThanOrEqual(lastEnd);
  });

  test("'loopPass' is the next start of a loop pass", () => {
    expect(nextSafeBoundary(2, S, INITIAL_SEGMENTS, 'loopPass')).toBe(2);
    expect(nextSafeBoundary(2.5, S, INITIAL_SEGMENTS, 'loopPass')).toBe(6);
    expect(nextSafeBoundary(5.2, S, INITIAL_SEGMENTS, 'loopPass')).toBe(6);
    expect(nextSafeBoundary(6, S, INITIAL_SEGMENTS, 'loopPass')).toBe(6);
    expect(nextSafeBoundary(7, S, INITIAL_SEGMENTS, 'loopPass')).toBe(10);
    // Looping the loop section explicitly.
    expect(nextSafeBoundary(3, S, [seg(0, 2, loopView(1))], 'loopPass')).toBe(4);
  });

  test("'loopPass' is null outside the loop section", () => {
    expect(nextSafeBoundary(0, S, INITIAL_SEGMENTS, 'loopPass')).toBeNull();
    expect(nextSafeBoundary(1.5, S, INITIAL_SEGMENTS, 'loopPass')).toBe(2);
    expect(nextSafeBoundary(0.5, S, [seg(0, 6, SONG)], 'loopPass')).toBeNull();
    expect(nextSafeBoundary(5, S, [seg(0, 6, SONG)], 'loopPass')).toBeNull();
    expect(nextSafeBoundary(1, S, [seg(0, 0, loopView(0))], 'loopPass')).toBeNull();
    expect(nextSafeBoundary(0, NO_LOOP, INITIAL_SEGMENTS, 'loopPass')).toBeNull();
    expect(nextSafeBoundary(0, EMPTY, INITIAL_SEGMENTS, 'loopPass')).toBeNull();
  });
});

describe('prune', () => {
  const segments = [seg(0, 0, SONG), seg(4, 2, loopView(1)), seg(8, 6, RELEASE)];

  test('keeps the last segment at or before g and every later one', () => {
    expect(prune(segments, 0)).toEqual(segments);
    expect(prune(segments, 3.9)).toEqual(segments);
    expect(prune(segments, 4)).toEqual(segments.slice(1));
    expect(prune(segments, 7)).toEqual(segments.slice(1));
    expect(prune(segments, 8)).toEqual(segments.slice(2));
    expect(prune(segments, 100)).toEqual(segments.slice(2));
  });

  test('before the first segment nothing is pruned', () => {
    expect(prune(segments.slice(1), 1)).toEqual(segments.slice(1));
  });

  test('returns a new list when it drops segments and leaves the input alone', () => {
    const copy = structuredClone(segments);
    const out = prune(segments, 8);
    expect(out).not.toBe(segments);
    expect(segments).toEqual(copy);
  });

  test('returns the input itself when nothing is dropped', () => {
    expect(prune(segments, 0)).toBe(segments);
    expect(prune(segments, 3.9)).toBe(segments);
    expect(prune(INITIAL_SEGMENTS, 5)).toBe(INITIAL_SEGMENTS);
    expect(prune([], 5)).toEqual([]);
  });
});

describe('with*', () => {
  test('INITIAL_SEGMENTS is the default song view and frozen', () => {
    expect(INITIAL_SEGMENTS).toEqual([seg(0, 0, SONG)]);
    expect(Object.isFrozen(INITIAL_SEGMENTS)).toBe(true);
    expect(Object.isFrozen(INITIAL_SEGMENTS[0])).toBe(true);
  });

  test('stopped (at = 0) replaces the list', () => {
    expect(withStartFrom(INITIAL_SEGMENTS, S, 2, 0)).toEqual([seg(0, 6, SONG)]);
    expect(withLoop(INITIAL_SEGMENTS, S, 1, 0)).toEqual([seg(0, 2, loopView(1))]);
    expect(withLoop([seg(0, 6, SONG)], S, 0, 0)).toEqual([seg(0, 0, loopView(0))]);
    expect(withStartFrom([seg(0, 2, loopView(1)), seg(4, 0, SONG)], S, 0, 0)).toEqual([
      seg(0, 0, SONG),
    ]);
  });

  test('playing appends at `at` and keeps the past', () => {
    expect(withStartFrom(INITIAL_SEGMENTS, S, 2, 4)).toEqual([seg(0, 0, SONG), seg(4, 6, SONG)]);
    expect(withLoop(INITIAL_SEGMENTS, S, 0, 3)).toEqual([seg(0, 0, SONG), seg(3, 0, loopView(0))]);
    expect(withRelease(INITIAL_SEGMENTS, S, 6)).toEqual([seg(0, 0, SONG), seg(6, 6, RELEASE)]);
  });

  test('a later call supersedes segments armed at or after its cycle', () => {
    const armed = [seg(0, 0, SONG), seg(10, 6, RELEASE)];
    expect(withLoop(armed, S, 0, 8)).toEqual([seg(0, 0, SONG), seg(8, 0, loopView(0))]);
    expect(withLoop(armed, S, 0, 10)).toEqual([seg(0, 0, SONG), seg(10, 0, loopView(0))]);
    expect(withLoop(armed, S, 0, 12)).toEqual([...armed, seg(12, 0, loopView(0))]);
  });

  test('the mapping before `at` is unchanged', () => {
    const before = bars(S, INITIAL_SEGMENTS, 7);
    const after = withLoop(INITIAL_SEGMENTS, S, 0, 7);
    expect(bars(S, after, 7)).toEqual(before);
    expect(bars(S, after, 10).slice(7)).toEqual([0, 1, 0]);
  });

  test('loop null goes back to song view from the bar playing at `at`', () => {
    const looping = [seg(0, 0, SONG), seg(3, 0, loopView(0))];
    // At cycle 6 the intro loop is at bar 1; the song plays on from there.
    const out = withLoop(looping, S, null, 6);
    expect(out).toEqual([...looping, seg(6, 1, SONG)]);
    expect(bars(S, out, 10).slice(6)).toEqual([1, 2, 3, 4]);
  });

  test('loop null is unchanged when nothing plays at `at`', () => {
    const ended = [seg(0, 6, SONG)];
    expect(withLoop(ended, S, null, 4)).toEqual(ended);
    expect(withLoop(INITIAL_SEGMENTS, EMPTY, null, 0)).toEqual([...INITIAL_SEGMENTS]);
  });

  test('out-of-range sections and a release with no loop section are unchanged', () => {
    expect(withStartFrom(INITIAL_SEGMENTS, S, 3, 2)).toEqual([...INITIAL_SEGMENTS]);
    expect(withStartFrom(INITIAL_SEGMENTS, S, -1, 2)).toEqual([...INITIAL_SEGMENTS]);
    expect(withLoop(INITIAL_SEGMENTS, S, 3, 2)).toEqual([...INITIAL_SEGMENTS]);
    expect(withLoop(INITIAL_SEGMENTS, EMPTY, 0, 0)).toEqual([...INITIAL_SEGMENTS]);
    expect(withRelease(INITIAL_SEGMENTS, NO_LOOP, 2)).toEqual([...INITIAL_SEGMENTS]);
  });

  test('inputs are never mutated and outputs are new lists', () => {
    const input = [seg(0, 0, SONG)];
    const out = withStartFrom(input, S, 1, 3);
    expect(input).toEqual([seg(0, 0, SONG)]);
    expect(out).not.toBe(input);
    expect(withRelease(input, NO_LOOP, 2)).not.toBe(input);
  });

  test('release replaces a loop of the loop section at the pass boundary', () => {
    const looping = [seg(0, 2, loopView(1))];
    const at = nextSafeBoundary(5.5, S, looping, 'loopPass')!;
    expect(at).toBe(8);
    const out = withRelease(looping, S, at);
    expect(bars(S, out, 11)).toEqual([2, 3, 4, 5, 2, 3, 4, 5, 6, 7, null]);
  });

  test('stop resets to the initial segments', () => {
    // The runtime resets with a copy of INITIAL_SEGMENTS; locate then plays from bar 0.
    const reset = [...INITIAL_SEGMENTS];
    expect(bars(S, reset, 3)).toEqual([0, 1, 2]);
  });
});

describe('canPlayEnding', () => {
  const inLoop: Located = { section: 1, localCycle: 1, songBar: 3 };
  const inIntro: Located = { section: 0, localCycle: 1, songBar: 1 };
  const inTail: Located = { section: 2, localCycle: 0, songBar: 6 };

  test('true while playing inside the loop section with no release armed', () => {
    expect(canPlayEnding(true, S, SONG, inLoop)).toBe(true);
    expect(canPlayEnding(true, S, loopView(1), inLoop)).toBe(true);
  });

  test('false otherwise', () => {
    expect(canPlayEnding(false, S, SONG, inLoop)).toBe(false);
    expect(canPlayEnding(true, null, SONG, inLoop)).toBe(false);
    expect(canPlayEnding(true, NO_LOOP, SONG, inLoop)).toBe(false);
    expect(canPlayEnding(true, EMPTY, SONG, inLoop)).toBe(false);
    expect(canPlayEnding(true, S, RELEASE, inLoop)).toBe(false);
    expect(canPlayEnding(true, S, SONG, null)).toBe(false);
    expect(canPlayEnding(true, S, SONG, inIntro)).toBe(false);
    expect(canPlayEnding(true, S, SONG, inTail)).toBe(false);
    expect(canPlayEnding(true, S, loopView(0), inIntro)).toBe(false);
  });
});

describe('clipSegments', () => {
  const withLoops = (loops: number) =>
    buildStructure({
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4, loop: true },
        { name: 'tail', bars: 2 },
      ],
      loops,
    });

  test('one loop is the linear layout', () => {
    const s = withLoops(1);
    expect(clipSegments(s)).toEqual([seg(0, 0, SONG), seg(6, 6, RELEASE)]);
    expect(bars(s, clipSegments(s), 9)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, null]);
  });

  test('three loops repeat the loop section then release the tail', () => {
    const s = withLoops(3);
    expect(clipSegments(s)).toEqual([seg(0, 0, SONG), seg(14, 6, RELEASE)]);
    expect(bars(s, clipSegments(s), 17)).toEqual([
      0,
      1,
      2,
      3,
      4,
      5,
      2,
      3,
      4,
      5,
      2,
      3,
      4,
      5,
      6,
      7,
      null,
    ]);
    expect(s.clipBars).toBe(16);
  });

  test('no loop section returns the initial segments', () => {
    expect(clipSegments(NO_LOOP)).toBe(INITIAL_SEGMENTS);
    expect(bars(NO_LOOP, clipSegments(NO_LOOP), 6)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  test('last loop bar and first tail bar', () => {
    const s = withLoops(3);
    expect(locate(13, s, clipSegments(s))!.songBar).toBe(5);
    expect(locate(14, s, clipSegments(s))!.songBar).toBe(6);
  });
});

describe('count-in', () => {
  const COUNT_IN = { at: 0, songBar: 0, view: { kind: 'count-in' as const } };

  test('withCountIn of INITIAL_SEGMENTS: one bar of count-in, then the song from bar 0', () => {
    const c = withCountIn(INITIAL_SEGMENTS);
    expect(c).toEqual([COUNT_IN, seg(1, 0, SONG)]);
    expect(bars(S, c, 9)).toEqual([null, 0, 1, 2, 3, 4, 5, 2, 3]);
    expect(INITIAL_SEGMENTS).toEqual([seg(0, 0, SONG)]); // not mutated
  });

  test('locate during the count-in is null, also at fractional and negative cycles', () => {
    const c = withCountIn(INITIAL_SEGMENTS);
    expect(locate(0, S, c)).toBeNull();
    expect(locate(0.99, S, c)).toBeNull();
    expect(locate(-3, S, c)).toBeNull();
    expect(locate(1, S, c)).toEqual({ section: 0, localCycle: 0, songBar: 0 });
    expect(locate(1.5, S, c)).toEqual({ section: 0, localCycle: 0.5, songBar: 0.5 });
  });

  test('an armed startFrom plays the section from bar 1', () => {
    const armed = withStartFrom(INITIAL_SEGMENTS, S, 2, 0);
    const c = withCountIn(armed);
    expect(c).toEqual([COUNT_IN, seg(1, 6, SONG)]);
    expect(bars(S, c, 5)).toEqual([null, 6, 7, null, null]);
  });

  test('an armed loopSection loops from bar 1', () => {
    const c = withCountIn(withLoop(INITIAL_SEGMENTS, S, 0, 0));
    expect(c).toEqual([COUNT_IN, seg(1, 0, loopView(0))]);
    expect(bars(S, c, 6)).toEqual([null, 0, 1, 0, 1, 0]);
  });

  test('every segment shifts by one cycle, songBar unchanged', () => {
    const list = [seg(0, 0, SONG), seg(3, 2, loopView(1)), seg(9, 6, RELEASE)];
    expect(withCountIn(list)).toEqual([
      COUNT_IN,
      seg(1, 0, SONG),
      seg(4, 2, loopView(1)),
      seg(10, 6, RELEASE),
    ]);
  });

  test('pieceEnd ends the count-in piece at cycle 1', () => {
    const c = withCountIn(INITIAL_SEGMENTS);
    expect(pieceEnd(0, S, c)).toBe(1);
    expect(pieceEnd(0.5, S, c)).toBe(1);
    expect(pieceEnd(1, S, c)).toBe(3); // the intro, bars 0-1, at cycles 1-2
  });

  test('inCountIn at 0, 0.99, 1, before 0, after prune, and on lists without one', () => {
    const c = withCountIn(INITIAL_SEGMENTS);
    expect(inCountIn(0, c)).toBe(true);
    expect(inCountIn(0.99, c)).toBe(true);
    expect(inCountIn(-0.05, c)).toBe(true);
    expect(inCountIn(1, c)).toBe(false);
    expect(inCountIn(50, c)).toBe(false);
    expect(inCountIn(Number.NaN, c)).toBe(false);
    const pruned = prune(c, 20);
    expect(pruned).toEqual([seg(1, 0, SONG)]);
    expect(inCountIn(0, pruned)).toBe(false);
    expect(inCountIn(0, INITIAL_SEGMENTS)).toBe(false);
    expect(inCountIn(0, [])).toBe(false);
  });

  test('a change made during the count-in replaces the segment at 1 and keeps the count-in', () => {
    const c = withCountIn(INITIAL_SEGMENTS);
    const at = nextSafeBoundary(0.3, S, c, 'bar')!;
    expect(at).toBe(1);
    expect(withStartFrom(c, S, 1, at)).toEqual([COUNT_IN, seg(1, 2, SONG)]);
    expect(nextSafeBoundary(0.3, S, c, 'loopPass')).toBeNull(); // bar 1 is the intro
  });

  test('canPlayEnding is false during the count-in (no located bar)', () => {
    const c = withCountIn(withLoop(INITIAL_SEGMENTS, S, 1, 0));
    expect(canPlayEnding(true, S, loopView(1), locate(0.5, S, c))).toBe(false);
  });

  test('INITIAL_SEGMENTS, LINEAR_SEGMENTS and clipSegments never contain a count-in', () => {
    expect(INITIAL_SEGMENTS).toEqual([seg(0, 0, SONG)]);
    expect(LINEAR_SEGMENTS).toEqual([seg(0, 0, RELEASE)]);
    for (const list of [INITIAL_SEGMENTS, LINEAR_SEGMENTS, clipSegments(S), clipSegments(NO_LOOP)])
      expect(list.some((x) => x.view.kind === 'count-in')).toBe(false);
  });
});

describe('restartAt', () => {
  const s = buildStructure({
    sections: [
      { name: 'intro', bars: 2 },
      { name: 'body', bars: 4, loop: true },
      { name: 'tail', bars: 2 },
    ],
  });

  test('a fresh start at the cycle plays the same song bars from it on', () => {
    // Looping the body from cycle 3, playing the release from 9.
    const seg = withRelease(withLoop(INITIAL_SEGMENTS, s, 1, 3), s, 9);
    const r = restartAt(seg, 5);
    expect(r.cycle).toBe(5);
    for (let g = 5; g < 12; g++) expect(locate(g, s, r.segments)).toEqual(locate(g, s, seg));
    // The segments before the one in force are dropped.
    expect(r.segments.map((x) => x.at)).toEqual([3, 9]);
  });

  test('a count-in before the cycle is dropped', () => {
    const seg = withCountIn(INITIAL_SEGMENTS);
    const r = restartAt(seg, 2);
    expect(r.segments).toEqual([{ at: 1, songBar: 0, view: { kind: 'song' } }]);
    expect(inCountIn(2, r.segments)).toBe(false);
    expect(locate(2, s, r.segments)?.songBar).toBe(1);
  });

  test('the cycle is kept as given (mid-bar too), at or after 0', () => {
    expect(restartAt(INITIAL_SEGMENTS, 3.7).cycle).toBe(3.7);
    expect(restartAt(INITIAL_SEGMENTS, -2).cycle).toBe(0);
    expect(restartAt(INITIAL_SEGMENTS, 0).segments).toBe(INITIAL_SEGMENTS);
  });
});

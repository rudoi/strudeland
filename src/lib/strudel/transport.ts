/**
 * The transport: which song bar plays at each global cycle. Plain data plus pure functions; the
 * runtime owns the one mutable segment list and the `Transport` object around it.
 *
 * Global cycle = the scheduler's cycle (0 at every start). Song bar = a position on the song's own
 * timeline (`section.start + localCycle`). One cycle is one bar.
 *
 * Invariant: segment `at` and `songBar` values are integers (they come from `nextSafeBoundary`,
 * from 0 when stopped, and from section starts), so the mapping only changes at integer cycles.
 * `song()` relies on this to compute its query pieces exactly.
 */
import type { Readable } from 'svelte/store';
import { authoredLoop, sectionRange, type LoopRange, type SongStructure } from './structure';

export type View =
  { kind: 'song' } | { kind: 'loop'; section: number; through?: number } | { kind: 'release' };

/**
 * One bar of silence before the song starts, for a recording's count-in. Only the runtime installs
 * it (`withCountIn`), never as the last segment, so `TransportState` never sees it.
 */
export type CountIn = { kind: 'count-in' };
export type SegmentView = View | CountIn;

/** From global cycle `at`, the song is at `songBar` and moves forward under `view`'s rules. */
export interface Segment {
  at: number;
  songBar: number;
  view: SegmentView;
}

export interface Located {
  section: number;
  localCycle: number;
  songBar: number;
}

export const INITIAL_SEGMENTS: readonly Segment[] = Object.freeze([
  Object.freeze({ at: 0, songBar: 0, view: Object.freeze({ kind: 'song' as const }) }),
]);

/**
 * The linear song timeline: global cycle `g` is song bar `g`, every section once, tail included
 * (`release` never loops). `Runtime.query` lays `song()` out on it, and callers locate its haps
 * with it.
 */
export const LINEAR_SEGMENTS: readonly Segment[] = Object.freeze([
  Object.freeze({ at: 0, songBar: 0, view: Object.freeze({ kind: 'release' as const }) }),
]);

/** Mathematical modulo: the result has the sign of `n`. */
const mod = (a: number, n: number): number => {
  const remainder = a % n;
  return remainder < 0 ? remainder + n : remainder;
};

/**
 * The segment in force at `g`: the last one with `at <= g`. Before the first segment (its
 * predecessors were pruned) the first one is extended backwards.
 */
export function segmentIndex(g: number, seg: readonly Segment[]): number {
  let i = 0;
  while (i + 1 < seg.length && seg[i + 1]!.at <= g) i++;
  return i;
}

/** Repeat range in force for this segment, if it starts before the loop's end. */
export function segmentLoop(s: SongStructure, segment: Segment): LoopRange | null {
  const { view } = segment;
  if (view.kind === 'loop') return sectionRange(s, view.section, view.through ?? view.section);
  if (view.kind !== 'song') return null;
  const loop = authoredLoop(s);
  return loop && segment.songBar < loop.end ? loop : null;
}

export function songBarAt(g: number, s: SongStructure, segment: Segment): number | null {
  const elapsed = g - segment.at;
  const p = segment.songBar + elapsed;
  if (segment.view.kind === 'count-in') return null;
  const loop = segmentLoop(s, segment);
  if (!loop) return segment.view.kind === 'loop' ? null : p;
  return segment.view.kind === 'loop' || p >= loop.end
    ? loop.start + mod(p - loop.start, loop.end - loop.start)
    : p;
}

/** The song position at global cycle `g` (negative clamps to 0); null past the end. */
export function locate(g: number, s: SongStructure, seg: readonly Segment[]): Located | null {
  if (s.sections.length === 0 || seg.length === 0) return null;
  const c = Math.max(0, g);
  const bar = songBarAt(c, s, seg[segmentIndex(c, seg)]!);
  if (bar === null || bar < 0 || bar >= s.totalBars) return null;
  // Sections are few (at most 64); a linear scan is fine.
  for (let i = 0; i < s.sections.length; i++) {
    const section = s.sections[i]!;
    if (bar < section.start + section.bars) {
      return { section: i, localCycle: bar - section.start, songBar: bar };
    }
  }
  return null;
}

/** The count-in segment `withCountIn` puts at global cycle 0 (`songBar` is ignored, kept 0). */
const COUNT_IN_SEGMENT: Segment = Object.freeze({
  at: 0,
  songBar: 0,
  view: Object.freeze({ kind: 'count-in' as const }),
});

/**
 * `seg` delayed by one bar of count-in: the count-in at cycle 0, then every segment one cycle
 * later. `songBar` keeps its meaning (song bars count from `g − segment.at`).
 */
export function withCountIn(seg: readonly Segment[]): Segment[] {
  return [COUNT_IN_SEGMENT, ...seg.map((x) => ({ ...x, at: x.at + 1 }))];
}

/** True when the segment in force at `max(0, g)` is a count-in. */
export function inCountIn(g: number, seg: readonly Segment[]): boolean {
  if (seg.length === 0 || Number.isNaN(g)) return false;
  return seg[segmentIndex(Math.max(0, g), seg)]!.view.kind === 'count-in';
}

/**
 * The first global cycle after `g` where the mapping changes: the end of the current section
 * occurrence or the next segment, whichever comes first. For a negative `g` it is 0, the first
 * cycle that plays. `located` is `locate(g, s, seg)` when the caller already has it.
 */
export function pieceEnd(
  g: number,
  s: SongStructure,
  seg: readonly Segment[],
  located: Located | null = g < 0 ? null : locate(g, s, seg),
): number {
  if (g < 0) return 0;
  let next = Infinity;
  for (const x of seg) {
    if (x.at > g) {
      next = x.at;
      break;
    }
  }
  if (located === null) return next;
  const bars = s.sections[located.section]!.bars;
  return Math.min(next, g + bars - located.localCycle);
}

/**
 * The earliest global cycle where a transport change can take effect without changing what the
 * scheduler has already queried (up to `lastEnd`). `'bar'`: the next integer cycle. `'loopPass'`:
 * the next start of a loop pass; null unless that bar is inside the loop section.
 */
export function nextSafeBoundary(
  lastEnd: number,
  s: SongStructure,
  seg: readonly Segment[],
  kind: 'bar' | 'loopPass',
): number | null {
  const c = Math.max(0, Math.ceil(lastEnd));
  if (kind === 'bar') return c;
  const located = locate(c, s, seg);
  const loop = authoredLoop(s);
  if (!located || !loop || !isLoopPass(s, seg[segmentIndex(c, seg)]!.view, located)) return null;
  if (located.songBar === loop.start) return c;
  return c + loop.end - located.songBar;
}

/**
 * Segments in force from `g` on: those after `g`, plus the last one at or before it. `seg` itself
 * when nothing is dropped (the common case: it runs every frame).
 */
export function prune(seg: readonly Segment[], g: number): readonly Segment[] {
  if (seg.length < 2 || seg[1]!.at > g) return seg;
  let last = -1;
  for (let i = 0; i < seg.length; i++) if (seg[i]!.at <= g) last = i;
  return seg.filter((x, i) => i === last || x.at > g);
}

/** `seg` with `segment` taking over at its `at`: segments from `at` on are superseded. */
function append(seg: readonly Segment[], segment: Segment): Segment[] {
  return [...seg.filter((x) => x.at < segment.at), segment];
}

/**
 * Where a fresh start continues a play from global cycle `at` (any cycle, not only a bar line):
 * the scheduler starts at that cycle, so the segments in force from `at` on keep mapping it to the
 * same song position (a count-in or a superseded segment before `at` is dropped). Without `song()`
 * the pattern simply plays on from cycle `at`.
 */
export function restartAt(
  seg: readonly Segment[],
  at: number,
): { cycle: number; segments: readonly Segment[] } {
  const cycle = Math.max(0, at);
  return { cycle, segments: prune(seg, cycle) };
}

/** Play on in `song` view from the start of `section`, from global cycle `at`. */
export function withStartFrom(
  seg: readonly Segment[],
  s: SongStructure,
  section: number,
  at: number,
): Segment[] {
  const target = s.sections[section];
  if (!target) return [...seg];
  return append(seg, { at, songBar: target.start, view: { kind: 'song' } });
}

/** Loop `section` from its start, from global cycle `at`. `null`: back to `song` view from the bar
 * playing at `at` (unchanged when nothing plays there). */
export function withLoop(
  seg: readonly Segment[],
  s: SongStructure,
  section: number | null,
  at: number,
  through = section,
): Segment[] {
  if (section === null) {
    const located = locate(at, s, seg);
    if (located === null) return [...seg];
    return append(seg, { at, songBar: located.songBar, view: { kind: 'song' } });
  }
  const range = through === null ? null : sectionRange(s, section, through);
  if (!range) return [...seg];
  const view: View =
    range.first === range.last
      ? { kind: 'loop', section }
      : { kind: 'loop', section, through: range.last };
  return append(seg, { at, songBar: range.start, view });
}

/** Leave the loop at `at` (a loop pass boundary) and play the tail once. Unchanged without a loop
 * section. */
export function withRelease(seg: readonly Segment[], s: SongStructure, at: number): Segment[] {
  const loop = authoredLoop(s);
  if (!loop) return [...seg];
  return append(seg, { at, songBar: loop.end, view: { kind: 'release' } });
}

/** The clip layout: the intro, the loop section `loops` times, then the tail once. Without a loop
 * section it is the linear layout. Global cycle `clipBars` and later are past the end. */
export function clipSegments(s: SongStructure): readonly Segment[] {
  const loop = authoredLoop(s);
  if (!loop) return INITIAL_SEGMENTS;
  return withRelease(INITIAL_SEGMENTS, s, loop.start + s.loops * (loop.end - loop.start));
}

function isLoopPass(s: SongStructure, view: SegmentView, located: Located): boolean {
  const loop = authoredLoop(s);
  if (!loop || located.songBar < loop.start || located.songBar >= loop.end) return false;
  return (
    view.kind === 'song' ||
    (view.kind === 'loop' &&
      view.section === loop.first &&
      (view.through ?? view.section) === loop.last)
  );
}

/** Play ending is possible while playing inside the loop section with no release armed yet. */
export function canPlayEnding(
  started: boolean,
  s: SongStructure | null,
  view: View,
  located: Located | null,
): boolean {
  return started && s !== null && located !== null && isLoopPass(s, view, located);
}

export interface TransportState {
  /** The view of the last segment. */
  view: View;
  /** True only while stopped with a non-default segment list (a start or loop is armed). */
  pendingStart: boolean;
  /** The section the armed start or loop begins in while `pendingStart`, else null. */
  armedSection: number | null;
}

export interface Transport {
  readonly store: Readable<TransportState>;
  startFrom(section: number, structure: SongStructure): void;
  loopSection(section: number | null, structure: SongStructure): void;
  loopRange(first: number, last: number, structure: SongStructure): void;
  playEnding(): void;
  /**
   * Back to `INITIAL_SEGMENTS`: drops anything armed or appended, for another song taking over.
   * While playing, the song plays on in `song` view from the global cycle (like a first evaluate).
   */
  reset(): void;
}

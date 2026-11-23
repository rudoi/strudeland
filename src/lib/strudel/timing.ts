/**
 * Maps logical layout cycles (one per written bar) onto the scheduler's reference clock
 * (one cycle per bar at meta.meter). Meter boundaries and repeated lengths accumulate in
 * sixteenth notes, which are integral for every supported denominator.
 */
import { meterAtBar, type Meter, type SongStructure } from './structure';
import { segmentIndex, segmentLoop, songBarAt, type Segment } from './transport';

const ticksPerBar = ({ num, den }: Meter) => (num * 16) / den;

interface MeterRun {
  bar: number;
  ticks: number;
  perBar: number;
}

function runIndex(runs: readonly MeterRun[], bar: number): number {
  let first = 0;
  let last = runs.length;
  while (first + 1 < last) {
    const mid = (first + last) >>> 1;
    if (runs[mid]!.bar <= bar) first = mid;
    else last = mid;
  }
  return first;
}

export interface LayoutClock {
  toClock(cycle: number): number;
  toCycle(clock: number): number;
  meterAt(cycle: number): Meter;
  nextMeterBoundary(cycle: number): number;
}

export function layoutClock(
  s: SongStructure,
  segments: readonly Segment[],
  anchor: { cycle: number; clock: number } = { cycle: 0, clock: 0 },
): LayoutClock {
  if (!s.meterChanges || segments.length === 0) {
    const offset = anchor.clock - anchor.cycle;
    return {
      toClock: (cycle) => cycle + offset,
      toCycle: (clock) => clock - offset,
      meterAt: () => s.meter,
      nextMeterBoundary: () => Infinity,
    };
  }

  const reference = ticksPerBar(s.meter);
  const runs: MeterRun[] = [{ bar: 0, ticks: 0, perBar: reference }];
  for (const change of s.meterChanges) {
    const tail = runs[runs.length - 1]!;
    runs.push({
      bar: change.bar,
      ticks: tail.ticks + (change.bar - tail.bar) * tail.perBar,
      perBar: ticksPerBar(change.meter),
    });
  }

  function songTicks(bar: number): number {
    const run = runs[runIndex(runs, bar)]!;
    return run.ticks + (bar - run.bar) * run.perBar;
  }

  const spans = segments.map((segment, i) => {
    const loop = segmentLoop(s, segment);
    const loopStart = loop ? songTicks(loop.start) : 0;
    const loopTicks = loop ? songTicks(loop.end) - loopStart : 0;
    const startMeter = meterAtBar(s, segments[i + 1]?.songBar ?? 0);

    function elapsedTicks(cycle: number): number {
      const elapsed = cycle - segment.at;
      if (segment.view.kind === 'count-in') return elapsed * ticksPerBar(startMeter);
      const linearBar = segment.songBar + elapsed;
      if (!loop || linearBar < loop.end) return songTicks(linearBar) - songTicks(segment.songBar);
      const width = loop.end - loop.start;
      const passes = Math.floor((linearBar - loop.start) / width);
      const bar = linearBar - passes * width;
      return passes * loopTicks + songTicks(bar) - songTicks(segment.songBar);
    }

    function meterAt(cycle: number): Meter {
      if (segment.view.kind === 'count-in') return startMeter;
      const bar = songBarAt(cycle, s, segment);
      return bar === null ? s.meter : meterAtBar(s, bar);
    }

    function nextBoundary(cycle: number): number {
      const segmentEnd = segments[i + 1]?.at ?? Infinity;
      if (segment.view.kind === 'count-in') return segmentEnd;
      const bar = songBarAt(cycle, s, segment);
      if (bar === null) return segmentEnd;
      const nextBar = runs[runIndex(runs, bar) + 1]?.bar ?? Infinity;
      if (!loop) return Math.min(segmentEnd, cycle + (nextBar - bar));
      if (nextBar < loop.end) return Math.min(segmentEnd, cycle + (nextBar - bar));
      const firstChange = runs[runIndex(runs, loop.start) + 1]?.bar ?? Infinity;
      if (firstChange >= loop.end) return segmentEnd;
      return Math.min(segmentEnd, cycle + (loop.end - bar));
    }

    return { elapsedTicks, meterAt, nextBoundary };
  });

  const offsets = [0];
  for (let i = 1; i < segments.length; i++) {
    offsets.push(offsets[i - 1]! + spans[i - 1]!.elapsedTicks(segments[i]!.at));
  }

  function rawTicks(cycle: number): number {
    const i = segmentIndex(cycle, segments);
    return offsets[i]! + spans[i]!.elapsedTicks(cycle);
  }

  const anchorTicks = rawTicks(anchor.cycle);
  const toClock = (cycle: number) => anchor.clock + (rawTicks(cycle) - anchorTicks) / reference;

  // Bound the search in integer cycles using the minimum bar size (one sixteenth).
  // Interpolate only once between adjacent integer cycles to recover the fractional position.
  function toCycle(clock: number): number {
    if (!Number.isFinite(clock)) return clock;
    if (clock === anchor.clock) return anchor.cycle;
    const limit = Math.ceil(Math.abs(clock - anchor.clock) * reference) + 1;
    let lo = Math.floor(anchor.cycle + (clock < anchor.clock ? -limit : 0));
    let hi = Math.ceil(anchor.cycle + (clock > anchor.clock ? limit : 0));
    while (hi - lo > 1) {
      const mid = lo + Math.floor((hi - lo) / 2);
      if (toClock(mid) <= clock) lo = mid;
      else hi = mid;
    }
    const start = toClock(lo);
    return lo + (clock - start) / (toClock(hi) - start);
  }

  return {
    toClock,
    toCycle,
    meterAt: (cycle) => spans[segmentIndex(cycle, segments)]!.meterAt(cycle),
    nextMeterBoundary: (cycle) => spans[segmentIndex(cycle, segments)]!.nextBoundary(cycle),
  };
}

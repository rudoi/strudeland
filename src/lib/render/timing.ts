import { beatsPerBar, bpmFromCps, cpsAtMeter, type SongStructure } from '../strudel/structure';
import { layoutClock } from '../strudel/timing';
import { clipSegments, LINEAR_SEGMENTS, type Segment } from '../strudel/transport';
import { windowFrames, type SegmentLayout } from './contract';

export interface RenderTiming {
  secondsAt(cycle: number): number;
  cycleAt(seconds: number): number;
  cpsAt(cycle: number): number;
  frameAt(cycle: number, sampleRate: number): number;
  minBarSec: number;
}

export function layoutSegments(
  layout: SegmentLayout,
  structure: SongStructure,
): readonly Segment[] {
  return layout === 'clip' ? clipSegments(structure) : LINEAR_SEGMENTS;
}

export function renderWindowFrames(
  from: number,
  to: number,
  cps: number,
  sampleRate: number,
  timing?: RenderTiming,
): number {
  return timing
    ? timing.frameAt(to, sampleRate) - timing.frameAt(from, sampleRate)
    : windowFrames(from, to, cps, sampleRate);
}

/** The reference clock uses the song's base meter. A constant-meter song uses the legacy
 * arithmetic, including its floating-point rounding, throughout the renderer. */
export function renderTiming(
  structure: SongStructure,
  segments: readonly Segment[],
  cps: number,
): RenderTiming | undefined {
  if (!structure.meterChanges?.length) return undefined;
  const clock = layoutClock(structure, segments);
  const bpm = cps === structure.cps ? structure.bpm : bpmFromCps(cps, structure.meter);
  const baseBeats = structure.beatsPerBar;
  const minBeats = structure.meterChanges.reduce(
    (min, change) => Math.min(min, beatsPerBar(change.meter)),
    baseBeats,
  );
  return {
    secondsAt: (cycle) => (clock.toSixteenths(cycle) * 15) / bpm,
    cycleAt: (seconds) => clock.toCycle(seconds * cps),
    cpsAt: (cycle) => cpsAtMeter(cps, structure.meter, clock.meterAt(cycle)),
    frameAt: (cycle, sampleRate) => Math.round((clock.toSixteenths(cycle) * 15 * sampleRate) / bpm),
    minBarSec: minBeats / baseBeats / cps,
  };
}

export function historyStart(fromBar: number, lookbackSec: number, timing: RenderTiming): number {
  const seconds = Math.max(0, timing.secondsAt(fromBar) - lookbackSec);
  return Math.max(0, Math.floor(timing.cycleAt(seconds) + 1e-9));
}

export function hapTiming(
  onset: number,
  duration: number,
  start: number,
  cps: number,
  timing?: RenderTiming,
  bufferStartSec?: number,
): { t: number; duration: number; cps: number; phaseSec?: number } {
  if (!timing) return { t: (onset - start) / cps, duration: duration / cps, cps };
  const phaseSec = timing.secondsAt(onset);
  // Anchor jobs to the same absolute frame grid. Rounding the chunk origin up
  // can put an onset from the opening fraction of a sample just before frame zero.
  const origin = bufferStartSec ?? timing.secondsAt(start);
  return {
    t: Math.max(0, phaseSec - origin),
    duration: timing.secondsAt(onset + duration) - phaseSec,
    cps: timing.cpsAt(onset),
    phaseSec,
  };
}

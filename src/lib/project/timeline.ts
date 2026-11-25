/** Pure helpers behind `Timeline.svelte`: layout and input validation. */
import {
  authoredLoop,
  barsProblem,
  beatsPerBar,
  bpmProblem,
  loopsProblem,
  meterProblem,
  meterString,
  type Meter,
  type MeterChange,
  type SongStructure,
} from '../strudel/structure';
import type { Located } from '../strudel/transport';
import type { MetaPath } from './meta-write';

/** The ruler of a song with no sections. */
export const FALLBACK_BARS = 16;
/** The most labels the ruler shows. */
export const MAX_RULER_LABELS = 32;

/** The smallest power of two giving at most `MAX_RULER_LABELS` labels over `totalBars`. */
export function rulerStride(totalBars: number): number {
  let stride = 1;
  while (Math.ceil(totalBars / stride) > MAX_RULER_LABELS) stride *= 2;
  return stride;
}

/** Ruler label positions (bars): every multiple of the stride below `totalBars`. */
export function rulerMarks(totalBars: number): number[] {
  if (!(totalBars > 0)) return [];
  const stride = rulerStride(totalBars);
  const marks: number[] = [];
  for (let bar = 0; bar < totalBars; bar += stride) marks.push(bar);
  return marks;
}

/** The beats per bar the ruler draws minor ticks for: integers in this range. */
const MINOR_BEATS = { min: 2, max: 8 } as const;

/**
 * How many minor (beat) ticks the ruler draws over `totalBars`: one per beat, only when every bar
 * is labelled (`rulerStride` is 1) and the meter has a whole number of beats from 2 to 8 (4/4 is 4,
 * 6/8 is 3, 7/8 is 3.5 and gets none). 0 means no minor ticks.
 */
export function minorTicks(totalBars: number, meter: Meter): number {
  if (!(totalBars > 0) || rulerStride(totalBars) !== 1) return 0;
  const beats = beatsPerBar(meter);
  if (!Number.isInteger(beats) || beats < MINOR_BEATS.min || beats > MINOR_BEATS.max) return 0;
  return totalBars * beats;
}

/** Beat grids for each meter run, and meter labels at their written bar lines. */
export function mixedMeterRuler(s: SongStructure): {
  ticks: { from: number; to: number; count: number }[];
  labels: MeterChange[];
} {
  const changes = s.meterChanges ?? [];
  const stride = rulerStride(s.totalBars);
  const ticks = [];
  if (stride === 1) {
    const runs = [{ bar: 0, meter: s.meter }, ...changes];
    for (const [i, { bar: from, meter }] of runs.entries()) {
      const to = runs[i + 1]?.bar ?? s.totalBars;
      if (to > from) ticks.push({ from, to, count: (to - from) * meter.num });
    }
  }
  const labels = [];
  let next = 0;
  for (const change of changes) {
    if (change.bar < next) continue;
    labels.push(change);
    next = change.bar + stride;
  }
  return { ticks, labels };
}

/**
 * How many of a section's `bars` pads are lit at `localCycle` (cycles into the section): the bars
 * reached so far, the current one included. 0 when `bars <= 0`; clamped to 0..bars otherwise, so a
 * negative cycle lights the first pad and one past the end lights them all.
 */
export function litBars(bars: number, localCycle: number): number {
  if (!(bars > 0)) return 0;
  return Math.min(bars, Math.floor(Math.max(0, localCycle)) + 1);
}

/**
 * The structure the Timeline shows and edits: none while the header has a MetaError, since the
 * session then holds the defaults and every edit would be refused.
 */
export const editableStructure = (s: {
  structure: SongStructure | null;
  metaError: string | null;
}): SongStructure | null => (s.metaError === null ? s.structure : null);

/** The bars the ruler spans: the song's, or the 16-bar fallback with no sections. */
export const rulerBars = (s: SongStructure | null): number =>
  s && s.sections.length > 0 ? s.totalBars : FALLBACK_BARS;

/** True when the structure has a loop section followed by at least one more section. */
export const hasEnding = (s: SongStructure | null): boolean =>
  s !== null && (authoredLoop(s)?.end ?? s.totalBars) < s.totalBars;

/**
 * The ruler playhead of a song without sections, as a percentage: the cycle modulo the 16-bar
 * fallback. Null when the evaluated song has sections (its playhead is in the blocks).
 */
export function playheadPct(
  evaluated: SongStructure | null,
  position: { cycle: number },
): number | null {
  if (evaluated && evaluated.sections.length > 0) return null;
  const c = Math.max(0, position.cycle);
  return ((c % FALLBACK_BARS) / FALLBACK_BARS) * 100;
}

/**
 * The playhead inside the blocks: the block of the located section, how far into it
 * (`localCycle` over the block's bars, clamped to 0..1), and the raw, unclamped `localCycle` (for
 * `litBars`). Null past the end, or when the static `structure` (what the blocks show) has no such
 * section.
 */
export function blockPlayhead(
  structure: SongStructure | null,
  located: Located | null,
): { section: number; fraction: number; localCycle: number } | null {
  const sec = located ? structure?.sections[located.section] : undefined;
  if (!located || !sec) return null;
  return {
    section: located.section,
    fraction: Math.min(1, Math.max(0, located.localCycle / sec.bars)),
    localCycle: located.localCycle,
  };
}

export type InputField = 'bpm' | 'meter' | 'loops' | 'bars';

export type ParsedInput =
  { ok: true; path: MetaPath; value: number | string } | { ok: false; error: string };

/**
 * Validate what was typed in a meta input and map it to the edit to apply. `section` is the index
 * for `bars`; `hasLoopSection` is for `loops`.
 */
export function parseInput(
  field: InputField,
  text: string,
  opts: { section?: number; hasLoopSection?: boolean } = {},
): ParsedInput {
  const trimmed = text.trim();
  const num = trimmed === '' ? NaN : Number(trimmed);
  let problem: string | null;
  let path: MetaPath;
  let value: number | string = num;
  switch (field) {
    case 'bpm':
      problem = bpmProblem(num);
      path = 'bpm';
      break;
    case 'meter':
      problem = meterProblem(trimmed);
      path = 'meter';
      value = trimmed;
      break;
    case 'loops':
      problem = loopsProblem(num, opts.hasLoopSection ?? false);
      path = 'loops';
      break;
    case 'bars':
      problem = barsProblem(num);
      path = `sections[${opts.section ?? 0}].bars`;
      break;
  }
  return problem ? { ok: false, error: problem } : { ok: true, path, value };
}

/** What a meta input shows for `s` (`section` is the index for `bars`; '' when out of range). */
export function fieldValue(s: SongStructure, field: InputField, section = 0): string {
  switch (field) {
    case 'bpm':
      return String(s.bpm);
    case 'meter':
      return meterString(s.meter);
    case 'loops':
      return String(s.loops);
    case 'bars':
      return String(s.sections[section]?.bars ?? '');
  }
}

/** The session calls a commit needs. */
export interface MetaEditor {
  applyMeta(path: MetaPath, value: number | string, seenDoc: string): boolean;
  activeText(): string | null;
}

export type CommitOutcome =
  | { kind: 'applied' }
  | { kind: 'invalid'; error: string }
  /** The doc changed since `seen`: nothing applied; reset the input and let the user retry. */
  | { kind: 'stale' }
  /** The header cannot be edited safely (e.g. a duplicate key): nothing applied. */
  | { kind: 'refused' };

/**
 * Validate one input commit and apply it against `seen`, the doc the input last saw. `seen` in the
 * result is what the next commit from the same focus compares against: the doc after an applied
 * edit, the current doc after a stale one (so a retry works), else unchanged.
 */
export function commitInput(
  editor: MetaEditor,
  field: InputField,
  text: string,
  opts: { section?: number; hasLoopSection?: boolean },
  seen: string,
): { outcome: CommitOutcome; seen: string } {
  const r = parseInput(field, text, opts);
  if (!r.ok) return { outcome: { kind: 'invalid', error: r.error }, seen };
  const now = editor.activeText() ?? '';
  if (now !== seen) return { outcome: { kind: 'stale' }, seen: now };
  if (!editor.applyMeta(r.path, r.value, seen)) return { outcome: { kind: 'refused' }, seen };
  return { outcome: { kind: 'applied' }, seen: editor.activeText() ?? '' };
}

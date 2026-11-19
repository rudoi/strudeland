// Haps of the evaluated pattern as inspectable events: bar, beat, section, cleaned value and the
// source line and column each came from. Pure (no Strudel, Tauri or DOM), so it runs under
// `bun test` with hand-made haps as well as real ones.

import type { Meter, SongStructure } from '../strudel/structure';
import { LINEAR_SEGMENTS, locate, type Located } from '../strudel/transport';
import type { EventLocation, InspectedEvent, SectionPos } from './protocol';

/** Longest `Location.text`, in characters. */
const TEXT_MAX = 120;

/** Offsets where each line starts. */
function lineStarts(code: string): number[] {
  const starts = [0];
  for (let i = code.indexOf('\n'); i !== -1; i = code.indexOf('\n', i + 1)) starts.push(i + 1);
  return starts;
}

/** Index of the line containing `offset` (already clamped). */
function lineIndex(starts: number[], offset: number): number {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid]! <= offset) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

const clamp = (code: string, offset: number) =>
  Math.min(Math.max(0, Math.floor(Number.isFinite(offset) ? offset : 0)), code.length);

/** 1-based line and column (UTF-16 code units) of `offset`; past the end clamps to the end. */
export function offsetToLineCol(
  code: string,
  offset: number,
  starts: number[] = lineStarts(code),
): { line: number; col: number } {
  const o = clamp(code, offset);
  const i = lineIndex(starts, o);
  return { line: i + 1, col: o - starts[i]! + 1 };
}

/** The line containing `offset`, trimmed (a CR before the LF included) and cut to 120 chars. */
function lineText(code: string, offset: number, starts: number[]): string {
  const i = lineIndex(starts, clamp(code, offset));
  const end = i + 1 < starts.length ? starts[i + 1]! - 1 : code.length;
  return code.slice(starts[i]!, end).trim().slice(0, TEXT_MAX);
}

/** A hap's `context.locations` as positions in `code`; duplicates (same start and end) dropped. */
export function toLocations(
  hapLocations: { start: number; end: number }[] | undefined,
  code: string,
  starts: number[] = lineStarts(code),
): EventLocation[] {
  if (!Array.isArray(hapLocations)) return [];
  const seen = new Set<string>();
  const out: EventLocation[] = [];
  for (const loc of hapLocations) {
    if (!loc || typeof loc.start !== 'number' || typeof loc.end !== 'number') continue;
    const key = `${loc.start}:${loc.end}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const start = offsetToLineCol(code, loc.start, starts);
    const end = offsetToLineCol(code, loc.end, starts);
    out.push({
      line: start.line,
      col: start.col,
      endLine: end.line,
      endCol: end.col,
      text: lineText(code, loc.start, starts),
    });
  }
  // Source order: Strudel lists a hap's locations in combination order, not position order.
  return out.sort((a, b) => a.line - b.line || a.col - b.col);
}

const isPlain = (v: unknown): v is Record<string, unknown> => {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
};

/** JSON data: finite numbers, strings, booleans, null, and arrays / plain objects of those. */
function isJson(v: unknown): boolean {
  if (v === null || typeof v === 'string' || typeof v === 'boolean') return true;
  if (typeof v === 'number') return Number.isFinite(v);
  if (Array.isArray(v)) return v.every(isJson);
  if (isPlain(v)) return Object.values(v).every(isJson);
  return false;
}

/** A hap value as a JSON object: `_` keys and non-JSON members dropped; a non-object wrapped. */
export function cleanValue(value: unknown): Record<string, unknown> {
  if (isPlain(value)) {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) {
      if (!k.startsWith('_') && isJson(v)) out[k] = v;
    }
    return out;
  }
  return isJson(value) ? { value } : {};
}

/**
 * The 1-based whole-bar position of a `locate` result: the song bar and the section with its bar.
 * `locate` keeps the fraction of a mid-bar cycle; reports use whole bars. Null when the section
 * index is outside the structure.
 */
export function barPosition(
  located: Located,
  structure: SongStructure,
): { songBar: number; section: SectionPos } | null {
  const section = structure.sections[located.section];
  if (!section) return null;
  return {
    songBar: Math.floor(located.songBar) + 1,
    section: { name: section.name, bar: Math.floor(located.localCycle) + 1 },
  };
}

/** Section name and 1-based bar within it at song position `begin`; null outside the song. */
function sectionAt(begin: number, structure: SongStructure | null): SectionPos | null {
  // Before bar 1 (a continuation under `all`) is outside every section.
  if (structure === null || begin < 0) return null;
  const located = locate(Math.floor(begin), structure, LINEAR_SEGMENTS);
  return located === null ? null : (barPosition(located, structure)?.section ?? null);
}

/**
 * Haps (from `Runtime.query` over the linear song timeline) as events, sorted by `begin`, then
 * source line and column (events without locations after those with), then input order. Without
 * `all` only onsets are kept. At most `limit` events; `truncated` when more matched.
 */
export function toEvents(
  haps: any[],
  opts: {
    code: string;
    structure: SongStructure | null;
    meter: Meter;
    all: boolean;
    limit: number;
  },
): { events: InspectedEvent[]; truncated: boolean } {
  const events: InspectedEvent[] = [];
  const starts = lineStarts(opts.code);
  for (const hap of haps) {
    const onset = hap.whole !== undefined && hap.whole !== null && hap.hasOnset();
    if (!onset && !opts.all) continue;
    const span = hap.whole ?? hap.part;
    const begin = Number(span.begin);
    const end = Number(span.end);
    const whole = Math.floor(begin);
    events.push({
      bar: whole + 1,
      beat: 1 + (begin - whole) * opts.meter.num,
      begin,
      end,
      onset,
      section: sectionAt(begin, opts.structure),
      value: cleanValue(hap.value),
      locations: toLocations(hap.context?.locations, opts.code, starts),
    });
  }
  // Array.prototype.sort is stable, so equal keys keep their input order.
  events.sort((a, b) => {
    if (a.begin !== b.begin) return a.begin - b.begin;
    const la = a.locations[0];
    const lb = b.locations[0];
    if (la && lb) return la.line - lb.line || la.col - lb.col;
    if (la) return -1;
    if (lb) return 1;
    return 0;
  });
  const truncated = events.length > opts.limit;
  return { events: truncated ? events.slice(0, opts.limit) : events, truncated };
}

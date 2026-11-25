/**
 * `song(meta, parts)`: plays a song's sections with exact boundaries (intro once, the loop section
 * forever, the tail on release), each section in its own local time from cycle 0. The mapping from
 * global cycles to sections is `transport.ts`'s `locate`, the same one the playhead uses.
 *
 * No runtime or `repl()` import: the runtime installs the segment source and reads the registry.
 */
import { Fraction, Hap, Pattern, reify, TimeSpan } from '@strudel/core';
import {
  buildStructure,
  isPlainObject,
  sameStructure,
  StructureError,
  type SongStructure,
} from './structure';
import { INITIAL_SEGMENTS, locate, pieceEnd, type Segment } from './transport';
import type { StrudelPattern } from './types';

let segmentSource: () => readonly Segment[] = () => INITIAL_SEGMENTS;
let registered: SongStructure | null = null;

/** Survives normal context spreads and stays out of JSON song data. */
export const songPlacement = Symbol('written song placement');
let observingPlacement = false;

/** Only inspection and final event edits need arrangement provenance through outer transforms. */
export function withSongPlacement<T>(query: () => T): T {
  const previous = observingPlacement;
  observingPlacement = true;
  try {
    return query();
  } finally {
    observingPlacement = previous;
  }
}

/** Called once by the runtime; the getter is read at every query. */
export function setSegmentSource(get: () => readonly Segment[]): void {
  segmentSource = get;
}

/** Clears the registry; the runtime calls it before each evaluate. */
export function beginEvaluate(): void {
  registered = null;
}

/** The structure the last evaluate's `song()` calls registered, or null if none ran. */
export function registeredStructure(): SongStructure | null {
  return registered;
}

function structureOf(meta: unknown): SongStructure {
  if (!isPlainObject(meta)) throw new Error('song: the first argument must be the meta object');
  let structure: SongStructure;
  try {
    structure = buildStructure(meta);
  } catch (e) {
    if (e instanceof StructureError) throw new Error(`song: ${e.message}`);
    throw e;
  }
  if (structure.sections.length === 0) {
    throw new Error("song: meta.sections is required, e.g. [{ name: 'intro', bars: 4 }]");
  }
  return structure;
}

function partsOf(structure: SongStructure, parts: unknown): any[] {
  if (!isPlainObject(parts)) {
    throw new Error('song: the second argument must be an object of parts, e.g. { intro: pat }');
  }
  const names = new Set(structure.sections.map((s) => s.name));
  for (const key of Object.keys(parts)) {
    if (!names.has(key)) throw new Error(`song: part '${key}' is not a section in meta.sections`);
  }
  return structure.sections.map((s) => {
    const part = Object.prototype.hasOwnProperty.call(parts, s.name) ? parts[s.name] : undefined;
    if (part === undefined) throw new Error(`song: no part for section '${s.name}'`);
    return reify(part);
  });
}

function register(structure: SongStructure) {
  if (registered && !sameStructure(registered, structure)) {
    throw new Error('song: one structure per song');
  }
  registered ??= structure;
}

/**
 * A pattern playing `parts[name]` for each section of `meta.sections`, in section-local time, laid
 * out by the transport's segments. Throws on a missing or invalid structure, a missing part, a part
 * for no section, or a second, different structure in the same evaluate.
 */
export function song(
  meta: Record<string, unknown>,
  parts: Record<string, unknown>,
): StrudelPattern {
  const structure = structureOf(meta);
  const pats = partsOf(structure, parts);
  register(structure);

  return new Pattern((state: any) => {
    // `Runtime.query` passes the linear layout in the query controls; playback reads the live one.
    const segments: readonly Segment[] = state.controls?._segments ?? segmentSource();
    const end = state.span.end;
    if (end.lt(0) || (end.eq(0) && state.span.begin.lt(0))) return [];
    let g = state.span.begin.lt(0) ? Fraction(0) : state.span.begin;
    const haps: any[] = [];
    // do-while, so a zero-width query is answered too.
    do {
      // The mapping only changes at integer cycles (segment invariant), so the bar containing `g`
      // gives the exact section occurrence and the exact end of this piece.
      const cycle: number = g.floor().valueOf();
      const located = locate(cycle, structure, segments);
      const boundary = pieceEnd(cycle, structure, segments, located);
      const to = boundary === Infinity ? end : end.min(Fraction(boundary));
      if (to.lte(g) && g.lt(end))
        throw new Error('song: transport segments must start at whole cycles');
      if (located === null) {
        if (boundary === Infinity) break;
      } else {
        const offset = cycle - located.localCycle; // global cycle where this occurrence starts
        const bars = structure.sections[located.section]!.bars;
        const occurrence = new TimeSpan(offset, offset + bars);
        const piece = new TimeSpan(g, to);
        const local = state.setSpan(new TimeSpan(g.sub(offset), to.sub(offset)));
        for (const hap of pats[located.section].query(local)) {
          const shifted = hap.withSpan((span: any) => span.withTime((t: any) => t.add(offset)));
          const part = shifted.part.intersection(piece);
          if (!part) continue;
          let whole;
          if (shifted.whole) {
            // Clipped to the section occurrence (resetJoin style), never to a segment boundary,
            // so a transport change mid-occurrence does not retrigger a sounding note.
            whole = shifted.whole.intersection(occurrence);
            if (!whole) continue;
          }
          const context = observingPlacement
            ? { ...hap.context, [songPlacement]: whole ?? part }
            : hap.context;
          haps.push(new Hap(whole, part, hap.value, context, hap.stateful));
        }
      }
      g = to;
    } while (g.lt(end));
    return haps;
  });
}

import {
  Fraction,
  Hap,
  Pattern,
  State,
  TimeSpan,
  freqToMidi,
  isNote,
  midiToFreq,
  noteToMidi,
  register,
} from '@strudel/core';
import { spellNote } from './note-name';
import { songPlacement, withSongPlacement } from './song';
import { LINEAR_SEGMENTS, locate, pieceEnd, type Segment } from './transport';
import { cpsAtMeter, meterAtBar, type SongStructure } from './structure';

type Data = null | string | number | boolean | Data[] | { [key: string]: Data };

export interface EventEditIdentity {
  from: [string, string];
  pitch: number;
  voice?: string;
  index?: number;
  count?: number;
  continuous?: boolean;
  period?: number;
  placement?: 'song' | 'absolute';
  capture?: Data;
  entry?: string;
}

export interface EventEditEntry extends Omit<EventEditIdentity, 'entry'> {
  id: string;
  to?: string;
  key?: number;
  until?: string;
  remove?: boolean;
}

export interface EventEditSpec {
  entries: EventEditEntry[];
}

let reporting: Set<string> | undefined;

function withReports<T>(sink: Set<string> | undefined, query: () => T): T {
  const previous = reporting;
  reporting = sink;
  try {
    return query();
  } finally {
    reporting = previous;
  }
}

/** Collect stale deterministic entries during one synchronous visible-window query. */
export function inspectEventEdits<T>(query: () => T): { value: T; staleEdits: string[] } {
  const sink = new Set<string>();
  return { value: withReports(sink, query), staleEdits: [...sink] };
}

const marker = Symbol('piano edit');
const EDGE = /^-?\d{1,24}(?:\/[1-9]\d{0,23})?$/;
const fraction = (v: unknown): v is string =>
  typeof v === 'string' && EDGE.test(v) && Math.abs(Number(Fraction(v))) <= 1_000_128;
const finite = (n: unknown): n is number => typeof n === 'number' && Number.isFinite(n);
const record = (x: unknown): x is Record<string, unknown> =>
  !!x && typeof x === 'object' && !Array.isArray(x);
const fields = new Set([
  'id',
  'from',
  'pitch',
  'voice',
  'index',
  'count',
  'continuous',
  'period',
  'placement',
  'capture',
  'to',
  'key',
  'until',
  'remove',
]);

export function validateEventSpec(spec: unknown): asserts spec is EventEditSpec {
  if (
    !record(spec) ||
    Object.keys(spec).length !== 1 ||
    !Array.isArray(spec.entries) ||
    spec.entries.length > 2048
  )
    throw new Error('piano edits: expected a bounded entries list');
  const ids = new Set<string>();
  for (const e of spec.entries) {
    if (
      !record(e) ||
      Object.keys(e).some((k) => !fields.has(k)) ||
      typeof e.id !== 'string' ||
      !/^[a-zA-Z0-9_-]{1,40}$/.test(e.id) ||
      ids.has(e.id) ||
      !Array.isArray(e.from) ||
      e.from.length !== 2 ||
      !e.from.every(fraction) ||
      !Fraction(e.from[0]).lt(Fraction(e.from[1])) ||
      !finite(e.pitch) ||
      (e.placement !== undefined && e.placement !== 'song' && e.placement !== 'absolute') ||
      (e.voice !== undefined && (typeof e.voice !== 'string' || e.voice.length > 2048)) ||
      (e.count !== undefined &&
        (!Number.isSafeInteger(e.count) || Number(e.count) < 1 || Number(e.count) > 16384)) ||
      (e.index !== undefined &&
        (!Number.isSafeInteger(e.index) ||
          Number(e.index) < 0 ||
          Number(e.index) >= Number(e.count ?? 1))) ||
      (e.to !== undefined && !fraction(e.to)) ||
      (e.until !== undefined &&
        (!fraction(e.until) || !Fraction(e.until).gt(Fraction(e.to ?? e.from[0])))) ||
      (e.period !== undefined &&
        (!Number.isSafeInteger(e.period) ||
          Number(e.period) < 1 ||
          Number(e.period) > 1024 ||
          (e.until !== undefined &&
            Number(Fraction(e.until).sub(Fraction(e.to ?? e.from[0]))) >
              Number(e.period) * 128))) ||
      (e.key !== undefined && !finite(e.key)) ||
      (e.remove !== undefined && typeof e.remove !== 'boolean') ||
      (e.continuous !== undefined && typeof e.continuous !== 'boolean') ||
      (e.capture !== undefined && boundedData(e.capture) === undefined)
    )
      throw new Error('piano edits: invalid or repeated event entry');
    ids.add(e.id);
  }
}

function boundedData(value: unknown): Data | undefined {
  let remaining = 256;
  const visit = (x: unknown, depth: number): boolean => {
    if (--remaining < 0 || depth > 6) return false;
    if (x === null || typeof x === 'boolean' || finite(x)) return true;
    if (typeof x === 'string') return x.length <= 512;
    if (!x || typeof x !== 'object') return false;
    if (Array.isArray(x)) return x.length <= 128 && x.every((item) => visit(item, depth + 1));
    if (Object.getPrototypeOf(x) !== null && Object.getPrototypeOf(x) !== Object.prototype)
      return false;
    const keys = Object.keys(x);
    return (
      keys.length <= 64 &&
      keys.every(
        (key) =>
          key.length <= 64 &&
          key !== '__proto__' &&
          Object.hasOwn(Object.getOwnPropertyDescriptor(x, key)!, 'value') &&
          visit((x as Record<string, unknown>)[key], depth + 1),
      )
    );
  };
  if (!visit(value, 0) || JSON.stringify(value).length > 2048) return;
  return value as Data;
}

function frequency(value: unknown): number | undefined {
  if (typeof value !== 'number' && typeof value !== 'string') return;
  const hz = Number(value);
  return finite(hz) && hz > 0 ? hz : undefined;
}

function scalarPitch(value: unknown): number | undefined {
  if (finite(value)) return value;
  if (typeof value !== 'string' || !isNote(value)) return;
  const midi = noteToMidi(value);
  return finite(midi) ? midi : undefined;
}

function octaveOffset(value: unknown) {
  return typeof value === 'number' || typeof value === 'string' ? 12 * Number(value) : 0;
}

export function eventPitch(value: unknown, primitive: boolean): number | undefined {
  if (!record(value)) return primitive ? scalarPitch(value) : undefined;
  const hz = frequency(value.freq);
  const base = hz === undefined ? scalarPitch(value.note) : freqToMidi(hz);
  if (base === undefined) return;
  const midi = base + octaveOffset(value.octave);
  return finite(midi) ? midi : undefined;
}

function retune(value: any, pitch: number | undefined) {
  if (pitch === undefined) return value;
  if (!record(value))
    return typeof value === 'string' && Number.isInteger(pitch) ? spellNote(pitch, value) : pitch;
  const base = pitch - octaveOffset(value.octave);
  if (frequency(value.freq) !== undefined) return { ...value, freq: midiToFreq(base) };
  if (typeof value.note === 'string' && Number.isInteger(base))
    return { ...value, note: spellNote(base, value.note) };
  // Both audio backends treat numeric zero as the missing note default.
  return { ...value, note: base === 0 ? 'c-1' : base };
}

/** The voice evidence is independent of source offsets and the selected pitch. */
function voiceOf(value: any, detailed = false): string {
  if (!record(value)) return typeof value;
  const kind = frequency(value.freq) !== undefined ? 'freq' : 'note';
  if (!detailed) {
    return JSON.stringify([kind, value.s ?? null, value.orbit ?? null, value.label ?? null]);
  }
  const keys = Object.keys(value)
    .filter((k) => k !== 'note' && k !== 'freq')
    .sort();
  const controls = keys.map((k) => [k, boundedData(value[k]) ?? null]);
  return '=' + JSON.stringify([kind, controls]);
}

function locations(hap: any): string {
  return (hap.context?.locations ?? []).map((loc: any) => loc.start + ':' + loc.end).join('/');
}

// Only the relative order is persisted. The absolute ranges are compared again
// inside the current evaluation, where source edits may have moved every range.
function inSourceOrder(haps: any[]): any[] {
  return haps.sort((a, b) => {
    const x = a.context?.locations ?? [],
      y = b.context?.locations ?? [];
    for (let i = 0; i < Math.min(x.length, y.length); i++) {
      const diff = x[i].start - y[i].start || x[i].end - y[i].end;
      if (diff) return diff;
    }
    return x.length - y.length;
  });
}

const spanOf = (hap: any) => hap.whole ?? hap.part;
const placementOf = (hap: any) =>
  hap.context?.[songPlacement]?.equals(spanOf(hap)) ? ('song' as const) : ('absolute' as const);

export function identifyEvent(
  hap: any,
  siblings: readonly any[],
  primitive: boolean,
  stochastic: boolean,
): EventEditIdentity | undefined {
  const pitch = eventPitch(hap.value, primitive);
  if (pitch === undefined || hap.stateful) return;
  const arc = spanOf(hap);
  const from: [string, string] = [arc.begin.toFraction(), arc.end.toFraction()];
  if (!from.every(fraction) || !arc.begin.lt(arc.end)) return;
  const placement = primitive ? undefined : placementOf(hap);
  let voice = voiceOf(hap.value);
  const simultaneous = siblings.filter(
    (h) =>
      (placement === undefined || placementOf(h) === placement) &&
      !!h.whole === !!hap.whole &&
      spanOf(h).equals(arc) &&
      soundingAt(h, hap.part.begin),
  );
  let lane = simultaneous.filter((h) => voiceOf(h.value) === voice);
  if (lane.length > 1) {
    voice = voiceOf(hap.value, true);
    lane = lane.filter((h) => voiceOf(h.value, true) === voice);
  }
  if (voice.length > 2048) return;
  const same = inSourceOrder(lane.filter((h) => eventPitch(h.value, primitive) === pitch));
  const opaqueTie = same.some(
    (h) =>
      h !== hap &&
      locations(h) === locations(hap) &&
      (h.context?.onSchedule !== hap.context?.onSchedule ||
        h.context?.onTrigger !== hap.context?.onTrigger),
  );
  if (opaqueTie) return;
  const index = same.indexOf(hap);
  if (index < 0) return;
  const capture =
    (!hap.whole || stochastic) && lane.length === 1 ? boundedData(hap.value) : undefined;
  if (!hap.whole && capture === undefined) return;
  return {
    from,
    pitch,
    voice,
    ...(placement ? { placement } : {}),
    ...(same.length > 1 ? { index, count: same.length } : {}),
    ...(!hap.whole ? { continuous: true } : {}),
    ...(capture !== undefined ? { capture } : {}),
    ...(typeof hap.context?.[marker] === 'string' ? { entry: hap.context[marker] } : {}),
  };
}

/** Origin probe confirms candidates on the same lane. A fragment never becomes a new onset. */
function candidates(
  haps: readonly any[],
  e: EventEditEntry,
  whole: any,
  continuous = false,
): any[] {
  return haps.filter(
    (h) =>
      (e.placement === undefined || placementOf(h) === e.placement) &&
      (continuous ? !h.whole : h.whole && h.whole.equals(whole)) &&
      (e.voice === undefined || voiceOf(h.value, e.voice.startsWith('=')) === e.voice),
  );
}

function pick(group: any[], e: EventEditEntry, canonical?: any): any {
  if (canonical) {
    const refs = locations(canonical);
    if (refs) {
      const local = group.filter((h) => locations(h) === refs);
      if (
        local.length === 1 &&
        (eventPitch(local[0].value, true) === e.pitch || e.capture !== undefined)
      )
        return local[0];
    }
  }
  const same = inSourceOrder(group.filter((h) => eventPitch(h.value, true) === e.pitch));
  if (same.length === (e.count ?? 1)) return same[e.index ?? 0];
  if (e.capture !== undefined && group.length === 1) return group[0];
}

function soundingAt(hap: any, time: any) {
  return hap.part.begin.eq(time) || (hap.part.begin.lt(time) && hap.part.end.gt(time));
}

/** Rank simultaneous voices at each part onset. A later fragment of the same
 * whole gets the same rank and retains its own span, value and context. */
function selectedParts(group: any[], entry: EventEditEntry, canonical: any): any[] {
  if (group.length <= 1) {
    const matched = pick(group, entry, canonical);
    return matched ? [matched] : [];
  }
  const ordered = [...group].sort((a, b) => a.part.begin.compare(b.part.begin));
  const selected = new Set<any>();
  let active: any[] = [];
  for (let i = 0; i < ordered.length;) {
    const at = ordered[i].part.begin;
    active = active.filter((h) => soundingAt(h, at));
    do {
      active.push(ordered[i++]);
    } while (i < ordered.length && ordered[i].part.begin.eq(at));
    const matched = pick(active, entry, canonical);
    if (matched && matched.part.begin.eq(at)) selected.add(matched);
  }
  return group.filter((h) => selected.has(h));
}

function replacementHap(entry: EventEditEntry, h: any, whole: any, part: any, destScale: any) {
  const old = h?.value;
  const pinned =
    entry.capture !== undefined &&
    (!h || entry.continuous || eventPitch(old, true) !== entry.pitch);
  let value = retune(pinned ? entry.capture : old, entry.key);
  if (entry.until && record(value) && typeof value.duration === 'number')
    value = { ...value, duration: value.duration * Number(destScale) };
  const context = { ...h?.context, [marker]: entry.id };
  if (entry.placement) context[songPlacement] = entry.placement === 'song' ? whole : undefined;
  return new Hap(whole, part, value, context, h?.stateful);
}

export function verifyEventOrigin(
  observation: any,
  identity: EventEditIdentity,
  haps: any[],
  requireOrigin = false,
): EventEditIdentity | undefined {
  const entry = { ...identity, id: '' };
  const arc = spanOf(observation);
  const group = candidates(haps, entry, arc).filter((h) => soundingAt(h, arc.begin));
  const origin = pick(group, entry, observation);
  if (!origin)
    return !requireOrigin && identity.capture !== undefined && group.length === 0
      ? identity
      : undefined;
  const same = inSourceOrder(group.filter((h) => eventPitch(h.value, true) === identity.pitch));
  const index = same.indexOf(origin);
  if (index < 0) return identity.capture === undefined ? undefined : identity;
  if (same.length > 1 && locations(observation) !== locations(origin)) return;
  const { index: _, count: __, ...base } = identity;
  return same.length > 1 ? { ...base, index, count: same.length } : base;
}

interface Layout {
  structure: SongStructure;
  segments(): readonly Segment[];
  cps(): number;
}

interface Piece {
  arc: any;
  offset: number;
}

function periodicPieces(arc: any, source: any, dest: any, period: number): Piece[] {
  const offsets = new Set<number>();
  for (const span of [source, dest]) {
    const first = Number(arc.begin.sub(span.end).div(period).floor()) + 1;
    const last = Number(arc.end.sub(span.begin).div(period).floor());
    for (let i = first; i <= last; i++) offsets.add(i * period);
  }
  return [...offsets].map((offset) => ({ arc, offset }));
}

/** Emit each surviving interval once, including when several edits overlap. */
function outsideCuts(hap: any, cuts: any[]): any[] {
  const remaining = [];
  let begin = hap.part.begin;
  for (const cut of cuts.sort((a, b) => a.begin.compare(b.begin))) {
    if (begin.lt(cut.begin))
      remaining.push(
        new Hap(undefined, new TimeSpan(begin, cut.begin), hap.value, hap.context, hap.stateful),
      );
    begin = begin.max(cut.end);
  }
  if (begin.lt(hap.part.end))
    remaining.push(
      new Hap(undefined, new TimeSpan(begin, hap.part.end), hap.value, hap.context, hap.stateful),
    );
  return remaining;
}

export function editPatternEvents(pattern: any, spec: EventEditSpec, layout?: Layout): any {
  validateEventSpec(spec);
  if (!spec.entries.length) return pattern;
  const entries = spec.entries.map((entry) => {
    const source = new TimeSpan(...entry.from);
    const a = Fraction(entry.to ?? entry.from[0]);
    const dest = new TimeSpan(a, entry.until ?? a.add(source.duration));
    const sourceScale = source.duration.div(dest.duration);
    const destScale = dest.duration.div(source.duration);
    return { entry, source, dest, sourceScale, destScale };
  });
  const queryInput = (state: any) =>
    layout ? withSongPlacement(() => pattern.query(state)) : pattern.query(state);
  return new Pattern((state: any) => {
    const haps = queryInput(state);
    const suppressed = new Set<any>();
    const cuts = new Map<any, any[]>();
    const activeByArc = new Map<any, any[]>();
    const additions: any[] = [];
    const atOriginal = new Map<any, any[]>();
    const probes = new Map<string, any[]>();
    const pieces = [];
    // Split only the edit's coordinates. Querying the main pattern twice can duplicate painters.
    let g = state.span.begin;
    do {
      const cycle = Number(g.floor());
      const seg = state.controls._segments ?? layout?.segments();
      const pos = layout && locate(cycle, layout.structure, seg);
      const end = layout
        ? Math.min(pieceEnd(cycle, layout.structure, seg, pos), Number(state.span.end))
        : Number(state.span.end);
      const to = end === Number(state.span.end) ? state.span.end : Fraction(end);
      if (!layout || (pos && cycle >= 0)) {
        const offset = pos ? cycle - pos.songBar : 0;
        pieces.push({ arc: new TimeSpan(g, to), offset });
      }
      if (to.gte(state.span.end) || to.lte(g)) break;
      g = to;
    } while (true);
    const absolutePiece = [{ arc: state.span, offset: 0 }];
    function probe(query: any, absolute: boolean): any[] {
      const key = (absolute ? 'a' : 'w') + query.begin.toFraction() + ':' + query.end.toFraction();
      const hit = probes.get(key);
      if (hit) return hit;
      let controls = { ...state.controls, painters: [] };
      if (layout) {
        const segments = absolute
          ? (state.controls._segments ?? layout.segments())
          : LINEAR_SEGMENTS;
        const cycle = Number(query.begin.floor());
        const writtenBar = absolute ? locate(cycle, layout.structure, segments)?.songBar : cycle;
        controls = {
          ...controls,
          _segments: segments,
          _cps: cpsAtMeter(
            layout.cps(),
            layout.structure.meter,
            writtenBar === undefined
              ? layout.structure.meter
              : meterAtBar(layout.structure, writtenBar),
          ),
        };
      }
      const probeState = new State(query, controls);
      // A destination lookup can query an unrelated origin outside the inspected window.
      const result = reporting
        ? withReports(undefined, () => queryInput(probeState))
        : queryInput(probeState);
      probes.set(key, result);
      return result;
    }
    for (const { entry, source, dest, sourceScale, destScale } of entries) {
      const periodic = entry.period !== undefined && !layout;
      const absolute = entry.placement === 'absolute';
      const occurrences = periodic
        ? periodicPieces(state.span, source, dest, entry.period!)
        : absolute
          ? absolutePiece
          : pieces;
      for (const { arc, offset } of occurrences) {
        const shift = (span: any) => span.withTime((t: any) => t.add(offset));
        const origin = shift(source),
          target = shift(dest);
        const sourcePart = origin.intersection(arc);
        const part = target.intersection(arc);
        if (!sourcePart && !part) continue;
        const readSource = periodic ? origin : source;
        const onset = entry.continuous
          ? readSource
          : new TimeSpan(readSource.begin, readSource.begin);
        const originGroup = candidates(probe(onset, absolute), entry, readSource, entry.continuous);
        const canonical = pick(originGroup, entry);
        // A stochastic voice that disappeared from this run may still be pinned.
        // Ambiguous or manually invalidated matches leave the original material intact.
        const missingCapture =
          entry.capture !== undefined && !canonical && originGroup.length === 0;
        if (!canonical && !missingCapture) {
          if (entry.capture === undefined) reporting?.add(entry.id);
          continue;
        }
        const inPlace = source.equals(dest) && !entry.continuous;
        let replacedInPlace = false;
        if (sourcePart) {
          let active = activeByArc.get(arc);
          if (!active) {
            active = haps.filter((h: any) => h.part.intersection(arc)) as any[];
            activeByArc.set(arc, active);
          }
          if (entry.continuous) {
            const h = pick(candidates(active, entry, origin, true), entry, canonical);
            const cut = h?.part.intersection(sourcePart);
            if (cut) {
              const previous = cuts.get(h);
              if (previous) previous.push(cut);
              else cuts.set(h, [cut]);
            }
          } else {
            const selected = selectedParts(candidates(active, entry, origin), entry, canonical);
            for (const h of selected) {
              suppressed.add(h);
              if (inPlace && !entry.remove) {
                const originalPart = h.part.intersection(sourcePart);
                if (!originalPart) continue;
                const replacement = replacementHap(entry, h, h.whole, originalPart, destScale);
                replacedInPlace = true;
                const previous = atOriginal.get(h);
                if (previous) previous.push(replacement);
                else atOriginal.set(h, [replacement]);
              }
            }
          }
        }
        if (!part || entry.remove || (inPlace && (!missingCapture || replacedInPlace))) continue;
        // Read the actual source slice for transforms whose whole contains a partially muted gate.
        const inverse = (t: any) => readSource.begin.add(t.sub(target.begin).mul(sourceScale));
        const query = entry.continuous ? readSource : part.withTime(inverse);
        const sourceGroup = candidates(probe(query, absolute), entry, readSource, entry.continuous);
        const selected = selectedParts(sourceGroup, entry, canonical);
        if (!selected.length && entry.capture !== undefined && !canonical && !sourceGroup.length)
          additions.push(replacementHap(entry, undefined, target, part, destScale));
        for (const h of selected) {
          const clipped = entry.continuous
            ? part
            : h.part
                .withTime((t: any) => target.begin.add(t.sub(readSource.begin).mul(destScale)))
                .intersection(part);
          if (clipped) additions.push(replacementHap(entry, h, target, clipped, destScale));
        }
      }
    }
    return [
      ...haps.flatMap((h: any) => {
        const removed = cuts.get(h);
        return removed
          ? outsideCuts(h, removed)
          : (atOriginal.get(h) ?? (suppressed.has(h) ? [] : [h]));
      }),
      ...additions,
    ];
  }, pattern._steps);
}

let installed = false;
export function registerEventEdits(): void {
  if (installed) return;
  if ('_pianoEdits' in Pattern.prototype || 'pianoEdits' in Pattern.prototype)
    throw new Error('piano edits: operation already registered');
  register(
    'pianoEdits',
    (spec: EventEditSpec, pattern: any) => editPatternEvents(pattern, spec),
    true,
    true,
  );
  // Strudel's register assigns both prototype methods before updating the
  // eval scope. Refuse a song's later registration before it mutates either one.
  for (const key of ['pianoEdits', '_pianoEdits'])
    Object.defineProperty(Pattern.prototype, key, { writable: false });
  installed = true;
}

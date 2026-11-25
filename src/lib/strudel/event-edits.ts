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

export function eventPitch(value: unknown, primitive: boolean): number | undefined {
  if (primitive && finite(value)) return value;
  if (primitive && typeof value === 'string' && isNote(value)) return noteToMidi(value);
  if (!record(value)) return;
  const { note, freq, octave } = value;
  const pitch =
    Number(freq) > 0 && Number.isFinite(Number(freq))
      ? freqToMidi(Number(freq))
      : finite(note)
        ? note
        : typeof note === 'string' && isNote(note)
          ? noteToMidi(note)
          : undefined;
  if (pitch === undefined) return;
  const midi = pitch + 12 * Number(octave ?? 0);
  return Number.isFinite(midi) ? midi : undefined;
}

function retune(value: any, pitch: number | undefined) {
  if (pitch === undefined) return value;
  if (!record(value)) return pitch;
  const base = pitch - 12 * Number(value.octave ?? 0);
  return Number(value.freq) > 0 ? { ...value, freq: midiToFreq(base) } : { ...value, note: base };
}

/** The voice evidence is independent of source offsets and the selected pitch. */
function voiceOf(value: any, detailed = false): string {
  if (!record(value)) return typeof value;
  const kind = Number(value.freq) > 0 ? 'freq' : 'note';
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
const sameSpan = (a: any, b: any) => a.begin.eq(b.begin) && a.end.eq(b.end);

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
  let voice = voiceOf(hap.value);
  const simultaneous = siblings.filter(
    (h) =>
      !!h.whole === !!hap.whole &&
      sameSpan(spanOf(h), arc) &&
      h.part.begin.lte(hap.part.begin) &&
      h.part.end.gte(hap.part.begin) &&
      (!h.part.end.eq(hap.part.begin) || h.part.begin.eq(hap.part.begin)),
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
      (continuous ? !h.whole : h.whole && sameSpan(h.whole, whole)) &&
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

export function verifyEventOrigin(
  observation: any,
  identity: EventEditIdentity,
  haps: any[],
): EventEditIdentity | undefined {
  const entry = { ...identity, id: '' };
  const group = candidates(haps, entry, spanOf(observation));
  const origin = pick(group, entry, observation);
  if (!origin) return identity.capture !== undefined && group.length === 0 ? identity : undefined;
  const same = inSourceOrder(group.filter((h) => eventPitch(h.value, true) === identity.pitch));
  const index = same.indexOf(origin);
  if (index < 0) return identity.capture === undefined ? undefined : identity;
  if (locations(observation) !== locations(origin)) return;
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

export function editPatternEvents(pattern: any, spec: EventEditSpec, layout?: Layout): any {
  validateEventSpec(spec);
  if (!spec.entries.length) return pattern;
  const entries = spec.entries.map((entry) => {
    const source = new TimeSpan(...entry.from);
    const a = Fraction(entry.to ?? entry.from[0]);
    const dest = new TimeSpan(a, entry.until ?? a.add(source.duration));
    return { entry, source, dest };
  });
  return new Pattern((state: any) => {
    const haps = pattern.query(state);
    const suppressed = new Set<any>();
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
    function probe(query: any): any[] {
      const key = query.begin.toFraction() + ':' + query.end.toFraction();
      const hit = probes.get(key);
      if (hit) return hit;
      let controls = { ...state.controls, painters: [] };
      if (layout)
        controls = {
          ...controls,
          _segments: LINEAR_SEGMENTS,
          _cps: cpsAtMeter(
            layout.cps(),
            layout.structure.meter,
            meterAtBar(layout.structure, Number(query.begin.floor())),
          ),
        };
      const result = pattern.query(new State(query, controls));
      probes.set(key, result);
      return result;
    }
    for (const { entry, source, dest } of entries) {
      const periodic = entry.period !== undefined && !layout;
      const occurrences = periodic
        ? periodicPieces(state.span, source, dest, entry.period!)
        : pieces;
      for (const { arc, offset } of occurrences) {
        const shift = (span: any) => span.withTime((t: any) => t.add(offset));
        const origin = shift(source),
          target = shift(dest);
        if (!origin.intersection(arc) && !target.intersection(arc)) continue;
        const readSource = periodic ? origin : source;
        const onset = entry.continuous
          ? readSource
          : new TimeSpan(readSource.begin, readSource.begin);
        const originGroup = candidates(probe(onset), entry, readSource, entry.continuous);
        const canonical = pick(originGroup, entry);
        // A stochastic voice that disappeared from this run may still be pinned.
        // Ambiguous or manually invalidated matches leave the original material intact.
        if (!canonical && !(entry.capture !== undefined && originGroup.length === 0)) continue;
        let removed: any;
        if (origin.intersection(arc)) {
          const active = haps.filter((h: any) => h.part.intersection(arc));
          if (entry.continuous) {
            const h = pick(candidates(active, entry, origin, true), entry, canonical);
            if (h && h.part.intersection(origin)) {
              suppressed.add(h);
              for (const part of [
                new TimeSpan(h.part.begin, h.part.end.min(origin.begin)),
                new TimeSpan(h.part.begin.max(origin.end), h.part.end),
              ]) {
                if (part.begin.lt(part.end))
                  additions.push(new Hap(undefined, part, h.value, h.context, h.stateful));
              }
            }
          } else {
            const h = pick(candidates(active, entry, origin), entry, canonical);
            removed = h;
            if (h) suppressed.add(h);
          }
        }
        const part = target.intersection(arc);
        if (!part || entry.remove) continue;
        // Read the actual source slice for transforms whose whole contains a partially muted gate.
        const ratio = source.duration.div(dest.duration);
        const inverse = (t: any) => readSource.begin.add(t.sub(target.begin).mul(ratio));
        const query = entry.continuous ? readSource : part.withTime(inverse);
        const sourceGroup = candidates(probe(query), entry, readSource, entry.continuous);
        const h = pick(sourceGroup, entry, canonical);
        if (!h && (entry.capture === undefined || canonical || sourceGroup.length)) continue;
        const old = h?.value;
        const value =
          entry.capture !== undefined &&
          (!h || entry.continuous || eventPitch(old, true) !== entry.pitch)
            ? entry.capture
            : old;
        let editedValue = retune(value, entry.key);
        if (entry.until && record(editedValue) && typeof editedValue.duration === 'number') {
          editedValue = {
            ...editedValue,
            duration: editedValue.duration * Number(dest.duration.div(source.duration)),
          };
        }
        const clipped =
          h && !entry.continuous
            ? h.part
                .withTime((t: any) => target.begin.add(t.sub(readSource.begin).div(ratio)))
                .intersection(part)
            : part;
        if (clipped) {
          const replacement = new Hap(
            target,
            clipped,
            editedValue,
            { ...h?.context, [marker]: entry.id },
            h?.stateful,
          );
          if (removed && source.begin.eq(dest.begin)) {
            const previous = atOriginal.get(removed);
            if (previous) previous.push(replacement);
            else atOriginal.set(removed, [replacement]);
          } else additions.push(replacement);
        }
      }
    }
    return [
      ...haps.flatMap((h: any) => atOriginal.get(h) ?? (suppressed.has(h) ? [] : [h])),
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
  installed = true;
}

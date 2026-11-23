/**
 * A song's structure and tempo, from its `meta` header: sections, an optional loop range, bpm and
 * time signatures. Pure: no Strudel, Tauri, DOM or `project/` imports, so the static path (`meta.ts`), the
 * evaluated path (`song()`), the headless host and the renderer all share it.
 */

export interface Meter {
  num: number;
  den: number;
}

export interface Section {
  name: string;
  /** Song bar where the section starts (sum of the bars before it). */
  start: number;
  bars: number;
  loop: boolean;
}

export interface LoopRange {
  first: number;
  last: number;
  start: number;
  end: number;
}

export interface MeterChange {
  /** Zero-based bar in the written song. */
  bar: number;
  meter: Meter;
}

export interface SongStructure {
  bpm: number;
  meter: Meter;
  /** Quarter notes per bar: num * 4 / den. */
  beatsPerBar: number;
  cps: number;
  /** [] when meta has no `sections`. */
  sections: Section[];
  /** First section of the authored loop, when one is declared. */
  loopIndex: number | null;
  /** Omitted for a legacy single-section marker. */
  loopRange?: LoopRange;
  meterChanges?: MeterChange[];
  /** meta.loops, default 1. */
  loops: number;
  /** Sum of all bars. */
  totalBars: number;
  /** Start of the loop section, else totalBars. */
  introEnd: number;
  /** null while the loop section loops, else totalBars. */
  endOfSong: number | null;
  /** intro + loops x loop + tail; with no loop section, totalBars. */
  clipBars: number;
  /** Duration at the declared quarter-note tempo, including complete loop passes. */
  clipSeconds: number;
}

export const DEFAULT_BPM = 120;
export const DEFAULT_METER: Meter = { num: 4, den: 4 };

export const MIN_BPM = 20;
export const MAX_BPM = 400;
export const MAX_METER_NUM = 32;
export const METER_DENS: readonly number[] = [1, 2, 4, 8, 16];
export const MAX_BARS = 10000;
export const MAX_LOOPS = 1000;
export const MAX_SECTIONS = 64;

/** Quarter notes per bar. */
export const beatsPerBar = (meter: Meter): number => (meter.num * 4) / meter.den;

/** Cycles per second for quarter-note `bpm`; one cycle is one bar. */
export const cpsFor = (bpm: number, meter: Meter): number => bpm / 60 / beatsPerBar(meter);

/** Local cycles per second at the same quarter-note tempo. */
export function cpsAtMeter(cps: number, reference: Meter, meter: Meter): number {
  const referenceBeats = beatsPerBar(reference);
  const beats = beatsPerBar(meter);
  return beats === referenceBeats ? cps : (cps * referenceBeats) / beats;
}

/** Quarter-note bpm for `cps`; the inverse of `cpsFor`. */
export const bpmFromCps = (cps: number, meter: Meter): number => cps * 60 * beatsPerBar(meter);

export const meterString = (m: Meter): string => `${m.num}/${m.den}`;

const METER_RE = /^(\d+)\/(\d+)$/;

/** `'6/8'` -> `{ num: 6, den: 8 }`; null when invalid (see `meterProblem`). */
export const parseMeter = (s: string): Meter | null => {
  const m = METER_RE.exec(s);
  if (!m) return null;
  const num = Number(m[1]);
  const den = Number(m[2]);
  if (num < 1 || num > MAX_METER_NUM || !METER_DENS.includes(den)) return null;
  return { num, den };
};

export const isInt = (v: unknown, min: number, max: number): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max;

export const isPlainObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** True when `sections` is an array with a `{ loop: true }` entry (checked or not). */
export const hasLoopSection = (sections: unknown): boolean =>
  Array.isArray(sections) && sections.some((s) => isPlainObject(s) && s.loop === true);

// Rule functions: null = valid, else the error message.

export const bpmProblem = (v: unknown): string | null =>
  typeof v === 'number' && Number.isFinite(v) && v >= MIN_BPM && v <= MAX_BPM
    ? null
    : `meta.bpm must be a number from ${MIN_BPM} to ${MAX_BPM}`;

// A 32-bit integer: the legacy RNG adds the seed to the time, so a huge one loses time resolution.
export const MIN_SEED = -2147483648;
export const MAX_SEED = 2147483647;

export const seedProblem = (v: unknown): string | null =>
  isInt(v, MIN_SEED, MAX_SEED)
    ? null
    : `meta.seed must be an integer between ${MIN_SEED} and ${MAX_SEED}`;

/** The live engines a song may name in `meta.engine` (the default is superdough). */
export const engineProblem = (v: unknown): string | null =>
  v === 'superdough' || v === 'rust' ? null : "meta.engine must be 'superdough' or 'rust'";

export const meterProblem = (v: unknown, path = 'meta.meter'): string | null =>
  typeof v === 'string' && parseMeter(v) !== null
    ? null
    : `${path} must be a string 'N/D' with N from 1 to ${MAX_METER_NUM} and D one of ${METER_DENS.join(', ')}`;

/** `path` names the value in the message (`meta.sections[1].bars` when called for a section). */
export const barsProblem = (v: unknown, path = 'bars'): string | null =>
  isInt(v, 1, MAX_BARS) ? null : `${path} must be an integer from 1 to ${MAX_BARS}`;

export const loopsProblem = (v: unknown, hasLoopSection: boolean): string | null => {
  if (!isInt(v, 1, MAX_LOOPS)) return `meta.loops must be an integer from 1 to ${MAX_LOOPS}`;
  if (!hasLoopSection) return 'meta.loops needs a section with loop: true or a meta.loop range';
  return null;
};

const NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;
const SECTION_KEYS = new Set(['name', 'bars', 'loop']);

/** Shape, names, uniqueness and at most one loop section. */
export const sectionsProblem = (v: unknown): string | null => {
  if (!Array.isArray(v) || v.length === 0) {
    return 'meta.sections must be a non-empty array of sections';
  }
  if (v.length > MAX_SECTIONS) return `meta.sections must have at most ${MAX_SECTIONS} sections`;
  const names = new Set<string>();
  let loopAt: number | null = null;
  for (let i = 0; i < v.length; i++) {
    const path = `meta.sections[${i}]`;
    const s: unknown = v[i];
    if (!isPlainObject(s)) return `${path} must be an object like { name: 'intro', bars: 4 }`;
    for (const key of Object.keys(s)) {
      if (!SECTION_KEYS.has(key)) return `${path} has an unknown key '${key}'`;
    }
    if (typeof s.name !== 'string' || !NAME_RE.test(s.name)) {
      return `${path}.name must be an identifier (letters, digits and _, not starting with a digit)`;
    }
    if (names.has(s.name)) return `${path}.name '${s.name}' is already used`;
    names.add(s.name);
    const bars = barsProblem(s.bars, `${path}.bars`);
    if (bars) return bars;
    if ('loop' in s && typeof s.loop !== 'boolean') return `${path}.loop must be true or false`;
    if (s.loop === true) {
      if (loopAt !== null) return `${path}.loop: only one section can loop`;
      loopAt = i;
    }
  }
  return null;
};

export function loopRangeProblem(
  v: unknown,
  sections?: readonly { name: string }[],
): string | null {
  if (!isPlainObject(v) || Object.keys(v).some((key) => key !== 'from' && key !== 'through')) {
    return "meta.loop must be an object like { from: 'first', through: 'last' }";
  }
  const { from, through } = v;
  if (typeof from !== 'string' || !NAME_RE.test(from))
    return 'meta.loop.from must be a section name';
  if (typeof through !== 'string' || !NAME_RE.test(through)) {
    return 'meta.loop.through must be a section name';
  }
  if (!sections) return null;
  if (sections.length === 0) return 'meta.loop needs meta.sections';
  const first = sections.findIndex((s) => s.name === from);
  const last = sections.findIndex((s) => s.name === through);
  if (first === -1) return `meta.loop.from names an unknown section '${from}'`;
  if (last === -1) return `meta.loop.through names an unknown section '${through}'`;
  return last < first ? 'meta.loop.through must be at or after meta.loop.from' : null;
}

export function meterChangesProblem(v: unknown, totalBars?: number): string | null {
  if (!Array.isArray(v)) return 'meta.meterChanges must be an array';
  if (totalBars === 0) return 'meta.meterChanges needs meta.sections';
  let last = 1;
  for (let i = 0; i < v.length; i++) {
    const change: unknown = v[i];
    const path = `meta.meterChanges[${i}]`;
    if (
      !isPlainObject(change) ||
      Object.keys(change).some((key) => key !== 'bar' && key !== 'meter')
    ) {
      return `${path} must be an object like { bar: 2, meter: '3/4' }`;
    }
    if (!isInt(change.bar, 2, totalBars ?? MAX_SECTIONS * MAX_BARS)) {
      const bound = totalBars === undefined ? '' : ` and at most ${totalBars}`;
      return `${path}.bar must be a whole bar number greater than 1${bound}`;
    }
    const bar = change.bar as number;
    if (bar <= last) return `${path}.bar must be after the previous meter change`;
    const problem = meterProblem(change.meter, `${path}.meter`);
    if (problem) return problem;
    last = bar;
  }
  return null;
}

/** Resolve inclusive section indices to the half-open span in the written song. */
export function sectionRange(
  s: Pick<SongStructure, 'sections'>,
  first: number,
  last: number,
): LoopRange | null {
  const from = s.sections[first];
  const through = s.sections[last];
  return from && through && first <= last
    ? { first, last, start: from.start, end: through.start + through.bars }
    : null;
}

export function authoredLoop(s: SongStructure): LoopRange | null {
  return s.loopRange ?? (s.loopIndex === null ? null : sectionRange(s, s.loopIndex, s.loopIndex));
}

export function meterAtBar(s: SongStructure, bar: number): Meter {
  const changes = s.meterChanges;
  if (!changes) return s.meter;
  let lo = 0;
  let hi = changes.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (changes[mid]!.bar <= bar) lo = mid + 1;
    else hi = mid;
  }
  return lo === 0 ? s.meter : changes[lo - 1]!.meter;
}

/** Equal by value (structures are plain JSON data); null equals only null. */
export const sameStructure = (a: SongStructure | null, b: SongStructure | null): boolean =>
  a === b || (a !== null && b !== null && JSON.stringify(a) === JSON.stringify(b));

export class StructureError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'StructureError';
  }
}

const has = (meta: Record<string, unknown>, key: string): boolean =>
  Object.prototype.hasOwnProperty.call(meta, key) && meta[key] !== undefined;

function check(problem: string | null) {
  if (problem !== null) throw new StructureError(problem);
}

/** intro + loops x loop + tail; with no loop section, totalBars. */
function clipDuration(
  s: Pick<SongStructure, 'totalBars' | 'cps' | 'bpm' | 'meter' | 'meterChanges'>,
  loop: LoopRange | null,
  loops: number,
) {
  const loopBars = loop ? loop.end - loop.start : 0;
  const clipBars = s.totalBars + (loops - 1) * loopBars;
  if (!s.meterChanges) return { clipBars, clipSeconds: clipBars / s.cps };
  let clipBeats = 0;
  const runs = [{ bar: 0, meter: s.meter }, ...s.meterChanges];
  for (const [i, run] of runs.entries()) {
    const end = runs[i + 1]?.bar ?? s.totalBars;
    const repeated = loop
      ? Math.max(0, Math.min(end, loop.end) - Math.max(run.bar, loop.start))
      : 0;
    clipBeats += (end - run.bar + repeated * (loops - 1)) * beatsPerBar(run.meter);
  }
  return { clipBars, clipSeconds: (clipBeats * 60) / s.bpm };
}

/**
 * `structure` with its loop section played `loops` times: equal to `buildStructure` of the same
 * meta with `loops` replaced. Throws StructureError (the `meta.loops` problem) when `loops` is not
 * an integer from 1 to MAX_LOOPS or the structure has no loop section.
 */
export function withLoops(structure: SongStructure, loops: number): SongStructure {
  const loop = authoredLoop(structure);
  check(loopsProblem(loops, loop !== null));
  return { ...structure, loops, ...clipDuration(structure, loop, loops) };
}

/**
 * The structure of a song from its `meta` (as evaluated or as parsed). Absent keys use the
 * defaults; throws StructureError (message = the first problem) on an invalid value.
 */
export function buildStructure(meta: Record<string, unknown>): SongStructure {
  let bpm = DEFAULT_BPM;
  if (has(meta, 'bpm')) {
    check(bpmProblem(meta.bpm));
    bpm = meta.bpm as number;
  }
  let meter: Meter = { ...DEFAULT_METER };
  if (has(meta, 'meter')) {
    check(meterProblem(meta.meter));
    meter = parseMeter(meta.meter as string)!;
  }
  const sections: Section[] = [];
  if (has(meta, 'sections')) {
    check(sectionsProblem(meta.sections));
    let start = 0;
    for (const s of meta.sections as Record<string, unknown>[]) {
      const bars = s.bars as number;
      sections.push({ name: s.name as string, start, bars, loop: s.loop === true });
      start += bars;
    }
  }
  const totalBars = sections.reduce((n, s) => n + s.bars, 0);
  const index = sections.findIndex((s) => s.loop);
  let loopRange: LoopRange | null = null;
  if (has(meta, 'loop')) {
    check(loopRangeProblem(meta.loop, sections));
    if (index !== -1) throw new StructureError('meta.loop cannot be used with section loop: true');
    const { from, through } = meta.loop as { from: string; through: string };
    loopRange = sectionRange(
      { sections },
      sections.findIndex((s) => s.name === from),
      sections.findIndex((s) => s.name === through),
    );
  }
  const loop = loopRange ?? sectionRange({ sections }, index, index);
  const loopIndex = loop?.first ?? null;
  let meterChanges: MeterChange[] | undefined;
  if (has(meta, 'meterChanges')) {
    check(meterChangesProblem(meta.meterChanges, totalBars));
    const changes: MeterChange[] = [];
    let previous = meter;
    for (const c of meta.meterChanges as { bar: number; meter: string }[]) {
      const next = parseMeter(c.meter)!;
      if (next.num === previous.num && next.den === previous.den) continue;
      changes.push({ bar: c.bar - 1, meter: next });
      previous = next;
    }
    if (changes.length) meterChanges = changes;
  }
  let loops = 1;
  if (has(meta, 'loops')) {
    check(loopsProblem(meta.loops, loopIndex !== null));
    loops = meta.loops as number;
  }

  const cps = cpsFor(bpm, meter);
  return {
    bpm,
    meter,
    beatsPerBar: beatsPerBar(meter),
    cps,
    sections,
    loopIndex,
    loops,
    totalBars,
    introEnd: loop?.start ?? totalBars,
    endOfSong: loop ? null : totalBars,
    ...clipDuration({ totalBars, cps, bpm, meter, meterChanges }, loop, loops),
    ...(loopRange && { loopRange }),
    ...(meterChanges && { meterChanges }),
  };
}

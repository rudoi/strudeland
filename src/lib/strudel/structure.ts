/**
 * A song's structure and tempo, from its `meta` header: sections (intro, one loop, tail), bpm and
 * meter. Pure: no Strudel, Tauri, DOM or `project/` imports, so the static path (`meta.ts`), the
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

export interface SongStructure {
  bpm: number;
  meter: Meter;
  /** Quarter notes per bar: num * 4 / den. */
  beatsPerBar: number;
  cps: number;
  /** [] when meta has no `sections`. */
  sections: Section[];
  loopIndex: number | null;
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
  /** clipBars / cps. */
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

export const meterProblem = (v: unknown): string | null =>
  typeof v === 'string' && parseMeter(v) !== null
    ? null
    : `meta.meter must be a string 'N/D' with N from 1 to ${MAX_METER_NUM} and D one of ${METER_DENS.join(', ')}`;

/** `path` names the value in the message (`meta.sections[1].bars` when called for a section). */
export const barsProblem = (v: unknown, path = 'bars'): string | null =>
  isInt(v, 1, MAX_BARS) ? null : `${path} must be an integer from 1 to ${MAX_BARS}`;

export const loopsProblem = (v: unknown, hasLoopSection: boolean): string | null => {
  if (!isInt(v, 1, MAX_LOOPS)) return `meta.loops must be an integer from 1 to ${MAX_LOOPS}`;
  if (!hasLoopSection) return 'meta.loops needs a section with loop: true';
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
function clipBarsOf(
  sections: readonly Section[],
  loopIndex: number | null,
  totalBars: number,
  loops: number,
): number {
  return loopIndex === null ? totalBars : totalBars + (loops - 1) * sections[loopIndex]!.bars;
}

/**
 * `structure` with its loop section played `loops` times: equal to `buildStructure` of the same
 * meta with `loops` replaced. Throws StructureError (the `meta.loops` problem) when `loops` is not
 * an integer from 1 to MAX_LOOPS or the structure has no loop section.
 */
export function withLoops(structure: SongStructure, loops: number): SongStructure {
  check(loopsProblem(loops, structure.loopIndex !== null));
  const { sections, loopIndex, totalBars, cps } = structure;
  const clipBars = clipBarsOf(sections, loopIndex, totalBars, loops);
  return { ...structure, loops, clipBars, clipSeconds: clipBars / cps };
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
  const index = sections.findIndex((s) => s.loop);
  const loopIndex = index === -1 ? null : index;
  let loops = 1;
  if (has(meta, 'loops')) {
    check(loopsProblem(meta.loops, loopIndex !== null));
    loops = meta.loops as number;
  }

  const totalBars = sections.reduce((n, s) => n + s.bars, 0);
  const cps = cpsFor(bpm, meter);
  let introEnd = totalBars;
  let endOfSong: number | null = totalBars;
  if (loopIndex !== null) {
    introEnd = sections[loopIndex]!.start;
    endOfSong = null;
  }
  const clipBars = clipBarsOf(sections, loopIndex, totalBars, loops);
  return {
    bpm,
    meter,
    beatsPerBar: beatsPerBar(meter),
    cps,
    sections,
    loopIndex,
    loops,
    totalBars,
    introEnd,
    endOfSong,
    clipBars,
    clipSeconds: clipBars / cps,
  };
}

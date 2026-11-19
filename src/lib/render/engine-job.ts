/**
 * The engine job: everything the Rust engine (`crates/strudeland-engine`) needs to render one
 * chunk, planned from the same selection, window and pause schedule `renderOffline` uses. The
 * render page plans it (`src/render/main.ts`, kind `job`); the harness mutates it in Node
 * (`applyMutation`). Job schema 2. Also the `$num`/`$unsupported`/`$set` value codec, the canonical JSON used for
 * interning, and the shape validator the Rust `job.rs` mirrors rule for rule. Pure: no Strudel,
 * DOM, Tauri or Playwright imports.
 */

import type { HapMutation, RenderRequest } from './contract';
import {
  bucketByBar,
  channelPlan,
  eventSources,
  matchesMutation,
  orbitCreationOrder,
  orbitNumber,
  pauseFrameOf,
  pauseSchedule,
  type Selected,
  type WindowPlan,
} from './plan';
import { eventsFromValues, type RenderEvent } from './schedule';

export const ENGINE_JOB_SCHEMA = 2;
/** Largest `frames` the engine accepts (2^31). */
const MAX_FRAMES = 2 ** 31;
const I32_MIN = -(2 ** 31);
const I32_MAX = 2 ** 31 - 1;
const U32_MAX = 2 ** 32 - 1;
/** Largest `channels.count`: a WAV's 16-bit channel field (`MAX_CHANNELS` in job.rs). */
const MAX_CHANNELS = 65535;

/** A control value as it crosses to the engine: JSON, with non-JSON numbers and members
 *  wrapped (`encodeValue`). */
export type EngineValue =
  null | boolean | number | string | EngineValue[] | { [k: string]: EngineValue };

export interface EngineHap {
  /** Seconds from frame 0 of the chunk (not clamped; the engine applies superdough's late clamp). */
  t: number;
  /** `hap.duration / cps`, seconds (includes `clip`/`legato`). */
  duration: number;
  /** Absolute layout cycle of the onset. */
  cycle: number;
  /** Index into `EngineJob.values`. */
  v: number;
  /** Index of the hap's event in the chunk's events (TS only; the engine ignores it). */
  event: number;
}

export interface EnginePause {
  /** The context time the bar-by-bar loop suspends at before scheduling these haps (0 first). */
  suspendSec: number;
  /** In superdough call order. */
  haps: EngineHap[];
}

export interface EngineOrbit {
  orbit: number;
  /** 0-based output channels. */
  channels: number[];
}

/** Where an engine ref came from (`scripts/engine/provenance.ts`); the engine ignores it.
 *  `schema` follows the job schema (2); `platform` is informational. */
export interface Provenance {
  schema: 2;
  sourceSha256: string;
  strudelCommit: string;
  /** sha256 of `vendor/strudel/PATCHES.md` and every file it lists. */
  strudelPatchesSha256: string;
  /** sha256 of the job planner (`src/lib/render/plan.ts`, `engine-job.ts`). */
  plannerSha256: string;
  playwright: string;
  chromium: string;
  platform: string;
}

export interface EngineJob {
  schema: 2;
  sampleRate: 44100;
  frames: number;
  seed: number;
  cps: number;
  channels: { count: number; multiChannelOrbits: boolean };
  /** Orbit creation order. */
  orbits: EngineOrbit[];
  pauses: EnginePause[];
  /** Interned control maps, `_` keys removed. */
  values: Record<string, EngineValue>[];
  provenance?: Provenance;
}

/** A sound and the controls the engine cannot play, first seen in layout bar `firstBar`. */
export interface UnsupportedSignature {
  s: string;
  keys: string[];
  firstBar: number;
}

// ---- value codec

const NUM = '$num';
const UNSUPPORTED = '$unsupported';
const SET = '$set';

const isPlainObject = (v: object): boolean => {
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
};

function encodeNumber(n: number): EngineValue {
  if (Number.isNaN(n)) return { [NUM]: 'NaN' };
  if (n === Infinity) return { [NUM]: 'Infinity' };
  if (n === -Infinity) return { [NUM]: '-Infinity' };
  if (Object.is(n, -0)) return { [NUM]: '-0' };
  return n;
}

function encodeInner(v: unknown, stack: object[]): EngineValue | undefined {
  if (v === undefined) return undefined;
  if (v === null || typeof v === 'string' || typeof v === 'boolean') return v;
  if (typeof v === 'number') return encodeNumber(v);
  if (typeof v !== 'object') return { [UNSUPPORTED]: typeof v };
  if (stack.includes(v)) return { [UNSUPPORTED]: 'cycle' };
  if (v instanceof Set) {
    stack.push(v);
    const members: EngineValue[] = [];
    for (const m of v) members.push(encodeInner(m, stack) ?? null);
    stack.pop();
    return { [SET]: members };
  }
  if (Array.isArray(v)) {
    stack.push(v);
    const out: EngineValue[] = [];
    for (let i = 0; i < v.length; i++) out.push(encodeInner(v[i], stack) ?? null);
    stack.pop();
    return out;
  }
  if (!isPlainObject(v)) {
    const name = (v as { constructor?: { name?: unknown } }).constructor?.name;
    return { [UNSUPPORTED]: typeof name === 'string' && name !== '' ? name : 'object' };
  }
  if (Object.hasOwn(v, NUM) || Object.hasOwn(v, UNSUPPORTED) || Object.hasOwn(v, SET)) {
    return { [UNSUPPORTED]: 'reserved key' };
  }
  stack.push(v);
  const out: { [k: string]: EngineValue } = {};
  for (const k of Object.keys(v)) {
    const e = encodeInner((v as Record<string, unknown>)[k], stack);
    if (e !== undefined) out[k] = e;
  }
  stack.pop();
  return out;
}

/**
 * A value as plain JSON for the engine. NaN, +-Infinity and -0 become `{ "$num": ... }`; a `Set`
 * (a subclass included) becomes `{ "$set": [members] }` in insertion order; strings, booleans,
 * null, arrays and plain objects recurse (an `undefined` member is omitted, an `undefined`
 * element or set member becomes null); functions, symbols, BigInts, other class instances (a
 * `Map` included) and cycles become `{ "$unsupported": "<typeof or constructor name>" }`, and a
 * plain object that has a `$num`, `$unsupported` or `$set` key of its own becomes
 * `{ "$unsupported": "reserved key" }`.
 * `undefined` itself encodes as null.
 */
export function encodeValue(v: unknown): EngineValue {
  return encodeInner(v, []) ?? null;
}

/** A control map for the engine: top-level `_` keys removed, every member encoded. The map
 *  itself is not checked for reserved keys: a control named `$num` is just an unknown control. */
export function encodeControlMap(value: Record<string, unknown>): Record<string, EngineValue> {
  const out: Record<string, EngineValue> = {};
  const stack: object[] = [value];
  for (const k of Object.keys(value)) {
    if (k.startsWith('_')) continue;
    const e = encodeInner(value[k], stack);
    if (e !== undefined) out[k] = e;
  }
  return out;
}

const NUM_DECODE: Record<string, number> = {
  NaN: NaN,
  Infinity: Infinity,
  '-Infinity': -Infinity,
  '-0': -0,
};

/** An `{ "$unsupported": type }` marker, as `decodeValue` keeps it. */
export interface UnsupportedMarker {
  $unsupported: string;
}

/** Whether `v` is an `{ "$unsupported": type }` marker. */
export function isUnsupportedMarker(v: unknown): v is UnsupportedMarker {
  return (
    typeof v === 'object' &&
    v !== null &&
    !Array.isArray(v) &&
    Object.keys(v).length === 1 &&
    typeof (v as Record<string, unknown>)[UNSUPPORTED] === 'string'
  );
}

/** Why an encoded value is not a valid wrapper, or null when every wrapper in it is valid. */
function wrapperProblem(v: unknown): string | null {
  if (Array.isArray(v)) {
    for (const x of v) {
      const p = wrapperProblem(x);
      if (p !== null) return p;
    }
    return null;
  }
  if (typeof v !== 'object' || v === null) return null;
  const rec = v as Record<string, unknown>;
  if (Object.hasOwn(rec, NUM)) {
    if (Object.keys(rec).length !== 1) return `${NUM} with other keys`;
    const tag = rec[NUM];
    if (typeof tag !== 'string' || !Object.hasOwn(NUM_DECODE, tag)) {
      return `bad ${NUM} ${JSON.stringify(tag)}`;
    }
    return null;
  }
  if (Object.hasOwn(rec, UNSUPPORTED)) {
    if (Object.keys(rec).length !== 1) return `${UNSUPPORTED} with other keys`;
    if (typeof rec[UNSUPPORTED] !== 'string') return `bad ${UNSUPPORTED}`;
    return null;
  }
  if (Object.hasOwn(rec, SET)) {
    if (Object.keys(rec).length !== 1) return `${SET} with other keys`;
    const members = rec[SET];
    if (!Array.isArray(members)) return `bad ${SET}`;
    return wrapperProblem(members);
  }
  for (const x of Object.values(rec)) {
    const p = wrapperProblem(x);
    if (p !== null) return p;
  }
  return null;
}

/**
 * The inverse of `encodeValue`: `$num` wrappers become their numbers, `$set` wrappers a `Set` of
 * their decoded members (in order); `$unsupported` markers are kept as they are (the original is
 * gone; see `isUnsupportedMarker`). Throws on a malformed wrapper (a `$num`, `$unsupported` or
 * `$set` key with other keys, or a bad payload).
 */
export function decodeValue(v: EngineValue): unknown {
  if (Array.isArray(v)) return v.map(decodeValue);
  if (typeof v !== 'object' || v === null) return v;
  const problem = wrapperProblem(v);
  if (problem !== null) throw new RangeError(`invalid value wrapper: ${problem}`);
  if (Object.hasOwn(v, NUM)) return NUM_DECODE[v[NUM] as string];
  if (Object.hasOwn(v, UNSUPPORTED)) return { [UNSUPPORTED]: v[UNSUPPORTED] };
  if (Object.hasOwn(v, SET)) return new Set((v[SET] as EngineValue[]).map(decodeValue));
  const out: Record<string, unknown> = {};
  for (const [k, x] of Object.entries(v)) out[k] = decodeValue(x);
  return out;
}

/** A control map decoded member by member (`decodeValue`); the map itself is never a wrapper. */
export function decodeControlMap(map: Record<string, EngineValue>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(map)) out[k] = decodeValue(v);
  return out;
}

/** `JSON.stringify` with object keys sorted recursively (`Array.prototype.sort`): the same
 *  string for maps that differ only in key order. Used to intern values. */
export function canonicalJson(v: unknown): string {
  return JSON.stringify(v, (_k, x: unknown) => {
    if (typeof x !== 'object' || x === null || Array.isArray(x)) return x;
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(x).sort()) out[k] = (x as Record<string, unknown>)[k];
    return out;
  });
}

// ---- planning

export interface PlannedJob {
  job: EngineJob;
  /** The chunk's unmutated events, `chunkSelected` order (`hap.event` indexes it). */
  events: RenderEvent[];
  /** Stereo pairs of the channel layout (1 for mix). */
  stemPairs: number;
}

/**
 * The engine job for one chunk, mirroring `renderOffline`: the channel plan from the chunk's
 * haps, events before anything else, orbits in creation order from the full schedule and the
 * chunk, the pause schedule, and per bar the haps in superdough call order. `chunkSelected` is
 * `selectHaps(query(plan.c0, plan.c1))`, `scheduleSelected` is `selectHaps(query(0, c1))`.
 * Throws as `renderOffline` does for a bar too short for the pause lead.
 */
export function planEngineJob(
  chunkSelected: Selected[],
  scheduleSelected: Selected[],
  plan: WindowPlan,
  req: RenderRequest,
  cps: number,
  seed: number,
): PlannedJob {
  const layout = channelPlan(
    chunkSelected.map((s) => s.value),
    req.mode,
  );
  const events = eventsFromValues(eventSources(chunkSelected), cps);
  const orbits = orbitCreationOrder(chunkSelected, scheduleSelected, layout.multiChannelOrbits).map(
    (o) => ({ orbit: orbitNumber({ orbit: o.orbit }), channels: o.channels }),
  );

  const nBars = plan.c1 - plan.c0;
  const { suspendSec } = pauseSchedule(nBars, cps, req.sampleRate);
  const buckets = bucketByBar(chunkSelected, plan.c0, nBars);

  const values: Record<string, EngineValue>[] = [];
  const interned = new Map<string, number>();
  const intern = (value: Record<string, unknown>): number => {
    const encoded = encodeControlMap(value);
    const key = canonicalJson(encoded);
    let i = interned.get(key);
    if (i === undefined) {
      i = values.length;
      values.push(encoded);
      interned.set(key, i);
    }
    return i;
  };

  const pauses: EnginePause[] = suspendSec.map((at, j) => ({
    suspendSec: at,
    haps: (buckets[j] ?? []).map((s) => ({
      t: (s.onsetCycle - plan.c0) / cps,
      duration: Number(s.hap.duration) / cps,
      cycle: s.onsetCycle,
      v: intern(s.value),
      event: s.index,
    })),
  }));

  const job: EngineJob = {
    schema: ENGINE_JOB_SCHEMA,
    sampleRate: req.sampleRate,
    frames: plan.frames,
    seed,
    cps,
    channels: { count: layout.channels, multiChannelOrbits: layout.multiChannelOrbits },
    orbits,
    pauses,
    values,
  };
  return { job, events, stemPairs: layout.pairs };
}

/**
 * The job without the haps the mutation drops, judged on the unmutated `events`
 * (`events[hap.event]`, as `renderOffline` does). Orbits, channels, values and the order of the
 * remaining haps are kept; a hap whose event is missing is kept.
 */
export function applyMutation(
  job: EngineJob,
  events: readonly RenderEvent[],
  mutation: HapMutation,
): EngineJob {
  return {
    ...job,
    pauses: job.pauses.map((p) => ({
      suspendSec: p.suspendSec,
      haps: p.haps.filter((h) => {
        const e = events[h.event];
        return e === undefined || !matchesMutation(mutation, e);
      }),
    })),
  };
}

// ---- unsupported signatures (detected by the engine binary: `strudeland-engine signatures`)

/** A signature as a report line: `s=<s> keys=<k1,k2> bar=<n>`. */
export const formatSignature = (sig: UnsupportedSignature): string =>
  `s=${sig.s} keys=${sig.keys.join(',')} bar=${sig.firstBar}`;

// ---- validation

export const isRecord = (x: unknown): x is Record<string, unknown> =>
  typeof x === 'object' && x !== null && !Array.isArray(x);
const isFiniteNumber = (x: unknown): x is number => typeof x === 'number' && Number.isFinite(x);
const isUint = (x: unknown, max: number): x is number =>
  Number.isInteger(x) && (x as number) >= 0 && (x as number) <= max;

const JOB_FIELDS = [
  'schema',
  'sampleRate',
  'frames',
  'seed',
  'cps',
  'channels',
  'orbits',
  'pauses',
  'values',
  'provenance',
];
const REQUIRED_JOB_FIELDS = JOB_FIELDS.filter((f) => f !== 'provenance');

/** Missing and unknown fields of `x` (the engine denies unknown fields). */
function fieldErrors(x: Record<string, unknown>, where: string, known: string[], required = known) {
  const errors: string[] = [];
  for (const f of required) if (!Object.hasOwn(x, f)) errors.push(`${where}: missing ${f}`);
  for (const f of Object.keys(x)) if (!known.includes(f)) errors.push(`${where}: unknown ${f}`);
  return errors;
}

/**
 * Checks an engine job's shape and every rule the engine enforces (`job.rs` `validate`, the same
 * rules in the same order): schema 2 (a schema 1 job is refused: regenerate it); sample rate 44100; integer `frames` in [0, 2^31]; integer
 * `seed` in the i32 range; finite `cps > 0`; integer `channels.count` in [1, 65535]; boolean
 * `multiChannelOrbits`; non-empty `pauses` with `pauses[0].suspendSec === 0`; finite, strictly
 * increasing `suspendSec` whose quantized frames strictly increase and (after the first) lie
 * before `frames`; every hap with finite `t >= 0`, finite `duration >= 0`, finite `cycle`, `v` an
 * index into `values` and `event` an unsigned integer; every orbit finite with non-empty
 * channels, all integers `>= 0`; `values` objects with valid `$num`/`$unsupported`/`$set` wrappers; no
 * missing or unknown fields. Collects every violation.
 */
export function validateEngineJob(x: unknown): { job: EngineJob | null; errors: string[] } {
  if (!isRecord(x)) return { job: null, errors: ['job: not an object'] };
  const errors: string[] = fieldErrors(x, 'job', JOB_FIELDS, REQUIRED_JOB_FIELDS);

  if (x.schema === 1) errors.push('schema 1 job: regenerate it');
  else if (x.schema !== ENGINE_JOB_SCHEMA) errors.push(`schema: ${String(x.schema)}, expected 2`);
  if (x.sampleRate !== 44100) errors.push(`sampleRate: ${String(x.sampleRate)}, expected 44100`);
  const framesOk = isUint(x.frames, MAX_FRAMES);
  if (!framesOk) errors.push(`frames: ${String(x.frames)}, expected an integer in [0, 2^31]`);
  if (!Number.isInteger(x.seed) || (x.seed as number) < I32_MIN || (x.seed as number) > I32_MAX) {
    errors.push(`seed: ${String(x.seed)}, expected an i32`);
  }
  if (!isFiniteNumber(x.cps) || x.cps <= 0) errors.push(`cps: ${String(x.cps)}, expected > 0`);

  if (!isRecord(x.channels)) {
    errors.push('channels: not an object');
  } else {
    errors.push(...fieldErrors(x.channels, 'channels', ['count', 'multiChannelOrbits']));
    const count = x.channels.count;
    if (!isUint(count, MAX_CHANNELS) || count < 1) {
      errors.push(`channels.count: ${String(count)}, expected an integer in [1, 65535]`);
    }
    if (typeof x.channels.multiChannelOrbits !== 'boolean') {
      errors.push('channels.multiChannelOrbits: not a boolean');
    }
  }

  const nValues = Array.isArray(x.values) ? x.values.length : 0;
  if (!Array.isArray(x.pauses)) {
    errors.push('pauses: not an array');
  } else if (x.pauses.length === 0) {
    errors.push('pauses: empty');
  } else {
    let prevSec = -Infinity;
    let prevFrame = -Infinity;
    x.pauses.forEach((p, j) => {
      const where = `pauses[${j}]`;
      if (!isRecord(p)) {
        errors.push(`${where}: not an object`);
        return;
      }
      errors.push(...fieldErrors(p, where, ['suspendSec', 'haps']));
      const sec = p.suspendSec;
      if (!isFiniteNumber(sec)) {
        errors.push(`${where}.suspendSec: ${String(sec)}, not finite`);
      } else {
        if (j === 0 && sec !== 0) errors.push(`${where}.suspendSec: ${sec}, expected 0`);
        const frame = pauseFrameOf(sec, 44100);
        if (j > 0 && !(sec > prevSec)) {
          errors.push(`${where}.suspendSec: ${sec}, not after ${prevSec}`);
        } else if (j > 0 && !(frame > prevFrame)) {
          errors.push(`${where}.suspendSec: frame ${frame}, not after frame ${prevFrame}`);
        }
        if (j > 0 && framesOk && frame >= (x.frames as number)) {
          errors.push(`${where}.suspendSec: frame ${frame}, not before frames ${x.frames}`);
        }
        prevSec = sec;
        prevFrame = frame;
      }
      if (!Array.isArray(p.haps)) {
        errors.push(`${where}.haps: not an array`);
        return;
      }
      p.haps.forEach((h, k) => errors.push(...hapErrors(h, `${where}.haps[${k}]`, nValues)));
    });
  }

  if (!Array.isArray(x.orbits)) {
    errors.push('orbits: not an array');
  } else {
    x.orbits.forEach((o, i) => {
      const where = `orbits[${i}]`;
      if (!isRecord(o)) {
        errors.push(`${where}: not an object`);
        return;
      }
      errors.push(...fieldErrors(o, where, ['orbit', 'channels']));
      if (!isFiniteNumber(o.orbit)) errors.push(`${where}.orbit: ${String(o.orbit)}, not finite`);
      if (!Array.isArray(o.channels) || o.channels.length === 0) {
        errors.push(`${where}.channels: expected a non-empty array`);
      } else if (!o.channels.every((c) => Number.isInteger(c) && (c as number) >= 0)) {
        errors.push(`${where}.channels: expected integers >= 0`);
      }
    });
  }

  if (!Array.isArray(x.values)) {
    errors.push('values: not an array');
  } else {
    x.values.forEach((v, i) => {
      if (!isRecord(v)) {
        errors.push(`values[${i}]: not an object`);
        return;
      }
      // A control map is not itself a wrapper (a control may be named `$num`); its members are.
      for (const [k, m] of Object.entries(v)) {
        const problem = wrapperProblem(m);
        if (problem !== null) errors.push(`values[${i}].${k}: ${problem}`);
      }
    });
  }

  return { job: errors.length === 0 ? (x as unknown as EngineJob) : null, errors };
}

function hapErrors(h: unknown, where: string, nValues: number): string[] {
  if (!isRecord(h)) return [`${where}: not an object`];
  const errors = fieldErrors(h, where, ['t', 'duration', 'cycle', 'v', 'event']);
  if (!isFiniteNumber(h.t) || h.t < 0) errors.push(`${where}.t: ${String(h.t)}, expected >= 0`);
  if (!isFiniteNumber(h.duration) || h.duration < 0) {
    errors.push(`${where}.duration: ${String(h.duration)}, expected >= 0`);
  }
  if (!isFiniteNumber(h.cycle)) errors.push(`${where}.cycle: ${String(h.cycle)}, not finite`);
  if (!isUint(h.v, U32_MAX) || h.v >= nValues) {
    errors.push(`${where}.v: ${String(h.v)}, not an index into values (${nValues})`);
  }
  if (!isUint(h.event, U32_MAX)) errors.push(`${where}.event: ${String(h.event)}, not a u32`);
  return errors;
}

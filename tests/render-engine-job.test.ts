import { describe, expect, test } from 'bun:test';
import invalidJobs from './helpers/invalid-jobs.json';
import orbitPairs from './helpers/orbit-pairs.json';
import {
  applyMutation,
  canonicalJson,
  decodeControlMap,
  decodeValue,
  encodeControlMap,
  encodeValue,
  ENGINE_JOB_SCHEMA,
  formatSignature,
  isUnsupportedMarker,
  planEngineJob,
  validateEngineJob,
  type EngineJob,
  type EngineValue,
} from '../src/lib/render/engine-job';
import {
  channelPlan,
  orbitChannels,
  pauseFrameOf,
  selectHaps,
  windowPlan,
} from '../src/lib/render/plan';
import { orbitPair } from '../src/lib/render/gates/stems';
import type { RenderRequest } from '../src/lib/render/contract';
import { hap, req } from './helpers/fake-offline';

const CPS = 0.5; // 2 s bars

function planFor(
  haps: any[],
  r: RenderRequest = req(),
  o: { scheduleHaps?: any[]; lookbackSec?: number; cps?: number; seed?: number } = {},
) {
  const cps = o.cps ?? CPS;
  const p = windowPlan(r, cps, o.lookbackSec ?? 0);
  return planEngineJob(
    selectHaps(haps).selected,
    selectHaps(o.scheduleHaps ?? haps).selected,
    p,
    r,
    cps,
    o.seed ?? 0,
  );
}

/** A small valid job, as the shared invalid-job table's base. */
const validJob = (): EngineJob => ({
  schema: 2,
  sampleRate: 44100,
  frames: 176400,
  seed: 0,
  cps: 0.5,
  channels: { count: 2, multiChannelOrbits: false },
  orbits: [{ orbit: 1, channels: [0, 1] }],
  pauses: [
    { suspendSec: 0, haps: [{ t: 0, duration: 1, cycle: 0, v: 0, event: 0 }] },
    { suspendSec: 1.95, haps: [{ t: 2, duration: 1, cycle: 1, v: 0, event: 1 }] },
  ],
  values: [{ s: 'sine', note: 60 }],
});

describe('planEngineJob', () => {
  test('header: schema, sample rate, frames, seed, cps', () => {
    const { job } = planFor([hap(0, 1, { s: 'sine' })], req({ toBar: 2 }), { seed: -5 });
    expect(job.schema).toBe(ENGINE_JOB_SCHEMA);
    expect(job.sampleRate).toBe(44100);
    expect(job.frames).toBe(2 * 2 * 44100);
    expect(job.seed).toBe(-5);
    expect(job.cps).toBe(CPS);
    expect(job.provenance).toBeUndefined();
    expect(validateEngineJob(job).errors).toEqual([]);
  });

  test('pause seconds and buckets: one pause per bar, haps in their bar', () => {
    const haps = [
      hap(0, 0.5, { s: 'a' }),
      hap(1.5, 2, { s: 'c' }),
      hap(1, 1.5, { s: 'b' }),
      hap(2, 3, { s: 'd' }),
    ];
    const { job } = planFor(haps, req({ toBar: 3 }));
    expect(job.pauses.map((p) => p.suspendSec)).toEqual([0, 1.95, 3.95]);
    const names = job.pauses.map((p) => p.haps.map((h) => job.values[h.v]!.s));
    expect(names).toEqual([['a'], ['b', 'c'], ['d']]);
  });

  test('call order is onset order, ties in query order; event is the selected index', () => {
    const haps = [hap(0.5, 1, { s: 'late' }), hap(0, 1, { s: 'x' }), hap(0, 1, { s: 'y' })];
    const { job, events } = planFor(haps, req({ toBar: 1 }));
    const order = job.pauses[0]!.haps.map((h) => job.values[h.v]!.s);
    expect(order).toEqual(['x', 'y', 'late']);
    expect(job.pauses[0]!.haps.map((h) => h.event)).toEqual([0, 1, 2]);
    job.pauses[0]!.haps.forEach((h) => expect(events[h.event]!.s).toBe(String(job.values[h.v]!.s)));
  });

  test('t is relative to c0, cycle is absolute, duration includes clip', () => {
    const r = req({ fromBar: 2, toBar: 3 });
    const haps = [hap(2.25, 2.75, { s: 'sine', clip: 2 })];
    const { job } = planFor(haps, r, { lookbackSec: 2 }); // one bar of lookback: c0 = 1
    const h = job.pauses[1]!.haps[0]!;
    expect(h.cycle).toBe(2.25);
    expect(h.t).toBe((2.25 - 1) / CPS);
    // whole 0.5 cycles, clip 2 -> hap.duration 1 cycle = 2 s
    expect(h.duration).toBe(1 / CPS);
  });

  test('t is not clamped (a hap before c0 never reaches a pause)', () => {
    const r = req({ fromBar: 1, toBar: 2 });
    const { job } = planFor([hap(0.5, 1.5, { s: 'early' }), hap(1, 2, { s: 'on' })], r);
    expect(job.pauses).toHaveLength(1);
    expect(job.pauses[0]!.haps.map((h) => job.values[h.v]!.s)).toEqual(['on']);
    expect(job.pauses[0]!.haps[0]!.t).toBe(0);
  });

  test('chunk vs full schedule: events and channels from the chunk, orbits from both', () => {
    const r = req({ fromBar: 1, toBar: 2, mode: 'stems' });
    const chunk = [hap(1, 2, { s: 'a', orbit: 2 })];
    const full = [hap(0, 1, { s: 'z', orbit: 5 }), ...chunk];
    const { job, events, stemPairs } = planFor(chunk, r, { scheduleHaps: full });
    expect(events.map((e) => e.s)).toEqual(['a']);
    // channel plan from the chunk only: orbit 2 -> 2 pairs (orbit 5 would make 5)
    expect(job.channels).toEqual({ count: 4, multiChannelOrbits: true });
    expect(stemPairs).toBe(2);
    expect(job.orbits).toEqual([
      { orbit: 5, channels: [8, 9] },
      { orbit: 2, channels: [2, 3] },
    ]);
  });

  test('orbit order by first onset; orbit values are numeric', () => {
    const haps = [
      hap(0.5, 1, { s: 'a', orbit: '3' }),
      hap(0, 1, { s: 'b' }),
      hap(0.75, 1, { s: 'c', orbit: 'nope' }),
    ];
    const { job } = planFor(haps, req({ toBar: 1 }));
    expect(job.orbits.map((o) => o.orbit)).toEqual([1, 3, 1]);
    expect(job.channels).toEqual({ count: 2, multiChannelOrbits: false });
  });

  test('values are interned regardless of key order; _ keys removed', () => {
    const haps = [
      hap(0, 0.5, { s: 'sine', note: 60, _id: 1 }),
      hap(0.5, 1, { note: 60, s: 'sine', _id: 2 }),
      hap(0.75, 1, { s: 'sine', note: 61 }),
    ];
    const { job } = planFor(haps, req({ toBar: 1 }));
    expect(job.values).toEqual([
      { s: 'sine', note: 60 },
      { s: 'sine', note: 61 },
    ]);
    expect(job.pauses[0]!.haps.map((h) => h.v)).toEqual([0, 0, 1]);
  });

  test('values are snapshots: later mutation of the hap value does not change the job', () => {
    const v = { s: 'sine', note: 60 };
    const { job } = planFor([hap(0, 1, v)], req({ toBar: 1 }));
    const selected = job.values[0]!;
    // superdough mutates values after planning
    (v as any).s = 'mutated';
    expect(selected.s).toBe('sine');
  });

  test('non-JSON control values are encoded', () => {
    const haps = [hap(0, 1, { s: 'sine', gain: NaN, pan: -0, fn: () => 1 })];
    const { job } = planFor(haps, req({ toBar: 1 }));
    expect(job.values[0]).toEqual({
      s: 'sine',
      gain: { $num: 'NaN' },
      pan: { $num: '-0' },
      fn: { $unsupported: 'function' },
    });
  });

  test('a zero-hap request: pauses per bar, no haps, no values, no orbits', () => {
    const { job, events } = planFor([], req({ toBar: 3 }));
    expect(job.pauses.map((p) => p.haps)).toEqual([[], [], []]);
    expect(job.values).toEqual([]);
    expect(job.orbits).toEqual([]);
    expect(events).toEqual([]);
    expect(validateEngineJob(job).errors).toEqual([]);
  });

  test('a bar too short for the pause lead throws, as renderOffline does', () => {
    const fast = 44100 / 256 / 4 + 1;
    const r = req({ toBar: 2 });
    const p = windowPlan(r, fast, 0);
    expect(() => planEngineJob([], [], p, r, fast, 0)).toThrow('bar too short for the pause lead');
  });

  test('the planned job survives JSON and validates', () => {
    const haps = [hap(0, 1, { s: 'sine', note: 'c3', orbit: 2 }), hap(1, 2, { s: 'saw' })];
    const { job } = planFor(haps, req({ toBar: 2, mode: 'stems' }));
    const back = JSON.parse(JSON.stringify(job));
    expect(back).toEqual(job);
    expect(validateEngineJob(back)).toEqual({ job: back, errors: [] });
  });
});

describe('applyMutation', () => {
  const haps = [
    hap(0, 0.5, { s: 'bass', note: 30, orbit: 2 }),
    hap(0, 0.5, { s: 'lead', note: 80, orbit: 1 }),
    hap(1, 1.5, { s: 'bass', note: 30, orbit: 2 }),
    hap(1.5, 2, { s: 'lead', note: 81, orbit: 1 }),
  ];
  const planned = planFor(haps, req({ toBar: 2, mode: 'stems' }));
  const names = (j: EngineJob) => j.pauses.map((p) => p.haps.map((h) => j.values[h.v]!.s));

  test('dropOrbit drops the orbit in the listed bars only; orbits and channels kept', () => {
    const m = applyMutation(planned.job, planned.events, {
      kind: 'dropOrbit',
      orbit: 2,
      bars: [1],
    });
    expect(names(m)).toEqual([['bass', 'lead'], ['lead']]);
    expect(m.orbits).toEqual(planned.job.orbits);
    expect(m.channels).toEqual(planned.job.channels);
    expect(m.values).toEqual(planned.job.values);
    expect(m.pauses.map((p) => p.suspendSec)).toEqual(planned.job.pauses.map((p) => p.suspendSec));
  });

  test('dropNotesBelowHz with and without bars', () => {
    const every = applyMutation(planned.job, planned.events, { kind: 'dropNotesBelowHz', hz: 100 });
    expect(names(every)).toEqual([['lead'], ['lead']]);
    const bar0 = applyMutation(planned.job, planned.events, {
      kind: 'dropNotesBelowHz',
      hz: 100,
      bars: [0],
    });
    expect(names(bar0)).toEqual([['lead'], ['bass', 'lead']]);
  });

  test('order is kept and the input is not changed', () => {
    const before = JSON.stringify(planned.job);
    const m = applyMutation(planned.job, planned.events, {
      kind: 'dropOrbit',
      orbit: 1,
      bars: [0, 1],
    });
    expect(m.pauses.map((p) => p.haps.map((h) => h.event))).toEqual([[0], [2]]);
    expect(JSON.stringify(planned.job)).toBe(before);
  });

  test('an unknown target is a no-op', () => {
    const m = applyMutation(planned.job, planned.events, {
      kind: 'dropOrbit',
      orbit: 9,
      bars: [0],
    });
    expect(m).toEqual(planned.job);
  });

  test('a hap whose event is missing is kept', () => {
    const m = applyMutation(planned.job, [], { kind: 'dropNotesBelowHz', hz: 1e9 });
    expect(m).toEqual(planned.job);
  });
});

describe('encodeValue / decodeValue', () => {
  const roundTrip = (x: unknown) => decodeValue(JSON.parse(JSON.stringify(encodeValue(x))));

  test('non-finite numbers and -0', () => {
    expect(encodeValue(NaN)).toEqual({ $num: 'NaN' });
    expect(encodeValue(Infinity)).toEqual({ $num: 'Infinity' });
    expect(encodeValue(-Infinity)).toEqual({ $num: '-Infinity' });
    expect(encodeValue(-0)).toEqual({ $num: '-0' });
    expect(encodeValue(0)).toBe(0);
    expect(Object.is(roundTrip(-0), -0)).toBe(true);
    expect(Object.is(roundTrip(0), 0)).toBe(true);
    expect(roundTrip(NaN)).toBeNaN();
    expect(roundTrip(Infinity)).toBe(Infinity);
    expect(roundTrip(-Infinity)).toBe(-Infinity);
    expect(roundTrip(1.5e-300)).toBe(1.5e-300);
  });

  test('JSON values pass through and round trip, nested', () => {
    const x = { a: [1, 'two', true, null, { b: [-0, NaN] }], c: { d: { e: -Infinity } }, s: '' };
    const back = roundTrip(x) as any;
    expect(back.a.slice(0, 4)).toEqual([1, 'two', true, null]);
    expect(Object.is(back.a[4].b[0], -0)).toBe(true);
    expect(back.a[4].b[1]).toBeNaN();
    expect(back.c.d.e).toBe(-Infinity);
    expect(back.s).toBe('');
    expect(encodeValue('x')).toBe('x');
    expect(encodeValue(false)).toBe(false);
    expect(encodeValue(null)).toBe(null);
  });

  test('undefined: omitted from objects, null in arrays, null alone', () => {
    expect(encodeValue({ a: undefined, b: 1 })).toEqual({ b: 1 });
    expect(encodeValue([undefined, 1])).toEqual([null, 1]);
    // eslint-disable-next-line no-sparse-arrays
    expect(encodeValue([1, , 3])).toEqual([1, null, 3]);
    expect(encodeValue(undefined)).toBe(null);
  });

  test('non-JSON members become $unsupported with the type or constructor name', () => {
    class Thing {}
    expect(encodeValue(() => 1)).toEqual({ $unsupported: 'function' });
    expect(encodeValue(Symbol('s'))).toEqual({ $unsupported: 'symbol' });
    expect(encodeValue(10n)).toEqual({ $unsupported: 'bigint' });
    expect(encodeValue(new Thing())).toEqual({ $unsupported: 'Thing' });
    expect(encodeValue(new Map())).toEqual({ $unsupported: 'Map' });
    expect(encodeValue(new Date(0))).toEqual({ $unsupported: 'Date' });
    expect(encodeValue({ f: { g: () => 1 } })).toEqual({ f: { g: { $unsupported: 'function' } } });
    const anon = Object.create({});
    expect(encodeValue(anon)).toEqual({ $unsupported: 'Object' });
    const noCtor = Object.create(Object.create(null));
    expect(encodeValue(noCtor)).toEqual({ $unsupported: 'object' });
  });

  test('null-prototype objects are plain', () => {
    const o = Object.create(null);
    o.a = 1;
    expect(encodeValue(o)).toEqual({ a: 1 });
  });

  test('cycles become $unsupported, shared (acyclic) references do not', () => {
    const a: any = { x: 1 };
    a.self = a;
    expect(encodeValue(a)).toEqual({ x: 1, self: { $unsupported: 'cycle' } });
    const shared = { k: 1 };
    expect(encodeValue([shared, shared])).toEqual([{ k: 1 }, { k: 1 }]);
    const arr: any[] = [1];
    arr.push(arr);
    expect(encodeValue(arr)).toEqual([1, { $unsupported: 'cycle' }]);
  });

  test('reserved keys in user objects', () => {
    expect(encodeValue({ $num: 'NaN' })).toEqual({ $unsupported: 'reserved key' });
    expect(encodeValue({ $unsupported: 'x', y: 1 })).toEqual({ $unsupported: 'reserved key' });
    expect(encodeValue({ nested: { $num: 1 } })).toEqual({
      nested: { $unsupported: 'reserved key' },
    });
  });

  test('$unsupported markers decode as themselves', () => {
    const d = decodeValue({ $unsupported: 'function' });
    expect(d).toEqual({ $unsupported: 'function' });
    expect(isUnsupportedMarker(d)).toBe(true);
    expect(isUnsupportedMarker({ $unsupported: 1 })).toBe(false);
    expect(isUnsupportedMarker({ $unsupported: 'x', y: 1 })).toBe(false);
    expect(isUnsupportedMarker([{ $unsupported: 'x' }])).toBe(false);
    expect(isUnsupportedMarker(null)).toBe(false);
  });

  test('malformed wrappers throw', () => {
    const bad: EngineValue[] = [
      { $num: 'nan' },
      { $num: 1 },
      { $num: 'NaN', x: 1 },
      { $unsupported: 1 },
      { $unsupported: 'f', y: 2 },
      [{ a: { $num: 'inf' } }],
    ];
    for (const b of bad) expect(() => decodeValue(b)).toThrow('invalid value wrapper');
  });

  test('a Set becomes a $set wrapper of its encoded members, in insertion order', () => {
    expect(encodeValue(new Set())).toEqual({ $set: [] });
    expect(encodeValue(new Set([2, 0, 'a']))).toEqual({ $set: [2, 0, 'a'] });
    expect(encodeValue(new Set([new Set([1]), new Set()]))).toEqual({
      $set: [{ $set: [1] }, { $set: [] }],
    });
    expect(encodeValue(new Set([NaN, undefined]))).toEqual({ $set: [{ $num: 'NaN' }, null] });
    expect(JSON.stringify(encodeValue(new Set([NaN, undefined])))).toBe(
      '{"$set":[{"$num":"NaN"},null]}',
    );
    expect(encodeValue([new Set([3])])).toEqual([{ $set: [3] }]);
    expect(encodeValue({ lfo: { __ids: new Set([0]), 0: { control: 'gain' } } })).toEqual({
      lfo: { __ids: { $set: [0] }, 0: { control: 'gain' } },
    });
    class Ids extends Set<number> {}
    expect(encodeValue(new Ids([1]))).toEqual({ $set: [1] });
    expect(encodeValue(new Set([() => 1]))).toEqual({ $set: [{ $unsupported: 'function' }] });
  });

  test('a cyclic Set, a $set key in a user object, and a Map', () => {
    const cyclic = new Set<unknown>([1]);
    cyclic.add(cyclic);
    expect(encodeValue(cyclic)).toEqual({ $set: [1, { $unsupported: 'cycle' }] });
    const viaObject: any = { a: 1 };
    viaObject.ids = new Set([viaObject]);
    expect(encodeValue(viaObject)).toEqual({ a: 1, ids: { $set: [{ $unsupported: 'cycle' }] } });
    expect(encodeValue({ $set: [] })).toEqual({ $unsupported: 'reserved key' });
    expect(encodeValue({ x: { $set: [1], y: 2 } })).toEqual({
      x: { $unsupported: 'reserved key' },
    });
    expect(encodeValue(new Map([[1, 2]]))).toEqual({ $unsupported: 'Map' });
  });

  test('a $set wrapper decodes to a Set; the round trip keeps insertion order', () => {
    const original = new Set<unknown>([3, 'b', 1, new Set([NaN]), [1, -0]]);
    const back = roundTrip(original) as Set<unknown>;
    expect(back).toBeInstanceOf(Set);
    const members = [...back];
    expect(members.slice(0, 3)).toEqual([3, 'b', 1]);
    expect(members[3]).toBeInstanceOf(Set);
    expect([...(members[3] as Set<number>)][0]).toBeNaN();
    expect(Object.is((members[4] as number[])[1], -0)).toBe(true);
    expect([...(roundTrip(new Set([1, 0])) as Set<number>)]).toEqual([1, 0]);
    expect(decodeValue({ $set: [] })).toEqual(new Set());
    expect(decodeValue({ $set: ['bad'] })).toEqual(new Set(['bad']));
  });

  test('malformed $set wrappers throw', () => {
    const bad: EngineValue[] = [
      { $set: 1 },
      { $set: {} },
      { $set: [], x: 1 },
      { $set: [{ $num: 'x' }] },
      [{ $set: [{ $set: 'no' }] }],
    ];
    for (const b of bad) expect(() => decodeValue(b)).toThrow('invalid value wrapper');
    expect(() => decodeValue({ $set: 1 })).toThrow('invalid value wrapper: bad $set');
    expect(() => decodeValue({ $set: [], x: 1 })).toThrow(
      'invalid value wrapper: $set with other keys',
    );
    // Check order: $num, $unsupported, $set (as in the Rust job.rs).
    expect(() => decodeValue({ $num: 'NaN', $set: [] })).toThrow('$num with other keys');
    expect(() => decodeValue({ $unsupported: 'x', $set: [] })).toThrow(
      '$unsupported with other keys',
    );
  });

  test('control maps: _ keys dropped at the top only, members encoded, $num key allowed', () => {
    const m = encodeControlMap({ s: 'x', _hidden: 1, n: { _keep: 2 }, g: NaN, u: undefined });
    expect(m).toEqual({ s: 'x', n: { _keep: 2 }, g: { $num: 'NaN' } });
    const named = encodeControlMap({ $num: 3 });
    expect(named).toEqual({ $num: 3 });
    expect(decodeControlMap(named)).toEqual({ $num: 3 });
    expect(decodeControlMap({ g: { $num: 'NaN' } }).g).toBeNaN();
    const self: any = { s: 'x' };
    self.me = self;
    expect(encodeControlMap(self)).toEqual({ s: 'x', me: { $unsupported: 'cycle' } });
  });
});

describe('canonicalJson', () => {
  test('key-order independent, recursively; arrays keep order', () => {
    const a = canonicalJson({ b: 1, a: { d: [2, { y: 1, x: 2 }], c: 3 } });
    const b = canonicalJson({ a: { c: 3, d: [2, { x: 2, y: 1 }] }, b: 1 });
    expect(a).toBe(b);
    expect(a).toBe('{"a":{"c":3,"d":[2,{"x":2,"y":1}]},"b":1}');
    expect(canonicalJson([2, 1])).not.toBe(canonicalJson([1, 2]));
  });

  test('$set members keep their order (the modulator connection order)', () => {
    const a = canonicalJson(encodeValue({ __ids: new Set([0, 1]) }));
    const b = canonicalJson(encodeValue({ __ids: new Set([1, 0]) }));
    expect(a).toBe('{"__ids":{"$set":[0,1]}}');
    expect(b).toBe('{"__ids":{"$set":[1,0]}}');
    expect(a).not.toBe(b);
  });

  test('distinct maps stay distinct', () => {
    expect(canonicalJson({ a: 1 })).not.toBe(canonicalJson({ a: '1' }));
    expect(canonicalJson({ a: { $num: 'NaN' } })).not.toBe(canonicalJson({ a: null }));
    expect(canonicalJson(null)).toBe('null');
    expect(canonicalJson('x')).toBe('"x"');
  });
});

describe('validateEngineJob', () => {
  test('a valid job is accepted, returned as is', () => {
    const j = validJob();
    expect(validateEngineJob(j)).toEqual({ job: j, errors: [] });
    expect(validateEngineJob({ ...j, provenance: { anything: true } }).errors).toEqual([]);
  });

  test('edges that are valid', () => {
    const j = validJob();
    j.frames = 2 ** 31;
    j.seed = -(2 ** 31);
    j.pauses[0]!.suspendSec = -0;
    j.pauses[0]!.haps[0]!.duration = 0;
    j.values = [{ s: 'sine', g: { $num: '-0' }, f: { $unsupported: 'function' }, $num: 1 }];
    j.orbits = [{ orbit: -1.5, channels: [0] }];
    expect(validateEngineJob(j).errors).toEqual([]);
  });

  test('a value holding a $set is valid', () => {
    const j = validJob();
    j.values = [{ s: 'sine', lfo: { __ids: { $set: [0] }, 0: { control: 'gain' } } }];
    expect(validateEngineJob(j).errors).toEqual([]);
    j.values = [{ s: 'sine', x: [{ $set: [{ $set: [] }, { $num: 'NaN' }] }] }];
    expect(validateEngineJob(j).errors).toEqual([]);
    j.values = [{ s: 'sine', x: { $set: 1 } }];
    expect(validateEngineJob(j).errors).toEqual(['values[0].x: bad $set']);
  });

  test('schema 1 is refused with a regenerate message; other schemas name the expected one', () => {
    const j: any = validJob();
    j.schema = 1;
    expect(validateEngineJob(j).errors).toEqual(['schema 1 job: regenerate it']);
    j.schema = 3;
    expect(validateEngineJob(j).errors).toEqual(['schema: 3, expected 2']);
    j.schema = '2';
    expect(validateEngineJob(j).errors).toEqual(['schema: 2, expected 2']);
    delete j.schema;
    expect(validateEngineJob(j).errors).toContain('schema: undefined, expected 2');
  });

  test('every entry of the shared invalid-job table is rejected', () => {
    expect(invalidJobs.length).toBeGreaterThan(30);
    const names = new Set<string>();
    for (const entry of invalidJobs as { name: string; rule: string; job: unknown }[]) {
      expect(names.has(entry.name)).toBe(false);
      names.add(entry.name);
      expect(typeof entry.rule).toBe('string');
      const r = validateEngineJob(entry.job);
      if (r.job !== null || r.errors.length === 0) throw new Error(`accepted: ${entry.name}`);
    }
  });

  test('the table differs from the valid base in one place per entry', () => {
    // Every non-shape entry, with its one change undone, is the valid base: so each entry tests
    // its own rule and nothing else.
    for (const entry of invalidJobs as { name: string; rule: string; job: any }[]) {
      if (entry.rule === 'shape') continue;
      const r = validateEngineJob(entry.job);
      expect({ name: entry.name, n: r.errors.length }).toEqual({ name: entry.name, n: 1 });
    }
  });

  test('collects every violation', () => {
    const j: any = validJob();
    j.schema = 1;
    j.cps = 0;
    j.pauses[0].haps[0].v = 5;
    j.orbits[0].channels = [];
    const r = validateEngineJob(j);
    expect(r.job).toBeNull();
    expect(r.errors).toHaveLength(4);
    expect(r.errors[0]).toBe('schema 1 job: regenerate it');
  });

  test('non-objects and wrong containers', () => {
    expect(validateEngineJob(null).errors).toEqual(['job: not an object']);
    expect(validateEngineJob('x').job).toBeNull();
    const j: any = validJob();
    j.pauses = {};
    j.orbits = null;
    j.values = 'v';
    const errors = validateEngineJob(j).errors;
    expect(errors).toContain('pauses: not an array');
    expect(errors).toContain('orbits: not an array');
    expect(errors).toContain('values: not an array');
  });

  test('the quantized pause frame rules', () => {
    const j: any = validJob();
    // 1.95 s -> trunc 85995 -> 86016; 1.951 s -> 86039 -> 86144 is fine
    j.pauses.push({ suspendSec: 1.951, haps: [] });
    expect(validateEngineJob(j).errors).toEqual([]);
    j.pauses[2].suspendSec = 1.9501; // same quantum as 1.95
    expect(validateEngineJob(j).errors.join()).toContain(
      `not after frame ${pauseFrameOf(1.95, 44100)}`,
    );
  });
});

describe('formatSignature', () => {
  test('formatSignature', () => {
    expect(formatSignature({ s: 'sine', keys: ['shape', 'vowel'], firstBar: 3 })).toBe(
      's=sine keys=shape,vowel bar=3',
    );
    expect(formatSignature({ s: 'supersaw', keys: [], firstBar: 0 })).toBe(
      's=supersaw keys= bar=0',
    );
  });
});

describe('orbit pairs (table shared with the Rust engine)', () => {
  test('orbitPair matches the table', () => {
    expect(orbitPairs.length).toBeGreaterThanOrEqual(10);
    for (const { orbit, pairs, pair } of orbitPairs) {
      expect([orbit, pairs, orbitPair(orbit, pairs)]).toEqual([orbit, pairs, pair]);
    }
  });

  test("an orbit's stems channels land on its pair (superdough's % channelCount)", () => {
    for (const { orbit, pairs, pair } of orbitPairs) {
      const channels = orbitChannels({ orbit }, true).map((c) => c % (2 * pairs));
      expect([orbit, pairs, channels]).toEqual([orbit, pairs, [2 * pair, 2 * pair + 1]]);
    }
  });

  test('channelPlan gives the table pairs for a single orbit', () => {
    let checked = 0;
    for (const { orbit, pairs, pair } of orbitPairs) {
      const plan = channelPlan([{ orbit }], 'stems');
      if (plan.pairs !== pairs) continue;
      checked++;
      expect(plan.channels).toBe(2 * pairs);
      expect(orbitPair(orbit, plan.pairs)).toBe(pair);
    }
    expect(checked).toBeGreaterThanOrEqual(5);
  });
});

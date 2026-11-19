/**
 * An in-memory `RecordingApi`: every call is logged in `fake.calls` (name and arguments), results
 * are scripted through the fields of `fake`, a method can be made to reject once (`failNext`) or
 * to wait (`hold`), and `emitState`/`emitLevel` play the Rust events. The memo library (`list`,
 * `rename`, `delete`, `read`) works on `fake.memos`, one list per project root; an unknown root is
 * `projectChanged`.
 */
import type { RecordingApi } from '../../src/lib/recording/api';
import {
  RecordingError,
  type DevicesResult,
  type InputDevice,
  type LevelEvent,
  type RecordingEntry,
  type RecordingStatus,
  type StateEvent,
  type StopFields,
} from '../../src/lib/recording/types';

type Method = Exclude<keyof RecordingApi, 'onLevel' | 'onState'>;

export const MIC: InputDevice = {
  id: 'mic-1',
  name: 'MacBook Pro Microphone',
  channels: 1,
  sampleRate: 48000,
  isDefault: true,
  bluetooth: false,
};
export const INTERFACE: InputDevice = {
  id: 'usb-2',
  name: 'USB Interface',
  channels: 2,
  sampleRate: 44100,
  isDefault: false,
  bluetooth: false,
};

/** A memo entry with a WAV of `frames` frames and no sidecar unless given. */
export function entry(stem: string, extra: Partial<RecordingEntry> = {}): RecordingEntry {
  return {
    stem,
    frames: 48000,
    sampleRate: 48000,
    duration: 1,
    hasSidecar: false,
    sidecar: null,
    sidecarError: null,
    size: 192044,
    modifiedMs: 1_700_000_000_000,
    ...extra,
  };
}

const STEM_RE = /^[A-Za-z0-9 _-]{1,64}$/;

export function fakeRecordingApi(
  opts: {
    devices?: InputDevice[];
    roots?: string[];
    permission?: RecordingStatus['permission'];
  } = {},
) {
  const devices = opts.devices ?? [MIC];
  const stateHandlers = new Set<(e: StateEvent) => void>();
  const levelHandlers = new Set<(e: LevelEvent) => void>();
  const fake = {
    /** Every call but `hostNowNs`, which only counts in `pings`. */
    calls: [] as { method: string; args: unknown[] }[],
    pings: 0,
    /** What `status()` returns. */
    status: {
      state: 'idle',
      permission: opts.permission ?? 'notDetermined',
      device: null,
      channel: 0,
      stem: null,
      elapsedSeconds: null,
    } as RecordingStatus,
    /** What `devices()` returns. */
    devices: {
      devices,
      selected: devices[0] ? { deviceId: devices[0].id, channel: 0 } : null,
    } as DevicesResult,
    /** The permission a successful arm reports. */
    armPermission: 'authorized' as RecordingStatus['permission'],
    /** The stem the next `start` resolves to; default `<songStem or memo>-01`. */
    nextStem: null as string | null,
    /** `hostNowNs()`; default: `hostNs` advanced by `hostStepNs` per call. */
    hostNs: 1_000_000_000_000n,
    hostStepNs: 1_000_000n,
    hostNowNs: null as (() => Promise<bigint>) | null,
    /** The sidecar of every `stop` call, in order. */
    stops: [] as (StopFields | null)[],
    /** One list per project root. */
    memos: new Map<string, RecordingEntry[]>((opts.roots ?? ['/p']).map((r) => [r, []])),
    bytes: new Map<string, Uint8Array>(),
    failures: new Map<Method, RecordingError[]>(),
    holds: new Map<Method, Promise<void>>(),
    /** Make the next call of `method` reject with `error` (queued, one per call). */
    failNext(method: Method, error: RecordingError) {
      const list = fake.failures.get(method) ?? [];
      list.push(error);
      fake.failures.set(method, list);
    },
    /** Calls of `method` wait for `p` before answering (until set to null). */
    hold(method: Method, p: Promise<void> | null) {
      if (p) fake.holds.set(method, p);
      else fake.holds.delete(method);
    },
    emitState(e: StateEvent) {
      for (const h of [...stateHandlers]) h(e);
    },
    emitLevel(e: LevelEvent) {
      for (const h of [...levelHandlers]) h(e);
    },
    listeners: () => ({ state: stateHandlers.size, level: levelHandlers.size }),
    /** Method names called, in order. */
    names: () => fake.calls.map((c) => c.method),
    argsOf: (method: string) => fake.calls.filter((c) => c.method === method).map((c) => c.args),
  };

  async function enter(method: Method, args: unknown[]): Promise<void> {
    if (method === 'hostNowNs') fake.pings++;
    else fake.calls.push({ method, args });
    const hold = fake.holds.get(method);
    if (hold) await hold;
    const failure = fake.failures.get(method)?.shift();
    if (failure) throw failure;
  }

  const memosOf = (root: string): RecordingEntry[] => {
    const list = fake.memos.get(root);
    if (!list) throw new RecordingError('the project changed', 'projectChanged');
    return list;
  };
  const find = (root: string, stem: string): RecordingEntry => {
    const e = memosOf(root).find((m) => m.stem === stem);
    if (!e) throw new RecordingError(`recording ${stem} not found`, 'notFound');
    return e;
  };
  let take: { root: string; stem: string } | null = null;

  const api: RecordingApi = {
    async status() {
      await enter('status', []);
      return { ...fake.status };
    },
    async devices() {
      await enter('devices', []);
      return { devices: [...fake.devices.devices], selected: fake.devices.selected };
    },
    async arm(o) {
      await enter('arm', o === undefined ? [] : [o]);
      const id = o?.deviceId ?? fake.devices.selected?.deviceId;
      const device = fake.devices.devices.find((d) => d.id === id) ?? null;
      if (!device) throw new RecordingError('no input device found', 'noDevice');
      const channel = o?.channel ?? fake.devices.selected?.channel ?? 0;
      fake.devices.selected = { deviceId: device.id, channel };
      fake.status = {
        ...fake.status,
        state: 'armed',
        permission: fake.armPermission,
        device,
        channel,
      };
      return { ...fake.status };
    },
    async disarm() {
      await enter('disarm', []);
      fake.status = { ...fake.status, state: 'idle', device: null };
      return { ...fake.status };
    },
    async start(o) {
      await enter('start', [o]);
      const stem = fake.nextStem ?? `${o.songStem ?? 'memo'}-01`;
      fake.nextStem = null;
      take = { root: o.root, stem };
      fake.status = { ...fake.status, state: 'recording', stem };
      return { stem };
    },
    async stop(o) {
      await enter('stop', [o]);
      fake.stops.push(o.sidecar);
      const t = take ?? { root: '/p', stem: fake.status.stem ?? 'memo-01' };
      take = null;
      fake.status = { ...fake.status, state: 'armed', stem: null };
      const e = entry(t.stem, { hasSidecar: true });
      const list = fake.memos.get(t.root);
      if (list && !list.some((m) => m.stem === t.stem)) list.push(e);
      return e;
    },
    async list(root) {
      await enter('list', [root]);
      return memosOf(root).map((m) => ({ ...m }));
    },
    async rename(root, stem, newStem) {
      await enter('rename', [root, stem, newStem]);
      const e = find(root, stem);
      if (!STEM_RE.test(newStem) || newStem !== newStem.trim()) {
        throw new RecordingError(
          `invalid recording name ${JSON.stringify(newStem)}`,
          'invalidName',
        );
      }
      if (memosOf(root).some((m) => m.stem === newStem)) {
        throw new RecordingError(`recording ${newStem} already exists`, 'exists');
      }
      e.stem = newStem;
      const bytes = fake.bytes.get(`${root}:${stem}`);
      if (bytes) {
        fake.bytes.delete(`${root}:${stem}`);
        fake.bytes.set(`${root}:${newStem}`, bytes);
      }
      return newStem;
    },
    async delete(root, stem) {
      await enter('delete', [root, stem]);
      find(root, stem);
      fake.memos.set(
        root,
        memosOf(root).filter((m) => m.stem !== stem),
      );
      fake.bytes.delete(`${root}:${stem}`);
    },
    async read(root, stem) {
      await enter('read', [root, stem]);
      find(root, stem);
      return fake.bytes.get(`${root}:${stem}`) ?? new Uint8Array();
    },
    async hostNowNs() {
      await enter('hostNowNs', []);
      if (fake.hostNowNs) return fake.hostNowNs();
      const t = fake.hostNs;
      fake.hostNs += fake.hostStepNs;
      return t;
    },
    async onLevel(cb) {
      levelHandlers.add(cb);
      return () => void levelHandlers.delete(cb);
    },
    async onState(cb) {
      stateHandlers.add(cb);
      return () => void stateHandlers.delete(cb);
    },
  };
  return { api, fake };
}

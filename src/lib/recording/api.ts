import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { kindedError } from '../command-error';
import { errorMessage } from '../errors';
import { hostNsOrNull } from '../host-ns';
import {
  isRecordingErrorKind,
  RecordingError,
  type DevicesResult,
  type LevelEvent,
  type RecordingEntry,
  type RecordingStatus,
  type StartFields,
  type StateEvent,
  type StopFields,
} from './types';

// The OS seam of recording: the only module that calls Tauri for it. The controller and the memo
// list only see `RecordingApi`, so they run under `bun test` with the fake in
// `tests/helpers/fake-recording-api.ts`.

export const LEVEL_EVENT = 'recording://level';
export const STATE_EVENT = 'recording://state';

export interface RecordingApi {
  status(): Promise<RecordingStatus>;
  devices(): Promise<DevicesResult>;
  /** Opens the stream (asking for microphone access first); re-arms with a new device or channel. */
  arm(opts?: { deviceId?: string; channel?: number }): Promise<RecordingStatus>;
  disarm(): Promise<RecordingStatus>;
  /** Resolves once the capture runs (its first anchor exists). The stem is Rust's default name. */
  start(opts: { root: string; songStem?: string; fields: StartFields }): Promise<{ stem: string }>;
  /**
   * Finishes the take into the project it started in (no root). `null` writes Rust's fallback
   * sidecar. Also succeeds for a take Rust already finalized (device lost, length cap).
   */
  stop(opts: { sidecar: StopFields | null }): Promise<RecordingEntry>;
  list(root: string): Promise<RecordingEntry[]>;
  /** Resolves to the new stem. */
  rename(root: string, stem: string, newStem: string): Promise<string>;
  delete(root: string, stem: string): Promise<void>;
  /** The WAV's bytes. */
  read(root: string, stem: string): Promise<Uint8Array>;
  /** The Mach host time in nanoseconds. */
  hostNowNs(): Promise<bigint>;
  /** Returns an unlisten function. */
  onLevel(cb: (e: LevelEvent) => void): Promise<() => void>;
  /** Returns an unlisten function. */
  onState(cb: (e: StateEvent) => void): Promise<() => void>;
}

/**
 * A rejection of a recording command to a `RecordingError`: the Rust `{ kind, message }` object,
 * else whatever was thrown as text with kind `other` (also for an unknown kind).
 */
export function toRecordingError(e: unknown): RecordingError {
  if (e instanceof RecordingError) return e;
  const k = kindedError(e);
  if (k) return new RecordingError(k.message, isRecordingErrorKind(k.kind) ? k.kind : 'other');
  return new RecordingError(errorMessage(e), 'other');
}

/** A non-negative decimal nanosecond string to a bigint; anything else throws `other`. */
export function parseHostNs(v: unknown): bigint {
  const ns = hostNsOrNull(v);
  if (ns === null) throw new RecordingError(`invalid host time ${JSON.stringify(v)}`, 'other');
  return ns;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toRecordingError(e);
  }
}

/** The Mach host time in nanoseconds (`host_now_ns`): the recording clock, also the live
 *  playback API's (`playback-api.ts` takes it from here rather than calling Tauri itself). */
export async function hostNowNs(): Promise<bigint> {
  return parseHostNs(await call<unknown>('host_now_ns'));
}

/** `args` without its undefined values (an absent optional argument, not a `null` one). */
const defined = (args: Record<string, unknown>): Record<string, unknown> =>
  Object.fromEntries(Object.entries(args).filter(([, v]) => v !== undefined));

/** The real implementation (invoke and listen). Argument keys are camelCase, as Tauri maps them. */
export function createTauriRecordingApi(): RecordingApi {
  return {
    status: () => call<RecordingStatus>('recording_status'),
    devices: () => call<DevicesResult>('recording_devices'),
    arm: (opts = {}) =>
      call<RecordingStatus>(
        'recording_arm',
        defined({ deviceId: opts.deviceId, channel: opts.channel }),
      ),
    disarm: () => call<RecordingStatus>('recording_disarm'),
    start: ({ root, songStem, fields }) =>
      call<{ stem: string }>('recording_start', defined({ root, songStem, fields })),
    stop: ({ sidecar }) => call<RecordingEntry>('recording_stop', { sidecar }),
    list: (root) => call<RecordingEntry[]>('recording_list', { root }),
    rename: (root, stem, newStem) => call<string>('recording_rename', { root, stem, newStem }),
    delete: (root, stem) => call<void>('recording_delete', { root, stem }),
    read: async (root, stem) =>
      new Uint8Array(await call<ArrayBuffer>('recording_read', { root, stem })),
    hostNowNs,
    onLevel: (cb) => listen<LevelEvent>(LEVEL_EVENT, (event) => cb(event.payload)),
    onState: (cb) => listen<StateEvent>(STATE_EVENT, (event) => cb(event.payload)),
  };
}

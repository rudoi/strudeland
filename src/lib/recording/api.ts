import { listen } from '@tauri-apps/api/event';
import { narrowKind, toCommandError } from '../ipc/command-error';
import { EVENTS, type CommandName } from '../ipc/bindings';
import { invokeCommand, type ArgsOf } from '../ipc/invoke';
import { hostNsOrNull } from '../host-ns';
import {
  RECORDING_ERROR_KINDS,
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
  const c = toCommandError(e);
  return new RecordingError(c.message, narrowKind(c.kind, RECORDING_ERROR_KINDS, 'other'), {
    cause: c.cause,
  });
}

/** `args` without its undefined values (an absent optional argument, not a `null` one). */
const defined = (args: Record<string, unknown>): Record<string, unknown> =>
  Object.fromEntries(Object.entries(args).filter(([, v]) => v !== undefined));

/** A non-negative decimal nanosecond string to a bigint; anything else throws `other`. */
export function parseHostNs(v: unknown): bigint {
  const ns = hostNsOrNull(v);
  if (ns === null) throw new RecordingError(`invalid host time ${JSON.stringify(v)}`, 'other');
  return ns;
}

async function call<N extends CommandName>(name: N, ...args: ArgsOf<N>): Promise<unknown> {
  const [a] = args;
  try {
    return await invokeCommand(name, ...((a === undefined ? [] : [defined(a)]) as ArgsOf<N>));
  } catch (e) {
    throw toRecordingError(e);
  }
}

/** The Mach host time in nanoseconds (`host_now_ns`): the recording clock, also the live
 *  playback API's (`playback-api.ts` takes it from here rather than calling Tauri itself). */
export async function hostNowNs(): Promise<bigint> {
  return parseHostNs(await call('host_now_ns'));
}

/** The real implementation (invoke and listen). Argument keys are camelCase, as Tauri maps them. */
export function createTauriRecordingApi(): RecordingApi {
  return {
    status: () => call('recording_status') as Promise<RecordingStatus>,
    devices: () => call('recording_devices') as Promise<DevicesResult>,
    arm: (opts = {}) =>
      call('recording_arm', {
        deviceId: opts.deviceId,
        channel: opts.channel,
      }) as Promise<RecordingStatus>,
    disarm: () => call('recording_disarm') as Promise<RecordingStatus>,
    start: ({ root, songStem, fields }) =>
      call('recording_start', { root, songStem, fields }) as Promise<{ stem: string }>,
    stop: ({ sidecar }) => call('recording_stop', { sidecar }) as Promise<RecordingEntry>,
    list: (root) => call('recording_list', { root }) as Promise<RecordingEntry[]>,
    rename: (root, stem, newStem) =>
      call('recording_rename', { root, stem, newStem }) as Promise<string>,
    delete: (root, stem) => call('recording_delete', { root, stem }) as Promise<void>,
    read: async (root, stem) =>
      new Uint8Array((await call('recording_read', { root, stem })) as ArrayBuffer),
    hostNowNs,
    onLevel: (cb) => listen<LevelEvent>(EVENTS.recordingLevel, (event) => cb(event.payload)),
    onState: (cb) =>
      listen<StateEvent>(EVENTS.recordingState, ({ payload }) => {
        const { error } = payload;
        cb(
          error
            ? {
                ...payload,
                error: {
                  ...error,
                  kind: narrowKind(String(error.kind), RECORDING_ERROR_KINDS, 'other'),
                },
              }
            : payload,
        );
      }),
  };
}

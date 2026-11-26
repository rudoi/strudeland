/**
 * The recording payloads as they cross IPC (camelCase, host nanoseconds as decimal strings,
 * channels 0-based), the sidecar fields the webview builds, and the controller's UI state. Mirrors
 * the Rust `recording.rs` and `strudeland-project` types. Pure types and one error class; the
 * only imports are `../ipc/command-error` and the generated `../ipc/bindings`, neither of which
 * imports Tauri.
 */

import type { CommandErrorKind } from '../ipc/bindings';
import { CommandError } from '../ipc/command-error';

/** Every `RecordingError` kind the Rust commands and events carry. */
export const RECORDING_ERROR_KINDS = [
  'permissionDenied',
  'noDevice',
  'deviceLost',
  'busy',
  'invalidName',
  'exists',
  'notFound',
  'projectChanged',
  'limitReached',
  'other',
] as const satisfies readonly CommandErrorKind[];
export type RecordingErrorKind = (typeof RECORDING_ERROR_KINDS)[number];

/** The structured rejection of every recording command (`{ kind, message }` over IPC). */
export class RecordingError extends CommandError<RecordingErrorKind> {
  constructor(message: string, kind: RecordingErrorKind, options?: ErrorOptions) {
    super(message, kind, options);
    this.name = 'RecordingError';
  }
}

/** An input device as `recording_devices` lists it. */
export interface InputDevice {
  id: string;
  name: string;
  channels: number;
  sampleRate: number;
  isDefault: boolean;
  bluetooth: boolean;
}

export type Permission = 'notDetermined' | 'denied' | 'restricted' | 'authorized';

export const FIXED_METER_BACKING_MESSAGE = 'song backing requires a fixed time signature';

/** The Rust recorder's state: no stream, a stream without a take, or a take. */
export type RecState = 'idle' | 'armed' | 'recording';

export interface RecordingStatus {
  state: RecState;
  permission: Permission;
  /** The armed device; null in `idle`. */
  device: InputDevice | null;
  /** 0-based: the armed channel, or in `idle` the saved one (0 when none). */
  channel: number;
  /** Set only while `recording`. */
  stem: string | null;
  elapsedSeconds: number | null;
}

/** A device and its 0-based channel. */
export interface InputSelection {
  deviceId: string;
  channel: number;
}

export interface DevicesResult {
  devices: InputDevice[];
  /** The saved choice resolved against the present devices; null without any input device. */
  selected: InputSelection | null;
}

/** `recording://level`: linear peak and RMS, about 30 Hz while armed or recording. */
export interface LevelEvent {
  peak: number;
  rms: number;
}

/** `recording://state`. */
export interface StateEvent {
  state: RecState;
  stem?: string | null;
  error?: { kind: RecordingErrorKind; message: string } | null;
}

// ---- sidecar schema 1 ----

export interface SidecarOutput {
  /** Seconds. */
  baseLatency: number;
  /** Seconds; null when the context reports none. */
  outputLatency: number | null;
}

export interface SidecarTempo {
  bpm: number;
  /** "4/4" */
  meter: string;
  cps: number;
  /** Quarter notes per bar (6/8 gives 3). */
  beatsPerBar: number;
  tempoOverride: boolean;
}

export interface SidecarBacking {
  song: boolean;
  metronome: boolean;
  countIn: boolean;
  /** "songs/x.js"; null with the song off. */
  songPath: string | null;
  startSection: string | null;
  loopSection: string | null;
}

/** A clock offset as the sidecar stores it: signed and non-negative decimal nanosecond strings. */
export interface ClockOffsetJson {
  offsetNs: string;
  rttNs: string;
}

export interface ClockOffsets {
  start: ClockOffsetJson | null;
  end: ClockOffsetJson | null;
}

/** What `recording_start` takes: enough for Rust to write a timing-null sidecar on its own. */
export interface StartFields {
  tempo: SidecarTempo;
  backing: SidecarBacking;
}

/** What `recording_stop` takes as `sidecar`; Rust adds the rest of the schema. */
export interface StopFields {
  output: SidecarOutput;
  tempo: SidecarTempo;
  backing: SidecarBacking;
  timing: {
    bar1HostNs: string | null;
    clockOffsetNs: ClockOffsets;
    /** `[contextTime, hostNs]` pairs. */
    outputStamps: [number, string][];
  };
}

export interface Sidecar {
  schema: 1;
  /** RFC 3339 UTC. */
  createdAt: string;
  app: { version: string };
  input: {
    deviceId: string;
    deviceName: string;
    /** 0-based. */
    channel: number;
    channels: number;
    sampleRate: number;
  };
  output: SidecarOutput;
  tempo: SidecarTempo;
  backing: SidecarBacking;
  timing: {
    bar1Frame: number | null;
    bar1HostNs: string | null;
    clockOffsetNs: ClockOffsets;
    /** `[frame, hostNs]` pairs. */
    anchors: [number, string][];
    outputStamps: [number, string][];
  };
  xruns: number;
  recovered: boolean;
}

/** A memo in `recordings/`, as `recording_list` and `recording_stop` return it. */
export interface RecordingEntry {
  stem: string;
  frames: number | null;
  sampleRate: number | null;
  /** Seconds. */
  duration: number | null;
  hasSidecar: boolean;
  /** The parsed sidecar; null when missing or invalid. */
  sidecar: Sidecar | null;
  sidecarError: string | null;
  /** Bytes of the WAV. */
  size: number;
  modifiedMs: number;
}

// ---- the controller ----

/** What plays during a take, as the UI resolved it. */
export interface BackingChoice {
  song: boolean;
  metronome: boolean;
  countIn: boolean;
}

export type RecordingPhase =
  'idle' | 'arming' | 'armed' | 'starting' | 'count-in' | 'recording' | 'stopping';

export interface RecordingUiError {
  kind: RecordingErrorKind | 'eval' | 'meta' | 'audio' | 'clock';
  message: string;
}

export interface RecordingUiState {
  phase: RecordingPhase;
  /** 1..num while counting in, else null. */
  countInBeat: number | null;
  /** `nowMs()` when the capture was running; elapsed = now - this. */
  takeStartedAtMs: number | null;
  /** The last `devices()` result. */
  devices: InputDevice[];
  /** The resolved selection, channel 0-based. */
  input: InputSelection | null;
  error: RecordingUiError | null;
  /** `notDetermined` until the first status or arm. */
  permission: Permission;
  /** The take went on, but something is off. */
  warning: string | null;
  /** The take in progress. */
  stem: string | null;
  /** The last finished take. */
  lastStem: string | null;
}

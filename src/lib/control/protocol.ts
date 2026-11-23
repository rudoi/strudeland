// Types the webview shares with the control socket (`strudeland-ctl` on the Rust side). No Tauri, no
// DOM: the session and the handler both import from here.
import type { LiveStats } from '../audio/playback-api';

export type { LiveStats };

/** JSON-RPC error codes; mirrors `strudeland-ctl`'s `protocol::codes` (checked by a test). */
export const ErrorCode = {
  PARSE_ERROR: -32700,
  INVALID_REQUEST: -32600,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  INTERNAL_ERROR: -32603,
  TIMEOUT: -32002,
  NOT_READY: -32003,
  NO_PROJECT: -32004,
  NO_SUCH_SONG: -32005,
  SONG_UNLOADABLE: -32006,
  NOTHING_EVALUATED: -32007,
  SCREENSHOT_FAILED: -32008,
  SONG_CHANGED: -32010,
  UNSAVED: -32011,
  SONG_EXISTS: -32012,
  OPEN_FAILED: -32013,
  BUSY: -32014,
} as const;

/** Thrown by the handler (and by nothing else) to produce a JSON-RPC error reply. */
export class ControlError extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
    this.name = 'ControlError';
  }
}

/** Every method name; mirrors `strudeland-ctl`'s `Method` (checked by a test). */
export const METHOD_NAMES = [
  'ping',
  'state',
  'code.get',
  'code.set',
  'eval',
  'play',
  'stop',
  'events',
  'screenshot',
  'song.new',
  'song.switch',
  'song.rename',
  'song.delete',
  'save',
  'project.open',
] as const;

export type MethodName = (typeof METHOD_NAMES)[number];

/** What `control://request` carries. `ping` and `screenshot` never reach the webview. */
export interface ControlRequest {
  id: number;
  method: Exclude<MethodName, 'ping' | 'screenshot'>;
  params: unknown;
  /** The effective timeout Rust waits for the reply; a request still queued after it is skipped. */
  timeoutMs?: number;
}

export type ControlReply = { result: unknown } | { error: { code: number; message: string } };

/** `'off'`: not running in Tauri. */
export type ControlStatus = 'off' | 'listening' | 'in-use' | 'disabled';

export interface ControlInfo {
  status: ControlStatus;
  path: string | null;
  detail: string | null;
}

export interface EvalOutcome {
  ok: boolean;
  /** The evaluated song's rel path. */
  song: string;
  title: string;
  started: boolean;
  audio: 'running' | 'suspended';
  /** Null when `ok`. */
  error: string | null;
}

export type Conflict = 'changed' | 'deleted' | null;

export interface AppStateSummary {
  project: { name: string; root: string } | null;
  songs: { path: string; title: string; dirty: boolean; conflict: Conflict }[];
  active: string | null;
  playing: { path: string; title: string } | null;
  started: boolean;
  cps: number;
  bpm: number;
  meter: string;
  audio: 'running' | 'suspended';
  evalError: string | null;
  metaError: string | null;
  tempoOverride: boolean;
  /** The evaluated song's `meta.seed`; left out without one. */
  seed?: number;
  playhead: Playhead | null;
  transport: TransportSummary;
  outputError: string | null;
  engine: 'superdough' | 'rust';
  live: LiveStats | null;
  /** The user's editor has vim keybindings on. */
  vim: boolean;
  /** The master levels held since the last play or evaluate; null until a frame has arrived. */
  levels: LevelsSummary | null;
}

export interface LevelsSummary {
  /** Held peak per channel in dBFS (0.1 steps); null for silence. */
  peakDb: [number | null, number | null];
  /** Held sum of both channels' over counts. */
  overs: number;
  /** Held lowest limiter gain in dB (0 or negative). */
  limitDb: number;
  /** The current limiter preference. */
  limiter: boolean;
}

export interface BarRange {
  from: number;
  to: number;
}

export interface EventsParams {
  bars?: BarRange;
  section?: string;
  all?: boolean;
  limit?: number;
}

export interface SectionPos {
  name: string;
  bar: number;
}

export interface EventLocation {
  line: number;
  col: number;
  endLine: number;
  endCol: number;
  text: string;
}

export interface InspectedEvent {
  bar: number;
  beat: number;
  begin: number;
  end: number;
  onset: boolean;
  section: SectionPos | null;
  value: Record<string, unknown>;
  locations: EventLocation[];
}

export interface EventsResult {
  song: { path: string; title: string };
  from: number;
  to: number;
  stale: boolean;
  truncated: boolean;
  meter: string;
  tempoOverride: boolean;
  events: InspectedEvent[];
}

export interface Playhead {
  cycle: number;
  songBar: number | null;
  section: SectionPos | null;
}

export interface TransportSummary {
  view: 'song' | 'loop' | 'release';
  loopSection: string | null;
  loopThrough?: string;
  armedSection: string | null;
}

export interface CodeSetResult {
  song: string;
  changed: boolean;
  dirty: boolean;
  conflict: Conflict;
  evaluated: EvalOutcome | null;
}

export interface SongRef {
  path: string;
  title: string;
}
export interface SongResult {
  song: SongRef;
}
export interface SongRenameResult {
  from: SongRef;
  to: SongRef;
}
export interface SongDeleteResult {
  song: SongRef;
  stopped: boolean;
}
export interface SaveResult {
  saved: SongRef[];
}
export interface ProjectOpenResult {
  project: { name: string; root: string };
  alreadyOpen: boolean;
}

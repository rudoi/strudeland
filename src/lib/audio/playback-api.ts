import { Channel, invoke } from '@tauri-apps/api/core';
import { kindedError } from '../command-error';
import { errorMessage } from '../errors';
import { hostNowNs as recordingHostNowNs } from '../recording/api';
import type { EngineValue } from '../render/engine-job';
import type { LevelFrame } from '../levels/levels';

// The OS seam of live playback: the only module that calls Tauri for it. The clock is the
// recording API's (`hostNowNs` from `../recording/api`, the one caller of `host_now_ns`). The Rust backend
// (`rust.ts`) and the runtime only see `PlaybackApi`, so they run under `bun test` with the fake
// in `tests/helpers/fake-playback-api.ts`. The shapes mirror `src-tauri/src/playback.rs` and
// `crates/strudeland-live` (`Batch`, `Event`, `Stats`): camelCase, host times as decimal strings.

/** One event of the player's `Channel` (Rust `strudeland_live::Event`). */
export type PlaybackEvent =
  | { kind: 'anchor'; frame: number; hostNs: string }
  | { kind: 'warning'; signature: string; keys: string[]; firstCycle: number }
  | ({ kind: 'stats' } & EngineStats)
  | { kind: 'scope'; samples: number[] }
  | ({ kind: 'levels' } & LevelFrame)
  | { kind: 'error'; message: string };

/** The engine's counters (cumulative) and its two windowed fields (the last second). */
export interface EngineStats {
  underruns: number;
  lateHaps: number;
  haps: number;
  batches: number;
  skippedHaps: number;
  renderHeadroomMs: number;
  batchDecodeUs: { p50: number; max: number };
}

/** `EngineStats` plus the `playback_schedule` round trip over the last 256 batches, in ms. */
export interface LiveStats extends EngineStats {
  invokeRttMs: { p50: number; p95: number; max: number };
}

export interface OrbitSpec {
  orbit: number;
  channels: number[];
}

/** `values` new to the session (`[index, encodedControlMap, orbitSpec | null]`) apply before
 *  `haps` (`[t, duration, cycle, index]`); every hap of the batch plays at `cps`. */
export interface PlaybackBatch {
  values: [number, Record<string, EngineValue>, OrbitSpec | null][];
  haps: [number, number, number, number][];
  cps: number;
}

const PLAYBACK_ERROR_KINDS = ['stale', 'output', 'badBatch', 'engine'] as const;

export type PlaybackErrorKind = (typeof PLAYBACK_ERROR_KINDS)[number];

export class PlaybackError extends Error {
  constructor(
    readonly kind: PlaybackErrorKind,
    message: string,
  ) {
    super(message);
    this.name = 'PlaybackError';
  }
}

export interface PlaybackApi {
  /** Opens a session (ending any earlier one); resolves to its id. */
  start(seed: number, onEvent: (e: PlaybackEvent) => void): Promise<number>;
  /** Resolves once the batch is queued to the engine thread. */
  schedule(session: number, batch: PlaybackBatch): Promise<void>;
  stop(session: number): Promise<void>;
  setSeed(session: number, seed: number): Promise<void>;
  /** Switches the live safety limiter of the session (a new session starts with it off). */
  setLimiter(session: number, on: boolean): Promise<void>;
  /** The Mach host time in nanoseconds (the recording API's `hostNowNs`). */
  hostNowNs(): Promise<bigint>;
}

/**
 * A rejection of a playback command to a `PlaybackError`: the Rust `{ kind, message }` object,
 * else kind `engine` (also for an unknown kind) with whatever was thrown as text.
 */
export function toPlaybackError(e: unknown): PlaybackError {
  if (e instanceof PlaybackError) return e;
  const k = kindedError(e);
  if (k && (PLAYBACK_ERROR_KINDS as readonly unknown[]).includes(k.kind)) {
    return new PlaybackError(k.kind as PlaybackErrorKind, k.message);
  }
  return new PlaybackError('engine', k?.message ?? errorMessage(e));
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toPlaybackError(e);
  }
}

/** The real implementation. Argument keys are camelCase, as Tauri maps them. */
export const tauriPlaybackApi: PlaybackApi = {
  async start(seed, onEvent) {
    const channel = new Channel<PlaybackEvent>();
    channel.onmessage = onEvent;
    const r = await call<{ session: number }>('playback_start', { seed, channel });
    return r.session;
  },
  schedule: (session, batch) => call<void>('playback_schedule', { session, batch }),
  stop: (session) => call<void>('playback_stop', { session }),
  setSeed: (session, seed) => call<void>('playback_seed', { session, seed }),
  setLimiter: (session, on) => call<void>('playback_limiter', { session, on }),
  async hostNowNs() {
    try {
      return await recordingHostNowNs();
    } catch (e) {
      // Whatever went wrong (a bad value, a failed invoke) is the engine's clock failing.
      throw new PlaybackError('engine', errorMessage(e));
    }
  },
};

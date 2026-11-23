import type { Hap } from '../strudel/types';
import type { LevelFrame } from '../levels/levels';
import type { LiveStats } from './playback-api';

/** A sound the engine cannot play: skipped, reported once per signature. */
export interface EngineWarning {
  signature: string;
  keys: string[];
  firstCycle: number;
}

/** The live audio graph: the context and the node everything audible feeds (pre-device). */
export interface AudioGraph {
  context: AudioContext;
  destination: AudioNode;
}

/** Wall time for a musical hap whose bar lengths vary. The onset cycle stays on the hap. */
export interface HapTiming {
  durationSec: number;
  phaseSec: number;
}

export interface AudioBackend {
  readonly id: 'superdough' | 'rust';
  /** One-time setup (worklets etc.). Idempotent. Call only from a user gesture. */
  init(): Promise<void>;
  /**
   * How long `resume()` may take before the runtime treats audio as suspended (ms); absent: the
   * runtime's default. A resume that completes later is ended with `stop`.
   */
  readonly resumeTimeoutMs?: number;
  /** Resume the clock. Call from a user gesture. Resolves true iff the clock is running afterwards. */
  resume(): Promise<boolean>;
  isRunning(): boolean;
  /** Seconds on the clock that haps are scheduled against. */
  getTime(): number;
  /**
   * Seeds the sound engine's own randomness (noise, oscillator phases).
   * Called once per successful evaluate.
   */
  setSeed(seed: number): void;
  /**
   * Plays one hap. `t` is the absolute target time on `getTime()`'s clock, latency included; the
   * hap lasts `timing.durationSec` or, in constant meter, `hap.duration / cps` seconds.
   */
  output(hap: Hap, t: number, cps: number, timing?: HapTiming): void | Promise<void>;
  /** Fills `buf` with the latest output samples; false when there is no signal source. */
  readScope?(buf: Float32Array): boolean;
  /**
   * The live graph, or null unless audio runs (it never creates the context) or while the output
   * is being rebuilt. Never throws. Callers call it each time they need a node and never keep the
   * result: the destination and the context can be replaced.
   */
  graph?(): AudioGraph | null;
  /** The engine's latest stats (a stable reference between updates), or null. */
  liveStats?(): LiveStats | null;
  /** Subscribes to skipped-sound warnings; returns the remover. */
  onWarning?(cb: (w: EngineWarning) => void): () => void;
  /** Subscribes to master level frames (push-based); returns the remover. */
  onLevels?(cb: (frame: LevelFrame) => void): () => void;
  /** Turns the master safety limiter on or off (the preference; kept across sessions). */
  setLimiter?(on: boolean): void;
  /** Whether this backend can limit its output right now. */
  limiterAvailable?(): boolean;
  /** The delay the master stage adds to what is heard, in seconds (0 when none). */
  masterLatencySec?(): number;
  /**
   * Ends the session (every scheduled voice stops); the next `resume()` opens a new one. The
   * runtime calls it at every stop and when the backend stops being the active one.
   */
  stop?(): Promise<void>;
}

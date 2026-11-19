/**
 * One master level frame (Rust `strudeland_live::LevelFrame`): per channel (0 left, 1 right) the
 * largest |x| (`peak`), the root mean square (`rms`) and the count of samples at or over full
 * scale (`overs`) since the previous frame, and `limitDb`, the lowest gain the limiter applied
 * (0 when it is off or idle, otherwise negative). Every field is finite.
 */
export interface LevelFrame {
  peak: [number, number];
  rms: [number, number];
  overs: [number, number];
  limitDb: number;
}

/** How long the peak readout keeps a held maximum before a lower frame replaces it. */
export const PEAK_HOLD_MS = 1000;
/** A frame older than this no longer counts as live: the meter goes inactive. */
export const LEVEL_FRESH_MS = 500;

/**
 * Two frames as one: per channel `peak` and `rms` by max (a display choice, not a power sum),
 * `overs` summed, `limitDb` the lowest.
 */
export function mergeFrames(a: LevelFrame, b: LevelFrame): LevelFrame {
  return {
    peak: [Math.max(a.peak[0], b.peak[0]), Math.max(a.peak[1], b.peak[1])],
    rms: [Math.max(a.rms[0], b.rms[0]), Math.max(a.rms[1], b.rms[1])],
    overs: [a.overs[0] + b.overs[0], a.overs[1] + b.overs[1]],
    limitDb: Math.min(a.limitDb, b.limitDb),
  };
}

/** What the hold keeps: the largest peak per channel, all overs, the deepest limiter gain. */
export interface LevelHoldSnapshot {
  peak: [number, number];
  overs: number;
  limitDb: number;
}

export interface LevelHold {
  push(frame: LevelFrame): void;
  reset(): void;
  /** Null from construction and after `reset()` until the next frame arrives. */
  snapshot(): LevelHoldSnapshot | null;
}

export function createLevelHold(): LevelHold {
  let held: LevelHoldSnapshot | null = null;
  return {
    push(f) {
      const overs = f.overs[0] + f.overs[1];
      held =
        held === null
          ? { peak: [f.peak[0], f.peak[1]], overs, limitDb: f.limitDb }
          : {
              peak: [Math.max(held.peak[0], f.peak[0]), Math.max(held.peak[1], f.peak[1])],
              overs: held.overs + overs,
              limitDb: Math.min(held.limitDb, f.limitDb),
            };
    },
    reset() {
      held = null;
    },
    snapshot() {
      return held === null ? null : { ...held, peak: [held.peak[0], held.peak[1]] };
    },
  };
}

/** A linear peak in dBFS; null when it is 0 (a non-finite peak counts as 1.0). */
export const peakDb = (peak: number): number | null => {
  const p = Number.isFinite(peak) ? peak : 1;
  return p > 0 ? 20 * Math.log10(p) : null;
};

export interface PeakReadout {
  /** The held maximum of both channels' peak in dBFS, null when there is none. */
  push(frame: LevelFrame): number | null;
  reset(): void;
}

/** `now` is in milliseconds (`performance.now()`). */
export function createPeakReadout(now: () => number): PeakReadout {
  let held: number | null = null;
  let at = -Infinity;
  return {
    push(frame) {
      const t = now();
      const db = peakDb(Math.max(frame.peak[0], frame.peak[1]));
      const expired = t - at > PEAK_HOLD_MS;
      if (db !== null && (held === null || db >= held || expired)) {
        held = db;
        at = t;
      } else if (db === null && expired) held = null;
      return held;
    },
    reset() {
      held = null;
      at = -Infinity;
    },
  };
}

/** The readout text: `-` for none, else one decimal with a sign and `db` (`-3.2db`, `+2.1db`). */
/** dB rounded to 0.1, with `-0` read as 0. */
export const roundDb = (db: number): number => Math.round(db * 10) / 10 || 0;

export function formatPeakDb(db: number | null): string {
  if (db === null || !Number.isFinite(db)) return '-';
  const r = Math.round(db * 10) / 10;
  return `${r < 0 ? '-' : '+'}${Math.abs(r).toFixed(1)}db`;
}

export interface ChannelView {
  rms: number;
  peak: number;
  clip: boolean;
}

/** The bit of `createMeterModel` the master meter uses (injected so this file stays pure). */
export interface ChannelModel {
  push(level: { peak: number; rms: number }): ChannelView;
  reset(): void;
}

export interface MasterMeterView {
  /** False: both bars empty, readout `-`. A frame of zeros is active (silence is a level). */
  active: boolean;
  channels: [ChannelView, ChannelView];
  /** The peak readout text. */
  readout: string;
  /** Either channel's clip colour is on. */
  clip: boolean;
  /** The latest frame's limiter gain is below -0.1 dB. */
  limiting: boolean;
}

export interface MasterMeter {
  /**
   * One scope-clock call. `frame` is what the runtime drained (null: none arrived); `live` false
   * means the clock called with nothing to draw (the runtime is stopped or the window hidden),
   * which is inactive at once.
   */
  step(frame: LevelFrame | null, live?: boolean): MasterMeterView;
  /** Clears the clip colour only. */
  clearClip(): void;
}

const EMPTY_VIEW: ChannelView = { rms: 0, peak: 0, clip: false };
/** The one inactive view: returning the same object lets a stopped meter skip re-rendering. */
const INACTIVE_VIEW: MasterMeterView = Object.freeze({
  active: false,
  channels: [EMPTY_VIEW, EMPTY_VIEW],
  readout: '-',
  clip: false,
  limiting: false,
}) as MasterMeterView;
const finite = (x: number): number => (Number.isFinite(x) ? x : 1);

/** The master meter's view model: two channel models, the peak readout and the freshness rule. */
export function createMasterMeter(now: () => number, newModel: () => ChannelModel): MasterMeter {
  const models: [ChannelModel, ChannelModel] = [newModel(), newModel()];
  const readout = createPeakReadout(now);
  let last: LevelFrame | null = null;
  let lastAt = -Infinity;
  let text = '-';
  let resting = true;
  const inactive = (): MasterMeterView => {
    if (!resting) {
      models[0].reset();
      models[1].reset();
      readout.reset();
      last = null;
      text = '-';
      resting = true;
    }
    return INACTIVE_VIEW;
  };
  return {
    step(frame, live = true) {
      if (!live) return inactive();
      const t = now();
      let channels: [ChannelView, ChannelView];
      if (frame) {
        last = frame;
        lastAt = t;
        text = formatPeakDb(readout.push(frame));
        channels = [
          models[0].push({ peak: finite(frame.peak[0]), rms: finite(frame.rms[0]) }),
          models[1].push({ peak: finite(frame.peak[1]), rms: finite(frame.rms[1]) }),
        ];
      } else if (last !== null && t - lastAt < LEVEL_FRESH_MS) {
        channels = [
          models[0].push({ peak: 0, rms: finite(last.rms[0]) }),
          models[1].push({ peak: 0, rms: finite(last.rms[1]) }),
        ];
      } else return inactive();
      resting = false;
      return {
        active: true,
        channels,
        readout: text,
        clip: channels[0].clip || channels[1].clip,
        limiting: last !== null && last.limitDb < -0.1,
      };
    },
    clearClip() {
      models[0].reset();
      models[1].reset();
    },
  };
}

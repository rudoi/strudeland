/** The input meter's pure level model: linear amplitude to display fractions, peak hold, clip. */

/** The bottom of the meter's scale. */
export const FLOOR_DB = -60;
/** How fast the peak marker falls, in fractions of the meter per second. */
export const PEAK_FALL_PER_S = 0.6;
/** How long the clip colour stays after a clipping peak. */
export const CLIP_HOLD_MS = 1500;
/** A peak at or above this linear amplitude clips. */
export const CLIP_LINEAR = 0.99;

/** Linear amplitude to a 0..1 fraction over -60..0 dBFS; non-finite or non-positive input is 0. */
export function levelFraction(linear: number): number {
  if (!Number.isFinite(linear) || linear <= 0) return 0;
  const db = 20 * Math.log10(linear);
  if (db <= FLOOR_DB) return 0;
  if (db >= 0) return 1;
  return (db - FLOOR_DB) / -FLOOR_DB;
}

export interface MeterView {
  rms: number;
  peak: number;
  clip: boolean;
}

export interface MeterModel {
  push(level: { peak: number; rms: number }): MeterView;
  reset(): void;
}

/** `now` is in milliseconds (`performance.now()`). */
export function createMeterModel(now: () => number): MeterModel {
  let peak = 0;
  let at: number | null = null;
  let clipUntil = -Infinity;
  return {
    push(level) {
      const t = now();
      const decayed =
        at === null ? 0 : Math.max(0, peak - (PEAK_FALL_PER_S * Math.max(0, t - at)) / 1000);
      peak = Math.max(levelFraction(level.peak), decayed);
      at = t;
      if (Number.isFinite(level.peak) && level.peak >= CLIP_LINEAR) clipUntil = t + CLIP_HOLD_MS;
      return { rms: levelFraction(level.rms), peak, clip: t < clipUntil };
    },
    reset() {
      peak = 0;
      at = null;
      clipUntil = -Infinity;
    },
  };
}

/** Pure maths for drawing a scope trace (no DOM, no audio). */

/** The share of the old frame erased each frame. */
export const AFTERGLOW = 0.42;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * The first index `i` in `1 .. buf.length / 2` where the signal rises through `threshold`
 * (`buf[i - 1] < threshold && buf[i] >= threshold`), else 0.
 */
export function triggerIndex(buf: ArrayLike<number>, threshold = 0): number {
  const end = Math.floor(buf.length / 2);
  for (let i = 1; i <= end && i < buf.length; i++) {
    if (buf[i - 1]! < threshold && buf[i]! >= threshold) return i;
  }
  return 0;
}

/**
 * Writes `count` points of `buf` from `start` into `out` as interleaved x, y pairs, across a
 * `width` × `height` box, scaled by `gain` and clamped, so every point lies inside the box with a
 * 1px margin. Samples past the end read as 0. `count` is capped at the pairs `out` holds; returns
 * the number of points written. Allocates nothing, so a renderer can call it every frame.
 */
export function tracePoints(
  out: Float32Array,
  buf: ArrayLike<number>,
  start: number,
  count: number,
  width: number,
  height: number,
  gain: number,
): number {
  const n = Math.max(0, Math.min(count, Math.floor(out.length / 2)));
  const half = height / 2;
  const span = Math.max(0, half - 1);
  for (let i = 0; i < n; i++) {
    const v = buf[start + i] ?? 0;
    const s = Number.isFinite(v) ? clamp(v * gain, -1, 1) : 0;
    out[2 * i] = count > 1 ? (i / (count - 1)) * width : 0;
    out[2 * i + 1] = half - s * span;
  }
  return n;
}

/** The flat line a resting scope draws. */
export function restLine(width: number, height: number): { x1: number; x2: number; y: number } {
  return { x1: 0, x2: width, y: height / 2 };
}

/**
 * An automatic gain: a peak follower with a fast attack and a slow release. Quiet songs are lifted
 * up to 8x, loud ones held to 0.8 of the box; silence gives 1.
 */
export function createGain(): { next(buf: ArrayLike<number>): number } {
  let env = 0;
  return {
    next(buf) {
      let p = 0;
      for (let i = 0; i < buf.length; i++) {
        const a = Math.abs(buf[i]!);
        if (a > p) p = a;
      }
      if (!Number.isFinite(p)) p = 0;
      env += (p - env) * (p > env ? 0.5 : 0.02);
      return env < 0.0001 ? 1 : clamp(0.8 / env, 0.5, 8);
    },
  };
}

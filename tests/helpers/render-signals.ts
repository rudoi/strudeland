/** Synthetic signals for the render analysis tests. Deterministic. */

import { mulberry32 } from 'superdough/rng.mjs';

export const SR = 44100;

export function sine(freq: number, amp: number, sec: number, sr = SR): Float32Array {
  const n = Math.round(sec * sr);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = amp * Math.sin((2 * Math.PI * freq * i) / sr);
  return out;
}

export function silence(sec: number, sr = SR): Float32Array {
  return new Float32Array(Math.round(sec * sr));
}

/** Short full-scale bursts of `clickSec` at each time in `timesSec`, in a buffer of `sec`. */
export function clickTrain(
  timesSec: number[],
  sec: number,
  sr = SR,
  amp = 1,
  clickSec = 0.005,
): Float32Array {
  const out = new Float32Array(Math.round(sec * sr));
  const len = Math.max(1, Math.round(clickSec * sr));
  for (const t of timesSec) {
    const start = Math.round(t * sr);
    for (let i = start; i < Math.min(out.length, start + len); i++) out[i] = amp;
  }
  return out;
}

/** Uniform noise in [-amp, amp) from a seeded mulberry32. */
export function noiseSeeded(seed: number, n: number, amp = 1): Float32Array {
  const rnd: () => number = mulberry32(seed);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = amp * (rnd() * 2 - 1);
  return out;
}

/** `x` delayed by `lag` samples (zeros in front), same length. Negative lag advances. */
export function delayed(x: ArrayLike<number>, lag: number): Float64Array {
  const out = new Float64Array(x.length);
  for (let i = 0; i < x.length; i++) {
    const j = i - lag;
    if (j >= 0 && j < x.length) out[i] = x[j]!;
  }
  return out;
}

export function concat(...parts: Float32Array[]): Float32Array {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

export function scale(x: Float32Array, k: number): Float32Array {
  return x.map((v) => v * k);
}

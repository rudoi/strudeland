/**
 * Integrated loudness, ITU-R BS.1770-4 (the gated measure EBU R128 uses). All channels weigh 1
 * (the mix is L/R; surround weights are out of scope). Pure.
 */

import { powerToDb } from './dsp';

interface Biquad {
  b: [number, number, number];
  /** a[0] = 1. */
  a: [number, number, number];
}

const SHELF_F0 = 1681.974450955533;
const SHELF_GAIN_DB = 3.999843853973347;
const SHELF_Q = 0.7071752369554196;
const SHELF_VB_EXP = 0.4996667741545416;
const HP_F0 = 38.13547087602444;
const HP_Q = 0.5003270373238773;

/** K-weighting biquads at `sampleRate`, by the bilinear transform of the BS.1770 analogue
 *  prototypes (reproduces the published 48 kHz table). */
export function kWeightingCoeffs(sampleRate: number): { shelf: Biquad; highpass: Biquad } {
  if (!(sampleRate > 0) || !Number.isFinite(sampleRate)) {
    throw new RangeError(`invalid sample rate ${sampleRate}`);
  }
  let k = Math.tan((Math.PI * SHELF_F0) / sampleRate);
  const vh = Math.pow(10, SHELF_GAIN_DB / 20);
  const vb = Math.pow(vh, SHELF_VB_EXP);
  let a0 = 1 + k / SHELF_Q + k * k;
  const shelf: Biquad = {
    b: [
      (vh + (vb * k) / SHELF_Q + k * k) / a0,
      (2 * (k * k - vh)) / a0,
      (vh - (vb * k) / SHELF_Q + k * k) / a0,
    ],
    a: [1, (2 * (k * k - 1)) / a0, (1 - k / SHELF_Q + k * k) / a0],
  };
  k = Math.tan((Math.PI * HP_F0) / sampleRate);
  a0 = 1 + k / HP_Q + k * k;
  const highpass: Biquad = {
    b: [1, -2, 1],
    a: [1, (2 * (k * k - 1)) / a0, (1 - k / HP_Q + k * k) / a0],
  };
  return { shelf, highpass };
}

function filter(x: ArrayLike<number>, f: Biquad): Float64Array {
  const [b0, b1, b2] = f.b;
  const [, a1, a2] = f.a;
  const out = new Float64Array(x.length);
  let x1 = 0;
  let x2 = 0;
  let y1 = 0;
  let y2 = 0;
  for (let i = 0; i < x.length; i++) {
    const x0 = x[i]!;
    const y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
    out[i] = y0;
    x2 = x1;
    x1 = x0;
    y2 = y1;
    y1 = y0;
  }
  return out;
}

const blockLoudness = (sumZ: number): number => -0.691 + powerToDb(sumZ);

/** Gated integrated loudness in LUFS; -Infinity when undefined (no channels, shorter than one
 *  400 ms block, or nothing above the -70 LUFS absolute gate). Non-finite samples give NaN.
 *  Throws RangeError on unequal channel lengths or a non-positive sample rate. */
export function integratedLufs(channels: Float32Array[], sampleRate: number): number {
  if (!(sampleRate > 0) || !Number.isFinite(sampleRate)) {
    throw new RangeError(`invalid sample rate ${sampleRate}`);
  }
  if (channels.length === 0) return -Infinity;
  const n = channels[0]!.length;
  if (channels.some((c) => c.length !== n)) throw new RangeError('channel length mismatch');
  const block = Math.round(0.4 * sampleRate);
  const step = Math.round(0.1 * sampleRate);
  if (n < block) return -Infinity;
  const count = Math.floor((n - block) / step) + 1;

  const { shelf, highpass } = kWeightingCoeffs(sampleRate);
  // Per-block sum over channels of the mean square, via a prefix sum of squares per channel.
  const sumZ = new Float64Array(count);
  for (const ch of channels) {
    const y = filter(filter(ch, shelf), highpass);
    const prefix = new Float64Array(n + 1);
    for (let i = 0; i < n; i++) prefix[i + 1] = prefix[i]! + y[i]! * y[i]!;
    for (let j = 0; j < count; j++) {
      const s = j * step;
      sumZ[j] = sumZ[j]! + (prefix[s + block]! - prefix[s]!) / block;
    }
  }
  if (sumZ.some((z) => Number.isNaN(z))) return NaN;

  const absolute: number[] = [];
  for (const z of sumZ) if (blockLoudness(z) > -70) absolute.push(z);
  if (absolute.length === 0) return -Infinity;
  const mean = (xs: number[]): number => xs.reduce((a, b) => a + b, 0) / xs.length;
  const relativeGate = blockLoudness(mean(absolute)) - 10;
  const kept = absolute.filter((z) => blockLoudness(z) > relativeGate);
  if (kept.length === 0) return -Infinity;
  return blockLoudness(mean(kept));
}

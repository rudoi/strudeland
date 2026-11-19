/**
 * Signal helpers for the render gates and features: levels, a bandpass, windowed RMS and band
 * envelopes, and an onset envelope. Pure and deterministic; float64 state, Float32Array output.
 */

/** The four feature bands (Hz). The seam gate uses its own SEAM_BANDS. */
export const BANDS: readonly { name: 'sub' | 'low' | 'mid' | 'high'; lo: number; hi: number }[] = [
  { name: 'sub', lo: 20, hi: 130 },
  { name: 'low', lo: 130, hi: 500 },
  { name: 'mid', lo: 500, hi: 2000 },
  { name: 'high', lo: 2000, hi: 16000 },
];

/** Linear amplitude floor of band envelopes: -100 dBFS. */
export const ENV_FLOOR = 1e-5;

const POW10 = [1, 10, 100, 1000, 1e4, 1e5, 1e6];

/** `x` rounded to `decimals` places: Math.round(x * 10^decimals) / 10^decimals. */
export function round(x: number, decimals: number): number {
  const m = POW10[decimals] ?? 10 ** decimals;
  return Math.round(x * m) / m;
}

/** 20 log10(max(x, floor)): a linear amplitude in dB. */
export const ampToDb = (x: number, floor: number): number => 20 * Math.log10(Math.max(x, floor));

/** 20 log10((a + floor) / (b + floor)): an amplitude ratio in dB. */
export const ratioDb = (a: number, b: number, floor: number): number =>
  20 * Math.log10((a + floor) / (b + floor));

/** 10 log10(p): a power (mean square) in dB. */
export const powerToDb = (p: number): number => 10 * Math.log10(p);

/** Ascending copy of `values`. */
export const sortedAsc = (values: readonly number[]): number[] => [...values].sort((a, b) => a - b);

/** Nearest-rank quantile of an ascending, non-empty list: sorted[round(p * (n - 1))], clamped
 *  to the list. */
export function nearestRank(sorted: readonly number[], p: number): number {
  const n = sorted.length;
  return sorted[Math.min(n - 1, Math.max(0, Math.round(p * (n - 1))))]!;
}

/** Median, the mean of the middle two for an even count; 0 for an empty list. */
export function median(values: readonly number[]): number {
  if (values.length === 0) return 0;
  const s = sortedAsc(values);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m]! : (s[m - 1]! + s[m]!) / 2;
}

/** Root mean square; 0 for an empty array. */
export function rms(x: Float32Array): number {
  if (x.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < x.length; i++) sum += x[i]! * x[i]!;
  return Math.sqrt(sum / x.length);
}

/** Mean of the channels; [] gives an empty array. Throws RangeError on unequal lengths. */
export function mixToMono(channels: Float32Array[]): Float32Array {
  if (channels.length === 0) return new Float32Array(0);
  const n = channels[0]!.length;
  if (channels.some((c) => c.length !== n)) throw new RangeError('channel length mismatch');
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    let sum = 0;
    for (const c of channels) sum += c[i]!;
    out[i] = sum / channels.length;
  }
  return out;
}

/** Max |x|; 0 for an empty array. */
export function peak(x: Float32Array): number {
  let p = 0;
  for (let i = 0; i < x.length; i++) {
    const a = Math.abs(x[i]!);
    if (a > p) p = a;
  }
  return p;
}

/** Count of NaN/Infinity samples over all channels and the first one (lowest frame, then lowest
 *  channel); frame and channel are -1 when there is none. */
export function firstNonFinite(channels: Float32Array[]): {
  count: number;
  frame: number;
  channel: number;
} {
  let count = 0;
  let frame = -1;
  let channel = -1;
  for (const [c, ch] of channels.entries()) {
    for (let i = 0; i < ch.length; i++) {
      if (!Number.isFinite(ch[i]!)) {
        count++;
        if (frame < 0 || i < frame) {
          frame = i;
          channel = c;
        }
      }
    }
  }
  return { count, frame, channel };
}

/** Fraction of samples with |x| strictly greater than 1.0; 0 for an empty array. */
export function clipFraction(x: Float32Array): number {
  if (x.length === 0) return 0;
  let n = 0;
  for (let i = 0; i < x.length; i++) if (Math.abs(x[i]!) > 1) n++;
  return n / x.length;
}

/** Seconds of the first sample with |x| above `thresholdDb` dBFS (absolute), or Infinity. */
export function firstAudibleSec(x: Float32Array, sampleRate: number, thresholdDb = -40): number {
  const thr = Math.pow(10, thresholdDb / 20);
  for (let i = 0; i < x.length; i++) if (Math.abs(x[i]!) > thr) return i / sampleRate;
  return Infinity;
}

/** RBJ constant-0-dB-peak bandpass centred on sqrt(lo * hi), direct form I, one forward pass,
 *  zero initial state. Throws RangeError unless 0 < lo < hi < sampleRate / 2. */
export function biquadBandpass(
  x: Float32Array,
  sampleRate: number,
  loHz: number,
  hiHz: number,
): Float32Array {
  if (!(loHz > 0 && loHz < hiHz && hiHz < sampleRate / 2)) {
    throw new RangeError(`invalid band ${loHz}-${hiHz} Hz at ${sampleRate} Hz`);
  }
  const f0 = Math.sqrt(loHz * hiHz);
  const q = f0 / (hiHz - loHz);
  const w0 = (2 * Math.PI * f0) / sampleRate;
  const alpha = Math.sin(w0) / (2 * q);
  const a0 = 1 + alpha;
  const b0 = alpha / a0;
  const b2 = -alpha / a0;
  const a1 = (-2 * Math.cos(w0)) / a0;
  const a2 = (1 - alpha) / a0;
  const out = new Float32Array(x.length);
  let x1 = 0;
  let x2 = 0;
  let y1 = 0;
  let y2 = 0;
  for (let i = 0; i < x.length; i++) {
    const x0 = x[i]!;
    const y0 = b0 * x0 + b2 * x2 - a1 * y1 - a2 * y2;
    out[i] = y0;
    x2 = x1;
    x1 = x0;
    y2 = y1;
    y1 = y0;
  }
  return out;
}

/** RMS of consecutive non-overlapping windows of max(1, round(windowSec * sr)) frames; the
 *  partial tail is dropped. */
export function rmsWindows(x: Float32Array, sampleRate: number, windowSec: number): Float32Array {
  const win = Math.max(1, Math.round(windowSec * sampleRate));
  const n = Math.floor(x.length / win);
  const out = new Float32Array(n);
  for (let k = 0; k < n; k++) {
    let sum = 0;
    for (let i = k * win; i < (k + 1) * win; i++) sum += x[i]! * x[i]!;
    out[k] = Math.sqrt(sum / win);
  }
  return out;
}

/** Bandpassed windowed RMS in dB, floored at ENV_FLOOR (silence is exactly -100). */
export function bandEnvelopeDb(
  x: Float32Array,
  sampleRate: number,
  loHz: number,
  hiHz: number,
  windowSec = 0.1,
): Float32Array {
  const w = rmsWindows(biquadBandpass(x, sampleRate, loHz, hiHz), sampleRate, windowSec);
  const out = new Float32Array(w.length);
  for (let i = 0; i < w.length; i++) out[i] = ampToDb(w[i]!, ENV_FLOOR);
  return out;
}

/** Positive RMS rise between consecutive hops: env[j] is the rise into hop j + 1, at time
 *  (j + 1) * hop / sr. Length max(0, floor(len / hop) - 1). Linear amplitude. */
export function onsetEnvelope(x: Float32Array, sampleRate: number, hopSec = 0.01): Float32Array {
  const hop = Math.max(1, Math.round(hopSec * sampleRate));
  const n = Math.floor(x.length / hop);
  const energy = rmsWindows(x, sampleRate, hop / sampleRate);
  const out = new Float32Array(Math.max(0, n - 1));
  for (let k = 1; k < n; k++) out[k - 1] = Math.max(0, energy[k]! - energy[k - 1]!);
  return out;
}

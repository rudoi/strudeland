/**
 * Golden features of a mix render: levels, band envelopes, onset envelope and a PCM hash of the
 * compared window, with stable (de)serialization. Pure; ships its own SHA-256 so it runs in Bun,
 * Chromium and tests alike.
 */

import { windowFrames, type RenderResult } from './contract';
import { ampToDb, BANDS, bandEnvelopeDb, mixToMono, onsetEnvelope, peak, round } from './dsp';
import { integratedLufs } from './loudness';

const FEATURES_SCHEMA = 1;

export interface Features {
  schema: 1;
  sampleRate: number;
  /** Frames of the compared window (lookback and overlap excluded). */
  frames: number;
  windowSec: number;
  /** Linear max |x| over both channels, 6 decimals. */
  peak: number;
  /** 20 log10(max(peak, 1e-5)), 2 decimals. */
  peakDb: number;
  /** integratedLufs, 2 decimals; null for -Infinity (JSON has no Infinity). */
  lufs: number | null;
  /** 100 ms windows, 0.01 dB steps. */
  bandsDb: { sub: number[]; low: number[]; mid: number[]; high: number[] };
  /** onsetEnvelope at a 10 ms hop of the mono mix, rounded to 1e-6. */
  onset: number[];
  /** Lowercase hex SHA-256 of the window PCM (little-endian float32, interleaved). */
  pcmSha256: string;
}

export class FeaturesError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FeaturesError';
  }
}

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/** SHA-256 (FIPS 180-4) of `bytes`, lowercase hex. Synchronous. */
export function sha256Hex(bytes: Uint8Array): string {
  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);
  const len = bytes.length;
  const padded = new Uint8Array((((len + 8) >> 6) + 1) << 6);
  padded.set(bytes);
  padded[len] = 0x80;
  const view = new DataView(padded.buffer);
  view.setUint32(padded.length - 8, Math.floor(len / 0x20000000), false);
  view.setUint32(padded.length - 4, (len << 3) >>> 0, false);
  const w = new Uint32Array(64);
  const rotr = (x: number, n: number) => (x >>> n) | (x << (32 - n));
  for (let off = 0; off < padded.length; off += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(off + i * 4, false);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15]!, 7) ^ rotr(w[i - 15]!, 18) ^ (w[i - 15]! >>> 3);
      const s1 = rotr(w[i - 2]!, 17) ^ rotr(w[i - 2]!, 19) ^ (w[i - 2]! >>> 10);
      w[i] = (w[i - 16]! + s0 + w[i - 7]! + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, hh] = [h[0]!, h[1]!, h[2]!, h[3]!, h[4]!, h[5]!, h[6]!, h[7]!];
    for (let i = 0; i < 64; i++) {
      const S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const t1 = (hh + S1 + ch + K[i]! + w[i]!) >>> 0;
      const S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (S0 + maj) >>> 0;
      hh = g;
      g = f;
      f = e;
      e = (d + t1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) >>> 0;
    }
    h[0] = (h[0]! + a) >>> 0;
    h[1] = (h[1]! + b) >>> 0;
    h[2] = (h[2]! + c) >>> 0;
    h[3] = (h[3]! + d) >>> 0;
    h[4] = (h[4]! + e) >>> 0;
    h[5] = (h[5]! + f) >>> 0;
    h[6] = (h[6]! + g) >>> 0;
    h[7] = (h[7]! + hh) >>> 0;
  }
  let out = '';
  for (let i = 0; i < 8; i++) out += h[i]!.toString(16).padStart(8, '0');
  return out;
}

const round2 = (v: number) => round(v, 2);
const round6 = (v: number) => round(v, 6);

const LITTLE_ENDIAN = new Uint8Array(new Uint32Array([1]).buffer)[0] === 1;

/** Little-endian float32 interleaved bytes of the window, independent of platform endianness. */
function windowBytes(win: Float32Array[]): Uint8Array {
  const n = win[0]!.length;
  const ch = win.length;
  if (LITTLE_ENDIAN) {
    // Native float32 order is the wanted byte order: interleave, then view the bytes.
    const out = new Float32Array(n * ch);
    for (let c = 0; c < ch; c++) {
      const x = win[c]!;
      for (let i = 0, o = c; i < n; i++, o += ch) out[o] = x[i]!;
    }
    return new Uint8Array(out.buffer);
  }
  const bytes = new Uint8Array(n * ch * 4);
  const view = new DataView(bytes.buffer);
  let o = 0;
  for (let i = 0; i < n; i++) {
    for (let c = 0; c < ch; c++, o += 4) view.setFloat32(o, win[c]![i]!, true);
  }
  return bytes;
}

/** Features of the compared window of a mix render. RangeError for stems, failed renders,
 *  a window outside the buffer, or non-finite samples. */
export function extractFeatures(result: RenderResult): Features {
  const req = result.request;
  if (req.mode !== 'mix') throw new RangeError('features need a mix render');
  if (!result.ok) throw new RangeError('render failed');
  const sr = result.sampleRate;
  const n = windowFrames(req.fromBar, req.toBar, result.cps, sr);
  const start = result.windowStartFrame;
  if (result.channels.length !== 2 || start < 0 || start + n > result.frames) {
    throw new RangeError('window outside the rendered buffer');
  }
  const win = result.channels.map((c) => c.subarray(start, start + n));
  for (const c of win) {
    for (let i = 0; i < c.length; i++) {
      if (!Number.isFinite(c[i]!)) throw new RangeError('non-finite samples');
    }
  }
  const mono = mixToMono(win);
  const pk = Math.max(peak(win[0]!), peak(win[1]!));
  const lufs = integratedLufs(win, sr);
  const bandsDb = { sub: [] as number[], low: [], mid: [], high: [] } as Features['bandsDb'];
  for (const b of BANDS) {
    bandsDb[b.name] = Array.from(bandEnvelopeDb(mono, sr, b.lo, b.hi, 0.1), round2);
  }
  return {
    schema: 1,
    sampleRate: sr,
    frames: n,
    windowSec: n / sr,
    peak: round6(pk),
    peakDb: round2(ampToDb(pk, 1e-5)),
    lufs: Number.isFinite(lufs) ? round2(lufs) : null,
    bandsDb,
    onset: Array.from(onsetEnvelope(mono, sr, 0.01), round6),
    pcmSha256: sha256Hex(windowBytes(win)),
  };
}

/** Stable JSON: interface key order, 2-space indent, arrays on one line, trailing newline. */
export function serializeFeatures(f: Features): string {
  const arr = (a: number[]) => JSON.stringify(a);
  const lines = [
    '{',
    `  "schema": ${f.schema},`,
    `  "sampleRate": ${f.sampleRate},`,
    `  "frames": ${f.frames},`,
    `  "windowSec": ${JSON.stringify(f.windowSec)},`,
    `  "peak": ${JSON.stringify(f.peak)},`,
    `  "peakDb": ${JSON.stringify(f.peakDb)},`,
    `  "lufs": ${JSON.stringify(f.lufs)},`,
    '  "bandsDb": {',
    `    "sub": ${arr(f.bandsDb.sub)},`,
    `    "low": ${arr(f.bandsDb.low)},`,
    `    "mid": ${arr(f.bandsDb.mid)},`,
    `    "high": ${arr(f.bandsDb.high)}`,
    '  },',
    `  "onset": ${arr(f.onset)},`,
    `  "pcmSha256": ${JSON.stringify(f.pcmSha256)}`,
    '}',
  ];
  return lines.join('\n') + '\n';
}

function isNumArray(v: unknown): v is number[] {
  return Array.isArray(v) && v.every((x) => typeof x === 'number' && Number.isFinite(x));
}

/** Parse and validate golden JSON. Throws FeaturesError. */
export function parseFeatures(json: string): Features {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch (e) {
    throw new FeaturesError(`invalid JSON: ${(e as Error).message}`);
  }
  if (typeof raw !== 'object' || raw === null) throw new FeaturesError('not an object');
  const o = raw as Record<string, unknown>;
  if (o.schema !== FEATURES_SCHEMA)
    throw new FeaturesError(`unsupported schema ${String(o.schema)}`);
  for (const k of ['sampleRate', 'frames', 'windowSec', 'peak', 'peakDb'] as const) {
    if (typeof o[k] !== 'number' || !Number.isFinite(o[k] as number)) {
      throw new FeaturesError(`${k} must be a finite number`);
    }
  }
  if (o.lufs !== null && (typeof o.lufs !== 'number' || !Number.isFinite(o.lufs))) {
    throw new FeaturesError('lufs must be a finite number or null');
  }
  const bands = o.bandsDb as Record<string, unknown> | null;
  if (typeof bands !== 'object' || bands === null) throw new FeaturesError('bandsDb missing');
  const bandsDb = {} as Record<string, number[]>;
  let len = -1;
  for (const b of BANDS) {
    const a = bands[b.name];
    if (!isNumArray(a)) throw new FeaturesError(`bandsDb.${b.name} must be a number array`);
    if (len >= 0 && a.length !== len) throw new FeaturesError('band arrays have unequal length');
    len = a.length;
    bandsDb[b.name] = a;
  }
  if (!isNumArray(o.onset)) throw new FeaturesError('onset must be a number array');
  if (typeof o.pcmSha256 !== 'string' || !/^[0-9a-f]{64}$/.test(o.pcmSha256)) {
    throw new FeaturesError('pcmSha256 must be 64 lowercase hex chars');
  }
  return {
    schema: 1,
    sampleRate: o.sampleRate as number,
    frames: o.frames as number,
    windowSec: o.windowSec as number,
    peak: o.peak as number,
    peakDb: o.peakDb as number,
    lufs: o.lufs as number | null,
    bandsDb: bandsDb as Features['bandsDb'],
    onset: o.onset,
    pcmSha256: o.pcmSha256,
  };
}

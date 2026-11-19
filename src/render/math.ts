/**
 * The V8 math reference's evaluation (page kind `math`): `evalMathGroup` runs one group of
 * inputs through `Math.*`, `x | 0` or one of superdough's distortion algorithms and returns the
 * results as f64 bit strings. Pure; `math-worklet.ts` calls it inside an AudioWorklet (so the
 * numbers come from the V8 the processors run in), and `bun test` covers it under JSC.
 */

import type { MathGroupInput, MathGroupOutput } from './protocol';

/** The number with the f64 bit pattern `hex` (16 hex digits, big endian). */
export function fromBitsHex(hex: string): number {
  if (!/^[0-9a-f]{16}$/.test(hex)) throw new RangeError(`not an f64 bit string: ${hex}`);
  const view = new DataView(new ArrayBuffer(8));
  view.setUint32(0, parseInt(hex.slice(0, 8), 16));
  view.setUint32(4, parseInt(hex.slice(8), 16));
  return view.getFloat64(0);
}

/** The f64 bit pattern of `x` as 16 lowercase hex digits (big endian). */
export function toBitsHex(x: number): string {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, x);
  const hi = view.getUint32(0).toString(16).padStart(8, '0');
  return hi + view.getUint32(4).toString(16).padStart(8, '0');
}

// ---- copied verbatim from vendor/strudel/packages/superdough (util.mjs `clamp`, helpers.mjs
// "Saturation curves"), with parameter types added; tests/engine-v8-math.test.ts compares the
// text with the vendored files.

// verbatim:util
const clamp = (num: number, min: number, max: number) => Math.min(Math.max(num, min), max);
// end verbatim:util

// verbatim:helpers

const __squash = (x: number) => x / (1 + x); // [0, inf) to [0, 1)
const _mod = (n: number, m: number) => ((n % m) + m) % m;

const _scurve = (x: number, k: number) => ((1 + k) * x) / (1 + k * Math.abs(x));
const _soft = (x: number, k: number) => Math.tanh(x * (1 + k));
const _hard = (x: number, k: number) => clamp((1 + k) * x, -1, 1);

const _fold = (x: number, k: number) => {
  // Closed form folding for audio rate
  let y = (1 + 0.5 * k) * x;
  const window = _mod(y + 1, 4);
  return 1 - Math.abs(window - 2);
};

const _sineFold = (x: number, k: number) => Math.sin((Math.PI / 2) * _fold(x, k));

const _cubic = (x: number, k: number) => {
  const t = __squash(Math.log1p(k));
  const cubic = (x - (t / 3) * x * x * x) / (1 - t / 3); // normalized to go from (-1, 1)
  return _soft(cubic, k);
};

const _diode = (x: number, k: number, asym = false) => {
  const g = 1 + 2 * k; // gain
  const t = __squash(Math.log1p(k));
  const bias = 0.07 * t;
  const pos = _soft(x + bias, 2 * k);
  const neg = _soft(asym ? bias : -x + bias, 2 * k);
  const y = pos - neg;
  // We divide by the derivative at 0 so that the distortion is roughly
  // the identity map near 0 => small values are preserved and undistorted
  const sech = 1 / Math.cosh(g * bias);
  const sech2 = sech * sech; // derivative of soft (i.e. tanh) is sech^2
  const denom = Math.max(1e-8, (asym ? 1 : 2) * g * sech2); // g from chain rule; 2 if both pos/neg have x
  return _soft(y / denom, k);
};

const _asym = (x: number, k: number) => _diode(x, k, true);

const _chebyshev = (x: number, k: number) => {
  const kl = 10 * Math.log1p(k);
  let tnm1 = 1;
  let tnm2 = x;
  let tn;
  let y = 0;
  for (let i = 1; i < 64; i++) {
    if (i < 2) {
      // Already set inital conditions
      y += i == 0 ? tnm1 : tnm2;
      continue;
    }
    tn = 2 * x * tnm1 - tnm2; // https://en.wikipedia.org/wiki/Chebyshev_polynomials#Recurrence_definition
    tnm2 = tnm1;
    tnm1 = tn;
    if (i % 2 === 0) {
      y += Math.min((1.3 * kl) / i, 2) * tn;
    }
  }
  // Soft clip
  return _soft(y, kl / 20);
};

export const distortionAlgorithms = {
  scurve: _scurve,
  soft: _soft,
  hard: _hard,
  cubic: _cubic,
  diode: _diode,
  asym: _asym,
  fold: _fold,
  sinefold: _sineFold,
  chebyshev: _chebyshev,
};
// end verbatim:helpers

/** The one-argument functions of the V8 math reference (`MathFn` is derived from these keys). */
export const UNARY = {
  sin: Math.sin,
  cos: Math.cos,
  tanh: Math.tanh,
  exp: Math.exp,
  expm1: Math.expm1,
  log1p: Math.log1p,
  cosh: Math.cosh,
  round: Math.round,
  toInt32: (x: number) => x | 0,
  tan: Math.tan,
  asin: Math.asin,
  acos: Math.acos,
  atan: Math.atan,
  sinh: Math.sinh,
  asinh: Math.asinh,
  acosh: Math.acosh,
  atanh: Math.atanh,
  log: Math.log,
  log2: Math.log2,
  log10: Math.log10,
  cbrt: Math.cbrt,
} satisfies Record<string, (x: number) => number>;

/** The binary `Math` functions and the `**` operator (`powOp`); `hypot` takes exactly two. */
export const BINARY = {
  pow: Math.pow,
  atan2: Math.atan2,
  hypot: (x: number, y: number) => Math.hypot(x, y),
  powOp: (x: number, y: number) => x ** y,
} satisfies Record<string, (x: number, y: number) => number>;

/** Whether a group of `fn` takes `ys`: the binary functions and the `distort:<name>` groups. */
export function isBinaryFn(fn: string): boolean {
  return Object.hasOwn(BINARY, fn) || fn.startsWith('distort:');
}

/** Evaluates one group. Throws on a malformed group (unknown function, `ys` present or absent
 *  against the function, unequal lengths, a bad bit string). */
export function evalMathGroup(g: MathGroupInput): MathGroupOutput {
  const xs = g.xs.map(fromBitsHex);
  const binary = isBinaryFn(g.fn);
  if (binary !== (g.ys !== null)) {
    throw new RangeError(`${g.id}: ys must be ${binary ? 'given' : 'null'} for ${g.fn}`);
  }
  let out: number[];
  if (binary) {
    const ys = g.ys!.map(fromBitsHex);
    if (ys.length !== xs.length) throw new RangeError(`${g.id}: xs and ys differ in length`);
    let f: (x: number, y: number) => number;
    if (Object.hasOwn(BINARY, g.fn)) f = BINARY[g.fn as keyof typeof BINARY];
    else {
      const name = g.fn.slice('distort:'.length);
      const algo = (distortionAlgorithms as Record<string, (x: number, k: number) => number>)[name];
      if (!algo) throw new RangeError(`${g.id}: unknown distortion algorithm ${name}`);
      f = algo;
    }
    out = xs.map((x, i) => f(x, ys[i]!));
  } else {
    const f = Object.hasOwn(UNARY, g.fn) ? UNARY[g.fn as keyof typeof UNARY] : undefined;
    if (!f) throw new RangeError(`${g.id}: unknown function ${g.fn}`);
    out = xs.map((x) => f(x));
  }
  return { id: g.id, out: out.map(toBitsHex) };
}

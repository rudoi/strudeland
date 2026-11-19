/**
 * The V8 math reference's input grid (`crates/strudeland-engine/tests/refs/v8-math.json`, case
 * `v8-math` of `engine:refs`): every group the engine's `jsmath` (and, from the shaper stage, its
 * distortion algorithms) is checked against bit for bit. Pure and PRNG-free: the grid is a
 * function of this file only, and `inputsSha256` is the provenance that makes the committed
 * reference stale when a group changes. The ranges are the ones the worklet processors see (LFO
 * phases and shapes, distortion arguments, `10^x`, `2^x`, `2^(x/12)` maps); a processor that later
 * reads outside them adds a group here.
 */

import type { DistortName, MathFn, MathGroupInput } from '../../src/render/protocol';
import { sha256 } from '../sha256';
import { fromBitsHex as fromBits, toBitsHex as f64Bits } from '../../src/render/math';

/** Points of every `linspace`. */
export const GRID_POINTS = 509;

/** `GRID_POINTS` values `lo + ((hi - lo) * i) / (GRID_POINTS - 1)`, `i = 0..GRID_POINTS-1`. */
export function linspace(lo: number, hi: number): number[] {
  return Array.from({ length: GRID_POINTS }, (_, i) => lo + ((hi - lo) * i) / (GRID_POINTS - 1));
}

/** `[x, -x]` for each `x`, in order. */
const pm = (...xs: number[]): number[] => xs.flatMap((x) => [x, -x]);

/** The next f64 above / below a finite `x` (bit pattern plus or minus one; from a zero, the
 *  smallest subnormal of that direction). */
export function nextUp(x: number): number {
  if (x === 0) return 5e-324;
  const b = BigInt(`0x${f64Bits(x)}`);
  return fromBits((x > 0 ? b + 1n : b - 1n).toString(16).padStart(16, '0'));
}
export function nextDown(x: number): number {
  if (x === 0) return -5e-324;
  const b = BigInt(`0x${f64Bits(x)}`);
  return fromBits((x > 0 ? b - 1n : b + 1n).toString(16).padStart(16, '0'));
}

const PI = Math.PI;
const TRIG_SPECIALS = [
  0,
  -0,
  ...pm(PI / 6, PI / 4, PI / 3, PI / 2, PI, (3 * PI) / 2, 2 * PI, 1e-300, 1e-8),
  1e6,
  1e10,
  ...pm(Infinity),
  NaN,
];

const ASIN_ACOS_SPECIALS = [0, -0, ...pm(1, 2, 1e-300, Infinity), nextDown(1), nextUp(-1), NaN];

/** The distortion algorithms in `distortionAlgorithms` order. */
export const DISTORT_NAMES: readonly DistortName[] = [
  'scurve',
  'soft',
  'hard',
  'cubic',
  'diode',
  'asym',
  'fold',
  'sinefold',
  'chebyshev',
];

/** The `d` values of the distortion groups and `k = Math.expm1(d)` as V8 evaluates it, as bit
 *  strings: the processor computes `k` in the worklet, and a host libm's `expm1` may differ in the
 *  last bit, so the grid (and its hash) must not depend on the generating machine. The bits were
 *  read from Chromium's `Math.expm1` (the `expm1` row of the reference agrees with them); a test
 *  checks each against the host's `Math.expm1(d)` within one ulp. */
export const DISTORT_K: readonly { d: number; k: string }[] = [
  { d: 0, k: '0000000000000000' },
  { d: 0.5, k: '3fe4c2531c3c0d38' },
  { d: 1, k: '3ffb7e151628aed3' },
  { d: 2, k: '40198e64b8d4ddae' },
  { d: 3.2, k: '40378853e624b794' },
  { d: 10, k: '40d5825dcf950560' },
];
/** Points of a distortion group per `d`: `x_i = -2 + i / 15`, `i = 0..60`. */
export const DISTORT_POINTS = 61;

const hex = (xs: number[]): string[] => xs.map(f64Bits);

function unary(id: string, fn: MathFn, xs: number[]): MathGroupInput {
  return { id, fn, xs: hex(xs), ys: null };
}

function binary(id: string, fn: MathFn, pairs: [number, number][]): MathGroupInput {
  return { id, fn, xs: hex(pairs.map((p) => p[0])), ys: hex(pairs.map((p) => p[1])) };
}

const withY = (xs: number[], y: number): [number, number][] => xs.map((x) => [x, y]);
const withX = (x: number, ys: number[]): [number, number][] => ys.map((y) => [x, y]);

function roundHalves(): number[] {
  const out: number[] = [];
  for (let k = -64; k <= 64; k++) {
    const h = k + 0.5;
    out.push(h, nextUp(h), nextDown(h));
  }
  return [
    ...out,
    -0,
    -0.5,
    -0.4999999999999999,
    0.49999999999999994,
    2 ** 52 + 0.5,
    NaN,
    ...pm(Infinity),
  ];
}

const TOINT32_EDGES = [
  ...pm(2 ** 31, 2 ** 31 + 1, 2 ** 31 - 1, 2 ** 32, 2 ** 32 + 1),
  2 ** 53,
  ...pm(0.5, 1.9, 2147483647.9),
  1e21,
  -1e21,
  0,
  -0,
  NaN,
  ...pm(Infinity),
];

const POW_SPECIAL: [number, number][] = [
  [0, 0],
  [-0, 3],
  [0, -1],
  [-8, 1 / 3],
  [NaN, 0],
  [1, NaN],
  [2, Infinity],
  [2, -Infinity],
  [0.5, Infinity],
  [-2, 2],
  [-2, 3],
  [Infinity, 0.5],
  [-Infinity, 3],
  [1e-300, 2],
  [1e300, 2],
];

/** The `Math` functions the headless host runs natively that are neither IEEE-exact nor
 *  overridden by `jsmath`: each has a `<fn>-main` and a `<fn>-special` group. */
export const VERIFICATION_SET: readonly MathFn[] = [
  'tan',
  'asin',
  'acos',
  'atan',
  'atan2',
  'sinh',
  'asinh',
  'acosh',
  'atanh',
  'log',
  'log2',
  'log10',
  'cbrt',
  'hypot',
];

/** The `atan2`/`hypot` pair grid's axis: `-4..4` in steps of one, then `±0.5`, `±0.25`. */
const PAIR_AXIS = [-4, -3, -2, -1, 0, 1, 2, 3, 4, ...pm(0.5, 0.25)];
/** Every pair `(a, b)` with `a, b` in `PAIR_AXIS`, `a` outer. */
const PAIR_GRID: [number, number][] = PAIR_AXIS.flatMap((a) =>
  PAIR_AXIS.map((b): [number, number] => [a, b]),
);
/** `2^k` for `k = -20..20` (exact) and `10^k` for `k = -10..10`, parsed from `1e<k>` literals
 *  (correctly rounded, so the grid does not depend on a host's `**`). */
const POWERS_OF_TWO = Array.from({ length: 41 }, (_, i) => 2 ** (i - 20));
const POWERS_OF_TEN = Array.from({ length: 21 }, (_, i) => Number(`1e${i - 10}`));
/** The `**` exponents of `smootherStep` (`signal.mjs`), each over `linspace(0, 1)`. */
const POW_OP_EXPONENTS = [5, 4, 3] as const;

/** Every group of the V8 math reference, in the reference's order. */
export function v8MathCases(): MathGroupInput[] {
  const lfoPairs: [number, number][] = [0.25, 0.5, 1, 1.5, 2, 3].flatMap((y) =>
    Array.from({ length: 85 }, (_, i): [number, number] => [i / 84, y]),
  );
  const pow2x12: [number, number][] = Array.from({ length: 241 }, (_, j) => [2, (j - 120) / 12]);
  const distortX = Array.from({ length: DISTORT_POINTS }, (_, i) => -2 + i / 15);
  const distort = DISTORT_NAMES.map((name): MathGroupInput => ({
    id: `distort:${name}`,
    fn: `distort:${name}`,
    xs: DISTORT_K.flatMap(() => hex(distortX)),
    ys: DISTORT_K.flatMap(({ k }) => distortX.map(() => k)),
  }));
  return [
    unary('sin-turns', 'sin', [...linspace(-8 * PI, 8 * PI), ...TRIG_SPECIALS]),
    unary('cos-turns', 'cos', [...linspace(-8 * PI, 8 * PI), ...TRIG_SPECIALS]),
    unary('sin-wide', 'sin', [...linspace(-1000, 1000), ...TRIG_SPECIALS]),
    unary('cos-wide', 'cos', [...linspace(-1000, 1000), ...TRIG_SPECIALS]),
    unary('tanh-main', 'tanh', [
      ...linspace(-20, 20),
      0,
      -0,
      ...pm(1e-10, 0.5, 1, 22, 40, 710, Infinity),
      NaN,
    ]),
    unary('exp-main', 'exp', [
      ...linspace(-50, 50),
      0,
      -0,
      ...pm(1),
      709,
      709.78,
      -745,
      -746,
      ...pm(Infinity),
      NaN,
    ]),
    unary('expm1-main', 'expm1', [
      ...linspace(-10, 10),
      0,
      -0,
      ...pm(1e-10, 1e-300, Infinity),
      NaN,
    ]),
    unary('log1p-main', 'log1p', [
      ...linspace(-0.99, 100),
      -1,
      0,
      -0,
      1e-10,
      1e-300,
      1e10,
      -2,
      Infinity,
      NaN,
    ]),
    unary('cosh-main', 'cosh', [...linspace(-30, 30), 0, -0, ...pm(710, 711, Infinity), NaN]),
    binary('pow-sq', 'pow', withY(linspace(-4, 4), 2)),
    binary('pow-4', 'pow', withY(linspace(-4, 4), 4)),
    binary('pow-half', 'pow', withY(linspace(0, 4), 0.5)),
    binary('pow-1p5', 'pow', withY(linspace(0, 4), 1.5)),
    binary('pow-one', 'pow', withY(linspace(-4, 4), 1)),
    binary('pow-lfo', 'pow', lfoPairs),
    binary('pow-10x', 'pow', withX(10, linspace(-3, 3))),
    binary('pow-2x', 'pow', withX(2, linspace(-8, 8))),
    binary('pow-2x12', 'pow', pow2x12),
    binary('pow-special', 'pow', POW_SPECIAL),
    unary('round-halves', 'round', roundHalves()),
    unary('toint32-edges', 'toInt32', TOINT32_EDGES),
    ...distort,
    // the Math functions the host runs natively (not jsmath), judged against Chromium there
    unary('tan-main', 'tan', linspace(-8 * PI, 8 * PI)),
    unary('tan-special', 'tan', TRIG_SPECIALS),
    unary('asin-main', 'asin', linspace(-1, 1)),
    unary('asin-special', 'asin', ASIN_ACOS_SPECIALS),
    unary('acos-main', 'acos', linspace(-1, 1)),
    unary('acos-special', 'acos', ASIN_ACOS_SPECIALS),
    unary('atan-main', 'atan', linspace(-20, 20)),
    unary('atan-special', 'atan', [0, -0, ...pm(1, 1e-300, 1e10, 1e300, Infinity), NaN]),
    binary('atan2-main', 'atan2', PAIR_GRID),
    binary('atan2-special', 'atan2', [
      [0, 0],
      [0, -0],
      [-0, 0],
      [-0, -0],
      [Infinity, Infinity],
      [Infinity, -Infinity],
      [-Infinity, Infinity],
      [-Infinity, -Infinity],
      [1, Infinity],
      [1, -Infinity],
      [NaN, 1],
    ]),
    unary('sinh-main', 'sinh', linspace(-30, 30)),
    unary('sinh-special', 'sinh', [0, -0, ...pm(1e-10, 710, 711, Infinity), NaN]),
    unary('asinh-main', 'asinh', linspace(-30, 30)),
    unary('asinh-special', 'asinh', [0, -0, ...pm(1e-10, 1e10, 1e300, Infinity), NaN]),
    unary('acosh-main', 'acosh', linspace(1, 30)),
    unary('acosh-special', 'acosh', [1, nextUp(1), 0.5, 0, 1e300, Infinity, NaN]),
    unary('atanh-main', 'atanh', linspace(-0.999, 0.999)),
    unary('atanh-special', 'atanh', [0, -0, ...pm(1, 2, 1e-10), nextDown(1), NaN]),
    unary('log-main', 'log', linspace(0.001, 100)),
    unary('log-special', 'log', [0, -0, 1, Math.E, 1e-300, 1e300, -1, Infinity, NaN]),
    unary('log2-main', 'log2', linspace(0.001, 100)),
    unary('log2-special', 'log2', [0, -0, 1, -1, Infinity, ...POWERS_OF_TWO, NaN]),
    unary('log10-main', 'log10', linspace(0.001, 100)),
    unary('log10-special', 'log10', [0, -0, 1, -1, Infinity, ...POWERS_OF_TEN, NaN]),
    unary('cbrt-main', 'cbrt', linspace(-1000, 1000)),
    unary('cbrt-special', 'cbrt', [0, -0, ...pm(1e-300, 1e300, 8, 27, 64, Infinity), NaN]),
    binary('hypot-main', 'hypot', [...PAIR_GRID, [3, 4], [5, 12]]),
    binary('hypot-special', 'hypot', [
      [0, 0],
      [-0, -0],
      [1e200, 1e200],
      [1e-200, 1e-200],
      [Infinity, NaN],
      [NaN, 1],
    ]),
    ...POW_OP_EXPONENTS.map((y) => binary(`powOp-${y}`, 'powOp', withY(linspace(0, 1), y))),
    binary('powOp-sq', 'powOp', withY(linspace(-4, 4), 2)),
    binary('powOp-special', 'powOp', POW_SPECIAL),
  ];
}

/** The reference's input hash: `sha256(JSON.stringify(v8MathCases()))`. */
export const inputsSha256 = (groups: MathGroupInput[] = v8MathCases()): string =>
  sha256(JSON.stringify(groups));

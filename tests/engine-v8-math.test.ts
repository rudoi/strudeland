import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  DISTORT_K,
  DISTORT_NAMES,
  DISTORT_POINTS,
  GRID_POINTS,
  inputsSha256,
  linspace,
  nextDown,
  nextUp,
  v8MathCases,
  VERIFICATION_SET,
} from '../scripts/engine/v8-math-cases';
import { MATH_FILE, REFS_DIR } from '../scripts/engine/refs-lib';
import { evalMathGroup, fromBitsHex, isBinaryFn, toBitsHex } from '../src/render/math';
import type { MathFn, MathGroupInput } from '../src/render/protocol';

const ROOT = resolve(import.meta.dir, '..');
const HEX = /^[0-9a-f]{16}$/;

const ALL_FNS: MathFn[] = [
  'sin',
  'cos',
  'tanh',
  'exp',
  'expm1',
  'log1p',
  'cosh',
  'pow',
  'round',
  'toInt32',
  ...DISTORT_NAMES.map((n) => `distort:${n}` as MathFn),
  ...VERIFICATION_SET,
  'powOp',
];

const GROUP_IDS = [
  'sin-turns',
  'cos-turns',
  'sin-wide',
  'cos-wide',
  'tanh-main',
  'exp-main',
  'expm1-main',
  'log1p-main',
  'cosh-main',
  'pow-sq',
  'pow-4',
  'pow-half',
  'pow-1p5',
  'pow-one',
  'pow-lfo',
  'pow-10x',
  'pow-2x',
  'pow-2x12',
  'pow-special',
  'round-halves',
  'toint32-edges',
  ...DISTORT_NAMES.map((n) => `distort:${n}`),
  ...VERIFICATION_SET.flatMap((fn) => [`${fn}-main`, `${fn}-special`]),
  'powOp-5',
  'powOp-4',
  'powOp-3',
  'powOp-sq',
  'powOp-special',
];

const group = (fn: MathFn, xs: number[], ys: number[] | null = null): MathGroupInput => ({
  id: 'g',
  fn,
  xs: xs.map(toBitsHex),
  ys: ys && ys.map(toBitsHex),
});
const evalTo = (g: MathGroupInput): number[] => evalMathGroup(g).out.map(fromBitsHex);

describe('bit strings', () => {
  test('round trip, big endian, lowercase', () => {
    expect(toBitsHex(1)).toBe('3ff0000000000000');
    expect(toBitsHex(-0)).toBe('8000000000000000');
    expect(toBitsHex(Infinity)).toBe('7ff0000000000000');
    for (const x of [0, -0, 1.5, -2.25e-308, 5e-324, Number.MAX_VALUE, -Infinity]) {
      expect(Object.is(fromBitsHex(toBitsHex(x)), x), String(x)).toBe(true);
    }
    expect(fromBitsHex(toBitsHex(NaN))).toBeNaN();
  });

  test('malformed bit strings throw', () => {
    expect(() => fromBitsHex('3FF0000000000000')).toThrow(/not an f64 bit string/);
    expect(() => fromBitsHex('3ff')).toThrow(/not an f64 bit string/);
    expect(() => fromBitsHex('zzzzzzzzzzzzzzzz')).toThrow(/not an f64 bit string/);
  });
});

describe('the grid', () => {
  const groups = v8MathCases();

  test('is deterministic', () => {
    expect(JSON.stringify(v8MathCases())).toBe(JSON.stringify(groups));
  });

  test('has exactly the table of groups, in order, with unique ids', () => {
    expect(groups.map((g) => g.id)).toEqual(GROUP_IDS);
    expect(new Set(groups.map((g) => g.id)).size).toBe(groups.length);
  });

  test('covers every MathFn', () => {
    expect([...new Set(groups.map((g) => g.fn))].sort()).toEqual([...ALL_FNS].sort());
  });

  test('every number is 16 lowercase hex digits; ys exactly for pow and distort, equal length', () => {
    for (const g of groups) {
      expect(g.xs.length, g.id).toBeGreaterThan(0);
      for (const h of g.xs) expect(h, g.id).toMatch(HEX);
      expect(g.ys !== null, g.id).toBe(isBinaryFn(g.fn));
      if (g.ys) {
        expect(g.ys.length, g.id).toBe(g.xs.length);
        for (const h of g.ys) expect(h, g.id).toMatch(HEX);
      }
    }
  });

  test('linspace has GRID_POINTS values from lo to hi', () => {
    const l = linspace(-4, 4);
    expect(l.length).toBe(GRID_POINTS);
    expect(l[0]).toBe(-4);
    expect(l[GRID_POINTS - 1]).toBe(4);
    expect(l[254]).toBe(0);
  });

  test('group sizes', () => {
    const size = (id: string) => groups.find((g) => g.id === id)!.xs.length;
    expect(size('pow-lfo')).toBe(6 * 85);
    expect(size('pow-2x12')).toBe(241);
    expect(size('pow-special')).toBe(15);
    expect(size('round-halves')).toBe(129 * 3 + 8);
    expect(size('toint32-edges')).toBe(24);
    for (const n of DISTORT_NAMES) expect(size(`distort:${n}`)).toBe(6 * DISTORT_POINTS);
  });

  test('the native-function and ** groups: sizes and inputs', () => {
    const find = (id: string) => groups.find((g) => g.id === id)!;
    const size = (id: string) => find(id).xs.length;
    const xs = (id: string) => find(id).xs.map(fromBitsHex);
    for (const fn of VERIFICATION_SET) {
      if (fn === 'atan2' || fn === 'hypot') continue;
      expect(size(`${fn}-main`), fn).toBe(GRID_POINTS);
    }
    expect(size('atan2-main')).toBe(13 * 13);
    expect(size('hypot-main')).toBe(13 * 13 + 2);
    expect(size('atan2-special')).toBe(11);
    expect(size('hypot-special')).toBe(6);
    expect(size('tan-special')).toBe(25);
    expect(size('asin-special')).toBe(13);
    expect(size('acosh-special')).toBe(7);
    expect(size('log2-special')).toBe(5 + 41 + 1);
    expect(size('log10-special')).toBe(5 + 21 + 1);
    for (const id of ['powOp-5', 'powOp-4', 'powOp-3', 'powOp-sq']) {
      expect(size(id), id).toBe(GRID_POINTS);
    }
    for (const [id, y] of [
      ['powOp-5', 5],
      ['powOp-4', 4],
      ['powOp-3', 3],
      ['powOp-sq', 2],
    ] as const) {
      expect(new Set(find(id).ys!.map(fromBitsHex)), id).toEqual(new Set([y]));
    }
    expect(xs('powOp-5')[0]).toBe(0);
    expect(xs('powOp-5')[GRID_POINTS - 1]).toBe(1);
    expect(xs('powOp-sq')[0]).toBe(-4);
    expect(find('powOp-special').xs).toEqual(find('pow-special').xs);
    expect(find('powOp-special').ys).toEqual(find('pow-special').ys);
    // the pair grid is (y, x) over -4..4 and ±0.5, ±0.25, first argument outer
    const a2 = find('atan2-main');
    expect([fromBitsHex(a2.xs[0]!), fromBitsHex(a2.ys![0]!)]).toEqual([-4, -4]);
    expect([fromBitsHex(a2.xs[12]!), fromBitsHex(a2.ys![12]!)]).toEqual([-4, -0.25]);
    expect(find('hypot-main').xs.slice(0, 169)).toEqual(a2.xs);
    expect(xs('hypot-main').slice(169)).toEqual([3, 5]);
    expect(xs('log2-special').slice(5, 46)).toEqual(
      Array.from({ length: 41 }, (_, i) => 2 ** (i - 20)),
    );
    expect(xs('log10-special').slice(5, 26)).toEqual(
      Array.from({ length: 21 }, (_, i) => Number(`1e${i - 10}`)),
    );
    expect(xs('asin-special')).toContain(nextDown(1));
    expect(xs('asin-special')).toContain(nextUp(-1));
    expect(xs('acosh-special')[1]).toBe(nextUp(1));
    // every special group except tan's ends with NaN (TRIG_SPECIALS already holds it)
    for (const fn of VERIFICATION_SET) {
      const last = xs(`${fn}-special`).at(-1)!;
      const lastY = find(`${fn}-special`).ys?.map(fromBitsHex).at(-1);
      expect(Number.isNaN(last) || Number.isNaN(lastY ?? 0), fn).toBe(true);
    }
  });

  test('nextUp and nextDown step one ulp toward +inf and -inf', () => {
    expect(nextUp(1)).toBe(1 + 2 ** -52);
    expect(nextDown(1)).toBe(1 - 2 ** -53);
    expect(nextUp(-1)).toBe(-1 + 2 ** -53);
    expect(nextDown(-1)).toBe(-1 - 2 ** -52);
    expect(nextUp(0)).toBe(5e-324);
    expect(nextDown(-0)).toBe(-5e-324);
  });

  test('the distort k bits are expm1(d) within one ulp of this host', () => {
    for (const { d, k } of DISTORT_K) {
      const v = fromBitsHex(k);
      const host = Math.expm1(d);
      expect(Math.abs(v - host) <= Math.abs(nextUp(host) - host), `d=${d}`).toBe(true);
    }
    expect(DISTORT_K.map((x) => x.d)).toEqual([0, 0.5, 1, 2, 3.2, 10]);
  });

  test('inputsSha256 is stable and changes with a group', () => {
    expect(inputsSha256()).toBe(inputsSha256(v8MathCases()));
    expect(inputsSha256()).toMatch(/^[0-9a-f]{64}$/);
    const changed = v8MathCases();
    changed[0] = { ...changed[0]!, xs: changed[0]!.xs.slice(1) };
    expect(inputsSha256(changed)).not.toBe(inputsSha256());
  });
});

describe('isBinaryFn', () => {
  test('true for pow, atan2, hypot, powOp and the distortion groups', () => {
    for (const fn of ['pow', 'atan2', 'hypot', 'powOp', 'distort:x', 'distort:soft']) {
      expect(isBinaryFn(fn), fn).toBe(true);
    }
  });

  test('false for the unary functions, unknown names and Object.prototype keys', () => {
    for (const fn of ['sin', 'tan', 'log', 'round', 'toInt32', 'nope', '', 'toString', 'distort']) {
      expect(isBinaryFn(fn), fn).toBe(false);
    }
  });

  test('agrees with the grid: ys exactly for the binary functions', () => {
    for (const g of v8MathCases()) expect(g.ys !== null, g.id).toBe(isBinaryFn(g.fn));
  });
});

describe('evalMathGroup', () => {
  test('Math.round takes halves toward +inf and keeps -0', () => {
    const out = evalTo(group('round', [-2.5, -0.5, 2.5, 0.49999999999999994, -0.4]));
    expect(out[0]).toBe(-2);
    expect(Object.is(out[1], -0)).toBe(true);
    expect(out[2]).toBe(3);
    expect(Object.is(out[3], 0)).toBe(true);
    expect(Object.is(out[4], -0)).toBe(true);
  });

  test('toInt32 wraps modulo 2^32 and maps NaN to 0', () => {
    expect(evalTo(group('toInt32', [2 ** 32 + 1, -1.5, 2 ** 31, NaN]))).toEqual([
      1,
      -1,
      -(2 ** 31),
      0,
    ]);
  });

  test('pow(x, 1) is x; pow takes its ys', () => {
    const xs = [-3.5, 0.1, 7];
    expect(evalTo(group('pow', xs, [1, 1, 1]))).toEqual(xs);
    expect(evalTo(group('pow', [2], [10]))).toEqual([1024]);
  });

  test('the unary functions and the ids pass through', () => {
    const r = evalMathGroup({ ...group('exp', [0]), id: 'exp-x' });
    expect(r.id).toBe('exp-x');
    expect(r.out).toEqual([toBitsHex(1)]);
    expect(evalTo(group('cosh', [0]))).toEqual([1]);
  });

  test('the native unary functions', () => {
    expect(evalTo(group('tan', [0, Math.PI / 4]))).toEqual([0, Math.tan(Math.PI / 4)]);
    expect(evalTo(group('asin', [1, 2]))).toEqual([Math.PI / 2, NaN]);
    expect(evalTo(group('acos', [1, -1]))).toEqual([0, Math.PI]);
    expect(evalTo(group('atan', [Infinity]))).toEqual([Math.PI / 2]);
    expect(evalTo(group('sinh', [0, -Infinity]))).toEqual([0, -Infinity]);
    expect(evalTo(group('asinh', [0]))).toEqual([0]);
    expect(evalTo(group('acosh', [1, 0.5]))).toEqual([0, NaN]);
    expect(evalTo(group('atanh', [1, -1, 2]))).toEqual([Infinity, -Infinity, NaN]);
    expect(evalTo(group('log', [1, 0, -1]))).toEqual([0, -Infinity, NaN]);
    expect(evalTo(group('log2', [1024, 2 ** -20]))).toEqual([10, -20]);
    expect(evalTo(group('log10', [1000, 1]))).toEqual([3, 0]);
    expect(evalTo(group('cbrt', [27, -8]))).toEqual([3, -2]);
    expect(Object.is(evalTo(group('cbrt', [-0]))[0], -0)).toBe(true);
    expect(Object.is(evalTo(group('atanh', [-0]))[0], -0)).toBe(true);
  });

  test('atan2 takes (y, x) as (xs, ys) and keeps the signs of zeros', () => {
    expect(evalTo(group('atan2', [1, 0], [0, -1]))).toEqual([Math.PI / 2, Math.PI]);
    const z = evalTo(group('atan2', [0, -0, 0, -0], [0, 0, -0, -0]));
    expect(Object.is(z[0], 0)).toBe(true);
    expect(Object.is(z[1], -0)).toBe(true);
    expect(z[2]).toBe(Math.PI);
    expect(z[3]).toBe(-Math.PI);
  });

  test('hypot of two arguments; Infinity wins over NaN', () => {
    expect(evalTo(group('hypot', [3, 5, -0], [4, 12, -0]))).toEqual([5, 13, 0]);
    expect(Object.is(evalTo(group('hypot', [-0], [-0]))[0], 0)).toBe(true);
    expect(evalTo(group('hypot', [Infinity, NaN], [NaN, 1]))).toEqual([Infinity, NaN]);
  });

  test('powOp is the ** operator', () => {
    expect(evalTo(group('powOp', [2, 0.5, -2, NaN, 1], [10, 3, 3, 0, NaN]))).toEqual([
      1024,
      0.125,
      -8,
      1,
      NaN,
    ]);
  });

  test('distortion algorithms map 0 to 0 and use their k', () => {
    for (const n of DISTORT_NAMES) {
      const [y] = evalTo(group(`distort:${n}` as MathFn, [0], [2]));
      expect(Math.abs(y!), n).toBeLessThan(1e-12);
    }
    expect(evalTo(group('distort:hard', [0.25, 0.9], [3, 3]))).toEqual([1, 1]);
    expect(evalTo(group('distort:scurve', [1], [0]))).toEqual([1]);
  });

  test('malformed groups throw', () => {
    expect(() => evalMathGroup(group('pow', [1]))).toThrow(/ys must be given/);
    expect(() => evalMathGroup(group('sin', [1], [1]))).toThrow(/ys must be null/);
    expect(() => evalMathGroup(group('pow', [1, 2], [1]))).toThrow(/differ in length/);
    expect(() => evalMathGroup(group('nope' as MathFn, [1]))).toThrow(/unknown function/);
    expect(() => evalMathGroup(group('toString' as MathFn, [1]))).toThrow(/unknown function/);
    expect(() => evalMathGroup(group('atan2', [1]))).toThrow(/ys must be given for atan2/);
    expect(() => evalMathGroup(group('hypot', [1]))).toThrow(/ys must be given for hypot/);
    expect(() => evalMathGroup(group('powOp', [1]))).toThrow(/ys must be given for powOp/);
    expect(() => evalMathGroup(group('tan', [1], [1]))).toThrow(/ys must be null for tan/);
    expect(() => evalMathGroup(group('powOp', [1, 2], [1]))).toThrow(/differ in length/);
    expect(() => evalMathGroup(group('distort:nope' as MathFn, [1], [1]))).toThrow(
      /unknown distortion algorithm/,
    );
  });
});

describe('verbatim copies', () => {
  const between = (text: string, from: string, to: string): string => {
    const a = text.indexOf(from);
    const b = text.indexOf(to, a + from.length);
    if (a < 0 || b < 0) throw new Error(`markers ${from} .. ${to} not found`);
    return text.slice(a + from.length, b);
  };
  const ours = readFileSync(join(ROOT, 'src/render/math.ts'), 'utf8');
  const untyped = (s: string) => s.replace(/: number/g, '').trim();

  test('the distortion algorithms are helpers.mjs "Saturation curves" verbatim', () => {
    const vendored = readFileSync(
      join(ROOT, 'vendor/strudel/packages/superdough/helpers.mjs'),
      'utf8',
    );
    const block = between(vendored, '// Saturation curves\n', '\nconst _algoNames');
    expect(untyped(between(ours, '// verbatim:helpers\n', '// end verbatim:helpers'))).toBe(
      block.trim(),
    );
  });

  test('clamp is util.mjs verbatim', () => {
    const util = readFileSync(join(ROOT, 'vendor/strudel/packages/superdough/util.mjs'), 'utf8');
    const line = util.split('\n').find((l) => l.startsWith('export const clamp = '))!;
    expect(untyped(between(ours, '// verbatim:util\n', '// end verbatim:util'))).toBe(
      line.replace(/^export /, ''),
    );
  });
});

describe('the committed reference', () => {
  test(`${MATH_FILE} has exactly the grid's ids, functions and inputs`, () => {
    const ref = JSON.parse(readFileSync(join(ROOT, REFS_DIR, MATH_FILE), 'utf8')) as {
      schema: number;
      groups: (MathGroupInput & { out: string[] })[];
    };
    expect(ref.schema).toBe(1);
    const grid = v8MathCases();
    expect(ref.groups.map((g) => g.id)).toEqual(grid.map((g) => g.id));
    for (const [i, g] of grid.entries()) {
      const r = ref.groups[i]!;
      expect(r.fn, g.id).toBe(g.fn);
      expect(r.xs, g.id).toEqual(g.xs);
      expect(r.ys, g.id).toEqual(g.ys);
      expect(r.out.length, g.id).toBe(g.xs.length);
    }
  });
});

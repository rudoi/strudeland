// The vendored Fraction fast path (PATCHES.md `fast-fraction`) against fraction.js's BigInt
// implementation: every result must be exactly equal (value, fields, `show`, `valueOf`, errors).
import { describe, expect, test } from 'bun:test';
import BigFractionClass from 'fraction.js';
import fraction, { gcd, isFraction, lcm } from '@strudel/core/fraction.mjs';

type Big = BigFractionClass;
// fraction.js's types declare only the constructor; the module is called as a function too.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const BigFraction = BigFractionClass as unknown as (...args: any[]) => Big;
// The vendored module is untyped JS; its values are used structurally here.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Fast = any;

const CASES = 5000;

// mulberry32, as in superdough's rng.mjs.
const prng = (seed: number) => () => {
  seed |= 0;
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const isBig = (x: unknown): x is Big => x instanceof BigFractionClass;
const bigShow = (b: Big) => b.s * b.n + '/' + b.d;

/** The fast result and the BigInt result agree exactly. */
function same(fast: Fast, big: unknown, label: string) {
  if (isBig(big)) {
    expect(isFraction(fast), label).toBe(true);
    expect(fast.show(), label).toBe(bigShow(big));
    expect(Object.is(fast.valueOf(), big.valueOf()), label).toBe(true);
    const fits =
      big.n <= BigInt(Number.MAX_SAFE_INTEGER) && big.d <= BigInt(Number.MAX_SAFE_INTEGER);
    expect(fast.big === null, label).toBe(fits);
    if (fits) {
      expect([fast.s, fast.n, fast.d], label).toEqual([
        Number(big.s),
        Number(big.n),
        Number(big.d),
      ]);
    }
  } else {
    expect(fast, label).toBe(big);
  }
}

/** Both throw the same error (constructor and message), or neither does. */
function outcome(f: () => unknown): { ok: true; value: unknown } | { ok: false; error: string } {
  try {
    return { ok: true, value: f() };
  } catch (e) {
    return { ok: false, error: `${(e as Error).constructor.name}: ${(e as Error).message}` };
  }
}
function check(fastF: () => unknown, bigF: () => unknown, label: string) {
  const a = outcome(fastF);
  const b = outcome(bigF);
  expect(a.ok, `${label}: ${a.ok ? '' : a.error} / ${b.ok ? '' : b.error}`).toBe(b.ok);
  if (a.ok && b.ok) {
    same(a.value, b.value, label);
  } else if (!a.ok && !b.ok) {
    expect(a.error, label).toBe(b.error);
  }
}

// BigInt references for the methods fraction.mjs defines itself.
const ref = {
  lt: (x: Big, y: Big) => x.compare(y) < 0,
  gt: (x: Big, y: Big) => x.compare(y) > 0,
  lte: (x: Big, y: Big) => x.compare(y) <= 0,
  gte: (x: Big, y: Big) => x.compare(y) >= 0,
  eq: (x: Big, y: Big) => x.compare(y) == 0,
  ne: (x: Big, y: Big) => x.compare(y) != 0,
  max: (x: Big, y: Big) => (x.compare(y) > 0 ? x : y),
  min: (x: Big, y: Big) => (x.compare(y) < 0 ? x : y),
};

const BINARY = ['add', 'sub', 'mul', 'div', 'mod', 'gcd', 'lcm', 'compare', 'equals'] as const;
const DERIVED = ['lt', 'gt', 'lte', 'gte', 'eq', 'ne', 'max', 'min'] as const;
const UNARY = ['floor', 'abs', 'inverse', 'valueOf'] as const;

/** Runs every method on a pair given as fraction.js inputs (strings, numbers). */
function checkPair(a: string | number, b: string | number, label: string) {
  const x = fraction(a);
  const y = fraction(b);
  const X = BigFraction(a);
  const Y = BigFraction(b);
  for (const m of BINARY) {
    check(
      () => x[m](y),
      () => X[m](Y),
      `${label} ${m} (${a}, ${b})`,
    );
    // The argument as the raw input, parsed by the method.
    check(
      () => x[m](b),
      () => X[m](b),
      `${label} ${m} (${a}, raw ${b})`,
    );
  }
  for (const m of DERIVED) {
    check(
      () => x[m](y),
      () => ref[m](X, Y),
      `${label} ${m} (${a}, ${b})`,
    );
  }
  for (const m of UNARY) {
    check(
      () => x[m](),
      () => X[m](),
      `${label} ${m} (${a})`,
    );
  }
  check(
    () => x.sam(),
    () => X.floor(),
    `${label} sam (${a})`,
  );
  check(
    () => x.nextSam(),
    () => X.floor().add(1),
    `${label} nextSam (${a})`,
  );
  check(
    () => x.cyclePos(),
    () => X.sub(X.floor()),
    `${label} cyclePos (${a})`,
  );
  check(
    () => x.mod(),
    () => X.mod(),
    `${label} mod() (${a})`,
  );
  check(
    () => x.or(y),
    () => (X.compare(0) == 0 ? Y : X),
    `${label} or (${a}, ${b})`,
  );
  expect(x.toFraction()).toBe(X.toFraction());
  expect(x.toFraction(true)).toBe(X.toFraction(true));
  expect(x.toString()).toBe(X.toString());
}

const randRational = (rand: () => number, max: number) => {
  const n = Math.floor(rand() * (max + 1));
  const d = 1 + Math.floor(rand() * max);
  return `${rand() < 0.5 ? '-' : ''}${n}/${d}`;
};

describe('fast Fraction equals fraction.js', () => {
  test('random rationals up to 1e4', () => {
    const rand = prng(1);
    for (let i = 0; i < CASES; i++) {
      checkPair(randRational(rand, 1e4), randRational(rand, 1e4), `#${i}`);
    }
  });

  test('cps-derived values: 17/72 and multiples of 1/1440', () => {
    const rand = prng(2);
    const pick = () => {
      const r = rand();
      if (r < 0.2) return '17/72';
      if (r < 0.3) return '72/17';
      return `${Math.floor(rand() * 200000) - 50000}/1440`;
    };
    for (let i = 0; i < CASES; i++) {
      checkPair(pick(), pick(), `#${i}`);
    }
  });

  test('integers, zero, one and negatives', () => {
    const values = [
      0,
      1,
      -1,
      2,
      -2,
      7,
      -7,
      '7/2',
      '-7/2',
      '1/2',
      '-1/2',
      '3/4',
      '-3/4',
      12,
      -12,
      '0/5',
    ];
    for (const a of values) {
      for (const b of values) {
        checkPair(a, b, 'edge');
      }
    }
  });

  test('mod and floor on negatives follow fraction.js', () => {
    expect(fraction('-7/2').mod(2).show()).toBe('-3/2');
    expect(fraction('7/2').mod(-2).show()).toBe('3/2');
    expect(fraction('-7/2').floor().show()).toBe('-4/1');
    expect(fraction('-4').floor().show()).toBe('-4/1');
    expect(fraction(0).show()).toBe('0/1');
    expect(fraction(-0).show()).toBe('0/1');
    expect(Object.is(fraction(-0).valueOf(), 0)).toBe(true);
    expect(fraction('-0').show()).toBe('0/1');
  });

  test('floats and strings are parsed by fraction.js', () => {
    const rand = prng(3);
    for (let i = 0; i < CASES; i++) {
      const f = (rand() - 0.5) * 100;
      check(
        () => fraction(f),
        () => BigFraction(f),
        `float ${f}`,
      );
      check(
        () => fraction('1/3').add(f),
        () => BigFraction('1/3').add(f),
        `add float ${f}`,
      );
      check(
        () => fraction('5/7').mul(f),
        () => BigFraction('5/7').mul(f),
        `mul float ${f}`,
      );
    }
    for (const s of ['0.5', '1.25', '-3.(3)', '2 1/2', '10:4', '+5', '1_000']) {
      check(
        () => fraction(s),
        () => BigFraction(s),
        s,
      );
    }
    // Objects, arrays and fraction.js values.
    check(
      () => fraction({ n: 3, d: 6, s: -1 }),
      () => BigFraction({ n: 3, d: 6, s: -1 }),
      'object',
    );
    check(
      () => fraction([4, 6]),
      () => BigFraction([4, 6]),
      'array',
    );
    check(
      () => fraction(BigFraction('5/10')),
      () => BigFraction('5/10'),
      'BigFraction',
    );
    // Undefined is zero for both, the two-argument form, and floor with places.
    check(
      () => fraction(undefined),
      () => BigFraction(undefined),
      'undefined',
    );
    check(
      () => fraction(1).add(undefined),
      () => BigFraction(1).add(undefined as never),
      'add undefined',
    );
    check(
      () => fraction('1/3').add(1, 6),
      () => BigFraction('1/3').add(1, 6),
      'two-argument add',
    );
    check(
      () => fraction('-7/3').floor(2),
      () => BigFraction('-7/3').floor(2),
      'floor places',
    );
  });

  test('a FastFraction returns itself', () => {
    const x = fraction('3/4');
    expect(fraction(x)).toBe(x);
  });
});

describe('maybe variants, maximum and the gcd/lcm exports', () => {
  test('maybe variants with undefined', () => {
    const x = fraction('3/4');
    for (const m of ['mulmaybe', 'divmaybe', 'addmaybe', 'submaybe']) {
      expect(x[m](undefined)).toBeUndefined();
    }
    same(x.mulmaybe('2/3'), BigFraction('3/4').mul('2/3'), 'mulmaybe');
    same(x.divmaybe('2/3'), BigFraction('3/4').div('2/3'), 'divmaybe');
    same(x.addmaybe('2/3'), BigFraction('3/4').add('2/3'), 'addmaybe');
    same(x.submaybe('2/3'), BigFraction('3/4').sub('2/3'), 'submaybe');
  });

  test('maximum', () => {
    same(fraction('1/3').maximum('1/2', 2, '-5'), BigFraction(2), 'maximum');
    same(fraction('7').maximum('1/2', 2), BigFraction(7), 'maximum self');
    same(fraction('7').maximum(), BigFraction(7), 'maximum none');
  });

  test('gcd and lcm exports with undefineds', () => {
    expect(gcd()).toBeUndefined();
    expect(gcd(undefined, undefined)).toBeUndefined();
    expect(lcm()).toBeUndefined();
    expect(lcm(undefined)).toBeUndefined();
    same(
      gcd(fraction('1/4'), undefined, fraction('1/6')),
      BigFraction(1).gcd('1/4').gcd('1/6'),
      'gcd',
    );
    same(lcm(fraction('1/4'), undefined, fraction('1/6')), BigFraction('1/6').lcm('1/4'), 'lcm');
    same(lcm(fraction(0), fraction(0)), BigFraction(0).lcm(0), 'lcm zeros');
  });
});

describe('overflow boundaries', () => {
  const P53 = 2n ** 53n;
  const P60 = 2n ** 60n;
  const values = [
    `${P53 - 1n}`,
    `${P53}`,
    `${P53 + 1n}`,
    `-${P53 + 1n}`,
    `${P53 - 1n}/${P53 - 2n}`,
    `1/${P53 - 1n}`,
    `1/${P53}`,
    `${P60 + 1n}/${P60}`,
    `-1/${P60}`,
    `${3n ** 33n}/${2n ** 52n}`,
    `${2n ** 30n + 1n}/${2n ** 29n + 3n}`,
    `${2n ** 27n}/${3n ** 17n}`,
    '1',
    '-1',
    '0',
    '17/72',
    '1/1440',
  ];

  test('every pair, fast and big operands mixed', () => {
    for (const a of values) {
      for (const b of values) {
        checkPair(a, b, 'boundary');
      }
    }
  });

  test('a result fits again after a fallback', () => {
    const x = fraction(`${P60 + 1n}/${P60}`);
    expect(x.big).not.toBeNull();
    const y = x.sub(1);
    expect(y.big).not.toBeNull();
    const z = y.mul(`${P60}`);
    expect(z.big).toBeNull();
    expect(z.show()).toBe('1/1');
    const w = x.sub(`1/${P60}`);
    expect(w.big).toBeNull();
    expect(w.show()).toBe('1/1');
  });

  test('products that overflow take the BigInt route', () => {
    const rand = prng(4);
    const big = () => {
      const n = BigInt(Math.floor(rand() * 2 ** 31)) * BigInt(Math.floor(rand() * 2 ** 22) + 1);
      const d = BigInt(Math.floor(rand() * 2 ** 31) + 1) * BigInt(Math.floor(rand() * 2 ** 22) + 1);
      return `${rand() < 0.5 ? '-' : ''}${n}/${d}`;
    };
    for (let i = 0; i < 1000; i++) {
      checkPair(big(), big(), `#${i}`);
    }
  });
});

describe('errors match fraction.js', () => {
  test('construction', () => {
    for (const v of [NaN, Infinity, -Infinity, 'a/0', '1/0', 'x', {}]) {
      check(
        () => fraction(v),
        () => BigFraction(v as never),
        `fraction(${String(v)})`,
      );
    }
  });

  test('division by zero', () => {
    for (const x of ['3/4', '0', '-2', `${2n ** 60n + 1n}/${2n ** 60n}`]) {
      check(
        () => fraction(x).div(0),
        () => BigFraction(x).div(0),
        `${x} div 0`,
      );
      check(
        () => fraction(x).mod(0),
        () => BigFraction(x).mod(0),
        `${x} mod 0`,
      );
      check(
        () => fraction(x).div(NaN),
        () => BigFraction(x).div(NaN),
        `${x} div NaN`,
      );
    }
    check(
      () => fraction(0).inverse(),
      () => BigFraction(0).inverse(),
      'inverse 0',
    );
  });

  test('unimplemented methods are not there', () => {
    expect(fraction(1).ceil).toBeUndefined();
    expect(fraction(1).simplify).toBeUndefined();
  });
});

describe('isFraction', () => {
  test('true for both classes', () => {
    expect(isFraction(fraction('1/3'))).toBe(true);
    expect(isFraction(fraction(`${2n ** 60n}/3`))).toBe(true);
    expect(isFraction(BigFraction('1/3'))).toBe(true);
  });
  test('false for other values', () => {
    for (const v of [{}, 1, '1', [], null, undefined]) {
      expect(isFraction(v)).toBe(false);
    }
  });
  test('_original is fraction.js', () => {
    expect(fraction._original).toBe(BigFraction);
  });
});

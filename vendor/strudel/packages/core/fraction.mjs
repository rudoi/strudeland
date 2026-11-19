/*
fraction.mjs - <short description TODO>
Copyright (C) 2022 Strudel contributors - see <https://codeberg.org/uzu/strudel/src/branch/main/packages/core/fraction.mjs>
This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU Affero General Public License for more details. You should have received a copy of the GNU Affero General Public License along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

import BigFraction from 'fraction.js';
import { TimeSpan } from './timespan.mjs';
import { removeUndefineds } from './util.mjs';

// strudeland patch (fast-fraction): a number-backed rational with a BigInt fallback.
// fraction.js 5.3.4 does all arithmetic in BigInt, which made pattern querying slow. A
// FastFraction holds `s` (-1 or 1; 1 for zero, as fraction.js), `n` and `d` (safe integers in
// lowest terms, d >= 1) and `big` (null, or the BigFraction when n or d does not fit). Every
// operation gives exactly fraction.js's result: an intermediate above Number.MAX_SAFE_INTEGER,
// a `big` operand, an argument that is not a FastFraction or a safe integer, and every error
// case compute with fraction.js and wrap its result.

const MAX_SAFE = Number.MAX_SAFE_INTEGER;
const MAX_SAFE_BIG = BigInt(MAX_SAFE);

// Euclid on non-negative safe integers (fraction.js's gcd: gcd(0, b) = b, gcd(a, 0) = a).
const igcd = (a, b) => {
  while (b !== 0) {
    const t = a % b;
    a = b;
    b = t;
  }
  return a;
};

class FastFraction {
  constructor(s, n, d, big) {
    this.s = s;
    this.n = n;
    this.d = d;
    this.big = big;
  }
}

const fast = (s, n, d) => new FastFraction(s, n, d, null);
const zero = () => new FastFraction(1, 0, 1, null);

// A signed numerator `sn` over `d` (both safe, d > 0), reduced.
const norm = (sn, d) => {
  if (sn === 0) {
    return zero();
  }
  const s = sn < 0 ? -1 : 1;
  const n = sn < 0 ? -sn : sn;
  if (d === 1) {
    return fast(s, n, 1);
  }
  const g = igcd(n, d);
  return g === 1 ? fast(s, n, d) : fast(s, n / g, d / g);
};

const wrap = (b) => {
  const s = Number(b.s);
  if (b.n <= MAX_SAFE_BIG && b.d <= MAX_SAFE_BIG) {
    return fast(s, Number(b.n), Number(b.d));
  }
  // Out of range: the numbers are approximations kept only so the fields stay numbers.
  return new FastFraction(s, Number(b.n), Number(b.d), b);
};

const toBig = (f) => (f.big !== null ? f.big : BigFraction({ s: f.s, n: f.n, d: f.d }));

// A method argument as fraction.js parses it.
const operand = (a, b) => {
  if (b === undefined) {
    if (a instanceof FastFraction) {
      return a;
    }
    if (typeof a === 'number' && Number.isSafeInteger(a)) {
      return a === 0 ? zero() : fast(a < 0 ? -1 : 1, Math.abs(a), 1);
    }
  }
  return wrap(BigFraction(a, b));
};

// The operation computed by fraction.js; a Fraction result is wrapped, others returned as is.
const viaBig = (x, name, o) => {
  const r = o === undefined ? toBig(x)[name]() : toBig(x)[name](toBig(o));
  return r instanceof BigFraction ? wrap(r) : r;
};

// fraction.js's gcd and lcm use the argument's numerator and denominator as parsed, before
// reducing ('2/4' is not '1/2' there), so only arguments known to be in lowest terms take the
// fast route.
const reduced = (a, b) =>
  b === undefined && (a instanceof FastFraction || (typeof a === 'number' && Number.isSafeInteger(a)));

const safe = (x) => x <= MAX_SAFE && x >= -MAX_SAFE;

const P = FastFraction.prototype;

// x + sign * o (sign 1 for add, -1 for sub), as fraction.js's `name` computes it.
const addSigned = (x, o, sign, name) => {
  if (x.big !== null || o.big !== null) {
    return viaBig(x, name, o);
  }
  if (x.d === o.d) {
    const sn = x.s * x.n + sign * o.s * o.n;
    return safe(sn) ? norm(sn, x.d) : viaBig(x, name, o);
  }
  const g = igcd(x.d, o.d);
  const a = x.s * x.n * (o.d / g);
  const b = sign * o.s * o.n * (x.d / g);
  const d = (x.d / g) * o.d;
  const sn = a + b;
  if (safe(a) && safe(b) && safe(sn) && d <= MAX_SAFE) {
    return norm(sn, d);
  }
  return viaBig(x, name, o);
};

// x * (on / od) with sign o.s (o itself for mul, its inverse for div), as fraction.js's `name`
// computes it. Inputs are in lowest terms, so cross-cancelling leaves the product in lowest terms.
const mulParts = (x, o, on, od, name) => {
  if (x.n === 0 || on === 0) {
    return zero();
  }
  const g1 = igcd(x.n, od);
  const g2 = igcd(on, x.d);
  const n = (x.n / g1) * (on / g2);
  const d = (x.d / g2) * (od / g1);
  if (n <= MAX_SAFE && d <= MAX_SAFE) {
    return fast(x.s * o.s, n, d);
  }
  return viaBig(x, name, o);
};

P.add = function (a, b) {
  return addSigned(this, operand(a, b), 1, 'add');
};

P.sub = function (a, b) {
  return addSigned(this, operand(a, b), -1, 'sub');
};

P.mul = function (a, b) {
  const o = operand(a, b);
  if (this.big !== null || o.big !== null) {
    return viaBig(this, 'mul', o);
  }
  return mulParts(this, o, o.n, o.d, 'mul');
};

P.div = function (a, b) {
  const o = operand(a, b);
  if (this.big !== null || o.big !== null || o.n === 0) {
    return viaBig(this, 'div', o);
  }
  return mulParts(this, o, o.d, o.n, 'div');
};

// fraction.js: the remainder takes the dividend's sign; the divisor's sign is ignored.
P.mod = function (a, b) {
  if (a === undefined) {
    if (this.big !== null) {
      return wrap(this.big.mod());
    }
    const r = this.n % this.d;
    return r === 0 ? zero() : fast(this.s, r, 1);
  }
  const o = operand(a, b);
  if (this.big !== null || o.big !== null || o.n === 0) {
    return viaBig(this, 'mod', o);
  }
  const x = o.d * this.n;
  const y = o.n * this.d;
  const d = o.d * this.d;
  if (x <= MAX_SAFE && y <= MAX_SAFE && d <= MAX_SAFE) {
    return norm(this.s * (x % y), d);
  }
  return viaBig(this, 'mod', o);
};

P.floor = function (places) {
  if (this.big !== null || places) {
    return wrap(toBig(this).floor(places));
  }
  const r = this.n % this.d;
  const q = (this.n - r) / this.d;
  if (this.s < 0) {
    return fast(-1, r > 0 ? q + 1 : q, 1);
  }
  return q === 0 ? zero() : fast(1, q, 1);
};

P.abs = function () {
  if (this.big !== null) {
    return wrap(this.big.abs());
  }
  return fast(1, this.n, this.d);
};

P.inverse = function () {
  if (this.big !== null || this.n === 0) {
    return viaBig(this, 'inverse');
  }
  return fast(this.s, this.d, this.n);
};

// gcd(a/b, c/d) = gcd(a, c) / lcm(b, d), the value fraction.js computes.
P.gcd = function (a, b) {
  if (!reduced(a, b)) {
    return wrap(toBig(this).gcd(a, b));
  }
  const o = operand(a, b);
  if (this.big !== null || o.big !== null) {
    return viaBig(this, 'gcd', o);
  }
  const n = igcd(o.n, this.n);
  const d = (o.d / igcd(o.d, this.d)) * this.d;
  return d <= MAX_SAFE ? norm(n, d) : viaBig(this, 'gcd', o);
};

// lcm(a/b, c/d) = lcm(a, c) / gcd(b, d), the value fraction.js computes.
P.lcm = function (a, b) {
  if (!reduced(a, b)) {
    return wrap(toBig(this).lcm(a, b));
  }
  const o = operand(a, b);
  if (this.big !== null || o.big !== null) {
    return viaBig(this, 'lcm', o);
  }
  if (this.n === 0 || o.n === 0) {
    return zero();
  }
  const n = (o.n / igcd(o.n, this.n)) * this.n;
  return n <= MAX_SAFE ? norm(n, igcd(o.d, this.d)) : viaBig(this, 'lcm', o);
};

P.compare = function (a, b) {
  const o = operand(a, b);
  if (this.big !== null || o.big !== null) {
    return viaBig(this, 'compare', o);
  }
  let t;
  if (this.d === o.d) {
    t = this.s * this.n - o.s * o.n;
  } else {
    const x = this.s * this.n * o.d;
    const y = o.s * o.n * this.d;
    if (!safe(x) || !safe(y)) {
      return viaBig(this, 'compare', o);
    }
    t = x - y;
  }
  return t > 0 ? 1 : t < 0 ? -1 : 0;
};

// Both sides are in lowest terms with zero as 0/1 (s = 1), so equal values have equal fields.
P.equals = function (a, b) {
  const o = operand(a, b);
  if (this.big !== null || o.big !== null) {
    return viaBig(this, 'equals', o);
  }
  return this.n === o.n && this.d === o.d && this.s === o.s;
};

P.valueOf = function () {
  if (this.big !== null) {
    return this.big.valueOf();
  }
  return (this.s * this.n) / this.d;
};

P.toFraction = function (showMixed) {
  return toBig(this).toFraction(showMixed);
};

P.toString = function (dec) {
  return toBig(this).toString(dec);
};

P.show = function (/* excludeWhole = false */) {
  // return this.toFraction(excludeWhole);
  if (this.big !== null) {
    return this.big.s * this.big.n + '/' + this.big.d;
  }
  return this.s * this.n + '/' + this.d;
};

// Returns the start of the cycle.
P.sam = function () {
  return this.floor();
};

// Returns the start of the next cycle.
P.nextSam = function () {
  return this.sam().add(1);
};

// Returns a TimeSpan representing the begin and end of the Time value's cycle
P.wholeCycle = function () {
  return new TimeSpan(this.sam(), this.nextSam());
};

// The position of a time value relative to the start of its cycle.
P.cyclePos = function () {
  return this.sub(this.sam());
};

P.lt = function (other) {
  return this.compare(other) < 0;
};

P.gt = function (other) {
  return this.compare(other) > 0;
};

P.lte = function (other) {
  return this.compare(other) <= 0;
};

P.gte = function (other) {
  return this.compare(other) >= 0;
};

P.eq = function (other) {
  return this.compare(other) == 0;
};

P.ne = function (other) {
  return this.compare(other) != 0;
};

P.max = function (other) {
  return this.gt(other) ? this : other;
};

P.maximum = function (...others) {
  others = others.map((x) => fraction(x));
  return others.reduce((max, other) => other.max(max), this);
};

P.min = function (other) {
  return this.lt(other) ? this : other;
};

P.mulmaybe = function (other) {
  return other !== undefined ? this.mul(other) : undefined;
};

P.divmaybe = function (other) {
  return other !== undefined ? this.div(other) : undefined;
};

P.addmaybe = function (other) {
  return other !== undefined ? this.add(other) : undefined;
};

P.submaybe = function (other) {
  return other !== undefined ? this.sub(other) : undefined;
};

P.or = function (other) {
  return this.eq(0) ? other : this;
};

const fraction = (n) => {
  if (n instanceof FastFraction) {
    return n;
  }
  if (typeof n === 'number' && Number.isSafeInteger(n)) {
    return n === 0 ? zero() : fast(n < 0 ? -1 : 1, Math.abs(n), 1);
  }
  /*
    https://github.com/infusion/Fraction.js/#doubles
    „If you pass a double as it is, Fraction.js will perform a number analysis based on Farey Sequences."
    „If you want to keep the number as it is, convert it to a string, as the string parser will not perform any further observations“
    
    -> those farey sequences turn out to make pattern querying ~20 times slower! always use strings!
    -> still, some optimizations could be done: .mul .div .add .sub calls still use numbers
    */
  // n = String(n); // this is actually faster but imprecise...
  return wrap(BigFraction(n));
};

export const gcd = (...fractions) => {
  fractions = removeUndefineds(fractions);
  if (fractions.length === 0) {
    return undefined;
  }

  return fractions.reduce((gcd, fraction) => gcd.gcd(fraction), fraction(1));
};

export const lcm = (...fractions) => {
  fractions = removeUndefineds(fractions);
  if (fractions.length === 0) {
    return undefined;
  }
  const x = fractions.pop();
  return fractions.reduce(
    (lcm, fraction) => (lcm === undefined || fraction === undefined ? undefined : lcm.lcm(fraction)),
    x,
  );
};

export const isFraction = (x) => x instanceof FastFraction || x instanceof BigFraction;

fraction._original = BigFraction;

export default fraction;

// "If you concern performance, cache Fraction.js objects and pass arrays/objects.“
// -> tested memoized version, but it's slower than unmemoized, even with repeated evaluation
/* const memo = {};
const memoizedFraction = (n) => {
  if (typeof n === 'number') {
    n = String(n);
  }
  if (memo[n] !== undefined) {
    return memo[n];
  }
  memo[n] = Fraction(n);
  return memo[n];
}; */

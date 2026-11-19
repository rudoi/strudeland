/**
 * The `String(number)` differential's pure parts: the fixed input set `S` (200 000 doubles,
 * hex-encoded bit patterns) and the comparison helpers. `string-number.probe.ts` evaluates the
 * set in Chromium; `crates/strudeland-host/tests/string_number.rs` evaluates it in the embedded V8
 * and compares. No Playwright here (tested by `tests/host-string-number.test.ts`).
 */

/** The generator's seed (fixed: the set must not change between runs). */
export const SEED = 0x5eed_0012;

/** The composition of `S`. */
export const COUNTS = {
  /** Uniformly random bit patterns that are finite. */
  bits: 100_000,
  /** `k / 2^j`. */
  binary: 25_000,
  /** `k / 10^j`. */
  decimal: 25_000,
  /** Products of two random 3-digit decimals (`a / 100 × b / 100`, `a, b` in 100..999). */
  products: 50_000,
} as const;

export const TOTAL = COUNTS.bits + COUNTS.binary + COUNTS.decimal + COUNTS.products;

export type Kind = keyof typeof COUNTS;

/** mulberry32: a small seeded 32-bit generator. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (t ^ (t >>> 14)) >>> 0;
  };
}

const view = new DataView(new ArrayBuffer(8));

/** The 16-digit lowercase hex bit pattern of `x`. */
export function numberToHex(x: number): string {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}

/** The double whose bit pattern is the hex string. */
export function hexToNumber(hex: string): number {
  if (!/^[0-9a-f]{16}$/.test(hex)) throw new Error(`bad hex double ${hex}`);
  view.setBigUint64(0, BigInt(`0x${hex}`));
  return view.getFloat64(0);
}

/** The set `S`, in a fixed order (bits, binary, decimal, products), with each value's kind. */
export function generateInputs(seed = SEED): { hex: string[]; kinds: Kind[] } {
  const next = mulberry32(seed);
  const below = (n: number) => next() % n;
  const hex: string[] = [];
  const kinds: Kind[] = [];
  const push = (h: string, kind: Kind) => {
    hex.push(h);
    kinds.push(kind);
  };
  while (hex.length < COUNTS.bits) {
    const h = next().toString(16).padStart(8, '0') + next().toString(16).padStart(8, '0');
    // Exponent all ones: an infinity or a NaN, not finite.
    if ((parseInt(h.slice(0, 3), 16) & 0x7ff) === 0x7ff) continue;
    push(h, 'bits');
  }
  for (let i = 0; i < COUNTS.binary; i++) {
    const k = 1 + below(1_000_000);
    const j = below(64);
    push(numberToHex((next() & 1 ? -1 : 1) * (k / 2 ** j)), 'binary');
  }
  for (let i = 0; i < COUNTS.decimal; i++) {
    const k = 1 + below(1_000_000);
    const j = below(23);
    push(numberToHex((next() & 1 ? -1 : 1) * (k / 10 ** j)), 'decimal');
  }
  for (let i = 0; i < COUNTS.products; i++) {
    const a = 100 + below(900);
    const b = 100 + below(900);
    push(numberToHex((a / 100) * (b / 100)), 'products');
  }
  return { hex, kinds };
}

/** One input whose outputs differ. */
export interface Difference {
  index: number;
  hex: string;
  expected: string;
  actual: string;
}

/** The comparison of two output lists over the same inputs. */
export interface Comparison {
  count: number;
  lengthMismatch: boolean;
  first: Difference[];
}

/** Compares `actual` with `expected` index by index; keeps the first `limit` differences. */
export function compareOutputs(
  hex: readonly string[],
  expected: readonly string[],
  actual: readonly string[],
  limit = 20,
): Comparison {
  const n = Math.min(expected.length, actual.length);
  const first: Difference[] = [];
  let count = 0;
  for (let i = 0; i < n; i++) {
    if (expected[i] === actual[i]) continue;
    count++;
    if (first.length < limit) {
      first.push({ index: i, hex: hex[i] ?? '?', expected: expected[i], actual: actual[i] });
    }
  }
  const lengthMismatch = expected.length !== actual.length || hex.length !== expected.length;
  return { count, lengthMismatch, first };
}

/** The comparison as text: the count, then one line per listed difference. */
export function formatDifferences(label: string, c: Comparison): string {
  const lines = [
    `${label}: ${c.count} difference${c.count === 1 ? '' : 's'}${c.lengthMismatch ? ' (length mismatch)' : ''}`,
  ];
  for (const d of c.first) {
    lines.push(`  #${d.index} 0x${d.hex}: expected ${d.expected} got ${d.actual}`);
  }
  return lines.join('\n');
}

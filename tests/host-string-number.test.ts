import { describe, expect, test } from 'bun:test';
import {
  compareOutputs,
  COUNTS,
  formatDifferences,
  generateInputs,
  hexToNumber,
  mulberry32,
  numberToHex,
  SEED,
  TOTAL,
} from '../scripts/host/string-number-lib';

const set = generateInputs();

describe('the input set S', () => {
  test('has 200 000 values in the planned composition', () => {
    expect(TOTAL).toBe(200_000);
    expect(set.hex.length).toBe(TOTAL);
    expect(set.kinds.length).toBe(TOTAL);
    const counts: Record<string, number> = {};
    for (const k of set.kinds) counts[k] = (counts[k] ?? 0) + 1;
    expect(counts).toEqual({ ...COUNTS });
    expect(COUNTS.binary + COUNTS.decimal).toBe(50_000);
  });

  test('is deterministic for the fixed seed and differs for another', () => {
    const again = generateInputs(SEED);
    expect(again.hex).toEqual(set.hex);
    expect(generateInputs(SEED + 1).hex.slice(0, 10)).not.toEqual(set.hex.slice(0, 10));
  });

  test('pins its first values', () => {
    // A change to the generator changes the set; the evidence names the seed and these.
    expect(set.hex.slice(0, 3)).toEqual([
      '953e445250d4a277',
      'd66a88d8115e54e1',
      '0d9251c4b0d1de02',
    ]);
    expect(set.hex.slice(100_000, 100_002)).toEqual(['c0ef436000000000', 'bef0bd6c00000000']);
    expect(set.hex.slice(150_000, 150_002)).toEqual(['4030162b6ae7d567', '40343a29c779a6b4']);
    expect(set.hex.every((h) => /^[0-9a-f]{16}$/.test(h))).toBe(true);
    const a = mulberry32(1);
    const b = mulberry32(1);
    expect([a(), a()]).toEqual([b(), b()]);
  });

  test('every value is finite and the kinds have their shapes', () => {
    for (let i = 0; i < TOTAL; i++) {
      const x = hexToNumber(set.hex[i]);
      expect(Number.isFinite(x)).toBe(true);
      if (set.kinds[i] === 'products') {
        expect(x).toBeGreaterThanOrEqual(1);
        expect(x).toBeLessThan(100);
      }
    }
  });

  test('the random bit patterns are distinct enough to cover both signs and tiny values', () => {
    const bits = set.hex.slice(0, COUNTS.bits).map(hexToNumber);
    expect(bits.some((x) => x < 0)).toBe(true);
    expect(bits.some((x) => Math.abs(x) < 1e-300)).toBe(true);
    expect(bits.some((x) => Math.abs(x) > 1e300)).toBe(true);
    expect(new Set(set.hex).size).toBeGreaterThan(190_000);
  });
});

describe('hex round trip', () => {
  test.each([0, -0, 1, -1.5, 0.1, 5e-324, Number.MAX_VALUE, 1e21])('%p', (x) => {
    expect(Object.is(hexToNumber(numberToHex(x)), x)).toBe(true);
  });

  test('pins known bit patterns', () => {
    expect(numberToHex(1)).toBe('3ff0000000000000');
    expect(numberToHex(-0)).toBe('8000000000000000');
    expect(hexToNumber('4000000000000000')).toBe(2);
  });

  test('a bad hex string throws', () => {
    expect(() => hexToNumber('3ff')).toThrow('bad hex double');
    expect(() => hexToNumber('3FF0000000000000')).toThrow('bad hex double');
  });
});

describe('compareOutputs and formatDifferences', () => {
  const hex = ['3ff0000000000000', '4000000000000000', '4008000000000000'];

  test('equal outputs have no differences', () => {
    const c = compareOutputs(hex, ['1', '2', '3'], ['1', '2', '3']);
    expect(c).toEqual({ count: 0, lengthMismatch: false, first: [] });
    expect(formatDifferences('String', c)).toBe('String: 0 differences');
  });

  test('one difference is listed with its input', () => {
    const c = compareOutputs(hex, ['1', '2', '3'], ['1', '2.0', '3']);
    expect(c.count).toBe(1);
    expect(c.first).toEqual([{ index: 1, hex: hex[1], expected: '2', actual: '2.0' }]);
    expect(formatDifferences('String', c)).toBe(
      'String: 1 difference\n  #1 0x4000000000000000: expected 2 got 2.0',
    );
  });

  test('a length mismatch is flagged and the common prefix compared', () => {
    const c = compareOutputs(hex, ['1', '2', '3'], ['1', 'x']);
    expect(c.lengthMismatch).toBe(true);
    expect(c.count).toBe(1);
    expect(formatDifferences('JSON', c)).toStartWith('JSON: 1 difference (length mismatch)');
  });

  test('only the first `limit` differences are kept, all are counted', () => {
    const many = Array.from({ length: 30 }, (_, i) => String(i));
    const c = compareOutputs(
      many,
      many,
      many.map((s) => `${s}!`),
      20,
    );
    expect(c.count).toBe(30);
    expect(c.first.length).toBe(20);
    expect(c.first[19].index).toBe(19);
  });
});

import { describe, expect, test } from 'bun:test';
import {
  DEFAULTS,
  UsageError,
  buildReport,
  formatTable,
  parseArgs,
  summarize,
} from '../scripts/bench/scheduler';

describe('parseArgs', () => {
  test('defaults', () => {
    expect(parseArgs([])).toEqual(DEFAULTS);
    expect(DEFAULTS.seconds).toBe(30);
    expect(DEFAULTS.section).toBe('orchestra');
  });
  test('every flag', () => {
    expect(
      parseArgs(['--song', 'a.js', '--seconds', '2.5', '--section', 'x', '--out', 'o.json']),
    ).toEqual({
      song: 'a.js',
      seconds: 2.5,
      section: 'x',
      out: 'o.json',
    });
  });
  test('bad arguments are usage errors', () => {
    for (const argv of [
      ['--seconds', '0'],
      ['--seconds', 'x'],
      ['--seconds', '-1'],
      ['--bogus'],
      ['--song'],
    ]) {
      expect(() => parseArgs(argv)).toThrow(UsageError);
    }
  });
});

describe('statistics', () => {
  test('a run: dsp nearestRank p50 and the max of the fires, ms per simulated second', () => {
    // sorted[round(0.5 * 3)] of [1, 2, 3, 4]
    expect(summarize([4, 1, 3, 2], 2, 7)).toEqual({
      msPerFire: { p50: 3, max: 4 },
      msPerSecond: 5,
      haps: 7,
    });
  });
});

describe('report', () => {
  test('shape', () => {
    const runs = [10, 30, 20].map((ms) => summarize([ms, ms], 2, 5));
    expect(runs[0]).toEqual({ msPerFire: { p50: 10, max: 10 }, msPerSecond: 10, haps: 5 });
    const r = buildReport(DEFAULTS, runs, { bun: '1', strudel: 'abc' });
    expect(r.msPerSecond).toBe(20);
    expect(r.runs).toHaveLength(3);
    expect(r.haps).toBe(5);
    expect(Object.keys(r).sort()).toEqual(
      [
        'bun',
        'haps',
        'msPerFire',
        'msPerSecond',
        'runs',
        'seconds',
        'section',
        'song',
        'strudel',
      ].sort(),
    );
    expect(formatTable(r)).toContain('median msPerSecond: 20.00');
  });
});

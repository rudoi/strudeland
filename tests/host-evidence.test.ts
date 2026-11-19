import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const EVIDENCE = join(import.meta.dir, '../plans/0012-headless-host/evidence');

describe('headless host evidence', () => {
  test.each([
    'runtime-bench.json',
    'runtime-bench.md',
    'string-number-diff.md',
    'third-party-grep.md',
    'quickjs-spot-timings.md',
  ])('%s exists', (file) => {
    expect(existsSync(join(EVIDENCE, file))).toBe(true);
  });

  const bench = JSON.parse(readFileSync(join(EVIDENCE, 'runtime-bench.json'), 'utf8'));

  test('runtime-bench.json records the V8 version and the fixed budget', () => {
    expect(typeof bench.versions.v8).toBe('string');
    expect(bench.versions.v8.length).toBeGreaterThan(0);
    expect(bench.budget.limits).toEqual({
      fullSongMaxMs: 2000,
      fullSongMedianMaxMs: 500,
      loadMedianMaxMs: 100,
      snapshotLoadRatio: 0.25,
    });
  });

  test.each([
    ['before query reuse', bench.remeasure.before],
    ['after query reuse', bench],
  ])('the historical verdict follows from the aggregate results %s', (_label, measurement) => {
    const { limits, results, passed } = measurement.budget;
    const maxPassed = measurement.totals.longestPlanMs <= limits.fullSongMaxMs;
    const medianPassed = results.medianFullSongMs <= limits.fullSongMedianMaxMs;
    const loadPassed = results.medianLoadMs <= limits.loadMedianMaxMs;
    expect(results.fullSongMaxPassed).toBe(maxPassed);
    expect(results.overFullSongMaxCount === 0).toBe(maxPassed);
    expect(results.fullSongMedianPassed).toBe(medianPassed);
    expect(results.loadMedianPassed).toBe(loadPassed);
    expect(passed).toBe(maxPassed && medianPassed && loadPassed);
    expect(results.snapshotTriggered).toBe(
      !loadPassed || results.medianLoadMs > limits.snapshotLoadRatio * results.medianPlanMs,
    );
  });
});

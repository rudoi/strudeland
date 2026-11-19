import { describe, expect, test } from 'bun:test';
import {
  ceilSigned,
  ceilTo,
  chooseThreshold,
  closestRule,
  dbJson,
  engineDecision,
  fixtureTiming,
  microTargetDb,
  microVerdict,
  platformName,
  nullResidualDb,
  quantiles,
  range,
  renderTimesSec,
  ruleGrid,
  seamFlagged,
  sweepRow,
  twinDecision,
  worstWindow,
  type EngineFixtureStats,
  type TwinFixtureStats,
} from '../scripts/render/probes/calibrate';

const band = (p95: number, max: number) => ({ p50: 0, p95, max });
const stat = (over: Partial<TwinFixtureStats> = {}): TwinFixtureStats => ({
  name: 'a',
  stochastic: false,
  bitEqual: true,
  residualDb: -200,
  bands: { sub: band(0, 0), mid: band(0, 0), high: band(0, 0) },
  lufsDelta: 0,
  peakDeltaDb: 0,
  ...over,
});

describe('quantiles', () => {
  test('empty is all zero', () => {
    expect(quantiles([])).toEqual({ n: 0, p50: 0, p95: 0, p99: 0, max: 0 });
  });
  test('nearest rank over 1..101', () => {
    const q = quantiles(Array.from({ length: 101 }, (_, i) => 101 - i));
    expect(q).toEqual({ n: 101, p50: 51, p95: 96, p99: 100, max: 101 });
  });
  test('single value', () => {
    expect(quantiles([3])).toEqual({ n: 1, p50: 3, p95: 3, p99: 3, max: 3 });
  });
});

describe('ceilTo', () => {
  test('rounds up to the step', () => {
    expect(ceilTo(0.31, 0.05)).toBe(0.35);
    expect(ceilTo(0.35, 0.05)).toBe(0.35);
    expect(ceilTo(1.91, 0.1)).toBe(2);
  });
  test('never below one step', () => {
    expect(ceilTo(0, 0.05)).toBe(0.05);
    expect(ceilTo(1e-7, 0.1)).toBe(0.1);
    expect(ceilTo(-3, 1)).toBe(1);
  });
  test('rejects a non-positive step', () => {
    expect(() => ceilTo(1, 0)).toThrow(RangeError);
    expect(() => ceilSigned(1, -1)).toThrow(RangeError);
  });
  test('ceilSigned keeps the sign', () => {
    expect(ceilSigned(-144.19, 1)).toBe(-144);
    expect(ceilSigned(-12, 1)).toBe(-12);
    expect(ceilSigned(0.21, 0.1)).toBe(0.3);
  });
});

describe('twinDecision', () => {
  test('all bit-equal: floors and the residual of -200', () => {
    const d = twinDecision([stat(), stat({ name: 'b' })]);
    expect(d.TWIN_BITEXACT_HOLDS).toBe(true);
    expect(d.TWIN_P95_DB).toBe(0.05);
    expect(d.TWIN_MAX_DB).toBe(0.1);
    expect(d.TWIN_RESIDUAL_DB).toBe(-197);
    expect(d.self).toEqual({ bandMeanDb: 0.1, bandP95Db: 0.5, lufsDb: 0.05, peakDb: 0.1 });
    expect(d.bitEqual).toEqual(['a', 'b']);
  });
  test('a non-stochastic difference breaks bit-exactness; the worst values set the tolerances', () => {
    const d = twinDecision([
      stat(),
      stat({
        name: 'b',
        bitEqual: false,
        residualDb: -40.5,
        bands: { sub: band(0.2, 1), mid: band(0.4, 1.9), high: band(0.1, 0.3) },
        lufsDelta: 0.04,
        peakDeltaDb: -0.08,
      }),
    ]);
    expect(d.TWIN_BITEXACT_HOLDS).toBe(false);
    expect(d.TWIN_P95_DB).toBe(0.5);
    expect(d.TWIN_MAX_DB).toBe(2.4);
    expect(d.TWIN_RESIDUAL_DB).toBe(-37);
    expect(d.self.bandMeanDb).toBeCloseTo(0.8);
    expect(d.self.bandP95Db).toBeCloseTo(1.2);
    expect(d.self.lufsDb).toBeCloseTo(0.08);
    expect(d.self.peakDb).toBeCloseTo(0.16);
    expect(d.notBitEqual).toEqual(['b']);
  });
  test('a stochastic fixture does not decide bit-exactness', () => {
    const d = twinDecision([stat(), stat({ name: 's', stochastic: true, bitEqual: false })]);
    expect(d.TWIN_BITEXACT_HOLDS).toBe(true);
  });
  test('no statistics throws', () => {
    expect(() => twinDecision([])).toThrow(RangeError);
  });
});

describe('sweep and choice', () => {
  test('sweepRow recall, and 0 with no mutated units', () => {
    expect(sweepRow(12, 9, 10, 1, 50)).toEqual({
      db: 12,
      recall: 0.9,
      falseAlarms: 1,
      cleanUnits: 50,
      mutatedUnits: 10,
    });
    expect(sweepRow(6, 0, 0, 0, 5).recall).toBe(0);
  });
  test('smallest clean threshold wins', () => {
    const rows = [sweepRow(10, 10, 10, 0, 9), sweepRow(6, 10, 10, 2, 9), sweepRow(8, 9, 10, 0, 9)];
    expect(chooseThreshold(rows)).toMatchObject({ db: 8, rule: 'clean' });
  });
  test('fallback: maximal recall with at most one false alarm, smallest on ties', () => {
    const rows = [
      sweepRow(6, 10, 10, 3, 9),
      sweepRow(8, 8, 10, 1, 9),
      sweepRow(10, 8, 10, 1, 9),
      sweepRow(12, 5, 10, 0, 9),
    ];
    const c = chooseThreshold(rows)!;
    expect(c.db).toBe(8);
    expect(c.rule).toBe('fallback');
    expect(c.reason).toContain('1 false alarm');
  });
  test('none when every threshold has two or more false alarms', () => {
    expect(chooseThreshold([sweepRow(6, 1, 1, 2, 3)])).toBeNull();
    expect(chooseThreshold([])).toBeNull();
  });
  test('seamFlagged adds the twin floor only over the line', () => {
    expect(seamFlagged(0.2, null, 0.25)).toBe(false);
    expect(seamFlagged(0.3, null, 0.25)).toBe(true);
    expect(seamFlagged(0.3, 0.1, 0.25)).toBe(false);
    expect(seamFlagged(30, 0.1, 3)).toBe(true);
    expect(seamFlagged(3, 5, 3)).toBe(false);
  });
});

describe('bass rule grid', () => {
  test('range has no float drift and includes the end', () => {
    expect(range(0.1, 0.3, 0.05)).toEqual([0.1, 0.15, 0.2, 0.25, 0.3]);
    expect(range(6, 24, 2)).toHaveLength(10);
    expect(range(5, 4, 1)).toEqual([]);
  });
  test('ruleGrid is the full product', () => {
    const g = ruleGrid({
      clusterMin: [2, 3],
      clusterSpanBars: [2, 4, 8],
      clusterDeepDb: [12],
      fractionMax: [0.1, 0.2],
    });
    expect(g).toHaveLength(12);
    expect(g[0]).toEqual({
      clusterMin: 2,
      clusterSpanBars: 2,
      clusterDeepDb: 12,
      fractionMax: 0.1,
    });
  });
  const steps = { clusterMin: 1, clusterSpanBars: 1, clusterDeepDb: 1, fractionMax: 0.05 };
  const start = { clusterMin: 3, clusterSpanBars: 4, clusterDeepDb: 15, fractionMax: 0.2 };
  test('closestRule picks the start when valid', () => {
    const far = { ...start, clusterMin: 2, clusterDeepDb: 12 };
    expect(closestRule([far, start], start, steps)).toEqual(start);
  });
  test('closestRule measures in axis steps; ties keep the first', () => {
    const a = { ...start, fractionMax: 0.3 }; // 2 steps
    const b = { ...start, clusterDeepDb: 14 }; // 1 step
    const c = { ...start, clusterDeepDb: 16 }; // 1 step
    expect(closestRule([a, b, c], start, steps)).toEqual(b);
    expect(closestRule([], start, steps)).toBeNull();
  });
});

describe('engine null', () => {
  const f32 = (xs: number[]) => Float32Array.from(xs);

  test('nullResidualDb is the dB ratio over all channels', () => {
    const ref = [f32([1, 0]), f32([0, 1])];
    expect(nullResidualDb(ref, [f32([1, 0]), f32([0, 1])])).toBe(-Infinity);
    // diff power (1 - f32(0.9))^2 over ref power 2
    expect(nullResidualDb(ref, [f32([0.9, 0]), f32([0, 1])])!).toBeCloseTo(
      10 * Math.log10((1 - Math.fround(0.9)) ** 2 / 2),
      6,
    );
    expect(nullResidualDb([f32([0, 0])], [f32([0, 0])])).toBe(-Infinity);
    expect(nullResidualDb([f32([0, 0])], [f32([0, 1])])).toBe(Infinity);
  });

  test('nullResidualDb is null for unequal shapes and NaN', () => {
    expect(nullResidualDb([f32([1])], [f32([1]), f32([1])])).toBeNull();
    expect(nullResidualDb([f32([1, 2])], [f32([1])])).toBeNull();
    expect(nullResidualDb([f32([1])], [f32([NaN])])).toBeNull();
  });

  test('worstWindow picks the loudest residual above the floor', () => {
    const sr = 10;
    const ref = [f32([1, 1, 1, 1, 0.5, 0.5, 0.5, 0.5, 1e-5, 1e-5])];
    const cand = [f32([1, 1, 1, 1, 0.5, 0.5, 0.5, 0.25, 0, 0])];
    const w = worstWindow(ref, cand, sr, 0.4, -60)!;
    // the last (quiet) window is below -60 dBFS and is skipped although it differs more
    expect(w.startSec).toBeCloseTo(0.4, 9);
    expect(w.residualDb).toBeCloseTo(10 * Math.log10(0.0625 / 1), 6);
    expect(w.refDbfs).toBeCloseTo(10 * Math.log10(0.25), 6);
    expect(w.diffDbfs).toBeCloseTo(10 * Math.log10(0.0625 / 4), 6);
  });

  test('worstWindow on equal buffers is -inf, on silence or bad shapes null', () => {
    const a = [f32([0.5, 0.5, 0.5, 0.5])];
    expect(worstWindow(a, [f32([0.5, 0.5, 0.5, 0.5])], 4, 0.5)!.residualDb).toBe(-Infinity);
    expect(worstWindow([f32([0, 0])], [f32([1, 1])], 2, 0.5)).toBeNull();
    expect(worstWindow(a, [f32([0.5])], 4)).toBeNull();
    expect(worstWindow([], [], 4)).toBeNull();
  });

  test('dbJson rounds and names infinities', () => {
    expect(dbJson(-12.3456)).toBe(-12.35);
    expect(dbJson(-Infinity)).toBe('-inf');
    expect(dbJson(Infinity)).toBe('inf');
    expect(dbJson(null)).toBeNull();
    expect(dbJson(NaN)).toBeNull();
  });

  test('microTargetDb: -40 for delay and reverb (local ones too), -60 otherwise', () => {
    expect(microTargetDb('delay-impulse-echoes')).toBe(-40);
    expect(microTargetDb('reverb-ir-swap')).toBe(-40);
    expect(microTargetDb('measure-feedback-echo')).toBe(-40);
    expect(microTargetDb('measure-suspend-quantization')).toBe(-60);
    expect(microTargetDb('osc-sine')).toBe(-60);
    expect(microTargetDb('noise-mix')).toBe(-60);
    expect(microTargetDb('fx-chain-delay')).toBe(-40);
    expect(microTargetDb('fx-chain-room')).toBe(-40);
    expect(microTargetDb('fx-chain-gain')).toBe(-60);
    expect(microTargetDb('fx-chain-mod')).toBe(-60);
  });

  test('microVerdict: thresholds, the platform skip and the floor rule', () => {
    const base = { maxResidualDb: -140, host: 'macos' };
    expect(microVerdict({ ...base, residual: -Infinity })).toEqual({ meetsThreshold: true });
    expect(microVerdict({ ...base, residual: -140 })).toEqual({ meetsThreshold: true });
    expect(microVerdict({ ...base, residual: -139.9 })).toEqual({ meetsThreshold: false });
    expect(microVerdict({ ...base, residual: null })).toEqual({ meetsThreshold: false });
    expect(microVerdict({ residual: -150, maxResidualDb: undefined, host: 'macos' })).toEqual({
      meetsThreshold: false,
    });
    // A platform case off its platform: rendered, reported, not judged.
    expect(microVerdict({ ...base, residual: -10, platform: 'macos', host: 'linux' })).toEqual({
      meetsThreshold: null,
    });
    expect(microVerdict({ ...base, residual: -10, platform: 'macos', host: 'macos' })).toEqual({
      meetsThreshold: false,
    });
    // Floor cases carry floorDb and meetsFloorRule.
    const floor = { residual: -150, maxResidualDb: -139, host: 'macos', floorDb: -145.66 };
    expect(microVerdict(floor)).toEqual({
      meetsThreshold: true,
      floorDb: -145.66,
      meetsFloorRule: true,
    });
    expect(microVerdict({ ...floor, residual: -130 }).meetsFloorRule).toBe(false);
    expect(microVerdict({ ...floor, maxResidualDb: -150, residual: -160 }).meetsFloorRule).toBe(
      false,
    );
    expect(platformName('darwin')).toBe('macos');
    expect(platformName('linux')).toBe('linux');
  });

  const efs = (over: Partial<EngineFixtureStats> = {}): EngineFixtureStats => ({
    name: 'a',
    parity: {
      frameDelta: 0,
      offsetSec: 0,
      peakDeltaDb: 0,
      lufsDeltaDb: 0,
      bandMeanDeltaDb: { sub: 0, low: 0 },
      bandP95DeltaDb: { sub: 0, low: 0 },
    },
    twin: { peakDeltaDb: 0, lufsDelta: 0, bandP95: 0 },
    ...over,
  });

  test('engineDecision keeps the self floors for exact renders', () => {
    const d = engineDecision([efs(), efs({ name: 'b' })]);
    expect(d.engine).toEqual({
      name: 'engine',
      frameTolerance: 0,
      maxOffsetSec: 0.01,
      peakDb: 0.1,
      lufsDb: 0.05,
      bandMeanDb: 0.1,
      bandP95Db: 0.5,
      hashMustMatch: false,
    });
    expect(d.worst.from).toEqual({});
  });

  test('engineDecision scales the worst deltas and names their fixtures', () => {
    const d = engineDecision([
      efs({
        name: 'a',
        parity: {
          frameDelta: 0,
          offsetSec: -0.02,
          peakDeltaDb: -0.3,
          lufsDeltaDb: null,
          bandMeanDeltaDb: { sub: -0.04, high: 0.2 },
          bandP95DeltaDb: { sub: 0.1, high: 0.25 },
        },
      }),
      efs({ name: 'b', twin: { peakDeltaDb: 0.01, lufsDelta: 0.04, bandP95: 0.3 } }),
    ]);
    expect(d.engine.maxOffsetSec).toBe(0.04);
    expect(d.engine.peakDb).toBe(0.6);
    expect(d.engine.lufsDb).toBe(0.08);
    expect(d.engine.bandMeanDb).toBe(0.6);
    expect(d.engine.bandP95Db).toBe(0.9);
    expect(d.worst.from).toEqual({
      offsetSec: 'a',
      peakDb: 'a',
      lufsDb: 'b',
      bandMeanDb: 'b',
      bandP95Db: 'b',
    });
  });

  test('engineDecision rejects an empty list', () => {
    expect(() => engineDecision([])).toThrow(RangeError);
  });
});

describe('render times', () => {
  const r = (renderMs: number) => ({ timing: { renderMs } });

  test('fixtureTiming keeps chunk order and rounds to 1 ms', () => {
    expect(fixtureTiming([r(1000.4), r(2.5)], [r(30.6), r(0)])).toEqual({
      rustRenderMs: [31, 0],
      superdoughRenderMs: [1000, 3],
    });
    expect(fixtureTiming([], [])).toEqual({ rustRenderMs: [], superdoughRenderMs: [] });
  });

  test('fixtureTiming refuses unequal chunk counts and bad times', () => {
    expect(() => fixtureTiming([r(1)], [])).toThrow('chunk counts differ: superdough 1, rust 0');
    expect(() => fixtureTiming([r(NaN)], [r(1)])).toThrow('bad render time NaN');
    expect(() => fixtureTiming([r(1)], [r(-1)])).toThrow('bad render time -1');
    expect(() => fixtureTiming([r(1)], [r(Infinity)])).toThrow('bad render time Infinity');
  });

  test('renderTimesSec sums the Rust chunks in seconds, by name', () => {
    const t = (rust: number[]) => ({ rustRenderMs: rust, superdoughRenderMs: rust.map(() => 9) });
    const out = renderTimesSec([
      { name: 'fx-chain', timing: t([1234, 1006]) },
      { name: 'extra-long-song', timing: t([20004, 11000]) },
      { name: 'empty', timing: t([]) },
    ]);
    expect(out).toEqual({ empty: 0, 'extra-long-song': 31, 'fx-chain': 2.24 });
    expect(Object.keys(out)).toEqual(['empty', 'extra-long-song', 'fx-chain']);
    // the superdough times are not counted
    expect(
      renderTimesSec([{ name: 'a', timing: { rustRenderMs: [5], superdoughRenderMs: [1e6] } }]),
    ).toEqual({ a: 0.01 });
  });

  test('renderTimesSec refuses a duplicate fixture', () => {
    const t = { rustRenderMs: [1], superdoughRenderMs: [1] };
    expect(() =>
      renderTimesSec([
        { name: 'a', timing: t },
        { name: 'a', timing: t },
      ]),
    ).toThrow('duplicate fixture a');
  });
});

import { describe, expect, mock, test } from 'bun:test';
import { buildStructure } from '../src/lib/strudel/structure';
import { LINEAR_SEGMENTS } from '../src/lib/strudel/transport';
import { renderTiming } from '../src/lib/render/timing';
import { biquadBandpass } from '../src/lib/render/dsp';
import {
  judgeSeam,
  seamConsistency,
  seamWithTwinEscalation,
  twinFloorDb,
  type SeamConsistency,
} from '../src/lib/render/gates/seam';
import { bandQuantiles, twinMetrics, twinNull } from '../src/lib/render/gates/twin';
import { bassEvents, bassGate, bassPresence } from '../src/lib/render/gates/bass';
import { deriveOpening, verifyMix, type MixInput } from '../src/lib/render/gates/mix';
import {
  judgeStemScores,
  mergePairVerdicts,
  orbitPair,
  stemsGate,
  stemsOnsetScores,
  type StemNoteScore,
  type StemsScoreOptions,
} from '../src/lib/render/gates/stems';
import { roundDb, roundSec } from '../src/lib/render/gates/types';
import type { RenderEvent } from '../src/lib/render/schedule';
import {
  BASS_CLUSTER_DEEP_DB,
  BASS_MISS_DB,
  SEAM_FAIL_DB,
  SEAM_TOLERANCE_DB,
  STEMS_MISS_DB,
  TWIN_MAX_DB,
} from '../src/lib/render/thresholds';
import { ev } from './helpers/render-events';
import { concat, noiseSeeded, scale, sine, silence, SR } from './helpers/render-signals';

const SEC = 2; // one bar of overlap at 120 bpm 4/4
const N = SEC * SR;
const SEAM = 16;

interface Mix {
  sub?: number;
  mid?: number;
  high?: number;
  noise?: number;
  seed?: number;
  sec?: number;
}

/** 80 Hz (sub) + 440 Hz (mid) + 6 kHz (high) sines at 0.1 plus a seeded noise bed at 0.01; each
 *  part scaled by its gain. */
function mono(m: Mix = {}): Float32Array {
  const sec = m.sec ?? SEC;
  const a = sine(80, 0.1 * (m.sub ?? 1), sec);
  const b = sine(440, 0.1 * (m.mid ?? 1), sec);
  const c = sine(6000, 0.1 * (m.high ?? 1), sec);
  const n = noiseSeeded(m.seed ?? 1, a.length, 0.01 * (m.noise ?? 1));
  return a.map((v, i) => v + b[i]! + c[i]! + n[i]!);
}
const st = (x: Float32Array): Float32Array[] => [x, x.slice()];
const mix = (m: Mix = {}) => st(mono(m));
const db = (d: number) => Math.pow(10, d / 20);
const opts = { toleranceDb: SEAM_TOLERANCE_DB };

/** The pad (80 Hz and 440 Hz) is missing for the first `sec` seconds: lookback too short. */
function padLost(sec: number): Float32Array[] {
  const k = Math.round(sec * SR);
  return st(concat(mono({ sub: 0, mid: 0 }).subarray(0, k), mono().subarray(k)));
}

/** `head` plus independent noise in the high band, the shape of a stochastic re-render. */
function twinOf(head: Float32Array[], amp: number, seed = 99): Float32Array[] {
  const n = biquadBandpass(noiseSeeded(seed, head[0]!.length, amp), SR, 2000, 16000);
  return head.map((ch) => ch.map((v, i) => v + n[i]!));
}

describe('gate rounding', () => {
  test('dB to 2 decimals, seconds to 3', () => {
    expect(roundDb(1.23456)).toBe(1.23);
    expect(roundSec(1.23456)).toBe(1.235);
    expect(roundDb(-0.004)).toBe(-0);
  });
});

describe('seamConsistency', () => {
  test('identical buffers: no deficit, pass', () => {
    const sc = seamConsistency(mix(), mix(), SR, SEAM, opts);
    expect(sc.worstDeficitDb).toBe(0);
    expect(sc.deficits).toEqual([]);
    expect(sc.windows).toBe(3 * 20);
    expect(sc.seamSec).toBe(SEAM);
    const g = judgeSeam(sc);
    expect(g).toEqual({
      gate: 'seam',
      verdict: 'pass',
      measurements: {
        seamSec: SEAM,
        windows: 60,
        worstDeficitDb: 0,
        deficitWindows: 0,
        twinFloorDb: null,
        bands: [],
      },
      reasons: [],
      warnings: [],
    });
  });

  test('head 0.3 dB quieter: under tolerance, pass', () => {
    const head = mix().map((ch) => ch.map((v) => v * 0.966));
    const sc = seamConsistency(mix(), head, SR, SEAM, opts);
    expect(sc.worstDeficitDb).toBeCloseTo(0.3, 1);
    expect(sc.deficits).toEqual([]);
    expect(judgeSeam(sc).verdict).toBe('pass');
  });

  test('head 2 dB quieter in one band: deficits over a 1 dB listing line and a warning, still pass', () => {
    // SEAM_TOLERANCE_DB (3.1) is above SEAM_FAIL_DB (3) since calibration, so listed deficits
    // that still pass only appear with a lower listing tolerance (or an escalated twin floor).
    const sc = seamConsistency(mix(), mix({ high: db(-2) }), SR, SEAM, { toleranceDb: 1 });
    expect(sc.deficits.length).toBeGreaterThan(0);
    expect(sc.deficits.every((d) => d.band === 'high')).toBe(true);
    expect(sc.worstDeficitDb).toBeGreaterThan(1.8);
    expect(sc.worstDeficitDb).toBeLessThan(2.1);
    expect(sc.deficits[1]!.tSec).toBeCloseTo(SEAM + 0.1, 9);
    const g = judgeSeam(sc);
    expect(g.verdict).toBe('pass');
    expect(g.warnings).toHaveLength(1);
    expect(g.warnings[0]).toContain(`over ${SEAM_TOLERANCE_DB} dB`);
    expect(g.measurements.bands).toEqual(['high']);
  });

  test('mutation lookback 0: a pad lost at the seam fails with a sub deficit', () => {
    const sc = seamConsistency(mix(), padLost(1.5), SR, SEAM, opts);
    // The plan expected > 40 dB (earlier song calibration: 43-78 dB). With these synthetic sines the
    // 6 kHz tone leaks through the broad second-order sub bandpass at about -35 dB, which caps
    // the measurable deficit near 35 dB: still ten times the fail line.
    expect(sc.worstDeficitDb).toBeGreaterThan(30);
    const g = judgeSeam(sc);
    expect(g.verdict).toBe('fail');
    expect(g.measurements.bands).toContain('sub');
    expect(g.reasons[0]).toMatch(/^seam at 16\.00s lost material: worst band deficit/);
    expect(g.reasons[0]).toContain('twin floor 0 dB');
  });

  test('one-sided: a louder head is not a loss', () => {
    const head = mix().map((ch) => ch.map((v) => v * db(10)));
    const sc = seamConsistency(mix(), head, SR, SEAM, opts);
    expect(sc.worstDeficitDb).toBe(0);
    expect(judgeSeam(sc).verdict).toBe('pass');
  });

  test('both silent: every window skipped, unverifiable', () => {
    const z = st(silence(SEC));
    const sc = seamConsistency(z, z, SR, SEAM, opts);
    expect(sc.windows).toBe(0);
    const g = judgeSeam(sc);
    expect(g.verdict).toBe('unverifiable');
    expect(g.reasons).toEqual(['overlap is silent in every band']);
  });

  test('tail audible and head silent: windows counted, fail', () => {
    const sc = seamConsistency(mix(), st(silence(SEC)), SR, SEAM, opts);
    expect(sc.windows).toBe(60);
    expect(judgeSeam(sc).verdict).toBe('fail');
  });

  test('mono mean: an L = -R pair cancels to silence', () => {
    const x = mono();
    const pair = [x, x.map((v) => -v)];
    const sc = seamConsistency(pair, pair, SR, SEAM, opts);
    expect(sc.windows).toBe(0);
    expect(judgeSeam(sc).verdict).toBe('unverifiable');
  });

  test('compares only the common length', () => {
    const long = mix({ sec: 3 });
    const sc = seamConsistency(long, mix(), SR, SEAM, opts);
    expect(sc.windows).toBe(60);
  });

  test('a partial window is dropped: 1050 frames at w 4410 gives 0 windows', () => {
    const x = st(mono().subarray(0, 1050));
    expect(seamConsistency(x, x, SR, SEAM, opts).windows).toBe(0);
  });

  test('window and floor options', () => {
    const sc = seamConsistency(mix(), mix(), SR, SEAM, { ...opts, windowSec: 0.5 });
    expect(sc.windows).toBe(3 * 4);
    const quiet = st(mono().map((v) => v * 1e-4));
    expect(seamConsistency(quiet, quiet, SR, SEAM, { ...opts, floor: 1e-3 }).windows).toBe(0);
  });
});

describe('judgeSeam', () => {
  const sc = (worst: number, twinFloorDb?: number): SeamConsistency => ({
    seamSec: 4.123456,
    windows: 30,
    worstDeficitDb: worst,
    deficits: [{ tSec: 4.2, band: 'mid', deficitDb: worst }],
    ...(twinFloorDb === undefined ? {} : { twinFloorDb }),
  });

  test('fails just over the fail line, passes on it', () => {
    expect(judgeSeam(sc(SEAM_FAIL_DB)).verdict).toBe('pass');
    expect(judgeSeam(sc(SEAM_FAIL_DB + 0.01)).verdict).toBe('fail');
  });

  test('the twin floor raises the fail line', () => {
    const g = judgeSeam(sc(4, 1.5));
    expect(g.verdict).toBe('pass');
    expect(g.measurements.twinFloorDb).toBe(1.5);
    const f = judgeSeam(sc(5, 1.5));
    expect(f.verdict).toBe('fail');
    expect(f.reasons[0]).toBe(
      'seam at 4.12s lost material: worst band deficit 5 dB (> 3 dB + twin floor 1.5 dB) in 1/30 windows',
    );
    expect(f.measurements.seamSec).toBe(4.123);
  });
});

describe('seamWithTwinEscalation', () => {
  test('no re-render at or under the fail line', async () => {
    const rerender = mock(async () => mix());
    const g = await seamWithTwinEscalation(mix(), mix({ high: db(-2) }), SR, SEAM, rerender);
    expect(g.verdict).toBe('pass');
    expect(rerender).not.toHaveBeenCalled();
    expect(g.measurements.twinFloorDb).toBeNull();
  });

  test('a noisy twin floor rescues a 4 dB high-band deficit', async () => {
    const tail = mix();
    const head = mix({ high: db(-4) });
    const plain = judgeSeam(seamConsistency(tail, head, SR, SEAM, opts));
    expect(plain.verdict).toBe('fail');

    const rerender = mock(async () => twinOf(head, 0.11));
    const g = await seamWithTwinEscalation(tail, head, SR, SEAM, rerender);
    expect(rerender).toHaveBeenCalledTimes(1);
    const floor = g.measurements.twinFloorDb as number;
    expect(floor).toBeGreaterThan(1.5);
    expect(floor).toBeLessThan(3.5);
    expect(g.verdict).toBe('pass');
  });

  test('a lost pad still fails: the twin lacks it too', async () => {
    const head = padLost(1.5);
    const rerender = mock(async () => twinOf(head, 0.11));
    const g = await seamWithTwinEscalation(mix(), head, SR, SEAM, rerender);
    expect(rerender).toHaveBeenCalledTimes(1);
    expect(g.verdict).toBe('fail');
    expect(g.measurements.bands).toContain('sub');
  });

  test('a bit-exact twin gives a 0 floor and the plain rule', async () => {
    const head = mix({ high: db(-4) });
    const g = await seamWithTwinEscalation(mix(), head, SR, SEAM, async () => head);
    expect(g.measurements.twinFloorDb).toBe(0);
    expect(g.verdict).toBe('fail');
  });
});

describe('twinFloorDb', () => {
  test('0 for equal heads; the worse of the two directions otherwise', () => {
    const head = mix();
    expect(twinFloorDb(head, head, SR, SEAM)).toBe(0);
    const quieter = mix({ high: db(-4) });
    const oneWay = seamConsistency(head, quieter, SR, SEAM, opts).worstDeficitDb;
    expect(oneWay).toBeGreaterThan(3);
    expect(twinFloorDb(head, quieter, SR, SEAM)).toBe(oneWay);
    expect(twinFloorDb(quieter, head, SR, SEAM)).toBe(oneWay);
  });
});

describe('bandQuantiles', () => {
  test('nearest rank on [0, 1, 2, 3, 4]', () => {
    expect(bandQuantiles([4, 0, 3, 1, 2])).toEqual({ cells: 5, p50: 2, p95: 4, max: 4 });
  });

  test('empty is all zero', () => {
    expect(bandQuantiles([])).toEqual({ cells: 0, p50: 0, p95: 0, max: 0 });
  });

  test('one value', () => {
    expect(bandQuantiles([1.5])).toEqual({ cells: 1, p50: 1.5, p95: 1.5, max: 1.5 });
  });
});

describe('twinNull', () => {
  const exact = { sampleRate: SR, mode: 'exact' as const };
  const tol = { sampleRate: SR, mode: 'tolerance' as const };

  test('identical buffers pass exact with residual -200', () => {
    const g = twinNull(mix(), mix(), exact);
    expect(g.gate).toBe('twin');
    expect(g.verdict).toBe('pass');
    expect(g.reasons).toEqual([]);
    expect(g.warnings).toEqual([]);
    expect(g.measurements).toMatchObject({
      mode: 'exact',
      frames: N,
      channels: 2,
      differingFrames: 0,
      residualDb: -200,
    });
    const bands = g.measurements.bands as Record<string, { cells: number; max: number }>;
    expect(Object.keys(bands)).toEqual(['sub', 'mid', 'high']);
    expect(bands.sub!.cells).toBe(20);
    expect(bands.high!.max).toBe(0);
    expect(twinNull(mix(), mix(), tol).verdict).toBe('pass');
  });

  test('one sample off by 1 ulp: exact fails, tolerance passes', () => {
    const b = mix();
    const bits = new Uint32Array(b[1]!.buffer);
    bits[12345] = bits[12345]! + 1;
    const g = twinNull(mix(), b, exact);
    expect(g.verdict).toBe('fail');
    expect(g.measurements.differingFrames).toBe(1);
    expect(g.reasons[0]).toBe('1 samples differ, first at frame 12345 ch 1');
    expect(typeof g.measurements.residualDb).toBe('number');
    expect(twinNull(mix(), b, tol).verdict).toBe('pass');
  });

  test('-0 against 0 differs by bit pattern', () => {
    const a = [new Float32Array([0, 1]), new Float32Array([0, 1])];
    const b = [new Float32Array([-0, 1]), new Float32Array([0, 1])];
    expect(twinMetrics(a, b, SR).differingFrames).toBe(1);
  });

  test('independent noise at -90 dB passes tolerance with residual near -90', () => {
    const sig = mono({ noise: 0 });
    const level = 0.1 * Math.sqrt(3 / 2) * db(-90) * Math.sqrt(3); // uniform amp for -90 dB rms
    const a = st(sig.map((v, i) => v + noiseA[i]! * level));
    const b = st(sig.map((v, i) => v + noiseB[i]! * level));
    const g = twinNull(a, b, tol);
    expect(g.verdict).toBe('pass');
    const r = g.measurements.residualDb as number;
    expect(r).toBeGreaterThan(-90);
    expect(r).toBeLessThan(-85); // two independent noises: -90 + 3 dB
  });

  test('independent noise at -40 dB fails tolerance on the residual', () => {
    const sig = mono({ noise: 0 });
    const level = 0.1 * Math.sqrt(3 / 2) * db(-40) * Math.sqrt(3);
    const a = st(sig.map((v, i) => v + noiseA[i]! * level));
    const b = st(sig.map((v, i) => v + noiseB[i]! * level));
    const g = twinNull(a, b, tol);
    expect(g.verdict).toBe('fail');
    expect(g.reasons.some((r) => r.startsWith('residual '))).toBe(true);
  });

  test('independent noise at -8 dB fails tolerance on the residual', () => {
    const sig = mono({ noise: 0 });
    const level = 0.1 * Math.sqrt(3 / 2) * db(-8) * Math.sqrt(3);
    const a = st(sig.map((v, i) => v + noiseA[i]! * level));
    const b = st(sig.map((v, i) => v + noiseB[i]! * level));
    const g = twinNull(a, b, tol);
    expect(g.verdict).toBe('fail');
    expect(g.reasons.some((r) => r.startsWith('residual '))).toBe(true);
  });

  test('an 8 dB change of the high sine for 1 s fails on the band max', () => {
    const a = mono({ sec: 4 });
    const loud = mono({ sec: 4, high: db(8) });
    const b = a.slice();
    b.set(loud.subarray(SR, 2 * SR), SR);
    const g = twinNull(st(a), st(b), tol);
    expect(g.verdict).toBe('fail');
    expect(
      g.reasons.some((r) => r.startsWith('high band max ') && r.endsWith(`> ${TWIN_MAX_DB} dB`)),
    ).toBe(true);
    // the change stays in the high band: the mid band sees only the filter skirt, the sub nothing
    const bands = g.measurements.bands as Record<string, { max: number }>;
    expect(bands.high!.max).toBeGreaterThan(7);
    expect(bands.mid!.max).toBeLessThan(2);
    expect(bands.sub!.max).toBeLessThan(0.1);
  });

  test('frame count mismatch fails, naming both counts', () => {
    const g = twinNull(mix(), mix({ sec: 1 }), tol);
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual([`frame count differs: ${N} vs ${SR}`]);
    expect(g.measurements).toMatchObject({ frames: N, residualDb: null, bands: null });
  });

  test('channel count mismatch fails', () => {
    const g = twinNull(mix(), [mono()], exact);
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual(['channel count differs: 2 vs 1']);
  });

  test('unequal channels within one render fail', () => {
    const g = twinNull([mono(), mono({ sec: 1 })], mix(), exact);
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual(['channels of one render have unequal lengths']);
  });

  test('a NaN in b fails in both modes', () => {
    const b = mix();
    b[0]![500] = NaN;
    b[0]![300] = Infinity;
    for (const o of [exact, tol]) {
      const g = twinNull(mix(), b, o);
      expect(g.verdict).toBe('fail');
      expect(g.reasons).toEqual(['2 non-finite samples in b, first at frame 300 ch 0']);
      expect(g.measurements.residualDb).toBeNull();
    }
  });

  test('both silent: tolerance unverifiable, exact pass', () => {
    const z = st(silence(1));
    const t = twinNull(z, z, tol);
    expect(t.verdict).toBe('unverifiable');
    expect(t.reasons).toEqual(['both renders are silent in every band']);
    expect(twinNull(z, z, exact).verdict).toBe('pass');
  });

  test('every measurement is finite JSON', () => {
    const g = twinNull(st(silence(1)), st(silence(1)), tol);
    expect(JSON.parse(JSON.stringify(g))).toEqual(g);
  });
});

const noiseA = noiseSeeded(11, N);
const noiseB = noiseSeeded(12, N);

// ---- stems

const BAR = 2; // 120 bpm 4/4
const SONG_SEC = 8 * BAR;
const PAIRS = 4; // orbits 2, 3, 5 on pairs 1, 2, 0

/** A 100 ms decaying 220 Hz click burst per note, plus (on `tail`) a seeded noise tail decaying
 *  over 1.5 s, the shape of a reverb that must not mask a dropped note. */
function stemSignal(
  notes: { tSec: number; gain: number }[],
  sec: number,
  o: { tail?: boolean; level?: (tSec: number) => number } = {},
): Float32Array {
  const out = new Float32Array(Math.round(sec * SR));
  const noise = noiseSeeded(5, out.length, 1);
  for (const n of notes) {
    const k = n.gain * (o.level?.(n.tSec) ?? 1);
    if (k === 0) continue;
    const i0 = Math.round(n.tSec * SR);
    const burst = Math.round(0.1 * SR);
    for (let i = 0; i < burst && i0 + i < out.length; i++) {
      out[i0 + i] += k * Math.sin((2 * Math.PI * 220 * i) / SR) * Math.exp(-i / (0.03 * SR));
    }
    if (!o.tail) continue;
    const tail = Math.round(1.5 * SR);
    for (let i = burst; i < tail && i0 + i < out.length; i++) {
      out[i0 + i] += 0.05 * k * noise[i0 + i]! * Math.exp(-(i - burst) / (0.5 * SR));
    }
  }
  return out;
}

/** A slow pad: 110 Hz sine ramping up over `attack` from each onset. */
function padSignal(onsets: number[], attack: number, sec: number): Float32Array {
  const out = new Float32Array(Math.round(sec * SR));
  for (const t of onsets) {
    const i0 = Math.round(t * SR);
    for (let i = 0; i0 + i < out.length && i < BAR * SR; i++) {
      out[i0 + i] += 0.3 * Math.min(1, i / (attack * SR)) * Math.sin((2 * Math.PI * 110 * i) / SR);
    }
  }
  return out;
}

const beats = (sec: number, phase: number) =>
  Array.from({ length: Math.round(sec / 0.5) }, (_, i) => i * 0.5 + phase);

function threeOrbitSong(level2?: (tSec: number) => number) {
  const e2 = beats(SONG_SEC, 0).map((t) => ev({ tSec: t, orbit: 2, gain: 0.8 }));
  const e3 = beats(SONG_SEC, 0.25).map((t) => ev({ tSec: t, orbit: 3, gain: 0.6 }));
  const e5 = Array.from({ length: 8 }, (_, b) => ev({ tSec: b * BAR, orbit: 5, attack: 1 }));
  const events = [...e2, ...e3, ...e5].sort((a, b) => a.tSec - b.tSec);
  const s2 = stemSignal(e2, SONG_SEC, { tail: true, level: level2 });
  const s3 = stemSignal(e3, SONG_SEC, { tail: true });
  const s5 = padSignal(
    e5.map((e) => e.tSec),
    1,
    SONG_SEC,
  );
  const z = silence(SONG_SEC);
  // pair 0: orbit 5, pair 1: orbit 2, pair 2: orbit 3, pair 3: empty
  const channels = [s5, s5.slice(), s2, s2.slice(), s3, s3.slice(), z, z.slice()];
  return { channels, events };
}

const inBars = (bars: number[], k: number) => (t: number) =>
  bars.includes(Math.floor(t / BAR)) ? k : 1;

describe('orbitPair', () => {
  test('superdough channel mapping, 16 pairs by default', () => {
    expect([0, 1, 2, 16, 17, 33].map((o) => orbitPair(o))).toEqual([0, 0, 1, 15, 0, 0]);
    expect(orbitPair(-1)).toBe(0);
    expect([1, 2, 4, 5].map((o) => orbitPair(o, 4))).toEqual([0, 1, 3, 0]);
  });
});

describe('stemsOnsetScores and the stems gate', () => {
  const clean = threeOrbitSong();
  const cleanScores = stemsOnsetScores(clean.channels, SR, clean.events, BAR);

  test('clean three-orbit song passes; the pad pair is a warning', () => {
    const { pairs, notes } = cleanScores;
    expect(pairs).toEqual([
      { pair: 0, orbits: [5], verifiable: false, reason: 'no fast-attack notes', notes: 0 },
      { pair: 1, orbits: [2], verifiable: true, notes: 32 },
      { pair: 2, orbits: [3], verifiable: true, notes: 32 },
    ]);
    // the last off-beat note (15.75 s) is scored; nothing is within 40 ms of the end
    expect(notes).toHaveLength(64);
    const judged = judgeStemScores(notes, STEMS_MISS_DB);
    expect(judged).toEqual({ tested: 16, misses: [] });
    const g = stemsGate(pairs, judged);
    expect(g.verdict).toBe('pass');
    expect(g.reasons).toEqual([]);
    expect(g.warnings).toEqual(['pair 0 (orbits 5) not verified: no fast-attack notes']);
    expect(g.measurements.tested).toBe(16);
    expect(g.measurements.misses).toBe(0);
    expect((g.measurements.pairs as { reason: string | null }[])[1]!.reason).toBeNull();
  });

  test('mutation: orbit 2 dropped in bars 2, 5, 6 fails with exactly those bars', () => {
    const m = threeOrbitSong(inBars([2, 5, 6], 0));
    const { pairs, notes } = stemsOnsetScores(m.channels, SR, m.events, BAR);
    for (const missDb of [6, 12, 24]) {
      const judged = judgeStemScores(notes, missDb);
      expect(judged.misses.map((x) => [x.orbit, x.pair, x.bar])).toEqual([
        [2, 1, 2],
        [2, 1, 5],
        [2, 1, 6],
      ]);
      expect(judged.tested).toBe(16);
    }
    const judged = judgeStemScores(notes, STEMS_MISS_DB);
    expect(judged.misses[0]!.relDb).toBeLessThan(-24);
    expect(judged.misses[0]!.tSec).toBe(4); // the bar's best note
    const g = stemsGate(pairs, judged);
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toHaveLength(3);
    expect(g.reasons[0]).toMatch(/^orbit 2 missing in bar 2: best onset -\d+(\.\d+)? dB$/);
  });

  test('attenuated, not dropped: -6 dB passes, -24 dB fails', () => {
    const at = (d: number) => {
      const m = threeOrbitSong(inBars([2, 5, 6], db(d)));
      const s = stemsOnsetScores(m.channels, SR, m.events, BAR);
      return judgeStemScores(s.notes, STEMS_MISS_DB).misses.map((x) => x.bar);
    };
    expect(at(-6)).toEqual([]);
    expect(at(-24)).toEqual([2, 5, 6]);
  });

  test('aliasing: orbits 0 and 1 share pair 0 and nothing is testable', () => {
    const notes = beats(4, 0).map((t, i) => ev({ tSec: t, orbit: i % 2 }));
    const s = stemSignal(notes, 4);
    const { pairs, notes: scored } = stemsOnsetScores([s, s.slice()], SR, notes, BAR);
    expect(pairs).toEqual([
      { pair: 0, orbits: [0, 1], verifiable: false, reason: 'shared by aliasing', notes: 0 },
    ]);
    expect(scored).toEqual([]);
    const g = stemsGate(pairs, judgeStemScores(scored, STEMS_MISS_DB));
    expect(g.verdict).toBe('unverifiable');
    expect(g.reasons[0]).toContain('every pair aliased');
    expect(g.warnings[0]).toContain('shared by aliasing');
  });

  test('edge guard: a note 20 ms before the buffer end is skipped, not missed', () => {
    const notes = [...beats(4, 0), 4 - 0.02].map((t) => ev({ tSec: t, orbit: 1 }));
    // the last note is not rendered at all: its rise would be in the next window
    const s = stemSignal(notes.slice(0, -1), 4);
    const { pairs, notes: scored } = stemsOnsetScores([s, s.slice()], SR, notes, BAR);
    expect(pairs[0]!.notes).toBe(9);
    expect(scored).toHaveLength(8);
    expect(scored.some((n) => n.tSec === 3.98)).toBe(false);
    expect(judgeStemScores(scored, STEMS_MISS_DB).misses).toEqual([]);
  });

  test('window filter: before the offset, past the end, and under minGain are ignored', () => {
    const s = stemSignal([{ tSec: 0.5, gain: 0.8 }], 2);
    const events = [
      ev({ tSec: 10.5 }),
      ev({ tSec: 9.5 }), // before offset 10
      ev({ tSec: 12.5 }), // past 10 + 2 s
      ev({ tSec: 11, gain: 0.04 }), // under STEMS_MIN_GAIN
    ];
    const { pairs, notes } = stemsOnsetScores([s, s.slice()], SR, events, BAR, { offsetSec: 10 });
    expect(pairs[0]!.notes).toBe(1);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.bar).toBe(5); // absolute layout bar
    expect(notes[0]!.tSec).toBe(10.5);
  });

  test('maxAttack and minGain options', () => {
    const s = stemSignal([{ tSec: 0.5, gain: 0.8 }], 2);
    const events = [ev({ tSec: 0.5, attack: 0.04 })];
    const r = (o: StemsScoreOptions) => stemsOnsetScores([s, s.slice()], SR, events, BAR, o);
    expect(r({}).pairs[0]!.verifiable).toBe(true);
    expect(r({ maxAttack: 0.03 }).pairs[0]!.reason).toBe('no fast-attack notes');
    expect(r({ minGain: 0.9 }).pairs).toEqual([]);
  });

  test('gain normalisation: a quiet note at 1/10 amplitude scores like a loud one', () => {
    const score = (quiet: boolean) => {
      const notes = beats(4, 0).map((t, i) => ev({ tSec: t, gain: quiet && i === 3 ? 0.1 : 1 }));
      const s = stemSignal(notes, 4);
      return stemsOnsetScores([s, s.slice()], SR, notes, BAR).notes;
    };
    const quiet = score(true);
    expect(quiet[3]!.score / score(false)[3]!.score).toBeCloseTo(1, 6);
    expect(judgeStemScores(quiet, STEMS_MISS_DB).misses).toEqual([]);
  });

  test('pooling: two chunks with their offsets give the same misses as one buffer', () => {
    const m = threeOrbitSong(inBars([2, 5, 6], 0));
    const half = 4 * BAR * SR;
    const chunk = (from: number, to: number) => m.channels.map((c) => c.slice(from, to));
    const a = stemsOnsetScores(chunk(0, half), SR, m.events, BAR);
    const b = stemsOnsetScores(chunk(half, 2 * half), SR, m.events, BAR, { offsetSec: 4 * BAR });
    const whole = stemsOnsetScores(m.channels, SR, m.events, BAR);
    const pooled = judgeStemScores([...a.notes, ...b.notes], STEMS_MISS_DB);
    const one = judgeStemScores(whole.notes, STEMS_MISS_DB);
    expect(pooled.misses.map((x) => [x.orbit, x.bar])).toEqual(
      one.misses.map((x) => [x.orbit, x.bar]),
    );
    expect(pooled.tested).toBe(one.tested);
    expect(mergePairVerdicts([a.pairs, b.pairs])).toEqual(whole.pairs);
  });

  test('pooling regression: a short final chunk judged alone false-alarms, pooled it does not', () => {
    const n = (bar: number, score: number): StemNoteScore => ({
      pair: 0,
      orbit: 1,
      bar,
      tSec: bar * BAR,
      score,
    });
    // bars 0-4: low (d1-like) notes; final chunk bars 5-7: two loud bars, one low bar
    const early = [0, 1, 2, 3, 4].flatMap((b) => [n(b, 0.2), n(b, 0.2)]);
    const last = [n(5, 1), n(5, 1), n(6, 1), n(6, 1), n(7, 0.2)];
    const alone = judgeStemScores(last, STEMS_MISS_DB);
    expect(alone.misses.map((x) => x.bar)).toEqual([7]);
    expect(alone.misses[0]!.relDb).toBe(-13.98);
    expect(judgeStemScores([...early, ...last], STEMS_MISS_DB)).toEqual({ tested: 8, misses: [] });
  });

  test('judgeStemScores: one note never misses, zero typical is skipped, misses sorted', () => {
    const s = (orbit: number, pair: number, bar: number, score: number): StemNoteScore => ({
      pair,
      orbit,
      bar,
      tSec: bar * 2 + 0.0004,
      score,
    });
    expect(judgeStemScores([s(1, 0, 0, 1e-9)], 12)).toEqual({ tested: 1, misses: [] });
    expect(judgeStemScores([s(1, 0, 0, 0), s(1, 0, 1, 0)], 12)).toEqual({ tested: 0, misses: [] });
    const r = judgeStemScores(
      [s(3, 2, 1, 0), s(3, 2, 0, 1), s(3, 2, 2, 1), s(2, 1, 1, 0), s(2, 1, 0, 1), s(2, 1, 3, 1)],
      -12, // the sign is ignored
    );
    expect(r.misses.map((m) => [m.bar, m.pair])).toEqual([
      [1, 1],
      [1, 2],
    ]);
    expect(r.misses[0]!.tSec).toBe(2);
    expect(r.misses[0]!.relDb).toBe(-240);
    // best per bar: a bar with one good note is not a miss
    expect(judgeStemScores([s(1, 0, 0, 1), s(1, 0, 0, 0), s(1, 0, 1, 1)], 12).misses).toEqual([]);
  });

  test('bar rounding snaps a time a hair before a bar line to that bar', () => {
    const s = stemSignal([], 8);
    const { notes } = stemsOnsetScores([s, s.slice()], SR, [ev({ tSec: 5.9999999 })], BAR);
    expect(notes[0]!.bar).toBe(3);
    expect(stemsOnsetScores([s, s.slice()], SR, [ev({ tSec: 5.99 })], BAR).notes[0]!.bar).toBe(2);
  });

  test('channel count errors throw', () => {
    const s = silence(1);
    expect(() => stemsOnsetScores([], SR, [], BAR)).toThrow(RangeError);
    expect(() => stemsOnsetScores([s, s, s], SR, [], BAR)).toThrow(RangeError);
    expect(() => stemsOnsetScores([s, silence(2)], SR, [], BAR)).toThrow(RangeError);
  });

  test('stemsGate lists at most 8 misses', () => {
    const misses = Array.from({ length: 11 }, (_, bar) => ({
      pair: 0,
      orbit: 1,
      bar,
      tSec: bar * 2,
      relDb: -30,
    }));
    const g = stemsGate([{ pair: 0, orbits: [1], verifiable: true, notes: 11 }], {
      tested: 11,
      misses,
    });
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toHaveLength(9);
    expect(g.reasons[8]).toBe('+3');
    expect(g.warnings).toEqual([]);
  });
});

describe('mergePairVerdicts', () => {
  const v = (
    pair: number,
    verifiable: boolean,
    notes: number,
    reason?: string,
    orbits = [pair + 1],
  ) =>
    reason === undefined
      ? { pair, orbits, verifiable, notes }
      : { pair, orbits, verifiable, notes, reason };

  test('verifiable in one chunk wins, notes summed', () => {
    const merged = mergePairVerdicts([
      [v(1, true, 4)],
      [v(1, false, 0, 'no fast-attack notes')],
      [v(1, true, 3)],
    ]);
    expect(merged).toEqual([v(1, true, 7)]);
    expect(mergePairVerdicts([[v(1, false, 0, 'no fast-attack notes')], [v(1, true, 2)]])).toEqual([
      v(1, true, 2),
    ]);
  });

  test('aliased in every chunk stays aliased; first reason kept; sorted by pair', () => {
    const merged = mergePairVerdicts([
      [v(2, false, 0, 'no fast-attack notes'), v(0, false, 0, 'shared by aliasing', [0, 1])],
      [v(0, false, 0, 'shared by aliasing', [1, 17]), v(2, false, 0, 'shared by aliasing', [3])],
    ]);
    expect(merged).toEqual([
      v(0, false, 0, 'shared by aliasing', [0, 1, 17]),
      v(2, false, 0, 'no fast-attack notes', [3]),
    ]);
    expect(mergePairVerdicts([])).toEqual([]);
  });

  test('does not mutate its input', () => {
    const a = [v(1, true, 4)];
    mergePairVerdicts([a, [v(1, true, 3)]]);
    expect(a).toEqual([v(1, true, 4)]);
  });
});

// ---- bass

/** 16 bass notes, one per half bar (2 s bars): 55 Hz plus 110 Hz sines, 0.25 s each, orbit 3.
 *  `amp(i)` scales note i (0 silences it); `gain(i)` is its scheduled gain. */
function bassSong(amp: (i: number) => number = () => 1, gain: (i: number) => number = () => 0.8) {
  const events = Array.from({ length: 16 }, (_, i) =>
    ev({ tSec: i, orbit: 3, freq: 55, s: 'sawtooth', gain: gain(i) }),
  );
  const x = new Float32Array(16 * SR);
  const note = sine(55, 0.3, 0.25).map((v, i) => v + 0.2 * Math.sin((2 * Math.PI * 110 * i) / SR));
  for (const [i, e] of events.entries())
    x.set(scale(note, amp(i) * e.gain), Math.round(e.tSec * SR));
  return { channels: st(x), events };
}

const bassOpts = { missDb: BASS_MISS_DB, barSec: BAR };
const presence = (s: ReturnType<typeof bassSong>) =>
  bassPresence(s.channels, SR, s.events, bassOpts);

describe('bassEvents', () => {
  test('filter table', () => {
    const base = { freq: 55 };
    const kept = (o: Partial<RenderEvent>) => bassEvents([ev({ ...base, ...o })]).length === 1;
    expect(kept({})).toBe(true);
    expect(kept({ freq: 130 })).toBe(true);
    expect(kept({ freq: 20 })).toBe(true);
    expect(kept({ s: 'white' })).toBe(false);
    expect(kept({ s: 'crackle' })).toBe(false);
    expect(kept({ s: 'whitey' })).toBe(true);
    expect(kept({ penv: 0.5 })).toBe(false);
    expect(kept({ attack: 0.3 })).toBe(false);
    expect(kept({ attack: 0.29 })).toBe(true);
    expect(kept({ gain: 0.04 })).toBe(false);
    expect(kept({ gain: 0.05 })).toBe(true);
    expect(kept({ freq: 131 })).toBe(false);
    expect(kept({ freq: 19 })).toBe(false);
    expect(kept({ freq: null })).toBe(false);
    expect(kept({ freq: NaN })).toBe(false);
    expect(bassEvents([ev({ freq: 200 })], { maxHz: 250 })).toHaveLength(1);
    expect(bassEvents([ev({ freq: 55, gain: 0.1 })], { minGain: 0.2 })).toHaveLength(0);
  });
});

describe('bassPresence and bassGate', () => {
  test('clean: pass, 16 checked, no misses', () => {
    const p = presence(bassSong());
    expect(p.checked).toBe(16);
    expect(p.misses).toEqual([]);
    expect(p.referenceRms).toBeGreaterThan(0);
    const g = bassGate(p, BAR);
    expect(g).toMatchObject({ gate: 'bass', verdict: 'pass', reasons: [], warnings: [] });
    expect(g.measurements).toMatchObject({
      checked: 16,
      misses: 0,
      deepMisses: 0,
      fraction: 0,
      worstRelDb: null,
      clusters: [],
    });
  });

  test('mutation: bars 2 and 3 dropped (4 notes) is a deep cluster: fail', () => {
    const p = presence(bassSong((i) => (i >= 4 && i < 8 ? 0 : 1)));
    expect(p.misses.map((m) => m.bar)).toEqual([2, 2, 3, 3]);
    expect(p.misses[0]).toMatchObject({ tSec: 4, hz: 55, orbit: 3, s: 'sawtooth' });
    expect(p.misses[0]!.relDb).toBeLessThan(-100);
    const g = bassGate(p, BAR);
    expect(g.verdict).toBe('fail');
    expect(g.measurements.clusters).toEqual([{ orbit: 3, fromBar: 2, toBar: 3, count: 4 }]);
    expect(g.reasons[0]).toBe(
      `orbit 3: 4 bass notes missing (<= -${BASS_CLUSTER_DEEP_DB} dB) in bars 2-3 (4.00s-8.00s)`,
    );
    expect(g.warnings).toEqual([]);
  });

  test('one dropped note: pass with one warning', () => {
    const g = bassGate(presence(bassSong((i) => (i === 9 ? 0 : 1))), BAR);
    expect(g.verdict).toBe('pass');
    expect(g.warnings).toHaveLength(1);
    expect(g.warnings[0]).toMatch(/^isolated bass miss at bar 4, orbit 3, -\d+(\.\d+)? dB$/);
  });

  test('4 dropped notes spread over bars 0, 3, 5, 7: no cluster, fails on the fraction', () => {
    const drop = [0, 6, 10, 14];
    const g = bassGate(presence(bassSong((i) => (drop.includes(i) ? 0 : 1))), BAR);
    expect(g.measurements.clusters).toEqual([]);
    expect(g.measurements.fraction).toBe(0.25);
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual(['4/16 bass notes missing (25% > 20%)']);
    expect(g.warnings).toHaveLength(4);
    const two = bassGate(presence(bassSong((i) => (i === 0 || i === 10 ? 0 : 1))), BAR);
    expect(two.verdict).toBe('pass');
    expect(two.measurements.fraction).toBe(0.125);
  });

  test('shallow misses never cluster; the same notes 20 dB down do', () => {
    // Calibration set BASS_CLUSTER_DEEP_DB = BASS_MISS_DB (16), so a shallow band (a miss, but not
    // deep) only exists with a deeper cluster rule: 18 dB here.
    const rule = { clusterDeepDb: 18 };
    // three notes in bars 2-3 (3/16, under the fraction line)
    const three = (d: number) => bassSong((i) => (i >= 4 && i < 7 ? db(d) : 1));
    const shallow = bassGate(presence(three(-17)), BAR, rule);
    expect(shallow.verdict).toBe('pass');
    expect(shallow.measurements.deepMisses).toBe(0);
    expect(shallow.warnings).toHaveLength(3);
    expect(shallow.warnings.every((w) => w.endsWith('(shallow, likely phase interaction)'))).toBe(
      true,
    );
    const deep = bassGate(presence(three(-20)), BAR, rule);
    expect(deep.verdict).toBe('fail');
    expect(deep.measurements.clusters).toEqual([{ orbit: 3, fromBar: 2, toBar: 3, count: 3 }]);
  });

  test('periodic shallow grid (every second note): only the fraction rule sees it', () => {
    const grid = (d: number) => presence(bassSong((i) => (i % 2 ? db(d) : 1)));
    // isolating the depth filter: with the fraction rule off, -13 dB passes with warnings
    // (cluster depth 18 dB so that -17 dB misses are shallow, see the test above)
    const off = { fractionMax: 1, clusterDeepDb: 18 };
    const g17 = bassGate(grid(-17), BAR, off);
    expect(g17.verdict).toBe('pass');
    expect(g17.warnings).toHaveLength(8);
    const g20 = bassGate(grid(-20), BAR, off);
    expect(g20.verdict).toBe('fail');
    expect(g20.measurements.clusters).toEqual([{ orbit: 3, fromBar: 0, toBar: 7, count: 8 }]);
    // 8/16 shallow misses is 50%: the default fraction rule fails it
    expect(bassGate(grid(-17), BAR, { clusterDeepDb: 18 }).reasons).toEqual([
      '8/16 bass notes missing (50% > 20%)',
    ]);
    // shallower than BASS_MISS_DB is no miss at all
    expect(bassGate(grid(-13), BAR).measurements.misses).toBe(0);
  });

  test('fewer than BASS_MIN_NOTES eligible notes is unverifiable', () => {
    const s = bassSong();
    const p = bassPresence(s.channels, SR, s.events.slice(0, 4), bassOpts);
    expect(p.checked).toBe(4);
    const g = bassGate(p, BAR);
    expect(g.verdict).toBe('unverifiable');
    expect(g.reasons).toEqual(['4 eligible bass notes']);
    expect(bassGate(p, BAR, { minNotes: 4 }).verdict).toBe('pass');
  });

  test('no eligible notes: checked 0, reference 0', () => {
    const s = bassSong();
    expect(bassPresence(s.channels, SR, [], bassOpts)).toEqual({
      checked: 0,
      misses: [],
      referenceRms: 0,
    });
  });

  test('gain normalisation: gain 0.2 notes at 0.2 amplitude score like gain 1 notes', () => {
    const p = presence(bassSong(undefined, (i) => (i % 2 ? 0.2 : 1)));
    expect(p.checked).toBe(16);
    expect(p.misses).toEqual([]);
  });

  test('notes outside the buffer are skipped; offsetSec shifts the buffer', () => {
    const s = bassSong();
    const tail = s.channels.map((c) => c.slice(8 * SR));
    const p = bassPresence(tail, SR, s.events, { ...bassOpts, offsetSec: 8 });
    expect(p.checked).toBe(8);
    const half = bassSong((i) => (i === 12 ? 0 : 1));
    const q = bassPresence(
      half.channels.map((c) => c.slice(8 * SR)),
      SR,
      half.events,
      { ...bassOpts, offsetSec: 8 },
    );
    expect(q.misses.map((m) => [m.tSec, m.bar])).toEqual([[12, 6]]);
    // the last note's window would run past a buffer cut 0.1 s after its onset
    const cut = s.channels.map((c) => c.slice(0, Math.round(15.1 * SR)));
    expect(bassPresence(cut, SR, s.events, bassOpts).checked).toBe(15);
  });

  test('rule overrides: cluster size, span and depth', () => {
    const p = presence(bassSong((i) => (i === 4 || i === 10 ? 0 : 1)));
    expect(bassGate(p, BAR).verdict).toBe('pass');
    expect(bassGate(p, BAR, { clusterMin: 2 }).verdict).toBe('fail'); // bars 2 and 5: span 3 <= 3
    expect(bassGate(p, BAR, { clusterMin: 2 }).measurements.clusters).toEqual([
      { orbit: 3, fromBar: 2, toBar: 5, count: 2 },
    ]);
    expect(bassGate(p, BAR, { clusterMin: 2, clusterSpanBars: 3 }).verdict).toBe('pass');
    expect(bassGate(p, BAR, { fractionMax: 0.1 }).verdict).toBe('fail');
    expect(bassGate(p, BAR, { fractionMax: 0.1, fractionMinNotes: 17 }).verdict).toBe('pass');
  });

  test('warnings cap at 8 plus a count', () => {
    const misses = Array.from({ length: 11 }, (_, i) => ({
      tSec: i * 8,
      bar: i * 4,
      hz: 55,
      orbit: 1,
      s: 'sine',
      relDb: -40,
    }));
    const g = bassGate({ checked: 200, misses, referenceRms: 0.1 }, BAR);
    expect(g.verdict).toBe('pass');
    expect(g.warnings).toHaveLength(9);
    expect(g.warnings[8]).toBe('+3');
    expect(g.measurements.worstRelDb).toBe(-40);
  });
});

// ---- mix

const MIX_SEC = 4 * BAR;
const MIX_FRAMES = MIX_SEC * SR;

/** A decaying 60 Hz kick starting at full `amp` (a 0.8 step) at each time. */
function kicks(times: number[], sec: number, amp = 0.8): Float32Array {
  const out = new Float32Array(Math.round(sec * SR));
  for (const t of times) {
    const i0 = Math.round(t * SR);
    for (let i = 0; i < 0.3 * SR && i0 + i < out.length; i++) {
      out[i0 + i] += amp * Math.exp(-i / (0.05 * SR)) * Math.cos((2 * Math.PI * 60 * i) / SR);
    }
  }
  return out;
}

const bed = (sec = MIX_SEC, seed = 3) => noiseSeeded(seed, Math.round(sec * SR), 0.01);
const add = (a: Float32Array, b: Float32Array) => a.map((v, i) => v + (b[i] ?? 0));
const kickTrack = () => add(bed(), kicks([0, 2, 4, 6], MIX_SEC));
const percussive = deriveOpening([ev({ tSec: 0, attack: 0, gain: 1 })]);
const mixIn = (o: Partial<MixInput> = {}): MixInput => ({
  sampleRate: SR,
  expectedFrames: MIX_FRAMES,
  barSec: BAR,
  seamSecs: [4],
  ...o,
});
/** The bed plus a 0.8 DC step from `atSec` on. */
const stepAt = (atSec: number) => bed().map((v, i) => (i >= atSec * SR ? v + 0.8 : v));

describe('deriveOpening', () => {
  test('a loud fast voice at t = 0 is a percussive opener', () => {
    expect(percussive).toEqual({
      expectedFirstAudibleSec: 0,
      hasPercussiveOpener: true,
      hasAudibleVoice: true,
    });
  });

  test('a 0.015-gain square under slow pads is not percussive', () => {
    const o = deriveOpening([
      ev({ tSec: 0, attack: 0.001, gain: 0.015 }),
      ev({ tSec: 0, attack: 1.5, gain: 0.5 }),
    ]);
    expect(o.hasPercussiveOpener).toBe(false);
    expect(o.expectedFirstAudibleSec).toBeCloseTo((0.001 * 0.01) / 0.015, 9);
  });

  test('attack ramp crossing time is attack * thr / gain', () => {
    const o = deriveOpening([ev({ tSec: 1, attack: 2, gain: 0.5 })]);
    expect(o.expectedFirstAudibleSec).toBeCloseTo(1 + 2 * 0.02, 9);
    expect(o.hasPercussiveOpener).toBe(false);
    expect(
      deriveOpening([ev({ tSec: 1, attack: 2, gain: 0.5 })], -20).expectedFirstAudibleSec,
    ).toBeCloseTo(1.4, 9);
  });

  test('percussive bounds: t < 0.01, attack < 0.02, gain >= 10x threshold', () => {
    const p = (o: Partial<RenderEvent>) =>
      deriveOpening([ev({ tSec: 0, attack: 0, gain: 1, ...o })]).hasPercussiveOpener;
    expect(p({ tSec: 0.01 })).toBe(false);
    expect(p({ attack: 0.02 })).toBe(false);
    expect(p({ gain: 0.1 })).toBe(true);
    expect(p({ gain: 0.099 })).toBe(false);
  });

  test('no audible events: {0, false}, no audible voice', () => {
    expect(deriveOpening([])).toEqual({
      expectedFirstAudibleSec: 0,
      hasPercussiveOpener: false,
      hasAudibleVoice: false,
    });
    expect(deriveOpening([ev({ gain: 0.01 })]).hasAudibleVoice).toBe(false);
  });
});

describe('verifyMix', () => {
  test('clean 4-bar kick track with an exact frame count passes without warnings', () => {
    const g = verifyMix(st(kickTrack()), mixIn({ opening: percussive }));
    expect(g.gate).toBe('mix');
    expect(g.reasons).toEqual([]);
    expect(g.warnings).toEqual([]);
    expect(g.verdict).toBe('pass');
    expect(g.measurements).toMatchObject({
      frames: MIX_FRAMES,
      expectedFrames: MIX_FRAMES,
      driftMs: 0,
      clipFraction: 0,
      firstAudibleSec: 0,
      expectedFirstAudibleSec: 0,
      clicks: 0,
    });
    // the seam reads high (a kick on it) but its neighbours read the same
    expect(g.measurements.worstSeamRatio as number).toBeGreaterThan(30);
    expect(g.measurements.peakDb as number).toBeLessThan(0);
  });

  test('NaN at frame 1000 fails and skips the level and click checks', () => {
    const x = kickTrack();
    x[1000] = NaN;
    const g = verifyMix([kickTrack(), x], mixIn());
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual(['1 non-finite samples, first at frame 1000 ch 1']);
    expect(g.measurements).toMatchObject({ peak: null, clipFraction: null, clicks: null });
    expect(g.measurements.worstSeamRatio).toBeNull();
    expect(verifyMix(st(kickTrack().fill(Infinity, 5, 7)), mixIn()).reasons[0]).toBe(
      '4 non-finite samples, first at frame 5 ch 0',
    );
  });

  test('one frame short or long fails with the drift', () => {
    const short = verifyMix(st(kickTrack().slice(1)), mixIn());
    expect(short.verdict).toBe('fail');
    expect(short.reasons).toEqual([
      `frame count ${MIX_FRAMES - 1} != expected ${MIX_FRAMES} (drift -0.023 ms)`,
    ]);
    expect(short.measurements.driftMs).toBe(-0.023);
    const long = verifyMix(st(concat(kickTrack(), new Float32Array(1))), mixIn());
    expect(long.reasons[0]).toContain('drift 0.023 ms');
  });

  test('channels of different lengths fail', () => {
    const g = verifyMix([kickTrack(), kickTrack().slice(5)], mixIn());
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual([`channel lengths differ: ${MIX_FRAMES}, ${MIX_FRAMES - 5}`]);
  });

  test('first 200 ms zeroed under a percussive opener fails the opening', () => {
    const x = kickTrack().fill(0, 0, Math.round(0.2 * SR));
    const g = verifyMix(st(x), mixIn({ opening: percussive }));
    expect(g.verdict).toBe('fail');
    expect(g.reasons).toEqual(['opening at 0.2s, percussive opener needs < 0.01s']);
    expect(g.measurements.firstAudibleSec).toBe(0.2);
  });

  test('fade-in pad: on time passes, 0.6 s late fails', () => {
    const opening = deriveOpening([ev({ tSec: 0, attack: 1.5, gain: 0.5 })]);
    const pad = (delaySec: number) => {
      const x = new Float32Array(MIX_FRAMES);
      const d = Math.round(delaySec * SR);
      for (let i = 0; i + d < x.length; i++) {
        x[i + d] = 0.5 * Math.min(1, i / (1.5 * SR)) * Math.sin((2 * Math.PI * 220 * i) / SR);
      }
      return st(x);
    };
    const ok = verifyMix(pad(0), mixIn({ opening, seamSecs: [] }));
    expect(ok.verdict).toBe('pass');
    expect(ok.measurements.firstAudibleSec as number).toBeLessThan(0.05);
    expect(ok.measurements.expectedFirstAudibleSec).toBe(0.03);
    const late = verifyMix(pad(0.6), mixIn({ opening, seamSecs: [] }));
    expect(late.verdict).toBe('fail');
    expect(late.reasons[0]).toMatch(/^opening at 0\.6\d\ds, later than expected 0\.03s \+ 0\.25s$/);
  });

  test('a DC step at a seam over a quiet bed is a click', () => {
    const g = verifyMix(st(stepAt(4)), mixIn());
    expect(g.verdict).toBe('fail');
    expect(g.measurements.clicks).toBe(1);
    expect(g.reasons[0]).toMatch(/^1 seam clicks at 4s \(worst ratio \d+(\.\d+)? > 30\)$/);
  });

  test('a click with no neighbouring bar lines to compare is judged on the ratio alone', () => {
    const g = verifyMix(st(stepAt(4)), mixIn({ barSec: 10 }));
    expect(g.measurements.clicks).toBe(1);
    expect(verifyMix(st(stepAt(4)), mixIn({ barSec: 0 })).measurements.clicks).toBe(1);
  });

  test('neighbouring seams are not used as neighbours', () => {
    // steps at 2, 4 and 6 s, all seams: each compares only against bar lines that are not seams
    const x = bed().map((v, i) => v + 0.8 * Math.floor(i / (2 * SR)));
    const g = verifyMix(st(x), mixIn({ seamSecs: [2, 4, 6] }));
    expect(g.measurements.clicks).toBe(3);
  });

  test('a seam outside the buffer is ignored', () => {
    const g = verifyMix(st(stepAt(4)), mixIn({ seamSecs: [-1, 0, MIX_SEC, 9] }));
    expect(g.verdict).toBe('pass');
    expect(g.measurements.clicks).toBe(0);
    expect(g.measurements.worstSeamRatio).toBe(0);
  });

  test('stereo: a click in the right channel only is caught', () => {
    const g = verifyMix([bed(), stepAt(4)], mixIn());
    expect(g.verdict).toBe('fail');
    expect(g.measurements.clicks).toBe(1);
  });

  test('hot mix: peak 1.7 passes with a clip warning', () => {
    const g = verifyMix(st(sine(220, 1.7, MIX_SEC)), mixIn({ seamSecs: [] }));
    expect(g.verdict).toBe('pass');
    expect(g.measurements.peak as number).toBeCloseTo(1.7, 3);
    expect(g.measurements.clipFraction as number).toBeGreaterThan(0);
    expect(g.warnings).toHaveLength(1);
    expect(g.warnings[0]).toMatch(
      /^peak 1\.7 \(4\.61 dBFS\) over full scale; clip fraction \d+(\.\d+)?%$/,
    );
  });

  test('silent buffer: passes without an opening; with one it depends on the schedule', () => {
    const z = st(new Float32Array(MIX_FRAMES));
    const g = verifyMix(z, mixIn());
    expect(g.verdict).toBe('pass');
    expect(g.warnings).toEqual([]);
    expect(g.measurements).toMatchObject({ peak: 0, peakDb: null, firstAudibleSec: null });
    const silentSchedule = verifyMix(z, mixIn({ opening: deriveOpening([]) }));
    expect(silentSchedule.verdict).toBe('pass');
    expect(silentSchedule.warnings).toEqual(['schedule has no audible voice']);
    const lost = verifyMix(z, mixIn({ opening: percussive }));
    expect(lost.verdict).toBe('fail');
    expect(lost.reasons).toEqual(['silent opening']);
    const legacy = { expectedFirstAudibleSec: 0, hasPercussiveOpener: false };
    expect(verifyMix(z, mixIn({ opening: legacy })).reasons).toEqual(['silent opening']);
  });

  test('zero channels is a programmer error', () => {
    expect(() => verifyMix([], mixIn())).toThrow(RangeError);
  });

  test('several failures are all reported', () => {
    const x = stepAt(4).slice(1);
    const g = verifyMix(st(x), mixIn({ opening: percussive }));
    expect(g.reasons).toHaveLength(3);
  });
});

describe('mixed-meter gates', () => {
  const clock = (backToFour?: number) => {
    const meterChanges = [{ bar: 2, meter: '3/8' }];
    if (backToFour !== undefined) meterChanges.push({ bar: backToFour, meter: '4/4' });
    const structure = buildStructure({ sections: [{ name: 'phrase', bars: 24 }], meterChanges });
    return renderTiming(structure, LINEAR_SEGMENTS, structure.cps)!;
  };

  test('bass misses, cluster bounds and stem notes follow the scheduled bars', () => {
    const timing = clock();
    const { channels, events } = bassSong((i) => (i >= 3 && i <= 5 ? 0 : 1));
    const scheduled = events.map((e) => ({ ...e, bar: Math.floor(timing.cycleAt(e.tSec)) }));
    const p = bassPresence(channels, SR, scheduled, { missDb: BASS_MISS_DB });
    expect(p.misses.map((m) => m.bar)).toEqual([2, 3, 5]);
    const bass = bassGate(p, timing.secondsAt);
    expect(bass.measurements.clusters).toEqual([{ orbit: 3, fromBar: 2, toBar: 5, count: 3 }]);
    expect(bass.reasons[0]).toContain('bars 2-5 (2.75s-5.75s)');
    const stems = stemsOnsetScores(channels, SR, scheduled);
    expect(stems.notes.slice(3, 6).map((n) => n.bar)).toEqual([2, 3, 5]);
  });

  test('a loud downbeat at a seam is compared with the actual neighboring bars of a later window', () => {
    // Starting at authored bar 1 (2 s), the next two downbeats are 0.75 s and 1.5 s into the buffer.
    const input = mixIn({ seamSecs: [0.75], meteredWindow: { timing: clock(4), fromBar: 1 } });
    const withNeighbor = add(bed(), kicks([0.75, 1.5], MIX_SEC));
    expect(verifyMix(st(withNeighbor), input).measurements.clicks).toBe(0);
    expect(verifyMix(st(add(bed(), kicks([0.75], MIX_SEC))), input).measurements.clicks).toBe(1);
  });
});

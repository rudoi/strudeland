import { describe, expect, test } from 'bun:test';
import { windowFrames, type RenderRequest, type RenderResult } from '../src/lib/render/contract';
import {
  assembleWindow,
  chunkRequest,
  contentOf,
  headOf,
  overlapOf,
  planChunks,
  renderChunkPlans,
  stochasticCrossings,
  totalContentSec,
  type ChunkPlan,
} from '../src/lib/render/chunks';
import { lookbackBars, lookbackSeconds, scaleChunkBars } from '../src/lib/render/lookback';
import type { RenderEvent } from '../src/lib/render/schedule';
import { CHUNK_MIN_BARS, SEAM_OVERLAP_BARS } from '../src/lib/render/thresholds';
import { ev, room } from './helpers/render-events';
import { SR } from './helpers/render-signals';

const BAR = 2.4; // 100 bpm 4/4
const plan = (totalBars: number, chunkBars: number, lookbackBars: number, barSec = BAR) =>
  planChunks({ fromBar: 0, toBar: totalBars, chunkBars, lookbackBars, barSec });

describe('planChunks', () => {
  test('first chunk has no lookback', () => {
    const [c0] = plan(16, 8, 2);
    expect(c0!.winBeginBar).toBe(0);
    expect(c0!.lookbackSec).toBe(0);
    expect(c0!.contentStartBar).toBe(0);
    expect(c0!.contentEndBar).toBe(8);
    expect(c0!.contentSec).toBeCloseTo(8 * BAR, 9);
    expect(c0!.seamCrossings).toBe(0);
  });

  test('later chunks carry the full lookback', () => {
    const p = plan(24, 8, 2);
    expect(p).toHaveLength(3);
    expect(p.map((c) => c.index)).toEqual([0, 1, 2]);
    expect(p[1]!.winBeginBar).toBe(6);
    expect(p[1]!.lookbackSec).toBeCloseTo(2 * BAR, 9);
    expect(p[1]!.contentStartBar).toBe(8);
    expect(p[2]!.winBeginBar).toBe(14);
  });

  test('a final partial chunk', () => {
    const p = plan(53, 8, 2);
    expect(p).toHaveLength(7);
    const last = p[p.length - 1]!;
    expect(last.contentStartBar).toBe(48);
    expect(last.contentEndBar).toBe(53);
    expect(last.contentSec).toBeCloseTo(5 * BAR, 9);
  });

  test('total content equals the whole range', () => {
    expect(totalContentSec(plan(53, 8, 2))).toBeCloseTo(53 * BAR, 6);
    expect(totalContentSec([])).toBe(0);
  });

  test('content windows are contiguous', () => {
    const p = plan(50, 7, 3);
    for (let i = 1; i < p.length; i++) expect(p[i]!.contentStartBar).toBe(p[i - 1]!.contentEndBar);
  });

  test('lookback clamps at bar 0', () => {
    const p = plan(20, 4, 4);
    expect(p[1]!.winBeginBar).toBe(0);
    expect(p[1]!.lookbackSec).toBeCloseTo(4 * BAR, 9);
  });

  test('a single chunk covering the whole range', () => {
    const p = plan(5, 8, 2);
    expect(p).toHaveLength(1);
    expect(p[0]!.contentEndBar).toBe(5);
    expect(p[0]!.overlapBars).toBe(0);
  });

  test('invalid input throws', () => {
    const ok = { fromBar: 0, toBar: 16, chunkBars: 8, lookbackBars: 2, barSec: BAR };
    expect(() => planChunks({ ...ok, toBar: 0 })).toThrow(/toBar/);
    expect(() => planChunks({ ...ok, fromBar: 16 })).toThrow(/toBar/);
    expect(() => planChunks({ ...ok, toBar: 16.5 })).toThrow(/toBar/);
    expect(() => planChunks({ ...ok, fromBar: -1 })).toThrow(/fromBar/);
    expect(() => planChunks({ ...ok, fromBar: 0.5 })).toThrow(/fromBar/);
    expect(() => planChunks({ ...ok, chunkBars: 0 })).toThrow(/chunkBars/);
    expect(() => planChunks({ ...ok, chunkBars: 2.5 })).toThrow(/chunkBars/);
    expect(() => planChunks({ ...ok, lookbackBars: -1 })).toThrow(/lookbackBars/);
    expect(() => planChunks({ ...ok, barSec: 0 })).toThrow(/barSec/);
    expect(() => planChunks({ ...ok, barSec: NaN })).toThrow(/barSec/);
    expect(() => planChunks({ ...ok, seamSearchBars: -1 })).toThrow(/seamSearchBars/);
    expect(() => planChunks({ ...ok, overlapBars: -1 })).toThrow(/overlapBars/);
    expect(() => planChunks({ ...ok, overlapBars: 0.5 })).toThrow(/overlapBars/);
  });

  test('an excerpt gets lookback on its first chunk', () => {
    const p = planChunks({ fromBar: 20, toBar: 36, chunkBars: 8, lookbackBars: 4, barSec: 2 });
    expect(p.map((c) => [c.contentStartBar, c.contentEndBar])).toEqual([
      [20, 28],
      [28, 36],
    ]);
    expect(p[0]!.winBeginBar).toBe(16);
    expect(p[0]!.lookbackSec).toBeCloseTo(8, 9);
    expect(p[0]!.seamCrossings).toBe(0);
  });

  test('an excerpt near the start clamps its lookback at bar 0', () => {
    const p = planChunks({ fromBar: 2, toBar: 10, chunkBars: 8, lookbackBars: 4, barSec: 2 });
    expect(p[0]!.winBeginBar).toBe(0);
    expect(p[0]!.lookbackSec).toBeCloseTo(4, 9);
  });

  test('overlap bars are 1, 1, 0 for three chunks', () => {
    expect(plan(24, 8, 2).map((c) => c.overlapBars)).toEqual([1, 1, 0]);
  });

  test('overlap override and 0 disables it', () => {
    const base = { fromBar: 0, toBar: 24, chunkBars: 8, lookbackBars: 2, barSec: 2 };
    expect(planChunks({ ...base, overlapBars: 2 }).map((c) => c.overlapBars)).toEqual([2, 2, 0]);
    expect(planChunks({ ...base, overlapBars: 0 }).map((c) => c.overlapBars)).toEqual([0, 0, 0]);
  });

  test('overlap is clipped to the bars that exist after the chunk', () => {
    // chunk 0 ends at 8, one bar remains: an overlap of 3 is clipped to 1
    const p = planChunks({
      fromBar: 0,
      toBar: 9,
      chunkBars: 8,
      lookbackBars: 0,
      barSec: 2,
      overlapBars: 3,
    });
    expect(p.map((c) => c.overlapBars)).toEqual([1, 0]);
  });

  test('odd meters and fast tempi keep the sums exact', () => {
    for (const barSec of [5.4545, 0.4]) {
      const p = planChunks({ fromBar: 3, toBar: 40, chunkBars: 8, lookbackBars: 3, barSec });
      expect(Math.abs(totalContentSec(p) - 37 * barSec)).toBeLessThan(1e-9);
      for (const c of p) {
        expect(Math.abs(c.lookbackSec - (c.contentStartBar - c.winBeginBar) * barSec)).toBeLessThan(
          1e-9,
        );
      }
    }
  });
});

describe('planChunks seam avoidance', () => {
  const base = { fromBar: 0, toBar: 24, chunkBars: 8, lookbackBars: 2, barSec: 2 };

  test('without avoidSeamBars the plan is unchanged', () => {
    const p = planChunks(base);
    expect(p.map((c) => c.contentEndBar)).toEqual([8, 16, 24]);
    expect(p.every((c) => c.seamCrossings === 0)).toBe(true);
  });

  test('a dirty bar 8 nudges to 9 on a tie', () => {
    const p = planChunks({ ...base, avoidSeamBars: { 8: 3 } });
    expect(p[0]!.contentEndBar).toBe(9);
    expect(p[1]!.contentStartBar).toBe(9);
    expect(p[1]!.seamCrossings).toBe(0);
  });

  test('prefers the nearest clean bar in either direction', () => {
    const p = planChunks({ ...base, avoidSeamBars: { 8: 1, 9: 1 } });
    expect(p[0]!.contentEndBar).toBe(7);
  });

  test('all candidates dirty takes the least dirty and reports it', () => {
    const p = planChunks({ ...base, avoidSeamBars: { 6: 5, 7: 4, 8: 9, 9: 2, 10: 7 } });
    expect(p[0]!.contentEndBar).toBe(9);
    expect(p[1]!.seamCrossings).toBe(2);
  });

  test('all candidates equally dirty keeps the target', () => {
    const p = planChunks({ ...base, avoidSeamBars: { 6: 1, 7: 1, 8: 1, 9: 1, 10: 1 } });
    expect(p[0]!.contentEndBar).toBe(8);
    expect(p[1]!.seamCrossings).toBe(1);
  });

  test('a candidate at or before the chunk start is never chosen', () => {
    // chunk 8 starts at bar 8: target 9 is dirty, 10 equally dirty, 8 clean but = the start
    const p = planChunks({
      ...base,
      chunkBars: 1,
      toBar: 12,
      seamSearchBars: 1,
      avoidSeamBars: { 9: 2, 10: 2 },
    });
    for (const c of p) expect(c.contentEndBar).toBeGreaterThan(c.contentStartBar);
    expect(p[8]!.contentStartBar).toBe(8);
    expect(p[8]!.contentEndBar).toBe(9);
    expect(p[9]!.seamCrossings).toBe(2);
  });

  test('the final boundary never moves', () => {
    const p = planChunks({ ...base, toBar: 3, avoidSeamBars: { 3: 9 } });
    expect(p).toHaveLength(1);
    expect(p[0]!.contentEndBar).toBe(3);
  });

  test('a nudge never skips past the final bar', () => {
    // the clean end bar 9 wins over a dirty target 8
    const p = planChunks({ ...base, toBar: 9, avoidSeamBars: { 8: 2, 7: 1 } });
    expect(p.map((c) => c.contentEndBar)).toEqual([9]);
    // bar 10 (clean, past the end) is skipped, so the least dirty in range (6) wins
    const q = planChunks({ ...base, toBar: 9, avoidSeamBars: { 6: 1, 7: 2, 8: 2, 9: 2 } });
    expect(q.map((c) => c.contentEndBar)).toEqual([6, 9]);
  });

  test('still covers every bar exactly once after nudging', () => {
    const p = planChunks({
      ...base,
      toBar: 40,
      avoidSeamBars: { 8: 2, 16: 1, 17: 1, 24: 3, 32: 1 },
    });
    expect(p[0]!.contentStartBar).toBe(0);
    expect(p[p.length - 1]!.contentEndBar).toBe(40);
    for (let i = 1; i < p.length; i++) expect(p[i]!.contentStartBar).toBe(p[i - 1]!.contentEndBar);
    expect(totalContentSec(p)).toBeCloseTo(80, 10);
  });

  test('seamSearchBars 0 disables nudging', () => {
    const p = planChunks({ ...base, avoidSeamBars: { 8: 3 }, seamSearchBars: 0 });
    expect(p[0]!.contentEndBar).toBe(8);
    expect(p[1]!.seamCrossings).toBe(3);
  });

  test('chunk 0 never reports crossings, even on a dirty fromBar', () => {
    const p = planChunks({ ...base, fromBar: 4, avoidSeamBars: { 4: 5 } });
    expect(p[0]!.seamCrossings).toBe(0);
  });
});

describe('stochasticCrossings', () => {
  test('a voice crossing bar lines 4, 6 and 8 counts bars 2, 3, 4', () => {
    const e = ev({ tSec: 3.5, durSec: 5, release: 0.5, stochastic: true });
    expect(stochasticCrossings([e], 2)).toEqual({ 2: 1, 3: 1, 4: 1 });
  });

  test('a voice ending exactly on a bar line does not count it', () => {
    const e = ev({ tSec: 3.5, durSec: 4, release: 0.5, stochastic: true }); // ends at 8.0
    expect(stochasticCrossings([e], 2)).toEqual({ 2: 1, 3: 1 });
  });

  test('a voice starting on a bar line does not count it', () => {
    const e = ev({ tSec: 4, durSec: 1, release: 0, stochastic: true });
    expect(stochasticCrossings([e], 2)).toEqual({});
  });

  test('overlapping voices add up', () => {
    const a = ev({ tSec: 1, durSec: 2, release: 0, stochastic: true });
    const b = ev({ tSec: 1.5, durSec: 3, release: 0, stochastic: true });
    expect(stochasticCrossings([a, b], 2)).toEqual({ 1: 2, 2: 1 });
  });

  test('non-stochastic events count nothing', () => {
    expect(stochasticCrossings([ev({ tSec: 0.5, durSec: 10 })], 2)).toEqual({});
  });

  test('throws on barSec <= 0', () => {
    expect(() => stochasticCrossings([], 0)).toThrow(RangeError);
  });
});

// ---- slicing helpers

const CPS = 0.6524;
const sampleAt = (layoutFrame: number, c: number) => layoutFrame + c * 1_000_000;

/** A chunk result whose samples are their layout frame index (plus 1e6 per channel), with the
 *  lookback and overlap a renderer would leave in the buffer. */
function chunkResult(
  p: Pick<ChunkPlan, 'contentStartBar' | 'contentEndBar' | 'overlapBars' | 'lookbackSec'>,
  opts: { channels?: number; cps?: number; ok?: boolean; short?: number } = {},
): RenderResult {
  const cps = opts.cps ?? CPS;
  const nch = opts.channels ?? 2;
  const request: RenderRequest = {
    code: 'x',
    layout: 'linear',
    fromBar: p.contentStartBar,
    toBar: p.contentEndBar,
    overlapBars: p.overlapBars,
    lookbackSec: p.lookbackSec,
    mode: nch > 2 ? 'stems' : 'mix',
    sampleRate: 44100,
  };
  const windowStartFrame = Math.round(p.lookbackSec * SR);
  const frames =
    windowStartFrame +
    windowFrames(p.contentStartBar, p.contentEndBar + p.overlapBars, cps, SR) -
    (opts.short ?? 0);
  const first = Math.round((p.contentStartBar * SR) / cps) - windowStartFrame;
  const channels = Array.from({ length: nch }, (_, c) => {
    const ch = new Float32Array(frames);
    for (let i = 0; i < frames; i++) ch[i] = sampleAt(first + i, c);
    return ch;
  });
  return {
    channels,
    sampleRate: SR,
    frames,
    windowStartFrame,
    lookbackSecUsed: p.lookbackSec,
    bufferStartSec: p.contentStartBar / cps - p.lookbackSec,
    cps,
    events: [],
    stemPairs: nch / 2,
    errors: [],
    ok: opts.ok ?? true,
    timing: { evalMs: 1, renderMs: 2, totalMs: 3 },
    request,
  };
}

const plans = () =>
  planChunks({ fromBar: 1, toBar: 13, chunkBars: 4, lookbackBars: 2, barSec: 1 / CPS });

describe('renderChunkPlans', () => {
  const CPS = 1 / BAR;
  /** The harness's inline recipe before it was shared, spelled out. */
  const inlineRecipe = (
    fromBar: number,
    toBar: number,
    events: RenderEvent[],
    overlapBars?: number,
  ): ChunkPlan[] => {
    const barSec = 1 / CPS;
    const lb = lookbackBars(lookbackSeconds(events), barSec);
    return planChunks({
      fromBar,
      toBar,
      chunkBars: scaleChunkBars(CHUNK_MIN_BARS, lb),
      lookbackBars: lb,
      barSec,
      avoidSeamBars: stochasticCrossings(events, barSec),
      ...(overlapBars === undefined ? {} : { overlapBars }),
    });
  };
  const eventSets: [string, RenderEvent[]][] = [
    ['no events', []],
    [
      'a long reverb tail',
      Array.from({ length: 40 }, (_, b) => ev({ tSec: b * BAR, durSec: 0.5, room: room(8) })),
    ],
    [
      'a stochastic voice crossing the first seam',
      [
        ev({ tSec: 0, durSec: 0.2 }),
        ev({ tSec: 7.5 * BAR, durSec: 1.2 * BAR, stochastic: true }),
        ev({ tSec: 15.5 * BAR, durSec: 1.2 * BAR, stochastic: true }),
      ],
    ],
  ];

  test.each(eventSets)('%s: equals the inline recipe, default overlap', (_name, events) => {
    const got = renderChunkPlans({ fromBar: 0, toBar: 40, events, cps: CPS });
    expect(got).toEqual(inlineRecipe(0, 40, events));
    expect(got.length).toBeGreaterThan(1);
    expect(got.slice(0, -1).every((p) => p.overlapBars === SEAM_OVERLAP_BARS)).toBe(true);
  });

  test.each(eventSets)('%s: equals the inline recipe, overlap 0', (_name, events) => {
    const got = renderChunkPlans({ fromBar: 3, toBar: 37, events, cps: CPS, overlapBars: 0 });
    expect(got).toEqual(inlineRecipe(3, 37, events, 0));
    expect(got.every((p) => p.overlapBars === 0)).toBe(true);
    expect(got[0]!.contentStartBar).toBe(3);
    expect(got.at(-1)!.contentEndBar).toBe(37);
  });

  test('the reverb tail scales the lookback and the chunk size', () => {
    const [, tail] = eventSets[1]!;
    const lb = lookbackBars(lookbackSeconds(tail), BAR);
    expect(lb).toBeGreaterThan(0);
    const got = renderChunkPlans({ fromBar: 0, toBar: 40, events: tail, cps: CPS });
    expect(got[1]!.winBeginBar).toBe(got[1]!.contentStartBar - lb);
    expect(got[0]!.contentEndBar).toBe(scaleChunkBars(CHUNK_MIN_BARS, lb));
  });

  test('the seam moves off the stochastic crossing', () => {
    const [, stochastic] = eventSets[2]!;
    const got = renderChunkPlans({ fromBar: 0, toBar: 40, events: stochastic, cps: CPS });
    expect(got[0]!.contentEndBar).not.toBe(CHUNK_MIN_BARS);
    expect(got.map((p) => p.seamCrossings)).toEqual(got.map(() => 0));
  });

  test('an explicit overlap of 2 reaches every chunk but the last', () => {
    const got = renderChunkPlans({ fromBar: 0, toBar: 24, events: [], cps: CPS, overlapBars: 2 });
    expect(got.map((p) => p.overlapBars)).toEqual([2, 2, 0]);
  });

  test.each([
    ['NaN cps', { cps: NaN }, /barSec must be > 0/],
    ['zero cps', { cps: 0 }, /barSec must be > 0/],
    ['negative cps', { cps: -1 }, /barSec must be > 0/],
    ['an empty window', { fromBar: 4, toBar: 4 }, /toBar must be an integer > fromBar/],
    ['a fractional bar', { toBar: 4.5 }, /toBar must be an integer > fromBar/],
    ['a negative overlap', { overlapBars: -1 }, /overlapBars must be a non-negative integer/],
  ] as const)('%s throws', (_name, o, message) => {
    expect(() => renderChunkPlans({ fromBar: 0, toBar: 8, events: [], cps: CPS, ...o })).toThrow(
      message,
    );
  });
});

describe('chunkRequest', () => {
  test('copies the base and overrides the window, overlap and lookback', () => {
    const base: RenderRequest = {
      code: 'song',
      layout: 'clip',
      fromBar: 0,
      toBar: 99,
      overlapBars: 7,
      lookbackSec: 'auto',
      mode: 'stems',
      sampleRate: 44100,
      mutation: { kind: 'dropOrbit', orbit: 2, bars: [3] },
      samples: { base: 'x' },
    };
    const p = plans()[1]!;
    expect(chunkRequest(base, p)).toEqual({
      ...base,
      fromBar: 5,
      toBar: 9,
      overlapBars: 1,
      lookbackSec: p.lookbackSec,
    });
    expect(base.fromBar).toBe(0);
  });
});

describe('contentOf, overlapOf, headOf', () => {
  test('contentOf drops the lookback and the overlap', () => {
    const p = plans()[1]!;
    const r = chunkResult(p);
    const c = contentOf(r);
    expect(c).toHaveLength(2);
    const start = Math.round((5 * SR) / CPS);
    const end = Math.round((9 * SR) / CPS);
    for (const [k, ch] of c.entries()) {
      expect(ch.length).toBe(windowFrames(5, 9, CPS, SR));
      expect(ch[0]).toBe(sampleAt(start, k));
      expect(ch[ch.length - 1]).toBe(sampleAt(end - 1, k));
    }
  });

  test('overlapOf of chunk k and headOf of chunk k+1 cover the same audio time', () => {
    const [p0, p1] = plans();
    const r0 = chunkResult(p0!);
    const r1 = chunkResult(p1!);
    const tail = overlapOf(r0);
    const head = headOf(r1, r0.request.overlapBars);
    expect(tail[0]!.length).toBe(windowFrames(5, 6, CPS, SR));
    expect(head[0]!.length).toBe(tail[0]!.length);
    for (let c = 0; c < 2; c++) expect(Array.from(head[c]!)).toEqual(Array.from(tail[c]!));
    expect(tail[0]![0]).toBe(sampleAt(Math.round((5 * SR) / CPS), 0));
  });

  test('overlapOf of the last chunk is empty', () => {
    const ps = plans();
    expect(overlapOf(chunkResult(ps[ps.length - 1]!))[0]!.length).toBe(0);
  });

  test('headOf 0 bars is empty, past the content throws', () => {
    const r = chunkResult(plans()[1]!);
    expect(headOf(r, 0)[0]!.length).toBe(0);
    expect(() => headOf(r, 5)).toThrow(RangeError);
    expect(() => headOf(r, -1)).toThrow(RangeError);
  });

  test('a short buffer throws', () => {
    const p = plans()[1]!;
    const shortOverlap = chunkResult(p, { short: 1 });
    expect(contentOf(shortOverlap)[0]!.length).toBe(windowFrames(5, 9, CPS, SR));
    expect(() => overlapOf(shortOverlap)).toThrow(RangeError);
    const shortContent = chunkResult(p, { short: windowFrames(9, 10, CPS, SR) + 1 });
    expect(() => contentOf(shortContent)).toThrow(RangeError);
    expect(() => headOf(shortContent, 1)).toThrow(RangeError);
  });
});

describe('assembleWindow', () => {
  test('three chunks telescope to the whole window with seams at the chunk starts', () => {
    const ps = plans();
    expect(ps).toHaveLength(3);
    const rs = ps.map((p) => chunkResult(p));
    const events = [ev({ tSec: 2 })];
    const w = assembleWindow(rs, events);
    expect(w.frames).toBe(windowFrames(1, 13, CPS, SR));
    expect(w.channels).toHaveLength(2);
    expect(w.channels.every((c) => c.length === w.frames)).toBe(true);
    const f0 = windowFrames(1, 5, CPS, SR);
    const f1 = windowFrames(5, 9, CPS, SR);
    expect(w.seamSecs).toEqual([f0 / SR, (f0 + f1) / SR]);
    const start = Math.round((1 * SR) / CPS);
    for (let c = 0; c < 2; c++) {
      const ch = w.channels[c]!;
      for (let i = 0; i < ch.length; i += 997) expect(ch[i]).toBe(sampleAt(start + i, c));
      expect(ch[ch.length - 1]).toBe(sampleAt(start + ch.length - 1, c));
    }
    expect(w.windowStartFrame).toBe(0);
    expect(w.lookbackSecUsed).toBe(0);
    expect(w.bufferStartSec).toBeCloseTo(1 / CPS, 12);
    expect(w.cps).toBe(CPS);
    expect(w.events).toBe(events);
    expect(w.ok).toBe(true);
    expect(w.timing).toEqual({ evalMs: 3, renderMs: 6, totalMs: 9 });
    expect(w.request).toEqual({ ...rs[0]!.request, toBar: 13, overlapBars: 0, lookbackSec: 0 });
    expect(w.stemPairs).toBe(1);
  });

  test('a single chunk has no seams', () => {
    const r = chunkResult({ contentStartBar: 0, contentEndBar: 4, overlapBars: 0, lookbackSec: 0 });
    const w = assembleWindow([r], []);
    expect(w.seamSecs).toEqual([]);
    expect(w.frames).toBe(windowFrames(0, 4, CPS, SR));
  });

  test('stems results concatenate per channel', () => {
    const rs = plans().map((p) => chunkResult(p, { channels: 6 }));
    const w = assembleWindow(rs, []);
    expect(w.channels).toHaveLength(6);
    expect(w.stemPairs).toBe(3);
    const start = Math.round((1 * SR) / CPS);
    for (let c = 0; c < 6; c++) {
      expect(w.channels[c]![0]).toBe(sampleAt(start, c));
      expect(w.channels[c]![w.frames - 1]).toBe(sampleAt(start + w.frames - 1, c));
    }
  });

  test('ok is false when one chunk is, and errors concatenate', () => {
    const rs = plans().map((p, i) => chunkResult(p, { ok: i !== 1 }));
    rs[1]!.errors = [{ kind: 'hap', message: 'bad', bar: 6 }];
    rs[2]!.errors = [{ kind: 'sample', message: 'missing' }];
    const w = assembleWindow(rs, []);
    expect(w.ok).toBe(false);
    expect(w.errors.map((e) => e.message)).toEqual(['bad', 'missing']);
  });

  test('a gap between chunks throws', () => {
    const rs = plans().map((p) => chunkResult(p));
    expect(() => assembleWindow([rs[0]!, rs[2]!], [])).toThrow(RangeError);
  });

  test('a channel count mismatch throws', () => {
    const [p0, p1] = plans();
    expect(() => assembleWindow([chunkResult(p0!), chunkResult(p1!, { channels: 4 })], [])).toThrow(
      RangeError,
    );
  });

  test('no results throws', () => {
    expect(() => assembleWindow([], [])).toThrow(RangeError);
  });
});

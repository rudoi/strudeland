import { describe, expect, test } from 'bun:test';
import { expectedFrames, windowFrames, type RenderRequest } from '../src/lib/render/contract';
import { cpsFor } from '../src/lib/strudel/structure';

const req = (over: Partial<RenderRequest> = {}): RenderRequest => ({
  code: 'sound("bd")',
  layout: 'linear',
  fromBar: 0,
  toBar: 4,
  overlapBars: 1,
  lookbackSec: 'auto',
  mode: 'mix',
  sampleRate: 44100,
  ...over,
});

describe('windowFrames', () => {
  test('telescopes over adjacent windows', () => {
    const cps = 0.6524;
    expect(windowFrames(0, 3, cps, 44100) + windowFrames(3, 8, cps, 44100)).toBe(
      windowFrames(0, 8, cps, 44100),
    );
  });

  test('rounds the edges, not the difference', () => {
    const cps = 0.6524;
    expect(windowFrames(3, 8, cps, 44100)).toBe(
      Math.round((8 * 44100) / cps) - Math.round((3 * 44100) / cps),
    );
  });

  test('empty window is 0', () => {
    expect(windowFrames(5, 5, 0.6524, 44100)).toBe(0);
  });
});

describe('expectedFrames', () => {
  test('4/4 at 120 bpm with 1 s lookback and 1 overlap bar', () => {
    expect(expectedFrames(req(), 0.5, 1)).toBe(485100);
  });

  test('odd cps gives an integer equal to the rounding rule', () => {
    const cps = cpsFor(137, { num: 7, den: 8 });
    const r = req({ fromBar: 2, toBar: 7, overlapBars: 1 });
    const got = expectedFrames(r, cps, 0.73);
    expect(Number.isInteger(got)).toBe(true);
    expect(got).toBe(
      Math.round(0.73 * 44100) + Math.round((8 * 44100) / cps) - Math.round((2 * 44100) / cps),
    );
  });

  test('lookback 0 is the window plus overlap', () => {
    expect(expectedFrames(req({ overlapBars: 0 }), 0.5, 0)).toBe(4 * 2 * 44100);
  });
});

describe('RenderRequest JSON', () => {
  test('both mutation kinds survive a JSON round trip', () => {
    const a = req({ mutation: { kind: 'dropOrbit', orbit: 2, bars: [1, 3] } });
    const b = req({ mutation: { kind: 'dropNotesBelowHz', hz: 80 }, lookbackSec: 0 });
    const c = req({
      mutation: { kind: 'dropNotesBelowHz', hz: 80, bars: [0] },
      samples: { base: 'http://x/' },
    });
    for (const r of [a, b, c]) expect(JSON.parse(JSON.stringify(r))).toEqual(r);
  });
});

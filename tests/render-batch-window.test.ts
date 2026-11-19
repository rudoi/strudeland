import { describe, expect, test } from 'bun:test';
import {
  batchAssembled,
  batchCrossChecks,
  batchSeamSecs,
  parseBatchInfo,
  type BatchInfo,
} from '../src/lib/render/batch-window';
import { windowFrames, type RenderRequest } from '../src/lib/render/contract';
import { ev } from './helpers/render-events';

const EVENTS = [ev({ tSec: 0, bar: 0 }), ev({ tSec: 2, bar: 1, freq: 110 })];

const INFO = {
  window: { fromBar: 0, toBar: 20, layout: 'linear' },
  cps: 0.5,
  structure: { bpm: 120 },
  lookbackSec: 0.25,
  errors: [],
  events: EVENTS,
  chunks: [
    { c0: 0, c1: 8, fromBar: 0, toBar: 8, lookbackSec: 0 },
    { c0: 8, c1: 16, fromBar: 8, toBar: 16, lookbackSec: 0.25 },
    { c0: 16, c1: 20, fromBar: 16, toBar: 20, lookbackSec: 0.25 },
  ],
  summary: { frames: 1764000 },
};

const BASE: RenderRequest = {
  code: '// a.js',
  layout: 'linear',
  fromBar: 0,
  toBar: 20,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: 44100,
};

const info = (o: Record<string, unknown> = {}): BatchInfo =>
  parseBatchInfo(JSON.stringify({ ...INFO, ...o }));

describe('parseBatchInfo', () => {
  test('a valid info file keeps only what the harness reads', () => {
    expect(info()).toEqual({
      window: { fromBar: 0, toBar: 20, layout: 'linear' },
      cps: 0.5,
      lookbackSec: 0.25,
      errors: [],
      events: EVENTS,
      chunks: [
        { fromBar: 0, toBar: 8 },
        { fromBar: 8, toBar: 16 },
        { fromBar: 16, toBar: 20 },
      ],
    });
    const withErrors = info({ errors: [{ kind: 'hap', message: 'm', bar: 3 }] });
    expect(withErrors.errors).toEqual([{ kind: 'hap', message: 'm', bar: 3 }]);
  });

  test('not JSON, not an object', () => {
    expect(() => parseBatchInfo('{')).toThrow(/^batch info: not JSON/);
    expect(() => parseBatchInfo('[]')).toThrow('batch info: the document must be an object');
    expect(() => parseBatchInfo('null')).toThrow('batch info: the document must be an object');
  });

  test('missing keys and wrong types name the key', () => {
    const cases: [Record<string, unknown>, string][] = [
      [{ window: undefined }, 'window must be an object'],
      [{ window: { ...INFO.window, fromBar: -1 } }, 'window.fromBar must be a bar'],
      [{ window: { ...INFO.window, toBar: 1.5 } }, 'window.toBar must be a bar'],
      [{ window: { ...INFO.window, layout: 'song' } }, 'window.layout must be clip or linear'],
      [{ cps: undefined }, 'cps must be a positive number'],
      [{ cps: 0 }, 'cps must be a positive number'],
      [{ cps: '0.5' }, 'cps must be a positive number'],
      [{ lookbackSec: null }, 'lookbackSec must be a number'],
      [{ errors: {} }, 'errors must be an array'],
      [{ errors: [{ kind: 'hap' }] }, 'errors[0] must be a render error'],
      [{ errors: [{ kind: 'hap', message: 'm', bar: 'x' }] }, 'errors[0] must be a render error'],
      [{ events: undefined }, 'events must be an array'],
      [{ events: [ev(), 3] }, 'events[1] must be an object'],
      [{ chunks: null }, 'chunks must be an array'],
      [{ chunks: [{ fromBar: 0 }] }, 'chunks[0] must be a chunk window with fromBar and toBar'],
    ];
    for (const [patch, want] of cases) {
      expect(() => info(patch)).toThrow(`batch info: ${want}`);
    }
  });
});

describe('batchCrossChecks', () => {
  const fx = { base: BASE, cps: 0.5, events: EVENTS };

  test('a batch that matches the fixture has no reasons, even with keys reordered', () => {
    const reordered = EVENTS.map((e) => Object.fromEntries(Object.entries(e).reverse()));
    expect(batchCrossChecks(info({ events: reordered }), fx)).toEqual([]);
  });

  test('an undefined property is an absent one and -0 is 0, as in the info file', () => {
    const events = EVENTS.map((e, i) => ({ ...e, extra: undefined, tSec: i === 0 ? -0 : e.tSec }));
    expect(batchCrossChecks(info(), { ...fx, events })).toEqual([]);
  });

  test('every difference is a reason', () => {
    expect(batchCrossChecks(info({ window: { ...INFO.window, layout: 'clip' } }), fx)).toEqual([
      "batch: the window is [0, 20) clip, the fixture's [0, 20) linear",
    ]);
    expect(batchCrossChecks(info({ cps: 0.5000001 }), fx)).toEqual([
      "batch: cps 0.5000001 differs from the planner's 0.5",
    ]);
    expect(batchCrossChecks(info({ events: [EVENTS[0], ev({ tSec: 2, bar: 1 })] }), fx)).toEqual([
      "batch: the batch schedule differs from the planner's (2 events against 2, first difference at events[1].freq)",
    ]);
    expect(batchCrossChecks(info({ events: [] }), fx)).toEqual([
      "batch: the batch schedule differs from the planner's (0 events against 2, first difference at events.length)",
    ]);
    expect(
      batchCrossChecks(
        info({
          errors: [
            { kind: 'other', message: 'console.error: x' },
            { kind: 'hap', message: 'bad', bar: 4 },
          ],
        }),
        fx,
      ),
    ).toEqual(['batch: other: console.error: x', 'batch: hap: bad (bar 4)']);
  });

  test('chunks that do not tile the window', () => {
    const tile = (chunks: { fromBar: number; toBar: number }[]) =>
      batchCrossChecks(info({ chunks }), fx);
    expect(tile([])).toEqual(['batch: the chunks (none) do not tile the window [0, 20)']);
    expect(
      tile([
        { fromBar: 0, toBar: 8 },
        { fromBar: 9, toBar: 20 },
      ]),
    ).toEqual(['batch: the chunks [0, 8) [9, 20) do not tile the window [0, 20)']);
    expect(tile([{ fromBar: 0, toBar: 19 }])).toEqual([
      'batch: the chunks [0, 19) do not tile the window [0, 20)',
    ]);
    expect(tile([{ fromBar: 1, toBar: 20 }])).toEqual([
      'batch: the chunks [1, 20) do not tile the window [0, 20)',
    ]);
    expect(
      tile([
        { fromBar: 0, toBar: 0 },
        { fromBar: 0, toBar: 20 },
      ]),
    ).toEqual(['batch: the chunks [0, 0) [0, 20) do not tile the window [0, 20)']);
  });
});

describe('batchAssembled', () => {
  const frames = windowFrames(0, 20, 0.5, 44100);
  const wav = (n = frames, channels = 2, sampleRate = 44100) => ({
    channels: Array.from({ length: channels }, (_, c) => new Float32Array(n).fill(c + 1)),
    sampleRate,
  });

  test('the window as assembleWindow would give it', () => {
    const w = wav();
    const a = batchAssembled(w, info(), BASE, EVENTS, 1234);
    expect(a.channels).toBe(w.channels);
    expect(a).toMatchObject({
      sampleRate: 44100,
      frames,
      windowStartFrame: 0,
      lookbackSecUsed: 0,
      bufferStartSec: 0,
      cps: 0.5,
      events: EVENTS,
      stemPairs: 1,
      errors: [],
      ok: true,
      timing: { evalMs: 0, renderMs: 1234, totalMs: 1234 },
      request: { ...BASE, overlapBars: 0, lookbackSec: 0 },
      seamSecs: [16, 32],
    });
  });

  test('a window not starting at bar 0 and bars that are not whole frames', () => {
    const cps = 0.4375;
    const base = { ...BASE, fromBar: 3, toBar: 10 };
    const i = info({
      cps,
      window: { fromBar: 3, toBar: 10, layout: 'linear' },
      chunks: [
        { fromBar: 3, toBar: 7 },
        { fromBar: 7, toBar: 10 },
      ],
    });
    const n = windowFrames(3, 10, cps, 44100);
    const a = batchAssembled(wav(n), i, base, EVENTS, 0);
    expect(a.bufferStartSec).toBe(3 / cps);
    expect(a.frames).toBe(n);
    expect(batchSeamSecs(i)).toEqual([windowFrames(3, 7, cps, 44100) / 44100]);
    expect(a.seamSecs).toEqual(batchSeamSecs(i));
    expect(a.request).toMatchObject({ fromBar: 3, toBar: 10 });
  });

  test('a WAV of the wrong shape is a RangeError', () => {
    expect(() => batchAssembled(wav(frames - 1), info(), BASE, EVENTS, 0)).toThrow(
      `batch: the WAV has ${frames - 1}/${frames - 1} frames, the window ${frames}`,
    );
    expect(() => batchAssembled(wav(frames, 4), info(), BASE, EVENTS, 0)).toThrow(
      'batch: the WAV has 4 channels, not 2',
    );
    expect(() => batchAssembled(wav(frames, 2, 48000), info(), BASE, EVENTS, 0)).toThrow(
      'batch: the WAV is 48000 Hz, not 44100',
    );
    const uneven = wav();
    uneven.channels[1] = new Float32Array(frames + 1);
    expect(() => batchAssembled(uneven, info(), BASE, EVENTS, 0)).toThrow(RangeError);
  });
});

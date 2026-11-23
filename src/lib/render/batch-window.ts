/**
 * The main window of a `render:check --planner batch` fixture: the whole Rust pipeline's render
 * (`strudeland-render render`, loudness off, a float WAV and an `--info` file) as the
 * `AssembledWindow` the gates and the parity features read. `parseBatchInfo` reads the info file,
 * `batchCrossChecks` compares it with the fixture's own (host-planned) schedule, and
 * `batchAssembled` builds the window. Pure: no fs, Strudel, DOM or Playwright imports.
 */

import type { AssembledWindow } from './chunks';
import { SAMPLE_RATE, type RenderError, type RenderRequest, type SegmentLayout } from './contract';
import { isRecord } from './engine-job';
import { firstDifference } from './host-parity-lib';
import type { RenderEvent } from './schedule';
import { renderWindowFrames, type RenderTiming } from './timing';

/** The parts of the dev binary's `--info` file the harness reads. */
export interface BatchInfo {
  window: { fromBar: number; toBar: number; layout: SegmentLayout };
  cps: number;
  lookbackSec: number;
  /** The planner's captured and selection errors. */
  errors: RenderError[];
  /** The schedule over `[0, toBar)`. */
  events: RenderEvent[];
  /** The chunk windows, in order. */
  chunks: { fromBar: number; toBar: number }[];
}

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isBar = (v: unknown): v is number => Number.isInteger(v) && (v as number) >= 0;

/** The info file's text as a `BatchInfo`; throws an `Error` naming the first key that is
 *  missing or of the wrong type. */
export function parseBatchInfo(text: string): BatchInfo {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    throw new Error(`batch info: not JSON (${(e as Error).message})`);
  }
  const bad = (key: string, want: string): never => {
    throw new Error(`batch info: ${key} must be ${want}`);
  };
  if (!isRecord(raw)) return bad('the document', 'an object');
  const w = raw.window;
  if (!isRecord(w)) return bad('window', 'an object');
  if (!isBar(w.fromBar)) return bad('window.fromBar', 'a bar');
  if (!isBar(w.toBar)) return bad('window.toBar', 'a bar');
  if (w.layout !== 'clip' && w.layout !== 'linear') return bad('window.layout', 'clip or linear');
  if (!isNum(raw.cps) || raw.cps <= 0) return bad('cps', 'a positive number');
  if (!isNum(raw.lookbackSec)) return bad('lookbackSec', 'a number');
  if (!Array.isArray(raw.errors)) return bad('errors', 'an array');
  raw.errors.forEach((e: unknown, i: number) => {
    const ok =
      isRecord(e) &&
      typeof e.kind === 'string' &&
      typeof e.message === 'string' &&
      (e.bar === undefined || isNum(e.bar));
    if (!ok) bad(`errors[${i}]`, 'a render error');
  });
  if (!Array.isArray(raw.events)) return bad('events', 'an array');
  raw.events.forEach((e: unknown, i: number) => {
    if (!isRecord(e)) bad(`events[${i}]`, 'an object');
  });
  if (!Array.isArray(raw.chunks)) return bad('chunks', 'an array');
  raw.chunks.forEach((c: unknown, i: number) => {
    if (!isRecord(c) || !isBar(c.fromBar) || !isBar(c.toBar)) {
      bad(`chunks[${i}]`, 'a chunk window with fromBar and toBar');
    }
  });
  return {
    window: { fromBar: w.fromBar, toBar: w.toBar, layout: w.layout },
    cps: raw.cps,
    lookbackSec: raw.lookbackSec,
    errors: raw.errors as RenderError[],
    events: raw.events as RenderEvent[],
    chunks: (raw.chunks as { fromBar: number; toBar: number }[]).map((c) => ({
      fromBar: c.fromBar,
      toBar: c.toBar,
    })),
  };
}

/** `v` as the info file carries it: through JSON, so an `undefined` property is an absent one and
 *  `-0` is `0` (the info file passes through serde, which also reorders keys; `firstDifference`
 *  ignores key order). */
const asJson = (v: unknown): unknown => JSON.parse(JSON.stringify(v));

/** What the fixture planned: its base request, cps and schedule. */
export interface BatchFixture {
  base: RenderRequest;
  cps: number;
  events: RenderEvent[];
}

const kindLine = (e: RenderError): string =>
  `${e.kind}: ${e.message}${e.bar === undefined ? '' : ` (bar ${e.bar})`}`;

/** The reasons the batch render does not stand for the fixture (empty: it does): a different
 *  window, cps or schedule, planner errors, or chunks that do not tile the window. */
export function batchCrossChecks(info: BatchInfo, fx: BatchFixture): string[] {
  const out: string[] = [];
  const { fromBar, toBar, layout } = fx.base;
  const w = info.window;
  if (w.fromBar !== fromBar || w.toBar !== toBar || w.layout !== layout) {
    out.push(
      `batch: the window is [${w.fromBar}, ${w.toBar}) ${w.layout}, the fixture's [${fromBar}, ${toBar}) ${layout}`,
    );
  }
  if (info.cps !== fx.cps) out.push(`batch: cps ${info.cps} differs from the planner's ${fx.cps}`);
  const diff = firstDifference({ events: asJson(info.events) }, { events: asJson(fx.events) });
  if (diff !== null) {
    out.push(
      `batch: the batch schedule differs from the planner's (${info.events.length} events against ${fx.events.length}, first difference at ${diff})`,
    );
  }
  for (const e of info.errors) out.push(`batch: ${kindLine(e)}`);
  const tiles =
    info.chunks.length > 0 &&
    info.chunks[0]!.fromBar === w.fromBar &&
    info.chunks.at(-1)!.toBar === w.toBar &&
    info.chunks.every(
      (c, i) => c.fromBar < c.toBar && (i === 0 || info.chunks[i - 1]!.toBar === c.fromBar),
    );
  if (!tiles) {
    const list = info.chunks.map((c) => `[${c.fromBar}, ${c.toBar})`).join(' ');
    out.push(
      `batch: the chunks ${list || '(none)'} do not tile the window [${w.fromBar}, ${w.toBar})`,
    );
  }
  return out;
}

/** The seam positions `assembleWindow` would report: the start of every chunk after the first,
 *  in seconds from the window start. */
export function batchSeamSecs(info: BatchInfo, timing?: RenderTiming): number[] {
  const from = info.window.fromBar;
  return info.chunks
    .slice(1)
    .map((c) => renderWindowFrames(from, c.fromBar, info.cps, SAMPLE_RATE, timing) / SAMPLE_RATE);
}

/** The batch render as the fixture's main window. Throws a `RangeError` unless the WAV is 2
 *  channels of `windowFrames(fromBar, toBar)` frames at 44100 Hz. `renderMs` is the dev binary's
 *  process time. */
export function batchAssembled(
  wav: { channels: Float32Array[]; sampleRate: number },
  info: BatchInfo,
  base: RenderRequest,
  events: RenderEvent[],
  renderMs: number,
  timing?: RenderTiming,
): AssembledWindow {
  const { fromBar, toBar } = base;
  const frames = renderWindowFrames(fromBar, toBar, info.cps, SAMPLE_RATE, timing);
  if (wav.sampleRate !== SAMPLE_RATE) {
    throw new RangeError(`batch: the WAV is ${wav.sampleRate} Hz, not ${SAMPLE_RATE}`);
  }
  if (wav.channels.length !== 2) {
    throw new RangeError(`batch: the WAV has ${wav.channels.length} channels, not 2`);
  }
  const lens = wav.channels.map((c) => c.length);
  if (lens.some((n) => n !== frames)) {
    throw new RangeError(`batch: the WAV has ${lens.join('/')} frames, the window ${frames}`);
  }
  return {
    channels: wav.channels,
    sampleRate: SAMPLE_RATE,
    frames,
    windowStartFrame: 0,
    lookbackSecUsed: 0,
    bufferStartSec: timing
      ? timing.frameAt(fromBar, SAMPLE_RATE) / SAMPLE_RATE
      : fromBar / info.cps,
    ...(timing ? { contentFrames: frames } : {}),
    cps: info.cps,
    events,
    stemPairs: 1,
    errors: [],
    ok: true,
    timing: { evalMs: 0, renderMs, totalMs: renderMs },
    request: { ...base, overlapBars: 0, lookbackSec: 0 },
    seamSecs: batchSeamSecs(info, timing),
  };
}

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { assembleSlices } from '../scripts/render/transfer';
import { planEngineJob } from '../src/lib/render/engine-job';
import type { PlannedRender } from '../src/lib/render/job-plan';
import { pageJobInfo } from '../src/lib/render/page-job';
import { eventSources, selectHaps, windowPlan } from '../src/lib/render/plan';
import { eventsFromValues } from '../src/lib/render/schedule';
import { buildStructure } from '../src/lib/strudel/structure';
import {
  asciiJson,
  checkJobSampleRate,
  JOB_SLICE_CHARS,
  pageJob,
  processorMessage,
  sliceCount,
  sliceText,
  type MessagingNode,
} from '../src/render/pcm';
import type { PageJobInfo } from '../src/render/protocol';
import { hap, req } from './helpers/fake-offline';

/** What the page serves for a job and what the driver does with it, without a browser. */
function fakePage(haps: any[]) {
  const r = req({ toBar: 2, mode: 'stems' });
  const p = windowPlan(r, 0.5, 0);
  const { selected } = selectHaps(haps);
  const planned = planEngineJob(selected, selected, p, r, 0.5, 3);
  const structure = buildStructure({ bpm: 120, meter: '4/4', sections: [{ name: 'a', bars: 2 }] });
  const errors = [{ kind: 'other' as const, message: 'x' }];
  const { info, json } = pageJob(planned, { errors, plan: p, cps: 0.5, structure });
  return { info, json, planned, structure, plan: p, errors };
}

/** The driver: every slice in order, joined and parsed. */
const fetchJob = (json: string, info: PageJobInfo, sliceChars?: number) => {
  const n = sliceCount(info.jsonLength, sliceChars);
  const slices = Array.from({ length: n }, (_, i) => sliceText(json, i, sliceChars));
  return JSON.parse(assembleSlices(slices));
};

const haps = [
  hap(0, 1, { s: 'sine', note: 60, orbit: 2, label: 'héllo 🎹' }),
  hap(1, 2, { s: 'square', gain: NaN }),
];

describe('page job kind', () => {
  test('PageJobInfo shape', () => {
    const { info, json, planned, structure, plan, errors } = fakePage(haps);
    expect(Object.keys(info).sort()).toEqual(
      ['cps', 'errors', 'events', 'jsonLength', 'plan', 'stemPairs', 'structure'].sort(),
    );
    expect(info.jsonLength).toBe(json.length);
    expect(info.events).toBe(planned.events);
    expect(info.events).toHaveLength(2);
    expect(info.stemPairs).toBe(2);
    expect(info.plan).toBe(plan);
    expect(info.structure).toBe(structure);
    expect(info.errors).toBe(errors);
    expect(info.cps).toBe(0.5);
  });

  test('slices reassemble into the job, for any slice size', () => {
    const { info, json, planned } = fakePage(haps);
    for (const size of [1, 7, 64, json.length - 1, json.length, json.length + 1]) {
      expect(fetchJob(json, info, size)).toEqual(planned.job);
    }
    expect(fetchJob(json, info)).toEqual(planned.job);
    expect(sliceCount(json.length)).toBe(1);
  });

  test('the JSON is ASCII, so a slice never splits a surrogate pair', () => {
    const { json, planned } = fakePage(haps);
    expect(/^[\x00-\x7e]*$/.test(json)).toBe(true);
    expect(JSON.parse(json)).toEqual(planned.job);
    expect(json).toContain('\\ud83c\\udfb9');
    expect(asciiJson({ a: 'é\u007f' })).toBe('{"a":"\\u00e9\\u007f"}');
    expect(JSON.parse(asciiJson('\ud800'))).toBe('\ud800');
    expect(asciiJson('\uffff\u0080')).toBe('"\\uffff\\u0080"');
  });

  test('page-job.ts is ASCII: the escaped range is spelled with escapes, not literal characters', () => {
    const source = readFileSync(join(import.meta.dir, '../src/lib/render/page-job.ts'), 'utf8');
    expect(/^[\x00-\x7e]*$/.test(source)).toBe(true);
    expect(source).toContain('/[\\u007f-\\uffff]/g');
  });

  test('slice counts and ranges', () => {
    expect(JOB_SLICE_CHARS).toBe(4 * 1024 * 1024);
    expect(sliceCount(0, 4)).toBe(0);
    expect(sliceCount(4, 4)).toBe(1);
    expect(sliceCount(5, 4)).toBe(2);
    expect(sliceText('abcdefghij', 2, 4)).toBe('ij');
    expect(() => sliceText('abc', 1, 4)).toThrow('slice 1 out of range (1 slices)');
    expect(() => sliceText('abc', -1, 4)).toThrow(RangeError);
    expect(() => sliceText('abc', 0.5, 4)).toThrow(RangeError);
    expect(() => sliceText('', 0, 4)).toThrow('out of range (0 slices)');
    expect(() => sliceCount(3, 0)).toThrow('positive integer');
    expect(() => sliceCount(3, 1.5)).toThrow('positive integer');
  });

  test('assembleSlices joins in order', () => {
    expect(assembleSlices(['{"a"', ':1}'])).toBe('{"a":1}');
    expect(assembleSlices([])).toBe('');
  });

  test('the job of the full window (fromBar 0, lookbackSec 0) carries the schedule events', () => {
    // RustRenderer.schedule asks for this job; its events must be what the page's schedule kind
    // returns: eventsFromValues over selectHaps(query(0, toBar + overlapBars)).
    const r = req({ toBar: 3, overlapBars: 1, mode: 'mix' });
    const cps = 0.5;
    const all = [
      hap(0, 1, { s: 'sine', note: 40 }),
      hap(1.5, 2, { s: 'square', note: 52, orbit: 3 }),
      hap(3.25, 3.5, { s: 'sawtooth', note: 64 }),
      hap(0.5, 1, { s: 'sine' }, { noWhole: true }),
    ];
    const plan = windowPlan(r, cps, 0);
    expect([plan.c0, plan.c1]).toEqual([0, 4]);
    const schedule = selectHaps(all).selected;
    const planned = planEngineJob(schedule, schedule, plan, r, cps, 0);
    expect(planned.events).toEqual(eventsFromValues(eventSources(schedule), cps));
    expect(planned.events).toHaveLength(3);
  });
});

describe('pageJobInfo', () => {
  const { info, json, structure, plan, errors } = fakePage([
    hap(0, 0.5, { s: 'sine', note: 60, orbit: 1 }),
  ]);
  const chunkPlan = { ...plan, fromBar: 0, toBar: 2, lookbackSec: 0 };
  const planned = (chunks: PlannedRender['chunks']): PlannedRender => ({
    window: { fromBar: 0, toBar: 2, layout: 'linear' },
    musicFrames: plan.contentFrames,
    cps: 0.5,
    structure,
    sections: [],
    lookbackSec: 0,
    errors,
    chunks,
  });
  const one = { json, plan: chunkPlan, info: { events: info.events, stemPairs: info.stemPairs } };

  test("one chunk with its info is the page's job info", () => {
    expect(pageJobInfo(planned([one]))).toEqual({ ...info, plan: chunkPlan });
  });

  test('zero chunks, two chunks, or a chunk without info throw', () => {
    expect(() => pageJobInfo(planned([]))).toThrow('not a single-chunk plan (0 chunk(s))');
    expect(() => pageJobInfo(planned([one, one]))).toThrow('not a single-chunk plan (2 chunk(s))');
    const { info: _, ...bare } = one;
    expect(() => pageJobInfo(planned([bare]))).toThrow('not a single-chunk plan (1 chunk(s))');
  });
});

describe('processorMessage', () => {
  const node = (): MessagingNode => ({ port: { onmessage: null }, onprocessorerror: null });

  test('resolves with the first message the processor posts', async () => {
    const n = node();
    const p = processorMessage<{ outputs: number[] }>(n, 'proc');
    n.port.onmessage!(new MessageEvent('message', { data: { outputs: [1] } }));
    expect(await p).toEqual({ outputs: [1] });
  });

  test('rejects with the processor error instead of waiting for a message', async () => {
    const n = node();
    const p = processorMessage(n, 'proc');
    n.onprocessorerror!(new ErrorEvent('processorerror', { message: 'boom in constructor' }));
    await expect(p).rejects.toThrow('proc: processor error: boom in constructor');
    const m = node();
    const q = processorMessage(m, 'proc');
    m.onprocessorerror!(new ErrorEvent('processorerror'));
    await expect(q).rejects.toThrow(/^proc: processor error$/);
  });
});

describe('checkJobSampleRate', () => {
  test('a job at SAMPLE_RATE or without a rate passes; any other rate is a clear error', () => {
    expect(() => checkJobSampleRate({ sampleRate: 44100 })).not.toThrow();
    expect(() => checkJobSampleRate({})).not.toThrow();
    expect(() => checkJobSampleRate({ sampleRate: 48000 })).toThrow(
      'job: sampleRate 48000 is not supported; jobs are planned at 44100 Hz',
    );
    expect(() => checkJobSampleRate({ sampleRate: '44100' })).toThrow(/sampleRate 44100 is not/);
  });
});

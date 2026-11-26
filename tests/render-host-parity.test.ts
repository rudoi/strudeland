import { describe, expect, test } from 'bun:test';
import {
  batchRequest,
  batchRequestJson,
  compareBatch,
  failureText,
  firstDifference,
  formatBatchDifference,
  infoWithoutErrors,
  jsonPath,
  parseParityArgs,
  PARITY_USAGE,
  scheduleFailed,
  textDifference,
  withoutErrors,
} from '../src/lib/render/host-parity-lib';
import type { PlannedRender } from '../src/lib/render/job-plan';
import type { PageJobInfo } from '../src/lib/render/page-job';

describe('jsonPath', () => {
  test('the a.b[2].c format', () => {
    expect(jsonPath(['a', 'b', 2, 'c'])).toBe('a.b[2].c');
    expect(jsonPath([0, 'x'])).toBe('[0].x');
    expect(jsonPath(['haps', 10, 'controls', '$num'])).toBe('haps[10].controls.$num');
  });

  test('the root and keys that are not identifiers', () => {
    expect(jsonPath([])).toBe('$');
    expect(jsonPath(['a b', 1])).toBe('["a b"][1]');
    expect(jsonPath(['x', '0'])).toBe('x["0"]');
    expect(jsonPath(['x', 'a"b'])).toBe('x["a\\"b"]');
  });
});

describe('firstDifference', () => {
  test('equal values are null', () => {
    expect(firstDifference(null, null)).toBeNull();
    expect(firstDifference(1.5, 1.5)).toBeNull();
    expect(firstDifference('s', 's')).toBeNull();
    expect(firstDifference([], [])).toBeNull();
    expect(firstDifference({}, {})).toBeNull();
    const v = { a: { b: [1, 2, { c: 'x' }] }, d: [true, null] };
    expect(firstDifference(v, structuredClone(v))).toBeNull();
  });

  test('key order alone is equal', () => {
    expect(firstDifference({ a: 1, b: 2 }, { b: 2, a: 1 })).toBeNull();
  });

  test('a differing leaf', () => {
    expect(firstDifference({ a: { b: [1, 2, { c: 3 }] } }, { a: { b: [1, 2, { c: 4 }] } })).toBe(
      'a.b[2].c',
    );
    expect(firstDifference(1, 2)).toBe('$');
    expect(firstDifference({ x: 0.1 + 0.2 }, { x: 0.3 })).toBe('x');
  });

  test('numbers compare exactly: -0 differs from 0', () => {
    expect(firstDifference({ x: -0 }, { x: 0 })).toBe('x');
    expect(firstDifference(NaN, NaN)).toBeNull();
  });

  test('the first difference in key order wins', () => {
    expect(firstDifference({ a: 1, b: 2 }, { a: 9, b: 9 })).toBe('a');
    expect(firstDifference([{ a: 1 }, { a: 2 }], [{ a: 1 }, { a: 3 }])).toBe('[1].a');
  });

  test('a differing array length: a prefix is reported as length', () => {
    expect(firstDifference({ a: [1, 2] }, { a: [1, 2, 3] })).toBe('a.length');
    expect(firstDifference({ a: [1, 2, 3] }, { a: [1, 2] })).toBe('a.length');
    expect(firstDifference([1], [1, 2])).toBe('length');
    // a differing element before the end wins over the length
    expect(firstDifference({ a: [1, 5] }, { a: [1, 2, 3] })).toBe('a[1]');
  });

  test('differing key sets', () => {
    expect(firstDifference({ a: 1, b: 2 }, { a: 1 })).toBe('b');
    expect(firstDifference({ a: 1 }, { a: 1, z: { y: 1 } })).toBe('z');
    expect(firstDifference({ p: { a: 1 } }, { p: { b: 1 } })).toBe('p.a');
  });

  test('differing kinds', () => {
    expect(firstDifference({ a: [] }, { a: {} })).toBe('a');
    expect(firstDifference({ a: null }, { a: {} })).toBe('a');
    expect(firstDifference({ a: '1' }, { a: 1 })).toBe('a');
    expect(firstDifference([1], { 0: 1 })).toBe('$');
  });
});

describe('parseParityArgs', () => {
  test('mixed-meter plans use the dedicated fixture and can select a report directory', () => {
    expect(parseParityArgs(['--mixed-meter', '--out', 'o'])).toMatchObject({
      mixedMeter: true,
      outDir: 'o',
    });
    const error = '--mixed-meter cannot be combined with --batch or fixture filters';
    expect(parseParityArgs(['--mixed-meter', '--batch'])).toEqual({ error });
    expect(parseParityArgs(['sine-basic', '--mixed-meter'])).toEqual({ error });
  });
  test('defaults', () => {
    expect(parseParityArgs([])).toEqual({
      filters: [],
      outDir: '.render-out',
      batch: false,
      help: false,
    });
  });

  test('filters, --out in both forms, help', () => {
    expect(parseParityArgs(['sine-*', '--out', 'o', 'tag:host'])).toEqual({
      filters: ['sine-*', 'tag:host'],
      outDir: 'o',
      batch: false,
      help: false,
    });
    expect(parseParityArgs(['--out=p'])).toEqual({
      filters: [],
      outDir: 'p',
      batch: false,
      help: false,
    });
    expect(parseParityArgs(['-h'])).toMatchObject({ help: true });
    expect(parseParityArgs(['--help'])).toMatchObject({ help: true });
  });

  test('errors', () => {
    expect(parseParityArgs(['--out'])).toEqual({ error: '--out needs a value' });
    expect(parseParityArgs(['--out='])).toEqual({ error: '--out needs a value' });
    expect(parseParityArgs(['--out', '--help'])).toEqual({ error: '--out needs a value' });
    expect(parseParityArgs(['--engine', 'rust'])).toEqual({ error: `unknown option '--engine'` });
    expect(parseParityArgs(['--batch=1'])).toEqual({ error: `unknown option '--batch=1'` });
  });

  test('usage names the flags and the exit codes', () => {
    for (const w of [
      '--out',
      '--help',
      '--batch',
      'plan-render',
      'batch-<i>.page.json',
      'STRUDELAND_HOST_BIN',
      'exit codes',
      'host-parity',
    ]) {
      expect(PARITY_USAGE).toContain(w);
    }
  });
});

describe('--batch', () => {
  test('composes with fixture selectors and --out', () => {
    expect(parseParityArgs(['--batch', 'pad-*', '--out', 'o', 'tag:host'])).toEqual({
      filters: ['pad-*', 'tag:host'],
      outDir: 'o',
      batch: true,
      help: false,
    });
    expect(parseParityArgs(['sine-basic', '--batch'])).toMatchObject({ batch: true });
  });

  test('the request is the window kind, in the host spelling', () => {
    expect(batchRequestJson({ fromBar: 40, toBar: 60, layout: 'linear' })).toBe(
      '{"length":{"kind":"window","fromBar":40,"toBar":60,"layout":"linear"}}',
    );
    expect(JSON.parse(batchRequestJson({ fromBar: 0, toBar: 12, layout: 'clip' }))).toEqual({
      length: { kind: 'window', fromBar: 0, toBar: 12, layout: 'clip' },
    });
  });

  test('batchRequestJson is the text of batchRequest', () => {
    const window = { fromBar: 3, toBar: 9, layout: 'linear' } as const;
    expect(batchRequest(window)).toEqual({ length: { kind: 'window', ...window } });
    expect(batchRequestJson(window)).toBe(JSON.stringify(batchRequest(window)));
  });

  const plans = [
    { fromBar: 0, toBar: 8 },
    { fromBar: 8, toBar: 20 },
  ];
  const A = '{"schema":2,"haps":[{"note":"c"}]}';
  const B = '{"schema":2,"haps":[{"note":"e"}]}';
  const chunk = (json: string, fromBar: number, toBar: number) => ({
    json,
    plan: { fromBar, toBar },
  });

  test('equal chunks have no difference', () => {
    expect(compareBatch(plans, [A, B], [chunk(A, 0, 8), chunk(B, 8, 20)])).toEqual([]);
    expect(compareBatch([], [], [])).toEqual([]);
  });

  test('a different chunk count stops the comparison', () => {
    expect(compareBatch(plans, [A, B], [chunk(A, 0, 20)])).toEqual([
      { kind: 'count', page: 2, host: 1 },
    ]);
    expect(compareBatch(plans, [A, B], [])).toEqual([{ kind: 'count', page: 2, host: 0 }]);
  });

  test('a different window and a different text, per chunk, in order', () => {
    expect(compareBatch(plans, [A, B], [chunk(A, 0, 9), chunk(A, 9, 20)])).toEqual([
      { kind: 'window', index: 0, page: { fromBar: 0, toBar: 8 }, host: { fromBar: 0, toBar: 9 } },
      {
        kind: 'window',
        index: 1,
        page: { fromBar: 8, toBar: 20 },
        host: { fromBar: 9, toBar: 20 },
      },
      { kind: 'text', index: 1, where: 'haps[0].note' },
    ]);
  });

  test('texts compare byte for byte: spacing and key order differ', () => {
    const spaced = '{"schema":2, "haps":[{"note":"c"}]}';
    const reordered = '{"haps":[{"note":"c"}],"schema":2}';
    expect(compareBatch([plans[0]!], [A], [chunk(spaced, 0, 8)])).toEqual([
      { kind: 'text', index: 0, where: 'key order or spacing' },
    ]);
    expect(compareBatch([plans[0]!], [A], [chunk(reordered, 0, 8)])).toEqual([
      { kind: 'text', index: 0, where: 'key order or spacing' },
    ]);
    // -0 and 0 are different bytes and different numbers.
    expect(textDifference('{"a":0}', '{"a":-0}')).toBe('a');
  });

  test('a chunk the page failed to plan is not compared (the caller reports it)', () => {
    expect(compareBatch(plans, [null, B], [chunk(A, 0, 8), chunk(B, 8, 20)])).toEqual([]);
  });

  test('unparseable text is reported as such', () => {
    const [d] = compareBatch([plans[0]!], [A], [chunk('{"schema":', 0, 8)]);
    expect(d).toMatchObject({ kind: 'text', index: 0 });
    expect((d as { where: string }).where).toStartWith('unparseable JSON (');
  });

  test('report lines', () => {
    expect(formatBatchDifference({ kind: 'count', page: 3, host: 2 })).toBe(
      'the page planned 3 chunk(s), the host 2',
    );
    expect(
      formatBatchDifference({
        kind: 'window',
        index: 1,
        page: { fromBar: 8, toBar: 20 },
        host: { fromBar: 9, toBar: 20 },
      }),
    ).toBe("chunk 1: the page's window is [8, 20), the host's [9, 20)");
    expect(formatBatchDifference({ kind: 'text', index: 2, where: 'haps[3].s' })).toBe(
      'chunk 2: first difference at haps[3].s',
    );
  });
});

describe('scheduleFailed', () => {
  test('a failed plan, or a plan whose info reports errors, failed to schedule', () => {
    expect(scheduleFailed({ ok: false, message: 'strudeland-host exited 1: eval: x' })).toBe(true);
    // The host's plan exits 0 with the schedule's errors in info.errors.
    expect(scheduleFailed({ ok: true, errors: ['hap: hap at cycle 0: boom'] })).toBe(true);
    expect(scheduleFailed({ ok: true, errors: [] })).toBe(false);
  });

  test('failureText: the message or the errors', () => {
    expect(failureText({ ok: false, message: 'm' })).toBe('m');
    expect(failureText({ ok: true, errors: ['a', 'b'] })).toBe('a; b');
  });
});

describe('withoutErrors and infoWithoutErrors', () => {
  const errors = [{ kind: 'other' as const, message: 'page only' }];

  test('a plan without its errors keeps every other key, and the input is unchanged', () => {
    const plan = {
      window: { fromBar: 0, toBar: 4, layout: 'linear' },
      musicFrames: 10,
      cps: 0.5,
      structure: {},
      lookbackSec: 0,
      errors,
      chunks: [],
    } as unknown as PlannedRender;
    const out = withoutErrors(plan);
    expect('errors' in out).toBe(false);
    expect(out as object).toEqual({
      window: plan.window,
      musicFrames: 10,
      cps: 0.5,
      structure: {},
      lookbackSec: 0,
      chunks: [],
    });
    expect(plan.errors).toBe(errors);
    // Plans that differ only in their errors compare equal.
    expect(firstDifference(withoutErrors(plan), withoutErrors({ ...plan, errors: [] }))).toBeNull();
    expect(firstDifference(withoutErrors(plan), withoutErrors({ ...plan, cps: 1 }))).toBe('cps');
  });

  test('an info without its errors keeps every other key', () => {
    const info = {
      jsonLength: 3,
      events: [],
      errors,
      plan: {},
      cps: 0.5,
      structure: {},
      stemPairs: 1,
    } as unknown as PageJobInfo;
    const out = infoWithoutErrors(info);
    expect('errors' in out).toBe(false);
    expect(out as object).toEqual({
      jsonLength: 3,
      events: [],
      plan: {},
      cps: 0.5,
      structure: {},
      stemPairs: 1,
    });
    expect(firstDifference(out, infoWithoutErrors({ ...info, errors: [] }))).toBeNull();
    expect(firstDifference(out, infoWithoutErrors({ ...info, stemPairs: 2 }))).toBe('stemPairs');
  });
});

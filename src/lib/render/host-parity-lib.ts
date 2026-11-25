/**
 * `host:parity`'s pure part: where two parsed JSON values first differ, the schedule-failure rule,
 * the command line, and `--batch`'s request and chunk comparison. Pure: no fs, Strudel, DOM,
 * Tauri, Playwright or cargo.
 */

import type { SegmentLayout } from './contract';

/** One step into a JSON value: an object key or an array index. */
export type JsonStep = string | number;

const IDENT = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

/** The path of `steps` as `a.b[2].c`: indices in brackets, identifier keys after a dot, any
 *  other key as a quoted bracket (`["a b"]`); the root is `$`. */
export function jsonPath(steps: readonly JsonStep[]): string {
  let out = '';
  for (const s of steps) {
    if (typeof s === 'number') out += `[${s}]`;
    else if (IDENT.test(s)) out += out === '' ? s : `.${s}`;
    else out += `[${JSON.stringify(s)}]`;
  }
  return out === '' ? '$' : out;
}

const kind = (x: unknown): string => (x === null ? 'null' : Array.isArray(x) ? 'array' : typeof x);

/** The first place, in key order (`a`'s keys first, then keys only `b` has), where `a` and `b`
 *  differ, as a `jsonPath`; `<path>.length` when one array is a prefix of the other; null when
 *  they are deep-equal. Numbers compare with `Object.is` (so `-0` and `0` differ, `NaN` equals
 *  `NaN`). */
export function firstDifference(a: unknown, b: unknown): string | null {
  const walk = (x: unknown, y: unknown, steps: JsonStep[]): string | null => {
    if (kind(x) !== kind(y)) return jsonPath(steps);
    if (Array.isArray(x)) {
      const ys = y as unknown[];
      const n = Math.min(x.length, ys.length);
      for (let i = 0; i < n; i++) {
        const d = walk(x[i], ys[i], [...steps, i]);
        if (d !== null) return d;
      }
      if (x.length !== ys.length) {
        return steps.length === 0 ? 'length' : `${jsonPath(steps)}.length`;
      }
      return null;
    }
    if (kind(x) === 'object') {
      const xo = x as Record<string, unknown>;
      const yo = y as Record<string, unknown>;
      for (const k of Object.keys(xo)) {
        if (!Object.hasOwn(yo, k)) return jsonPath([...steps, k]);
        const d = walk(xo[k], yo[k], [...steps, k]);
        if (d !== null) return d;
      }
      for (const k of Object.keys(yo)) {
        if (!Object.hasOwn(xo, k)) return jsonPath([...steps, k]);
      }
      return null;
    }
    return Object.is(x, y) ? null : jsonPath(steps);
  };
  return walk(a, b, []);
}

export interface ParityOptions {
  /** Fixture names, globs or `tag:<tag>` (`selectFixtures`); empty = all. */
  filters: string[];
  outDir: string;
  /** Compare the batch planner's chunk jobs (one plan-render per fixture) instead. */
  batch: boolean;
  /** Compare full plans for the dedicated mixed-meter fixture. */
  mixedMeter?: boolean;
  help: boolean;
}

export const PARITY_USAGE = `usage: bun run host:parity [fixture | glob | tag:<tag>]... [--batch] [--out DIR]
       bun run host:parity --mixed-meter [--out DIR]

Plans every engine job render:check --engine rust asks for (the full window, every chunk, and
the stems chunks of fixtures with the stems gate) on the render page in Chromium and with the
headless host (strudeland-host, built with cargo, or STRUDELAND_HOST_BIN), and compares the job
text byte for byte. On a difference both texts go to <out>/host-parity/<fixture>/<n>.page.json
and <n>.host.json. No fixture arguments = every fixture.

With --batch it compares the batch planner instead: the fixture's window planned in chunks with
no seam overlap, every chunk on the render page against one strudeland-host plan-render call
(the chunk count, each chunk's window and each job's text byte for byte; differences go to
batch-<i>.page.json and batch-<i>.host.json). The plain request set is not compared then.

options:
  --batch      compare the batch planner's chunk jobs
  --mixed-meter compare the mixed-meter clip and loop preset plans
  --out DIR    difference and song folder (default .render-out)
  -h, --help   this text

exit codes: 0 every job equal, 1 a job differs or only one side failed to plan it, 2 harness
error (usage, invalid corpus, Chromium or render page missing, host build failed)`;

/** `host:parity`'s command line. */
export function parseParityArgs(argv: string[]): ParityOptions | { error: string } {
  const o: ParityOptions = { filters: [], outDir: '.render-out', batch: false, help: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === '-h' || arg === '--help') {
      o.help = true;
    } else if (arg === '--batch') {
      o.batch = true;
    } else if (arg === '--mixed-meter') {
      o.mixedMeter = true;
    } else if (arg === '--out' || arg.startsWith('--out=')) {
      const v = arg === '--out' ? argv[++i] : arg.slice('--out='.length);
      if (v === undefined || v === '' || (arg === '--out' && v.startsWith('-'))) {
        return { error: '--out needs a value' };
      }
      o.outDir = v;
    } else if (arg.startsWith('-') && arg !== '-') {
      return { error: `unknown option '${arg}'` };
    } else {
      o.filters.push(arg);
    }
  }
  if (o.mixedMeter && (o.batch || o.filters.length > 0)) {
    return { error: '--mixed-meter cannot be combined with --batch or fixture filters' };
  }
  return o;
}

/** One side's planning of a request: planned, with the errors its info reports, or failed. */
export type PlanOutcome = { ok: true; errors: readonly string[] } | { ok: false; message: string };

/** Whether a side failed to schedule the song, by the rule `prepareFixture` applies to the page's
 *  schedule: a failed plan, or a plan whose info reports errors (the host's `plan` exits 0 with
 *  the schedule's errors in `info.errors`). */
export function scheduleFailed(o: PlanOutcome): boolean {
  return !o.ok || o.errors.length > 0;
}

/** A failed side's reason for the report: its failure message or its errors. */
export function failureText(o: PlanOutcome): string {
  return o.ok ? o.errors.join('; ') : o.message;
}

/** The `plan-render` request of a fixture's window: the explicit window kind, the mix. */
export function batchRequestJson(window: {
  fromBar: number;
  toBar: number;
  layout: SegmentLayout;
}): string {
  return JSON.stringify({
    length: {
      kind: 'window',
      fromBar: window.fromBar,
      toBar: window.toBar,
      layout: window.layout,
    },
  });
}

/** A chunk's content window. */
export interface ChunkBars {
  fromBar: number;
  toBar: number;
}

/** One chunk of the host's `plan-render` output, as `--batch` reads it. */
export interface HostBatchChunk {
  json: string;
  plan: ChunkBars;
}

/** Where a batch differs: the chunk count, a chunk's window, or a job's text (`where` is the
 *  first differing JSON path, or why there is none). */
export type BatchDifference =
  | { kind: 'count'; page: number; host: number }
  | { kind: 'window'; index: number; page: ChunkBars; host: ChunkBars }
  | { kind: 'text'; index: number; where: string };

/** Where two job texts first differ (they are known to differ). */
export function textDifference(page: string, host: string): string {
  try {
    return firstDifference(JSON.parse(page), JSON.parse(host)) ?? 'key order or spacing';
  } catch (e) {
    return `unparseable JSON (${e instanceof Error ? e.message : String(e)})`;
  }
}

/**
 * Compares the page's chunk plans and job texts (`page[i]`, `null` where the page failed to plan
 * chunk `i`, which the caller reports) with the host's batch: the chunk count first (a different
 * count stops there), then per chunk the window and the job text byte for byte.
 */
export function compareBatch(
  plans: readonly ChunkBars[],
  page: readonly (string | null)[],
  host: readonly HostBatchChunk[],
): BatchDifference[] {
  if (plans.length !== host.length) {
    return [{ kind: 'count', page: plans.length, host: host.length }];
  }
  const out: BatchDifference[] = [];
  host.forEach((h, index) => {
    const p = plans[index]!;
    if (p.fromBar !== h.plan.fromBar || p.toBar !== h.plan.toBar) {
      out.push({
        kind: 'window',
        index,
        page: { fromBar: p.fromBar, toBar: p.toBar },
        host: { fromBar: h.plan.fromBar, toBar: h.plan.toBar },
      });
    }
    const text = page[index];
    if (text === null || text === undefined || text === h.json) return;
    out.push({ kind: 'text', index, where: textDifference(text, h.json) });
  });
  return out;
}

/** A batch difference as a report line. */
export function formatBatchDifference(d: BatchDifference): string {
  switch (d.kind) {
    case 'count':
      return `the page planned ${d.page} chunk(s), the host ${d.host}`;
    case 'window':
      return (
        `chunk ${d.index}: the page's window is [${d.page.fromBar}, ${d.page.toBar}), ` +
        `the host's [${d.host.fromBar}, ${d.host.toBar})`
      );
    case 'text':
      return `chunk ${d.index}: first difference at ${d.where}`;
  }
}

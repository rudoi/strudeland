/**
 * `bun run host:parity [fixture|glob|tag:<tag>]… [--batch] [--out DIR]`: plans every engine job
 * `render:check --engine rust` asks for, on the render page in Chromium and with the headless
 * host (`strudeland-host`), and compares the job text byte for byte. It renders nothing, so it
 * does not depend on the engine. Imports Playwright (through `superdough-renderer.ts`) and runs
 * cargo: never import this from a test.
 *
 * `--batch` compares the batch planner instead: per fixture, the window's chunk plans with no seam
 * overlap (`renderChunkPlans`, overlap 0), each chunk's job planned on the page, against one
 * `strudeland-host plan-render` call over the same window (chunk count, windows, job text byte
 * for byte). A song the page cannot schedule must make `plan-render` fail too.
 *
 * Per fixture, the request set is the full window (what `RustRenderer.schedule` plans), every
 * chunk (`chunkRequest`) and, when the fixture has the stems gate, every stems chunk
 * (`stemsRequest`). Mutations are applied after planning, so they are not part of it. A song the
 * page cannot schedule is planned on the host only, which must fail too (a failed plan or errors
 * in its info, the rule the page side gets from `prepareFixture`). Exit 0 every request equal, 1 a
 * difference or a failure on one side only, 2 harness error.
 */

import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { selectFixtures } from '../../src/lib/render/check-options';
import { chunkRequest, renderChunkPlans } from '../../src/lib/render/chunks';
import { SAMPLE_RATE, type RenderRequest } from '../../src/lib/render/contract';
import type { FixtureEntry } from '../../src/lib/render/corpus';
import {
  batchRequestJson,
  compareBatch,
  failureText,
  firstDifference,
  formatBatchDifference,
  parseParityArgs,
  PARITY_USAGE,
  scheduleFailed,
  type PlanOutcome,
} from '../../src/lib/render/host-parity-lib';
import type { PlannedRender } from '../../src/lib/render/job-plan';
import { errorMessage } from '../../src/lib/errors';
import { ensureBuilt, Limiter, loadCorpus, prepareFixture, stemsRequest } from './check';
import { findHostBinary, HostPlanner } from './host-planner';
import type { JobSource } from './job-source';
import { bunRunner } from './rust-engine';
import { SuperdoughRenderer } from './superdough-renderer';
import { formatRenderError, HarnessError, isFatalHarnessError } from './transfer';

const ROOT = resolve(import.meta.dir, '../..');
/** Render pages at once. */
const PAGE_CONCURRENCY = 2;
/** Host processes at once. */
const HOST_CONCURRENCY = 4;

type Planned = { ok: true; text: string; errors: string[] } | { ok: false; message: string };

interface FixtureParity {
  status: 'ok' | 'DIFF' | 'FAIL';
  /** Requests compared. */
  requests: number;
  /** Detail lines (differences, one-sided failures). */
  lines: string[];
}

/** Rethrows a fatal harness error (exit 2); anything else is this side's planning failure. */
async function plan(source: JobSource, limit: Limiter, req: RenderRequest): Promise<Planned> {
  try {
    const { text, info } = await limit.run(() => source.job(req));
    return { ok: true, text, errors: info.errors.map(formatRenderError) };
  } catch (e) {
    if (e instanceof HarnessError && isFatalHarnessError(e)) throw e;
    return { ok: false, message: errorMessage(e) };
  }
}

/** The host's batch plan of a window, or its failure; a fatal harness error is rethrown. */
async function planBatch(
  host: HostPlanner,
  limit: Limiter,
  code: string,
  window: { fromBar: number; toBar: number; layout: RenderRequest['layout'] },
): Promise<{ ok: true; planned: PlannedRender } | { ok: false; message: string }> {
  try {
    const song = host.songFile(code);
    const planned = await limit.run(() => host.planRender(song, batchRequestJson(window)));
    return { ok: true, planned };
  } catch (e) {
    if (e instanceof HarnessError && isFatalHarnessError(e)) throw e;
    return { ok: false, message: errorMessage(e) };
  }
}

/** The request set of a scheduled fixture (module doc). */
function requestSet(
  fixture: FixtureEntry,
  base: RenderRequest,
  plans: Parameters<typeof chunkRequest>[1][],
): RenderRequest[] {
  const full: RenderRequest = { ...base, fromBar: 0, lookbackSec: 0, mode: 'mix' };
  delete full.mutation;
  const reqs = [full, ...plans.map((p) => chunkRequest(base, p))];
  if (fixture.gates.includes('stems')) reqs.push(...plans.map((p) => stemsRequest(base, p)));
  return reqs;
}

/** A chunk plan's content window. */
const chunkBars = (p: { contentStartBar: number; contentEndBar: number }) => ({
  fromBar: p.contentStartBar,
  toBar: p.contentEndBar,
});

async function main(argv: string[]): Promise<number> {
  const parsed = parseParityArgs(argv);
  if ('error' in parsed) {
    console.error(`host:parity: ${parsed.error}\n\n${PARITY_USAGE}`);
    return 2;
  }
  if (parsed.help) {
    console.log(PARITY_USAGE);
    return 0;
  }
  const outDir = resolve(parsed.outDir);
  const loaded = loadCorpus();
  if ('error' in loaded) {
    for (const line of loaded.error) console.error(`host:parity: ${line}`);
    return 2;
  }
  const { corpus, io } = loaded;
  const selected = selectFixtures(corpus, parsed.filters);
  if ('error' in selected) {
    console.error(`host:parity: ${selected.error}`);
    return 2;
  }
  if (!ensureBuilt(false)) return 2;

  let host: HostPlanner;
  try {
    if (!process.env.STRUDELAND_HOST_BIN) {
      console.log('host:parity: building strudeland-host (cargo build --release)');
    }
    const bin = await findHostBinary(process.env, bunRunner, ROOT);
    host = new HostPlanner({ bin: bin.path, version: bin.version, outDir });
  } catch (e) {
    if (e instanceof HarnessError) {
      console.error(`host:parity: ${e.code}: ${e.message}`);
      return 2;
    }
    throw e;
  }
  const page = new SuperdoughRenderer();
  const pageLimit = new Limiter(PAGE_CONCURRENCY);
  const hostLimit = new Limiter(HOST_CONCURRENCY);
  const readSong = (f: string) => io.readSong(f) ?? '';

  const compareBatchFixture = async (fixture: FixtureEntry): Promise<FixtureParity> => {
    const dir = join(outDir, 'host-parity', fixture.name);
    rmSync(dir, { recursive: true, force: true });
    const code = readSong(fixture.file);
    const [fromBar, toBar] = fixture.windowBars;
    const window = { fromBar, toBar, layout: fixture.layout };
    const prepared = await prepareFixture(fixture, {
      renderer: page,
      corpus,
      readSong,
      limit: pageLimit,
    });
    if ('errors' in prepared || 'unsupported' in prepared) {
      if (!('errors' in prepared)) throw new Error('the page renderer reported unsupported haps');
      const h = await planBatch(host, hostLimit, code, window);
      const outcome: PlanOutcome = h.ok
        ? { ok: true, errors: h.planned.errors.map(formatRenderError) }
        : h;
      const pageErr = prepared.errors.join('; ');
      return scheduleFailed(outcome)
        ? {
            status: 'ok',
            requests: 1,
            lines: [
              `${fixture.name}: both failed (page: ${pageErr}; host: ${failureText(outcome)})`,
            ],
          }
        : {
            status: 'FAIL',
            requests: 1,
            lines: [`${fixture.name} batch: the page failed (${pageErr}), the host planned`],
          };
    }
    const { fx } = prepared;
    const plans = renderChunkPlans({
      fromBar,
      toBar,
      events: fx.sched.events,
      cps: fx.cps,
      overlapBars: 0,
    });
    const [pages, h] = await Promise.all([
      Promise.all(plans.map((p) => plan(page, pageLimit, chunkRequest(fx.base, p)))),
      planBatch(host, hostLimit, code, window),
    ]);
    const out: FixtureParity = { status: 'ok', requests: plans.length, lines: [] };
    const at = `${fixture.name} batch`;
    const pageFailures = pages.flatMap((p, i) => (p.ok ? [] : [`chunk ${i}: ${p.message}`]));
    if (!h.ok) {
      if (pageFailures.length === plans.length) {
        out.lines.push(`${at}: both failed (page: ${pageFailures.join('; ')}; host: ${h.message})`);
      } else {
        out.status = 'FAIL';
        out.lines.push(`${at}: the host failed: ${h.message}`);
      }
      return out;
    }
    if (pageFailures.length > 0) {
      out.status = 'FAIL';
      for (const f of pageFailures) out.lines.push(`${at}: the page failed: ${f}`);
    }
    const texts = pages.map((p) => (p.ok ? p.text : null));
    const differences = compareBatch(plans.map(chunkBars), texts, h.planned.chunks);
    for (const d of differences) {
      if (out.status === 'ok') out.status = 'DIFF';
      if (d.kind === 'text') {
        mkdirSync(dir, { recursive: true });
        writeFileSync(join(dir, `batch-${d.index}.page.json`), texts[d.index]!);
        writeFileSync(join(dir, `batch-${d.index}.host.json`), h.planned.chunks[d.index]!.json);
      }
      out.lines.push(`${at}: ${formatBatchDifference(d)}`);
    }
    return out;
  };

  const compare = async (fixture: FixtureEntry): Promise<FixtureParity> => {
    const dir = join(outDir, 'host-parity', fixture.name);
    rmSync(dir, { recursive: true, force: true });
    const prepared = await prepareFixture(fixture, {
      renderer: page,
      corpus,
      readSong,
      limit: pageLimit,
    });
    if ('errors' in prepared || 'unsupported' in prepared) {
      // The page cannot schedule the song: the host must fail on the full window too, by the
      // same rule (a failed plan or errors in its info, `scheduleFailed`).
      if (!('errors' in prepared)) throw new Error('the page renderer reported unsupported haps');
      const full: RenderRequest = {
        code: readSong(fixture.file),
        layout: fixture.layout,
        fromBar: 0,
        toBar: fixture.windowBars[1],
        overlapBars: 0,
        lookbackSec: 0,
        mode: 'mix',
        sampleRate: SAMPLE_RATE,
      };
      const h = await plan(host, hostLimit, full);
      const pageErr = prepared.errors.join('; ');
      return scheduleFailed(h)
        ? {
            status: 'ok',
            requests: 1,
            lines: [`${fixture.name}: both failed (page: ${pageErr}; host: ${failureText(h)})`],
          }
        : {
            status: 'FAIL',
            requests: 1,
            lines: [`${fixture.name} request 0: the page failed (${pageErr}), the host planned`],
          };
    }
    const { fx } = prepared;
    const reqs = requestSet(fixture, fx.base, fx.plans);
    const results = await Promise.all(
      reqs.map(async (req) => {
        const [p, h] = await Promise.all([plan(page, pageLimit, req), plan(host, hostLimit, req)]);
        return { p, h };
      }),
    );
    const out: FixtureParity = { status: 'ok', requests: reqs.length, lines: [] };
    results.forEach(({ p, h }, n) => {
      const at = `${fixture.name} request ${n}`;
      if (!p.ok || !h.ok) {
        if (!p.ok && !h.ok) {
          out.lines.push(`${at}: both failed (page: ${p.message}; host: ${h.message})`);
          return;
        }
        out.status = 'FAIL';
        out.lines.push(
          h.ok
            ? `${at}: the page failed: ${(p as { message: string }).message}`
            : `${at}: the host failed: ${h.message}`,
        );
        return;
      }
      if (p.text === h.text) return;
      if (out.status === 'ok') out.status = 'DIFF';
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, `${n}.page.json`), p.text);
      writeFileSync(join(dir, `${n}.host.json`), h.text);
      let where: string;
      try {
        where = firstDifference(JSON.parse(p.text), JSON.parse(h.text)) ?? 'key order or spacing';
      } catch (e) {
        where = `unparseable JSON (${errorMessage(e)})`;
      }
      out.lines.push(`${at}: first difference at ${where}`);
    });
    return out;
  };

  try {
    console.log(
      `host:parity${parsed.batch ? ' --batch' : ''}: ${selected.length} fixture(s), ` +
        `chromium ${await page.version()}, host ${await host.version()}`,
    );
    const pending = selected.map((f) => (parsed.batch ? compareBatchFixture(f) : compare(f)));
    // Rejections are read in order below; keep the others from being reported as unhandled.
    for (const p of pending) p.catch(() => {});
    const counts = { ok: 0, DIFF: 0, FAIL: 0 };
    for (const [i, fixture] of selected.entries()) {
      const r = await pending[i]!;
      counts[r.status]++;
      console.log(
        `${r.status.padEnd(4)} ${fixture.name} (${r.requests} ${parsed.batch ? 'chunk' : 'request'}(s))`,
      );
      for (const line of r.lines) console.log(`  ${line}`);
    }
    console.log(`${counts.ok} ok, ${counts.DIFF} differ, ${counts.FAIL} failed`);
    return counts.DIFF + counts.FAIL === 0 ? 0 : 1;
  } catch (e) {
    if (e instanceof HarnessError) {
      console.error(`host:parity: ${e.code}: ${e.message}`);
      return 2;
    }
    console.error(`host:parity: ${errorMessage(e)}`);
    return 2;
  } finally {
    await page.dispose().catch(() => {});
    await host.dispose();
  }
}

if (import.meta.main) {
  process.exit(await main(Bun.argv.slice(2)));
}

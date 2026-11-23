/**
 * `bun run render:check`: renders the corpus in headless Chromium, runs each fixture's gates,
 * compares features with the goldens, runs the mutation set and writes the report.
 *
 * `runCheck(opts, deps)` is the whole flow over an injected `Renderer`, fs and clock (tested with
 * a fake renderer); `main` does the argument parsing, manifest validation and page build, and
 * imports the Playwright renderer only after those succeed. Exit 0 all pass, 1 a fixture failed or
 * a mutation was missed, 2 harness error.
 *
 * `--engine rust` renders with the Rust engine (`rust-renderer.ts`) and judges it against the
 * superdough goldens with `PROFILES.engine`. A fixture whose schedule has haps the engine cannot
 * play is reported `unsupported` without gates; it fails the run only when its corpus.json
 * `rust` field disagrees. `--planner host` plans its jobs with the headless host
 * (`host-planner.ts`) instead of the render page, so that run opens no browser. `--planner batch`
 * is `host`, and each fixture's main window (`fx.assembled`: the mix and bass gates, the parity
 * features, `--keep-wav`) comes from the whole Rust pipeline (`batch-render.ts`) instead of the
 * joined chunk renders; twin, seam, stems and the mutations keep the per-request host plans.
 */

import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import {
  isBuildStale,
  parseCheckArgs,
  plansWithHost,
  selectFixtures,
  USAGE,
  type CheckOptions,
} from '../../src/lib/render/check-options';
import { ENGINE_TRAITS, type EngineName } from '../../src/lib/render/engines';
import {
  SAMPLE_RATE as SR,
  type RenderRequest,
  type RenderResult,
  type Renderer,
  type ScheduleResult,
} from '../../src/lib/render/contract';
import {
  assembleWindow,
  chunkRequest,
  contentOf,
  headOf,
  overlapOf,
  renderChunkPlans,
  type AssembledWindow,
  type ChunkPlan,
} from '../../src/lib/render/chunks';
import {
  samplesMap,
  validateCorpus,
  type Corpus,
  type CorpusIo,
  type FixtureEntry,
} from '../../src/lib/render/corpus';
import { formatSignature, type UnsupportedSignature } from '../../src/lib/render/engine-job';
import {
  extractFeatures,
  parseFeatures,
  serializeFeatures,
  type Features,
} from '../../src/lib/render/features';
import { bassGate, bassPresence } from '../../src/lib/render/gates/bass';
import { deriveOpening, verifyMix } from '../../src/lib/render/gates/mix';
import { seamWithTwinEscalation } from '../../src/lib/render/gates/seam';
import {
  judgeStemScores,
  mergePairVerdicts,
  stemsGate,
  stemsOnsetScores,
  type PairVerdict,
  type StemNoteScore,
} from '../../src/lib/render/gates/stems';
import { twinNull, type TwinMode } from '../../src/lib/render/gates/twin';
import type { GateName, GateResult } from '../../src/lib/render/gates/types';
import { lookbackBars, lookbackSeconds } from '../../src/lib/render/lookback';
import { compareFeatures, profileFor } from '../../src/lib/render/parity';
import {
  formatReport,
  goldenSummary,
  reportExitCode,
  reportJson,
  combineResults,
  type FixtureReport,
  type GoldenChange,
  type MutationReport,
  type Report,
} from '../../src/lib/render/report';
import { BASS_MISS_DB, STEMS_MISS_DB, TWIN_BITEXACT_HOLDS } from '../../src/lib/render/thresholds';
import {
  layoutSegments,
  renderTiming,
  renderWindowFrames,
  type RenderTiming,
} from '../../src/lib/render/timing';
import { encodeWav } from '../../src/lib/render/wav';
import { errorMessage } from '../../src/lib/errors';
import { readPlaywrightPin } from '../playwright-pin';
import type { JobSource } from './job-source';
import { judgeMutation, MUTATIONS, validateMutations, type MutationSpec } from './mutations';
import {
  formatRenderError,
  HarnessError,
  isFatalHarnessError,
  RenderFailedError,
} from './transfer';

export interface CheckFs {
  /** File text, or null when it does not exist. */
  readText(path: string): string | null;
  /** Writes the file, creating its folder. */
  writeText(path: string, text: string): void;
  writeBytes(path: string, bytes: Uint8Array): void;
}

export interface CheckClock {
  /** Milliseconds, monotonic. */
  now(): number;
  /** Wall-clock ISO time, for `startedAt`. */
  iso(): string;
}

export interface CheckDeps {
  /** Its `engine` picks the twin mode and the parity profile. */
  renderer: Renderer;
  /** The browser version (`renderer.version()`; Chromium renders the page for both engines);
   *  `none` when the headless host plans the Rust engine's jobs (`--planner host`). */
  engineVersion: string;
  /** The Rust engine build (`strudeland-engine <version> (<sha>)`); null for superdough. */
  rustVersion: string | null;
  /** The exact `playwright` devDependency version. */
  playwright: string;
  /** A validated corpus. */
  corpus: Corpus;
  /** Song text of `tests/render/corpus/<file>`. */
  readSong(file: string): string;
  fs: CheckFs;
  clock: CheckClock;
  /** `tests/render/goldens`. */
  goldensDir: string;
  /** Default MUTATIONS. */
  mutations?: MutationSpec[];
  /** Progress lines; default none. */
  log?: (line: string) => void;
  /** `--planner batch`: the fixture's main window from the whole Rust pipeline, set as
   *  `fx.assembled` before any gate reads it. Absent: the joined chunk renders. */
  batchWindow?: (fx: FixtureContext) => Promise<AssembledWindow>;
}

export interface CheckRun {
  report: Report;
  exitCode: 0 | 1 | 2;
}

/** A mutation's `apply`; undefined everywhere means the unmutated run. */
export type RequestTx = (r: RenderRequest) => RenderRequest;

const applyTx = (req: RenderRequest, tx?: RequestTx): RenderRequest => (tx ? tx(req) : req);

/** A deterministic engine's twins are always compared bit for bit. */
export function twinMode(
  bitExactHolds: boolean,
  stochastic: boolean,
  engine: EngineName,
): TwinMode {
  if (ENGINE_TRAITS[engine].deterministic) return 'exact';
  return bitExactHolds && !stochastic ? 'exact' : 'tolerance';
}

/** At most `n` tasks at once, started in call order. */
export class Limiter {
  private active = 0;
  private readonly waiting: (() => void)[] = [];
  constructor(private readonly n: number) {}

  async run<T>(task: () => Promise<T>): Promise<T> {
    if (this.active < this.n) this.active++;
    else await new Promise<void>((r) => this.waiting.push(r));
    try {
      return await task();
    } finally {
      // Hand the slot to the next waiter, or free it.
      const next = this.waiting.shift();
      if (next) next();
      else this.active--;
    }
  }
}

/** Renders that stop together: after the first failure, those still waiting for a slot fail
 *  with the same error without rendering. */
export interface RenderBatch {
  failure?: { error: unknown };
}

/** Everything a gate needs about one fixture's planned window. */
export interface FixtureContext {
  fixture: FixtureEntry;
  base: RenderRequest;
  sched: ScheduleResult;
  cps: number;
  barSec: number;
  timing?: RenderTiming;
  plans: ChunkPlan[];
  /** Unmutated mix renders by `JSON.stringify(request)`, in flight or done. */
  memo: Map<string, Promise<RenderResult>>;
  renderer: Renderer;
  /** Bounds the renders (and schedules) of every fixture sharing it. */
  limit: Limiter;
  /** Control renders started ahead of their gate: the twin re-render of chunk 0 and the stems
   *  chunks. Taken (and cleared) by the gate. */
  twinRender?: Promise<RenderResult>;
  stemsRenders?: Promise<RenderResult>[];
  /** Set when --keep-wav: the stems gate keeps its chunk contents here. */
  stemsContent?: Float32Array[][];
  assembled?: AssembledWindow;
  log: (line: string) => void;
}

export async function renderOk(renderer: Renderer, req: RenderRequest): Promise<RenderResult> {
  const r = await renderer.render(req);
  if (!r.ok) {
    throw new RenderFailedError(
      r.errors.length > 0 ? r.errors.map(formatRenderError) : ['other: render returned ok: false'],
    );
  }
  return r;
}

/** One uncached `renderOk` under the fixture's limit, logged with `label` when given. Callers
 *  may stop awaiting after a sibling fails, so a rejection is never left unhandled. */
export function renderIn(
  fx: FixtureContext,
  req: RenderRequest,
  batch: RenderBatch = {},
  label?: string,
): Promise<RenderResult> {
  const p = fx.limit.run(async () => {
    if (batch.failure) throw batch.failure.error;
    const t0 = performance.now();
    try {
      const r = await renderOk(fx.renderer, req);
      if (label) {
        fx.log(`  ${fx.fixture.name}: ${label} ${((performance.now() - t0) / 1000).toFixed(1)}s`);
      }
      return r;
    } catch (e) {
      batch.failure ??= { error: e };
      throw e;
    }
  });
  p.catch(() => {});
  return p;
}

/** renderIn, memoized for unmutated mix requests (stems results are never kept); concurrent
 *  callers share one render. */
function memoRender(
  fx: FixtureContext,
  req: RenderRequest,
  batch: RenderBatch,
  label?: string,
): Promise<RenderResult> {
  if (!(req.mode === 'mix' && req.mutation === undefined)) return renderIn(fx, req, batch, label);
  const key = JSON.stringify(req);
  let p = fx.memo.get(key);
  if (!p) {
    p = renderIn(fx, req, batch, label);
    fx.memo.set(key, p);
  }
  return p;
}

/** Every chunk of the plan, rendered concurrently (within the fixture's limit). */
export function chunkResults(
  fx: FixtureContext,
  tx?: RequestTx,
  batch: RenderBatch = {},
): Promise<RenderResult[]> {
  const n = fx.plans.length;
  return Promise.all(
    fx.plans.map((plan) =>
      memoRender(
        fx,
        applyTx(chunkRequest(fx.base, plan), tx),
        batch,
        n > 1 ? `chunk ${plan.index + 1}/${n}` : undefined,
      ),
    ),
  );
}

export async function assembledOf(
  fx: FixtureContext,
  tx?: RequestTx,
  batch: RenderBatch = {},
): Promise<AssembledWindow> {
  if (tx === undefined && fx.assembled) return fx.assembled;
  const a = assembleWindow(await chunkResults(fx, tx, batch), fx.sched.events);
  if (tx === undefined) fx.assembled = a;
  return a;
}

/** The stems-mode request of a chunk (no overlap); shared with `engine:refs`. */
export const stemsRequest = (base: RenderRequest, plan: ChunkPlan): RenderRequest => ({
  ...chunkRequest(base, plan),
  mode: 'stems',
  overlapBars: 0,
});

/** Starts the control renders the fixture's twin and stems gates will need, behind the chunks
 *  already queued in `batch`. */
function prefetchGateRenders(fx: FixtureContext, batch: RenderBatch): void {
  const { gates } = fx.fixture;
  if (gates.includes('twin')) {
    fx.twinRender = renderIn(fx, chunkRequest(fx.base, fx.plans[0]!), batch);
  }
  if (gates.includes('stems')) {
    fx.stemsRenders = fx.plans.map((plan) => renderIn(fx, stemsRequest(fx.base, plan), batch));
  }
}

/** A prefetched control render, once; undefined for a mutated run or when none was started. */
function takePrefetched<K extends 'twinRender' | 'stemsRenders'>(
  fx: FixtureContext,
  key: K,
  tx: RequestTx | undefined,
): FixtureContext[K] {
  if (tx !== undefined) return undefined;
  const p = fx[key];
  delete fx[key];
  return p;
}

function seamShortfall(fx: FixtureContext): GateResult | null {
  const want = fx.fixture.expectChunks ?? 2;
  if (fx.plans.length >= want) return null;
  return {
    gate: 'seam',
    verdict: 'fail',
    measurements: { chunks: fx.plans.length, expectChunks: want },
    reasons: [`planned ${fx.plans.length} chunk(s), expected >= ${want}`],
    warnings: [],
  };
}

/** One gate of a planned fixture. `tx` is undefined for a normal run and a mutation's `apply`
 *  otherwise; it reaches every request the gate renders, while the gates always get the
 *  unmutated `sched.events`. Throws RenderFailedError on a failed render. */
export async function runGate(
  gate: GateName,
  fx: FixtureContext,
  tx?: RequestTx,
  batch: RenderBatch = {},
): Promise<GateResult> {
  const { base, plans, cps, barSec, sched, fixture, timing } = fx;
  const { fromBar, toBar } = base;
  switch (gate) {
    case 'mix': {
      const a = await assembledOf(fx, tx, batch);
      return verifyMix(a.channels, {
        sampleRate: SR,
        expectedFrames: renderWindowFrames(fromBar, toBar, cps, SR, timing),
        barSec,
        seamSecs: a.seamSecs,
        meteredWindow: timing ? { timing, fromBar } : undefined,
        opening: fromBar === 0 ? deriveOpening(sched.events) : undefined,
      });
    }
    case 'bass': {
      const a = await assembledOf(fx, tx, batch);
      return bassGate(
        bassPresence(a.channels, SR, sched.events, {
          missDb: BASS_MISS_DB,
          offsetSec: a.bufferStartSec,
          barSec: timing ? undefined : barSec,
        }),
        timing?.secondsAt ?? barSec,
      );
    }
    case 'twin': {
      const chunks = chunkResults(fx, tx, batch);
      // The re-render is deliberately uncached: the gate compares two independent renders.
      const twinReq = applyTx(chunkRequest(base, plans[0]!), tx);
      const twin = takePrefetched(fx, 'twinRender', tx) ?? renderIn(fx, twinReq, batch);
      const [results, second] = await Promise.all([chunks, twin]);
      const first = results[0]!;
      return twinNull(first.channels, second.channels, {
        sampleRate: SR,
        mode: twinMode(TWIN_BITEXACT_HOLDS, fixture.stochastic, fx.renderer.engine),
      });
    }
    case 'seam': {
      const short = seamShortfall(fx);
      if (short) return short;
      const results = await chunkResults(fx, tx, batch);
      const perSeam: GateResult[] = [];
      for (let k = 0; k + 1 < plans.length; k++) {
        const bars = plans[k]!.overlapBars;
        perSeam.push(
          await seamWithTwinEscalation(
            overlapOf(results[k]!),
            headOf(results[k + 1]!, bars, timing),
            SR,
            timing ? timing.secondsAt(plans[k]!.contentEndBar) : plans[k]!.contentEndBar * barSec,
            async () =>
              headOf(
                await renderIn(fx, applyTx(chunkRequest(base, plans[k + 1]!), tx), batch),
                bars,
                timing,
              ),
          ),
        );
      }
      return combineResults('seam', perSeam);
    }
    case 'stems': {
      const notes: StemNoteScore[] = [];
      const perChunk: PairVerdict[][] = [];
      const renders =
        takePrefetched(fx, 'stemsRenders', tx) ??
        plans.map((plan) => renderIn(fx, applyTx(stemsRequest(base, plan), tx), batch));
      for (const pending of renders) {
        const r = await pending;
        const content = contentOf(r);
        if (fx.stemsContent && tx === undefined) fx.stemsContent.push(content);
        const scored = stemsOnsetScores(content, SR, sched.events, timing ? undefined : barSec, {
          offsetSec: timing ? timing.secondsAt(r.request.fromBar) : r.request.fromBar / cps,
        });
        notes.push(...scored.notes);
        perChunk.push(scored.pairs);
      }
      return stemsGate(mergePairVerdicts(perChunk), judgeStemScores(notes, STEMS_MISS_DB));
    }
  }
}

/** The manifest's promises: `expect.clip`, `expect.unverifiable`, and that every other listed
 *  gate is verifiable. */
export function applyExpectations(fixture: FixtureEntry, g: GateResult): GateResult {
  let out = g;
  if (g.gate === 'mix' && fixture.expect?.clip) {
    const clip = g.measurements.clipFraction;
    if (!(typeof clip === 'number' && clip > 0)) {
      out = { ...out, verdict: 'fail', reasons: [...out.reasons, 'expected clipping'] };
    }
  }
  const expectUnver = fixture.expect?.unverifiable?.includes(g.gate) ?? false;
  if (expectUnver && out.verdict !== 'unverifiable') {
    out = { ...out, verdict: 'fail', reasons: [...out.reasons, 'expected unverifiable'] };
  } else if (!expectUnver && out.verdict === 'unverifiable') {
    out = { ...out, verdict: 'fail', reasons: out.reasons.map((r) => `unverifiable: ${r}`) };
    if (out.reasons.length === 0) out.reasons = ['unverifiable'];
  }
  return out;
}

/** Errors that stop the whole run (exit 2): the environment (no Chromium, no built page) or a
 *  broken harness. A page that fails or stalls on one fixture is that fixture's error. */
const isFatal = (e: unknown): boolean => e instanceof HarnessError && isFatalHarnessError(e);

/** A per-fixture failure as report lines. */
function fixtureErrorLines(e: unknown): string[] {
  if (e instanceof RenderFailedError) return e.lines;
  if (e instanceof HarnessError) return [`${e.code}: ${e.message}`];
  return [`exception: ${errorMessage(e)}`];
}

/** A gate that stands: pass, or unverifiable after applyExpectations (so expected). */
const gateOk = (g: GateResult) => g.verdict !== 'fail';

/** Chunks of channels joined end to end. Chunks may have different channel counts (each stems
 *  chunk has its own channel plan): the result has the widest count, and a chunk missing a
 *  channel contributes silence of its length there. */
export function concatChannels(parts: Float32Array[][]): Float32Array[] {
  const nch = Math.max(0, ...parts.map((p) => p.length));
  const lens = parts.map((p) => Math.max(0, ...p.map((c) => c.length)));
  const total = lens.reduce((n, l) => n + l, 0);
  return Array.from({ length: nch }, (_, c) => {
    const out = new Float32Array(total);
    let off = 0;
    parts.forEach((p, k) => {
      const ch = p[c];
      if (ch) out.set(ch, off);
      off += lens[k]!;
    });
    return out;
  });
}

/** The fixture's base request, schedule and chunk plan (no audio yet); or the schedule's
 *  errors; or (after a schedule without errors) the renderer's unsupported signatures, when
 *  there are any. Shared with the calibration probes. */
export async function prepareFixture(
  fixture: FixtureEntry,
  deps: Pick<CheckDeps, 'renderer' | 'corpus' | 'readSong'> & { limit?: Limiter },
  log: (line: string) => void = () => {},
): Promise<
  { fx: FixtureContext } | { errors: string[] } | { unsupported: UnsupportedSignature[] }
> {
  const limit = deps.limit ?? new Limiter(Infinity);
  const [fromBar, toBar] = fixture.windowBars;
  const base: RenderRequest = {
    code: deps.readSong(fixture.file),
    layout: fixture.layout,
    fromBar,
    toBar,
    overlapBars: 0,
    lookbackSec: 0,
    mode: 'mix',
    sampleRate: SR,
    ...(fixture.usesSamples ? { samples: samplesMap(deps.corpus) } : {}),
  };
  const sched = await limit.run(() => deps.renderer.schedule(base));
  if (sched.errors.length > 0) return { errors: sched.errors.map(formatRenderError) };
  if (sched.unsupported && sched.unsupported.length > 0) return { unsupported: sched.unsupported };
  const cps = sched.cps;
  const barSec = 1 / cps;
  const timing = renderTiming(sched.structure, layoutSegments(base.layout, sched.structure), cps);
  const lookbackSec = lookbackSeconds(sched.events);
  const plans = renderChunkPlans({
    fromBar,
    toBar,
    events: sched.events,
    cps,
    timing,
    lookbackSec,
  });
  const lb = lookbackBars(lookbackSec, timing?.minBarSec ?? barSec);
  const seconds = timing
    ? timing.secondsAt(toBar) - timing.secondsAt(fromBar)
    : barSec * (toBar - fromBar);
  log(
    `  ${fixture.name}: ${plans.length} chunk(s), lookback ${lb} bar(s), ` +
      `${seconds.toFixed(1)}s of audio`,
  );
  return {
    fx: {
      fixture,
      base,
      sched,
      cps,
      barSec,
      timing,
      plans,
      memo: new Map(),
      renderer: deps.renderer,
      limit,
      log,
    },
  };
}

interface FixtureOutcome {
  report: FixtureReport;
  golden: GoldenChange | null;
  /** Kept for the mutation phase. */
  ctx: FixtureContext | null;
}

async function runFixture(
  fixture: FixtureEntry,
  opts: CheckOptions,
  deps: CheckDeps & { limit: Limiter },
  keepCtx: boolean,
): Promise<FixtureOutcome> {
  const log = deps.log ?? (() => {});
  const t0 = deps.clock.now();
  const [fromBar, toBar] = fixture.windowBars;
  const report: FixtureReport = {
    name: fixture.name,
    status: 'pass',
    bars: toBar - fromBar,
    chunks: 0,
    seconds: 0,
    gates: [],
    parity: null,
    parityNote: null,
    errors: [],
    unsupported: [],
  };
  const finish = (golden: GoldenChange | null, ctx: FixtureContext | null): FixtureOutcome => {
    report.seconds = Math.round(deps.clock.now() - t0) / 1000;
    return { report, golden, ctx };
  };
  const fail = (lines: string[]) => {
    report.status = 'error';
    report.errors.push(...lines);
  };

  let fx: FixtureContext | null = null;
  try {
    const prepared = await prepareFixture(fixture, deps, log);
    if ('errors' in prepared) {
      fail(prepared.errors);
      return finish(opts.updateGoldens ? notWritten(fixture) : null, null);
    }
    const disagree = ENGINE_TRAITS[deps.renderer.engine].refusesHaps
      ? manifestDisagreement(fixture, prepared)
      : null;
    if ('unsupported' in prepared) {
      report.status = 'unsupported';
      report.unsupported = prepared.unsupported.map(formatSignature);
      if (disagree) report.errors.push(disagree);
      return finish(opts.updateGoldens ? notWritten(fixture) : null, null);
    }
    if (disagree) {
      fail([disagree]);
      return finish(null, null);
    }
    fx = prepared.fx;
    if (opts.keepWav && fixture.gates.includes('stems')) fx.stemsContent = [];
    report.chunks = fx.plans.length;

    // The chunks (or the batch render), then the twin and stems renders, all start at once
    // (within the limit).
    const batch: RenderBatch = {};
    const ctx = fx;
    const batchWindow = deps.batchWindow;
    const assembled = batchWindow
      ? ctx.limit.run(() => batchWindow(ctx)).then((a) => (ctx.assembled = a))
      : assembledOf(fx, undefined, batch);
    prefetchGateRenders(fx, batch);
    await assembled;
    for (const gate of fixture.gates) {
      report.gates.push(applyExpectations(fixture, await runGate(gate, fx, undefined, batch)));
    }
  } catch (e) {
    if (isFatal(e)) throw e;
    fail(fixtureErrorLines(e));
  }

  if (fx?.assembled && opts.keepWav) {
    try {
      const dir = join(opts.outDir, 'wav');
      deps.fs.writeBytes(
        join(dir, `${fixture.name}.wav`),
        encodeWav(fx.assembled.channels, SR, 'f32'),
      );
      if (fx.stemsContent && fx.stemsContent.length > 0) {
        deps.fs.writeBytes(
          join(dir, `${fixture.name}.stems.wav`),
          encodeWav(concatChannels(fx.stemsContent), SR, 'f32'),
        );
      }
    } catch (e) {
      fail([`keep-wav: ${errorMessage(e)}`]);
    }
  }

  if (report.status === 'error' || !fx?.assembled) {
    report.status = 'error';
    return finish(opts.updateGoldens ? notWritten(fixture) : null, null);
  }

  const gatesOk = report.gates.every(gateOk);
  let golden: GoldenChange | null = null;
  let candidate: Features | null = null;
  try {
    candidate = extractFeatures(fx.assembled);
  } catch (e) {
    report.parityNote = `features: ${errorMessage(e)}`;
  }
  const goldenPath = join(deps.goldensDir, `${fixture.name}.json`);
  const goldenText = deps.fs.readText(goldenPath);
  if (opts.updateGoldens) {
    if (gatesOk && candidate) {
      let old: Features | null = null;
      try {
        old = goldenText === null ? null : parseFeatures(goldenText);
      } catch {
        old = null;
      }
      deps.fs.writeText(goldenPath, serializeFeatures(candidate));
      golden = goldenSummary(fixture.name, old, candidate);
    } else {
      golden = notWritten(fixture);
    }
  } else if (candidate) {
    if (goldenText === null) {
      report.parityNote = 'no golden (run with --update-goldens)';
    } else {
      try {
        report.parity = compareFeatures(
          parseFeatures(goldenText),
          candidate,
          profileFor(deps.renderer.engine),
        );
      } catch (e) {
        report.parityNote = `golden ${fixture.name}.json: ${errorMessage(e)}`;
      }
    }
  }

  const parityOk = opts.updateGoldens
    ? report.parityNote === null
    : report.parityNote === null && report.parity?.verdict === 'pass';
  report.status = gatesOk && parityOk ? 'pass' : 'fail';
  delete fx.twinRender;
  delete fx.stemsRenders;
  if (!keepCtx) {
    fx.memo.clear();
    delete fx.assembled;
  }
  return finish(golden, keepCtx ? fx : null);
}

/** The corpus.json `rust` field against what the Rust renderer detected, or null when they
 *  agree. */
export function manifestDisagreement(
  fixture: FixtureEntry,
  prepared: { fx: FixtureContext } | { unsupported: UnsupportedSignature[] },
): string | null {
  const detected = 'unsupported' in prepared ? 'unsupported' : 'supported';
  if (fixture.rust === detected) return null;
  return detected === 'unsupported'
    ? 'corpus.json marks this fixture rust: supported, but the engine cannot play every hap; ' +
        'update `rust` in corpus.json (or the engine)'
    : 'corpus.json marks this fixture rust: unsupported, but the engine plays every hap; ' +
        'update `rust` in corpus.json';
}

const notWritten = (fixture: FixtureEntry): GoldenChange => ({
  name: fixture.name,
  state: 'not-written',
});

/** Runs `work` over `items` with `n` workers. The first thrown error stops new work and settles
 *  the returned promise at once (in-flight items are abandoned; the caller disposes the
 *  renderer). */
export async function pool<T>(
  items: T[],
  n: number,
  work: (item: T, index: number) => Promise<void>,
): Promise<unknown | null> {
  let next = 0;
  let failure: { error: unknown } | null = null;
  let abort!: () => void;
  const aborted = new Promise<void>((r) => (abort = r));
  const worker = async () => {
    while (failure === null && next < items.length) {
      const i = next++;
      try {
        await work(items[i]!, i);
      } catch (e) {
        failure ??= { error: e };
        abort();
        return;
      }
    }
  };
  await Promise.race([
    Promise.all(Array.from({ length: Math.min(n, items.length) }, worker)),
    aborted,
  ]);
  return failure === null ? null : (failure as { error: unknown }).error;
}

/** The whole check over the selected fixtures (`selected`, default selectFixtures of the
 *  filters); writes `<outDir>/report.json` in every case. */
export async function runCheck(
  opts: CheckOptions,
  deps: CheckDeps,
  selected: FixtureEntry[] | { error: string } = selectFixtures(deps.corpus, opts.filters),
): Promise<CheckRun> {
  const log = deps.log ?? (() => {});
  const report: Report = {
    schema: 1,
    startedAt: deps.clock.iso(),
    engine: deps.renderer.engine,
    engineVersion: deps.rustVersion,
    playwright: deps.playwright,
    chromium: deps.engineVersion,
    fixtures: [],
    mutations: [],
    goldens: [],
    harnessError: null,
  };
  const write = (exitCode: 0 | 1 | 2): CheckRun => {
    deps.fs.writeText(join(opts.outDir, 'report.json'), reportJson(report));
    return { report, exitCode };
  };

  if ('error' in selected) {
    report.harnessError = selected.error;
    return write(2);
  }
  const specs = opts.mutations ? (deps.mutations ?? MUTATIONS) : [];
  const targets = new Set(specs.map((s) => s.fixture));

  const limit = new Limiter(opts.concurrency);
  const fixtureDeps = { ...deps, limit };

  // A mutation starts as soon as its fixture's control run is done, sharing the render limit
  // with the fixtures still running; reports keep the order of `specs`.
  const mutationReports: (MutationReport | undefined)[] = new Array(specs.length);
  const mutationRuns: Promise<void>[] = [];
  let mFailure: { error: unknown } | null = null;
  const runMutation = async (spec: MutationSpec, i: number, outcome: FixtureOutcome) => {
    const ctx = outcome.ctx;
    const control = ctx ? outcome.report.gates.find((g) => g.gate === spec.gate) : undefined;
    let mutated: GateResult | undefined;
    let error: string | undefined;
    const runs = ctx !== null && control?.verdict === 'pass';
    if (runs) {
      log(`mutation ${spec.name} on ${spec.fixture}`);
      try {
        mutated = await runGate(spec.gate, ctx, spec.apply);
      } catch (e) {
        if (isFatal(e)) {
          mFailure ??= { error: e };
          return;
        }
        error =
          e instanceof RenderFailedError
            ? `mutated render failed: ${e.lines.join('; ')}`
            : fixtureErrorLines(e).join('; ');
      }
    }
    const m = judgeMutation(spec, control, mutated, error);
    mutationReports[i] = m;
    if (runs) log(`  ${spec.name}: ${m.status}`);
  };

  const outcomes: (FixtureOutcome | undefined)[] = new Array(selected.length);
  let done = 0;
  const failure = await pool(selected, opts.concurrency, async (fixture, i) => {
    log(`start ${fixture.name} (bars ${fixture.windowBars[0]}-${fixture.windowBars[1]})`);
    const out = await runFixture(fixture, opts, fixtureDeps, targets.has(fixture.name));
    outcomes[i] = out;
    done++;
    log(
      `[${done}/${selected.length}] ${fixture.name}  ${out.report.status}  ` +
        `${out.report.seconds.toFixed(1)}s`,
    );
    for (const [j, spec] of specs.entries()) {
      if (spec.fixture === fixture.name) mutationRuns.push(runMutation(spec, j, out));
    }
  });
  for (const o of outcomes) {
    if (!o) continue;
    report.fixtures.push(o.report);
    if (o.golden) report.goldens.push(o.golden);
  }
  if (failure !== null) {
    report.harnessError = failure instanceof Error ? failure.message : String(failure);
    return write(2);
  }

  await Promise.all(mutationRuns);
  const selectedNames = new Set(selected.map((f) => f.name));
  for (const [i, spec] of specs.entries()) {
    if (selectedNames.has(spec.fixture)) continue;
    mutationReports[i] = {
      name: spec.name,
      fixture: spec.fixture,
      gate: spec.gate,
      status: 'skipped',
      detail: 'fixture not selected',
    };
  }
  report.mutations = mutationReports.filter((m): m is MutationReport => m !== undefined);
  if (mFailure !== null) {
    const error = (mFailure as { error: unknown }).error;
    report.harnessError = error instanceof Error ? error.message : String(error);
    return write(2);
  }
  return write(reportExitCode(report));
}

// ---- command line

const ROOT = resolve(import.meta.dir, '../..');
const CORPUS_DIR = join(ROOT, 'tests/render/corpus');
const MANIFEST = join(ROOT, 'tests/render/corpus.json');
const GOLDENS_DIR = join(ROOT, 'tests/render/goldens');
const BUILT = join(ROOT, 'dist-render/index.html');

const nodeFs: CheckFs = {
  readText: (p) => (existsSync(p) ? readFileSync(p, 'utf8') : null),
  writeText: (p, text) => {
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, text);
  },
  writeBytes: (p, bytes) => {
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, bytes);
  },
};

function newestMtime(path: string, filter: (file: string) => boolean = () => true): number {
  if (!existsSync(path)) return 0;
  const st = statSync(path);
  if (!st.isDirectory()) return filter(path) ? st.mtimeMs : 0;
  let newest = 0;
  for (const name of readdirSync(path)) {
    if (name === 'node_modules') continue;
    newest = Math.max(newest, newestMtime(join(path, name), filter));
  }
  return newest;
}

/** Newest mtime among the render page's sources (04 section 4.2). */
function newestRenderSource(root: string = ROOT): number {
  return Math.max(
    newestMtime(join(root, 'src')),
    newestMtime(join(root, 'render')),
    newestMtime(join(root, 'vite.render.config.ts')),
    newestMtime(join(root, 'package.json')),
    newestMtime(join(root, 'bun.lock')),
    newestMtime(join(root, 'vendor/strudel/packages'), (f) => /\.(mjs|ts|js)$/.test(f)),
  );
}

const SONG_IO: CorpusIo = {
  readSong: (f) => nodeFs.readText(join(CORPUS_DIR, f)),
  sampleExists: (f) => existsSync(join(CORPUS_DIR, 'samples', f)),
  listSongs: () => readdirSync(CORPUS_DIR).filter((f) => f.endsWith('.js')),
};

/** Reads and validates `tests/render/corpus.json` (error lines on failure). */
export function loadCorpus(): { corpus: Corpus; io: CorpusIo } | { error: string[] } {
  let raw: unknown;
  try {
    raw = JSON.parse(readFileSync(MANIFEST, 'utf8'));
  } catch (e) {
    return { error: [`cannot read ${MANIFEST}: ${(e as Error).message}`] };
  }
  const { corpus, errors } = validateCorpus(raw, SONG_IO);
  if (!corpus) {
    return {
      error: [
        'invalid tests/render/corpus.json',
        ...errors.map((e) => `  ${e.path || '(manifest)'}: ${e.message}`),
      ],
    };
  }
  return { corpus, io: SONG_IO };
}

/** Builds the render page when asked or stale; false when the build failed. */
export function ensureBuilt(rebuild: boolean): boolean {
  const built = existsSync(BUILT) ? statSync(BUILT).mtimeMs : null;
  if (rebuild || isBuildStale(newestRenderSource(), built)) {
    console.log('render:check: building the render page');
    const build = Bun.spawnSync(['bun', 'run', 'render:build'], {
      cwd: ROOT,
      stdio: ['inherit', 'inherit', 'inherit'],
    });
    if (build.exitCode !== 0) {
      console.error(`render:check: render:build exited ${build.exitCode}`);
      return false;
    }
  }
  return true;
}

/** The exact `playwright` devDependency version; `unknown` when package.json has none. */
export function playwrightPin(): string {
  return readPlaywrightPin(ROOT) ?? 'unknown';
}

/** The renderer for `engine`; for rust, builds the engine first (`STRUDELAND_ENGINE_BIN`
 *  skips the build) and, with `planner` host or batch, the host (`STRUDELAND_HOST_BIN` skips that build;
 *  no Chromium page is created then). Imports the Playwright renderers lazily. A failed build
 *  throws `HarnessError('build-missing')`. `plannerVersion` is the host build for the host
 *  planner, null when the render page plans. */
export async function createRenderer(
  engine: EngineName,
  opts: Pick<CheckOptions, 'outDir' | 'keepWav' | 'planner'>,
): Promise<{
  renderer: Renderer & { version(): Promise<string> };
  rustVersion: string | null;
  plannerVersion: string | null;
}> {
  if (engine === 'superdough') {
    const { SuperdoughRenderer } = await import('./superdough-renderer');
    return { renderer: new SuperdoughRenderer(), rustVersion: null, plannerVersion: null };
  }
  const { bunRunner, findEngineBinary } = await import('./rust-engine');
  const override = process.env.STRUDELAND_ENGINE_BIN;
  console.log(
    override
      ? `render:check: engine binary from STRUDELAND_ENGINE_BIN: ${override}`
      : 'render:check: building strudeland-engine (cargo build --release)',
  );
  const { path: bin, version } = await findEngineBinary(process.env, bunRunner, ROOT);
  if (version.endsWith('-dirty)')) {
    console.log('render:check: warning: the working tree has uncommitted changes');
  }
  let jobs: JobSource;
  let plannerVersion: string | null = null;
  if (plansWithHost(opts.planner)) {
    const { findHostBinary, HostPlanner } = await import('./host-planner');
    if (!process.env.STRUDELAND_HOST_BIN) {
      console.log('render:check: building strudeland-host (cargo build --release)');
    }
    const host = await findHostBinary(process.env, bunRunner, ROOT);
    console.log(`render:check: planning with strudeland-host (${host.path})`);
    jobs = new HostPlanner({ bin: host.path, version: host.version, outDir: opts.outDir });
    plannerVersion = host.version;
  } else {
    const { SuperdoughRenderer } = await import('./superdough-renderer');
    jobs = new SuperdoughRenderer();
  }
  const { RustRenderer } = await import('./rust-renderer');
  return {
    renderer: new RustRenderer({
      bin,
      version,
      outDir: opts.outDir,
      keepWav: opts.keepWav,
      jobs,
    }),
    rustVersion: version,
    plannerVersion,
  };
}

/** `--planner batch`: builds `strudeland-render` (`STRUDELAND_RENDER_BIN` skips the build; a
 *  failed build throws `HarnessError('build-missing')`) and returns the fixtures' main-window
 *  render and the binary's version. */
export async function createBatchWindow(outDir: string): Promise<{
  batchWindow: NonNullable<CheckDeps['batchWindow']>;
  version: string;
}> {
  const { bunRunner } = await import('./rust-engine');
  const { BatchRenderer, findRenderBinary } = await import('./batch-render');
  if (!process.env.STRUDELAND_RENDER_BIN) {
    console.log('render:check: building strudeland-render (cargo build --release)');
  }
  const bin = await findRenderBinary(process.env, bunRunner, ROOT);
  console.log(`render:check: main windows rendered by strudeland-render (${bin.path})`);
  const batch = new BatchRenderer({ bin: bin.path, outDir });
  return {
    batchWindow: (fx) =>
      batch.window({
        name: fx.fixture.name,
        songPath: join(CORPUS_DIR, fx.fixture.file),
        base: fx.base,
        cps: fx.cps,
        events: fx.sched.events,
        timing: fx.timing,
      }),
    version: bin.version,
  };
}

async function main(argv: string[]): Promise<number> {
  const parsed = parseCheckArgs(argv);
  if ('error' in parsed) {
    console.error(`render:check: ${parsed.error}\n\n${USAGE}`);
    return 2;
  }
  if (parsed.help) {
    console.log(USAGE);
    return 0;
  }
  const opts: CheckOptions = { ...parsed, outDir: resolve(parsed.outDir) };

  const loaded = loadCorpus();
  if ('error' in loaded) {
    for (const line of loaded.error) console.error(`render:check: ${line}`);
    return 2;
  }
  const { corpus, io } = loaded;
  const selected = selectFixtures(corpus, opts.filters);
  if ('error' in selected) {
    console.error(`render:check: ${selected.error}`);
    return 2;
  }
  const problems = validateMutations(corpus);
  if (problems.length > 0) {
    for (const p of problems) console.error(`render:check: ${p}`);
    return 2;
  }

  // The host planner (and the batch, which plans with it) never loads the render page.
  const usesPage = !(opts.engine === 'rust' && plansWithHost(opts.planner));
  if (usesPage && !ensureBuilt(opts.rebuild)) return 2;

  let created: Awaited<ReturnType<typeof createRenderer>>;
  try {
    created = await createRenderer(opts.engine, opts);
  } catch (e) {
    if (e instanceof HarnessError) {
      console.error(`render:check: ${e.code}: ${e.message}`);
      return 2;
    }
    throw e;
  }
  const { renderer, rustVersion, plannerVersion } = created;
  let batchWindow: CheckDeps['batchWindow'];
  let batchVersion: string | null = null;
  try {
    if (opts.planner === 'batch') {
      const made = await createBatchWindow(opts.outDir);
      batchWindow = made.batchWindow;
      batchVersion = made.version;
    }
    // The host planner opens no browser: the report's chromium field reads `none`, and the
    // planner's version is on the run header line only.
    const engineVersion = plannerVersion === null ? await renderer.version() : 'none';
    console.log(
      `render:check: ${selected.length} fixture(s), concurrency ${opts.concurrency}, ` +
        (plannerVersion === null ? `chromium ${engineVersion}` : `planner ${plannerVersion}`) +
        (rustVersion === null ? '' : `, engine ${rustVersion}`) +
        (batchVersion === null ? '' : `, render ${batchVersion}`),
    );
    const run = await runCheck(
      opts,
      {
        renderer,
        engineVersion,
        rustVersion,
        playwright: playwrightPin(),
        corpus,
        readSong: (f) => io.readSong(f) ?? '',
        fs: nodeFs,
        clock: { now: () => performance.now(), iso: () => new Date().toISOString() },
        goldensDir: GOLDENS_DIR,
        log: (line) => console.log(line),
        batchWindow,
      },
      selected,
    );
    console.log('');
    process.stdout.write(formatReport(run.report));
    console.log(`report: ${join(opts.outDir, 'report.json')}`);
    return run.exitCode;
  } catch (e) {
    if (e instanceof HarnessError) {
      console.error(`render:check: ${e.code}: ${e.message}`);
      return 2;
    }
    throw e;
  } finally {
    await renderer.dispose().catch(() => {});
  }
}

if (import.meta.main) {
  process.exit(await main(Bun.argv.slice(2)));
}

/**
 * `bun run engine:refs [--write] [name...]`: the Chromium references the Rust engine is measured
 * against (`crates/strudeland-engine/tests/refs/`).
 *
 * - Micro cases (`tests/render/engine-micro/<name>.js`): all bars as one chunk, `lookbackSec 0`,
 *   linear, mix. The job comes from the render page (`RustRenderer.jobFor`, unmutated) and the
 *   audio from superdough in Chromium (`SuperdoughRenderer.render`, not `renderOk`: a case that
 *   expects a render error still returns its audio); they become `<name>.job.json` (with
 *   `provenance`) and `<name>.wav` (f32 stereo).
 * - Corpus fixtures with refs (`fixtureHasRefs`): for every chunk `render:check` plans
 *   (`prepareFixture`'s `fx.plans`), the mix job and the superdough reference envelope, plus the
 *   stems variants when the fixture has the stems gate; no WAV.
 * - The V8 math reference (case `v8-math`): the grid of `v8-math-cases.ts` evaluated twice, in
 *   two fresh pages (page kind `math`); the two must be identical. It becomes `v8-math.json`.
 * - Floor cases (`floor` in `thresholds.json`, one of `FLOOR_CASES`): 8 renders, settled against
 *   the committed WAV and `floorDb` by `settleFloorCase`.
 * - `index.json`: every file's sha256 and size and every case's provenance.
 *
 * Budgets are checked on the final set before anything is written. Without `--write` it prints
 * the diff summary only. Imports Playwright (through the renderers): never import it from a test;
 * the pure parts are in `refs-lib.ts`.
 */

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { chunkRequest } from '../../src/lib/render/chunks';
import {
  SAMPLE_RATE,
  type Renderer,
  type RenderRequest,
  type RenderResult,
} from '../../src/lib/render/contract';
import type { FixtureEntry } from '../../src/lib/render/corpus';
import type { EngineJob } from '../../src/lib/render/engine-job';
import { decodeWav, encodeWav } from '../../src/lib/render/wav';
import { errorMessage } from '../../src/lib/errors';
import { ensureBuilt, loadCorpus, prepareFixture, stemsRequest } from '../render/check';
import type { RustRenderer } from '../render/rust-renderer';
import type { SuperdoughRenderer } from '../render/superdough-renderer';
import { formatRenderError } from '../render/transfer';
import { fixtureCase, mathProvenanceOf, MICRO_DIR, provenanceOf } from './provenance';
import { v8MathCases } from './v8-math-cases';
import {
  budgetProblems,
  diffRefs,
  envelopeFileText,
  fixtureEnvelopeFile,
  fixtureJobFile,
  formatDiff,
  formatTotals,
  hasChanges,
  INDEX_FILE,
  indexText,
  jobFileText,
  mergeRefs,
  microJobFile,
  microWavFile,
  parseRefsArgs,
  referenceEnvelope,
  REFS_DIR,
  REFS_USAGE,
  selectCases,
  FLOOR_CASES,
  FLOOR_RUNS,
  MATH_CASE,
  MEASURE_CASES,
  MATH_FILE,
  mathFileText,
  mathGroups,
  mathOutputsDiffer,
  partialRunRefusal,
  settleFloorCase,
  THRESHOLDS_FILE,
  totalsOf,
  utf8,
  type GeneratedCase,
  type IndexCase,
  type RefsIndex,
} from './refs-lib';

const ROOT = resolve(import.meta.dir, '../..');
const REFS = join(ROOT, REFS_DIR);
/** A measure case's evidence and the directory it is written to. */
interface MeasureEvidence {
  dir: string;
  body: { agrees: boolean };
}

class CaseError extends Error {}

interface ExpectError {
  bar: number;
  engineKind: string;
}

function thresholdOf(name: string): { expectErrors?: ExpectError[]; floor?: unknown } {
  const path = join(REFS, THRESHOLDS_FILE);
  if (!existsSync(path)) return {};
  const t = JSON.parse(readFileSync(path, 'utf8'));
  return t.cases?.[name] ?? {};
}

const expectedErrors = (name: string): ExpectError[] => thresholdOf(name).expectErrors ?? [];

/** The committed refs a run starts from. */
interface Committed {
  files: Map<string, Uint8Array>;
  index: RefsIndex | null;
}

/** The micro request: all bars of the case's structure as one chunk. */
export async function microRequest(
  sd: Pick<Renderer, 'schedule'>,
  code: string,
): Promise<RenderRequest> {
  const probe: RenderRequest = {
    code,
    layout: 'linear',
    fromBar: 0,
    toBar: 1,
    overlapBars: 0,
    lookbackSec: 0,
    mode: 'mix',
    sampleRate: SAMPLE_RATE,
  };
  const sched = await sd.schedule(probe);
  if (sched.errors.length > 0) {
    throw new CaseError(sched.errors.map(formatRenderError).join('; '));
  }
  const bars = sched.structure.totalBars;
  if (!Number.isInteger(bars) || bars < 1) {
    throw new CaseError(`meta has ${bars} bars; a micro case needs sections with bars >= 1`);
  }
  return { ...probe, toBar: bars };
}

function checkRender(result: RenderResult, job: EngineJob, expected: ExpectError[]): void {
  if (result.frames !== job.frames) {
    throw new CaseError(`superdough rendered ${result.frames} frames, the job has ${job.frames}`);
  }
  if (result.channels.length !== job.channels.count) {
    throw new CaseError(
      `superdough rendered ${result.channels.length} channels, the job has ${job.channels.count}`,
    );
  }
  if (expected.length === 0) {
    if (result.errors.length > 0) {
      throw new CaseError(`render errors: ${result.errors.map(formatRenderError).join('; ')}`);
    }
    return;
  }
  for (const e of expected) {
    if (!result.errors.some((r) => r.bar === e.bar)) {
      throw new CaseError(`expected a render error at bar ${e.bar}, got none`);
    }
  }
}

async function microCase(
  name: string,
  sd: SuperdoughRenderer,
  rust: RustRenderer,
  evidence: Map<string, MeasureEvidence>,
  committed: Committed,
): Promise<GeneratedCase> {
  const code = readFileSync(join(ROOT, MICRO_DIR, `${name}.js`), 'utf8');
  const req = await microRequest(sd, code);
  const floor = thresholdOf(name).floor !== undefined;
  if (floor && !FLOOR_CASES.includes(name)) {
    throw new CaseError(`declares floor but is not one of FLOOR_CASES (${FLOOR_CASES.join(', ')})`);
  }
  // Each call opens its own page, so the job and the reference render run at once.
  const [{ job }, result] = await Promise.all([rust.jobFor(req), sd.render(req)]);
  checkRender(result, job, expectedErrors(name));
  const provenance = provenanceOf(ROOT, name);
  const jobText = jobFileText(job, provenance);
  const entry: IndexCase = {
    kind: 'micro',
    provenance,
    jobs: [microJobFile(name)],
    envelopes: null,
    wav: microWavFile(name),
    data: null,
  };
  let channels = result.channels;
  let wavBytes: Uint8Array | null = null;
  if (floor) {
    // Renders 2..8, in sequence, each in its own fresh page.
    const renders = [result.channels];
    for (let k = 1; k < FLOOR_RUNS; k++) {
      const r = await sd.render(req);
      checkRender(r, job, expectedErrors(name));
      renders.push(r.channels);
    }
    const c = committed.index?.cases[name];
    const cJob = committed.files.get(microJobFile(name));
    const cWav = committed.files.get(microWavFile(name));
    const prior =
      c && cJob && cWav && typeof c.floorDb === 'number'
        ? { jobBytes: cJob, wav: decodeWav(cWav).channels, floorDb: c.floorDb }
        : null;
    const settled = settleFloorCase(jobText, renders, prior);
    if ('error' in settled) throw new CaseError(settled.error);
    if (settled.status === 'unchanged') {
      wavBytes = cWav!;
      entry.floorDb = settled.floorDb;
      console.log(
        `  ${name}: floor ${settled.floorDb} dB kept (worst ${settled.worstDb.toFixed(2)} dB)`,
      );
    } else {
      channels = settled.wav;
      entry.floorDb = settled.floorDb;
      console.log(`  ${name}: floor ${settled.floorDb} dB (${settled.reason})`);
    }
  }
  const files = new Map<string, Uint8Array>([
    [microJobFile(name), utf8(jobText)],
    [microWavFile(name), wavBytes ?? encodeWav(channels, SAMPLE_RATE, 'f32')],
  ]);
  const measure = MEASURE_CASES.get(name);
  if (measure) {
    const body = { ...measure.evidence(job, result.channels), provenance };
    evidence.set(name, { dir: measure.dir, body });
  }
  return { name, entry, files };
}

/** The V8 math reference: the grid evaluated twice in two fresh pages; they must agree. */
async function mathCase(sd: SuperdoughRenderer): Promise<GeneratedCase> {
  const inputs = v8MathCases();
  const [first, second] = await Promise.all([sd.math(inputs), sd.math(inputs)]);
  const differ = mathOutputsDiffer(first, second);
  if (differ !== null) throw new CaseError(`V8 math is not deterministic: ${differ}`);
  const provenance = mathProvenanceOf(ROOT);
  const text = mathFileText(provenance, mathGroups(inputs, first));
  return {
    name: MATH_CASE,
    entry: { kind: 'math', provenance, jobs: [], envelopes: null, wav: null, data: MATH_FILE },
    files: new Map([[MATH_FILE, utf8(text)]]),
  };
}

async function fixtureCaseRefs(
  fixture: FixtureEntry,
  deps: { sd: SuperdoughRenderer; rust: RustRenderer; readSong: (f: string) => string },
  corpus: Parameters<typeof prepareFixture>[1]['corpus'],
  log: (line: string) => void,
): Promise<GeneratedCase> {
  const prepared = await prepareFixture(
    fixture,
    { renderer: deps.sd, corpus, readSong: deps.readSong },
    log,
  );
  if ('errors' in prepared) throw new CaseError(prepared.errors.join('; '));
  if ('unsupported' in prepared) throw new CaseError('superdough reported unsupported haps');
  const { fx } = prepared;
  const name = fixtureCase(fixture.name);
  const provenance = provenanceOf(ROOT, name);
  const files = new Map<string, Uint8Array>();
  const jobs: string[] = [];
  const envelopes: string[] = [];
  const modes: ('mix' | 'stems')[] = fixture.gates.includes('stems') ? ['mix', 'stems'] : ['mix'];
  for (const mode of modes) {
    for (const [k, plan] of fx.plans.entries()) {
      const req = mode === 'mix' ? chunkRequest(fx.base, plan) : stemsRequest(fx.base, plan);
      const [{ job }, result] = await Promise.all([deps.rust.jobFor(req), deps.sd.render(req)]);
      checkRender(result, job, []);
      const jobFile = fixtureJobFile(fixture.name, mode, k);
      const envFile = fixtureEnvelopeFile(fixture.name, mode, k);
      files.set(jobFile, utf8(jobFileText(job, provenance)));
      files.set(envFile, utf8(envelopeFileText(referenceEnvelope(result.channels))));
      jobs.push(jobFile);
      envelopes.push(envFile);
    }
  }
  return {
    name,
    entry: { kind: 'fixture', provenance, jobs, envelopes, wav: null, data: null },
    files,
  };
}

function readCommitted(): Committed {
  const files = new Map<string, Uint8Array>();
  let index: RefsIndex | null = null;
  if (!existsSync(REFS)) return { files, index };
  for (const f of readdirSync(REFS)) {
    if (f === THRESHOLDS_FILE) continue;
    const bytes = new Uint8Array(readFileSync(join(REFS, f)));
    if (f === INDEX_FILE) index = JSON.parse(new TextDecoder().decode(bytes));
    else files.set(f, bytes);
  }
  return { files, index };
}

async function main(argv: string[]): Promise<number> {
  const parsed = parseRefsArgs(argv);
  if ('error' in parsed) {
    console.error(`engine:refs: ${parsed.error}\n\n${REFS_USAGE}`);
    return 2;
  }
  if (parsed.help) {
    console.log(REFS_USAGE);
    return 0;
  }
  const loaded = loadCorpus();
  if ('error' in loaded) {
    for (const line of loaded.error) console.error(`engine:refs: ${line}`);
    return 2;
  }
  const microNames = readdirSync(join(ROOT, MICRO_DIR))
    .filter((f) => f.endsWith('.js'))
    .map((f) => f.slice(0, -3));
  const selection = selectCases(parsed.names, microNames, loaded.corpus.fixtures);
  if ('error' in selection) {
    console.error(`engine:refs: ${selection.error}`);
    return 2;
  }
  const committed = readCommitted();
  const full = parsed.names.length === 0;
  const refusal = partialRunRefusal(committed.index, full, parsed.write);
  if (refusal !== null) {
    console.error(`engine:refs: ${refusal}`);
    return 2;
  }
  if (!ensureBuilt(false)) return 2;

  const { SuperdoughRenderer } = await import('../render/superdough-renderer');
  const { RustRenderer } = await import('../render/rust-renderer');
  const sd = new SuperdoughRenderer();
  // Only jobFor is used: the engine binary never runs here.
  const rust = new RustRenderer({ bin: '', version: '', outDir: '', keepWav: false, jobs: sd });
  const generated: GeneratedCase[] = [];
  const failed: string[] = [];
  const evidence = new Map<string, MeasureEvidence>();
  const log = (line: string) => console.log(line);
  try {
    console.log(`engine:refs: chromium ${await sd.version()}`);
    for (const name of selection.micro) {
      try {
        generated.push(await microCase(name, sd, rust, evidence, committed));
        console.log(`  ${name}: ok`);
      } catch (e) {
        failed.push(`${name}: ${errorMessage(e)}`);
        if (!(e instanceof CaseError)) console.error(e);
      }
    }
    if (selection.math) {
      try {
        generated.push(await mathCase(sd));
        console.log(`  ${MATH_CASE}: ok`);
      } catch (e) {
        failed.push(`${MATH_CASE}: ${errorMessage(e)}`);
        if (!(e instanceof CaseError)) console.error(e);
      }
    }
    const readSong = (f: string) => loaded.io.readSong(f) ?? '';
    for (const fname of selection.fixtures) {
      const fixture = loaded.corpus.fixtures.find((f) => f.name === fname)!;
      try {
        const g = await fixtureCaseRefs(fixture, { sd, rust, readSong }, loaded.corpus, log);
        generated.push(g);
        console.log(`  ${g.name}: ${g.entry.jobs.length} chunk job(s)`);
      } catch (e) {
        failed.push(`${fname}: ${errorMessage(e)}`);
        if (!(e instanceof CaseError)) console.error(e);
      }
    }
  } finally {
    await rust.dispose().catch(() => {});
  }
  if (failed.length > 0) {
    for (const f of failed) console.error(`engine:refs: case failed: ${f}`);
    console.error('engine:refs: nothing written');
    return 1;
  }

  const next = mergeRefs(committed.files, committed.index, generated, full);
  const wavFrames = new Map<string, number>();
  for (const [name, c] of Object.entries(next.index.cases)) {
    if (c.wav) wavFrames.set(name, decodeWav(next.files.get(c.wav)!).channels[0]!.length);
  }
  const before = new Map(committed.files);
  if (committed.index) before.set(INDEX_FILE, utf8(indexText(committed.index)));
  const after = new Map(next.files);
  after.set(INDEX_FILE, utf8(indexText(next.index)));
  const diff = diffRefs(before, after);
  console.log(`engine:refs: ${formatTotals(totalsOf(next.files))}`);
  for (const line of formatDiff(diff, before, after)) console.log(line);
  for (const [name, e] of evidence) {
    console.log(`engine:refs: ${name}: agrees with its rule: ${e.body.agrees}`);
  }

  const problems = budgetProblems(next.files, wavFrames);
  if (problems.length > 0) {
    for (const p of problems) console.error(`engine:refs: over budget: ${p}`);
    console.error('engine:refs: nothing written');
    return 1;
  }
  if (!parsed.write) {
    if (!hasChanges(diff)) {
      console.log('engine:refs: refs are current');
      return 0;
    }
    console.log('engine:refs: refs differ; run with --write to update them');
    return 1;
  }
  mkdirSync(REFS, { recursive: true });
  for (const name of diff.removed) rmSync(join(REFS, name));
  for (const name of [...diff.added, ...diff.changed])
    writeFileSync(join(REFS, name), after.get(name)!);
  for (const [name, e] of evidence) {
    const dir = join(ROOT, e.dir);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `${name}.json`), `${JSON.stringify(e.body, null, 2)}\n`);
    console.log(`engine:refs: wrote ${e.dir}/${name}.json`);
  }
  console.log(`engine:refs: wrote ${diff.added.length + diff.changed.length} file(s)`);
  return 0;
}

if (import.meta.main) {
  process.exit(await main(Bun.argv.slice(2)));
}

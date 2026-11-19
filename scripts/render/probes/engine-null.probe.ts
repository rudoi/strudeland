/**
 * Engine null: every `rust: "supported"` corpus fixture and every micro case rendered with both
 * engines (superdough in Chromium, the `strudeland-engine` binary on the page's job). Writes
 * `plans/0011-engine-effects/evidence/engine-null.json` (`PROBE_EVIDENCE_DIR`, `PROBE_LABEL` as
 * the other probes) with, per case:
 *
 * - the null residual of the Rust render against the superdough render of the same request
 *   (`nullResidualDb`, every sample of every channel, no alignment search: both buffers start at
 *   frame 0 of the same plan), per chunk for fixtures, and the worst 100 ms window;
 * - fixtures: the parity deltas against the superdough golden (`compareFeatures` with
 *   `PROFILES.engine`, as `render:check --engine rust` computes them), the deltas against the
 *   superdough render of this run, the golden's own drift (superdough render against the golden),
 *   the superdough twin floor (two superdough renders of chunk 0) and the gate verdicts on the
 *   Rust renders (unchanged `thresholds.ts` constants);
 * - micro cases: the `compareFeatures` deltas, the committed `maxResidualDb` and the 05 section 5
 *   target (-60 dB, delay and reverb -40 dB).
 * - fixtures also carry `timing` (`rustRenderMs`, `superdoughRenderMs`: each chunk's
 *   `timing.renderMs`), and `decision.renderTimesSec` maps each fixture to the sum of its Rust
 *   chunk times in seconds.
 *
 * The decision (`decision.engine` = `PROFILES.engine`, `engineDecision`) and the checks that must
 * hold (gates pass, targets and thresholds met) close the file.
 *
 *   bun scripts/render/probes/engine-null.probe.ts [fixture | micro case …]
 *
 * Builds the engine (`cargo build --release`) unless `STRUDELAND_ENGINE_BIN` is set. Imports
 * Playwright (through the renderers): run by hand, never from a test.
 */

import { execSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chunkRequest } from '../../../src/lib/render/chunks';
import { SAMPLE_RATE as SR, type RenderResult } from '../../../src/lib/render/contract';
import type { FixtureEntry } from '../../../src/lib/render/corpus';
import { extractFeatures, parseFeatures, type Features } from '../../../src/lib/render/features';
import { twinMetrics } from '../../../src/lib/render/gates/twin';
import type { GateResult } from '../../../src/lib/render/gates/types';
import {
  compareFeatures,
  PROFILES,
  type ParityProfile,
  type ParityResult,
} from '../../../src/lib/render/parity';
import { errorMessage } from '../../../src/lib/errors';
import { microRequest } from '../../engine/refs';
import { MICRO_DIR } from '../../engine/provenance';
import { INDEX_FILE, REFS_DIR, THRESHOLDS_FILE } from '../../engine/refs-lib';
import {
  applyExpectations,
  assembledOf,
  chunkResults,
  ensureBuilt,
  Limiter,
  loadCorpus,
  playwrightPin,
  pool,
  prepareFixture,
  renderIn,
  runGate,
  type FixtureContext,
} from '../check';
import { bunRunner, findEngineBinary } from '../rust-engine';
import {
  dbJson,
  engineDecision,
  fixtureTiming,
  microTargetDb,
  microVerdict,
  nullResidualDb,
  platformName,
  renderTimesSec,
  worstWindow,
  type EngineFixtureStats,
  type FixtureTiming,
  type WorstWindow,
} from './calibrate';
import { ENGINE_EFFECTS_EVIDENCE_DIR as EVIDENCE_DIR } from './evidence-path';
import { CONCURRENCY, ROOT, writeEvidence } from './probe-env';

const WINDOW_SEC = 0.1;
const FLOOR_DB = -60;

const windowJson = (w: WorstWindow | null) =>
  w === null
    ? null
    : {
        startSec: Math.round(w.startSec * 1000) / 1000,
        residualDb: dbJson(w.residualDb),
        diffDbfs: dbJson(w.diffDbfs),
        refDbfs: dbJson(w.refDbfs),
      };

/** The deltas of a parity result, without its prose. */
const deltas = (p: ParityResult) => ({
  verdict: p.verdict,
  frameDelta: p.frameDelta,
  offsetSec: p.offsetSec,
  aligned: p.aligned,
  peakDeltaDb: p.peakDeltaDb,
  lufsDeltaDb: p.lufsDeltaDb,
  bandMeanDeltaDb: p.bandMeanDeltaDb,
  bandP95DeltaDb: p.bandP95DeltaDb,
  hashEqual: p.hashEqual,
  reasons: p.reasons,
});

function features(r: RenderResult): Features | string {
  try {
    return extractFeatures(r);
  } catch (e) {
    return `features: ${errorMessage(e)}`;
  }
}

function compare(a: Features | string, b: Features | string, p: ParityProfile) {
  if (typeof a === 'string') return a;
  if (typeof b === 'string') return b;
  return deltas(compareFeatures(a, b, p));
}

const argv = Bun.argv.slice(2).filter((a) => !a.startsWith('-'));
const loaded = loadCorpus();
if ('error' in loaded) throw new Error(loaded.error.join('\n'));
if (!ensureBuilt(false)) throw new Error('render:build failed');
const { corpus, io } = loaded;
const microAll = readdirSync(join(ROOT, MICRO_DIR))
  .filter((f) => f.endsWith('.js'))
  .map((f) => f.slice(0, -3))
  .sort();
const supported = corpus.fixtures.filter((f) => f.rust === 'supported');
for (const n of argv) {
  if (!microAll.includes(n) && !supported.some((f) => f.name === n)) {
    throw new Error(`${n} is neither a micro case nor a rust-supported fixture`);
  }
}
const fixtures = supported.filter((f) => argv.length === 0 || argv.includes(f.name));
const micro = microAll.filter((n) => argv.length === 0 || argv.includes(n));
const thresholds = JSON.parse(readFileSync(join(ROOT, REFS_DIR, THRESHOLDS_FILE), 'utf8'));
const index = JSON.parse(readFileSync(join(ROOT, REFS_DIR, INDEX_FILE), 'utf8'));

const { SuperdoughRenderer } = await import('../superdough-renderer');
const { RustRenderer } = await import('../rust-renderer');
const sd = new SuperdoughRenderer();
const outDir = mkdtempSync(join(tmpdir(), 'engine-null-'));
const { path: bin, version } = await findEngineBinary(process.env, bunRunner, ROOT);
const rust = new RustRenderer({ bin, version, outDir, keepWav: false, jobs: sd });
const t0 = performance.now();
const log = (line: string) =>
  console.log(`[${((performance.now() - t0) / 1000).toFixed(0).padStart(5)}s] ${line}`);
let harness = 'unknown';
try {
  harness = execSync('git rev-parse --short HEAD', { cwd: ROOT }).toString().trim();
} catch {
  // not a git checkout
}

try {
  const header = {
    generatedAt: new Date().toISOString(),
    playwright: playwrightPin(),
    chromium: await sd.version(),
    engine: version,
    harness,
  };
  const limit = new Limiter(CONCURRENCY);
  const readSong = (f: string) => io.readSong(f) ?? '';

  // ---- fixtures
  const fixtureRows: Record<string, unknown>[] = [];
  const decisionRows: EngineFixtureStats[] = [];
  const timingRows: { name: string; timing: FixtureTiming }[] = [];
  const gateFailures: string[] = [];
  const prepare = async (f: FixtureEntry, renderer: typeof sd | typeof rust) => {
    const engine = renderer.engine;
    const p = await prepareFixture(f, { renderer, corpus, readSong, limit }, () => {});
    if ('errors' in p) throw new Error(`${f.name} (${engine}): ${p.errors.join('; ')}`);
    if ('unsupported' in p) throw new Error(`${f.name}: unsupported by the ${engine} renderer`);
    return p.fx;
  };
  const failure = await pool(fixtures, CONCURRENCY, async (f, i) => {
    const started = performance.now();
    const [sdFx, rsFx]: FixtureContext[] = await Promise.all([prepare(f, sd), prepare(f, rust)]);
    const twin = renderIn(sdFx, chunkRequest(sdFx.base, sdFx.plans[0]!));
    const [sdChunks, rsChunks] = await Promise.all([chunkResults(sdFx), chunkResults(rsFx)]);
    const [sdWin, rsWin] = await Promise.all([assembledOf(sdFx), assembledOf(rsFx)]);
    const gates: GateResult[] = [];
    for (const g of f.gates) gates.push(applyExpectations(f, await runGate(g, rsFx)));
    for (const g of gates) {
      if (g.verdict === 'fail') gateFailures.push(`${f.name} ${g.gate}: ${g.reasons.join('; ')}`);
    }
    const sdTwin = await twin;
    const timing = fixtureTiming(sdChunks, rsChunks);
    timingRows.push({ name: f.name, timing });
    const tm = twinMetrics(sdChunks[0]!.channels, sdTwin.channels, SR);
    const golden = parseFeatures(
      readFileSync(join(ROOT, 'tests/render/goldens', `${f.name}.json`), 'utf8'),
    );
    const sdFeat = features(sdWin);
    const rsFeat = features(rsWin);
    const twinFeat = features(sdTwin);
    const sd0Feat = features(sdChunks[0]!);
    if (typeof rsFeat === 'string') throw new Error(`${f.name}: ${rsFeat}`);
    const vsGolden = compareFeatures(golden, rsFeat, PROFILES.engine);
    const chunks = sdChunks.map((s, k) => {
      const r = rsChunks[k]!;
      return {
        index: k,
        frames: s.frames,
        residualDb: dbJson(nullResidualDb(s.channels, r.channels)),
        worstWindow: windowJson(worstWindow(s.channels, r.channels, SR, WINDOW_SEC, FLOOR_DB)),
      };
    });
    const twinBandP95 = Math.max(0, ...Object.values(tm.bands).map((b) => b.p95));
    const twinPeak =
      typeof sd0Feat === 'string' || typeof twinFeat === 'string'
        ? 0
        : Math.abs(sd0Feat.peakDb - twinFeat.peakDb);
    const twinLufs =
      typeof sd0Feat === 'string' ||
      typeof twinFeat === 'string' ||
      sd0Feat.lufs === null ||
      twinFeat.lufs === null
        ? 0
        : Math.abs(sd0Feat.lufs - twinFeat.lufs);
    decisionRows.push({
      name: f.name,
      parity: {
        frameDelta: vsGolden.frameDelta,
        offsetSec: vsGolden.offsetSec,
        peakDeltaDb: vsGolden.peakDeltaDb,
        lufsDeltaDb: vsGolden.lufsDeltaDb,
        bandMeanDeltaDb: vsGolden.bandMeanDeltaDb,
        bandP95DeltaDb: vsGolden.bandP95DeltaDb,
      },
      twin: { peakDeltaDb: twinPeak, lufsDelta: twinLufs, bandP95: twinBandP95 },
    });
    fixtureRows[i] = {
      name: f.name,
      kind: f.kind,
      stochastic: f.stochastic,
      windowBars: f.windowBars,
      chunks,
      residualDb: dbJson(
        Math.max(
          ...sdChunks.map((s, k) => nullResidualDb(s.channels, rsChunks[k]!.channels) ?? Infinity),
        ),
      ),
      hashEqual: typeof sdFeat !== 'string' && sdFeat.pcmSha256 === rsFeat.pcmSha256,
      parityVsGolden: deltas(vsGolden),
      parityVsSuperdough: compare(sdFeat, rsFeat, PROFILES.engine),
      goldenDrift: compare(golden, sdFeat, PROFILES.self),
      twinFloor: {
        bitEqual: tm.differingFrames === 0,
        differingFrames: tm.differingFrames,
        residualDb: dbJson(tm.residualDb === -200 ? -Infinity : tm.residualDb),
        bands: tm.bands,
        peakDeltaDb: twinPeak,
        lufsDelta: twinLufs,
      },
      gates: gates.map((g) => ({ gate: g.gate, verdict: g.verdict, reasons: g.reasons })),
      timing,
    };
    sdFx.memo.clear();
    rsFx.memo.clear();
    log(
      `${f.name}: residual ${fixtureRows[i]!.residualDb} dB, parity ${vsGolden.verdict}, ` +
        `twin floor ${dbJson(tm.residualDb)} dB, gates ${gates.map((g) => `${g.gate}:${g.verdict}`).join(' ')} ` +
        `(${((performance.now() - started) / 1000).toFixed(1)}s)`,
    );
  });
  if (failure !== null) throw failure;

  // ---- micro cases
  const microRows: Record<string, unknown>[] = [];
  const microFailure = await pool(micro, CONCURRENCY, async (name, i) => {
    const code = readFileSync(join(ROOT, MICRO_DIR, `${name}.js`), 'utf8');
    const req = await microRequest(sd, code);
    const [s, r] = await Promise.all([
      limit.run(() => sd.render(req)),
      limit.run(() => rust.render(req)),
    ]);
    const residual = nullResidualDb(s.channels, r.channels);
    const max = thresholds.cases?.[name]?.maxResidualDb as number | undefined;
    const target = microTargetDb(name);
    microRows[i] = {
      name,
      frames: s.frames,
      residualDb: dbJson(residual),
      worstWindow: windowJson(worstWindow(s.channels, r.channels, SR, WINDOW_SEC, FLOOR_DB)),
      parity: compare(features(s), features(r), PROFILES.engine),
      superdoughErrors: s.errors.map((e) => `${e.kind}@${e.bar}`),
      engineErrors: r.errors.map((e) => `${e.kind}@${e.bar}`),
      targetDb: target,
      meetsTarget: residual !== null && residual <= target,
      maxResidualDb: max ?? null,
      ...microVerdict({
        residual,
        maxResidualDb: max,
        platform: thresholds.cases?.[name]?.platform as string | undefined,
        host: platformName(process.platform),
        floorDb: index.cases?.[name]?.floorDb as number | undefined,
      }),
    };
    log(`${name}: residual ${dbJson(residual)} dB (max ${max} dB, target ${target} dB)`);
  });
  if (microFailure !== null) throw microFailure;

  const decision =
    decisionRows.length > 0
      ? {
          ...engineDecision([...decisionRows].sort((a, b) => a.name.localeCompare(b.name))),
          renderTimesSec: renderTimesSec(timingRows),
          gatesHold: gateFailures.length === 0,
          gateFailures,
          microTargetsMet: microRows.every((m) => m.meetsTarget),
          microThresholdsMet: microRows.every((m) => m.meetsThreshold !== false),
          missedTargets: microRows.filter((m) => !m.meetsTarget).map((m) => m.name),
          missedThresholds: microRows.filter((m) => m.meetsThreshold === false).map((m) => m.name),
        }
      : null;
  const path = writeEvidence(
    'engine-null',
    {
      ...header,
      windowSec: WINDOW_SEC,
      floorDb: FLOOR_DB,
      residual:
        '10*log10(sum((superdough - rust)^2) / sum(superdough^2)) over every sample of every channel, no alignment search',
      fixtures: fixtureRows,
      micro: microRows,
      decision,
    },
    EVIDENCE_DIR,
  );
  console.log(JSON.stringify(decision, null, 2));
  console.log(`wrote ${path}`);
} finally {
  await rust.dispose().catch(() => {});
  rmSync(outDir, { recursive: true, force: true });
}

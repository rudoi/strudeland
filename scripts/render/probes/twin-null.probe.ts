/**
 * Twin-render null: chunk 0 of every corpus fixture rendered three times, each in its own page.
 * Writes evidence/twin-null.json with each fixture's bit equality, worst residual, worst band
 * envelope quantiles (SEAM_BANDS, 100 ms) and feature deltas, plus the decisions of 04 section 6
 * probe 1 (TWIN_BITEXACT_HOLDS, TWIN_P95_DB, TWIN_MAX_DB, TWIN_RESIDUAL_DB, PROFILES.self).
 *
 *   bun scripts/render/probes/twin-null.probe.ts [fixture…]
 */

import { chunkRequest } from '../../../src/lib/render/chunks';
import { SAMPLE_RATE as SR, type RenderResult } from '../../../src/lib/render/contract';
import { extractFeatures } from '../../../src/lib/render/features';
import { twinMetrics } from '../../../src/lib/render/gates/twin';
import { pool, renderOk } from '../check';
import { twinDecision, type BandStats, type TwinFixtureStats } from './calibrate';
import { CONCURRENCY, openProbe, writeEvidence } from './probe-env';

const RENDERS = 3;
const PAIRS: [number, number][] = [
  [0, 1],
  [0, 2],
  [1, 2],
];

const env = await openProbe(Bun.argv.slice(2));
try {
  const fixtures = env.fixtures();
  const jobs = fixtures.flatMap((f) => Array.from({ length: RENDERS }, (_, k) => ({ f, k })));
  const renders = new Map<string, RenderResult[]>();
  const ctxs = new Map<string, Awaited<ReturnType<typeof env.prepare>>>();
  for (const f of fixtures) ctxs.set(f.name, await env.prepare(f));
  const failure = await pool(jobs, CONCURRENCY, async ({ f, k }) => {
    const fx = ctxs.get(f.name)!;
    const t0 = performance.now();
    const r = await renderOk(env.renderer, chunkRequest(fx.base, fx.plans[0]!));
    const list = renders.get(f.name) ?? [];
    list[k] = r;
    renders.set(f.name, list);
    env.log(
      `${f.name} render ${k + 1}/${RENDERS} ${((performance.now() - t0) / 1000).toFixed(1)}s`,
    );
  });
  if (failure !== null) throw failure;

  const stats: (TwinFixtureStats & Record<string, unknown>)[] = [];
  for (const f of fixtures) {
    const rs = renders.get(f.name)!;
    const feats = rs.map((r) => extractFeatures(r));
    const pairs = PAIRS.map(([i, j]) => {
      const m = twinMetrics(rs[i]!.channels, rs[j]!.channels, SR);
      return {
        pair: [i, j],
        differingFrames: m.differingFrames,
        firstDiff: m.firstDiff,
        residualDb: m.residualDb,
        bands: m.bands,
        lufsDelta:
          feats[i]!.lufs === null || feats[j]!.lufs === null
            ? 0
            : Math.abs(feats[i]!.lufs! - feats[j]!.lufs!),
        peakDeltaDb: Math.abs(feats[i]!.peakDb - feats[j]!.peakDb),
        hashEqual: feats[i]!.pcmSha256 === feats[j]!.pcmSha256,
      };
    });
    const bands: Record<string, BandStats> = {};
    for (const name of Object.keys(pairs[0]!.bands)) {
      bands[name] = {
        p50: Math.max(...pairs.map((p) => p.bands[name]!.p50)),
        p95: Math.max(...pairs.map((p) => p.bands[name]!.p95)),
        max: Math.max(...pairs.map((p) => p.bands[name]!.max)),
      };
    }
    const fx = ctxs.get(f.name)!;
    stats.push({
      name: f.name,
      stochastic: f.stochastic,
      chunk0Bars: [fx.plans[0]!.winBeginBar, fx.plans[0]!.contentEndBar + fx.plans[0]!.overlapBars],
      frames: rs[0]!.frames,
      renderSec: rs.map((r) => Math.round(r.timing.totalMs / 100) / 10),
      bitEqual: pairs.every((p) => p.differingFrames === 0),
      residualDb: Math.max(...pairs.map((p) => p.residualDb)),
      bands,
      lufsDelta: Math.max(...pairs.map((p) => p.lufsDelta)),
      peakDeltaDb: Math.max(...pairs.map((p) => p.peakDeltaDb)),
      pairs: pairs.map(({ bands: _b, ...rest }) => rest),
    });
    env.log(
      `${f.name}: bitEqual ${stats[stats.length - 1]!.bitEqual}, residual ` +
        `${stats[stats.length - 1]!.residualDb.toFixed(2)} dB`,
    );
  }

  const decision = twinDecision(stats);
  const path = writeEvidence('twin-null', {
    ...env.header,
    renders: RENDERS,
    windowSec: 0.1,
    bands: ['sub', 'mid', 'high'],
    fixtures: stats,
    decision,
  });
  console.log(JSON.stringify(decision, null, 2));
  console.log(`wrote ${path}`);
} finally {
  await env.close();
}

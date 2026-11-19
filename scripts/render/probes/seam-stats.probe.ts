/**
 * Seam statistics: every fixture with the seam gate rendered with its real chunk plan. Per seam,
 * the one-sided head-quieter deficit of every SEAM_BANDS window and the twin floor (the head
 * rendered again, two-way worst deficit). Writes evidence/seam-stats.json with per-fixture and
 * pooled p95, p99 and max per band, and the SEAM_TOLERANCE_DB decision of 04 section 6 probe 2.
 *
 *   bun scripts/render/probes/seam-stats.probe.ts [fixture…]
 */

import { chunkRequest, headOf, overlapOf } from '../../../src/lib/render/chunks';
import { SAMPLE_RATE as SR } from '../../../src/lib/render/contract';
import { seamConsistency, twinFloorDb } from '../../../src/lib/render/gates/seam';
import { roundSec } from '../../../src/lib/render/gates/types';
import { SEAM_BANDS, SEAM_TOLERANCE_DB } from '../../../src/lib/render/thresholds';
import { chunkResults, pool, renderIn } from '../check';
import { ceilTo, quantiles, type Quantiles } from './calibrate';
import { CONCURRENCY, openProbe, writeEvidence } from './probe-env';

const env = await openProbe(Bun.argv.slice(2));

interface SeamRecord {
  seam: number;
  seamSec: number;
  windows: number;
  worstDeficitDb: number;
  deficitWindowsOverTolerance: number;
  bandWorstDb: Record<string, number>;
  twinFloorDb: number;
}

try {
  const fixtures = env.fixtures('seam');
  const perFixture: Record<string, { seams: SeamRecord[]; bands: Record<string, Quantiles> }> = {};
  const pooledValues: Record<string, number[]> = Object.fromEntries(
    SEAM_BANDS.map(([b]) => [b, []]),
  );
  const failure = await pool(fixtures, CONCURRENCY, async (f) => {
    const fx = await env.prepare(f);
    const results = await chunkResults(fx);
    const seams: SeamRecord[] = [];
    const values: Record<string, number[]> = Object.fromEntries(SEAM_BANDS.map(([b]) => [b, []]));
    for (let k = 0; k + 1 < fx.plans.length; k++) {
      const bars = fx.plans[k]!.overlapBars;
      const seamSec = fx.plans[k]!.contentEndBar * fx.barSec;
      const tail = overlapOf(results[k]!);
      const head = headOf(results[k + 1]!, bars);
      const listed = seamConsistency(tail, head, SR, seamSec, { toleranceDb: SEAM_TOLERANCE_DB });
      // Every non-silent window's deficit (toleranceDb -Infinity lists them all).
      const all = seamConsistency(tail, head, SR, seamSec, { toleranceDb: -Infinity });
      const bandWorstDb: Record<string, number> = {};
      for (const [b] of SEAM_BANDS) {
        const ds = all.deficits.filter((d) => d.band === b).map((d) => Math.max(0, d.deficitDb));
        values[b]!.push(...ds);
        bandWorstDb[b] = ds.length > 0 ? Math.max(...ds) : 0;
      }
      const twin = headOf(await renderIn(fx, chunkRequest(fx.base, fx.plans[k + 1]!)), bars);
      const floorDb = twinFloorDb(head, twin, SR, seamSec);
      seams.push({
        seam: k,
        seamSec: roundSec(seamSec),
        windows: listed.windows,
        worstDeficitDb: listed.worstDeficitDb,
        deficitWindowsOverTolerance: listed.deficits.length,
        bandWorstDb,
        twinFloorDb: floorDb,
      });
      env.log(
        `${f.name} seam ${k} @ ${seamSec.toFixed(2)}s: worst ${listed.worstDeficitDb} dB, ` +
          `twin floor ${floorDb} dB`,
      );
    }
    for (const [b] of SEAM_BANDS) pooledValues[b]!.push(...values[b]!);
    perFixture[f.name] = {
      seams,
      bands: Object.fromEntries(SEAM_BANDS.map(([b]) => [b, quantiles(values[b]!)])),
    };
  });
  if (failure !== null) throw failure;

  const pooled = Object.fromEntries(SEAM_BANDS.map(([b]) => [b, quantiles(pooledValues[b]!)]));
  const pooledMax = Math.max(0, ...Object.values(pooled).map((q) => q.max));
  const decision = {
    pooledMaxDb: pooledMax,
    SEAM_TOLERANCE_DB: ceilTo(1.25 * pooledMax, 0.05),
    previous: SEAM_TOLERANCE_DB,
  };
  const ordered = Object.fromEntries(
    fixtures.filter((f) => perFixture[f.name]).map((f) => [f.name, perFixture[f.name]]),
  );
  const path = writeEvidence('seam-stats', {
    ...env.header,
    windowSec: 0.1,
    bands: SEAM_BANDS.map(([b]) => b),
    note: 'deficit = 20 log10(tail / head) per band window, clamped at 0 (one-sided); 2 decimals',
    fixtures: ordered,
    pooled,
    decision,
  });
  console.log(JSON.stringify({ pooled, decision }, null, 2));
  console.log(`wrote ${path}`);
} finally {
  await env.close();
}

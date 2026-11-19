/**
 * Stem twin bisect (diagnosis aid): renders chunk 0 of one fixture N times in stems mode and prints,
 * per stereo stem, how many renders differ from render 0, the worst residual and the first differing
 * time in seconds from the window start (negative inside the lookback). Prints only; writes no evidence.
 *
 *   bun scripts/render/probes/stem-twin.probe.ts <fixture> [renders]
 *
 * `STEM_FILTER='v.orbit === 8'` keeps only the haps whose value passes (appended as
 * `.filterValues(v => …)` to the song's last `$:` pattern before its final `.compressor(`, or to the
 * end of the code).
 */

import { chunkRequest } from '../../../src/lib/render/chunks';
import { SAMPLE_RATE as SR, type RenderResult } from '../../../src/lib/render/contract';
import { twinMetrics } from '../../../src/lib/render/gates/twin';
import { pool, renderOk } from '../check';
import { CONCURRENCY, openProbe } from './probe-env';

const [name, n] = Bun.argv.slice(2);
if (!name) throw new Error('usage: stem-twin.probe.ts <fixture> [renders]');
const RENDERS = Number(n ?? 6);
if (!Number.isInteger(RENDERS) || RENDERS < 2)
  throw new Error(`renders must be an integer >= 2: ${n}`);
const env = await openProbe([name]);
try {
  const fx = await env.prepare(env.fixtures()[0]!);
  const req = { ...chunkRequest(fx.base, fx.plans[0]!), mode: 'stems' as const };
  const filter = process.env.STEM_FILTER;
  if (filter) {
    const at = req.code.lastIndexOf(').compressor(');
    const f = `.filterValues((v) => ${filter})`;
    req.code = at >= 0 ? req.code.slice(0, at + 1) + f + req.code.slice(at + 1) : req.code + f;
  }
  const rs: RenderResult[] = [];
  const failure = await pool([...Array(RENDERS).keys()], CONCURRENCY, async (k) => {
    rs[k] = await renderOk(env.renderer, req);
  });
  if (failure !== null) throw failure;
  for (let s = 0; s < rs[0]!.stemPairs; s++) {
    const pick = (r: RenderResult) => r.channels.slice(2 * s, 2 * s + 2);
    let differ = 0;
    let worst = -Infinity;
    let first: number | null = null;
    for (let k = 1; k < RENDERS; k++) {
      const m = twinMetrics(pick(rs[0]!), pick(rs[k]!), SR);
      if (m.differingFrames === 0) continue;
      differ++;
      worst = Math.max(worst, m.residualDb);
      const t = (m.firstDiff!.frame - rs[0]!.windowStartFrame) / SR;
      first = first === null ? t : Math.min(first, t);
    }
    console.log(
      `stem ${s}: ${differ}/${RENDERS - 1} differ` +
        (differ ? `, worst ${worst.toFixed(1)} dB, first at ${first!.toFixed(3)} s` : ''),
    );
  }
} finally {
  await env.close();
}

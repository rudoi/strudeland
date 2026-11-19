/**
 * Bus modulator twin: one `bmod` song rendered 6 times with superdough, all 15 pairs compared
 * bit for bit. `unpatched` records the floor before the `bmod-gate` vendored patch (its
 * `webAudioTimeout` connection lands a run-dependent number of quanta late); `patched` is the
 * acceptance of the patch and exits 1 when any pair differs. Writes
 * plans/0011-engine-effects/evidence/bmod-<variant>-twin.json.
 *
 *   bun scripts/render/probes/bmod-twin.probe.ts unpatched|patched
 */

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { RenderRequest, RenderResult } from '../../../src/lib/render/contract';
import { SAMPLE_RATE as SR } from '../../../src/lib/render/contract';
import { twinMetrics } from '../../../src/lib/render/gates/twin';
import { pool, renderOk } from '../check';
import { ENGINE_EFFECTS_EVIDENCE_DIR as EVIDENCE_DIR } from './evidence-path';
import { CONCURRENCY, ROOT, openProbe, writeEvidence } from './probe-env';

const RENDERS = 6;
const MODULATORS = resolve(ROOT, 'vendor/strudel/packages/superdough/modulators.mjs');
const UNPATCHED = /webAudioTimeout\(\s*ac,/;

/** A feeder voice on bus 1 with `dry(0)`, a carrier whose `cutoff` a `bmod` reads from bus 1,
 *  the two on different orbits. */
const SONG = `const meta = { title: 'bmod-twin', bpm: 120, meter: '4/4' };
$: s("sine").note("c2").gain(0.8).bus(1).dry(0).orbit(2)
$: s("sawtooth").note("<c3 g3>").lpf(900).bmod({ b: 1, c: 'cutoff', dep: 2 }).orbit(1)
`;

const variant = Bun.argv[2];
if (variant !== 'unpatched' && variant !== 'patched') {
  console.error('usage: bun scripts/render/probes/bmod-twin.probe.ts unpatched|patched');
  process.exit(2);
}
if (UNPATCHED.test(readFileSync(MODULATORS, 'utf8')) !== (variant === 'unpatched')) {
  console.error(`variant ${variant} does not match modulators.mjs`);
  process.exit(2);
}

const req: RenderRequest = {
  code: SONG,
  layout: 'linear',
  fromBar: 0,
  toBar: 4,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: SR,
};

const env = await openProbe([]);
let differing = 0;
try {
  const renders: RenderResult[] = [];
  const failure = await pool(
    Array.from({ length: RENDERS }, (_, k) => k),
    CONCURRENCY,
    async (k) => {
      const t0 = performance.now();
      renders[k] = await renderOk(env.renderer, req);
      env.log(`render ${k + 1}/${RENDERS} ${((performance.now() - t0) / 1000).toFixed(1)}s`);
    },
  );
  if (failure !== null) throw failure;

  const pairs: {
    pair: [number, number];
    differingFrames: number;
    firstDiff: { frame: number; channel: number } | null;
    residualDb: number | null;
  }[] = [];
  for (let i = 0; i < RENDERS; i++) {
    for (let j = i + 1; j < RENDERS; j++) {
      const m = twinMetrics(renders[i]!.channels, renders[j]!.channels, SR);
      pairs.push({
        pair: [i, j],
        differingFrames: m.differingFrames,
        firstDiff: m.firstDiff,
        // twinMetrics clamps an exact null (-Infinity) to -200: report it as not finite.
        residualDb: m.differingFrames === 0 || !Number.isFinite(m.residualDb) ? null : m.residualDb,
      });
    }
  }
  const diffs = pairs.filter((p) => p.differingFrames > 0);
  differing = diffs.length;
  const worst = diffs
    .map((p) => p.residualDb)
    .filter((x): x is number => x !== null)
    .reduce<number | null>((a, x) => (a === null || x > a ? x : a), null);

  const path = writeEvidence(
    'bmod-' + variant + '-twin',
    {
      header: env.header,
      variant,
      source: SONG,
      renders: RENDERS,
      window: { fromBar: req.fromBar, toBar: req.toBar },
      pairs,
      differingPairs: differing,
      worstResidualDb: worst,
    },
    EVIDENCE_DIR,
  );
  console.log(
    `bmod-twin ${variant}: ${differing}/${pairs.length} pairs differ, worst ` +
      `${worst === null ? 'none' : worst.toFixed(2)} dB`,
  );
  console.log(`wrote ${path}`);
} finally {
  await env.close();
}
if (variant === 'patched' && differing > 0) process.exit(1);

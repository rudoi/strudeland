/**
 * Worklet determinism: renders micro cases 8 times each with superdough in Chromium (in
 * sequence, a fresh page per render, as `engine:refs` does) and records whether the renders are
 * bit-identical. A case that differs is either a stale `currentTime` (whole-quantum shifts of an
 * onset, end or envelope step: redesign the case) or a summing order (the measured-floor rule,
 * only at a junction with three or more sounding inputs). Writes
 * plans/0011-engine-effects/evidence/worklet-determinism.json:
 * `{ header, runs: 8, cases: [{ name, bitEqual, worstResidualDb, firstDiffFrame, hashes }],
 * differing }`.
 *
 *   bun scripts/render/probes/worklet-determinism.probe.ts [case...]
 *
 * Without names: every micro source in tests/render/engine-micro/ (working-tree sources included)
 * whose name has a worklet prefix (`WORKLET_CASE_PREFIXES`).
 */

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { MICRO_DIR } from '../../engine/provenance';
import { determinismOf, FLOOR_RUNS, isWorkletCase } from '../../engine/refs-lib';
import { microRequest } from '../../engine/refs';
import { ENGINE_EFFECTS_EVIDENCE_DIR as EVIDENCE_DIR } from './evidence-path';
import { ROOT, openProbe, writeEvidence } from './probe-env';

const RUNS = FLOOR_RUNS;

const sources = readdirSync(join(ROOT, MICRO_DIR))
  .filter((f) => f.endsWith('.js'))
  .map((f) => f.slice(0, -3))
  .sort();
const names = Bun.argv.slice(2);
for (const n of names) {
  if (!sources.includes(n)) {
    console.error(`unknown micro case ${n}`);
    process.exit(2);
  }
}
const cases = names.length > 0 ? names : sources.filter(isWorkletCase);
if (cases.length === 0) {
  console.error('no worklet micro cases');
  process.exit(2);
}

const env = await openProbe([]);
try {
  const results = [];
  for (const name of cases) {
    const req = await microRequest(
      env.renderer,
      readFileSync(join(ROOT, MICRO_DIR, `${name}.js`), 'utf8'),
    );
    const renders: Float32Array[][] = [];
    for (let k = 0; k < RUNS; k++) {
      const r = await env.renderer.render(req);
      if (r.errors.length > 0) env.log(`${name} run ${k + 1}: ${r.errors.length} render error(s)`);
      renders.push(r.channels);
    }
    const d = determinismOf(name, renders);
    env.log(
      `${name}: ${d.bitEqual ? 'bit-identical' : `differs (worst ${d.worstResidualDb} dB, first frame ${d.firstDiffFrame})`}`,
    );
    results.push(d);
  }
  const differing = results.filter((d) => !d.bitEqual).map((d) => d.name);
  const path = writeEvidence(
    'worklet-determinism',
    { header: env.header, runs: RUNS, cases: results, differing },
    EVIDENCE_DIR,
  );
  console.log(`worklet-determinism: ${differing.length}/${results.length} differ; wrote ${path}`);
} finally {
  await env.close();
}

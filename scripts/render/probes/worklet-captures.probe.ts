/**
 * Re-runs the two worklet measure captures by hand, without touching the refs: renders
 * `measure-worklet-start` and `measure-worklet-teardown` (their sources must be in
 * tests/render/engine-micro/) in Chromium and writes the same evidence `engine:refs --write`
 * writes (`workletStartEvidence`, `workletTeardownEvidence`) to
 * plans/0011-engine-effects/evidence/<case>.json (`PROBE_EVIDENCE_DIR`/`PROBE_LABEL` move it).
 *
 *   bun scripts/render/probes/worklet-captures.probe.ts
 */

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { MICRO_DIR, provenanceOf } from '../../engine/provenance';
import {
  WORKLET_START_CASE,
  WORKLET_TEARDOWN_CASE,
  workletStartEvidence,
  workletTeardownEvidence,
} from '../../engine/refs-lib';
import { microRequest } from '../../engine/refs';
import type { SuperdoughRenderer } from '../superdough-renderer';
import { ENGINE_EFFECTS_EVIDENCE_DIR as EVIDENCE_DIR } from './evidence-path';
import { ROOT, openProbe, writeEvidence } from './probe-env';

const CAPTURES = [
  [WORKLET_START_CASE, workletStartEvidence],
  [WORKLET_TEARDOWN_CASE, workletTeardownEvidence],
] as const;

for (const [name] of CAPTURES) {
  if (!existsSync(join(ROOT, MICRO_DIR, `${name}.js`))) {
    console.error(`${MICRO_DIR}/${name}.js is missing`);
    process.exit(2);
  }
}

const env = await openProbe([]);
const { RustRenderer } = await import('../rust-renderer');
// Only jobFor is used: the engine binary never runs here.
const rust = new RustRenderer({
  bin: '',
  version: '',
  outDir: '',
  keepWav: false,
  jobs: env.renderer as SuperdoughRenderer,
});
let disagree = 0;
try {
  for (const [name, evidenceOf] of CAPTURES) {
    const req = await microRequest(
      env.renderer,
      readFileSync(join(ROOT, MICRO_DIR, `${name}.js`), 'utf8'),
    );
    const [{ job }, result] = await Promise.all([rust.jobFor(req), env.renderer.render(req)]);
    const evidence = { ...evidenceOf(job, result.channels), provenance: provenanceOf(ROOT, name) };
    if (!evidence.agrees) disagree++;
    const path = writeEvidence(name, evidence, EVIDENCE_DIR);
    console.log(`${name}: agrees ${evidence.agrees}; wrote ${path}`);
  }
} finally {
  await rust.dispose().catch(() => {});
  await env.close();
}
if (disagree > 0) process.exit(1);

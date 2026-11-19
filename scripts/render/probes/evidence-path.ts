/**
 * Where a calibration probe writes its evidence. Pure (no Playwright), so it is unit tested.
 * `PROBE_EVIDENCE_DIR` (absolute or relative to the repo root) moves the directory from the
 * probe's default (the render harness plan's evidence unless the probe names another);
 * `PROBE_LABEL` suffixes the file name, so several runs of one probe sit side by side.
 */

import { isAbsolute, join, resolve } from 'node:path';

export const DEFAULT_EVIDENCE_DIR = 'plans/0008-render-harness/evidence';
/** The Rust engine core's evidence (its suspend and feedback measure cases). */
export const ENGINE_CORE_EVIDENCE_DIR = 'plans/0010-rust-engine-core/evidence';
/** The engine effects' evidence (the engine null, worklet and bmod-twin probes, the worklet
 *  measure cases). */
export const ENGINE_EFFECTS_EVIDENCE_DIR = 'plans/0011-engine-effects/evidence';

const LABEL = /^[A-Za-z0-9._-]+$/;

export function evidencePath(
  root: string,
  name: string,
  env: Record<string, string | undefined>,
  defaultDir: string = DEFAULT_EVIDENCE_DIR,
): { dir: string; path: string } {
  const raw = env.PROBE_EVIDENCE_DIR || defaultDir;
  const dir = isAbsolute(raw) ? raw : resolve(root, raw);
  const label = env.PROBE_LABEL;
  if (label && !LABEL.test(label)) {
    throw new Error(`PROBE_LABEL must match ${LABEL}: ${JSON.stringify(label)}`);
  }
  return { dir, path: join(dir, label ? `${name}-${label}.json` : `${name}.json`) };
}

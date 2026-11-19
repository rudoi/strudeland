/**
 * What every calibration probe needs: the validated corpus, the render page built like
 * `render:check`, one Chromium renderer, fixture selection, the evidence header and the evidence
 * writer. Imports Playwright (through the renderer): run by hand, never from a test.
 */

import { execSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import type { Renderer } from '../../../src/lib/render/contract';
import type { Corpus, FixtureEntry } from '../../../src/lib/render/corpus';
import type { GateName } from '../../../src/lib/render/gates/types';
import { DEFAULT_CONCURRENCY } from '../../../src/lib/render/check-options';
import {
  ensureBuilt,
  Limiter,
  loadCorpus,
  playwrightPin,
  prepareFixture,
  type FixtureContext,
} from '../check';
import { evidencePath } from './evidence-path';

export const ROOT = resolve(import.meta.dir, '../../..');

export interface EvidenceHeader {
  generatedAt: string;
  playwright: string;
  chromium: string;
  harness: string;
}

export interface ProbeEnv {
  renderer: Renderer;
  corpus: Corpus;
  header: EvidenceHeader;
  /** Corpus fixtures named in argv (all when none) that carry `gate`, if given. */
  fixtures(gate?: GateName): FixtureEntry[];
  prepare(fixture: FixtureEntry): Promise<FixtureContext>;
  log(line: string): void;
  close(): Promise<void>;
}

export async function openProbe(argv: string[]): Promise<ProbeEnv> {
  const loaded = loadCorpus();
  if ('error' in loaded) throw new Error(loaded.error.join('\n'));
  if (!ensureBuilt(false)) throw new Error('render:build failed');
  const { corpus, io } = loaded;
  const names = argv.filter((a) => !a.startsWith('-'));
  for (const n of names) {
    if (!corpus.fixtures.some((f) => f.name === n)) throw new Error(`unknown fixture ${n}`);
  }
  const { SuperdoughRenderer } = await import('../superdough-renderer');
  const renderer = new SuperdoughRenderer();
  const chromium = await renderer.version();
  let harness = 'unknown';
  try {
    harness = execSync('git rev-parse --short HEAD', { cwd: ROOT }).toString().trim();
  } catch {
    // not a git checkout
  }
  const t0 = performance.now();
  const log = (line: string) =>
    console.log(`[${((performance.now() - t0) / 1000).toFixed(0).padStart(5)}s] ${line}`);
  const readSong = (f: string) => io.readSong(f) ?? '';
  // Bounds the renders that go through a fixture context (chunkResults, renderIn).
  const limit = new Limiter(CONCURRENCY);
  return {
    renderer,
    corpus,
    header: {
      generatedAt: new Date().toISOString(),
      playwright: playwrightPin(),
      chromium,
      harness,
    },
    fixtures: (gate) =>
      corpus.fixtures.filter(
        (f) =>
          (names.length === 0 || names.includes(f.name)) &&
          (gate === undefined || f.gates.includes(gate)),
      ),
    prepare: async (fixture) => {
      const p = await prepareFixture(fixture, { renderer, corpus, readSong, limit }, log);
      if ('errors' in p) throw new Error(`${fixture.name}: ${p.errors.join('; ')}`);
      if ('unsupported' in p) throw new Error(`${fixture.name}: unsupported by the renderer`);
      return p.fx;
    },
    log,
    close: () => renderer.dispose(),
  };
}

/** Writes `<evidence dir>/<name>[-<label>].json` (2-space JSON, trailing newline) and returns its
 *  path; `PROBE_EVIDENCE_DIR` and `PROBE_LABEL` as in `evidence-path.ts`, `defaultDir` when
 *  `PROBE_EVIDENCE_DIR` is not set. */
export function writeEvidence(name: string, body: object, defaultDir?: string): string {
  const { path } = evidencePath(ROOT, name, process.env, defaultDir);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(body, null, 2) + '\n');
  return path;
}

/** Renders at once in a probe, as in render:check. */
export const CONCURRENCY = DEFAULT_CONCURRENCY;

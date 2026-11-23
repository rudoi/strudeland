/**
 * Where an engine reference came from: the sha256 of its source (a micro case or a corpus fixture
 * with its window and gates), the vendored Strudel commit and its local patches, the job planner,
 * the Playwright pin and the Chromium it drives, and the generating platform. `engine:refs` stamps it into every ref job and `index.json`; the
 * staleness test (`tests/engine-artifacts.test.ts`) compares the committed value with
 * `provenanceOf` (`isFresh`, every field but `platform`). No Playwright import; it reads files
 * under `root` only.
 */

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { ENGINE_JOB_SCHEMA, type Provenance } from '../../src/lib/render/engine-job';
import { readPlaywrightPin } from '../playwright-pin';
import { sha256 } from '../sha256';
import { inputsSha256 } from './v8-math-cases';
import { parsePatches } from '../strudel-patches';

/** Micro-case sources, relative to the repo root. */
export const MICRO_DIR = 'tests/render/engine-micro';
/** Corpus songs and manifest, relative to the repo root. */
const CORPUS_DIR = 'tests/render/corpus';
const CORPUS_MANIFEST = 'tests/render/corpus.json';
/** The log of local patches to the vendored Strudel; paths in it are relative to `VENDOR_DIR`. */
const VENDOR_DIR = 'vendor/strudel';
const PATCHES_FILE = 'vendor/strudel/PATCHES.md';
/** The shared job planner (the render page's and the headless host's): the haps, window, pauses
 *  and values a ref job holds, the schedule and lookback that set a fixture job's window, the
 *  evaluate-query-plan sequence (`job-plan.ts`) and the job text it is written from
 *  (`page-job.ts`). Appended only (the hash is order-sensitive). */
export const PLANNER_FILES = [
  'src/lib/render/plan.ts',
  'src/lib/render/engine-job.ts',
  'src/lib/render/lookback.ts',
  'src/lib/render/schedule.ts',
  'src/lib/render/job-plan.ts',
  'src/lib/render/page-job.ts',
  'src/lib/render/timing.ts',
  'src/lib/strudel/timing.ts',
] as const;
/** A corpus fixture's case name is this prefix plus the fixture name (its ref files start with
 *  it too); every other case name is a micro case. */
export const FIXTURE_PREFIX = 'fixture-';

/** The V8 math reference's case name and file (case kind `math`); no micro source or corpus
 *  fixture may take the name. */
export const MATH_CASE = 'v8-math';
export const MATH_FILE = 'v8-math.json';

export type CaseKind = 'micro' | 'fixture' | 'math';

export const caseKind = (name: string): CaseKind =>
  name === MATH_CASE ? 'math' : name.startsWith(FIXTURE_PREFIX) ? 'fixture' : 'micro';

/** The case name of a corpus fixture. */
export const fixtureCase = (fixture: string): string => `${FIXTURE_PREFIX}${fixture}`;

const readText = (root: string, path: string): string => readFileSync(join(root, path), 'utf8');

/** The hashed source text: a micro case's file, or a fixture's song file + `"\n"` +
 *  `JSON.stringify({ windowBars, layout, gates })` of its manifest entry (so a window change, or a
 *  gate change that adds or drops the stems refs, makes its refs stale). Throws on an unknown
 *  fixture or a missing file. */
export function sourceText(root: string, name: string): string {
  if (caseKind(name) === 'micro') return readText(root, join(MICRO_DIR, `${name}.js`));
  const fixture = name.slice(FIXTURE_PREFIX.length);
  const manifest = JSON.parse(readText(root, CORPUS_MANIFEST)) as {
    fixtures?: {
      name: string;
      file: string;
      windowBars: unknown;
      layout: unknown;
      gates: unknown;
    }[];
  };
  const entry = manifest.fixtures?.find((f) => f.name === fixture);
  if (!entry) throw new Error(`no corpus fixture ${fixture} in ${CORPUS_MANIFEST}`);
  const song = readText(root, join(CORPUS_DIR, entry.file));
  const window = { windowBars: entry.windowBars, layout: entry.layout, gates: entry.gates };
  return `${song}\n${JSON.stringify(window)}`;
}

/** sha256 of `PATCHES.md` and, for every vendored file a patch lists (sorted, unique), a line
 *  `<path> <sha256 of its text>` (`-` for a file a patch deleted): the local edits on top of
 *  `strudelCommit`, so changing a patch's code makes the refs stale even when `PATCHES.md` does
 *  not change. */
export function strudelPatchesSha256(root: string): string {
  const log = readText(root, PATCHES_FILE);
  const files = [...new Set(parsePatches(log).flatMap((p) => p.files))].sort();
  const lines = files.map((f) => {
    const path = join(VENDOR_DIR, f);
    return `${f} ${existsSync(join(root, path)) ? sha256(readText(root, path)) : '-'}`;
  });
  return sha256(`${log}\n${lines.join('\n')}`);
}

/** sha256 of the job planner's sources (`PLANNER_FILES`, each as `<path>\n<text>\n`). */
export function plannerSha256(root: string): string {
  return sha256(PLANNER_FILES.map((f) => `${f}\n${readText(root, f)}\n`).join(''));
}

/** The vendored Strudel commit and its local patches (`strudelPatchesSha256`), the job planner
 *  (`plannerSha256`), the exact `playwright` devDependency and the Chromium `browserVersion`
 *  playwright-core bundles. */
export function toolchainOf(root: string): {
  strudelCommit: string;
  strudelPatchesSha256: string;
  plannerSha256: string;
  playwright: string;
  chromium: string;
} {
  const upstream = JSON.parse(readText(root, 'vendor/strudel/UPSTREAM.json'));
  const browsers = JSON.parse(readText(root, 'node_modules/playwright-core/browsers.json')) as {
    browsers?: { name: string; browserVersion?: string }[];
  };
  const chromium = browsers.browsers?.find((b) => b.name === 'chromium')?.browserVersion;
  const playwright = readPlaywrightPin(root);
  if (typeof upstream.commit !== 'string') throw new Error('UPSTREAM.json has no commit');
  if (playwright === null) throw new Error('package.json has no playwright pin');
  if (typeof chromium !== 'string') throw new Error('browsers.json has no chromium version');
  return {
    strudelCommit: upstream.commit,
    strudelPatchesSha256: strudelPatchesSha256(root),
    plannerSha256: plannerSha256(root),
    playwright,
    chromium,
  };
}

/** The provenance a ref of case `name` generated now on this machine would carry. */
export function provenanceOf(
  root: string,
  name: string,
  platform: string = `${process.platform}-${process.arch}`,
): Provenance {
  return {
    schema: ENGINE_JOB_SCHEMA,
    sourceSha256: sha256(sourceText(root, name)),
    ...toolchainOf(root),
    platform,
  };
}

const COMPARED = [
  'schema',
  'sourceSha256',
  'strudelCommit',
  'strudelPatchesSha256',
  'plannerSha256',
  'playwright',
  'chromium',
] as const;

/** Compares a committed provenance with the current one field by field, except `platform`
 *  (informational). `differs` names the differing fields in a fixed order. */
export function isFresh(
  committed: Provenance,
  current: Provenance,
): { fresh: boolean; differs: string[] } {
  const differs = COMPARED.filter((k) => committed[k] !== current[k]);
  return { fresh: differs.length === 0, differs: [...differs] };
}

/** The provenance of the V8 math reference (`v8-math.json` and its `index.json` entry): the grid
 *  (`inputsSha256` of `v8-math-cases.ts`) and the Chromium it was evaluated in. It does not hash
 *  the Strudel commit, patches or planner: V8 math depends on Chromium only. `schema` is the
 *  `v8-math.json` format version. */
export interface MathProvenance {
  schema: 1;
  inputsSha256: string;
  playwright: string;
  chromium: string;
  platform: string;
}

/** The math provenance a reference generated now on this machine would carry. */
export function mathProvenanceOf(
  root: string,
  platform: string = `${process.platform}-${process.arch}`,
): MathProvenance {
  const { playwright, chromium } = toolchainOf(root);
  return { schema: 1, inputsSha256: inputsSha256(), playwright, chromium, platform };
}

const MATH_COMPARED = ['schema', 'inputsSha256', 'playwright', 'chromium'] as const;

/** `isFresh` for the math provenance: every field but `platform`. */
export function isMathFresh(
  committed: MathProvenance,
  current: MathProvenance,
): { fresh: boolean; differs: string[] } {
  const differs = MATH_COMPARED.filter((k) => committed[k] !== current[k]);
  return { fresh: differs.length === 0, differs: [...differs] };
}

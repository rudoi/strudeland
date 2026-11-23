import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import type { Provenance } from '../src/lib/render/engine-job';
import type { MathProvenance } from '../scripts/engine/provenance';
import { inputsSha256 } from '../scripts/engine/v8-math-cases';
import {
  caseKind,
  fixtureCase,
  isFresh,
  isMathFresh,
  MATH_CASE,
  MATH_FILE,
  mathProvenanceOf,
  plannerSha256,
  PLANNER_FILES,
  provenanceOf,
  sourceText,
  strudelPatchesSha256,
  toolchainOf,
} from '../scripts/engine/provenance';

const sha = (s: string) => createHash('sha256').update(s).digest('hex');

let root: string;

const PATCHES = [
  '# Local patches',
  '',
  '## audio-rng: seedable random numbers',
  'Upstream: candidate',
  'Seeds the noise.',
  '```files',
  'packages/superdough/noise.mjs',
  '```',
  '',
  '## hap-tap: tap every hap',
  'Upstream: candidate',
  'Taps.',
  '```files',
  'packages/core/repl.mjs',
  'packages/superdough/noise.mjs',
  '```',
  '',
].join('\n');

function put(path: string, text: string): void {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'strudeland-prov-'));
  put('vendor/strudel/UPSTREAM.json', JSON.stringify({ commit: 'abc123' }));
  put('vendor/strudel/PATCHES.md', PATCHES);
  put('vendor/strudel/packages/superdough/noise.mjs', 'patched noise\n');
  put('vendor/strudel/packages/core/repl.mjs', 'patched repl\n');
  put('src/lib/render/plan.ts', 'plan\n');
  put('src/lib/render/engine-job.ts', 'engine job\n');
  put('src/lib/render/lookback.ts', 'lookback\n');
  put('src/lib/render/schedule.ts', 'schedule\n');
  put('src/lib/render/job-plan.ts', 'job plan\n');
  put('src/lib/render/page-job.ts', 'page job\n');
  put('package.json', JSON.stringify({ devDependencies: { playwright: '1.64.1' } }));
  put(
    'node_modules/playwright-core/browsers.json',
    JSON.stringify({
      browsers: [
        { name: 'chromium-headless-shell', browserVersion: '1.0' },
        { name: 'chromium', browserVersion: '156.0.8078.4' },
      ],
    }),
  );
  put('tests/render/engine-micro/osc-sine.js', 'micro source\n');
  put('tests/render/corpus/song.js', 'song source\n');
  put(
    'tests/render/corpus.json',
    JSON.stringify({
      fixtures: [
        {
          name: 'song',
          file: 'song.js',
          windowBars: [0, 4],
          layout: 'linear',
          gates: ['seam'],
        },
      ],
    }),
  );
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

describe('case names', () => {
  test('fixture cases carry the prefix; everything else is micro', () => {
    expect(fixtureCase('bass-line')).toBe('fixture-bass-line');
    expect(caseKind('fixture-bass-line')).toBe('fixture');
    expect(caseKind('osc-sine')).toBe('micro');
    expect(caseKind('fixtures')).toBe('micro');
  });

  test('v8-math is the math case', () => {
    expect(MATH_CASE).toBe('v8-math');
    expect(MATH_FILE).toBe('v8-math.json');
    expect(caseKind('v8-math')).toBe('math');
    expect(caseKind('v8-math-2')).toBe('micro');
    expect(caseKind('fixture-v8-math')).toBe('fixture');
  });
});

describe('sourceText', () => {
  test('a micro case is its file', () => {
    expect(sourceText(root, 'osc-sine')).toBe('micro source\n');
  });

  test('a fixture is its song, a newline and its window and gates', () => {
    expect(sourceText(root, 'fixture-song')).toBe(
      'song source\n\n{"windowBars":[0,4],"layout":"linear","gates":["seam"]}',
    );
  });

  test('unknown fixtures and missing files throw', () => {
    expect(() => sourceText(root, 'fixture-nope')).toThrow(/no corpus fixture nope/);
    expect(() => sourceText(root, 'missing-case')).toThrow();
  });
});

describe('toolchainOf', () => {
  test('reads the strudel commit and patches, the planner, the pins', () => {
    expect(toolchainOf(root)).toEqual({
      strudelCommit: 'abc123',
      strudelPatchesSha256: sha(
        `${PATCHES}\npackages/core/repl.mjs ${sha('patched repl\n')}\n` +
          `packages/superdough/noise.mjs ${sha('patched noise\n')}`,
      ),
      plannerSha256: sha(
        'src/lib/render/plan.ts\nplan\n\nsrc/lib/render/engine-job.ts\nengine job\n\n' +
          'src/lib/render/lookback.ts\nlookback\n\nsrc/lib/render/schedule.ts\nschedule\n\n' +
          'src/lib/render/job-plan.ts\njob plan\n\nsrc/lib/render/page-job.ts\npage job\n\n',
      ),
      playwright: '1.64.1',
      chromium: '156.0.8078.4',
    });
  });

  test('a patched file edit changes the patches hash even with PATCHES.md unchanged', () => {
    const before = strudelPatchesSha256(root);
    put('vendor/strudel/packages/superdough/noise.mjs', 'patched noise, edited\n');
    expect(strudelPatchesSha256(root)).not.toBe(before);
  });

  test('a PATCHES.md edit changes the patches hash; a deleted listed file is hashed as -', () => {
    const before = strudelPatchesSha256(root);
    put('vendor/strudel/PATCHES.md', PATCHES + '\n');
    const edited = strudelPatchesSha256(root);
    expect(edited).not.toBe(before);
    rmSync(join(root, 'vendor/strudel/packages/core/repl.mjs'));
    expect(strudelPatchesSha256(root)).not.toBe(edited);
  });

  test('a planner edit changes the planner hash', () => {
    expect(PLANNER_FILES).toEqual([
      'src/lib/render/plan.ts',
      'src/lib/render/engine-job.ts',
      'src/lib/render/lookback.ts',
      'src/lib/render/schedule.ts',
      'src/lib/render/job-plan.ts',
      'src/lib/render/page-job.ts',
      'src/lib/render/timing.ts',
      'src/lib/strudel/timing.ts',
    ]);
    for (const f of PLANNER_FILES) {
      const before = plannerSha256(root);
      put(f, 'edited\n');
      expect(plannerSha256(root)).not.toBe(before);
    }
  });

  test('missing fields throw', () => {
    put('node_modules/playwright-core/browsers.json', JSON.stringify({ browsers: [] }));
    expect(() => toolchainOf(root)).toThrow(/chromium/);
    put('package.json', '{}');
    expect(() => toolchainOf(root)).toThrow(/playwright/);
    put('vendor/strudel/UPSTREAM.json', '{}');
    expect(() => toolchainOf(root)).toThrow(/commit/);
  });
});

describe('provenanceOf', () => {
  test('hashes the source and records the toolchain and platform', () => {
    expect(provenanceOf(root, 'osc-sine', 'darwin-arm64')).toEqual({
      schema: 2,
      sourceSha256: sha('micro source\n'),
      strudelCommit: 'abc123',
      strudelPatchesSha256: strudelPatchesSha256(root),
      plannerSha256: plannerSha256(root),
      playwright: '1.64.1',
      chromium: '156.0.8078.4',
      platform: 'darwin-arm64',
    });
  });

  test('the platform defaults to this process', () => {
    expect(provenanceOf(root, 'osc-sine').platform).toBe(`${process.platform}-${process.arch}`);
  });

  test('a fixture window or gate change changes the hash', () => {
    const entry = {
      name: 'song',
      file: 'song.js',
      windowBars: [0, 4],
      layout: 'linear',
      gates: ['seam'],
    };
    const hashWith = (o: object) => {
      put('tests/render/corpus.json', JSON.stringify({ fixtures: [{ ...entry, ...o }] }));
      return provenanceOf(root, 'fixture-song').sourceSha256;
    };
    const before = hashWith({});
    expect(hashWith({ windowBars: [0, 8] })).not.toBe(before);
    expect(hashWith({ gates: ['seam', 'stems'] })).not.toBe(before);
    expect(hashWith({})).toBe(before);
  });

  test('works on the real repository', () => {
    const p = provenanceOf(resolve(import.meta.dir, '..'), 'fixture-sine-basic');
    expect(p.sourceSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(p.strudelPatchesSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(p.plannerSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(p.playwright).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe('isFresh', () => {
  const base: Provenance = {
    schema: 2,
    sourceSha256: 'a',
    strudelCommit: 'b',
    strudelPatchesSha256: 'e',
    plannerSha256: 'f',
    playwright: 'c',
    chromium: 'd',
    platform: 'darwin-arm64',
  };

  test('equal provenance is fresh', () => {
    expect(isFresh(base, { ...base })).toEqual({ fresh: true, differs: [] });
  });

  test('platform is informational', () => {
    expect(isFresh(base, { ...base, platform: 'linux-x64' }).fresh).toBe(true);
  });

  test('names every differing field in order', () => {
    const now = { ...base, chromium: 'x', sourceSha256: 'y', schema: 3 as 2 };
    expect(isFresh(base, now)).toEqual({
      fresh: false,
      differs: ['schema', 'sourceSha256', 'chromium'],
    });
    expect(isFresh(base, { ...base, strudelCommit: 'z', playwright: 'q' }).differs).toEqual([
      'strudelCommit',
      'playwright',
    ]);
    const local = { ...base, plannerSha256: 'p', strudelPatchesSha256: 's' };
    expect(isFresh(base, local)).toEqual({
      fresh: false,
      differs: ['strudelPatchesSha256', 'plannerSha256'],
    });
  });
});

describe('mathProvenanceOf', () => {
  test('hashes the grid and records the pins and platform, not the Strudel commit', () => {
    expect(mathProvenanceOf(root, 'darwin-arm64')).toEqual({
      schema: 1,
      inputsSha256: inputsSha256(),
      playwright: '1.64.1',
      chromium: '156.0.8078.4',
      platform: 'darwin-arm64',
    });
    expect(mathProvenanceOf(root).platform).toBe(`${process.platform}-${process.arch}`);
  });

  test('a Strudel patch or planner edit does not change it; a Chromium pin does', () => {
    const before = mathProvenanceOf(root, 'p');
    put('vendor/strudel/PATCHES.md', PATCHES + '\n');
    put('src/lib/render/plan.ts', 'edited\n');
    expect(mathProvenanceOf(root, 'p')).toEqual(before);
    put(
      'node_modules/playwright-core/browsers.json',
      JSON.stringify({ browsers: [{ name: 'chromium', browserVersion: '157.0.0.0' }] }),
    );
    expect(mathProvenanceOf(root, 'p').chromium).toBe('157.0.0.0');
  });
});

describe('isMathFresh', () => {
  const base: MathProvenance = {
    schema: 1,
    inputsSha256: 'a',
    playwright: 'b',
    chromium: 'c',
    platform: 'darwin-arm64',
  };

  test('equal is fresh; platform is informational', () => {
    expect(isMathFresh(base, { ...base })).toEqual({ fresh: true, differs: [] });
    expect(isMathFresh(base, { ...base, platform: 'linux-x64' }).fresh).toBe(true);
  });

  test('names every differing field in order', () => {
    expect(isMathFresh(base, { ...base, chromium: 'x', inputsSha256: 'y' })).toEqual({
      fresh: false,
      differs: ['inputsSha256', 'chromium'],
    });
    expect(isMathFresh(base, { ...base, schema: 2 as 1, playwright: 'z' }).differs).toEqual([
      'schema',
      'playwright',
    ]);
  });
});

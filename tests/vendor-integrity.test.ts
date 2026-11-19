import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { afterEach, describe, expect, test } from 'bun:test';
import { jsonText } from '../scripts/json-text';
import { sha256 } from '../scripts/sha256';
import { isVendoredPath } from '../scripts/strudel-sync';
import { checkVendor, walkVendorPackages } from './helpers/vendor-integrity';

const vendor = resolve(import.meta.dir, '../vendor/strudel');
const readJson = (rel: string) => JSON.parse(readFileSync(join(vendor, rel), 'utf8'));

const EXCLUDED_AT_INITIAL = [
  'core/bench/pattern.bench.mjs',
  'core/bench/signal.bench.mjs',
  'core/test/controls.test.mjs',
  'core/test/drawLine.test.mjs',
  'core/test/euclid.test.js',
  'core/test/fraction.test.mjs',
  'core/test/pattern.test.mjs',
  'core/test/signal.test.mjs',
  'core/test/solmization.test.js',
  'core/test/util.test.mjs',
  'core/test/value.test.mjs',
  'mini/bench/mini.bench.mjs',
  'mini/test/mini.test.mjs',
  'tonal/test/tonal.test.mjs',
  'tonal/test/tonleiter.test.mjs',
  'transpiler/test/transpiler.test.mjs',
].map((p) => `packages/${p}`);

describe('vendor/strudel', () => {
  const report = checkVendor(vendor);
  const upstream = readJson('UPSTREAM.json');
  const manifest = readJson('upstream-manifest.json');

  test('every manifest file exists', () => {
    expect(report.missing).toEqual([]);
  });

  test('every manifest file matches its hash unless PATCHES.md lists it', () => {
    expect(report.modifiedUnlisted).toEqual([]);
  });

  test('every file PATCHES.md lists differs from upstream or exists as an added file', () => {
    expect(report.staleListed).toEqual([]);
    expect(report.listedMissing).toEqual([]);
  });

  test('no unlisted files under the vendored packages', () => {
    expect(report.unlisted).toEqual([]);
  });

  test('UPSTREAM.json and upstream-manifest.json agree on the commit', () => {
    expect(upstream.commit).toMatch(/^[0-9a-f]{40}$/);
    expect(manifest.commit).toBe(upstream.commit);
    expect(manifest.schema).toBe(1);
  });

  test('UPSTREAM.json has the repo, an ISO date and sorted paths', () => {
    expect(Object.keys(upstream)).toEqual(['repo', 'commit', 'date', 'paths']);
    expect(upstream.repo).toBe('https://codeberg.org/uzu/strudel');
    expect(upstream.date).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
    expect(upstream.paths).toEqual([...upstream.paths].sort());
  });

  test('manifest keys are sorted, include LICENSE, and sit under a vendored path', () => {
    const keys = Object.keys(manifest.files);
    expect(keys).toEqual([...keys].sort());
    expect(keys).toContain('LICENSE');
    for (const k of keys.filter((k) => k !== 'LICENSE')) {
      expect(upstream.paths.some((p: string) => k.startsWith(`${p}/`))).toBe(true);
      expect(isVendoredPath(k.slice('packages/'.length))).toBe(true);
    }
    for (const hash of Object.values(manifest.files)) expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  test('the manifest has none of the excluded files', () => {
    for (const p of EXCLUDED_AT_INITIAL) expect(manifest.files[p]).toBeUndefined();
  });

  test('the JSON files are in the sync script format', () => {
    for (const f of ['UPSTREAM.json', 'upstream-manifest.json', 'doc.json'])
      expect(readFileSync(join(vendor, f), 'utf8')).toBe(jsonText(readJson(f)));
  });
});

describe('checkVendor over a fixture', () => {
  let dir = '';
  afterEach(() => {
    if (dir) rmSync(dir, { recursive: true, force: true });
    dir = '';
  });

  const hash = sha256;
  const put = (rel: string, text: string) => {
    mkdirSync(dirname(join(dir, rel)), { recursive: true });
    writeFileSync(join(dir, rel), text);
  };
  function fixture(patches = '# Patches\n') {
    dir = mkdtempSync(join(tmpdir(), 'vendor-integrity-'));
    put('LICENSE', 'license');
    put('packages/core/a.mjs', 'a');
    put('packages/core/b.mjs', 'b');
    put(
      'upstream-manifest.json',
      jsonText({
        schema: 1,
        commit: 'x',
        files: {
          LICENSE: hash('license'),
          'packages/core/a.mjs': hash('a'),
          'packages/core/b.mjs': hash('b'),
        },
      }),
    );
    put('PATCHES.md', patches);
  }
  const entry = (id: string, ...files: string[]) =>
    `## ${id}: t\nUpstream: candidate\nwhy\n\`\`\`files\n${files.join('\n')}\n\`\`\`\n`;
  const empty = {
    missing: [],
    modifiedUnlisted: [],
    staleListed: [],
    listedMissing: [],
    unlisted: [],
  };

  test('happy: a pristine tree reports nothing', () => {
    fixture();
    expect(checkVendor(dir)).toEqual(empty);
  });

  test('a patched file listed in PATCHES.md and an added listed file are accepted', () => {
    fixture(entry('p', 'packages/core/a.mjs', 'packages/core/new.mjs'));
    put('packages/core/a.mjs', 'patched');
    put('packages/core/new.mjs', 'new');
    expect(checkVendor(dir)).toEqual(empty);
  });

  test('error: a modified unlisted file is reported', () => {
    fixture();
    put('packages/core/a.mjs', 'patched');
    expect(checkVendor(dir).modifiedUnlisted).toEqual(['packages/core/a.mjs']);
  });

  test('error: a missing manifest file is reported', () => {
    fixture();
    rmSync(join(dir, 'packages/core/b.mjs'));
    expect(checkVendor(dir).missing).toEqual(['packages/core/b.mjs']);
  });

  test('error: a modified LICENSE is reported', () => {
    fixture();
    put('LICENSE', 'other');
    expect(checkVendor(dir).modifiedUnlisted).toEqual(['LICENSE']);
  });

  test('error: a listed file identical to upstream is stale', () => {
    fixture(entry('p', 'packages/core/a.mjs'));
    expect(checkVendor(dir).staleListed).toEqual(['p: packages/core/a.mjs']);
  });

  test('error: a listed added file that does not exist is reported', () => {
    fixture(entry('p', 'packages/core/gone.mjs'));
    expect(checkVendor(dir).listedMissing).toEqual(['p: packages/core/gone.mjs']);
  });

  test('error: an unlisted file is reported, including under dist, test and bench', () => {
    fixture();
    for (const p of ['extra.mjs', 'dist/index.mjs', 'test/x.test.mjs', 'bench/x.bench.mjs'])
      put(`packages/core/${p}`, 'x');
    put('packages/other/index.mjs', 'x');
    expect(checkVendor(dir).unlisted).toEqual([
      'packages/core/bench/x.bench.mjs',
      'packages/core/dist/index.mjs',
      'packages/core/extra.mjs',
      'packages/core/test/x.test.mjs',
      'packages/other/index.mjs',
    ]);
  });

  test('edge: node_modules directories are skipped by the walk', () => {
    fixture();
    put('packages/core/node_modules/dep/index.js', 'x');
    put('packages/core/sub/node_modules/x.js', 'x');
    expect(walkVendorPackages(dir)).toEqual(['packages/core/a.mjs', 'packages/core/b.mjs']);
    expect(checkVendor(dir)).toEqual(empty);
  });

  test('error: a malformed PATCHES.md throws', () => {
    fixture('## Bad: t\nUpstream: candidate\n```files\npackages/core/a.mjs\n```\n');
    expect(() => checkVendor(dir)).toThrow('id must match');
  });
});

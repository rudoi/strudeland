// strudel-pins.json (schema 2): the vendored Strudel commit agrees everywhere it is recorded, the
// npm pins (kabelsalat) match what is installed, and package.json consumes the vendored packages.
// The vendored tree itself is checked file by file in vendor-integrity.test.ts.
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, test } from 'bun:test';
import { jsonText } from '../scripts/json-text';
import { sha256 } from '../scripts/sha256';
import { readPins, updatePins } from '../scripts/strudel-sync';

const root = resolve(import.meta.dir, '..');
const read = (rel: string) => readFileSync(join(root, rel));
const readJson = (rel: string) => JSON.parse(read(rel).toString());

const pins = readPins(root);
const upstream = readJson('vendor/strudel/UPSTREAM.json');
const manifest = readJson('vendor/strudel/upstream-manifest.json');
const docs = readJson('crates/strudeland-cli/strudel-docs.json');
const pkg = readJson('package.json');

const deps: Record<string, string> = { ...pkg.dependencies, ...pkg.devDependencies };
const workspaceDeps = Object.keys(deps)
  .filter((n) => deps[n] === 'workspace:*')
  .sort();
const catalog: Record<string, string> = pkg.workspaces.catalog;
const vendorPackages = join(root, 'vendor/strudel/packages');

describe('strudel-pins.json', () => {
  test('is schema 2 with the keys in order and a 40-hex commit', () => {
    expect(Object.keys(pins)).toEqual(['schema', 'strudel', 'npm']);
    expect(Object.keys(pins.strudel)).toEqual(['repo', 'commit', 'date']);
    expect(pins.strudel.commit).toMatch(/^[0-9a-f]{40}$/);
    expect(pins.strudel.date).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
    expect(read('strudel-pins.json').toString()).toBe(jsonText(pins));
  });

  test('the strudel block is what the sync script would write from UPSTREAM.json', () => {
    expect(updatePins(pins, upstream)).toEqual(pins);
  });

  test('the commit agrees across UPSTREAM.json, the manifest and the embedded docs', () => {
    expect(upstream.repo).toBe(pins.strudel.repo);
    expect(upstream.commit).toBe(pins.strudel.commit);
    expect(upstream.date).toBe(pins.strudel.date);
    expect(manifest.commit).toBe(pins.strudel.commit);
    expect(docs.source).toEqual({ repo: pins.strudel.repo, commit: pins.strudel.commit });
  });

  test('the npm block holds only the kabelsalat packages', () => {
    expect(Object.keys(pins.npm).sort()).toEqual(['@kabelsalat/lib', '@kabelsalat/web']);
  });

  for (const [name, pin] of Object.entries(pins.npm)) {
    test(`${name} matches its pin, and package.json lists the same exact version`, () => {
      expect(readJson(`node_modules/${name}/package.json`).version).toBe(pin.version);
      expect(sha256(read(`node_modules/${name}/${pin.file}`))).toBe(pin.sha256);
      expect(pkg.dependencies[name]).toBe(pin.version);
    });
  }
});

describe('vendor/strudel/doc.json', () => {
  test('parses, has a non-empty docs array with relative paths, and is what codemirror imports', () => {
    const doc = readJson('vendor/strudel/doc.json');
    expect(Array.isArray(doc.docs)).toBe(true);
    expect(doc.docs.length).toBeGreaterThan(0);
    for (const d of doc.docs)
      if (d.meta?.path !== undefined) expect(d.meta.path).toMatch(/^packages\//);
    // packages/codemirror/autocomplete.mjs imports '../../doc.json'.
    expect(read('vendor/strudel/packages/codemirror/autocomplete.mjs').toString()).toContain(
      "'../../doc.json'",
    );
    const codemirror = join(root, 'vendor/strudel/packages/codemirror');
    expect(resolve(codemirror, '../../doc.json')).toBe(join(root, 'vendor/strudel/doc.json'));
  });
});

describe('package.json', () => {
  test('the vendored packages are workspaces, consumed as workspace:* dependencies', () => {
    expect(pkg.workspaces.packages).toEqual(['vendor/strudel/packages/*']);
    expect(workspaceDeps.length).toBeGreaterThan(0);
    // Every workspace dependency is a vendored package directory.
    for (const name of workspaceDeps) {
      const dir = JSON.parse(
        readFileSync(join(vendorPackages, name.replace(/^@strudel\//, ''), 'package.json'), 'utf8'),
      ).name;
      expect(dir).toBe(name);
    }
  });

  test('no Strudel package or superdough comes from npm', () => {
    for (const name of Object.keys(deps))
      if (name.startsWith('@strudel/') || name === 'superdough')
        expect(deps[name]).toBe('workspace:*');
  });

  test('catalog versions equal the root direct dependencies where both exist', () => {
    const shared = Object.keys(catalog).filter((n) => n in deps);
    expect(shared.length).toBeGreaterThan(0);
    for (const name of shared) expect(deps[name]).toBe(catalog[name]);
  });

  test('the catalog covers every catalog: spec of the vendored packages', () => {
    const used = new Set<string>();
    for (const dir of new Bun.Glob('*/package.json').scanSync({ cwd: vendorPackages })) {
      const p = JSON.parse(readFileSync(join(vendorPackages, dir), 'utf8'));
      for (const group of ['dependencies', 'devDependencies', 'peerDependencies'])
        for (const [name, spec] of Object.entries<string>(p[group] ?? {}))
          if (spec.startsWith('catalog:')) used.add(name);
    }
    for (const name of used) expect(catalog[name]).toBeDefined();
  });

  test('one vite: the root version overrides the vendored packages devDependencies', () => {
    expect(pkg.overrides.vite).toBe(pkg.devDependencies.vite);
    expect(readJson('node_modules/vite/package.json').version).toBe(pkg.devDependencies.vite);
  });

  test('the vendored packages use the Vitest override without Tinypool', () => {
    const vitest = readJson('node_modules/vitest/package.json');
    expect(vitest.version).toBe(pkg.overrides.vitest);
    expect(vitest.dependencies).not.toHaveProperty('tinypool');
  });
});

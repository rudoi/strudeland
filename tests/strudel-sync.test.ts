// Tests of the sync script. No network: upstream is a local fixture git repo, jsdoc is replaced by an
// injected doc.json producer, and `git merge-file` is real.
import { spawnSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from 'bun:test';
import { jsonText } from '../scripts/json-text';
import {
  checkDeps,
  DOC_PATHS,
  readPins,
  updatePins,
  writePinsAndDocs,
  EXIT_CONFLICT,
  ENGINE_REMINDER,
  HOST_REMINDER,
  EXIT_REFUSED,
  isVendoredPath,
  normalizeDocJson,
  parseArgs,
  planFile,
  sync,
  type SyncOptions,
  type Upstream,
  VENDORED_PATHS,
} from '../scripts/strudel-sync';
import { sha256 } from '../scripts/sha256';
import { walkFiles } from './helpers/vendor-integrity';

const enc = (s: string) => new TextEncoder().encode(s);
const dec = (b: Uint8Array) => new TextDecoder().decode(b);

describe('isVendoredPath', () => {
  test('true for ordinary package files', () => {
    for (const p of ['core/repl.mjs', 'codemirror/themes/nord.mjs', 'core/.npmignore'])
      expect(isVendoredPath(p)).toBe(true);
  });

  test('false for test, bench, node_modules and dist paths', () => {
    for (const p of [
      'core/test/x.mjs',
      'core/bench/x.mjs',
      'a/__tests__/x.mjs',
      'core/x.test.mjs',
      'core/x.test.js',
      'core/x.bench.mjs',
      'core/x.bench.js',
      'core/node_modules/x/y.js',
      'core/dist/index.mjs',
    ])
      expect(isVendoredPath(p)).toBe(false);
  });

  test('edge: names that only resemble excluded ones are vendored', () => {
    for (const p of ['core/testing.mjs', 'core/x.test.ts', 'core/latest/x.mjs', 'core/distx.mjs'])
      expect(isVendoredPath(p)).toBe(true);
  });

  test('false for the 16 files excluded at the initial commit', () => {
    const excluded = [
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
    ];
    expect(excluded.filter(isVendoredPath)).toEqual([]);
  });
});

describe('planFile', () => {
  const base = enc('one\ntwo\nthree\nfour\nfive\n');
  const theirs = enc('ONE\ntwo\nthree\nfour\nfive\n');

  test('theirs same as base keeps ours, whatever ours is', () => {
    expect(planFile({ base, theirs: base, ours: base })).toEqual({ action: 'keep' });
    expect(planFile({ base, theirs: base, ours: enc('patched') })).toEqual({ action: 'keep' });
    expect(planFile({ base, theirs: base, ours: null })).toEqual({ action: 'keep' });
    expect(planFile({ base: null, theirs: null, ours: enc('ours only') })).toEqual({
      action: 'keep',
    });
  });

  test('pristine ours is replaced with theirs', () => {
    const plan = planFile({ base, theirs, ours: base });
    expect(plan).toMatchObject({ action: 'write', why: 'theirs' });
    expect(plan.action === 'write' && dec(plan.content)).toBe(dec(theirs));
  });

  test('a patched file merges cleanly', () => {
    const ours = enc('one\ntwo\nthree\nfour\nFIVE\n');
    const plan = planFile({ base, theirs, ours });
    expect(plan).toMatchObject({ action: 'write', why: 'merged' });
    expect(plan.action === 'write' && dec(plan.content)).toBe('ONE\ntwo\nthree\nfour\nFIVE\n');
  });

  test('a conflicting merge writes markers and is flagged', () => {
    const ours = enc('uno\ntwo\nthree\nfour\nfive\n');
    const plan = planFile({ base, theirs, ours });
    expect(plan).toMatchObject({ action: 'conflict', why: 'merge' });
    const text = plan.action === 'conflict' && plan.content ? dec(plan.content) : '';
    expect(text).toContain('<<<<<<< vendored\nuno\n');
    expect(text).toContain('||||||| upstream (old)\none\n');
    expect(text).toContain('=======\nONE\n>>>>>>> upstream (new)\n');
  });

  test('custom labels appear in the markers', () => {
    const plan = planFile(
      { base, theirs, ours: enc('uno\ntwo\nthree\nfour\nfive\n') },
      { ours: 'mine', base: 'old', theirs: 'new' },
    );
    const text = plan.action === 'conflict' && plan.content ? dec(plan.content) : '';
    expect(text).toContain('<<<<<<< mine');
    expect(text).toContain('>>>>>>> new');
  });

  test('added upstream', () => {
    const plan = planFile({ base: null, theirs, ours: null });
    expect(plan).toMatchObject({ action: 'write', why: 'added' });
  });

  test('added upstream while we already added a different file is a conflict that keeps ours', () => {
    expect(planFile({ base: null, theirs, ours: enc('ours') })).toEqual({
      action: 'conflict',
      why: 'added-both',
    });
  });

  test('edge: ours already equal to theirs is kept', () => {
    expect(planFile({ base: null, theirs, ours: theirs })).toEqual({ action: 'keep' });
    expect(planFile({ base, theirs, ours: theirs })).toEqual({ action: 'keep' });
  });

  test('removed upstream, pristine: delete', () => {
    expect(planFile({ base, theirs: null, ours: base })).toEqual({ action: 'delete' });
  });

  test('removed upstream but patched: conflict, keep ours', () => {
    expect(planFile({ base, theirs: null, ours: enc('patched') })).toEqual({
      action: 'conflict',
      why: 'removed-patched',
    });
  });

  test('edge: removed upstream and already gone here: keep', () => {
    expect(planFile({ base, theirs: null, ours: null })).toEqual({ action: 'keep' });
  });

  test('edge: changed upstream but deleted here: conflict', () => {
    expect(planFile({ base, theirs, ours: null })).toEqual({
      action: 'conflict',
      why: 'changed-deleted',
    });
  });
});

describe('checkDeps', () => {
  const pkg = (
    name: string,
    deps: Record<string, string> = {},
    dev: Record<string, string> = {},
  ) => ({
    name,
    dependencies: deps,
    devDependencies: dev,
  });

  test('refuses a workspace dependency on a package that is not vendored, naming it', () => {
    const r = checkDeps({
      packages: [
        {
          path: 'packages/core',
          target: pkg('@strudel/core', { '@strudel/edo': 'workspace:*' }),
          base: pkg('@strudel/core'),
        },
      ],
      rootCatalog: {},
    });
    expect(r.refusals).toHaveLength(1);
    expect(r.refusals[0]).toContain('packages/core');
    expect(r.refusals[0]).toContain('@strudel/edo');
  });

  test('refuses a workspace devDependency on an unvendored package too', () => {
    const r = checkDeps({
      packages: [
        {
          path: 'packages/core',
          target: pkg('@strudel/core', {}, { x: 'workspace:^' }),
          base: null,
        },
      ],
      rootCatalog: {},
    });
    expect(r.refusals).toHaveLength(1);
  });

  test('accepts workspace:* on a vendored name', () => {
    const core = pkg('@strudel/core');
    const mini = pkg('@strudel/mini', { '@strudel/core': 'workspace:*' });
    const r = checkDeps({
      packages: [
        { path: 'packages/core', target: core, base: core },
        { path: 'packages/mini', target: mini, base: mini },
      ],
      rootCatalog: {},
    });
    expect(r).toEqual({ refusals: [], warnings: [] });
  });

  test('warns for a catalog: dependency missing from the root catalog', () => {
    const cm = pkg('@strudel/codemirror', {
      '@codemirror/view': 'catalog:',
      '@codemirror/state': 'catalog:',
    });
    const r = checkDeps({
      packages: [{ path: 'packages/codemirror', target: cm, base: cm }],
      rootCatalog: { '@codemirror/state': '6.7.6' },
    });
    expect(r.refusals).toEqual([]);
    expect(r.warnings).toHaveLength(1);
    expect(r.warnings[0]).toContain('@codemirror/view');
    expect(r.warnings[0]).toContain('add an exact pin');
  });

  test('warns for a new external dependency', () => {
    const r = checkDeps({
      packages: [
        {
          path: 'packages/core',
          target: pkg('@strudel/core', { 'fraction.js': '^5.2.1', 'left-pad': '^1.0.0' }),
          base: pkg('@strudel/core', { 'fraction.js': '^5.2.1' }),
        },
      ],
      rootCatalog: {},
    });
    expect(r.refusals).toEqual([]);
    expect(r.warnings).toEqual([
      'packages/core: new dependency left-pad (^1.0.0); bun.lock freezes it',
    ]);
  });

  test('silent when nothing changed', () => {
    const p = pkg('@strudel/core', { 'fraction.js': '^5.2.1' }, { vite: '^6.0.11' });
    expect(
      checkDeps({ packages: [{ path: 'packages/core', target: p, base: p }], rootCatalog: {} }),
    ).toEqual({ refusals: [], warnings: [] });
  });

  test('edge: without a base (init) new dependencies are not reported', () => {
    const p = pkg('@strudel/core', { 'fraction.js': '^5.2.1' });
    expect(
      checkDeps({ packages: [{ path: 'packages/core', target: p, base: null }], rootCatalog: {} }),
    ).toEqual({ refusals: [], warnings: [] });
  });
});

describe('normalizeDocJson', () => {
  const raw = {
    docs: [
      { name: 'fast', meta: { filename: 'pattern.mjs', path: '/tmp/x/strudel/packages/core' } },
      { name: 'theme', meta: { path: '/tmp/x/strudel/packages/codemirror/themes' } },
      { kind: 'package', files: ['/tmp/x/strudel/packages/core/index.mjs'] },
      { name: 'nometa' },
    ],
  };

  test('rewrites absolute meta.path and files to the packages/ suffix', () => {
    const doc = JSON.parse(normalizeDocJson(raw));
    expect(doc.docs[0].meta).toEqual({ filename: 'pattern.mjs', path: 'packages/core' });
    expect(doc.docs[1].meta.path).toBe('packages/codemirror/themes');
    expect(doc.docs[2].files).toEqual(['packages/core/index.mjs']);
    expect(doc.docs[3]).toEqual({ name: 'nometa' });
  });

  test('a clone dir that itself contains packages/ still yields the last one', () => {
    const doc = JSON.parse(
      normalizeDocJson({ docs: [{ meta: { path: '/a/packages/tmp/strudel/packages/mini' } }] }),
    );
    expect(doc.docs[0].meta.path).toBe('packages/mini');
  });

  test('output is stable and ends with a newline', () => {
    const once = normalizeDocJson(raw);
    expect(once.endsWith('}\n')).toBe(true);
    expect(normalizeDocJson(JSON.parse(once))).toBe(once);
    expect(once).toBe(jsonText(JSON.parse(once)));
  });

  test('does not mutate its input', () => {
    const input = structuredClone(raw);
    normalizeDocJson(input);
    expect(input).toEqual(raw);
  });

  test('errors: no docs array, or a path outside packages/', () => {
    expect(() => normalizeDocJson({})).toThrow('no docs array');
    expect(() => normalizeDocJson(null)).toThrow('no docs array');
    expect(() => normalizeDocJson({ docs: [{ meta: { path: '/tmp/elsewhere' } }] })).toThrow(
      'outside packages/',
    );
  });
});

describe('parseArgs', () => {
  test('target, with and without --init', () => {
    expect(parseArgs(['main'])).toEqual({ init: false, target: 'main' });
    expect(parseArgs(['--init', 'abc'])).toEqual({ init: true, target: 'abc' });
    expect(parseArgs(['abc', '--init'])).toEqual({ init: true, target: 'abc' });
  });
  test('errors: no target, two targets, unknown flag', () => {
    for (const a of [[], ['a', 'b'], ['--force'], ['--init']])
      expect(typeof parseArgs(a)).toBe('string');
  });
});

test('DOC_PATHS are the eight packages without the Vite plugin', () => {
  expect(DOC_PATHS).toHaveLength(8);
  expect(VENDORED_PATHS).toEqual([...DOC_PATHS, 'packages/vite-plugin-bundle-audioworklet'].sort());
});

// ---------------------------------------------------------------------------------------------
// End to end over a local fixture upstream

const GIT_ENV = {
  ...process.env,
  GIT_AUTHOR_NAME: 'Fixture',
  GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
  GIT_COMMITTER_NAME: 'Fixture',
  GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
  // Nothing from the machine's git configuration leaks into the fixtures.
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: '/dev/null',
};

function git(cwd: string, ...args: string[]): string {
  const r = spawnSync('git', ['-c', 'commit.gpgsign=false', ...args], {
    cwd,
    env: GIT_ENV,
    encoding: 'utf8',
  });
  if (r.status !== 0) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
  return r.stdout.trim();
}

const PATHS = ['packages/core', 'packages/mini'];
const CORE_PKG = jsonText({ name: '@strudel/core', dependencies: { 'fraction.js': '^5.2.1' } });
const MINI_PKG = jsonText({
  name: '@strudel/mini',
  dependencies: { '@strudel/core': 'workspace:*' },
});
const LINES = 'one\ntwo\nthree\nfour\nfive\nsix\n';

let tmp = '';
let upstreamDir = '';
/** The first-commit upstream, built once. Tests that never commit read it directly. */
let sharedUpstream = '';
let sharedRoot = '';
let root = '';
let vendor = '';
let logs: string[] = [];

function put(dir: string, rel: string, text: string) {
  mkdirSync(dirname(join(dir, rel)), { recursive: true });
  writeFileSync(join(dir, rel), text);
}

/** Commits `files` (null deletes) to the repo at `dir` and returns the commit sha. */
function commitTo(dir: string, files: Record<string, string | null>, message: string): string {
  for (const [rel, text] of Object.entries(files)) {
    if (text === null) rmSync(join(dir, rel));
    else put(dir, rel, text);
  }
  git(dir, 'add', '-A');
  git(dir, 'commit', '--quiet', '--allow-empty', '-m', message);
  return git(dir, 'rev-parse', 'HEAD');
}

/** Commits to this test's own copy of the fixture upstream (made on the first commit). */
function commit(files: Record<string, string | null>, message = 'change'): string {
  if (upstreamDir === sharedUpstream) {
    upstreamDir = join(tmp, 'upstream');
    cpSync(sharedUpstream, upstreamDir, { recursive: true });
  }
  return commitTo(upstreamDir, files, message);
}

const fakeDocJson: SyncOptions['docJson'] = (clone, paths) => ({
  docs: paths.map((p) => ({ name: p, meta: { filename: 'index.mjs', path: join(clone, p) } })),
});

/** Commits everything in the fixture repo root, so the sync's clean-tree check passes. */
function commitRoot() {
  git(root, 'add', '-A');
  git(root, 'commit', '--quiet', '--allow-empty', '-m', 'local');
}

/** Runs a sync on the fixture root; unless `dirty`, the root's changes are committed first. */
function runSync(target: string, extra: Partial<SyncOptions> = {}, { dirty = false } = {}) {
  if (!dirty) commitRoot();
  return sync({
    root,
    target,
    init: false,
    repo: upstreamDir,
    paths: PATHS,
    docJson: fakeDocJson,
    log: (l) => logs.push(l),
    ...extra,
  });
}

const vread = (rel: string) => readFileSync(join(vendor, rel), 'utf8');
const vjson = (rel: string) => JSON.parse(vread(rel));
const sha = sha256;

function tree(dir: string, skip: string[] = []): Record<string, string> {
  if (!existsSync(dir)) return {};
  return Object.fromEntries(
    walkFiles(dir, { skip }).map((f) => [f, readFileSync(join(dir, f), 'utf8')] as const),
  );
}

/** `tree` of the fixture root without its `.git/`. */
const rootTree = () => tree(root, ['.git']);

const FIXTURE_PINS = {
  schema: 2,
  strudel: { repo: 'old', commit: '0'.repeat(40), date: '2000-01-01T00:00:00+00:00' },
  npm: { '@kabelsalat/web': { version: '0.4.1', file: 'dist/index.mjs', sha256: 'f'.repeat(64) } },
};

let first = '';

beforeAll(() => {
  sharedRoot = mkdtempSync(join(tmpdir(), 'strudel-sync-test-shared-'));
  sharedUpstream = join(sharedRoot, 'upstream');
  mkdirSync(sharedUpstream, { recursive: true });
  git(sharedUpstream, 'init', '--quiet', '-b', 'main');
  first = commitTo(
    sharedUpstream,
    {
      LICENSE: 'AGPL\n',
      'README.md': 'not vendored\n',
      'packages/core/package.json': CORE_PKG,
      'packages/core/index.mjs': LINES,
      'packages/core/repl.mjs': 'repl\n',
      'packages/core/test/pattern.test.mjs': 'test\n',
      'packages/core/bench/x.bench.mjs': 'bench\n',
      'packages/mini/package.json': MINI_PKG,
      'packages/mini/mini.mjs': 'mini\n',
      'packages/mini/mini.test.mjs': 'test\n',
      'packages/edo/package.json': jsonText({ name: '@strudel/edo' }),
    },
    'first',
  );
});

afterAll(() => {
  rmSync(sharedRoot, { recursive: true, force: true });
});

beforeEach(() => {
  tmp = mkdtempSync(join(tmpdir(), 'strudel-sync-test-'));
  upstreamDir = sharedUpstream;
  root = join(tmp, 'root');
  vendor = join(root, 'vendor/strudel');
  logs = [];
  put(root, 'package.json', jsonText({ name: 'fixture-root', private: true }));
  put(root, 'strudel-pins.json', jsonText(FIXTURE_PINS));
  git(root, 'init', '--quiet', '-b', 'main');
});

afterEach(() => {
  rmSync(tmp, { recursive: true, force: true });
});

describe('sync --init', () => {
  test('happy: vendors the paths, minus tests, and writes the derived files', () => {
    const calls: Upstream[] = [];
    const r = runSync(first, { init: true, pinsAndDocs: (_root, u) => calls.push(u) });
    expect(r.exitCode).toBe(0);
    expect(r.commit).toBe(first);
    expect(r.conflicts).toEqual([]);
    expect(Object.keys(tree(join(vendor, 'packages'))).sort()).toEqual([
      'core/index.mjs',
      'core/package.json',
      'core/repl.mjs',
      'mini/mini.mjs',
      'mini/package.json',
    ]);
    expect(vread('LICENSE')).toBe('AGPL\n');
    const upstream = vjson('UPSTREAM.json');
    expect(upstream).toEqual({
      repo: upstreamDir,
      commit: first,
      date: git(upstreamDir, 'log', '-1', '--format=%cI', first),
      paths: PATHS,
    });
    expect(Object.keys(upstream)).toEqual(['repo', 'commit', 'date', 'paths']);
    expect(vread('UPSTREAM.json')).toBe(jsonText(upstream));
    const manifest = vjson('upstream-manifest.json');
    expect(Object.keys(manifest)).toEqual(['schema', 'commit', 'files']);
    expect(manifest.schema).toBe(1);
    expect(manifest.commit).toBe(first);
    expect(manifest.files).toEqual({
      LICENSE: sha('AGPL\n'),
      'packages/core/index.mjs': sha(LINES),
      'packages/core/package.json': sha(CORE_PKG),
      'packages/core/repl.mjs': sha('repl\n'),
      'packages/mini/mini.mjs': sha('mini\n'),
      'packages/mini/package.json': sha(MINI_PKG),
    });
    expect(Object.keys(manifest.files)).toEqual(Object.keys(manifest.files).sort());
    expect(vjson('doc.json')).toEqual({
      docs: PATHS.map((p) => ({ name: p, meta: { filename: 'index.mjs', path: p } })),
    });
    expect(vread('PATCHES.md').startsWith('# ')).toBe(true);
    expect(calls).toEqual([upstream]);
    expect(logs.join('\n')).toContain('first');
    expect(logs.at(-1)).toBe(HOST_REMINDER);
    expect(logs.at(-2)).toBe(ENGINE_REMINDER);
    expect(HOST_REMINDER).toContain('bun run host:build');
    expect(ENGINE_REMINDER).toContain('bun run engine:vectors --write');
    expect(ENGINE_REMINDER).toContain('bun run engine:refs --write');
  });

  test('the docs producer gets the clone at the target and only documented paths', () => {
    let seen: { clone: string; paths: string[]; readme: boolean } | null = null;
    runSync(first, {
      init: true,
      paths: ['packages/core', 'packages/vite-plugin-bundle-audioworklet'],
      docJson: (clone, paths) => {
        seen = { clone, paths, readme: existsSync(join(clone, 'README.md')) };
        return { docs: [] };
      },
    });
    // The Vite plugin path does not exist in the fixture, so the run refuses before the docs step.
    expect(seen).toBeNull();
    const r = runSync(first, {
      init: true,
      docJson: (clone, paths) => {
        seen = { clone, paths, readme: existsSync(join(clone, 'README.md')) };
        return { docs: [] };
      },
    });
    expect(r.exitCode).toBe(0);
    expect(seen!.paths).toEqual(PATHS);
    expect(seen!.readme).toBe(true);
  });

  test('resolves a branch name', () => {
    const r = runSync('main', { init: true });
    expect(r.exitCode).toBe(0);
    expect(vjson('UPSTREAM.json').commit).toBe(first);
  });

  test('refuses a non-empty vendor/strudel', () => {
    put(vendor, 'stray', 'x');
    const r = runSync(first, { init: true });
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('--init');
  });

  test('refuses a vendored path with no package.json, writing nothing', () => {
    const r = runSync(first, { init: true, paths: ['packages/core', 'packages/nope'] });
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('packages/nope');
    expect(existsSync(vendor)).toBe(false);
  });
});

describe('sync', () => {
  beforeEach(() => {
    const r = runSync(first, { init: true });
    if (r.exitCode !== 0) throw new Error(r.refusals.join('\n'));
    logs = [];
  });

  test('refuses without UPSTREAM.json', () => {
    rmSync(join(vendor, 'UPSTREAM.json'));
    const r = runSync(first);
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('--init');
  });

  test('refuses an unknown target and a malformed one', () => {
    expect(runSync('no-such-branch').exitCode).toBe(EXIT_REFUSED);
    expect(runSync('0'.repeat(40)).exitCode).toBe(EXIT_REFUSED);
    const r = runSync('--upload-pack=evil');
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('not a commit or branch name');
  });

  test('refuses a malformed PATCHES.md before touching anything', () => {
    put(vendor, 'PATCHES.md', '## Bad Id: x\nUpstream: candidate\n');
    const before = tree(vendor);
    const r = runSync(commit({ 'packages/core/repl.mjs': 'repl 2\n' }));
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(tree(vendor)).toEqual(before);
  });

  test('refuses uncommitted changes to the vendored or derived files, writing nothing', () => {
    const next = commit({ 'packages/core/repl.mjs': 'repl 2\n' });
    commitRoot();
    for (const [rel, text] of [
      ['vendor/strudel/packages/core/index.mjs', 'edited\n'],
      ['vendor/strudel/packages/core/untracked.mjs', 'new\n'],
      ['strudel-pins.json', jsonText({ ...FIXTURE_PINS, edited: true })],
      ['crates/strudeland-cli/strudel-docs.json', '{}\n'],
    ] as const) {
      put(root, rel, text);
      const before = rootTree();
      const r = runSync(next, {}, { dirty: true });
      expect(r.exitCode).toBe(EXIT_REFUSED);
      expect(r.refusals[0]).toContain('uncommitted changes');
      expect(r.refusals[0]).toContain(rel);
      expect(rootTree()).toEqual(before);
      git(root, 'reset', '--quiet', '--hard');
      git(root, 'clean', '--quiet', '-fd');
    }
    // A change elsewhere in the repo does not count.
    put(root, 'notes.txt', 'x\n');
    expect(runSync(next, {}, { dirty: true }).exitCode).toBe(0);
  });

  test('refuses a vendored file with unresolved conflict markers, writing nothing', () => {
    put(vendor, 'packages/core/index.mjs', LINES.replace('one', 'uno'));
    const next = commit({ 'packages/core/index.mjs': LINES.replace('one', 'ONE') });
    expect(runSync(next).exitCode).toBe(EXIT_CONFLICT);
    // The markers get committed without being resolved.
    commitRoot();
    const before = rootTree();
    const r = runSync(commit({ 'packages/core/repl.mjs': 'repl 2\n' }));
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('conflict markers');
    expect(r.refusals[0]).toContain('vendor/strudel/packages/core/index.mjs');
    expect(rootTree()).toEqual(before);
    // A marker-like line that does not start a line is not a marker.
    put(vendor, 'packages/core/index.mjs', LINES + 'x <<<<<<< y\n');
    expect(runSync('main').exitCode).toBe(0);
  });

  test('refuses outside a git checkout', () => {
    rmSync(join(root, '.git'), { recursive: true, force: true });
    const r = runSync(first, {}, { dirty: true });
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals[0]).toContain('git status failed');
  });

  test('same commit again leaves the tree byte-identical', () => {
    const before = tree(vendor);
    const r = runSync(first);
    expect(r.exitCode).toBe(0);
    expect(r.changes).toEqual([]);
    expect(tree(vendor)).toEqual(before);
  });

  test('pristine files follow upstream: changed, added, removed; the derived files move', () => {
    const next = commit(
      {
        'packages/core/repl.mjs': 'repl 2\n',
        'packages/core/new.mjs': 'new\n',
        'packages/mini/mini.mjs': null,
        'packages/core/test/more.test.mjs': 'ignored\n',
        LICENSE: 'AGPL v2\n',
      },
      'second',
    );
    const r = runSync(next);
    expect(r.exitCode).toBe(0);
    expect(vread('packages/core/repl.mjs')).toBe('repl 2\n');
    expect(vread('packages/core/new.mjs')).toBe('new\n');
    expect(existsSync(join(vendor, 'packages/mini/mini.mjs'))).toBe(false);
    expect(existsSync(join(vendor, 'packages/core/test'))).toBe(false);
    expect(vread('LICENSE')).toBe('AGPL v2\n');
    expect(vjson('UPSTREAM.json').commit).toBe(next);
    const manifest = vjson('upstream-manifest.json');
    expect(manifest.commit).toBe(next);
    expect(manifest.files['packages/core/new.mjs']).toBe(sha('new\n'));
    expect(manifest.files['packages/mini/mini.mjs']).toBeUndefined();
    expect(manifest.files.LICENSE).toBe(sha('AGPL v2\n'));
    expect(r.changes).toEqual([
      'added packages/core/new.mjs',
      'theirs packages/core/repl.mjs',
      'deleted packages/mini/mini.mjs',
    ]);
    // The log of the range is printed for the commit body.
    expect(logs.some((l) => l.includes('second'))).toBe(true);
    expect(logs.some((l) => l.includes('first'))).toBe(false);
  });

  test('removing every file of a directory removes the directory', () => {
    commit({ 'packages/core/sub/a.mjs': 'a\n' });
    runSync('main');
    expect(existsSync(join(vendor, 'packages/core/sub/a.mjs'))).toBe(true);
    runSync(commit({ 'packages/core/sub/a.mjs': null }));
    expect(existsSync(join(vendor, 'packages/core/sub'))).toBe(false);
  });

  test('a patched file merges cleanly with an upstream change elsewhere in it', () => {
    put(vendor, 'packages/core/index.mjs', LINES.replace('six', 'SIX (patched)'));
    const r = runSync(commit({ 'packages/core/index.mjs': LINES.replace('one', 'ONE') }));
    expect(r.exitCode).toBe(0);
    expect(r.changes).toEqual(['merged packages/core/index.mjs']);
    expect(vread('packages/core/index.mjs')).toBe(
      LINES.replace('one', 'ONE').replace('six', 'SIX (patched)'),
    );
  });

  test('a conflicting merge writes markers, lists the file, and exits 1', () => {
    put(vendor, 'packages/core/index.mjs', LINES.replace('one', 'uno'));
    const next = commit({ 'packages/core/index.mjs': LINES.replace('one', 'ONE') });
    const r = runSync(next);
    expect(r.exitCode).toBe(EXIT_CONFLICT);
    expect(r.conflicts).toEqual(['packages/core/index.mjs: merge conflict, markers written']);
    const text = vread('packages/core/index.mjs');
    expect(text).toContain('<<<<<<< vendored\nuno\n');
    expect(text).toContain(`||||||| upstream ${first.slice(0, 7)}\none\n`);
    expect(text).toContain(`>>>>>>> upstream ${next.slice(0, 7)}\n`);
    // The derived files still move to the target.
    expect(vjson('UPSTREAM.json').commit).toBe(next);
    expect(logs.join('\n')).toContain('conflicts');
    expect(logs.at(-1)).toBe(HOST_REMINDER);
    expect(logs.at(-2)).toBe(ENGINE_REMINDER);
  });

  test('removed upstream but patched: kept, listed, exit 1', () => {
    put(vendor, 'packages/mini/mini.mjs', 'mini patched\n');
    const r = runSync(commit({ 'packages/mini/mini.mjs': null }));
    expect(r.exitCode).toBe(EXIT_CONFLICT);
    expect(r.conflicts).toEqual([
      'packages/mini/mini.mjs: removed upstream but patched; kept ours',
    ]);
    expect(vread('packages/mini/mini.mjs')).toBe('mini patched\n');
  });

  test('added upstream where we added a file: kept, listed, exit 1', () => {
    put(vendor, 'packages/core/rng.mjs', 'ours\n');
    const r = runSync(commit({ 'packages/core/rng.mjs': 'theirs\n' }));
    expect(r.exitCode).toBe(EXIT_CONFLICT);
    expect(r.conflicts[0]).toStartWith('packages/core/rng.mjs: added upstream');
    expect(vread('packages/core/rng.mjs')).toBe('ours\n');
  });

  test('files only in ours are never touched, and PATCHES.md is kept', () => {
    put(vendor, 'packages/core/rng.mjs', 'ours\n');
    const patches =
      '# P\n\n## rng: add rng\nUpstream: candidate\nx\n```files\npackages/core/rng.mjs\n```\n';
    put(vendor, 'PATCHES.md', patches);
    const r = runSync(commit({ 'packages/core/repl.mjs': 'repl 2\n' }));
    expect(r.exitCode).toBe(0);
    expect(vread('packages/core/rng.mjs')).toBe('ours\n');
    expect(vread('PATCHES.md')).toBe(patches);
  });

  test('refuses a workspace dependency on an unvendored package and writes nothing', () => {
    const before = tree(vendor);
    const r = runSync(
      commit({
        'packages/core/repl.mjs': 'repl 2\n',
        'packages/mini/package.json': jsonText({
          name: '@strudel/mini',
          dependencies: { '@strudel/core': 'workspace:*', '@strudel/edo': 'workspace:*' },
        }),
      }),
    );
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals.join('\n')).toContain('@strudel/edo');
    expect(tree(vendor)).toEqual(before);
  });

  test('warns about a new dependency and a catalog entry missing from the root', () => {
    const r = runSync(
      commit({
        'packages/core/package.json': jsonText({
          name: '@strudel/core',
          dependencies: {
            'fraction.js': '^5.2.1',
            'left-pad': '^1.0.0',
            '@codemirror/view': 'catalog:',
          },
        }),
      }),
    );
    expect(r.exitCode).toBe(0);
    expect(r.warnings).toHaveLength(3);
    put(
      root,
      'package.json',
      jsonText({ workspaces: { packages: [], catalog: { '@codemirror/view': '6.43.15' } } }),
    );
    // Same target again: the catalog entry now resolves, and nothing is new against the base.
    expect(runSync('main').warnings).toEqual([]);
  });

  test('a jsdoc failure writes nothing', () => {
    const before = tree(vendor);
    expect(() =>
      runSync(commit({ 'packages/core/repl.mjs': 'repl 2\n' }), {
        docJson: () => {
          throw new Error('jsdoc broke');
        },
      }),
    ).toThrow('jsdoc broke');
    expect(tree(vendor)).toEqual(before);
  });

  test('the temp clone is removed afterwards', () => {
    const scratch = () =>
      readdirSync(tmpdir()).filter((n) => n.startsWith('strudel-sync-') && !n.includes('test'));
    const before = scratch();
    runSync(first);
    expect(scratch()).toEqual(before);
  });
});

// ---------------------------------------------------------------------------------------------
// strudel-pins.json and strudel-docs.json

describe('pins and docs', () => {
  const rread = (rel: string) => readFileSync(join(root, rel), 'utf8');
  const UP: Upstream = {
    repo: 'https://example.org/strudel',
    commit: 'b'.repeat(40),
    date: '2026-01-02T03:04:05+01:00',
    paths: PATHS,
  };

  test('updatePins replaces the strudel block, keeps npm, and orders the keys', () => {
    const next = updatePins(FIXTURE_PINS as any, UP);
    expect(next).toEqual({
      schema: 2,
      strudel: { repo: UP.repo, commit: UP.commit, date: UP.date },
      npm: FIXTURE_PINS.npm,
    });
    expect(Object.keys(next)).toEqual(['schema', 'strudel', 'npm']);
    expect(Object.keys(next.strudel)).toEqual(['repo', 'commit', 'date']);
  });

  test('readPins: happy, and errors for a missing file, bad JSON, schema 1 and no npm block', () => {
    expect(readPins(root).npm).toEqual(FIXTURE_PINS.npm);
    put(root, 'strudel-pins.json', '{ nope');
    expect(() => readPins(root)).toThrow('strudel-pins.json:');
    put(root, 'strudel-pins.json', jsonText({ schema: 1, packages: {} }));
    expect(() => readPins(root)).toThrow('expected schema 2, found 1');
    put(root, 'strudel-pins.json', jsonText({ schema: 2, strudel: {} }));
    expect(() => readPins(root)).toThrow('no npm block');
    rmSync(join(root, 'strudel-pins.json'));
    expect(() => readPins(root)).toThrow('strudel-pins.json is missing');
  });

  test('writePinsAndDocs writes the pins block and the docs file from vendor/strudel', () => {
    put(root, 'vendor/strudel/UPSTREAM.json', jsonText(UP));
    put(
      root,
      'vendor/strudel/doc.json',
      jsonText({
        docs: [
          { name: 'fast', synonyms: ['density'] },
          { name: 'hidden', access: 'private' },
        ],
      }),
    );
    writePinsAndDocs(root, UP);
    expect(rread('strudel-pins.json')).toBe(jsonText(updatePins(FIXTURE_PINS as any, UP)));
    const docs = JSON.parse(rread('crates/strudeland-cli/strudel-docs.json'));
    expect(docs.schema).toBe(2);
    expect(docs.source).toEqual({ repo: UP.repo, commit: UP.commit });
    expect(docs.entries.map((e: { name: string }) => e.name)).toEqual(['fast']);
  });

  test('sync updates the pins block and regenerates the docs file', () => {
    expect(runSync(first, { init: true }).exitCode).toBe(0);
    const upstream = vjson('UPSTREAM.json');
    const pins = JSON.parse(rread('strudel-pins.json'));
    expect(pins.strudel).toEqual({
      repo: upstream.repo,
      commit: first,
      date: upstream.date,
    });
    expect(pins.npm).toEqual(FIXTURE_PINS.npm);
    const docs = JSON.parse(rread('crates/strudeland-cli/strudel-docs.json'));
    expect(docs.source).toEqual({ repo: upstream.repo, commit: first });
    expect(docs.entries.map((e: { name: string }) => e.name)).toEqual(PATHS);

    // A later sync moves both to the new commit.
    const next = commit({ 'packages/core/repl.mjs': 'repl 2\n' });
    expect(runSync(next).exitCode).toBe(0);
    expect(JSON.parse(rread('strudel-pins.json')).strudel.commit).toBe(next);
    expect(JSON.parse(rread('crates/strudeland-cli/strudel-docs.json')).source.commit).toBe(next);
  });

  test('error: sync refuses, writing nothing, when strudel-pins.json is not schema 2', () => {
    put(root, 'strudel-pins.json', jsonText({ schema: 1, packages: {} }));
    const r = runSync(first, { init: true });
    expect(r.exitCode).toBe(EXIT_REFUSED);
    expect(r.refusals).toEqual(['strudel-pins.json: expected schema 2, found 1']);
    expect(existsSync(vendor)).toBe(false);
    expect(existsSync(join(root, 'crates'))).toBe(false);
  });

  test('an injected pinsAndDocs skips the pins check', () => {
    rmSync(join(root, 'strudel-pins.json'));
    expect(runSync(first, { init: true, pinsAndDocs: () => {} }).exitCode).toBe(0);
  });
});

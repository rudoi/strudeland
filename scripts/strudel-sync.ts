// Moves vendor/strudel/ to another upstream Strudel commit with a 3-way merge, so local patches
// survive, and regenerates the files derived from upstream (doc.json, upstream-manifest.json,
// UPSTREAM.json, LICENSE). Never commits and never runs upstream's install or build scripts.
//
//   bun run strudel:sync <commit|branch|main>    sync from the commit in UPSTREAM.json
//   bun run strudel:sync --init <commit|branch>  create vendor/strudel/ from scratch
//
// Exit codes: 0 done, 1 conflicts (files written with markers or kept, listed), 2 refused (nothing
// written; also for uncommitted changes to vendor/strudel/ or the derived files, and for conflict
// markers left from an earlier sync).
import { spawnSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { DOCS_FILE, extractVendored, renderDocs } from './extract-strudel-docs';
import { jsonText } from './json-text';
import { sha256 } from './sha256';
import { parsePatches } from './strudel-patches';

const UPSTREAM_REPO = 'https://codeberg.org/uzu/strudel';
/** Upstream paths vendored by `--init`, sorted. A sync uses `UPSTREAM.json`'s `paths`. */
export const VENDORED_PATHS = [
  'packages/codemirror',
  'packages/core',
  'packages/draw',
  'packages/mini',
  'packages/superdough',
  'packages/tonal',
  'packages/transpiler',
  'packages/vite-plugin-bundle-audioworklet',
  'packages/webaudio',
];
/** Paths jsdoc documents into doc.json: the vendored packages, without the Vite plugin. */
export const DOC_PATHS = VENDORED_PATHS.filter(
  (p) => !p.endsWith('/vite-plugin-bundle-audioworklet'),
);
const JSDOC_PACKAGES = ['jsdoc@4.0.4', 'jsdoc-json@2.0.2'];
export const EXIT_CONFLICT = 1;
export const EXIT_REFUSED = 2;

export type Upstream = { repo: string; commit: string; date: string; paths: string[] };
type Manifest = { schema: 1; commit: string; files: Record<string, string> };

const PATCHES_TEMPLATE = `# Local patches to the vendored Strudel

Each entry is one local change to \`vendor/strudel/\`: what it does, whether to send it upstream, and
every file it modifies or adds (paths relative to \`vendor/strudel/\`). The integrity test fails if a
vendored file differs from \`upstream-manifest.json\` without being listed here.
`;

// ---------------------------------------------------------------------------------------------
// Pure helpers

const EXCLUDED_DIRS = new Set(['test', 'bench', '__tests__', 'node_modules', 'dist']);
const EXCLUDED_FILE = /\.(test|bench)\.m?js$/;

/** Whether a file (path relative to upstream's `packages/`) is vendored. */
export function isVendoredPath(relPath: string): boolean {
  const parts = relPath.split('/');
  const base = parts.pop() ?? '';
  if (parts.some((d) => EXCLUDED_DIRS.has(d))) return false;
  return !EXCLUDED_FILE.test(base);
}

type FileVersions = {
  base: Uint8Array | null;
  theirs: Uint8Array | null;
  ours: Uint8Array | null;
};

type FilePlan =
  | { action: 'keep' }
  | { action: 'write'; content: Uint8Array; why: 'theirs' | 'added' | 'merged' }
  | { action: 'delete' }
  | {
      action: 'conflict';
      why: 'merge' | 'added-both' | 'removed-patched' | 'changed-deleted';
      /** Content to write (the merge result with markers); absent means keep ours. */
      content?: Uint8Array;
    };

const same = (a: Uint8Array | null, b: Uint8Array | null): boolean =>
  a === null || b === null ? a === b : Buffer.from(a).equals(Buffer.from(b));

/**
 * Decides what a sync does with one file, given its upstream version at the old commit (`base`),
 * at the target (`theirs`) and in `vendor/` (`ours`); null means absent. Patched files are merged
 * with `git merge-file`.
 */
export function planFile(
  v: FileVersions,
  labels: { ours: string; base: string; theirs: string } = {
    ours: 'vendored',
    base: 'upstream (old)',
    theirs: 'upstream (new)',
  },
): FilePlan {
  const { base, theirs, ours } = v;
  if (same(base, theirs)) return { action: 'keep' };
  if (same(ours, theirs)) return { action: 'keep' };
  if (base === null) {
    // theirs is present: upstream added the file.
    if (ours === null) return { action: 'write', content: theirs!, why: 'added' };
    return { action: 'conflict', why: 'added-both' };
  }
  if (theirs === null) {
    if (ours === null) return { action: 'keep' };
    if (same(ours, base)) return { action: 'delete' };
    return { action: 'conflict', why: 'removed-patched' };
  }
  if (ours === null) return { action: 'conflict', why: 'changed-deleted' };
  if (same(ours, base)) return { action: 'write', content: theirs, why: 'theirs' };
  const merged = mergeFile(ours, base, theirs, labels);
  return merged.conflicts
    ? { action: 'conflict', why: 'merge', content: merged.content }
    : { action: 'write', content: merged.content, why: 'merged' };
}

function mergeFile(
  ours: Uint8Array,
  base: Uint8Array,
  theirs: Uint8Array,
  labels: { ours: string; base: string; theirs: string },
): { content: Uint8Array; conflicts: boolean } {
  const dir = mkdtempSync(join(tmpdir(), 'strudel-merge-'));
  try {
    const files = { ours, base, theirs };
    for (const [name, data] of Object.entries(files)) writeFileSync(join(dir, name), data);
    const r = spawnSync(
      'git',
      [
        'merge-file',
        '-p',
        '--diff3',
        '-L',
        labels.ours,
        '-L',
        labels.base,
        '-L',
        labels.theirs,
        join(dir, 'ours'),
        join(dir, 'base'),
        join(dir, 'theirs'),
      ],
      { maxBuffer: 1 << 28 },
    );
    if (r.error) throw r.error;
    // The status is the number of conflicts (capped at 127); a negative value (255) is an error.
    if (r.status === null || r.status > 127)
      throw new Error(`git merge-file failed: ${r.stderr.toString().trim()}`);
    return { content: new Uint8Array(r.stdout), conflicts: r.status > 0 };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

type PackageJson = {
  name?: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
};

const DEP_FIELDS = [
  'dependencies',
  'devDependencies',
  'peerDependencies',
  'optionalDependencies',
] as const;

function allDeps(pkg: PackageJson | null): Map<string, string> {
  const out = new Map<string, string>();
  for (const field of DEP_FIELDS)
    for (const [name, spec] of Object.entries(pkg?.[field] ?? {}))
      if (!out.has(name)) out.set(name, spec);
  return out;
}

type DepsInput = {
  /** One per vendored package: its path, its target `package.json`, and its base one (null on init). */
  packages: { path: string; target: PackageJson; base: PackageJson | null }[];
  /** The root `package.json` catalog (`catalog` or `workspaces.catalog`). */
  rootCatalog: Record<string, string>;
};

/**
 * Checks the target packages' dependencies. A `workspace:` dependency on a package that is not
 * vendored is a refusal. A `catalog:` dependency missing from the root catalog, and a dependency
 * the base `package.json` did not have, are warnings.
 */
export function checkDeps(input: DepsInput): { refusals: string[]; warnings: string[] } {
  const vendored = new Set(input.packages.map((p) => p.target.name).filter(Boolean));
  const refusals: string[] = [];
  const warnings: string[] = [];
  for (const { path, target, base } of input.packages) {
    const before = base === null ? null : allDeps(base);
    for (const [name, spec] of allDeps(target)) {
      if (spec.startsWith('workspace:')) {
        if (!vendored.has(name))
          refusals.push(`${path}: depends on ${name} (${spec}), which is not vendored`);
        continue;
      }
      if (spec.startsWith('catalog:') && !(name in input.rootCatalog))
        warnings.push(
          `${path}: ${name} is "${spec}" but the root package.json catalog has no ${name}; add an exact pin`,
        );
      if (before !== null && !before.has(name))
        warnings.push(`${path}: new dependency ${name} (${spec}); bun.lock freezes it`);
    }
  }
  return { refusals, warnings };
}

/** Rewrites an absolute jsdoc path to its part from `packages/` on. */
function relativeDocPath(p: string): string {
  const posix = p.replace(/\\/g, '/');
  if (posix === 'packages' || posix.startsWith('packages/')) return posix;
  const i = posix.lastIndexOf('/packages/');
  if (i < 0) throw new Error(`doc.json path outside packages/: ${p}`);
  return posix.slice(i + 1);
}

/**
 * Normalizes jsdoc-json output so it does not depend on where the clone was: every entry's
 * `meta.path` and `files` paths become relative (`packages/...`). Returns the file text (2-space
 * indent, final newline).
 */
export function normalizeDocJson(doc: unknown): string {
  if (typeof doc !== 'object' || doc === null || !Array.isArray((doc as { docs?: unknown }).docs))
    throw new Error('doc.json has no docs array');
  const docs = (doc as { docs: Record<string, unknown>[] }).docs.map((entry) => {
    const out = { ...entry };
    const meta = out.meta as { path?: unknown } | undefined;
    if (meta && typeof meta.path === 'string')
      out.meta = { ...meta, path: relativeDocPath(meta.path) };
    if (Array.isArray(out.files))
      out.files = out.files.map((f) => (typeof f === 'string' ? relativeDocPath(f) : f));
    return out;
  });
  return jsonText({ ...(doc as object), docs });
}

/** Code-unit order, which is what `Array.prototype.sort` uses without a comparator. */
const sorted = (xs: Iterable<string>): string[] => [...xs].sort();

// ---------------------------------------------------------------------------------------------
// Git and file system

/** Spawns without throwing; `stdout` is null when the command could not run or exited non-zero. */
function spawn(cmd: string, args: string[], cwd?: string) {
  const r = spawnSync(cmd, args, { cwd, maxBuffer: 1 << 28, encoding: 'utf8' });
  return { ...r, stdout: r.error || r.status !== 0 ? null : r.stdout };
}

function run(cmd: string, args: string[], cwd?: string): string {
  const r = spawn(cmd, args, cwd);
  if (r.error) throw r.error;
  if (r.stdout === null)
    throw new Error(`${cmd} ${args.join(' ')} failed (${r.status}): ${r.stderr.trim()}`);
  return r.stdout;
}

const git = (cwd: string, ...args: string[]) => run('git', args, cwd);

function tryGit(cwd: string, ...args: string[]): string | null {
  return spawn('git', args, cwd).stdout?.trim() ?? null;
}

/** Resolves a sha, branch or tag to a full commit sha in `clone`, or null. */
function resolveCommit(clone: string, target: string): string | null {
  for (const ref of [target, `origin/${target}`]) {
    const sha = tryGit(clone, 'rev-parse', '--verify', '--quiet', `${ref}^{commit}`);
    if (sha) return sha;
  }
  return null;
}

/** Repo-relative paths of the vendored files under `path` (e.g. `packages/core`) in a checkout. */
function listVendored(checkout: string, path: string): string[] {
  return git(checkout, 'ls-files', '-z', '--', path)
    .split('\0')
    .filter((f) => f !== '' && isVendoredPath(f.slice('packages/'.length)));
}

/** The files a sync writes in the repo; they must be committed before it runs. */
const WRITTEN_PATHS = ['vendor/strudel', 'strudel-pins.json', DOCS_FILE];

/**
 * Why the repo at `root` is not ready for a sync, or null: uncommitted changes to the files a sync
 * writes (so a sync never mixes with local edits), or unresolved conflict markers from an earlier
 * sync in a vendored file.
 */
export function treeProblem(root: string): string | null {
  const status = spawn('git', ['status', '--porcelain', '--', ...WRITTEN_PATHS], root).stdout;
  if (status === null) return `git status failed in ${root}; run the sync in a git checkout`;
  if (status.trim() !== '')
    return `uncommitted changes; commit or stash them first:\n${status.trimEnd()}`;
  const marked = git(root, 'ls-files', '-z', '--', 'vendor/strudel')
    .split('\0')
    .filter((f) => f !== '' && /^<{7} /m.test(readFileSync(join(root, f), 'utf8')));
  if (marked.length > 0)
    return `unresolved conflict markers; resolve and commit them first:\n${marked.join('\n')}`;
  return null;
}

const readOrNull = (p: string): Uint8Array | null =>
  existsSync(p) ? new Uint8Array(readFileSync(p)) : null;

function removeEmptyDirs(from: string, stopAt: string): void {
  let dir = from;
  while (dir.startsWith(stopAt + '/') && existsSync(dir) && readdirSync(dir).length === 0) {
    rmdirSync(dir);
    dir = dirname(dir);
  }
}

function writeFile(p: string, data: Uint8Array | string): void {
  mkdirSync(dirname(p), { recursive: true });
  writeFileSync(p, data);
}

/** Runs jsdoc over `paths` in `clone` and returns the parsed jsdoc-json output. */
function runJsdoc(
  clone: string,
  paths: string[],
  scratch: string,
  log: (line: string) => void = () => {},
): unknown {
  const out = join(clone, 'doc.json');
  const jsdocArgs = (template: string) => [
    ...paths,
    '--template',
    template,
    '--destination',
    'doc.json',
    '-c',
    'jsdoc/jsdoc.config.json',
  ];
  const add = (cwd: string) =>
    spawnSync('bun', ['add', '--ignore-scripts', ...JSDOC_PACKAGES], { cwd, encoding: 'utf8' });
  if (add(clone).status === 0) {
    log(`jsdoc installed in the clone`);
    run(join(clone, 'node_modules/.bin/jsdoc'), jsdocArgs('./node_modules/jsdoc-json'), clone);
  } else {
    // `bun add` fails in the pnpm workspace (workspace: specifiers); install into an empty dir.
    const tools = join(scratch, 'jsdoc');
    mkdirSync(tools, { recursive: true });
    writeFileSync(join(tools, 'package.json'), '{ "private": true }\n');
    const r = add(tools);
    if (r.status !== 0) throw new Error(`bun add ${JSDOC_PACKAGES.join(' ')} failed: ${r.stderr}`);
    log(`jsdoc installed in a separate directory (bun add failed in the clone)`);
    run(
      join(tools, 'node_modules/.bin/jsdoc'),
      jsdocArgs(join(tools, 'node_modules/jsdoc-json')),
      clone,
    );
  }
  return JSON.parse(readFileSync(out, 'utf8'));
}

type PinsFile = {
  schema: 2;
  strudel: { repo: string; commit: string; date: string };
  npm: Record<string, { version: string; file: string; sha256: string }>;
};

/** Reads `<root>/strudel-pins.json`; throws unless it is a schema 2 file with an `npm` block. */
export function readPins(root: string): PinsFile {
  const path = join(root, 'strudel-pins.json');
  if (!existsSync(path)) throw new Error('strudel-pins.json is missing');
  let pins: PinsFile;
  try {
    pins = JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    throw new Error(`strudel-pins.json: ${(e as Error).message}`);
  }
  if (pins?.schema !== 2)
    throw new Error(`strudel-pins.json: expected schema 2, found ${String(pins?.schema)}`);
  if (typeof pins.npm !== 'object' || pins.npm === null)
    throw new Error('strudel-pins.json: no npm block');
  return pins;
}

/** `pins` with its `strudel` block set to `upstream`'s repo, commit and date; `npm` kept. */
export function updatePins(pins: PinsFile, upstream: Upstream): PinsFile {
  return {
    schema: 2,
    strudel: { repo: upstream.repo, commit: upstream.commit, date: upstream.date },
    npm: pins.npm,
  };
}

/**
 * Writes the repo-level files that record the vendored commit: the `strudel` block of
 * `strudel-pins.json` and the CLI's `strudel-docs.json` (what `bun run docs:extract` writes), both
 * from the already written `vendor/strudel/` files.
 */
export function writePinsAndDocs(root: string, upstream: Upstream): void {
  writeFile(join(root, 'strudel-pins.json'), jsonText(updatePins(readPins(root), upstream)));
  writeFile(join(root, DOCS_FILE), renderDocs(extractVendored(root)));
}

// ---------------------------------------------------------------------------------------------
// The sync

export type SyncOptions = {
  /** Repo root (the directory holding `vendor/`), a git checkout. */
  root: string;
  target: string;
  init: boolean;
  /** Upstream clone URL or local path. */
  repo?: string;
  /** Paths to vendor on `--init`; a sync uses `UPSTREAM.json`'s. */
  paths?: string[];
  /** Produces raw jsdoc-json output for the target checkout. */
  docJson?: (
    clone: string,
    paths: string[],
    scratch: string,
    log: (line: string) => void,
  ) => unknown;
  /** Writes repo-level files derived from `UPSTREAM.json`. */
  pinsAndDocs?: (root: string, upstream: Upstream) => void;
  log?: (line: string) => void;
};

type SyncResult = {
  exitCode: 0 | 1 | 2;
  commit?: string;
  /** Files that need a manual resolution, `<path>: <why>`. */
  conflicts: string[];
  /** Files written or deleted, `<action> <path>`. */
  changes: string[];
  warnings: string[];
  refusals: string[];
};

/** Printed after every sync: the Rust engine's JS vectors and Chromium references come from
 *  the vendored superdough and must be regenerated (and reviewed) with it. */
export const ENGINE_REMINDER =
  'then regenerate the engine artifacts: run `bun run engine:vectors --write` and `bun run engine:refs --write`, and review their summaries';

/** Printed right after `ENGINE_REMINDER`: the headless host bundle is built from the vendored
 *  Strudel and must be rebuilt (and committed) with it. */
export const HOST_REMINDER =
  'then rebuild the headless host bundle: run `bun run host:build` and commit `crates/strudeland-host/js/`';

export function sync(opts: SyncOptions): SyncResult {
  const log = opts.log ?? ((l: string) => console.log(l));
  const result: SyncResult = {
    exitCode: 0,
    conflicts: [],
    changes: [],
    warnings: [],
    refusals: [],
  };
  const refuse = (...msgs: string[]): SyncResult => {
    result.refusals.push(...msgs);
    result.exitCode = EXIT_REFUSED;
    for (const r of result.refusals) log(`refused: ${r}`);
    return result;
  };
  const vendor = join(opts.root, 'vendor/strudel');
  const upstreamFile = join(vendor, 'UPSTREAM.json');
  const patchesFile = join(vendor, 'PATCHES.md');

  if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(opts.target))
    return refuse(`not a commit or branch name: ${opts.target}`);
  const dirty = treeProblem(opts.root);
  if (dirty) return refuse(dirty);
  let old: Upstream | null = null;
  if (opts.init) {
    if (existsSync(vendor) && readdirSync(vendor).length > 0)
      return refuse('--init needs an empty or missing vendor/strudel/');
  } else {
    if (!existsSync(upstreamFile))
      return refuse('vendor/strudel/UPSTREAM.json is missing; create the snapshot with --init');
    old = JSON.parse(readFileSync(upstreamFile, 'utf8')) as Upstream;
    if (existsSync(patchesFile)) {
      try {
        parsePatches(readFileSync(patchesFile, 'utf8'));
      } catch (e) {
        return refuse((e as Error).message);
      }
    }
  }
  const repo = opts.repo ?? old?.repo ?? UPSTREAM_REPO;
  const paths = sorted(old?.paths ?? opts.paths ?? VENDORED_PATHS);

  const scratch = mkdtempSync(join(tmpdir(), 'strudel-sync-'));
  try {
    const clone = join(scratch, 'upstream');
    log(`cloning ${repo}`);
    run('git', ['clone', '--quiet', '--filter=blob:none', '--no-checkout', repo, clone]);
    const commit = resolveCommit(clone, opts.target);
    if (!commit) return refuse(`${opts.target} is not a commit or branch of ${repo}`);
    git(clone, 'checkout', '--quiet', '--detach', commit);
    let baseCheckout: string | null = null;
    if (old) {
      if (tryGit(clone, 'cat-file', '-e', `${old.commit}^{commit}`) === null)
        return refuse(`the vendored commit ${old.commit} is not in ${repo}`);
      if (old.commit === commit) baseCheckout = clone;
      else {
        baseCheckout = join(scratch, 'base');
        git(clone, 'worktree', 'add', '--quiet', '--detach', baseCheckout, old.commit);
      }
    }

    // Dependencies first: a refusal writes nothing.
    const pkgs: DepsInput['packages'] = [];
    for (const path of paths) {
      const target = readOrNull(join(clone, path, 'package.json'));
      if (!target) return refuse(`${path}: no package.json at ${commit}`);
      const base = baseCheckout ? readOrNull(join(baseCheckout, path, 'package.json')) : null;
      const parse = (b: Uint8Array) => JSON.parse(Buffer.from(b).toString('utf8')) as PackageJson;
      pkgs.push({ path, target: parse(target), base: base ? parse(base) : null });
    }
    // The default pins writer needs a valid strudel-pins.json; check it before writing anything.
    if (!opts.pinsAndDocs) {
      try {
        readPins(opts.root);
      } catch (e) {
        return refuse((e as Error).message);
      }
    }
    const rootPkg = JSON.parse(readFileSync(join(opts.root, 'package.json'), 'utf8'));
    const rootCatalog = { ...rootPkg.workspaces?.catalog, ...rootPkg.catalog };
    const deps = checkDeps({ packages: pkgs, rootCatalog });
    result.warnings.push(...deps.warnings);
    if (deps.refusals.length > 0) return refuse(...deps.refusals);

    // Plan every file before writing anything.
    const theirsFiles = new Set(paths.flatMap((p) => listVendored(clone, p)));
    const baseFiles = new Set(
      baseCheckout ? paths.flatMap((p) => listVendored(baseCheckout, p)) : [],
    );
    const short = (sha: string) => sha.slice(0, 7);
    const labels = {
      ours: 'vendored',
      base: `upstream ${old ? short(old.commit) : ''}`.trim(),
      theirs: `upstream ${short(commit)}`,
    };
    const plans: [string, FilePlan][] = [];
    for (const file of sorted(new Set([...theirsFiles, ...baseFiles]))) {
      const versions = {
        base: baseFiles.has(file) ? readOrNull(join(baseCheckout!, file)) : null,
        theirs: theirsFiles.has(file) ? readOrNull(join(clone, file)) : null,
        ours: readOrNull(join(vendor, file)),
      };
      plans.push([file, planFile(versions, labels)]);
    }

    const license = readOrNull(join(clone, 'LICENSE'));
    if (!license) return refuse(`no LICENSE at the root of ${repo} at ${commit}`);
    log('generating doc.json');
    const docText = normalizeDocJson(
      (opts.docJson ?? runJsdoc)(
        clone,
        DOC_PATHS.filter((p) => paths.includes(p)),
        scratch,
        log,
      ),
    );
    const files: Record<string, string> = { LICENSE: sha256(license) };
    for (const file of theirsFiles) files[file] = sha256(readFileSync(join(clone, file)));
    const manifest: Manifest = {
      schema: 1,
      commit,
      files: Object.fromEntries(sorted(Object.keys(files)).map((k) => [k, files[k]])),
    };
    const upstream: Upstream = {
      repo,
      commit,
      date: git(clone, 'log', '-1', '--format=%cI', commit).trim(),
      paths,
    };

    for (const [file, plan] of plans) {
      const dest = join(vendor, file);
      if (plan.action === 'write') {
        writeFile(dest, plan.content);
        result.changes.push(`${plan.why} ${file}`);
      } else if (plan.action === 'delete') {
        rmSync(dest);
        removeEmptyDirs(dirname(dest), vendor);
        result.changes.push(`deleted ${file}`);
      } else if (plan.action === 'conflict') {
        if (plan.content) writeFile(dest, plan.content);
        result.conflicts.push(`${file}: ${CONFLICT_TEXT[plan.why]}`);
      }
    }
    writeFile(join(vendor, 'LICENSE'), license);
    writeFile(join(vendor, 'doc.json'), docText);
    writeFile(join(vendor, 'upstream-manifest.json'), jsonText(manifest));
    writeFile(upstreamFile, jsonText(upstream));
    if (!existsSync(patchesFile)) writeFile(patchesFile, PATCHES_TEMPLATE);
    (opts.pinsAndDocs ?? writePinsAndDocs)(opts.root, upstream);

    result.commit = commit;
    for (const c of result.changes) log(c);
    for (const w of result.warnings) log(`warning: ${w}`);
    log('');
    log(
      old && old.commit !== commit
        ? `upstream ${short(old.commit)}..${short(commit)}:`
        : `upstream ${short(commit)}:`,
    );
    const range = old && old.commit !== commit ? [`${old.commit}..${commit}`] : ['-1', commit];
    log(git(clone, 'log', '--oneline', ...range).trimEnd());
    if (result.conflicts.length > 0) {
      result.exitCode = EXIT_CONFLICT;
      log('');
      log('conflicts (resolve, then update PATCHES.md):');
      for (const c of result.conflicts) log(`  ${c}`);
    }
    log('');
    log(ENGINE_REMINDER);
    log(HOST_REMINDER);
    return result;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

const CONFLICT_TEXT: Record<Extract<FilePlan, { action: 'conflict' }>['why'], string> = {
  merge: 'merge conflict, markers written',
  'added-both': 'added upstream, but a vendored file already exists; kept ours',
  'removed-patched': 'removed upstream but patched; kept ours',
  'changed-deleted': 'changed upstream but deleted here; left deleted',
};

export function parseArgs(argv: string[]): { init: boolean; target: string } | string {
  const init = argv.includes('--init');
  const rest = argv.filter((a) => a !== '--init');
  if (rest.length !== 1 || rest[0].startsWith('-'))
    return 'usage: bun run strudel:sync [--init] <commit|branch|main>';
  return { init, target: rest[0] };
}

if (import.meta.main) {
  const args = parseArgs(process.argv.slice(2));
  if (typeof args === 'string') {
    console.error(args);
    process.exit(EXIT_REFUSED);
  }
  const root = join(import.meta.dir, '..');
  const r = sync({ root, ...args });
  process.exit(r.exitCode);
}

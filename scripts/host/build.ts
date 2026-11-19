/**
 * `bun run host:build`: bundles the headless host (`src/host/main.ts`) into one unminified IIFE,
 * `crates/strudeland-host/js/host.js`, and writes `host.inputs.json`: the repo-relative realpath
 * and sha256 of every module the bundler read, keys in byte order, plus `combined` (the sha256 of
 * `<path>\n<sha256>\n` per file, in key order). `tests/host-bundle.test.ts` recomputes the hashes
 * from disk, so a stale bundle fails `bun test`. Fails if a superdough, webaudio, codemirror,
 * svelte or `src/lib/audio/` module is reachable from the entry.
 */

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { errorMessage } from '../../src/lib/errors';
import { byteOrder, jsonText, sortedKeys } from '../json-text';
import { sha256 } from '../sha256';

export const REPO_ROOT = resolve(import.meta.dir, '../..');
export const ENTRY = 'src/host/main.ts';
export const HOST_JS_DIR = 'crates/strudeland-host/js';
export const HOST_JS = `${HOST_JS_DIR}/host.js`;
export const HOST_INPUTS = `${HOST_JS_DIR}/host.inputs.json`;

/** Modules that must not be reachable from the entry (repo-relative path prefixes). */
export const FORBIDDEN_PREFIXES = [
  'node_modules/superdough',
  'vendor/strudel/packages/superdough',
  'vendor/strudel/packages/webaudio',
  'vendor/strudel/packages/codemirror',
  'node_modules/@codemirror',
  'node_modules/svelte/',
  'src/lib/audio/',
] as const;

export interface HostInputs {
  files: Record<string, string>;
  combined: string;
}

/** The sha256 of `<path>\n<sha256>\n` per file, in key order. */
export function combinedSha256(files: Record<string, string>): string {
  return sha256(
    Object.keys(files)
      .map((p) => `${p}\n${files[p]}\n`)
      .join(''),
  );
}

/** The inputs file for `files` (path to sha256): keys sorted by byte order, `combined` set. */
export function hostInputs(files: Record<string, string>): HostInputs {
  const sorted = sortedKeys(files);
  return { files: sorted, combined: combinedSha256(sorted) };
}

/** The inputs file's text: two-space JSON with a trailing newline. */
export const inputsText = (inputs: HostInputs): string => jsonText(inputs);

/** The forbidden modules among `paths` (repo-relative), in order. */
export const forbiddenInputs = (paths: readonly string[]): string[] =>
  paths.filter((p) => FORBIDDEN_PREFIXES.some((f) => p.startsWith(f)));

/** Hashes the metafile's inputs (paths relative to `base`, the bundler's working directory):
 *  each real file's realpath relative to `root`, mapped to its sha256. Keys that are not files
 *  (virtual modules) are skipped. */
export function hashInputs(
  metafileInputs: readonly string[],
  root: string,
  base: string = process.cwd(),
): Record<string, string> {
  const realRoot = realpathSync(root);
  const files: Record<string, string> = {};
  for (const key of metafileInputs) {
    const abs = resolve(base, key);
    if (!existsSync(abs)) continue;
    const real = realpathSync(abs);
    files[relative(realRoot, real).split('\\').join('/')] = sha256(readFileSync(real));
  }
  return files;
}

/**
 * What is wrong with an inputs file's text against the files under `root`: format problems (not
 * the `{ files, combined }` shape, keys out of byte order, a wrong `combined`, not the exact text
 * `inputsText` writes), then `missing <path>` and `changed <path>` per file. Empty when current.
 */
export function inputsProblems(text: string, root: string): string[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (e) {
    return [`${HOST_INPUTS} is not JSON: ${errorMessage(e)}`];
  }
  const p = parsed as Partial<HostInputs> | null;
  const files = p?.files;
  if (
    p === null ||
    typeof p !== 'object' ||
    files === null ||
    typeof files !== 'object' ||
    Array.isArray(files) ||
    typeof p.combined !== 'string' ||
    Object.values(files).some((h) => typeof h !== 'string')
  ) {
    return [`${HOST_INPUTS} is not { files: { path: sha256 }, combined: sha256 }`];
  }
  const problems: string[] = [];
  const keys = Object.keys(files);
  if (keys.length === 0) problems.push(`${HOST_INPUTS} lists no files`);
  if (keys.join('\n') !== [...keys].sort(byteOrder).join('\n')) {
    problems.push(`${HOST_INPUTS} keys are not sorted by byte order`);
  }
  if (p.combined !== combinedSha256(files))
    problems.push(`${HOST_INPUTS} combined is not the hash of its files`);
  if (text !== inputsText(p as HostInputs)) {
    problems.push(`${HOST_INPUTS} is not two-space JSON with a trailing newline`);
  }
  for (const path of keys) {
    const abs = join(root, path);
    if (!existsSync(abs)) problems.push(`missing ${path}`);
    else if (sha256(readFileSync(abs)) !== files[path]) problems.push(`changed ${path}`);
  }
  return problems;
}

export interface BuiltBundle {
  js: string;
  inputs: HostInputs;
}

/** Runs the bundler (cwd must be `root`, the metafile's paths are relative to it). */
export async function buildBundle(root = REPO_ROOT): Promise<BuiltBundle> {
  const outdir = mkdtempSync(join(tmpdir(), 'strudeland-host-build-'));
  try {
    const result = await Bun.build({
      entrypoints: [join(root, ENTRY)],
      target: 'browser',
      format: 'iife',
      minify: false,
      sourcemap: 'none',
      metafile: true,
      define: {
        'process.env.NODE_ENV': '"production"',
        'import.meta.url': '"file:///host.js"',
      },
      outdir,
    });
    if (!result.success) {
      const first = result.logs.find((l) => l.level === 'error') ?? result.logs[0];
      throw new Error(
        `host:build: bundling failed: ${first ? String(first.message ?? first) : 'unknown error'}`,
      );
    }
    if (result.outputs.length !== 1) {
      throw new Error(`host:build: expected one output, got ${result.outputs.length}`);
    }
    const metafile = result.metafile;
    if (!metafile) throw new Error('host:build: the bundler returned no metafile');
    const files = hashInputs(Object.keys(metafile.inputs), root);
    const forbidden = forbiddenInputs(Object.keys(files));
    if (forbidden.length > 0) {
      throw new Error(
        `host:build: forbidden modules are reachable from ${ENTRY}: ${forbidden.join(', ')}`,
      );
    }
    let js = await result.outputs[0]!.text();
    if (!js.endsWith('\n')) js += '\n';
    return { js, inputs: hostInputs(files) };
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
}

async function main(): Promise<number> {
  process.chdir(REPO_ROOT);
  let built: BuiltBundle;
  try {
    built = await buildBundle(REPO_ROOT);
  } catch (e) {
    console.error(errorMessage(e));
    return 1;
  }
  mkdirSync(join(REPO_ROOT, HOST_JS_DIR), { recursive: true });
  writeFileSync(join(REPO_ROOT, HOST_JS), built.js);
  writeFileSync(join(REPO_ROOT, HOST_INPUTS), inputsText(built.inputs));
  const bytes = Buffer.byteLength(built.js);
  const count = Object.keys(built.inputs.files).length;
  console.log(
    `${HOST_JS}: ${bytes} bytes (${(bytes / 1024).toFixed(1)} KiB), ${count} input files`,
  );
  return 0;
}

if (import.meta.main) process.exit(await main());

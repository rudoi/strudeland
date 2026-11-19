/**
 * What the vector generators (`engine:vectors`, `render:vectors`) share: the file text, the
 * arguments, reading the committed directory, and the check-or-write run.
 */

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { diffRefs, formatDiff, hasChanges, utf8 } from './engine/refs-lib';

/** `{ schema, generator, ...sections }` as JSON with one array element per line (compact
 *  elements, readable diffs). */
export function vectorsFileText(generator: string, sections: Record<string, unknown>): string {
  const all: Record<string, unknown> = { schema: 1, generator, ...sections };
  const members = Object.entries(all).map(([k, v]) => {
    const body = Array.isArray(v)
      ? v.length === 0
        ? '[]'
        : `[\n${v.map((x) => JSON.stringify(x)).join(',\n')}\n]`
      : JSON.stringify(v);
    return `${JSON.stringify(k)}:${body}`;
  });
  return `{${members.join(',\n')}}\n`;
}

export interface VectorsOptions {
  write: boolean;
  help: boolean;
}

export function parseVectorsArgs(argv: string[]): VectorsOptions | { error: string } {
  const o: VectorsOptions = { write: false, help: false };
  for (const a of argv) {
    if (a === '--write') o.write = true;
    else if (a === '--help' || a === '-h') o.help = true;
    else return { error: `unknown argument ${a}` };
  }
  return o;
}

/** Every committed file of the vectors directory, file name -> bytes (all of them are the
 *  generator's: a file it does not generate shows up as removed). */
export function readVectors(dir: string): Map<string, Uint8Array> {
  const out = new Map<string, Uint8Array>();
  if (!existsSync(dir)) return out;
  for (const f of readdirSync(dir).sort()) out.set(f, new Uint8Array(readFileSync(join(dir, f))));
  return out;
}

/** A vector generator: its directory (relative to the repo root) and its `bun run` command. */
export interface VectorsCommand {
  dir: string;
  command: string;
}

/** Checks the committed files of `cmd.dir` against `generate()`, or with `--write` writes the
 *  generated files and removes the ones it no longer generates. Exit 0 current or written, 1 they
 *  differ, 2 bad arguments. */
export function runVectors(
  cmd: VectorsCommand,
  argv: string[],
  root: string,
  log: (line: string) => void,
  generate: () => Map<string, string>,
): number {
  const usage = `usage: bun run ${cmd.command} [--write]`;
  const opts = parseVectorsArgs(argv);
  if ('error' in opts) {
    log(opts.error);
    log(usage);
    return 2;
  }
  if (opts.help) {
    log(usage);
    return 0;
  }
  const dir = join(root, cmd.dir);
  const before = readVectors(dir);
  const after = new Map([...generate()].map(([n, t]) => [n, utf8(t)] as const));
  const diff = diffRefs(before, after);
  if (!hasChanges(diff)) {
    log(`vectors are current (${after.size} files)`);
    return 0;
  }
  for (const line of formatDiff(diff, before, after)) log(line);
  if (!opts.write) {
    log(`vectors differ; run \`bun run ${cmd.command} --write\` and review the diff`);
    return 1;
  }
  mkdirSync(dir, { recursive: true });
  for (const [name, bytes] of after) writeFileSync(join(dir, name), bytes);
  for (const name of diff.removed) rmSync(join(dir, name));
  log(`wrote ${cmd.dir}`);
  return 0;
}

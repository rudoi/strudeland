/**
 * `bun run ipc:build`: reads the Tauri commands in `src-tauri/src` and writes the committed
 * `src/lib/ipc/bindings.ts`: every command registered in `lib.rs`'s `generate_handler!` with its
 * invoke argument keys and simple value types, every emitted `*_EVENT` name keyed by its value,
 * and every camelCased `*Kind` enum (the command error kinds). It is plain regexes over the Rust
 * source, and it fails with `file:line` on anything it cannot read rather than guess.
 * `tests/ipc-bindings.test.ts` regenerates the file in memory, so stale bindings fail `bun test`.
 */

import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

export const REPO_ROOT = resolve(import.meta.dir, '../..');
export const BINDINGS = 'src/lib/ipc/bindings.ts';
const SRC = 'src-tauri/src';

/** A refusal: the Rust source has something the generator does not read. */
export class IpcParseError extends Error {
  constructor(
    readonly file: string,
    readonly line: number,
    reason: string,
  ) {
    super(`${file}:${line}: ${reason}`);
    this.name = 'IpcParseError';
  }
}

export interface HandlerEntry {
  module: string;
  name: string;
  line: number;
}

export interface CommandArg {
  key: string;
  type: string;
  optional: boolean;
}

/** The command's error type: `CommandError`, `ExportError`, `&'static str`, or none. */
export type CommandErrorType = 'command' | 'export' | 'text' | null;

export interface ParsedCommand {
  args: CommandArg[];
  error: CommandErrorType;
}

export interface EventConst {
  value: string;
  line: number;
}

export interface KindEnum {
  name: string;
  variants: string[];
}

export interface BindingsModel {
  commands: { name: string; args: CommandArg[] }[];
  events: string[];
  kinds: (KindEnum & { file: string })[];
}

const ATTRIBUTE = /^#\[tauri::command\]$/;
const ATTRIBUTE_WITH_ARGS = /^#\[tauri::command\s*\(/;
const FN = /^pub (?:async )?fn ([a-z_][a-z0-9_]*)\(/;
const INJECTED = [
  /^(?:tauri::)?State\s*</,
  /^(?:tauri::)?(?:AppHandle|WebviewWindow|Window|Webview)(?:\s*<.*>)?$/,
];
const NUMBER = /^(?:u8|u16|u32|u64|i8|i16|i32|i64|usize|f32|f64)$/;
const EVENT_LINE = /^(?:pub(?:\(crate\))? )?const [A-Z_]+_EVENT: &str\b/;
const EVENT = /^(?:pub(?:\(crate\))? )?const [A-Z_]+_EVENT: &str = "([a-z]+:\/\/[^"\\]+)";$/;
const KIND_ENUM = /^pub enum ([A-Z][A-Za-z0-9]*Kind) \{$/;
const CAMEL_SERDE = '#[serde(rename_all = "camelCase")]';

function isComment(line: string): boolean {
  return line.trimStart().startsWith('//');
}

/** The one-based line of `offset` in `text`. */
function lineAt(text: string, offset: number): number {
  return text.slice(0, offset).split('\n').length;
}

/** `device_id` → `deviceId`, as Tauri renames command arguments. */
export function camelCase(snake: string): string {
  return joinCamel(snake.split('_').filter((p) => p !== ''));
}

/** `project://changed` → `projectChanged`. */
export function eventKey(value: string): string {
  const parts = value.split(/[^A-Za-z0-9]+/).filter((p) => p !== '');
  return joinCamel(parts.map((p) => p.toLowerCase()));
}

/** The first part as it is, then each later part with its first letter upper-cased. */
function joinCamel(parts: string[]): string {
  return parts.map((p, i) => (i === 0 ? p : p[0]!.toUpperCase() + p.slice(1))).join('');
}

/** `CommandErrorKind` → `COMMAND_ERROR_KINDS`. */
export function kindsConst(name: string): string {
  return `${name.replace(/(?<=[a-z0-9])(?=[A-Z])/g, '_').toUpperCase()}S`;
}

/**
 * Splits `text` on commas at bracket depth 0 (`<>`, `()`, `[]`; the `>` of `->` is not a
 * bracket). Lifetimes are plain characters. Each part keeps its start offset.
 */
function splitTopLevel(text: string): { part: string; offset: number }[] {
  const parts: { part: string; offset: number }[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i]!;
    if (c === '<' || c === '(' || c === '[') depth++;
    else if ((c === '>' && text[i - 1] !== '-') || c === ')' || c === ']') depth--;
    else if (c === ',' && depth === 0) {
      parts.push({ part: text.slice(start, i), offset: start });
      start = i + 1;
    }
  }
  parts.push({ part: text.slice(start), offset: start });
  return parts;
}

/** The `module::name` entries of `tauri::generate_handler![ … ]`, in order. */
export function parseHandler(libRs: string, file = `${SRC}/lib.rs`): HandlerEntry[] {
  const open = 'tauri::generate_handler![';
  const at = libRs.indexOf(open);
  if (at < 0) throw new IpcParseError(file, 1, 'no tauri::generate_handler![ … ]');
  if (libRs.indexOf(open, at + 1) >= 0) {
    throw new IpcParseError(
      file,
      lineAt(libRs, libRs.indexOf(open, at + 1)),
      'a second generate_handler!',
    );
  }
  const bodyStart = at + open.length;
  const end = libRs.indexOf(']', bodyStart);
  if (end < 0) throw new IpcParseError(file, lineAt(libRs, at), 'generate_handler! is not closed');
  const entries: HandlerEntry[] = [];
  const seen = new Set<string>();
  for (const { part, offset } of splitTopLevel(libRs.slice(bodyStart, end))) {
    const text = part.trim();
    if (text === '') continue;
    const line = lineAt(libRs, bodyStart + offset + part.indexOf(text));
    const m = /^([a-z_][a-z0-9_]*)::([a-z_][a-z0-9_]*)$/.exec(text);
    if (!m) throw new IpcParseError(file, line, `cannot read the handler entry \`${text}\``);
    if (seen.has(m[2]!)) throw new IpcParseError(file, line, `\`${m[2]}\` is registered twice`);
    seen.add(m[2]!);
    entries.push({ module: m[1]!, name: m[2]!, line });
  }
  return entries;
}

/** The TS type of a Rust argument type, and whether the key is optional. */
function valueType(rust: string): { type: string; optional: boolean } {
  const option = /^Option\s*<(.*)>$/.exec(rust);
  if (option) return { type: valueType(option[1]!.trim()).type, optional: true };
  if (/^(?:String|&(?:'\w+ )?str|PathBuf|std::path::PathBuf)$/.test(rust)) {
    return { type: 'string', optional: false };
  }
  if (rust === 'bool') return { type: 'boolean', optional: false };
  if (NUMBER.test(rust)) return { type: 'number', optional: false };
  return { type: 'unknown', optional: false };
}

function errorType(ret: string, file: string, line: number): CommandErrorType {
  if (/\bResult\s*</.test(ret) && !ret.startsWith('Result<')) {
    throw new IpcParseError(file, line, `cannot read the return type \`${ret}\`: use Result<T, E>`);
  }
  const result = /^Result<(.*)>$/.exec(ret);
  if (!result) return null;
  // rustfmt may break a long Result over lines, with a trailing comma.
  const parts = splitTopLevel(result[1]!).filter(
    (p, i, all) => i < all.length - 1 || p.part.trim() !== '',
  );
  if (parts.length !== 2) {
    throw new IpcParseError(file, line, `cannot read the return type \`${ret}\`: use Result<T, E>`);
  }
  const e = parts[1]!.part.trim();
  if (e === 'CommandError') return 'command';
  if (e === 'ExportError') return 'export';
  if (e === "&'static str") return 'text';
  if (e === 'String') {
    throw new IpcParseError(
      file,
      line,
      'a command error must not be a String: return CommandError',
    );
  }
  throw new IpcParseError(
    file,
    line,
    `unsupported error type \`${e}\`: return CommandError, ExportError or &'static str`,
  );
}

/** Every `#[tauri::command]` fn in `source`, with its attribute's line. */
export function parseCommands(
  source: string,
  file: string,
): (ParsedCommand & { name: string; line: number })[] {
  const lines = source.split('\n');
  const offsets: number[] = [];
  let acc = 0;
  for (const l of lines) {
    offsets.push(acc);
    acc += l.length + 1;
  }
  const commands: (ParsedCommand & { name: string; line: number })[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    if (ATTRIBUTE_WITH_ARGS.test(line)) {
      throw new IpcParseError(
        file,
        i + 1,
        'a #[tauri::command(…)] with arguments changes the keys',
      );
    }
    if (!ATTRIBUTE.test(line)) continue;
    let j = i + 1;
    while (j < lines.length && isComment(lines[j]!)) j++;
    const fnLine = lines[j];
    const m = fnLine === undefined ? null : FN.exec(fnLine);
    if (!m) {
      throw new IpcParseError(
        file,
        j + 1,
        'expected `pub (async )?fn <name>(` after #[tauri::command]',
      );
    }
    const name = m[1]!;
    const open = offsets[j]! + m[0].length;
    let depth = 1;
    let k = open;
    for (; k < source.length && depth > 0; k++) {
      const c = source[k]!;
      if (c === '/' && source[k + 1] === '/') {
        throw new IpcParseError(
          file,
          lineAt(source, k),
          `a // comment in \`${name}\`'s parameters`,
        );
      }
      if (c === '<' || c === '(' || c === '[') depth++;
      else if ((c === '>' && source[k - 1] !== '-') || c === ')' || c === ']') depth--;
    }
    if (depth > 0) throw new IpcParseError(file, j + 1, `\`${name}\`'s parameters are not closed`);
    const params = source.slice(open, k - 1);
    const args: CommandArg[] = [];
    const keys = new Set<string>();
    for (const { part, offset } of splitTopLevel(params)) {
      const text = part.trim().replace(/\s+/g, ' ');
      if (text === '') continue;
      const at = lineAt(source, open + offset + part.indexOf(part.trim()));
      const p = /^(?:mut )?([a-z_][a-z0-9_]*): (.+)$/.exec(text);
      if (!p) throw new IpcParseError(file, at, `cannot read the parameter \`${text}\``);
      const type = p[2]!;
      if (INJECTED.some((r) => r.test(type))) continue;
      const key = camelCase(p[1]!);
      if (keys.has(key)) {
        throw new IpcParseError(file, at, `two parameters of \`${name}\` camelCase to \`${key}\``);
      }
      keys.add(key);
      args.push({ key, ...valueType(type) });
    }
    const brace = source.indexOf('{', k);
    if (brace < 0) throw new IpcParseError(file, j + 1, `\`${name}\` has no body`);
    const tail = source.slice(k, brace).trim().replace(/\s+/g, ' ');
    let error: CommandErrorType = null;
    if (tail.startsWith('->')) {
      error = errorType(tail.slice(2).trim(), file, lineAt(source, k));
    } else if (tail !== '') {
      throw new IpcParseError(
        file,
        lineAt(source, k),
        `cannot read \`${tail}\` after the parameters`,
      );
    }
    commands.push({ name, line: i + 1, args, error });
  }
  return commands;
}

/** The `#[tauri::command]` fn `name` in `source`. */
export function parseCommand(source: string, file: string, name: string): ParsedCommand {
  const found = parseCommands(source, file).find((c) => c.name === name);
  if (!found) throw new IpcParseError(file, 1, `no #[tauri::command] fn \`${name}\``);
  return { args: found.args, error: found.error };
}

/** The column-0 `*_EVENT: &str` consts in `source`. */
export function parseEvents(source: string, file: string): EventConst[] {
  const events: EventConst[] = [];
  source.split('\n').forEach((line, i) => {
    if (!EVENT_LINE.test(line)) return;
    const m = EVENT.exec(line);
    if (!m)
      throw new IpcParseError(
        file,
        i + 1,
        'expected `const <NAME>_EVENT: &str = "<scheme>://<path>";`',
      );
    events.push({ value: m[1]!, line: i + 1 });
  });
  return events;
}

/** The column-0 `pub enum <Name>Kind {` under `#[serde(rename_all = "camelCase")]`. */
export function parseKinds(source: string, file: string): KindEnum[] {
  const lines = source.split('\n');
  const kinds: KindEnum[] = [];
  for (let i = 0; i < lines.length; i++) {
    const m = KIND_ENUM.exec(lines[i]!);
    if (!m) continue;
    let camel = false;
    for (let j = i - 1; j >= 0 && /^(?:#\[|\/\/)/.test(lines[j]!); j--) {
      if (lines[j] === CAMEL_SERDE) camel = true;
    }
    if (!camel) continue;
    const variants: string[] = [];
    let j = i + 1;
    for (; j < lines.length && lines[j] !== '}'; j++) {
      const text = lines[j]!.trim();
      if (text === '' || text.startsWith('//')) continue;
      if (text.startsWith('#[')) {
        if (/^#\[serde\(\s*rename/.test(text)) {
          throw new IpcParseError(file, j + 1, `a #[serde(rename…)] on a ${m[1]} variant`);
        }
        continue;
      }
      for (const piece of text.split(',')) {
        const v = piece.trim();
        if (v === '') continue;
        if (!/^[A-Z][A-Za-z0-9]*$/.test(v)) {
          throw new IpcParseError(file, j + 1, `${m[1]} variant \`${v}\` is not a unit variant`);
        }
        variants.push(v[0]!.toLowerCase() + v.slice(1));
      }
    }
    if (j >= lines.length) throw new IpcParseError(file, i + 1, `${m[1]} is not closed`);
    kinds.push({ name: m[1]!, variants });
  }
  return kinds;
}

const quote = (s: string) => `'${s}'`;

/** The bindings file. Its formatting is fixed here, so prettier never touches it. */
export function render(model: BindingsModel): string {
  const out: string[] = [];
  out.push('// Generated by `bun run ipc:build` from src-tauri/src. Do not edit.', '');
  out.push(
    '/** Every registered command and its invoke argument keys, in `generate_handler!` order. */',
  );
  out.push('export const COMMANDS = {');
  for (const c of model.commands) {
    out.push(`  ${c.name}: [${c.args.map((a) => quote(a.key)).join(', ')}],`);
  }
  out.push('} as const;');
  out.push('export type CommandName = keyof typeof COMMANDS;', '');
  out.push(
    '/** The invoke arguments of each command. Struct payloads are `unknown`: their types are',
  );
  out.push(' *  hand-written beside each adapter. */');
  out.push('export interface CommandArgs {');
  for (const c of model.commands) {
    const body =
      c.args.length === 0
        ? 'Record<string, never>'
        : `{ ${c.args
            .map((a) => (a.optional ? `${a.key}?: ${a.type} | null` : `${a.key}: ${a.type}`))
            .join('; ')} }`;
    out.push(`  ${c.name}: ${body};`);
  }
  out.push('}', '');
  out.push(
    '/** Every event the app emits, keyed by the camelCased name: `project://changed` → projectChanged. */',
  );
  out.push('export const EVENTS = {');
  for (const value of model.events) out.push(`  ${eventKey(value)}: ${quote(value)},`);
  out.push('} as const;');
  for (const k of model.kinds) {
    const name = kindsConst(k.name);
    out.push('', `/** \`${k.name}\` (${k.file}), in declaration order. */`);
    out.push(`export const ${name} = [`);
    for (const v of k.variants) out.push(`  ${quote(v)},`);
    out.push('] as const;');
    out.push(`export type ${k.name} = (typeof ${name})[number];`);
  }
  return `${out.join('\n')}\n`;
}

/** Reads `src-tauri/src` under `root` into the bindings model. */
export function readModel(root: string): BindingsModel {
  const libFile = `${SRC}/lib.rs`;
  const libRs = readFileSync(join(root, libFile), 'utf8');
  const handler = parseHandler(libRs, libFile);
  const files = readdirSync(join(root, SRC))
    .filter((f) => f.endsWith('.rs'))
    .sort();
  const sources = new Map(
    files.map((f) => [f.slice(0, -3), readFileSync(join(root, SRC, f), 'utf8')]),
  );

  const registered = new Map(handler.map((h) => [`${h.module}::${h.name}`, h]));
  const parsed = new Map<string, ParsedCommand>();
  const events: { value: string; file: string; line: number }[] = [];
  const kinds: (KindEnum & { file: string })[] = [];
  for (const [module, source] of sources) {
    const file = `${SRC}/${module}.rs`;
    for (const c of parseCommands(source, file)) {
      const id = `${module}::${c.name}`;
      if (!registered.has(id)) {
        throw new IpcParseError(
          file,
          c.line,
          `\`${c.name}\` is a #[tauri::command] not registered in generate_handler!`,
        );
      }
      parsed.set(id, c);
    }
    for (const e of parseEvents(source, file)) {
      const dup = events.find(
        (x) => x.value === e.value || eventKey(x.value) === eventKey(e.value),
      );
      if (dup) {
        throw new IpcParseError(
          file,
          e.line,
          `event \`${e.value}\` collides with ${dup.file}:${dup.line}`,
        );
      }
      events.push({ ...e, file });
    }
    for (const k of parseKinds(source, file)) kinds.push({ ...k, file });
  }

  const commands = handler.map((h) => {
    const c = parsed.get(`${h.module}::${h.name}`);
    if (!c) {
      throw new IpcParseError(
        libFile,
        h.line,
        `no #[tauri::command] fn \`${h.name}\` in ${SRC}/${h.module}.rs`,
      );
    }
    return { name: h.name, args: c.args };
  });
  events.sort((a, b) => (a.value < b.value ? -1 : a.value > b.value ? 1 : 0));
  return { commands, events: events.map((e) => e.value), kinds };
}

/** Reads `src-tauri/src` under `root` and returns the bindings text. */
export function generate(root: string): string {
  return render(readModel(root));
}

/** `bun run ipc:build`: 0 written, 1 a refusal (`file:line: reason`), 2 any argument. */
export function main(
  argv: string[],
  root: string,
  log: (line: string) => void = console.log,
  fail: (line: string) => void = console.error,
): number {
  if (argv.length > 0) {
    fail('usage: bun run ipc:build (takes no arguments)');
    return 2;
  }
  let model: BindingsModel;
  try {
    model = readModel(root);
  } catch (e) {
    fail(e instanceof Error ? e.message : String(e));
    return 1;
  }
  writeFileSync(join(root, BINDINGS), render(model));
  const kinds = model.kinds.reduce((n, k) => n + k.variants.length, 0);
  log(
    `wrote ${BINDINGS} (${model.commands.length} commands, ${model.events.length} events, ${kinds} kinds)`,
  );
  return 0;
}

if (import.meta.main) process.exit(main(process.argv.slice(2), REPO_ROOT));

import {
  parseExpressionAt,
  tokenizer,
  tokTypes,
  type ArrayExpression,
  type Expression,
  type Literal,
  type Node,
  type ObjectExpression,
} from 'acorn';
import type { EvaluateOptions } from '../strudel/runtime';
import {
  bpmProblem,
  buildStructure,
  engineProblem,
  loopRangeProblem,
  loopsProblem,
  meterChangesProblem,
  meterProblem,
  sectionsProblem,
  seedProblem,
  StructureError,
  type SongStructure,
} from '../strudel/structure';

/** A value allowed in a song's `meta` header: JSON-like literals only. */
type MetaValue = string | number | boolean | null | MetaValue[] | { [key: string]: MetaValue };

export interface Meta {
  title?: string;
  bpm?: number;
  seed?: number;
  meter?: unknown;
  /** The live engine, `'superdough'` (default) or `'rust'`. */
  engine?: unknown;
  sections?: unknown;
  loop?: unknown;
  meterChanges?: unknown;
  loops?: unknown;
  [key: string]: unknown;
}

/** A header that parses but holds something other than a plain literal value. */
export class MetaError extends Error {
  /** 0-based offset into the code. */
  readonly pos: number;
  /** 1-based (acorn's convention). */
  readonly line: number;
  /** 0-based (acorn's convention). */
  readonly column: number;

  constructor(message: string, pos: number, line: number, column: number) {
    super(message);
    this.name = 'MetaError';
    this.pos = pos;
    this.line = line;
    this.column = column;
  }

  /** The message with its 1-based line and column: `msg (line:column)`. */
  get located(): string {
    return `${this.message} (${this.line}:${this.column + 1})`;
  }

  /** As a failed evaluate reports it: `meta: msg (line:column)`. */
  get reported(): string {
    return `meta: ${this.located}`;
  }
}

/**
 * A song's header as read from its text. On success, `options` is what `runtime.evaluate` takes
 * and `title` is `meta.title` when it is a string. On error there are no options, so a song with
 * a broken header cannot be evaluated, and `structure` is the default one.
 */
export type SongHeader =
  | {
      ok: true;
      meta: Meta;
      title: string | undefined;
      structure: SongStructure;
      options: EvaluateOptions;
    }
  | { ok: false; error: MetaError; structure: SongStructure };

/** The structure of a song without a usable header. */
const DEFAULT_STRUCTURE = buildStructure({});

/** The transpiler's grammar, so anything that evaluates also parses. */
const PARSE_OPTIONS = {
  ecmaVersion: 2022,
  sourceType: 'script',
  allowAwaitOutsideFunction: true,
  locations: true,
} as const;

function errorAt(node: Node, message: string): MetaError {
  const { line, column } = node.loc!.start;
  return new MetaError(message, node.start, line, column);
}

/** Own data property, so a `__proto__` key cannot change the prototype. */
function define(obj: Record<string, MetaValue>, key: string, value: MetaValue) {
  Object.defineProperty(obj, key, { value, enumerable: true, writable: true, configurable: true });
}

/** `ArrowFunctionExpression` -> `an arrow function expression`. */
function describe(node: Node): string {
  const words = node.type.replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();
  return (/^[aeiou]/.test(words) ? 'an ' : 'a ') + words;
}

function toValue(node: Node, path: string): MetaValue {
  const expr = node as Expression;
  switch (expr.type) {
    case 'Literal': {
      const v = expr.value;
      if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean' || v === null) {
        if (expr.regex || expr.bigint !== undefined) break;
        return v;
      }
      break;
    }
    case 'UnaryExpression':
      if (
        expr.operator === '-' &&
        expr.argument.type === 'Literal' &&
        typeof expr.argument.value === 'number'
      ) {
        return -expr.argument.value;
      }
      break;
    case 'ArrayExpression':
      return expr.elements.map((el, i) => {
        if (el === null) throw errorAt(expr, `${path} must not have holes`);
        return toValue(el, `${path}[${i}]`);
      });
    case 'ObjectExpression':
      return toObject(expr, path);
  }
  throw errorAt(node, `${path} must be a literal value, not ${describe(node)}`);
}

type Check = (key: string, value: MetaValue, node: Node) => void;

function toObject(
  node: ObjectExpression,
  path: string,
  check?: Check,
): { [key: string]: MetaValue } {
  const out: Record<string, MetaValue> = {};
  for (const prop of node.properties) {
    if (prop.type === 'SpreadElement') throw errorAt(prop, `${path} must not use spread`);
    if (prop.computed) throw errorAt(prop.key, `${path} must not have computed keys`);
    if (prop.kind !== 'init') throw errorAt(prop, `${path} must not have getters or setters`);
    if (prop.method) throw errorAt(prop, `${path} must not have methods`);
    let key: string;
    if (prop.key.type === 'Identifier') key = prop.key.name;
    else if (prop.key.type === 'Literal' && typeof prop.key.value === 'string')
      key = prop.key.value;
    else throw errorAt(prop.key, `${path} keys must be identifiers or strings`);
    const value = toValue(prop.value, `${path}.${key}`);
    check?.(key, value, prop.value);
    define(out, key, value);
  }
  return out;
}

/** A string literal written with double quotes, which the transpiler turns into mini-notation. */
const isDoubleQuoted = (node: Node): boolean =>
  node.type === 'Literal' &&
  typeof (node as Literal).value === 'string' &&
  (node as Literal).raw?.startsWith('"') === true;

const MINI_QUOTES = 'must use single quotes: double-quoted strings are mini-notation';

/**
 * The strings the app reads at runtime (`meter`, `engine`, `sections[i].name`) must be
 * single-quoted: the transpiler turns every double-quoted string into mini-notation, so it would reach `song()` as a
 * Pattern. `title` is only read statically (for display), so it may use either quote.
 */
function checkQuotes(key: string, node: Node) {
  if ((key === 'meter' || key === 'engine') && isDoubleQuoted(node)) {
    throw errorAt(node, `meta.${key} ${MINI_QUOTES}`);
  }
  function checkObject(obj: ObjectExpression, path: string, fields: readonly string[]) {
    for (const prop of obj.properties) {
      if (prop.type !== 'Property' || prop.computed) continue;
      const name =
        prop.key.type === 'Identifier'
          ? prop.key.name
          : prop.key.type === 'Literal'
            ? prop.key.value
            : null;
      if (typeof name === 'string' && fields.includes(name) && isDoubleQuoted(prop.value)) {
        throw errorAt(prop.value, `${path}.${name} ${MINI_QUOTES}`);
      }
    }
  }
  if (key === 'loop' && node.type === 'ObjectExpression') {
    checkObject(node as ObjectExpression, 'meta.loop', ['from', 'through']);
  }
  if ((key === 'sections' || key === 'meterChanges') && node.type === 'ArrayExpression') {
    const fields = key === 'sections' ? ['name'] : ['meter'];
    (node as ArrayExpression).elements.forEach((el, i) => {
      if (el?.type === 'ObjectExpression') checkObject(el, `meta.${key}[${i}]`, fields);
    });
  }
}

/**
 * Rules for the fields this app reads, shared with `structure.ts`. Checked on every occurrence, so
 * a duplicate key cannot hide one. Errors point at the value node; errors inside `sections` point
 * at the `sections` value and name the index in the message, except a double-quoted name, which
 * points at the literal. `loops` needs the loop section, so
 * `readHeader` checks it once the whole object is known.
 */
function checkKnownField(key: string, value: MetaValue, node: Node) {
  checkQuotes(key, node);
  let problem: string | null = null;
  if (key === 'title' && typeof value !== 'string') problem = 'meta.title must be a string';
  else if (key === 'bpm') problem = bpmProblem(value);
  else if (key === 'seed') problem = seedProblem(value);
  else if (key === 'meter') problem = meterProblem(value);
  else if (key === 'engine') problem = engineProblem(value);
  else if (key === 'sections') problem = sectionsProblem(value);
  else if (key === 'loop') problem = loopRangeProblem(value);
  else if (key === 'meterChanges') problem = meterChangesProblem(value);
  if (problem !== null) throw errorAt(node, problem);
}

/** Whitespace and comments; unterminated comments are left in place. */
const GAP = /(?:\s|\/\/[^\n\r\u2028\u2029]*|\/\*[\s\S]*?\*\/)*/y;
export const LINE_BREAK = /[\n\r\u2028\u2029]/;

/**
 * Offset of the `{` when the file's first statement starts with `const meta = {`, else null. Only
 * these four tokens are read, so the rest of the file is never tokenized.
 */
function headerStart(code: string): number | null {
  const tokens = tokenizer(code, PARSE_OPTIONS);
  const expected = [tokTypes._const, 'meta', tokTypes.eq, tokTypes.braceL] as const;
  let brace = -1;
  for (const want of expected) {
    const tok = tokens.getToken();
    if (typeof want === 'string') {
      if (tok.type !== tokTypes.name || code.slice(tok.start, tok.end) !== want) return null;
    } else if (tok.type !== want) return null;
    brace = tok.start;
  }
  return brace;
}

/** True when the statement can end at `pos`: `;`, the end of the file, or a line break (ASI). */
function statementEnds(code: string, pos: number): boolean {
  GAP.lastIndex = pos;
  GAP.exec(code);
  const next = GAP.lastIndex;
  return next === code.length || code[next] === ';' || LINE_BREAK.test(code.slice(pos, next));
}

/**
 * The `const meta = {…}` object of the header, or null when the file has no header. Only the
 * header statement is parsed (this runs on every keystroke), so the song body is never looked at.
 * Throws acorn's SyntaxError if the header itself does not parse.
 */
function findHeader(code: string): ObjectExpression | null {
  const brace = headerStart(code);
  if (brace === null) return null;
  // The maximal expression from `{`: `{…}, x = 1` or `{…} + 1` come back as something else.
  const init = parseExpressionAt(code, brace, PARSE_OPTIONS);
  if (init.type !== 'ObjectExpression' || !statementEnds(code, init.end)) return null;
  return init;
}

/**
 * The header object, its checked value and the structure built from it, or null when the file has
 * no header. Throws acorn's SyntaxError if the header does not parse and MetaError if it breaks a
 * rule.
 */
function readHeader(
  code: string,
): { header: ObjectExpression; meta: Meta; structure: SongStructure } | null {
  const header = findHeader(code);
  if (!header) return null;
  const loops: { value: MetaValue; node: Node }[] = [];
  // The last occurrence of each key: the value `meta` keeps, so the one a cross-field error is about.
  const lastNode = new Map<string, Node>();
  const meta: Meta = toObject(header, 'meta', (key, value, node) => {
    checkKnownField(key, value, node);
    if (key === 'loops') loops.push({ value, node });
    lastNode.set(key, node);
  });
  // The per-field checks have passed, so `buildStructure` can only fail on a cross-field rule. It
  // checks loop, then meter changes, then loops.
  let structure: SongStructure;
  try {
    structure = buildStructure(meta);
  } catch (e) {
    if (!(e instanceof StructureError)) throw e;
    if (e.field !== 'loops') throw errorAt(lastNode.get(e.field)!, e.message);
    // Earlier duplicate `loops` keys are only range-checked, in source order, before the last.
    rangeCheckEarlierLoops(loops);
    throw errorAt(lastNode.get('loops')!, e.message);
  }
  rangeCheckEarlierLoops(loops);
  return { header, meta, structure };
}

/** Every `loops` occurrence but the last (the value `buildStructure` checks) needs only a valid range. */
function rangeCheckEarlierLoops(loops: readonly { value: MetaValue; node: Node }[]) {
  for (const { value, node } of loops.slice(0, -1)) {
    const problem = loopsProblem(value, true);
    if (problem !== null) throw errorAt(node, problem);
  }
}

/**
 * Read the optional `const meta = {…}` header of a song without evaluating it. Never throws.
 * A syntax error in the header is not a `MetaError`: evaluating reports it, so the result is ok
 * with an empty `meta`. A syntax error after the header does not matter: the header is still
 * read, so the title stays put while the body is mid-edit.
 */
export function readSongHeader(code: string): SongHeader {
  let read: ReturnType<typeof readHeader> = null;
  try {
    read = readHeader(code);
  } catch (e) {
    if (e instanceof MetaError) return { ok: false, error: e, structure: DEFAULT_STRUCTURE };
  }
  const meta: Meta = read?.meta ?? {};
  const structure = read?.structure ?? DEFAULT_STRUCTURE;
  return {
    ok: true,
    meta,
    title: typeof meta.title === 'string' ? meta.title : undefined,
    structure,
    options: {
      cps: structure.cps,
      meter: structure.meter,
      seed: meta.seed,
      engine: meta.engine === 'rust' ? 'rust' : 'superdough',
      expectedStructure: structure,
    },
  };
}

/**
 * The header's acorn object (its `properties[].key/value` `start`/`end` are offsets into `code`),
 * or null when there is no header, it does not parse, or it has a `MetaError`. Never throws.
 */
export function headerRanges(code: string): ObjectExpression | null {
  try {
    return readHeader(code)?.header ?? null;
  } catch {
    return null;
  }
}

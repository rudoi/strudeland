import { parse, type AnyNode, type Comment } from 'acorn';
import { mini2ast } from '@strudel/mini';
import { getLanguages } from '@strudel/transpiler';
import { staticStructure } from '../project/evaluate-options';
import { parseMeta } from '../project/meta';

export interface FormatEdit {
  from: number;
  to: number;
  insert: string;
}

const scriptOptions = {
  ecmaVersion: 2022,
  sourceType: 'script',
  allowAwaitOutsideFunction: true,
} as const;

// The mini parser accepts a quoted body. Its offsets therefore include the opening quote,
// just like offsets in the source literal (whether delimited by double quotes or backticks).
type MiniNode = MiniPattern | MiniAtom | MiniElement;
interface MiniPattern {
  type_: 'pattern';
  arguments_: { alignment: string; _steps?: boolean };
  source_: MiniNode[];
}
interface MiniAtom {
  type_: 'atom';
  source_: string;
  location_: { start: { offset: number }; end: { offset: number } };
}
interface MiniElement {
  type_: 'element';
  source_: MiniNode;
  options_: { ops: unknown[]; weight: number; reps: number };
}

function bareSequence(node: MiniNode): MiniNode[] | undefined {
  if (
    node.type_ === 'pattern' &&
    node.arguments_.alignment === 'fastcat' &&
    !node.arguments_._steps
  ) {
    return node.source_;
  }
}

function bareElement(node?: MiniNode): MiniNode | undefined {
  if (node?.type_ !== 'element') return;
  const { ops, weight, reps } = node.options_;
  if (!ops.length && weight === 1 && reps === 1) return node.source_;
}

function rowAtoms(sequence: MiniNode): MiniAtom[] | undefined {
  const children = bareSequence(sequence);
  if (children?.length !== 16 && children?.length !== 32) return;
  const atoms: MiniAtom[] = [];
  for (const child of children) {
    const atom = bareElement(child);
    if (atom?.type_ !== 'atom') return;
    atoms.push(atom);
  }
  return atoms;
}

function barsInSlowSequence(root: MiniNode): MiniAtom[][] | undefined {
  const children = bareSequence(root);
  if (children?.length !== 1) return;
  const slow = bareElement(children[0]);
  if (
    slow?.type_ !== 'pattern' ||
    slow.arguments_.alignment !== 'polymeter_slowcat' ||
    slow.source_.length !== 1
  )
    return;
  const bars = bareSequence(slow.source_[0]);
  if (!bars?.length) return;
  const rows: MiniAtom[][] = [];
  for (const bar of bars) {
    const sequence = bareElement(bar);
    const atoms = sequence && rowAtoms(sequence);
    if (!atoms) return;
    rows.push(atoms);
  }
  return rows;
}

function apply(source: string, edits: readonly FormatEdit[]): string {
  let result = '';
  let pos = 0;
  for (const edit of edits) {
    result += source.slice(pos, edit.from) + edit.insert;
    pos = edit.to;
  }
  return result + source.slice(pos);
}

function miniStructure(node: MiniNode): string {
  return JSON.stringify(node, (key, value) => (key === 'location_' ? undefined : value));
}

function formatLiteral(literal: string, indent: string): FormatEdit[] {
  if (literal.length < 33 || literal.includes('\\')) return [];
  const quoted = '"' + literal.slice(1, -1) + '"';
  const root: MiniNode = mini2ast(quoted);
  const flat = rowAtoms(root);
  const bars = flat ? [flat] : barsInSlowSequence(root);
  if (!bars) return [];
  const multiline = !flat || flat.length === 32;
  const quote = multiline ? '`' : literal[0];
  const edits: FormatEdit[] = [];
  function change(from: number, to: number, insert: string) {
    if (literal.slice(from, to) !== insert) edits.push({ from, to, insert });
  }

  // Consume the entire literal as its exact expected spellings separated by PEG whitespace.
  // This also rules out source operators with neutral AST options, e.g. @1.
  let pos = 1;
  const whitespace = /[ \t\n\r\u00a0]*/y;
  function token(spelling: string, separator: string, atom?: MiniAtom): boolean {
    whitespace.lastIndex = pos;
    whitespace.exec(quoted);
    const start = whitespace.lastIndex;
    if (!quoted.startsWith(spelling, start)) return false;
    if (atom) {
      const { start: a, end: b } = atom.location_;
      if (
        a.offset > start ||
        b.offset < start + spelling.length ||
        quoted.slice(a.offset, b.offset).trim() !== spelling
      )
        return false;
    }
    change(pos, start, separator);
    pos = start + spelling.length;
    return true;
  }

  change(0, 1, quote);
  // Acorn normalizes CRLF inside raw templates; LF keeps mini offsets in source coordinates.
  const newline = '\n';
  const rowStart = newline + indent + '  ';
  if (!flat && !token('<', '')) return [];
  for (const atoms of bars) {
    const width = Math.max(...atoms.map((atom) => atom.source_.length));
    if (!flat && !token('[', rowStart)) return [];
    for (let i = 0; i < atoms.length; i++) {
      let gap = '';
      if (i === 0) {
        if (flat && multiline) gap = rowStart;
      } else if (i === 16) {
        gap = flat ? rowStart : rowStart + ' ';
      } else {
        const boundary = atoms.length === 32 && i % 8 === 0 ? 4 : i % 4 === 0 ? 2 : 1;
        gap = ' '.repeat(width - atoms[i - 1].source_.length + boundary);
      }
      if (!token(atoms[i].source_, gap, atoms[i])) return [];
    }
    if (!flat && !token(']', '')) return [];
  }
  if (!flat && !token('>', newline + indent)) return [];
  // Consume the final whitespace and match the synthetic closing double quote.
  if (!token('"', flat && multiline ? newline + indent : '') || pos !== quoted.length) return [];
  change(pos - 1, pos, quote);
  if (!edits.length) return [];
  const next = apply(literal, edits);
  const nextRoot = mini2ast('"' + next.slice(1, -1) + '"');
  return miniStructure(root) === miniStructure(nextRoot) ? edits : [];
}

function miniDisabledRanges(comments: Comment[], codeEnd: number): [number, number][] {
  const ranges: [number, number][] = [];
  const stack: number[] = [];
  for (const comment of comments) {
    const value = comment.value.trim();
    if (value.startsWith('mini-off')) stack.push(comment.start);
    if (value.startsWith('mini-on')) {
      const start = stack.pop();
      if (start !== undefined) ranges.push([start, comment.end]);
    }
  }
  for (const start of stack) ranges.push([start, codeEnd]);
  return ranges;
}

interface Candidate {
  node: AnyNode;
  edits: FormatEdit[];
}

function candidates(source: string, script: AnyNode, comments: Comment[]): Candidate[] {
  const disabled = miniDisabledRanges(comments, source.length);
  const literals: AnyNode[] = [];
  function visit(node: AnyNode) {
    if (node.type === 'TaggedTemplateExpression') return;
    if (
      node.type === 'VariableDeclarator' &&
      node.id.type === 'Identifier' &&
      node.id.name === 'meta'
    )
      return;
    if (node.type === 'ExpressionStatement' && 'directive' in node) return;
    if (node.type === 'TemplateLiteral' || node.type === 'Literal') {
      if (
        node.type === 'TemplateLiteral' ? node.expressions.length !== 0 : source[node.start] !== '"'
      )
        return;
      if (disabled.some(([start, end]) => start <= node.start && node.start < end)) return;
      literals.push(node);
      return;
    }
    for (const [key, value] of Object.entries(node)) {
      if (key === 'key' || (node.type === 'MemberExpression' && key === 'property')) continue;
      for (const child of Array.isArray(value) ? value : [value]) {
        if (child && typeof child.type === 'string') visit(child);
      }
    }
  }
  visit(script);
  const found: Candidate[] = [];
  let previousEnd = -1;
  let previousIndent = '';
  for (const node of literals.sort((a, b) => a.start - b.start)) {
    const line = node.start - node.loc!.start.column;
    // A previous multiline literal may move the rest of its closing line onto another
    // indentation level. Lay out subsequent literals against that same resulting line.
    const indent =
      line < previousEnd ? previousIndent : /^[ \t]*/.exec(source.slice(line, node.start))![0];
    try {
      const edits = formatLiteral(source.slice(node.start, node.end), indent);
      if (!edits.length) continue;
      found.push({
        node,
        edits: edits.map((e) => ({
          from: node.start + e.from,
          to: node.start + e.to,
          insert: e.insert,
        })),
      });
      previousEnd = node.end;
      previousIndent = indent;
    } catch {
      // Partially typed or unsupported mini must not prevent other literals from formatting.
    }
  }
  return found;
}

// A physical newline requires converting a double-quoted literal to a template. Across an
// implicit semicolon its opening backtick can instead become a tag on the preceding expression.
// Mask only the exact intended literal spans, then compare the full Acorn trees.
function syntax(script: AnyNode, spans: Map<number, number>): string {
  return JSON.stringify(script, (key, value) => {
    if (key === 'start' || key === 'end' || key === 'loc') return undefined;
    if (
      (value?.type === 'Literal' || value?.type === 'TemplateLiteral') &&
      spans.get(value.start) === value.end
    )
      return '<mini>';
    return typeof value === 'bigint' ? String(value) : value;
  });
}

function verified(source: string, script: AnyNode, choices: Candidate[]): FormatEdit[] | undefined {
  const before = new Map<number, number>();
  const after = new Map<number, number>();
  let delta = 0;
  const edits: FormatEdit[] = [];
  for (const { node, edits: local } of choices) {
    before.set(node.start, node.end);
    const start = node.start + delta;
    for (const edit of local) {
      edits.push(edit);
      delta += edit.insert.length - (edit.to - edit.from);
    }
    after.set(start, node.end + delta);
  }
  try {
    const next = parse(apply(source, edits), scriptOptions);
    if (syntax(script, before) === syntax(next, after)) return edits;
  } catch {
    // A replacement template can also be illegal, e.g. directly after an optional chain.
  }
}

/** Localized, ordered CodeMirror changes. Never evaluates the song or starts audio. */
export function formatSong(source: string): readonly FormatEdit[] {
  try {
    if (getLanguages().has('minilang')) return [];
    const comments: Comment[] = [];
    const script = parse(source, { ...scriptOptions, locations: true, onComment: comments });
    const { meta, error } = parseMeta(source);
    const { meter } = staticStructure(meta);
    if (error || meter.num !== 4 || meter.den !== 4) return [];
    const all = candidates(source, script, comments);
    if (!all.length) return [];
    const edits = verified(source, script, all);
    if (edits || all.length === 1) return edits ?? [];
    // Isolate the rare unsafe substitution so independent literals still get their layout.
    return (
      verified(
        source,
        script,
        all.filter((c) => verified(source, script, [c])),
      ) ?? []
    );
  } catch {
    return [];
  }
}

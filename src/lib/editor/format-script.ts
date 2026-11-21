import {
  parse,
  tokTypes,
  type AnyNode,
  type Comment,
  type Node,
  type ObjectExpression,
  type Token,
} from 'acorn';
import { headerRanges } from '../project/meta';
import type { FormatEdit } from './format-song';

export const scriptOptions = {
  ecmaVersion: 2022,
  sourceType: 'script',
  allowAwaitOutsideFunction: true,
} as const;

interface Line {
  anchor: number;
  continuation: boolean;
}

interface Gap {
  from: number;
  to: number;
  layout: string | Line;
}

const lineBreak = /\r\n|[\r\n\u2028\u2029]/;
const patternGroups = new Set(['stack', 'seq', 'cat']);

function readScript(source: string) {
  const tokens: Token[] = [];
  const comments: Comment[] = [];
  const ast = parse(source, {
    ...scriptOptions,
    preserveParens: true,
    onToken: tokens,
    onComment: comments,
  });
  return { ast, tokens, comments };
}

function sameSyntax(
  source: string,
  before: ReturnType<typeof readScript>,
  result: string,
): boolean {
  const after = readScript(result);
  const syntax = (ast: AnyNode) =>
    JSON.stringify(ast, (key, value) => {
      if (key === 'start' || key === 'end') return undefined;
      return typeof value === 'bigint' ? String(value) : value;
    });
  type Span = Pick<Node, 'start' | 'end'>;
  const sameText = (original: Span[], formatted: Span[]) =>
    original.length === formatted.length &&
    original.every(
      ({ start, end }, i) =>
        source.slice(start, end) === result.slice(formatted[i].start, formatted[i].end),
    );
  return (
    syntax(before.ast) === syntax(after.ast) &&
    sameText(before.tokens, after.tokens) &&
    sameText(before.comments, after.comments)
  );
}

function scriptNewline(source: string, tokens: Token[]): string {
  let previous = 0;
  for (const token of tokens) {
    const newline = source.slice(previous, token.start).match(lineBreak)?.[0];
    if (newline) return newline;
    previous = token.end;
  }
  return '\n';
}

function resolveGaps(source: string, gaps: Gap[], newline: string) {
  const anchors = [
    ...new Set(gaps.flatMap(({ layout }) => (typeof layout === 'string' ? [] : [layout.anchor]))),
  ].sort((a, b) => a - b);
  const indents = new Map<number, string>();
  const edits: FormatEdit[] = [];
  let nextAnchor = 0;
  let cursor = 0;
  let indent = '';
  let inIndent = true;
  let result = '';

  function emit(text: string) {
    result += text;
    const lastBreak = Math.max(
      text.lastIndexOf('\n'),
      text.lastIndexOf('\r'),
      text.lastIndexOf('\u2028'),
      text.lastIndexOf('\u2029'),
    );
    if (lastBreak >= 0) {
      indent = '';
      inIndent = true;
      text = text.slice(lastBreak + 1);
    }
    if (inIndent) {
      const leading = /^[ \t]*/.exec(text)![0];
      indent += leading;
      inIndent = leading.length === text.length;
    }
  }

  for (const { from, to, layout } of gaps.sort((a, b) => a.from - b.from || a.to - b.to)) {
    if (from < cursor) throw new Error('Overlapping JavaScript layout gaps');
    // A zero-width gap at an anchor inserts whitespace before its token.
    while (nextAnchor < anchors.length && anchors[nextAnchor] < from) {
      const anchor = anchors[nextAnchor++];
      emit(source.slice(cursor, anchor));
      indents.set(anchor, indent);
      cursor = anchor;
    }
    emit(source.slice(cursor, from));
    const old = source.slice(from, to);
    let insert: string;
    if (typeof layout === 'string') {
      insert = layout;
    } else {
      const base = indents.get(layout.anchor);
      if (base === undefined) throw new Error('Missing JavaScript indentation anchor');
      // Retain existing blank lines and line endings.
      const breaks = lineBreak.test(old) ? old.replace(/[^\S\r\n\u2028\u2029]+$/, '') : newline;
      insert = breaks + base + (layout.continuation ? '  ' : '');
    }
    if (old !== insert) edits.push({ from, to, insert });
    emit(insert);
    cursor = to;
  }
  return { edits, result: result + source.slice(cursor) };
}

function collectGaps(source: string, ast: AnyNode, tokens: Token[]): Gap[] {
  const gaps: Gap[] = [];

  function tokenAfter(pos: number): Token {
    let low = 0;
    let high = tokens.length - 1;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (tokens[mid].start < pos) low = mid + 1;
      else high = mid;
    }
    return tokens[low];
  }

  function gap(from: number, to: number, layout: Gap['layout']) {
    // A literal, operator, comment, or parenthesis between these boundaries stays put.
    if (from > to || !/^\s*$/.test(source.slice(from, to))) return false;
    gaps.push({ from, to, layout });
    return true;
  }

  function space(from: number, to: number, text: string) {
    if (!lineBreak.test(source.slice(from, to))) gap(from, to, text);
  }

  function group(open: Token, items: Node[], anchor: number, multiline: boolean) {
    if (!items.length) return;
    const itemLayout = multiline ? { anchor, continuation: true } : ' ';
    let previous = open.end;
    for (const [i, item] of items.entries()) {
      gap(previous, item.start, itemLayout);
      const next = tokenAfter(item.end);
      if (next.type === tokTypes.comma) {
        gap(item.end, next.start, '');
        previous = next.end;
      } else if (i === items.length - 1) {
        previous = item.end;
      } else {
        return;
      }
    }
    gap(previous, tokenAfter(previous).start, multiline ? { anchor, continuation: false } : ' ');
  }

  function object(node: ObjectExpression, multiline: boolean) {
    group(tokenAfter(node.start), node.properties, node.start, multiline);
    for (const property of node.properties) {
      if (
        property.type !== 'Property' ||
        property.computed ||
        property.method ||
        property.shorthand ||
        property.kind !== 'init'
      ) {
        continue;
      }
      const colon = tokenAfter(property.key.end);
      if (colon.type === tokTypes.colon) {
        space(property.key.end, colon.start, '');
        space(colon.end, property.value.start, ' ');
      }
    }
  }

  // Use the project's header grammar, including validation and the required leading position.
  const header = headerRanges(source);
  const first = ast.type === 'Program' ? ast.body[0] : undefined;
  const leadingDeclaration =
    first?.type === 'VariableDeclaration' &&
    first.kind === 'const' &&
    first.declarations.length === 1
      ? first.declarations[0]
      : undefined;
  function metadata(node: ObjectExpression) {
    if (!header || header.start !== node.start) return;
    object(node, true);
    for (const property of node.properties) {
      if (property.type !== 'Property' || property.value.type !== 'ArrayExpression') continue;
      const key = property.key;
      if (
        (key.type === 'Identifier' ? key.name : key.type === 'Literal' ? key.value : null) !==
        'sections'
      ) {
        continue;
      }
      const list = property.value;
      group(
        tokenAfter(list.start),
        list.elements.filter((item) => item !== null),
        list.start,
        true,
      );
      for (const item of list.elements) {
        if (item?.type !== 'ObjectExpression') continue;
        const simple = item.properties.every(
          (prop) =>
            prop.type === 'Property' &&
            (prop.value.type === 'Literal' || prop.value.type === 'UnaryExpression'),
        );
        // Existing multiline section entries can contain comments or intentional grouping.
        object(item, !simple || lineBreak.test(source.slice(item.start, item.end)));
      }
    }
  }

  function visitCall(node: Extract<AnyNode, { type: 'CallExpression' }>): boolean {
    let hasChain = false;
    for (const argument of node.arguments) {
      if (visit(argument)) hasChain = true;
    }
    if (node.callee.type !== 'Identifier') return hasChain;
    const [first, second] = node.arguments;
    if (
      node.callee.name === 'song' &&
      first?.type === 'Identifier' &&
      first.name === 'meta' &&
      second?.type === 'ObjectExpression'
    ) {
      const comma = tokenAfter(first.end);
      if (comma.type === tokTypes.comma) {
        space(first.end, comma.start, '');
        space(comma.end, second.start, ' ');
      }
      object(second, true);
    }
    if (hasChain && patternGroups.has(node.callee.name)) {
      const open = tokenAfter(node.callee.end);
      if (open.type === tokTypes.parenL) group(open, node.arguments, node.start, true);
    }
    return hasChain;
  }

  function spine(node: AnyNode, blocked = false): boolean {
    const methods: { from: number; dot: number }[] = [];
    let nested = false;
    let baseCall = false;
    let receiver = node;
    while (receiver.type === 'CallExpression' || receiver.type === 'MemberExpression') {
      blocked ||= receiver.optional;
      if (receiver.type === 'CallExpression') {
        if (visitCall(receiver)) nested = true;
        const callee = receiver.callee;
        if (callee.type !== 'MemberExpression') {
          baseCall = true;
          receiver = callee;
          continue;
        }
        const dot = tokenAfter(callee.object.end);
        if (!callee.optional && !callee.computed && dot.type === tokTypes.dot) {
          methods.push({ from: callee.object.end, dot: dot.start });
        }
        receiver = callee;
      } else {
        if (receiver.computed) {
          blocked = true;
          if (visit(receiver.property)) nested = true;
        }
        receiver = receiver.object;
      }
    }
    let unwrapped: AnyNode = receiver;
    while (unwrapped.type === 'ParenthesizedExpression') unwrapped = unwrapped.expression;
    const qualifies =
      !blocked && methods.length >= (baseCall || unwrapped.type === 'CallExpression' ? 1 : 2);
    let expanded = false;
    if (qualifies) {
      for (const { from, dot } of methods) {
        const planned = gap(from, dot, { anchor: receiver.start, continuation: true });
        if (planned || lineBreak.test(source.slice(from, dot))) expanded = true;
      }
    }
    return visit(receiver) || expanded || nested;
  }

  function visit(node: AnyNode): boolean {
    switch (node.type) {
      case 'TaggedTemplateExpression':
      case 'TemplateLiteral':
        return false;
      case 'ChainExpression':
        return spine(node.expression, true);
      case 'CallExpression':
      case 'MemberExpression':
        return spine(node);
      case 'VariableDeclarator':
        if (
          node === leadingDeclaration &&
          node.id.type === 'Identifier' &&
          source.slice(node.id.start, node.id.end) === 'meta' &&
          node.init?.type === 'ObjectExpression'
        ) {
          if (node.init.start === header?.start) {
            const equals = tokenAfter(node.id.end);
            if (equals.type === tokTypes.eq) {
              space(node.id.end, equals.start, ' ');
              space(equals.end, node.init.start, ' ');
            }
            metadata(node.init);
          }
          return false;
        }
        break;
    }
    let hasChain = false;
    for (const value of Object.values(node)) {
      for (const child of Array.isArray(value) ? value : [value]) {
        if (child && typeof child.type === 'string' && visit(child)) hasChain = true;
      }
    }
    return hasChain;
  }

  visit(ast);
  return gaps;
}

export function formatScript(source: string): FormatEdit[] {
  try {
    const script = readScript(source);
    const gaps = collectGaps(source, script.ast, script.tokens);
    const { edits, result } = resolveGaps(source, gaps, scriptNewline(source, script.tokens));
    return edits.length && sameSyntax(source, script, result) ? edits : [];
  } catch {
    return [];
  }
}

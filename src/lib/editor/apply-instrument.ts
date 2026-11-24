import { parse, type AnyNode, type Comment, type Node, type Pattern } from 'acorn';
import { miniDisabledRanges } from './format-song';
import { scriptOptions, scriptShape } from './format-script';

const options = { ...scriptOptions, preserveParens: true };
const patternExpressions = new Set([
  'Identifier',
  'CallExpression',
  'MemberExpression',
  'ConditionalExpression',
  'LogicalExpression',
]);

function childNodes(node: Node): AnyNode[] {
  return Object.values(node)
    .flat()
    .filter((value): value is AnyNode => value != null && typeof value.type === 'string');
}

function isNamed(pattern: Pattern | null | undefined, name: string): boolean {
  if (!pattern) return false;
  switch (pattern.type) {
    case 'Identifier':
      return pattern.name === name;
    case 'RestElement':
      return isNamed(pattern.argument, name);
    case 'AssignmentPattern':
      return isNamed(pattern.left, name);
    case 'ArrayPattern':
      return pattern.elements.some((item) => isNamed(item, name));
    case 'ObjectPattern':
      return pattern.properties.some((item) =>
        isNamed(item.type === 'RestElement' ? item.argument : item.value, name),
      );
    default:
      return false;
  }
}

type Visit = {
  node: AnyNode;
  parent?: Visit;
  lexical: AnyNode;
  variable: AnyNode;
};

function scopeForChildren({ node, lexical, variable }: Visit) {
  switch (node.type) {
    case 'FunctionDeclaration':
    case 'FunctionExpression':
    case 'ArrowFunctionExpression':
    case 'StaticBlock':
      return { lexical: node, variable: node };
    case 'BlockStatement':
    case 'CatchClause':
    case 'ForStatement':
    case 'ForInStatement':
    case 'ForOfStatement':
    case 'SwitchStatement':
      return { lexical: node, variable };
    default:
      return { lexical, variable };
  }
}

function instrumentBindingScopes({ node, lexical, variable }: Visit): AnyNode[] {
  switch (node.type) {
    case 'VariableDeclaration':
      return node.declarations.some((decl) => isNamed(decl.id, 'instruments'))
        ? [node.kind === 'var' ? variable : lexical]
        : [];
    case 'ClassDeclaration':
      return isNamed(node.id, 'instruments') ? [lexical] : [];
    case 'ClassExpression':
      return isNamed(node.id, 'instruments') ? [node] : [];
    case 'FunctionDeclaration':
    case 'FunctionExpression':
    case 'ArrowFunctionExpression': {
      if (node.type === 'FunctionDeclaration' && isNamed(node.id, 'instruments')) return [lexical];
      const binds =
        isNamed(node.id, 'instruments') || node.params.some((p) => isNamed(p, 'instruments'));
      return binds ? [node] : [];
    }
    case 'CatchClause':
      return isNamed(node.param, 'instruments') ? [node] : [];
    case 'WithStatement':
      return [node.body];
    default:
      return [];
  }
}

function isPatternSelection(
  { node, parent }: Visit,
  miniDisabled: ReturnType<typeof miniDisabledRanges>,
): boolean {
  let expression = node;
  while (expression.type === 'ParenthesizedExpression' || expression.type === 'ChainExpression') {
    expression = expression.expression;
  }
  let selected = node;
  while (
    parent?.node.type === 'ParenthesizedExpression' ||
    parent?.node.type === 'ChainExpression'
  ) {
    selected = parent.node;
    parent = parent.parent;
  }
  const enclosing = parent?.node;
  if (
    ((enclosing?.type === 'CallExpression' || enclosing?.type === 'NewExpression') &&
      enclosing.callee === selected) ||
    (enclosing?.type === 'TaggedTemplateExpression' && enclosing.tag === selected)
  )
    return false;
  if (expression.type === 'NewExpression') {
    return expression.callee.type === 'Identifier' && expression.callee.name === 'Pattern';
  }
  if (
    enclosing?.type === 'ExpressionStatement' &&
    (enclosing.directive === 'use strict' || enclosing.directive === 'use asm')
  ) {
    return false;
  }
  const mini =
    (expression.type === 'Literal' && expression.raw?.startsWith('"')) ||
    (expression.type === 'TemplateLiteral' && expression.expressions.length === 0);
  if (mini) {
    return !miniDisabled.some(
      ([start, end]) => start <= expression.start && expression.start < end,
    );
  }
  return patternExpressions.has(expression.type);
}

/** Replace one whole expression, keeping the surrounding script's parse tree intact. */
export function instrumentApplication(source: string, from: number, to: number, id: string) {
  if (
    !Number.isInteger(from) ||
    !Number.isInteger(to) ||
    from < 0 ||
    from >= to ||
    to > source.length
  ) {
    throw new Error('Select a complete pattern expression in the editor.');
  }

  const comments: Comment[] = [];
  let script;
  try {
    script = parse(source, { ...options, onComment: comments });
  } catch {
    throw new Error('Fix the song syntax before applying an instrument.');
  }
  const selection = source.slice(from, to);
  const expression = selection.trim();
  const start = from + selection.length - selection.trimStart().length;
  const end = start + expression.length;
  let target: Visit | undefined;
  const miniDisabled = miniDisabledRanges(comments, source.length);
  const pending: Visit[] = [{ node: script, lexical: script, variable: script }];
  while (pending.length) {
    const visit = pending.pop()!;
    const { node } = visit;
    if (instrumentBindingScopes(visit).some((scope) => scope.start <= start && scope.end >= end)) {
      throw new Error('The selected expression has its own instruments namespace.');
    }
    if (node.start === start && node.end === end && isPatternSelection(visit, miniDisabled)) {
      target = visit;
    }
    // Static metadata values are read independently of Strudel.
    if (
      node.type === 'VariableDeclarator' &&
      visit.parent?.parent?.node === script &&
      node.id.type === 'Identifier' &&
      node.id.name === 'meta'
    )
      continue;
    const scopes = scopeForChildren(visit);
    for (const child of childNodes(node)) {
      pending.push({ node: child, parent: visit, ...scopes });
    }
  }
  if (!target) throw new Error('Select a complete pattern expression in the editor.');

  if (!/^[a-z][a-z0-9_]{0,63}$/.test(id)) throw new Error('Invalid instrument ID.');
  const insert = `(${expression}).apply(instruments.${id}())`;
  const replacement = parse(insert, options).body[0];
  if (replacement?.type !== 'ExpressionStatement') throw new Error('Select a pattern expression.');

  let rewritten;
  try {
    rewritten = parse(source.slice(0, start) + insert + source.slice(end), options);
  } catch {
    throw new Error('The selection cannot be replaced with a pattern here.');
  }
  const parent = target.parent?.node;
  // Acorn marks a leading quoted mini pattern as a directive. The transpiler plays it.
  if (parent?.type === 'ExpressionStatement') delete parent.directive;
  if (
    scriptShape(script, { target: target.node, expression: replacement.expression }) !==
    scriptShape(rewritten)
  ) {
    throw new Error('The selection cannot be replaced with a pattern here.');
  }
  return { from: start, to: end, insert };
}

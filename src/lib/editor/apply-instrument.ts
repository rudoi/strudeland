import { parse, type AnyNode, type Node, type Pattern } from 'acorn';
import { scriptOptions, scriptShape } from './format-script';

const options = { ...scriptOptions, preserveParens: true };
const patternExpressions = new Set([
  'Identifier',
  'CallExpression',
  'MemberExpression',
  'ParenthesizedExpression',
  'ConditionalExpression',
  'LogicalExpression',
  'ChainExpression',
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

function shadowsInstruments(node: AnyNode): boolean {
  switch (node.type) {
    case 'VariableDeclarator':
    case 'ClassDeclaration':
    case 'ClassExpression':
      return isNamed(node.id, 'instruments');
    case 'FunctionDeclaration':
    case 'FunctionExpression':
    case 'ArrowFunctionExpression':
      return isNamed(node.id, 'instruments') || node.params.some((p) => isNamed(p, 'instruments'));
    case 'CatchClause':
      return isNamed(node.param, 'instruments');
    case 'WithStatement':
      return true;
    default:
      return false;
  }
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

  let script;
  try {
    script = parse(source, options);
  } catch {
    throw new Error('Fix the song syntax before applying an instrument.');
  }
  const selection = source.slice(from, to);
  const expression = selection.trim();
  const start = from + selection.length - selection.trimStart().length;
  const end = start + expression.length;
  let target: AnyNode | undefined;
  const pending: AnyNode[] = [...script.body];
  while (pending.length) {
    const node = pending.pop()!;
    if (shadowsInstruments(node)) {
      throw new Error('The song declares its own instruments namespace.');
    }
    if (node.start === start && node.end === end && patternExpressions.has(node.type)) {
      target = node;
    }
    // Static metadata values are read independently of Strudel.
    if (node.type !== 'VariableDeclarator' || !isNamed(node.id, 'meta')) {
      pending.push(...childNodes(node));
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
  if (
    scriptShape(script, { target, expression: replacement.expression }) !== scriptShape(rewritten)
  ) {
    throw new Error('The selection cannot be replaced with a pattern here.');
  }
  return { from: start, to: end, insert };
}

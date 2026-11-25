import { parse, type AnyNode, type Comment, type Pattern } from 'acorn';
import { getLanguages } from '@strudel/transpiler';
import { miniDisabledRanges } from '../editor/format-song';
import { scriptOptions, scriptShape } from '../editor/format-script';
import {
  matchesPitchSelection,
  planPhraseEdit,
  readPhrase,
  type AuthoredPhrase,
  type PhraseInput,
} from './phrase';
import type { RollDefinition, RollEdit, RollPatch, RollTarget, RollUse, SourceSpan } from './types';

type Scope = {
  parent?: Scope;
  bindings: Map<string, Binding>;
};

type Visit = {
  node: AnyNode;
  parent?: Visit;
  key?: string;
  lexical: Scope;
  variable: Scope;
};

type Binding = {
  name: string;
  declaration: AnyNode;
  initializer?: AnyNode;
};

type Script = ReturnType<typeof readScript>;

const spanOf = ({ start, end }: AnyNode): SourceSpan => ({ from: start, to: end });
const exact = (node: AnyNode, { from, to }: SourceSpan) => node.start === from && node.end === to;
const within = (node: AnyNode, { from, to }: SourceSpan) => node.start <= from && node.end >= to;

function ungroup(node: AnyNode): AnyNode {
  while (node.type === 'ParenthesizedExpression') node = node.expression;
  return node;
}

/** Binding identifiers are tracked separately from shorthand object keys and references. */
function bind(
  node: Pattern | null | undefined,
  scope: Scope,
  declarations: Set<AnyNode>,
  initializer?: AnyNode,
): void {
  if (!node) return;
  switch (node.type) {
    case 'Identifier':
      declarations.add(node);
      scope.bindings.set(node.name, { name: node.name, declaration: node, initializer });
      break;
    case 'RestElement':
      bind(node.argument, scope, declarations);
      break;
    case 'AssignmentPattern':
      bind(node.left, scope, declarations);
      break;
    case 'ArrayPattern':
      for (const element of node.elements) bind(element, scope, declarations);
      break;
    case 'ObjectPattern':
      for (const property of node.properties) {
        bind(
          property.type === 'RestElement' ? property.argument : property.value,
          scope,
          declarations,
        );
      }
      break;
  }
}

function readScript(source: string) {
  const comments: Comment[] = [];
  const ast = parse(source, {
    ...scriptOptions,
    preserveParens: true,
    onComment: comments,
  });
  const visits = new Map<AnyNode, Visit>();
  const declarations = new Set<AnyNode>();
  const scope: Scope = { bindings: new Map() };
  const definitions = new Map<AnyNode, Binding>();
  let staticSafe = true;

  function walk(visit: Visit) {
    const { node } = visit;
    visits.set(node, visit);
    let { lexical, variable } = visit;
    const childScope = (): Scope => ({ parent: lexical, bindings: new Map() });
    switch (node.type) {
      case 'VariableDeclaration':
        for (const declaration of node.declarations) {
          const initializer =
            node.kind === 'const' && declaration.init ? ungroup(declaration.init) : undefined;
          const target = node.kind === 'var' ? variable : lexical;
          bind(declaration.id, target, declarations, initializer);
          if (initializer && declaration.id.type === 'Identifier') {
            definitions.set(initializer, target.bindings.get(declaration.id.name)!);
          }
        }
        break;
      case 'FunctionDeclaration':
      case 'FunctionExpression':
      case 'ArrowFunctionExpression':
        if (node.type === 'FunctionDeclaration') bind(node.id, lexical, declarations);
        lexical = variable = childScope();
        if (node.type === 'FunctionExpression') bind(node.id, lexical, declarations);
        for (const parameter of node.params) bind(parameter, lexical, declarations);
        break;
      case 'ClassDeclaration':
      case 'ClassExpression':
        if (node.type === 'ClassDeclaration') bind(node.id, lexical, declarations);
        lexical = childScope();
        if (node.type === 'ClassExpression') bind(node.id, lexical, declarations);
        break;
      case 'StaticBlock':
        lexical = variable = childScope();
        break;
      case 'CatchClause':
        lexical = childScope();
        bind(node.param, lexical, declarations);
        break;
      case 'BlockStatement':
      case 'ForStatement':
      case 'ForOfStatement':
      case 'ForInStatement':
      case 'SwitchStatement':
        lexical = childScope();
        break;
      case 'WithStatement':
        staticSafe = false;
        break;
      case 'CallExpression': {
        if (node.callee.type !== 'Identifier') break;
        if (node.callee.name === 'eval') staticSafe = false;
        if (node.callee.name === 'register' || node.callee.name === 'registerControl') {
          const registered = node.arguments[0];
          if (
            registered?.type !== 'Literal' ||
            typeof registered.value !== 'string' ||
            ['note', 'm', 's', 'sound'].includes(registered.value) ||
            neutralControls.has(registered.value)
          ) {
            staticSafe = false;
          }
        }
        break;
      }
      case 'AssignmentExpression':
      case 'UpdateExpression': {
        const target = node.type === 'AssignmentExpression' ? node.left : node.argument;
        const identifier = target.type === 'MemberExpression' ? target.property : target;
        const name =
          identifier.type === 'Identifier'
            ? identifier.name
            : identifier.type === 'Literal'
              ? identifier.value
              : null;
        if (name === 'note' || name === 'm' || name === 's' || name === 'sound') staticSafe = false;
        break;
      }
    }
    for (const [key, value] of Object.entries(node)) {
      for (const child of Array.isArray(value) ? value : [value]) {
        if (child && typeof child.type === 'string') {
          walk({ node: child, parent: visit, key, lexical, variable });
        }
      }
    }
  }
  walk({ node: ast, lexical: scope, variable: scope });
  return {
    ast,
    comments,
    declarations,
    visits,
    definitions,
    staticSafe,
    miniDisabled: miniDisabledRanges(comments, source.length),
  };
}

function lookup({ lexical }: Visit, name: string): Binding | undefined {
  for (let scope: Scope | undefined = lexical; scope; scope = scope.parent) {
    const binding = scope.bindings.get(name);
    if (binding) return binding;
  }
}

function isReference({ node, parent, key }: Visit, script: Script): boolean {
  if (node.type !== 'Identifier' || script.declarations.has(node)) return false;
  const enclosing = parent?.node;
  if (!enclosing) return false;
  if (
    enclosing.type === 'LabeledStatement' ||
    enclosing.type === 'BreakStatement' ||
    enclosing.type === 'ContinueStatement'
  ) {
    return key !== 'label';
  }
  if (enclosing.type === 'MemberExpression' && key === 'property') return enclosing.computed;
  if (
    (enclosing.type === 'Property' ||
      enclosing.type === 'MethodDefinition' ||
      enclosing.type === 'PropertyDefinition') &&
    key === 'key'
  ) {
    return enclosing.computed;
  }
  return true;
}

function unshadowed(node: AnyNode, script: Script): node is AnyNode & { name: string } {
  return node.type === 'Identifier' && !lookup(script.visits.get(node)!, node.name);
}

function callName(node: AnyNode, script: Script): string | undefined {
  if (node.type !== 'CallExpression' || node.optional) return;
  const callee = ungroup(node.callee);
  if (unshadowed(callee, script)) return callee.name;
}

function method(node: AnyNode) {
  if (node.type !== 'CallExpression' || node.optional) return;
  const callee = ungroup(node.callee);
  if (
    callee.type === 'MemberExpression' &&
    !callee.optional &&
    !callee.computed &&
    callee.property.type === 'Identifier'
  ) {
    return { name: callee.property.name, receiver: ungroup(callee.object) };
  }
}

function usesMini(visit: Visit, script: Script) {
  return (
    !lookup(visit, 'm') &&
    !getLanguages().has('minilang') &&
    !script.miniDisabled.some(([from, to]) => visit.node.start >= from && visit.node.start < to)
  );
}

function literal(node: AnyNode, script: Script, source: string): PhraseInput | undefined {
  const raw = source.slice(node.start, node.end);
  const span = spanOf(node);
  if (node.type === 'Literal' && typeof node.value === 'number' && /^\d+$/.test(raw)) {
    return { kind: 'numeric', span, body: raw };
  }
  if (
    raw.includes('\\') ||
    raw.includes('\r') ||
    (node.type !== 'TemplateLiteral' &&
      !(node.type === 'Literal' && typeof node.value === 'string'))
  )
    return;
  if (node.type === 'TemplateLiteral' && node.expressions.length) return;
  if (raw[0] === "'") return { kind: 'plain', span, body: raw.slice(1, -1) };
  if (!usesMini(script.visits.get(node)!, script)) return;
  return { kind: 'mini', span, body: raw.slice(1, -1) };
}

const neutralControls = new Set([
  'attack',
  'decay',
  'sustain',
  'release',
  'gain',
  'pan',
  'lpf',
  'hpf',
  'orbit',
  'clip',
  'room',
  'roomsize',
  'delay',
  'delaytime',
  'delayfeedback',
  'velocity',
  'postgain',
]);

function soundName(node: AnyNode | undefined, script: Script, source: string): string | undefined {
  if (!node) return;
  const input = literal(node, script, source);
  if (input && /^[a-z][a-z0-9_-]*$/i.test(input.body)) return input.body.toLowerCase();
}

function authoredInput(node: AnyNode, script: Script, source: string): PhraseInput | undefined {
  if (!script.staticSafe) return;
  if (node.type === 'Literal' || node.type === 'TemplateLiteral') {
    if (pitchProducer(node, script)?.nested) return literal(node, script, source);
    return;
  }
  if (node.type !== 'CallExpression' || node.arguments.length !== 1) return;
  const argument = ungroup(node.arguments[0]);
  const chain = method(node);
  if (callName(node, script) === 'note') return literal(argument, script, source);
  if (!chain) return;
  // Receiver-based note setters inherit timing from their receiver. Query them in the host.
  if (chain.name === 's' || chain.name === 'sound') {
    const sound = soundName(argument, script, source);
    const input = sound && authoredInput(chain.receiver, script, source);
    if (input) return { ...input, sound };
  } else if (
    neutralControls.has(chain.name) &&
    argument.type === 'Literal' &&
    typeof argument.value === 'number'
  ) {
    return authoredInput(chain.receiver, script, source);
  }
}

function phraseAt(node: AnyNode, script: Script, source: string): AuthoredPhrase | undefined {
  if (destination(script.visits.get(node)!, script).incomplete) return;
  const input = authoredInput(node, script, source);
  return input && readPhrase(input);
}

function resolveAlias(binding: Binding | undefined, script: Script) {
  const seen = new Set<Binding>();
  let transparent = true;
  while (binding?.initializer) {
    if (seen.has(binding)) return;
    seen.add(binding);
    const node = binding.initializer;
    const next = receiverBinding(node, script, true);
    if (!next?.initializer || next.declaration.start > node.start) return { binding, transparent };
    transparent &&= node.type === 'Identifier';
    binding = next;
  }
}

function receiverBinding(node: AnyNode, script: Script, knownOnly = false): Binding | undefined {
  // A transform of a known binding can lead back to its source, without editing the transform.
  for (;;) {
    if (node.type === 'Identifier') return lookup(script.visits.get(node)!, node.name);
    const chain = method(node);
    if (!chain || (knownOnly && !knownTransforms.has(chain.name))) return;
    node = chain.receiver;
  }
}

const valueContainers = new Set(['stack', 'cat', 'seq', 'sequence', 'fastcat', 'slowcat']);

/** A sequence may contain direct pitches before the surrounding note call tags the values. */
function pitchProducer(node: AnyNode, script: Script) {
  let nested = false;
  for (let at = script.visits.get(node); at?.parent; at = at.parent) {
    const parent = at.parent.node;
    if (parent.type === 'ParenthesizedExpression') continue;
    if (parent.type !== 'CallExpression' || at.key !== 'arguments') return;
    const name = callName(parent, script);
    if (name === 'note' && parent.arguments.length === 1) return { owner: parent, nested };
    if (method(parent)?.name === 'note' && parent.arguments.length === 1) {
      return { owner: parent, nested: false };
    }
    if (!name || !valueContainers.has(name)) return;
    nested = true;
  }
}
const knownTransforms = new Set([
  ...neutralControls,
  's',
  'sound',
  'transpose',
  'fast',
  'slow',
  'rev',
  'mask',
  'clip',
  'off',
  'striate',
  'early',
  'late',
  'degrade',
  'degradeBy',
  'ply',
  'sometimes',
  'every',
  'struct',
  'legato',
  'scale',
  'fit',
  'add',
  'sub',
]);

function sectionAtProperty(property: Visit, script: Script): string | undefined {
  const { node } = property;
  const outer = property.parent;
  const call = outer?.parent?.node;
  if (
    node.type !== 'Property' ||
    node.computed ||
    outer?.node.type !== 'ObjectExpression' ||
    call?.type !== 'CallExpression' ||
    call.arguments[1] !== outer.node ||
    callName(call, script) !== 'song'
  )
    return;
  if (node.key.type === 'Identifier') return node.key.name;
  if (node.key.type === 'Literal' && typeof node.key.value === 'string') return node.key.value;
}

/** Trace a proven data-flow path from a pattern value to a section, alias or final expression. */
function destination(visit: Visit, script: Script) {
  for (let at: Visit = visit; at.parent; at = at.parent) {
    const { node } = at.parent;
    if (node.type === 'Property' && at.key === 'value') {
      const section = sectionAtProperty(at.parent, script);
      return section === undefined ? { incomplete: true } : { section };
    }
    if (node.type === 'VariableDeclarator' && at.key === 'init') {
      const alias = script.definitions.get(ungroup(at.node));
      return alias ? { alias } : { incomplete: true };
    }
    if (node.type === 'ExpressionStatement') return {};
    if (node.type === 'ParenthesizedExpression' || node.type === 'ChainExpression') continue;
    if (node.type === 'CallExpression') {
      const name = callName(node, script);
      if (name && (valueContainers.has(name) || (name === 'note' && at.key === 'arguments')))
        continue;
      const transform = method(node);
      if (at.key === 'callee' && transform && knownTransforms.has(transform.name)) continue;
    }
    if (node.type === 'MemberExpression' && at.key === 'object') continue;
    return { incomplete: true };
  }
  return {};
}

/** Collect written references through immutable aliases without treating same-spelled names as uses. */
function definitionUses(
  root: Binding,
  script: Script,
): Pick<RollDefinition, 'uses' | 'incomplete'> {
  const visitsByBinding = new Map<Binding, Visit[]>();
  for (const visit of script.visits.values()) {
    if (!isReference(visit, script) || visit.node.type !== 'Identifier') continue;
    const binding = lookup(visit, visit.node.name);
    if (!binding) continue;
    const list = visitsByBinding.get(binding);
    if (list) list.push(visit);
    else visitsByBinding.set(binding, [visit]);
  }
  const known = new Map<Binding, string[]>();
  const uses: RollUse[] = [];
  let incomplete = !script.staticSafe;
  function collect(binding: Binding): string[] {
    const existing = known.get(binding);
    if (existing) return existing;
    const sections: string[] = [];
    known.set(binding, sections);
    for (const ref of visitsByBinding.get(binding) ?? []) {
      const next = destination(ref, script);
      if (next.incomplete) incomplete = true;
      const local =
        next.section !== undefined ? [next.section] : next.alias ? collect(next.alias) : [];
      uses.push({ span: spanOf(ref.node), sections: local });
      for (const section of local) if (!sections.includes(section)) sections.push(section);
    }
    return sections;
  }
  collect(root);
  return { uses: uses.sort((a, b) => a.span.from - b.span.from), incomplete };
}

function trimSelection(source: string, selection: SourceSpan, comments: Comment[]): SourceSpan {
  let { from, to } = selection;
  for (;;) {
    while (from < to && /\s/.test(source[from])) from++;
    while (from < to && /\s/.test(source[to - 1])) to--;
    const first = comments.find((comment) => comment.start === from && comment.end <= to);
    const last = comments.find((comment) => comment.end === to && comment.start >= from);
    if (!first && !last) return { from, to };
    if (first) from = first.end;
    if (last) to = last.start;
    if (from >= to) return { from, to };
  }
}

function expressionIsValue(visit: Visit, script: Script): boolean {
  const { node, parent, key } = visit;
  if (!parent) return false;
  if (
    key === 'callee' ||
    (parent.node.type === 'TaggedTemplateExpression' && key === 'tag') ||
    (parent.node.type === 'MemberExpression' && key === 'property' && !parent.node.computed) ||
    (parent.node.type === 'AssignmentExpression' && key === 'left') ||
    parent.node.type === 'UpdateExpression' ||
    parent.node.type === 'ObjectPattern' ||
    (parent.node.type === 'Property' && key === 'key')
  )
    return false;
  return node.type !== 'Identifier' || isReference(visit, script);
}

function liftLiteral(script: Script, literalNode: AnyNode, source: string): AnyNode | undefined {
  if (phraseAt(literalNode, script, source)) return literalNode;
  return pitchProducer(literalNode, script)?.owner;
}

const inspectedExpressions = new Set([
  'Identifier',
  'CallExpression',
  'MemberExpression',
  'NewExpression',
  'ChainExpression',
  'ConditionalExpression',
  'LogicalExpression',
  'TaggedTemplateExpression',
]);

function selectedNode(source: string, selection: SourceSpan, script: Script): AnyNode | undefined {
  for (const { node } of script.visits.values()) {
    if (!exact(node, selection)) continue;
    let candidate = ungroup(node);
    while (candidate.type === 'LabeledStatement') candidate = candidate.body;
    if (candidate.type === 'ExpressionStatement' && !candidate.directive)
      candidate = ungroup(candidate.expression);
    if (candidate.type === 'VariableDeclaration' && candidate.declarations.length === 1) {
      const init = candidate.declarations[0].init;
      if (init) candidate = ungroup(init);
    }
    if (candidate.type === 'Identifier' && script.declarations.has(candidate)) {
      const definition = [...script.definitions.values()].find(
        (binding) => binding.declaration === candidate,
      );
      if (definition) return definition.initializer;
    }
    if (candidate.type === 'Literal' || candidate.type === 'TemplateLiteral') {
      const enclosing = liftLiteral(script, candidate, source);
      if (!enclosing) continue;
      if (enclosing === candidate) return candidate;
      candidate = enclosing;
    }
    if (
      inspectedExpressions.has(candidate.type) &&
      expressionIsValue(script.visits.get(candidate)!, script)
    ) {
      // A numeric sequence has no pitch tags until its enclosing note call is applied.
      return pitchProducer(candidate, script)?.owner ?? candidate;
    }
  }
  // Narrow selection within a mini literal must start and end on complete pitch atoms.
  for (const { node } of script.visits.values()) {
    if ((node.type !== 'Literal' && node.type !== 'TemplateLiteral') || !within(node, selection))
      continue;
    const expression = liftLiteral(script, node, source);
    if (!expression) continue;
    const input = literal(node, script, source);
    if (input && matchesPitchSelection(input, selection)) return expression;
  }
}

function locateTarget(source: string, selection: SourceSpan) {
  if (
    !Number.isInteger(selection.from) ||
    !Number.isInteger(selection.to) ||
    selection.from < 0 ||
    selection.from >= selection.to ||
    selection.to > source.length
  )
    return;
  const script = readScript(source);
  const trimmed = trimSelection(source, selection, script.comments);
  if (trimmed.from >= trimmed.to) return;
  const node = selectedNode(source, trimmed, script);
  if (node) return { node, script };
}

export function resolveRollTarget(source: string, selection: SourceSpan): RollTarget | null {
  try {
    const selected = locateTarget(source, selection);
    if (!selected) return null;
    const { node, script } = selected;
    const expression = spanOf(node);
    const direct = destination(script.visits.get(node)!, script);
    const authored = phraseAt(node, script, source);
    const resolved = resolveAlias(
      script.definitions.get(node) ?? receiverBinding(node, script) ?? (authored && direct.alias),
      script,
    );
    const binding = resolved?.binding;
    const init = binding?.initializer;
    const fullyAuthored = init && (init === node ? authored : phraseAt(init, script, source));
    const authoredDefinition = fullyAuthored
      ? init
      : authored && direct.alias === binding
        ? node
        : init;
    const definition =
      binding && authoredDefinition
        ? {
            name: binding.name,
            expression: spanOf(authoredDefinition),
            phrase: fullyAuthored || (authoredDefinition === node ? authored : undefined),
            ...definitionUses(binding, script),
          }
        : undefined;
    const directReference =
      node.type === 'Identifier' &&
      resolved?.transparent &&
      binding &&
      binding.declaration.start < node.start;
    const phrase = directReference ? definition?.phrase : authored;
    const sections =
      direct.section !== undefined
        ? [direct.section]
        : (definition?.uses.find(({ span }) => span.from === node.start && span.to === node.end)
            ?.sections ??
          (init === node || direct.alias === binding
            ? [...new Set(definition?.uses.flatMap((use) => use.sections))]
            : []));
    const label = source
      .slice(expression.from, expression.to)
      .trim()
      .split(/\s*\r?\n\s*/u, 1)[0];
    return { expression, label, phrase, sections, definition };
  } catch {
    return null;
  }
}

export function planRollEdit(
  source: string,
  target: SourceSpan,
  edit: RollEdit,
): readonly RollPatch[] | null {
  try {
    const selected = locateTarget(source, target);
    if (!selected || !exact(selected.node, target)) return null;
    const { node, script } = selected;
    const phrase = phraseAt(node, script, source);
    if (!phrase) return null;
    const patch = planPhraseEdit(source, phrase, edit);
    if (!patch) return null;
    const nextSource = source.slice(0, patch.from) + patch.insert + source.slice(patch.to);
    const next = readScript(nextSource);
    const input = [...script.visits.keys()].find((candidate) =>
      exact(candidate, phrase.input.span),
    );
    if (!input) return null;
    const nextNode = [...next.visits.keys()].find((candidate) =>
      exact(candidate, {
        from: phrase.input.span.from,
        to: phrase.input.span.to + patch.insert.length - (patch.to - patch.from),
      }),
    );
    if (nextNode?.type !== input.type) return null;
    const replacement = parse('undefined', scriptOptions).body[0];
    return scriptShape(script.ast, { target: input, expression: replacement }) ===
      scriptShape(next.ast, { target: nextNode, expression: replacement })
      ? [patch]
      : null;
  } catch {
    return null;
  }
}

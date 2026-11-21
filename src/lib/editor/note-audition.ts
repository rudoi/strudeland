import { ensureSyntaxTree } from '@codemirror/language';
import type { EditorState, Transaction } from '@codemirror/state';
import { noteToMidi } from '@strudel/core';
import { mini2ast } from '@strudel/mini';
import { isAuditionSound } from '../audio/audition-sound';
import type { AuditionRequest } from '../audio/note-audition';

type Tree = NonNullable<ReturnType<typeof ensureSyntaxTree>>;
type SyntaxNode = Tree['topNode'];
type Span = { from: number; to: number };
export type NoteAuditionEdit = AuditionRequest & Span & { kind: 'note' | 'confirm' };

type VoicePitch = Span & { pitch: number };
type MiniOp = { type_: string; arguments_: { amount?: MiniNode | number } };
type MiniNode =
  | { type_: 'atom'; source_: string; location_: { start: { offset: number } } }
  | { type_: 'element'; source_: MiniNode; options_: { ops: MiniOp[]; weight: number } }
  | { type_: 'pattern'; source_: MiniNode[]; arguments_: { stepsPerCycle?: MiniNode } };

/** Only source children produce pattern values. Operator arguments have their own atom locations. */
function* pitchedAtoms(node: MiniNode, quoted: string): Generator<VoicePitch> {
  if (node.type_ === 'pattern') {
    for (const child of node.source_) yield* pitchedAtoms(child, quoted);
  } else if (node.type_ !== 'atom') {
    yield* pitchedAtoms(node.source_, quoted);
  } else if (/^[a-gA-G][#bsf]*-?\d+$/.test(node.source_)) {
    const pitch = noteToMidi(node.source_);
    if (pitch >= 0 && pitch <= 127) {
      // Krill includes whitespace before a step that follows certain modifiers.
      const from = quoted.indexOf(node.source_, node.location_.start.offset);
      yield { from, to: from + node.source_.length, pitch };
    }
  }
}

const nodeText = (state: EditorState, node: SyntaxNode) => state.sliceDoc(node.from, node.to);
const isComment = (name: string) => name === 'LineComment' || name === 'BlockComment';

/** Mirror the transpiler's nested mini-off / mini-on comment directives. */
function miniEnabled(tree: Tree, state: EditorState, position: number) {
  let depth = 0;
  tree.iterate({
    to: position,
    enter(node) {
      if (!isComment(node.name)) return;
      const comment = state.sliceDoc(node.from + 2, node.to).trimStart();
      if (comment.startsWith('mini-off')) depth++;
      if (comment.startsWith('mini-on') && depth) depth--;
    },
  });
  return depth === 0;
}

function literal(state: EditorState, node: SyntaxNode, allowUnclosed = false) {
  if (node.name !== 'String' && node.name !== 'TemplateString') return null;
  const raw = nodeText(state, node);
  const quote = raw[0];
  if (raw.includes('\\') || (quote === '`' && raw.includes('${'))) return null;
  if (raw.length > 1 && raw.endsWith(quote)) return raw.slice(1, -1);
  return allowUnclosed ? raw.slice(1) : null;
}

function onlyArgument(list: SyntaxNode | null) {
  if (list?.name !== 'ArgList') return null;
  let arg: SyntaxNode | null = null;
  for (let node = list.firstChild; node; node = node.nextSibling) {
    // A zero-width error can stand in for the closing ')' while writing the call.
    if (
      node.name === '(' ||
      node.name === ')' ||
      isComment(node.name) ||
      (node.type.isError && node.from === node.to)
    )
      continue;
    if (arg) return null;
    arg = node;
  }
  return arg;
}

function unwrapped(node: SyntaxNode | null): SyntaxNode | null {
  while (node?.name === 'ParenthesizedExpression') node = node.firstChild?.nextSibling ?? null;
  return node;
}
function outerCall(note: SyntaxNode) {
  let node = note;
  for (;;) {
    if (node.parent?.name === 'ParenthesizedExpression') {
      node = node.parent;
      continue;
    }
    const member = node.parent;
    if (member?.name !== 'MemberExpression' || member.firstChild?.from !== node.from) return node;
    if (member.parent?.name !== 'CallExpression') return null;
    node = member.parent;
  }
}

const voiceControls = new Set(['attack', 'decay', 'sustain', 'release', 'gain']);
// These calls keep the voice's source and base pitch, including the effects in project templates.
const ignoredCalls = new Set([
  'pan',
  'lpf',
  'hpf',
  'room',
  'penv',
  'pdecay',
  'fast',
  'slow',
  'orbit',
  'clip',
  'delay',
  'delaytime',
  'delayfeedback',
  'velocity',
  'postgain',
  'roomsize',
]);
const containers = new Set(['song', 'stack', 'cat', 'seq', 'fastcat', 'slowcat']);

function singleSound(node: MiniNode): string | null {
  if (node.type_ === 'atom') return node.source_;
  if (node.type_ === 'pattern') {
    return node.source_.length === 1 && !node.arguments_.stepsPerCycle
      ? singleSound(node.source_[0])
      : null;
  }
  for (const op of node.options_.ops) {
    if (op.type_ === 'replicate') continue;
    const amount = op.arguments_.amount;
    if (
      op.type_ !== 'stretch' ||
      typeof amount !== 'object' ||
      amount.type_ !== 'atom' ||
      !(Number(amount.source_) > 0)
    )
      return null;
  }
  return node.options_.weight > 0 ? singleSound(node.source_) : null;
}

function fixedSound(state: EditorState, tree: Tree, arg: SyntaxNode | null) {
  if (!arg) return null;
  const value = literal(state, arg);
  if (value === null) return null;
  // Single quotes carry a plain JS sound name. Mini literals can repeat the same instrument.
  const plain = state.sliceDoc(arg.from, arg.from + 1) === "'";
  if (!plain && !miniEnabled(tree, state, arg.from)) return null;
  const name = plain ? value : singleSound(mini2ast('"' + value + '"'));
  const sound = name?.toLowerCase();
  return sound && isAuditionSound(sound) ? sound : null;
}

function localVoice(state: EditorState, tree: Tree, editedNote: SyntaxNode) {
  const outer = outerCall(editedNote);
  if (!outer) return null;
  // The enclosing arrangement may contain sibling voices. Never take their controls or
  // guess the result of applying a function or an override to the whole arrangement.
  for (let parent = outer.parent; parent; parent = parent.parent) {
    if (parent.name === 'CallExpression') {
      const fn = parent.firstChild;
      if (fn?.name !== 'VariableName' || !containers.has(nodeText(state, fn))) return null;
    }
  }
  const controls: Record<string, number> = {};
  let sound: string | null = null;
  let call = unwrapped(outer);
  while (call?.name === 'CallExpression') {
    const fn = call.firstChild;
    if (!fn || fn.nextSibling?.name !== 'ArgList') return null;
    const member = fn.name === 'MemberExpression';
    const method = member ? fn.lastChild : fn;
    if (
      !method ||
      (member
        ? method.name !== 'PropertyName' || method.prevSibling?.name !== '.'
        : method.name !== 'VariableName')
    )
      return null;
    const name = nodeText(state, method);
    const arg = onlyArgument(call.lastChild);
    if (name === 'note') {
      if (call.from !== editedNote.from || call.to !== editedNote.to) return null;
    } else if (name === 's' || name === 'sound') {
      if (sound !== null) return null;
      sound = fixedSound(state, tree, arg);
      if (!sound) return null;
    } else if (!member) {
      return null;
    } else if (voiceControls.has(name)) {
      if (!arg || (arg.name !== 'Number' && arg.name !== 'UnaryExpression')) return null;
      const value = Number(nodeText(state, arg));
      if (!Number.isFinite(value)) return null;
      // Walk from the outside in, where the last setting wins.
      controls[name] ??= value;
    } else if (!ignoredCalls.has(name)) {
      return null;
    }
    if (!member) return { sound: sound ?? 'triangle', controls };
    call = unwrapped(fn.firstChild);
  }
  return null;
}

function noteAt(state: EditorState, from: number, to: number) {
  const tree = ensureSyntaxTree(state, state.doc.length, 20);
  if (!tree) return null;
  let node: SyntaxNode | null = tree.resolveInner(to, -1);
  while (node && node.name !== 'String' && node.name !== 'TemplateString') node = node.parent;
  if (!node || state.sliceDoc(node.from, node.from + 1) === "'") return null;
  const list = node.parent;
  const argument = onlyArgument(list);
  if (argument?.from !== node.from || argument.to !== node.to) return null;
  const call = list?.parent;
  const fn = call?.firstChild;
  const name = fn?.name === 'MemberExpression' ? fn.lastChild : fn;
  if (!call || call.name !== 'CallExpression' || !name || nodeText(state, name) !== 'note')
    return null;
  const value = literal(state, node, true);
  if (value === null || !miniEnabled(tree, state, node.from)) return null;
  try {
    const quoted = '"' + value + '"';
    for (const atom of pitchedAtoms(mini2ast(quoted), quoted)) {
      const start = node.from + atom.from;
      const end = node.from + atom.to;
      if (from < start || to > end || (from === to && to !== end)) continue;
      const voice = localVoice(state, tree, call);
      return voice && { from: start, to: end, pitch: atom.pitch, ...voice };
    }
  } catch {
    // Invalid or partially typed mini notation has no candidate.
  }
  return null;
}

/** Attribute a single input to a pitch, or identify the previous pitch confirmed by a delimiter. */
export function inspectNoteAudition(tr: Transaction): NoteAuditionEdit | null {
  if (
    !tr.isUserEvent('input.type') ||
    tr.startState.selection.ranges.length !== 1 ||
    tr.newSelection.ranges.length !== 1 ||
    !tr.newSelection.main.empty
  )
    return null;
  const changes: { fromA: number; toA: number; fromB: number; toB: number }[] = [];
  tr.changes.iterChangedRanges(
    (fromA, toA, fromB, toB) => changes.push({ fromA, toA, fromB, toB }),
    true,
  );
  if (changes.length !== 1) return null;
  const { fromA, toA, fromB, toB } = changes[0];
  if (toB === fromB || tr.newSelection.main.head !== toB) return null;

  const before = tr.startState.selection.main;
  const inserted = tr.newDoc.sliceString(fromB, toB);
  if (before.from !== fromA) return null;
  if (before.empty && /^[\s,\]}>"`]$/.test(inserted)) {
    // CodeMirror overtypes its own closing quote by replacing it with the identical character.
    if (fromA !== toA && tr.startState.sliceDoc(fromA, toA) !== inserted) return null;
    const previous = noteAt(tr.startState, fromA, fromA);
    return previous && { kind: 'confirm', ...previous };
  }
  if (before.to !== toA || !/^[a-gA-G#bsf\d-]+$/.test(inserted)) return null;
  const current = noteAt(tr.state, fromB, toB);
  return current && { kind: 'note', ...current };
}

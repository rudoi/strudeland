import { Fraction } from '@strudel/core';
import { parse, type AnyNode } from 'acorn';
import {
  validateEventSpec,
  type EventEditEntry,
  type EventEditIdentity,
} from '../strudel/event-edits';
import { EDIT_COMMENT, songEditComment } from '../strudel/event-document';
import type { RollEdit, RollNote, RollPatch, SourceSpan } from './types';

export type EventEditScope = { kind: 'expression'; expression: SourceSpan } | { kind: 'song' };
const options = { ecmaVersion: 2022, allowAwaitOutsideFunction: true } as const;

export function preparePhraseEvents(notes: readonly RollNote[], period?: number): RollNote[] {
  const groups = new Map<string, RollNote[]>();
  const keyOf = (n: RollNote) => JSON.stringify([n.start, n.end, n.pitch]);
  for (const note of notes) {
    const key = keyOf(note),
      group = groups.get(key);
    if (group) group.push(note);
    else groups.set(key, [note]);
  }
  return notes.map((note) => {
    const group = groups.get(keyOf(note))!;
    return {
      ...note,
      event: {
        from: [note.start, note.end],
        pitch: note.pitch,
        ...(period ? { period } : {}),
        ...(group.length > 1 ? { index: group.indexOf(note), count: group.length } : {}),
      },
    };
  });
}

function literalData(node: AnyNode): unknown {
  if (node.type === 'Literal') return node.value;
  if (
    node.type === 'UnaryExpression' &&
    node.operator === '-' &&
    node.argument.type === 'Literal' &&
    typeof node.argument.value === 'number'
  )
    return -node.argument.value;
  if (node.type === 'ArrayExpression') return node.elements.map((e) => e && literalData(e));
  if (node.type === 'ObjectExpression') {
    const fields = node.properties.map((p) => {
      if (p.type !== 'Property' || p.computed || p.kind !== 'init' || p.method || p.shorthand)
        throw new Error('expected a plain property');
      const name =
        p.key.type === 'Identifier'
          ? p.key.name
          : p.key.type === 'Literal'
            ? p.key.value
            : undefined;
      if (typeof name !== 'string' || name === '__proto__')
        throw new Error('invalid entry property');
      return [name, literalData(p.value)];
    });
    return Object.fromEntries(fields);
  }
  throw new Error('expected literal event data');
}

function entriesArray(node: AnyNode) {
  if (node.type !== 'ObjectExpression') throw new Error('expected event entries');
  const prop = node.properties[0];
  if (prop?.type !== 'Property' || prop.value.type !== 'ArrayExpression')
    throw new Error('expected an entries array');
  return prop.value;
}

export function readEventSpec(node: AnyNode) {
  const data = literalData(node);
  validateEventSpec(data);
  return data;
}

function findExpression(code: string, span: SourceSpan) {
  const stack: { node: AnyNode; property?: AnyNode }[] = [{ node: parse(code, options) }];
  while (stack.length) {
    const { node, property } = stack.pop()!;
    if (
      node.start === span.from &&
      node.end === span.to &&
      !['Program', 'ExpressionStatement', 'Property'].includes(node.type)
    )
      return { node, shorthand: property?.type === 'Property' && property.shorthand };
    for (const v of Object.values(node))
      for (const child of Array.isArray(v) ? v : [v]) {
        if (
          child &&
          typeof child === 'object' &&
          typeof child.type === 'string' &&
          child.start <= span.from &&
          child.end >= span.to
        )
          stack.push({ node: child, property: node });
      }
  }
}

function expressionEntries(node: AnyNode) {
  if (
    node.type !== 'CallExpression' ||
    node.callee.type !== 'MemberExpression' ||
    node.callee.computed ||
    node.callee.property.type !== 'Identifier' ||
    node.callee.property.name !== '_pianoEdits'
  )
    return;
  const spec = node.arguments[0];
  if (!spec || node.arguments.length !== 1) throw new Error('expected one event spec');
  const data = readEventSpec(spec);
  return { entries: data.entries, array: entriesArray(spec) };
}

function periodicOffset(entry: EventEditEntry, note: RollNote) {
  const start = Fraction(entry.to ?? entry.from[0]);
  const end = entry.until ?? start.add(Fraction(entry.from[1]).sub(entry.from[0]));
  const offset = Fraction(note.start).sub(start);
  return offset.mod(entry.period).eq(0) && Fraction(note.end).sub(offset).eq(end) ? offset : null;
}

/** The runtime re-queries a saved edit's original interval, at the observed repeat phase. */
export function eventMoveSource(source: string, scope: EventEditScope, note: RollNote) {
  const identity = note.event;
  if (!identity) return;
  try {
    let saved: EventEditEntry | undefined;
    if (identity.entry) {
      if (scope.kind === 'song') {
        saved = songEditComment(source)?.spec.entries.find((entry) => entry.id === identity.entry);
      } else {
        const found = findExpression(source, scope.expression);
        saved =
          found && expressionEntries(found.node)?.entries.find((e) => e.id === identity.entry);
      }
    }
    const origin = saved ?? identity;
    const offset =
      saved?.period && scope.kind === 'expression' ? periodicOffset(saved, note) : Fraction(0);
    if (!offset) return;
    return {
      start: Fraction(origin.from[0]).add(offset).toFraction(),
      end: Fraction(origin.from[1]).add(offset).toFraction(),
      period: origin.period,
      placement: origin.placement,
    };
  } catch {
    return;
  }
}

function jsString(value: string): string {
  return (
    "'" +
    JSON.stringify(value)
      .slice(1, -1)
      .replaceAll("'", "\\'")
      .replaceAll('\u2028', '\\u2028')
      .replaceAll('\u2029', '\\u2029') +
    "'"
  );
}
function printJs(data: unknown): string {
  if (typeof data === 'string') return jsString(data);
  if (Array.isArray(data)) return '[' + data.map(printJs).join(', ') + ']';
  if (data && typeof data === 'object')
    return (
      '{ ' +
      Object.entries(data)
        .map(
          ([key, v]) => (/^[A-Za-z_$][\w$]*$/.test(key) ? key : jsString(key)) + ': ' + printJs(v),
        )
        .join(', ') +
      ' }'
    );
  return JSON.stringify(data);
}

export function sameEventOrigin(a: EventEditIdentity, b: EventEditIdentity) {
  return (
    a.placement === b.placement &&
    a.from[0] === b.from[0] &&
    a.from[1] === b.from[1] &&
    a.pitch === b.pitch &&
    a.voice === b.voice &&
    Boolean(a.continuous) === Boolean(b.continuous) &&
    (a.index ?? 0) === (b.index ?? 0) &&
    (a.count ?? 1) === (b.count ?? 1)
  );
}

export function planEventEdit(
  source: string,
  scope: EventEditScope,
  note: RollNote,
  gesture: RollEdit,
): readonly RollPatch[] | null {
  if (gesture.kind === 'insert' || !note.event || gesture.id !== note.id) return null;
  try {
    let existing: EventEditEntry[] = [],
      array: ReturnType<typeof entriesArray> | undefined,
      shorthand = false,
      scalar = false;
    if (scope.kind === 'song') {
      const comment = songEditComment(source);
      if (comment) {
        existing = comment.spec.entries;
        array = entriesArray(comment.node);
      }
    } else {
      const found = findExpression(source, scope.expression);
      if (!found) return null;
      shorthand = !!found.shorthand;
      const n = found.node;
      scalar = n.type === 'Literal' || n.type === 'TemplateLiteral';
      if (scalar && /\bglobalThis\b/.test(source) && /\breify\b/.test(source)) return null;
      const wrapper = expressionEntries(n);
      if (wrapper) {
        existing = wrapper.entries;
        array = wrapper.array;
      }
    }
    const { entry: originalId, ...identity } = note.event;
    const index = existing.findIndex((e) =>
      originalId ? e.id === originalId : sameEventOrigin(e, identity),
    );
    let sequence = 1;
    while (source.includes('roll' + sequence)) sequence++;
    const prior = index >= 0 ? existing[index] : undefined;
    const entry: EventEditEntry = { ...(prior ?? identity), id: prior?.id ?? 'roll' + sequence };
    if (gesture.kind === 'pitch') entry.key = gesture.pitch;
    if (gesture.kind === 'move') {
      const sourceLength = Fraction(entry.from[1]).sub(entry.from[0]);
      const offset =
        scope.kind === 'expression' && prior?.period ? periodicOffset(prior, note) : Fraction(0);
      if (!offset) return null;
      const start = Fraction(gesture.start).sub(offset);
      const end = Fraction(gesture.end).sub(offset);
      entry.to = start.toFraction();
      if (entry.until !== undefined || !end.sub(start).eq(sourceLength))
        entry.until = end.toFraction();
    }
    if (gesture.kind === 'resize') entry.until = gesture.end;
    if (gesture.kind === 'delete') entry.remove = true;
    const entries = [...existing];
    if (index >= 0) entries[index] = entry;
    else entries.push(entry);
    validateEventSpec({ entries });
    const song = scope.kind === 'song';
    const printed = song ? JSON.stringify(entry).replaceAll('*/', '*\\/') : printJs(entry);
    let patches: RollPatch[];
    if (array) {
      const old = index >= 0 ? array.elements[index] : undefined;
      if (old) patches = [{ from: old.start, to: old.end, insert: printed }];
      else {
        const last = array.elements.at(-1);
        const at = last?.end ?? array.start + 1;
        patches = [{ from: at, to: at, insert: (last ? ', ' : '') + printed }];
      }
    } else if (scope.kind === 'song') {
      patches = [
        {
          from: source.length,
          to: source.length,
          insert: '\n/* ' + EDIT_COMMENT + ' {"entries":[' + printed + ']} */',
        },
      ];
    } else {
      const { from, to } = scope.expression;
      patches = [
        {
          from,
          to: from,
          insert: shorthand
            ? source.slice(from, to) + ': ('
            : scalar
              ? /\bglobalThis\b/.test(source)
                ? 'reify('
                : 'globalThis.reify('
              : '(',
        },
        { from: to, to, insert: ')._pianoEdits({ entries: [' + printed + '] })' },
      ];
    }
    const patched = () =>
      patches.reduceRight((code, p) => code.slice(0, p.from) + p.insert + code.slice(p.to), source);
    let changed = patched();
    if (scope.kind === 'expression' && !array) {
      // Opening parentheses at a statement boundary can attach to the preceding
      // call through ASI. Require the wrapper itself to remain a distinct expression.
      const prefix = patches[0],
        suffix = patches[1];
      const intact = (skip: number) => {
        const from =
          prefix.from +
          skip +
          (shorthand ? source.slice(scope.expression.from, scope.expression.to).length + 2 : 0);
        const to = suffix.from + prefix.insert.length + suffix.insert.length;
        const match = findExpression(changed, { from, to })?.node;
        return (
          match?.type === 'CallExpression' &&
          match.callee.type === 'MemberExpression' &&
          match.callee.property.type === 'Identifier' &&
          match.callee.property.name === '_pianoEdits'
        );
      };
      if (!intact(0)) {
        prefix.insert = ';' + prefix.insert;
        changed = patched();
        if (!intact(1)) return null;
      }
    } else parse(changed, options);
    return patches;
  } catch {
    return null;
  }
}

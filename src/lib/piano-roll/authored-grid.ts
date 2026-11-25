import { parse, type AnyNode, type Program } from 'acorn';
import { Fraction } from '@strudel/core';
import { scriptOptions } from '../editor/format-script';
import { decodeNoteGrid, type GridScore } from '../strudel/note-grid';
import { isMidi, namePitch } from './phrase';
import type { RollEdit, RollPatch, RollPhrase, SourceSpan } from './types';

type Lookup = (node: AnyNode) => AnyNode | undefined;
const span = (node: AnyNode) => ({ from: node.start, to: node.end });
const same = (a: SourceSpan | undefined, b: SourceSpan) => a?.from === b.from && a?.to === b.to;

export interface AuthoredGrid {
  rows: SourceSpan;
  expression: SourceSpan;
  baseExpression: SourceSpan;
  rowsName?: string;
  rowDeclaration?: SourceSpan;
  patternDeclaration?: SourceSpan;
  rowSpans: SourceSpan[];
  phrase: RollPhrase;
  hasEventEdits: boolean;
  score: GridScore;
}

function children(node: AnyNode): AnyNode[] {
  return Object.values(node).flatMap((value) =>
    (Array.isArray(value) ? value : [value]).filter(
      (child): child is AnyNode => typeof child?.type === 'string',
    ),
  );
}

function ungroup(node: AnyNode): AnyNode {
  while (node.type === 'ParenthesizedExpression') node = node.expression;
  return node;
}

function basePattern(node: AnyNode): AnyNode {
  node = ungroup(node);
  while (
    node.type === 'CallExpression' &&
    node.callee.type === 'MemberExpression' &&
    !node.callee.computed &&
    node.callee.property.type === 'Identifier' &&
    node.callee.property.name === '_pianoEdits'
  )
    node = ungroup(node.callee.object);
  return node;
}

function singleConst(node: AnyNode | undefined) {
  if (
    node?.type !== 'VariableDeclaration' ||
    node.kind !== 'const' ||
    node.declarations.length !== 1
  )
    return;
  const decl = node.declarations[0];
  if (decl.id.type === 'Identifier' && decl.init) return { id: decl.id, init: ungroup(decl.init) };
}

// Each local identifier has a distinct role. Matching references by declaration also
// detects lookalikes that use a different binding with the same visible spelling.
const legacy = parse(
  String.raw`
const steps = rows.flatMap((bar, index) => {
  const slots = bar.trim().split(/\s+/);
  if (slots.length !== 16) throw new Error('invalid row');
  return slots;
});
const events = [];
let previous;
for (let at = 0; at < steps.length;) {
  const cell = steps[at];
  let runSize = 1;
  while (steps[at + runSize] === '_') runSize++;
  if (cell === '~' || cell === '_') {
    events.push([runSize, silence]);
  } else {
    const [pitch, glide] = cell.split(':');
    const bend = glide && previous ? noteToMidi(pitch) - noteToMidi(previous) : 0.55;
    events.push([runSize, note(pitch).penv(bend).pattack(glide ? Number(glide) : 0.09)]);
    previous = pitch;
  }
  at += runSize;
}
stepcat(...events).slow(rows.length);
`,
  scriptOptions,
).body;

const roleNames = new Set(
  'rows steps bar index slots events previous at cell runSize pitch glide bend'.split(' '),
);

function matchesForm(
  form: unknown,
  value: unknown,
  roles: Map<string, AnyNode>,
  lookup: Lookup,
  memberProperty = false,
): boolean {
  if (form === null || typeof form !== 'object') return form === value;
  if (Array.isArray(form))
    return (
      Array.isArray(value) &&
      form.length === value.length &&
      form.every((child, index) => matchesForm(child, value[index], roles, lookup))
    );
  if (!value || typeof value !== 'object' || !('type' in value)) return false;
  const expected = form as AnyNode;
  const actual = ungroup(value as AnyNode);
  if (expected.type !== actual.type) return false;
  if (expected.type === 'Identifier' && actual.type === 'Identifier') {
    if (memberProperty) return expected.name === actual.name;
    if (!roleNames.has(expected.name)) return expected.name === actual.name && !lookup(actual);
    const binding = lookup(actual) ?? actual;
    if (!roles.has(expected.name)) {
      if ([...roles.values()].includes(binding)) return false;
      roles.set(expected.name, binding);
    }
    return roles.get(expected.name) === binding;
  }
  if (expected.type === 'Literal' && actual.type === 'Literal') {
    if (expected.regex)
      return (
        actual.regex?.pattern === expected.regex.pattern &&
        actual.regex.flags === expected.regex.flags
      );
    return (
      expected.value === actual.value &&
      (typeof expected.value !== 'string' || actual.raw?.startsWith("'") === true)
    );
  }
  // All recognized rows have passed the width check, so this diagnostic cannot execute.
  if (expected.type === 'NewExpression' && actual.type === 'NewExpression') {
    return matchesForm(expected.callee, actual.callee, roles, lookup);
  }
  const record = actual as unknown as Record<string, unknown>;
  return Object.entries(expected).every(
    ([key, child]) =>
      ['start', 'end', 'type'].includes(key) ||
      matchesForm(
        child,
        record[key],
        roles,
        lookup,
        key === 'property' && expected.type === 'MemberExpression' && !expected.computed,
      ),
  );
}

function createGrid(
  source: string,
  rows: AnyNode,
  expression: AnyNode,
  patternDeclaration?: AnyNode,
  rowDeclaration?: AnyNode,
  requiredWidth?: number,
): AuthoredGrid | undefined {
  if (rows.type !== 'ArrayExpression' || !rows.elements.length) return;
  const rowSpans: SourceSpan[] = [];
  const values: string[] = [];
  for (const literal of rows.elements) {
    if (literal?.type !== 'Literal' || typeof literal.value !== 'string') return;
    const raw = source.slice(literal.start, literal.end);
    if (raw[0] !== "'" || raw.slice(1, -1) !== literal.value) return;
    rowSpans.push(span(literal));
    values.push(literal.value);
  }
  let score: GridScore;
  try {
    score = decodeNoteGrid(values);
  } catch {
    return;
  }
  if (requiredWidth && score.width !== requiredWidth) return;
  const notes = score.runs.flatMap(({ start, end, tone }) => {
    if (!tone) return [];
    const from = rowSpans[Math.floor(start / score.width)].from + 1 + score.cells[start].offset;
    return [
      {
        id: `grid-${start}`,
        pitch: tone.midi,
        glideBend: tone.bend,
        start: Fraction(start).div(score.width).toFraction(),
        end: Fraction(end).div(score.width).toFraction(),
        origin: { from, to: from + tone.spelling.length },
        pitchEditable: true,
        timeEditable: true,
      },
    ];
  });
  const base = basePattern(expression);
  return {
    rows: span(rows),
    rowSpans,
    expression: span(expression),
    baseExpression: span(base),
    rowsName: rowDeclaration?.type === 'Identifier' ? rowDeclaration.name : undefined,
    patternDeclaration: patternDeclaration && span(patternDeclaration),
    rowDeclaration: rowDeclaration && span(rowDeclaration),
    score,
    phrase: { bars: score.bars, notes, insertable: true },
    hasEventEdits: base !== ungroup(expression),
  };
}

export function findAuthoredGrids(
  source: string,
  program: Program,
  lookup: Lookup,
  staticSafe: boolean,
): AuthoredGrid[] {
  if (!staticSafe) return [];
  const found: AuthoredGrid[] = [];
  const nodes: AnyNode[] = [];
  const parents = new Map<AnyNode, AnyNode>();
  const refs = new Map<AnyNode, AnyNode[]>();
  const consts = new Map<AnyNode, AnyNode>();
  function visit(node: AnyNode) {
    nodes.push(node);
    const decl = singleConst(node);
    if (decl) consts.set(decl.id, decl.init);
    const parent = parents.get(node);
    if (
      node.type === 'Identifier' &&
      !(parent?.type === 'MemberExpression' && !parent.computed && parent.property === node) &&
      !(
        parent?.type === 'Property' &&
        !parent.computed &&
        parent.key === node &&
        parent.value !== node
      )
    ) {
      const target = lookup(node);
      if (target && target !== node) {
        const uses = refs.get(target) ?? [];
        uses.push(node);
        refs.set(target, uses);
      }
    }
    for (const child of children(node)) {
      parents.set(child, node);
      visit(child);
    }
  }
  visit(program);
  const protectedNames = new Set([
    'noteGrid',
    'flatMap',
    'trim',
    'split',
    'push',
    'penv',
    'pattack',
    'slow',
    'stepcat',
    'noteToMidi',
    'note',
    'silence',
    'Number',
  ]);
  for (const node of nodes) {
    const target =
      node.type === 'UpdateExpression'
        ? node.argument
        : node.type === 'AssignmentExpression'
          ? node.left
          : undefined;
    const key = target?.type === 'MemberExpression' ? target.property : target;
    if (
      key?.type === 'Identifier' &&
      protectedNames.has(key.name) &&
      (target !== key || !lookup(key))
    )
      return [];
    if (key?.type === 'Literal' && protectedNames.has(String(key.value))) return [];
    if (
      node.type === 'CallExpression' &&
      node.callee.type === 'Identifier' &&
      ['register', 'registerControl'].includes(node.callee.name)
    ) {
      const name = node.arguments[0];
      if (name?.type !== 'Literal' || protectedNames.has(String(name.value))) return [];
    }
  }
  const contains = (owner: AnyNode, node: AnyNode) =>
    node.start >= owner.start && node.end <= owner.end;
  for (const node of nodes) {
    if (
      node.type === 'CallExpression' &&
      node.callee.type === 'Identifier' &&
      node.callee.name === 'noteGrid' &&
      !lookup(node.callee) &&
      !node.optional &&
      node.arguments.length === 1
    ) {
      const operand = ungroup(node.arguments[0]);
      const definition = operand.type === 'Identifier' ? lookup(operand) : undefined;
      const rows = definition ? consts.get(definition) : operand;
      if (
        !rows ||
        (definition &&
          (rows.end > node.start || refs.get(definition)?.some((ref) => ref !== operand)))
      )
        continue;
      let expression: AnyNode = node;
      for (let parent = parents.get(expression); parent; parent = parents.get(expression)) {
        if (
          basePattern(parent) === node ||
          (parent.type === 'MemberExpression' &&
            parent.object === expression &&
            !parent.computed &&
            parent.property.type === 'Identifier' &&
            parent.property.name === '_pianoEdits')
        )
          expression = parent;
        else break;
      }
      const parent = parents.get(expression);
      const grid = createGrid(
        source,
        rows,
        expression,
        parent?.type === 'VariableDeclarator' ? parent.id : undefined,
        definition,
      );
      if (grid) found.push(grid);
    }
    if (node.type !== 'Program' && node.type !== 'BlockStatement') continue;
    const body = node.body;
    for (let i = 0; i + legacy.length < body.length; i++) {
      const rows = singleConst(body[i]);
      const pattern = singleConst(body[i + legacy.length]);
      if (rows?.init.type !== 'ArrayExpression' || !pattern) continue;
      const roles = new Map<string, AnyNode>([['rows', rows.id]]);
      const output = legacy.at(-1)!;
      if (
        output.type !== 'ExpressionStatement' ||
        !legacy
          .slice(0, -1)
          .every((form, j) => matchesForm(form, body[i + j + 1], roles, lookup)) ||
        !matchesForm(output.expression, basePattern(pattern.init), roles, lookup)
      )
        continue;
      const privateBindings = ['rows', 'steps', 'events', 'previous'];
      if (
        privateBindings.some((role) =>
          refs
            .get(roles.get(role)!)
            ?.some(
              (ref) => ref.start < body[i + 1].start || ref.end > basePattern(pattern.init).end,
            ),
        )
      )
        continue;
      const grid = createGrid(source, rows.init, pattern.init, pattern.id, rows.id, 16);
      if (grid) found.push(grid);
    }
  }
  return found;
}

function cellSpan(grid: AuthoredGrid, slot: number): SourceSpan {
  const cell = grid.score.cells[slot];
  const from = grid.rowSpans[Math.floor(slot / grid.score.width)].from + 1 + cell.offset;
  return { from, to: from + cell.text.length };
}

export function matchGridSelection(
  grid: AuthoredGrid,
  selection: SourceSpan,
): { row: number } | undefined {
  const caret = selection.from === selection.to;
  const row = grid.rowSpans.findIndex((span) =>
    caret ? span.from < selection.from && selection.from < span.to : same(span, selection),
  );
  if (row >= 0) return { row };
  if (!caret) {
    const slot = grid.score.cells.findIndex((_, i) => same(cellSpan(grid, i), selection));
    if (slot >= 0) return { row: Math.floor(slot / grid.score.width) };
  }
  const names = [grid.rowDeclaration, grid.patternDeclaration];
  if (
    names.some(
      (name) =>
        name &&
        (same(name, selection) ||
          (caret && name.from <= selection.from && selection.from < name.to)),
    ) ||
    same(grid.rows, selection) ||
    same(grid.expression, selection)
  )
    return { row: 0 };
}

/** Accept only exact grid edges. Fractional off-grid destinations go to the event edit. */
function slotAt(edge: string, width: number): number {
  if (!/^\d{1,12}(?:\/[1-9]\d{0,11})?$/.test(edge)) return NaN;
  const slot = Fraction(edge).mul(width).toFraction();
  return /^\d+$/.test(slot) && Number.isSafeInteger(Number(slot)) ? Number(slot) : NaN;
}

export function planGridEdit(
  source: string,
  grid: AuthoredGrid,
  edit: RollEdit,
): RollPatch[] | null {
  if (grid.hasEventEdits) return null;
  const { score } = grid;
  const events = score.runs.flatMap(({ start, end, tone }) =>
    tone
      ? [
          {
            start,
            end,
            pitch: tone.midi,
            spelling: tone.spelling,
            text: score.cells[start].text,
          },
        ]
      : [],
  );
  const original =
    edit.kind === 'insert' ? undefined : events.find((event) => `grid-${event.start}` === edit.id);
  if (edit.kind !== 'insert' && !original) return null;
  const pitch = edit.kind === 'pitch' || edit.kind === 'insert' ? edit.pitch : original!.pitch;
  if (!isMidi(pitch)) return null;
  const start =
    edit.kind === 'move' || edit.kind === 'insert'
      ? slotAt(edit.start, score.width)
      : original!.start;
  const end =
    edit.kind === 'resize' || edit.kind === 'insert'
      ? slotAt(edit.end, score.width)
      : start + original!.end - original!.start;
  if (
    !Number.isSafeInteger(start) ||
    !Number.isSafeInteger(end) ||
    start < 0 ||
    end <= start ||
    end > score.cells.length
  )
    return null;
  const expected = events.filter((event) => event !== original);
  if (edit.kind !== 'delete') {
    if (expected.some((event) => event.start < end && event.end > start)) return null;
    const spelling =
      original?.pitch === pitch ? original.spelling : namePitch(pitch, original?.spelling);
    const text = spelling + (original?.text.slice(original.spelling.length) ?? '');
    expected.push({ start, end, pitch, spelling, text });
  }
  expected.sort((a, b) => a.start - b.start);
  const tokens: string[] = [];
  let eventIndex = 0;
  for (const [slot, cell] of score.cells.entries()) {
    const event = expected[eventIndex];
    if (event && slot >= event.start) {
      tokens.push(slot === event.start ? event.text : '_');
      if (slot === event.end - 1) eventIndex++;
    } else {
      const beginsSilence = slot > 0 && expected[eventIndex - 1]?.end === slot;
      tokens.push(cell.text === '_' && !beginsSilence ? '_' : '~');
    }
  }
  const patches: RollPatch[] = [];
  for (const [slot, text] of tokens.entries()) {
    const before = score.cells[slot].text;
    if (before === text) continue;
    const range = cellSpan(grid, slot);
    if (source.slice(range.from, range.to) !== before) return null;
    if (edit.kind === 'pitch' && original && original.start === slot) {
      const suffixLength = original.text.length - original.spelling.length;
      patches.push({
        from: range.from,
        to: range.to - suffixLength,
        insert: text.slice(0, text.length - suffixLength),
      });
    } else {
      patches.push({
        ...range,
        insert: text,
        ...((edit.kind === 'move' || edit.kind === 'insert' || edit.kind === 'delete') &&
        slot === start
          ? { reveal: true }
          : {}),
      });
    }
  }
  if (!patches.length) return null;

  // Reparse the exact changed array. The lexical proof is outside these single-quoted
  // values. Cell-only patches with unchanged array shape preserve that proof.
  let raw = source.slice(grid.rows.from, grid.rows.to);
  for (const patch of [...patches].reverse()) {
    const from = patch.from - grid.rows.from;
    const to = patch.to - grid.rows.from;
    raw = raw.slice(0, from) + patch.insert + raw.slice(to);
  }
  try {
    const statement = parse(raw, scriptOptions).body[0];
    if (statement.type !== 'ExpressionStatement' || statement.expression.type !== 'ArrayExpression')
      return null;
    const rows = statement.expression.elements.map((row) =>
      row?.type === 'Literal' && typeof row.value === 'string' ? row.value : '',
    );
    const verified = decodeNoteGrid(rows);
    if (verified.width !== score.width || verified.bars !== score.bars) return null;
    const notes = verified.runs.filter((run) => run.tone);
    if (
      notes.length !== expected.length ||
      !expected.every((e, index) => {
        const n = notes[index];
        const suffix = e.text.slice(e.spelling.length + 1);
        const attack = suffix ? Number(suffix) : 0.09;
        const bend = suffix && index > 0 ? e.pitch - expected[index - 1].pitch : 0.55;
        return (
          n.start === e.start &&
          n.end === e.end &&
          n.tone?.midi === e.pitch &&
          n.tone.spelling === e.spelling &&
          n.tone.attack === attack &&
          n.tone.bend === bend
        );
      })
    )
      return null;
  } catch {
    return null;
  }
  return patches;
}

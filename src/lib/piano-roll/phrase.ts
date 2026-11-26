import { spellNote } from '../strudel/note-name';
import { Fraction, State, TimeSpan, noteToMidi } from '@strudel/core';
import { getLeafLocation, mini2ast, patternifyAST } from '@strudel/mini';
import type { RollEdit, RollNote, RollPatch, RollPhrase, SourceSpan } from './types';

type Rational = {
  add(value: Rational | number): Rational;
  sub(value: Rational | number): Rational;
  mul(value: Rational | number): Rational;
  div(value: Rational | number): Rational;
  compare(value: Rational | number): number;
  floor(): Rational;
  lcm(value: Rational): Rational;
  toFraction(): string;
  valueOf(): number;
};
const fraction: (value: string | number) => Rational = Fraction;

type MiniLocation = { start: { offset: number }; end: { offset: number } };
type MiniAtom = { type_: 'atom'; source_: string; location_: MiniLocation };
type MiniElement = {
  type_: 'element';
  source_: MiniNode;
  location_: MiniLocation;
  options_: { weight: number; reps: number; ops: MiniOp[] };
};
type MiniPattern = {
  type_: 'pattern';
  source_: MiniNode[];
  arguments_: { alignment: string; _steps?: boolean };
};
type MiniNode = MiniAtom | MiniElement | MiniPattern;
type MiniOp = {
  type_: string;
  arguments_: Record<string, MiniNode | number | string | null>;
};

interface MiniBar {
  span: SourceSpan;
  /** A bare slow-sequence step needs brackets if replaced by several cells. */
  wrap: boolean;
}

export interface PhraseInput {
  span: SourceSpan;
  body: string;
  kind: 'numeric' | 'plain' | 'mini';
  sound?: string;
}

export interface AuthoredPhrase extends RollPhrase {
  input: PhraseInput;
  editableBars: readonly MiniBar[];
}

const MAX_CELLS = 512;
const MAX_WEIGHT = 256;
const MAX_BARS = 64;
const MAX_DEPTH = 24;
const MAX_ENCODED_STEPS = 256;
const PITCH = /^[a-gA-G][#bsf]*(?:-?\d+)?$/;
const MIDI = /^(?:0|[1-9]\d{0,2})$/;
const REST = /^[-~]$/;

export function isMidi(pitch: number): boolean {
  return Number.isInteger(pitch) && pitch >= 0 && pitch <= 127;
}

function atomPitch(spelling: string): number | undefined {
  const pitch = MIDI.test(spelling)
    ? Number(spelling)
    : PITCH.test(spelling)
      ? noteToMidi(spelling)
      : -1;
  return isMidi(pitch) ? pitch : undefined;
}

function verifiedSlice(body: string, location: MiniLocation, base: number): SourceSpan {
  const text = body.slice(location.start.offset, location.end.offset);
  const start = location.start.offset + text.length - text.trimStart().length;
  return { from: base + start, to: base + start + text.trim().length };
}

function barBody(span: SourceSpan, quoted: string, offset: number): SourceSpan {
  const interior = quoted.slice(span.from - offset, span.to - offset);
  const from = span.from + interior.length - interior.trimStart().length;
  return { from, to: from + interior.trim().length };
}

function smallPositive(value: unknown, limit: number): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0 && value <= limit;
}

function opNumber(value: MiniNode | number | string | null | undefined): number {
  if (typeof value === 'number') return value;
  if (typeof value !== 'object' || value === null) return NaN;
  if (value.type_ === 'atom') return MIDI.test(value.source_) ? Number(value.source_) : NaN;
  if (value.type_ !== 'element' || value.options_.weight !== 1 || value.options_.ops.length)
    return NaN;
  return value.source_.type_ === 'atom' ? opNumber(value.source_) : NaN;
}

/** Restrict the local mini query to a finite grammar and a small upper bound on produced cells. */
function queryCost(node: MiniNode, depth: number): number {
  if (depth > MAX_DEPTH) return Infinity;
  switch (node.type_) {
    case 'atom':
      return atomPitch(node.source_) !== undefined || REST.test(node.source_) ? 1 : Infinity;
    case 'pattern':
      if (
        !['fastcat', 'stack', 'polymeter_slowcat'].includes(node.arguments_.alignment) ||
        node.arguments_._steps ||
        !node.source_.length ||
        node.source_.length > MAX_CELLS
      ) {
        return Infinity;
      }
      return node.source_.reduce((count, child) => count + queryCost(child, depth + 1), 0);
    case 'element': {
      const { weight, reps, ops } = node.options_;
      if (!smallPositive(weight, MAX_WEIGHT) || !smallPositive(reps, 8) || ops.length > 4) {
        return Infinity;
      }
      let cost = queryCost(node.source_, depth + 1);
      for (const op of ops) {
        const { amount, pulse, step, rotation } = op.arguments_;
        if (op.type_ === 'replicate' || op.type_ === 'stretch') {
          const n = opNumber(amount);
          if (!smallPositive(n, 8)) return Infinity;
          cost *= n;
        } else if (op.type_ === 'bjorklund') {
          const steps = opNumber(step);
          const pulses = opNumber(pulse);
          const offset = rotation == null ? 0 : opNumber(rotation);
          if (
            !smallPositive(steps, 16) ||
            !smallPositive(pulses, steps) ||
            !Number.isInteger(offset)
          ) {
            return Infinity;
          }
          cost *= steps;
        } else {
          return Infinity;
        }
      }
      return cost;
    }
    default:
      return Infinity;
  }
}

function collectAtoms(
  node: MiniNode,
  quoted: string,
  input: PhraseInput,
  locations: Map<string, SourceSpan>,
): void {
  // Only value nodes own pitches. Euclidean and stretch operands can be MIDI-looking numbers.
  if (node.type_ === 'element') {
    collectAtoms(node.source_, quoted, input, locations);
  } else if (node.type_ === 'pattern') {
    for (const child of node.source_) collectAtoms(child, quoted, input, locations);
  } else if (!REST.test(node.source_)) {
    const span = verifiedSlice(quoted, node.location_, input.span.from);
    if (quoted.slice(span.from - input.span.from, span.to - input.span.from) === node.source_) {
      const [from, to] = getLeafLocation(quoted, node, input.span.from);
      locations.set(`${from}:${to}`, span);
    }
  }
}

function unchangedElement(node: MiniNode): node is MiniElement {
  return node.type_ === 'element' && node.options_.ops.length === 0 && node.options_.reps === 1;
}

function plainSteps(node: MiniNode): node is MiniPattern {
  return (
    node.type_ === 'pattern' &&
    node.arguments_.alignment === 'fastcat' &&
    !node.arguments_._steps &&
    node.source_.every((child) => unchangedElement(child) && child.source_.type_ === 'atom')
  );
}

function measureBars(root: MiniNode): readonly MiniNode[] | null {
  if (root.type_ !== 'pattern' || root.source_.length !== 1) return null;
  const [node] = root.source_;
  if (
    !unchangedElement(node) ||
    node.options_.weight !== 1 ||
    node.source_.type_ !== 'pattern' ||
    node.source_.arguments_.alignment !== 'polymeter_slowcat' ||
    node.source_.source_.length !== 1
  ) {
    return null;
  }
  const [sequence] = node.source_.source_;
  return sequence.type_ === 'pattern' && sequence.arguments_.alignment === 'fastcat'
    ? sequence.source_
    : null;
}

/** The local projection can only cover periods proved by the syntax. */
function singleCycle(node: MiniNode): boolean {
  switch (node.type_) {
    case 'atom':
      return true;
    case 'element':
      return (
        !node.options_.ops.some((op) => op.type_ === 'stretch' && op.arguments_.type === 'slow') &&
        singleCycle(node.source_)
      );
    case 'pattern':
      return node.arguments_.alignment !== 'polymeter_slowcat' && node.source_.every(singleCycle);
  }
}

function editableBars(
  root: MiniNode,
  explicitBars: readonly MiniNode[] | null,
  quoted: string,
  offset: number,
): MiniBar[] {
  if (plainSteps(root)) {
    const span = barBody({ from: offset + 1, to: offset + quoted.length - 1 }, quoted, offset);
    return [{ span, wrap: false }];
  }
  if (
    !explicitBars?.length ||
    explicitBars.some((bar) => !unchangedElement(bar) || bar.options_.weight !== 1)
  ) {
    return [];
  }
  const bars: MiniBar[] = [];
  for (const bar of explicitBars as MiniElement[]) {
    const src = bar.source_;
    if (src.type_ === 'atom') {
      bars.push({ span: verifiedSlice(quoted, src.location_, offset), wrap: true });
      continue;
    }
    if (!plainSteps(src)) return [];
    const { from, to } = verifiedSlice(quoted, bar.location_, offset);
    if (quoted[from - offset] !== '[' || quoted[to - offset - 1] !== ']') return [];
    bars.push({ span: barBody({ from: from + 1, to: to - 1 }, quoted, offset), wrap: false });
  }
  return bars;
}

interface MiniHap {
  value: string | number;
  whole?: { begin: Rational; end: Rational };
  context?: { locations?: { start: number; end: number }[] };
}

export function readPhrase(input: PhraseInput): AuthoredPhrase | undefined {
  if (input.kind !== 'mini') {
    if (input.kind === 'plain' && !PITCH.test(input.body)) return;
    const pitch = atomPitch(input.body);
    if (pitch === undefined) return;
    const inset = input.kind === 'plain' ? 1 : 0;
    const origin = { from: input.span.from + inset, to: input.span.to - inset };
    return {
      input,
      notes: [
        {
          id: String(origin.from),
          start: '0',
          end: '1',
          pitch,
          origin,
          sound: input.sound,
          pitchEditable: true,
        },
      ],
      bars: 1,
      sound: input.sound,
      insertable: false,
      editableBars: [],
    };
  }
  if (input.body.length > 4096) return;

  try {
    const quoted = `"${input.body}"`;
    const root: MiniNode = mini2ast(quoted);
    if (queryCost(root, 0) > MAX_CELLS) return;
    const explicitBars = measureBars(root);
    if (explicitBars) {
      if (
        explicitBars.some(
          (bar) => !unchangedElement(bar) || bar.options_.weight !== 1 || !singleCycle(bar),
        )
      )
        return;
    } else if (!singleCycle(root)) {
      return;
    }
    const bars = explicitBars?.length ?? 1;
    if (bars > MAX_BARS) return;
    const editBars = editableBars(root, explicitBars, quoted, input.span.from);
    const insertable = editBars.length === bars;
    const origins = new Map<string, SourceSpan>();
    collectAtoms(root, quoted, input, origins);
    const pattern = patternifyAST(root, quoted, undefined, input.span.from);
    const haps: MiniHap[] = pattern.query(new State(new TimeSpan(0, bars)));
    if (haps.length > MAX_CELLS) return;
    const notes: RollNote[] = [];
    for (const hap of haps) {
      const pitch = atomPitch(String(hap.value));
      if (!hap.whole || pitch === undefined) return;
      const matches = new Set(
        (hap.context?.locations ?? [])
          .map((loc) => origins.get(`${loc.start}:${loc.end}`))
          .filter((location): location is SourceSpan => location !== undefined),
      );
      const origin = matches.size === 1 ? matches.values().next().value : undefined;
      const start = hap.whole.begin.toFraction();
      const end = hap.whole.end.toFraction();
      notes.push({
        id: `${origin?.from ?? 'generated'}:${start}:${notes.length}`,
        start,
        end,
        pitch,
        origin,
        sound: input.sound,
        pitchEditable: origin !== undefined,
        timeEditable: origin !== undefined && insertable,
      });
    }
    notes.sort((a, b) => fraction(a.start).compare(fraction(b.start)) || a.pitch - b.pitch);
    return { input, notes, bars, sound: input.sound, editableBars: editBars, insertable };
  } catch {
    return;
  }
}

/** Locate explicitly written pitch leaves without running a variable or generated pattern. */
export function matchesPitchSelection(input: PhraseInput, selection: SourceSpan): boolean {
  if (input.kind !== 'mini' || input.body.length > 4096) return false;
  try {
    const quoted = `"${input.body}"`;
    const origins = new Map<string, SourceSpan>();
    collectAtoms(mini2ast(quoted), quoted, input, origins);
    const spans = [...origins.values()];
    return (
      spans.some(({ from }) => from === selection.from) &&
      spans.some(({ to }) => to === selection.to)
    );
  } catch {
    return false;
  }
}

export function namePitch(pitch: number, old = ''): string {
  return MIDI.test(old) ? String(pitch) : spellNote(pitch, old);
}

type Cell = { start: Rational; end: Rational; spelling: string };
type ExpectedNote = Pick<RollNote, 'start' | 'end' | 'pitch'>;

function stepCount(length: Rational, factor: number): number {
  const result = length.mul(factor).toFraction();
  return /^\d+$/.test(result) ? Number(result) : NaN;
}

function serialize(cells: readonly Cell[], bar: number): string | null {
  if (cells.length > MAX_ENCODED_STEPS) return null;
  const withRests: Cell[] = [];
  let cursor = fraction(bar);
  for (const cell of cells) {
    if (
      cell.start.compare(cursor) < 0 ||
      cell.end.compare(cell.start) <= 0 ||
      cell.end.compare(bar + 1) > 0
    ) {
      return null;
    }
    if (cell.start.compare(cursor) > 0)
      withRests.push({ start: cursor, end: cell.start, spelling: '~' });
    withRests.push(cell);
    cursor = cell.end;
  }
  if (cursor.compare(bar + 1) < 0)
    withRests.push({ start: cursor, end: fraction(bar + 1), spelling: '~' });
  const durations = withRests.map((cell) => cell.end.sub(cell.start));
  let factor = fraction(1);
  for (const duration of durations) {
    const denominator = duration.toFraction().split('/')[1] ?? '1';
    factor = factor.lcm(fraction(denominator));
    if (factor.compare(MAX_ENCODED_STEPS) > 0) return null;
  }
  const counts = durations.map((duration) => stepCount(duration, factor.valueOf()));
  if (!counts.every((count) => smallPositive(count, MAX_WEIGHT))) return null;
  return withRests
    .map((cell, i) => (counts[i] === 1 ? cell.spelling : `${cell.spelling}@${counts[i]}`))
    .join(' ');
}

function editTime(
  source: string,
  phrase: AuthoredPhrase,
  edit: Exclude<RollEdit, { kind: 'pitch' }>,
  note?: RollNote,
): { patches: RollPatch[]; expected: ExpectedNote[] } | undefined {
  if (edit.kind !== 'insert' && (!note?.origin || !note.timeEditable)) return;
  const startEdge = edit.kind === 'move' || edit.kind === 'insert' ? edit.start : note!.start;
  const endEdge = edit.kind === 'delete' ? note!.end : edit.end;
  const boundedPosition = (value: string) => /^\d{1,8}(?:\/[1-9]\d{0,7})?$/.test(value);
  if (!boundedPosition(startEdge) || !boundedPosition(endEdge)) return;
  const start = fraction(startEdge);
  const end = fraction(endEdge);
  const pitch = edit.kind === 'insert' ? edit.pitch : note!.pitch;
  if (!isMidi(pitch)) return;
  const bar = start.floor().valueOf();
  if (!phrase.editableBars[bar] || end.compare(start) <= 0 || end.compare(bar + 1) > 0) return;
  const sourceBar = note ? fraction(note.start).floor().valueOf() : bar;
  if (note && (!phrase.editableBars[sourceBar] || fraction(note.end).compare(sourceBar + 1) > 0))
    return;
  if (
    edit.kind === 'move' &&
    start.compare(fraction(note!.start)) === 0 &&
    end.compare(fraction(note!.end)) === 0
  )
    return;
  const untouched = phrase.notes.filter((event) => event !== note);
  const expected: ExpectedNote[] = [...untouched];
  const spelling = note?.origin ? source.slice(note.origin.from, note.origin.to) : namePitch(pitch);
  if (edit.kind !== 'delete') {
    expected.push({ start: start.toFraction(), end: end.toFraction(), pitch });
  }
  const affected = sourceBar === bar ? [bar] : [Math.min(bar, sourceBar), Math.max(bar, sourceBar)];
  const patches: RollPatch[] = [];
  for (const index of affected) {
    const target = phrase.editableBars[index];
    const cells = untouched
      .filter((event) => fraction(event.start).floor().compare(index) === 0)
      .map((event) => ({
        start: fraction(event.start),
        end: fraction(event.end),
        spelling: event.origin ? source.slice(event.origin.from, event.origin.to) : '',
      }));
    if (index === bar && edit.kind !== 'delete') cells.push({ start, end, spelling });
    if (cells.some((cell) => atomPitch(cell.spelling) === undefined)) return;
    cells.sort((a, b) => a.start.compare(b.start));
    const replacement = serialize(cells, index);
    if (replacement === null) return;
    patches.push({
      ...target.span,
      insert: target.wrap && replacement.includes(' ') ? `[${replacement}]` : replacement,
      ...(edit.kind === 'move' && index === bar ? { reveal: true } : {}),
    });
  }
  return { patches, expected };
}

function noteKeys(notes: readonly ExpectedNote[]): string[] {
  return notes.map(({ start, end, pitch }) => `${start}:${end}:${pitch}`).sort();
}

/** Reparse the candidate with the same pure mini query and verify notes as an exact multiset. */
export function planPhraseEdit(
  source: string,
  phrase: AuthoredPhrase,
  edit: RollEdit,
): RollPatch[] | null {
  try {
    const note = edit.kind === 'insert' ? undefined : phrase.notes.find(({ id }) => edit.id === id);
    let planned: { patches: RollPatch[]; expected: ExpectedNote[] } | undefined;
    if (edit.kind === 'pitch') {
      if (!isMidi(edit.pitch) || !note?.origin || !note.pitchEditable) return null;
      const origin = note.origin;
      planned = {
        patches: [
          { ...origin, insert: namePitch(edit.pitch, source.slice(origin.from, origin.to)) },
        ],
        expected: phrase.notes.map((event) =>
          event.origin?.from === origin.from && event.origin.to === origin.to
            ? { ...event, pitch: edit.pitch }
            : event,
        ),
      };
    } else {
      planned = editTime(source, phrase, edit, note);
    }
    if (!planned) return null;
    const { patches, expected } = planned;
    if (patches.every((patch) => source.slice(patch.from, patch.to) === patch.insert)) return null;
    const { input } = phrase;
    const inset = input.kind === 'numeric' ? 0 : 1;
    const bodyStart = input.span.from + inset;
    let body = input.body;
    for (const patch of [...patches].reverse()) {
      body =
        body.slice(0, patch.from - bodyStart) + patch.insert + body.slice(patch.to - bodyStart);
    }
    const projected = readPhrase({ ...input, body });
    if (projected?.bars !== phrase.bars) return null;
    const actual = noteKeys(projected.notes);
    const wanted = noteKeys(expected);
    return actual.length === wanted.length && actual.every((note, i) => note === wanted[i])
      ? patches
      : null;
  } catch {
    return null;
  }
}

import { expect, test } from 'bun:test';
import { EditorState } from '@codemirror/state';
import type { Extension } from '@codemirror/state';
import { history, isolateHistory, undo } from '@codemirror/commands';
import {
  claudeEdit,
  claudeMark,
  claudeMarkField,
  hasClaudeMark,
} from '../src/lib/editor/claude-mark';

function make(doc: string, extra: Extension[] = []) {
  return EditorState.create({ doc, extensions: [claudeMark, ...extra] });
}

function marked(state: EditorState): { pos: number; cls: string }[] {
  const out: { pos: number; cls: string }[] = [];
  const set = state.field(claudeMarkField);
  set.between(0, state.doc.length, (from, _to, value) => {
    out.push({ pos: from, cls: (value.spec as { class: string }).class });
  });
  return out;
}

test('an annotated multi-line change marks exactly the changed lines', () => {
  const s = make('aaa\nbbb\nccc\nddd');
  const next = s.update({
    changes: { from: 4, to: 11, insert: 'XX\nYY\nZZ' },
    annotations: claudeEdit.of(true),
  }).state;
  // doc: aaa / XX / YY / ZZ / ddd -> lines 2-4 start at 4, 7, 10
  expect(marked(next)).toEqual([
    { pos: 4, cls: 'cm-claude-edit' },
    { pos: 7, cls: 'cm-claude-edit' },
    { pos: 10, cls: 'cm-claude-edit' },
  ]);
});

test('separate changes on one line mark it once', () => {
  const s = make('abcdef\nxyz');
  const next = s.update({
    changes: [
      { from: 0, to: 1, insert: 'A' },
      { from: 4, to: 5, insert: 'E' },
    ],
    annotations: claudeEdit.of(true),
  }).state;
  expect(marked(next).map((m) => m.pos)).toEqual([0]);
});

test('an inserted line ending in a line break does not mark the untouched next line', () => {
  const s = make('aaa\nbbb\n');
  // Append "ccc\n" after the final newline: only the new line is marked, not the empty last one.
  const next = s.update({
    changes: { from: 8, insert: 'ccc\n' },
    annotations: claudeEdit.of(true),
  }).state;
  expect(next.doc.toString()).toBe('aaa\nbbb\nccc\n');
  expect(marked(next).map((m) => m.pos)).toEqual([8]);
});

test('an insertion starting with a line break at a line end does not mark that line', () => {
  // 'a' -> 'a\nb': appending to a file without a trailing newline marks only the new line.
  const s = make('a');
  const next = s.update({
    changes: { from: 1, insert: '\nb' },
    annotations: claudeEdit.of(true),
  }).state;
  expect(next.doc.toString()).toBe('a\nb');
  expect(marked(next).map((m) => m.pos)).toEqual([2]);
  // A replacement that also removes the line's tail still marks it.
  const r = make('ab').update({
    changes: { from: 1, to: 2, insert: '\nc' },
    annotations: claudeEdit.of(true),
  }).state;
  expect(marked(r).map((m) => m.pos)).toEqual([0, 2]);
});

test('inserting a bare line break at a line start marks the new empty line', () => {
  const s = make('aaa\nbbb');
  const next = s.update({
    changes: { from: 4, insert: '\n' },
    annotations: claudeEdit.of(true),
  }).state;
  expect(marked(next).map((m) => m.pos)).toEqual([4]);
});

test('a pure deletion marks the joined line', () => {
  const s = make('aaa\nbbb\nccc');
  const next = s.update({
    changes: { from: 3, to: 8 },
    annotations: claudeEdit.of(true),
  }).state;
  expect(next.doc.toString()).toBe('aaaccc');
  expect(marked(next).map((m) => m.pos)).toEqual([0]);
});

test('clears on typing, undo, reload and input.meta', () => {
  for (const userEvent of ['input.type', 'undo', 'reload', 'input.meta']) {
    const s = make('aaa\nbbb');
    const m = s.update({
      changes: { from: 0, insert: 'x' },
      annotations: claudeEdit.of(true),
    }).state;
    expect(hasClaudeMark(m)).toBe(true);
    const cleared = m.update({ changes: { from: 0, insert: 'y' }, userEvent }).state;
    expect(hasClaudeMark(cleared)).toBe(false);
  }
});

test('non-doc transactions keep and map the marks', () => {
  const s = make('aaa\nbbb');
  const m = s.update({ changes: { from: 4, insert: 'x' }, annotations: claudeEdit.of(true) }).state;
  const sel = m.update({ selection: { anchor: 1 } }).state;
  expect(marked(sel)).toEqual(marked(m));
  const effects = m.update({ effects: [] }).state;
  expect(marked(effects)).toEqual(marked(m));
});

test('hasClaudeMark is false without the field and when unmarked, true when marked', () => {
  expect(hasClaudeMark(EditorState.create({ doc: 'a' }))).toBe(false);
  const s = make('a');
  expect(hasClaudeMark(s)).toBe(false);
  expect(
    hasClaudeMark(
      s.update({ changes: { from: 0, insert: 'b' }, annotations: claudeEdit.of(true) }).state,
    ),
  ).toBe(true);
});

test('createExtensions includes the field', async () => {
  const { GlobalRegistrator } = await import('@happy-dom/global-registrator');
  GlobalRegistrator.register();
  try {
    const { createEditorState } = await import('../src/lib/editor/setup');
    const runtime = {
      scheduler: {},
      store: { subscribe: () => () => {} },
      evaluate: async () => true,
      stop: () => {},
      resumeAudio: async () => true,
    } as never;
    const s = createEditorState('a\nb', runtime);
    expect(hasClaudeMark(s)).toBe(false);
    const next = s.update({
      changes: { from: 0, insert: 'x' },
      annotations: claudeEdit.of(true),
    }).state;
    expect(hasClaudeMark(next)).toBe(true);
  } finally {
    GlobalRegistrator.unregister();
  }
});

test('typing right after an isolated claude edit is a separate undo step', () => {
  let s = make('hello', [history()]);
  s = s.update({
    changes: { from: 5, insert: ' world' },
    annotations: [claudeEdit.of(true), isolateHistory.of('full')],
  }).state;
  s = s.update({ changes: { from: 11, insert: '!' }, userEvent: 'input.type' }).state;
  expect(s.doc.toString()).toBe('hello world!');
  let cur = s;
  undo({ state: cur, dispatch: (tr) => (cur = tr.state) });
  expect(cur.doc.toString()).toBe('hello world');
  undo({ state: cur, dispatch: (tr) => (cur = tr.state) });
  expect(cur.doc.toString()).toBe('hello');
});

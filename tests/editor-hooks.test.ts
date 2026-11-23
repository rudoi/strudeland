import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createEditor, createEditorState, createExtensions } =
  await import('../src/lib/editor/setup');
const { EditorView, keymap } = await import('@codemirror/view');
const { EditorSelection } = await import('@codemirror/state');
import type { EditorState } from '@codemirror/state';
import type { KeyBinding } from '@codemirror/view';
import type { EditorHooks } from '../src/lib/editor/setup';
import type { NoteAuditionEdit } from '../src/lib/editor/note-audition';
import type { Runtime } from '../src/lib/strudel/runtime';

function fakeRuntime() {
  const calls: string[] = [];
  return {
    calls,
    runtime: {
      scheduler: {},
      drawingScheduler: {},
      store: { subscribe: () => () => {} },
      evaluate: async (c: string) => (calls.push('eval:' + c), true),
      stop: () => void calls.push('stop'),
      resumeAudio: async () => true,
      canAudition: () => true,
      prepareAudition: async () => true,
      start: async () => true,
      cycleTime: () => null,
      transport: {
        store: { subscribe: () => () => {} },
        startFrom: () => {},
        loopSection: () => {},
        loopRange: () => {},
        playEnding: () => {},
        reset: () => {},
      },
      position: () => ({ cycle: 0, located: null }),
      query: () => null,
      readScope: () => false,
      drainLevels: () => null,
      levels: () => null,
      setLimiter: () => {},
    } satisfies Runtime,
  };
}

/** The highest-precedence binding for `key`, as CodeMirror would pick it. */
function binding(state: EditorState, key: string): KeyBinding {
  const found = state
    .facet(keymap)
    .flat()
    .find((b) => b.key === key);
  if (!found?.run) throw new Error(`no binding for ${key}`);
  return found;
}
const press = (view: InstanceType<typeof EditorView>, key: string) =>
  binding(view.state, key).run!(view);

test('note audition only inspects editor input when enabled', () => {
  const { runtime } = fakeRuntime();
  let enabled = false;
  const edits: (NoteAuditionEdit | null)[] = [];
  const view = createEditor(document.body, 'note("g").s("sine")', runtime, {
    auditionEnabled: () => enabled,
    onAuditionEdit: (edit) => edits.push(edit),
  });
  view.dispatch({ selection: { anchor: 7 } });
  view.dispatch({
    changes: { from: 7, insert: '2' },
    selection: { anchor: 8 },
    userEvent: 'input.type',
  });
  expect(edits).toEqual([]);

  enabled = true;
  view.dispatch({ selection: { anchor: 8, head: 7 } });
  view.dispatch({
    changes: { from: 7, to: 8, insert: '3' },
    selection: { anchor: 8 },
    userEvent: 'input.type',
  });
  expect(edits.at(-1)).toMatchObject({ kind: 'note', pitch: 55, sound: 'sine' });
  view.dispatch({ changes: { from: 7, to: 8, insert: '4' }, userEvent: 'reload' });
  expect(edits.at(-1)).toBeNull();
  view.destroy();
});

function hooked() {
  const fake = fakeRuntime();
  const fired: string[] = [];
  const hooks: EditorHooks = {
    onSave: () => void fired.push('save'),
    onEvaluate: () => void fired.push('evaluate'),
    onDocChange: () => void fired.push('change'),
  };
  return { ...fake, fired, hooks };
}

test('createEditorState holds the doc and the same extensions as createEditor', () => {
  const { runtime } = fakeRuntime();
  const state = createEditorState('s("sine")', runtime);
  expect(state.doc.toString()).toBe('s("sine")');
  const view = createEditor(document.body, 'x', runtime);
  expect(state.facet(keymap).length).toBe(view.state.facet(keymap).length);
  expect(state.facet(EditorView.decorations).length).toBe(
    view.state.facet(EditorView.decorations).length,
  );
  expect(binding(state, 'Mod-Enter')).toBeDefined();
  expect(binding(state, 'Mod-.')).toBeDefined();
  view.destroy();
});

test('without hooks, Mod-Enter evaluates through the runtime and there is no Mod-s', () => {
  const { calls, runtime } = fakeRuntime();
  const view = createEditor(document.body, 'note("c")', runtime);
  expect(press(view, 'Mod-Enter')).toBe(true);
  expect(press(view, 'Ctrl-Enter')).toBe(true);
  expect(press(view, 'Mod-.')).toBe(true);
  expect(calls).toEqual(['eval:note("c")', 'eval:note("c")', 'stop']);
  expect(
    view.state
      .facet(keymap)
      .flat()
      .some((b) => b.key === 'Mod-s'),
  ).toBe(false);
  view.destroy();
});

test('Mod-s runs onSave once and prevents the default', () => {
  const { runtime, hooks, fired, calls } = hooked();
  const view = createEditor(document.body, 'x', runtime, hooks);
  const save = binding(view.state, 'Mod-s');
  expect(save.preventDefault).toBe(true);
  expect(save.run!(view)).toBe(true);
  expect(fired).toEqual(['save']);
  expect(calls).toEqual([]);
  view.destroy();
});

test('Mod-Enter and Ctrl-Enter run onEvaluate instead of the runtime', () => {
  const { runtime, hooks, fired, calls } = hooked();
  const view = createEditor(document.body, 'x', runtime, hooks);
  expect(press(view, 'Mod-Enter')).toBe(true);
  expect(press(view, 'Ctrl-Enter')).toBe(true);
  expect(fired).toEqual(['evaluate', 'evaluate']);
  expect(calls).toEqual([]);
  view.destroy();
});

test('hooks without onEvaluate fall back to the runtime; Mod-. still stops', () => {
  const { calls, runtime } = fakeRuntime();
  const saved: string[] = [];
  const view = createEditor(document.body, 'y', runtime, { onSave: () => void saved.push('s') });
  press(view, 'Mod-Enter');
  press(view, 'Mod-.');
  press(view, 'Mod-s');
  expect(calls).toEqual(['eval:y', 'stop']);
  expect(saved).toEqual(['s']);
  view.destroy();
});

test('onDocChange fires on an insert, not on a selection-only transaction', () => {
  const { runtime, hooks, fired } = hooked();
  const view = createEditor(document.body, 'abc', runtime, hooks);
  view.dispatch({ selection: EditorSelection.cursor(1) });
  expect(fired).toEqual([]);
  view.dispatch({ changes: { from: 0, insert: 'z' } });
  expect(fired).toEqual(['change']);
  expect(view.state.doc.toString()).toBe('zabc');
  view.destroy();
});

test('createExtensions without onDocChange installs no listener', () => {
  const { runtime } = fakeRuntime();
  const withHook = createExtensions(runtime, { onDocChange: () => {} });
  expect(withHook.length).toBe(createExtensions(runtime).length + 1);
  expect(createExtensions(runtime, {}).length).toBe(createExtensions(runtime).length);
});

test('setState with a new editor state keeps keymap and hooks working', () => {
  const first = hooked();
  const view = createEditor(document.body, 'one', first.runtime, first.hooks);
  const second = hooked();
  view.setState(createEditorState('two', second.runtime, second.hooks));
  expect(view.state.doc.toString()).toBe('two');
  press(view, 'Mod-s');
  press(view, 'Mod-Enter');
  view.dispatch({ changes: { from: 3, insert: '!' } });
  expect(second.fired).toEqual(['save', 'evaluate', 'change']);
  expect(first.fired).toEqual([]);
  view.destroy();
});

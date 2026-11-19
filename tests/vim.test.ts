import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, afterEach, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createEditor, createEditorState, keymapCompartment, keymapFor, setKeymap, withKeymap } =
  await import('../src/lib/editor/setup');
const { createVimMode, defineVimWrite, vimAdapter, vimChip } =
  await import('../src/lib/editor/vim-mode');
const { get } = await import('svelte/store');
const { attachState } = await import('../src/lib/editor/visuals');
const { getCM, Vim } = await import('@replit/codemirror-vim');
const { EditorView } = await import('@codemirror/view');
import type { EditorHooks } from '../src/lib/editor/setup';
import { stubRuntime } from './helpers/stub-runtime';

type View = InstanceType<typeof EditorView>;

const views: View[] = [];
afterEach(() => {
  for (const v of views.splice(0)) v.destroy();
});

function editor(doc: string, vim: boolean, hooks: EditorHooks = {}) {
  const rt = stubRuntime();
  const fired: string[] = [];
  const view = createEditor(document.body, doc, rt.runtime, {
    onSave: () => void fired.push('save'),
    onEvaluate: () => void fired.push('evaluate'),
    vim: () => vim,
    ...hooks,
  });
  views.push(view);
  return { view, rt, fired };
}

const cm = (view: View) => vimAdapter(view)!;
const vimState = (view: View) => getCM(view)?.state.vim ?? null;
const hasVim = (view: View) => getCM(view) !== null;

/** A real keydown on the editor content, as the browser would deliver it (Mod is Ctrl here). */
function keydown(view: View, key: string, mods: { ctrl?: boolean; meta?: boolean } = {}) {
  const keyCode = key === 'Enter' ? 13 : key === '.' ? 190 : key.toUpperCase().charCodeAt(0);
  const event = new KeyboardEvent('keydown', {
    key,
    keyCode,
    ctrlKey: mods.ctrl ?? false,
    metaKey: mods.meta ?? false,
    bubbles: true,
    cancelable: true,
  });
  view.contentDOM.dispatchEvent(event);
  return event;
}

describe('keymapFor and withKeymap', () => {
  test('keymapFor returns the same object per value, different between values', () => {
    expect(keymapFor(true)).toBe(keymapFor(true));
    expect(keymapFor(false)).toBe(keymapFor(false));
    expect(keymapFor(true)).not.toBe(keymapFor(false));
  });

  test('new states take the vim hook; without it they use the default keymap', () => {
    const rt = stubRuntime();
    const plain = createEditorState('x', rt.runtime);
    expect(keymapCompartment.get(plain)).toBe(keymapFor(false));
    const on = createEditorState('x', rt.runtime, { vim: () => true });
    expect(keymapCompartment.get(on)).toBe(keymapFor(true));
  });

  test('the vim hook is read for each new state', () => {
    const rt = stubRuntime();
    let vim = false;
    const hooks = { vim: () => vim };
    expect(keymapCompartment.get(createEditorState('', rt.runtime, hooks))).toBe(keymapFor(false));
    vim = true;
    expect(keymapCompartment.get(createEditorState('', rt.runtime, hooks))).toBe(keymapFor(true));
  });

  test('withKeymap returns the same state when it already matches', () => {
    const rt = stubRuntime();
    const state = createEditorState('abc', rt.runtime);
    expect(withKeymap(state, false)).toBe(state);
    const on = createEditorState('abc', rt.runtime, { vim: () => true });
    expect(withKeymap(on, true)).toBe(on);
  });

  test('withKeymap switches the keymap and keeps the doc and selection', () => {
    const rt = stubRuntime();
    const state = createEditorState('abc', rt.runtime);
    const moved = state.update({ selection: { anchor: 2 } }).state;
    const on = withKeymap(moved, true);
    expect(on).not.toBe(moved);
    expect(keymapCompartment.get(on)).toBe(keymapFor(true));
    expect(on.doc.toString()).toBe('abc');
    expect(on.selection.main.head).toBe(2);
    expect(keymapCompartment.get(withKeymap(on, false))).toBe(keymapFor(false));
  });

  test('a vim state shown in a view starts in normal mode', () => {
    const { view } = editor('abc', false);
    expect(hasVim(view)).toBe(false);
    attachState(view, withKeymap(view.state, true));
    expect(hasVim(view)).toBe(true);
    expect(vimState(view)?.insertMode).toBe(false);
    expect(view.scrollDOM.classList.contains('cm-vimMode')).toBe(true);
  });
});

test('a state swap from insert mode leaves no insert-mode key listener behind', () => {
  const { view } = editor('abc', true);
  Vim.handleKey(cm(view), 'i', 'user');
  expect(vimState(view)?.insertMode).toBe(true);
  attachState(view, withKeymap(createEditorState('xyz', stubRuntime().runtime), true));
  setKeymap(view, false);
  const last = Vim.getVimGlobalState_().macroModeState.lastInsertModeChanges;
  last.maybeReset = false;
  const before = last.changes.length;
  keydown(view, 'Backspace');
  expect(last.changes.length).toBe(before);
});

describe('setKeymap', () => {
  test('toggles vim on and off on the shown view', () => {
    const { view } = editor('abc', false);
    expect(setKeymap(view, true)).toBe(true);
    expect(hasVim(view)).toBe(true);
    expect(view.scrollDOM.classList.contains('cm-vimMode')).toBe(true);
    expect(vimState(view)?.insertMode).toBe(false);
    expect(setKeymap(view, false)).toBe(true);
    expect(hasVim(view)).toBe(false);
    expect(view.scrollDOM.classList.contains('cm-vimMode')).toBe(false);
    expect(view.state.doc.toString()).toBe('abc');
  });

  test('returns false and does nothing when the view already has that keymap', () => {
    const { view } = editor('abc', true);
    const state = view.state;
    expect(setKeymap(view, true)).toBe(false);
    expect(view.state).toBe(state);
    const off = editor('abc', false).view;
    const offState = off.state;
    expect(setKeymap(off, false)).toBe(false);
    expect(off.state).toBe(offState);
  });

  test('turning vim on again starts in normal mode', () => {
    const { view } = editor('abc', true);
    Vim.handleKey(cm(view), 'i', 'user');
    expect(vimState(view)?.insertMode).toBe(true);
    setKeymap(view, false);
    setKeymap(view, true);
    expect(vimState(view)?.insertMode).toBe(false);
  });

  test('off from insert mode with the : panel open leaves no vim class or panel', () => {
    const { view } = editor('abc', true);
    Vim.handleKey(cm(view), 'i', 'user');
    expect(vimState(view)?.insertMode).toBe(true);
    // Open the ex prompt from insert mode (<C-o>:), then toggle off.
    Vim.handleKey(cm(view), '<C-o>', 'user');
    Vim.handleKey(cm(view), ':', 'user');
    expect(vimState(view)?.insertModeReturn).toBe(true);
    expect(view.dom.querySelector('.cm-vim-panel')).not.toBeNull();
    expect(setKeymap(view, false)).toBe(true);
    expect(hasVim(view)).toBe(false);
    expect(view.scrollDOM.classList.contains('cm-vimMode')).toBe(false);
    expect(view.dom.querySelector('.cm-vim-panel')).toBeNull();
    // Typing still edits the doc normally.
    view.dispatch({ changes: { from: 0, insert: 'z' } });
    expect(view.state.doc.toString()).toBe('zabc');
  });

  // `A` leaves the cursor after the text; the doc is unchanged in every mode.
  test.each([
    ['normal', [], [], 0],
    ['insert', ['A'], [true], 3],
    ['visual', ['v'], [], null],
  ] as const)('off from %s mode removes vim; exitInsertMode calls: %p', (_, keys, want, head) => {
    const { view } = editor('abc', true);
    for (const k of keys) Vim.handleKey(cm(view), k, 'user');
    const original = Vim.exitInsertMode;
    const exits: (boolean | undefined)[] = [];
    Vim.exitInsertMode = (c, keepCursor) => (exits.push(keepCursor), original(c, keepCursor));
    try {
      expect(setKeymap(view, false)).toBe(true);
    } finally {
      Vim.exitInsertMode = original;
    }
    expect(exits).toEqual([...want]);
    expect(hasVim(view)).toBe(false);
    expect(view.scrollDOM.classList.contains('cm-vimMode')).toBe(false);
    if (head !== null) {
      expect(view.state.selection.main.head).toBe(head);
      view.dispatch({ changes: { from: head, insert: '!' } });
      expect(view.state.doc.toString()).toBe('abc'.slice(0, head) + '!' + 'abc'.slice(head));
    }
  });
});

describe("strudeland's shortcuts under vim (real keydown)", () => {
  const cases: [string, string, { ctrl?: boolean; meta?: boolean }, string][] = [
    ['Mod-Enter', 'Enter', { ctrl: true }, 'evaluate'],
    ['Ctrl-Enter', 'Enter', { ctrl: true }, 'evaluate'],
    ['Mod-.', '.', { ctrl: true }, 'stop'],
    ['Mod-s', 's', { ctrl: true }, 'save'],
  ];

  for (const mode of ['normal', 'insert'] as const) {
    for (const [name, key, mods, expected] of cases) {
      test(`${name} in ${mode} mode`, () => {
        const { view, rt, fired } = editor('note("c")', true);
        if (mode === 'insert') Vim.handleKey(cm(view), 'i', 'user');
        expect(vimState(view)?.insertMode).toBe(mode === 'insert');
        const event = keydown(view, key, mods);
        const got = expected === 'stop' ? (rt.stops() > 0 ? ['stop'] : []) : fired;
        expect(got).toEqual([expected]);
        expect(event.defaultPrevented).toBe(true);
        expect(view.state.doc.toString()).toBe('note("c")');
        expect(vimState(view)?.insertMode).toBe(mode === 'insert');
      });
    }
  }

  test('the shortcuts also fire with vim off', () => {
    const { view, rt, fired } = editor('x', false);
    keydown(view, 'Enter', { ctrl: true });
    keydown(view, '.', { ctrl: true });
    keydown(view, 's', { ctrl: true });
    expect(fired).toEqual(['evaluate', 'save']);
    expect(rt.stops()).toBe(1);
  });

  test('a plain key in normal mode is a vim command, not text', () => {
    const { view } = editor('abc', true);
    keydown(view, 'x');
    expect(view.state.doc.toString()).toBe('bc');
  });
});

describe(':w', () => {
  let restore: (() => void) | null = null;
  afterEach(() => {
    restore?.();
    restore = null;
  });

  test(':w calls save; the restore function makes it a no-op', () => {
    const { view } = editor('abc', true);
    const saved: string[] = [];
    restore = defineVimWrite(() => void saved.push('save'));
    Vim.handleEx(cm(view), 'w');
    Vim.handleEx(cm(view), 'write');
    expect(saved).toEqual(['save', 'save']);
    restore();
    restore = null;
    Vim.handleEx(cm(view), 'w');
    expect(saved).toEqual(['save', 'save']);
    expect(view.state.doc.toString()).toBe('abc');
  });

  test('the last definition wins', () => {
    const { view } = editor('abc', true);
    const saved: string[] = [];
    const first = defineVimWrite(() => void saved.push('first'));
    restore = defineVimWrite(() => void saved.push('second'));
    Vim.handleEx(cm(view), 'w');
    expect(saved).toEqual(['second']);
    first();
    Vim.handleEx(cm(view), 'w');
    expect(saved).toEqual(['second', 'second']);
  });
});

describe('external edits', () => {
  test('a code.set-style dispatch in insert mode keeps insert mode', () => {
    const { view } = editor('abc\ndef', true);
    Vim.handleKey(cm(view), 'i', 'user');
    expect(vimState(view)?.insertMode).toBe(true);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: 'xy' },
      selection: { anchor: Math.min(view.state.selection.main.head, 2) },
    });
    expect(view.state.doc.toString()).toBe('xy');
    expect(vimState(view)?.insertMode).toBe(true);
  });

  test('a code.set-style dispatch in normal mode keeps normal mode', () => {
    const { view } = editor('abc', true);
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: 'q' } });
    expect(vimState(view)?.insertMode).toBe(false);
    expect(hasVim(view)).toBe(true);
  });
});

describe('vim mode store', () => {
  const modes: ReturnType<typeof createVimMode>[] = [];
  afterEach(() => {
    for (const m of modes.splice(0)) m.destroy();
  });
  function store() {
    const m = createVimMode();
    modes.push(m);
    return m;
  }

  test.each([
    [[], { mode: 'normal' }],
    [['i'], { mode: 'insert' }],
    [['v'], { mode: 'visual' }],
    [['V'], { mode: 'visual', subMode: 'linewise' }],
    [['<C-v>'], { mode: 'visual', subMode: 'blockwise' }],
  ] as const)('starts null; attaching after %p seeds %p', (keys, want) => {
    const m = store();
    expect(get(m.store)).toBeNull();
    const { view } = editor('abc\ndef', true);
    for (const k of keys) Vim.handleKey(cm(view), k, 'user');
    m.attach(view);
    expect(get(m.store)).toEqual(want);
  });

  test('follows i, v, V, <C-v>, R and <Esc>', () => {
    const m = store();
    const { view } = editor('abc\ndef', true);
    m.attach(view);
    const seen: unknown[] = [];
    const unsub = m.store.subscribe((s) => void seen.push(s));
    const keys = ['i', '<Esc>', 'v', '<Esc>', 'V', '<Esc>', '<C-v>', '<Esc>', 'R', '<Esc>'];
    for (const k of keys) Vim.handleKey(cm(view), k, 'user');
    unsub();
    // Each mode, and normal after each <Esc>; repeats of the same value may appear.
    const distinct = seen.filter((s, i) => JSON.stringify(s) !== JSON.stringify(seen[i - 1]));
    expect(distinct).toEqual([
      { mode: 'normal' },
      { mode: 'insert' },
      { mode: 'normal' },
      { mode: 'visual' },
      { mode: 'normal' },
      { mode: 'visual', subMode: 'linewise' },
      { mode: 'normal' },
      { mode: 'visual', subMode: 'blockwise' },
      { mode: 'normal' },
      { mode: 'replace' },
      { mode: 'normal' },
    ]);
  });

  test('<C-o> from insert mode reads as normal', () => {
    const m = store();
    const { view } = editor('abc', true);
    m.attach(view);
    Vim.handleKey(cm(view), 'i', 'user');
    Vim.handleKey(cm(view), '<C-o>', 'user');
    expect(get(m.store)).toEqual({ mode: 'normal' });
  });

  test('re-attaches after a state swap; the old adapter is dropped', () => {
    const m = store();
    const { view } = editor('abc', true);
    m.attach(view);
    const old = cm(view);
    Vim.handleKey(old, 'i', 'user');
    expect(get(m.store)).toEqual({ mode: 'insert' });
    attachState(view, withKeymap(createEditorState('xyz', stubRuntime().runtime), true));
    expect(cm(view)).not.toBe(old);
    m.attach(view);
    expect(get(m.store)).toEqual({ mode: 'normal' });
    Vim.handleKey(cm(view), 'v', 'user');
    expect(get(m.store)).toEqual({ mode: 'visual' });
  });

  test('attaching a second view detaches from the first', () => {
    const m = store();
    const a = editor('abc', true).view;
    const b = editor('abc', true).view;
    m.attach(a);
    m.attach(b);
    Vim.handleKey(cm(a), 'i', 'user');
    expect(get(m.store)).toEqual({ mode: 'normal' });
    Vim.handleKey(cm(b), 'v', 'user');
    expect(get(m.store)).toEqual({ mode: 'visual' });
  });

  test('a view without vim, attach(undefined) and destroy give null', () => {
    const m = store();
    const { view } = editor('abc', true);
    m.attach(view);
    m.attach(editor('abc', false).view);
    expect(get(m.store)).toBeNull();
    m.attach(view);
    expect(get(m.store)).toEqual({ mode: 'normal' });
    m.attach(undefined);
    expect(get(m.store)).toBeNull();
    Vim.handleKey(cm(view), 'i', 'user');
    expect(get(m.store)).toBeNull();
    m.attach(view);
    m.destroy();
    expect(get(m.store)).toBeNull();
    Vim.handleKey(cm(view), '<Esc>', 'user');
    expect(get(m.store)).toBeNull();
  });

  test('a keymap toggle needs a re-attach: off gives null, on starts in normal mode', () => {
    const m = store();
    const { view } = editor('abc', true);
    m.attach(view);
    Vim.handleKey(cm(view), 'i', 'user');
    expect(setKeymap(view, false)).toBe(true);
    m.attach(view);
    expect(get(m.store)).toBeNull();
    expect(setKeymap(view, true)).toBe(true);
    m.attach(view);
    expect(get(m.store)).toEqual({ mode: 'normal' });
  });
});

describe('vimChip', () => {
  test('off is a plain vim chip', () => {
    expect(vimChip(null)).toEqual({ label: 'vim', tone: 'neutral', lit: false });
  });

  const rows: [Parameters<typeof vimChip>[0], string, string][] = [
    [{ mode: 'normal' }, 'vim · normal', 'neutral'],
    [{ mode: 'insert' }, 'vim · insert', 'accent'],
    [{ mode: 'visual' }, 'vim · visual', 'warning'],
    [{ mode: 'visual', subMode: 'linewise' }, 'vim · v-line', 'warning'],
    [{ mode: 'visual', subMode: 'blockwise' }, 'vim · v-block', 'warning'],
    [{ mode: 'replace' }, 'vim · replace', 'danger'],
  ];
  for (const [mode, label, tone] of rows) {
    test(`${label}`, () => {
      expect(vimChip(mode)).toEqual({ label, tone: tone as never, lit: true });
    });
  }
});

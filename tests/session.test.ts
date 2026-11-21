import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, mock, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { buildStructure, cpsFor } = await import('../src/lib/strudel/structure');
const {
  createSession,
  lineChanges,
  mergeOrder,
  replaceChange,
  shouldAutoEval,
  songTitle,
  toLF,
  windowTitle,
} = await import('../src/lib/project/session');
const { createEditorState, keymapCompartment, keymapFor, setKeymap, withKeymap } =
  await import('../src/lib/editor/setup');
const { commitInput } = await import('../src/lib/project/timeline');
const { EditorView } = await import('@codemirror/view');
const { redo, undo, undoDepth } = await import('@codemirror/commands');
const { claudeMarkField, hasClaudeMark } = await import('../src/lib/editor/claude-mark');
const { get } = await import('svelte/store');
const { highlightMiniLocations, updateMiniLocations } = await import('@strudel/codemirror');
import type { Session } from '../src/lib/project/session';
import type { EditorState } from '@codemirror/state';
import { ProjectError } from '../src/lib/project/api';
import { deferred, fakeProjectApi, flush } from './helpers/fake-project-api';
import { stubRuntime } from './helpers/stub-runtime';

// ---------------------------------------------------------------- fixtures

const A = `const meta = { title: 'Alpha' };\n$: s("a")\n`;
const B = `$: s("b")\n`; // no header: title falls back to the file name
const C = `const meta = { title: 'Gamma' };\n$: s(\n`; // body syntax error: the header still reads
const Q = `const meta = { title: 'Queue' };\n$: s("q")\n`;

const PA = 'songs/a.js';
const PB = 'songs/b.js';
const PC = 'songs/c.js';

/** Ranges currently drawn by the mini-notation highlights (marks styled inline, unlike the rest). */
function highlighted(view: InstanceType<typeof EditorView>): [number, number][] {
  const out: [number, number][] = [];
  for (const source of view.state.facet(EditorView.decorations)) {
    const set = typeof source === 'function' ? source(view) : source;
    for (let it = set.iter(); it.value; it.next()) {
      if (it.value.spec.attributes?.style) out.push([it.from, it.to]);
    }
  }
  return out;
}
const located = (start: number, end: number) => ({
  value: {},
  whole: { begin: 0 },
  context: { locations: [{ start, end }] },
});

const PROJECTS = {
  '/p': { name: 'Proj', files: { [PA]: A, [PB]: B, [PC]: C } },
  '/q': { name: 'Other', files: { 'songs/q.js': Q } },
  '/empty': { name: 'Empty', files: {} },
  '/one': { name: 'One', files: { [PA]: A } },
};

function setup(
  opts: {
    last?: string | null;
    tauri?: boolean;
    withView?: boolean;
    beforeProjectChange?: () => Promise<void>;
    prepareState?: (state: EditorState) => EditorState;
    onShown?: (view: InstanceType<typeof EditorView>) => void;
    vim?: () => boolean;
    formatOnSave?: () => boolean;
  } = {},
) {
  const { api, fake } = fakeProjectApi(PROJECTS);
  fake.last = opts.last === undefined ? '/p' : opts.last;
  const rt = stubRuntime();
  const view = opts.withView === false ? undefined : new EditorView({ parent: document.body });
  const created: string[] = [];
  let session!: Session;
  session = createSession({
    api,
    runtime: rt.runtime,
    createState: (doc: string) => {
      created.push(doc);
      return createEditorState(doc, rt.runtime, {
        onDocChange: () => session.docChanged(),
        vim: opts.vim,
      });
    },
    isTauri: () => opts.tauri ?? true,
    formatOnSave: opts.formatOnSave,
    beforeProjectChange: opts.beforeProjectChange,
    prepareState: opts.prepareState,
    onShown: opts.onShown,
  });
  session.attachView(view);
  const st = () => get(session.store);
  const title = () => windowTitle(st());
  const v = () => view!;
  const doc = () => v().state.doc.toString();
  const item = (path: string) => st().songs.find((s) => s.path === path);
  /** Type at the end of the doc, as the user would. */
  const type = (text: string) =>
    v().dispatch({
      changes: { from: v().state.doc.length, insert: text },
      userEvent: 'input.type',
    });
  const backspace = (n: number) =>
    v().dispatch({
      changes: { from: v().state.doc.length - n, to: v().state.doc.length },
      userEvent: 'delete.backward',
    });
  const undoView = () => undo({ state: v().state, dispatch: (tr) => v().dispatch(tr) });
  const closeWindow = async () => {
    let prevented = false;
    await fake.closeHandlers[0]({ preventDefault: () => void (prevented = true) });
    return prevented;
  };
  return {
    api,
    fake,
    rt,
    view,
    session,
    st,
    doc,
    item,
    type,
    backspace,
    undoView,
    closeWindow,
    created,
    title,
  };
}

async function opened(opts: Parameters<typeof setup>[0] = {}) {
  const h = setup(opts);
  await h.session.init();
  await flush();
  return h;
}

// ---------------------------------------------------------------- helpers

/** What `p` rejects with (fails the test if it resolves). */
async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  throw new Error('expected a rejection');
}

/** The line numbers carrying a Claude-edit mark. */
function markedLines(state: import('@codemirror/state').EditorState): number[] {
  const out: number[] = [];
  state.field(claudeMarkField).between(0, state.doc.length, (from) => {
    out.push(state.doc.lineAt(from).number);
  });
  return out;
}

describe('pure helpers', () => {
  test('mergeOrder follows the listing and keeps unlisted songs after their neighbour', () => {
    expect(mergeOrder(['a', 'b', 'c'], ['c', 'a', 'b'])).toEqual(['a', 'b', 'c']);
    // listed but not known: left out
    expect(mergeOrder(['a', 'x', 'b'], ['a', 'b'])).toEqual(['a', 'b']);
    // known but not listed (a deleted conflict): stays after the path it followed
    expect(mergeOrder(['a', 'c', 'd'], ['a', 'b', 'c', 'd'])).toEqual(['a', 'b', 'c', 'd']);
    expect(mergeOrder(['b', 'c'], ['a', 'b', 'c'])).toEqual(['a', 'b', 'c']);
    expect(mergeOrder(['ab', 'b'], ['a', 'b', 'ab'])).toEqual(['a', 'ab', 'b']);
    expect(mergeOrder([], ['x', 'y'])).toEqual(['x', 'y']);
  });

  test('windowTitle: strudeland, the project name, or project — song with a dirty dot', () => {
    const base = {
      available: true,
      recents: [],
      project: null,
      songs: [],
      active: null,
      error: null,
      metaError: null,
      structure: null,
      evaluated: null,
      notice: null,
    };
    const project = { root: '/p', name: 'Proj', songs: [] };
    const song = { path: PA, title: 'Alpha', dirty: false, conflict: null };
    expect(windowTitle(base)).toBe('strudeland');
    expect(windowTitle({ ...base, project })).toBe('Proj');
    expect(windowTitle({ ...base, project, songs: [song], active: PA })).toBe('Proj — Alpha');
    expect(windowTitle({ ...base, project, songs: [{ ...song, dirty: true }], active: PA })).toBe(
      'Proj — Alpha ●',
    );
  });

  test('lineChanges: one trimmed change per run of changed lines', () => {
    const apply = (from: string, to: string) => {
      let out = from;
      for (const c of [...lineChanges(from, to)].reverse()) {
        out = out.slice(0, c.from) + c.insert + out.slice(c.to);
      }
      return out;
    };
    expect(lineChanges('a\nb\nc\nd\n', 'a\nB\nc\nD\n')).toEqual([
      { from: 2, to: 3, insert: 'B' },
      { from: 6, to: 7, insert: 'D' },
    ]);
    expect(lineChanges('a', 'a\nb')).toEqual([{ from: 1, to: 1, insert: '\nb' }]);
    expect(lineChanges('a\nb\n', 'b\n')).toEqual([{ from: 0, to: 2, insert: '' }]);
    expect(lineChanges('x', 'x')).toEqual([]);
    for (const [x, y] of [
      ['', 'a\nb\n'],
      ['a\nb\n', ''],
      ['a\nb\nc', 'c\nb\na'],
      ['1\n2\n3\n4\n5\n', '1\nx\n3\ny\nz\n5\n6'],
      ['a\n\n\nb', '\na\n\nb\n'],
    ]) {
      expect(apply(x, y)).toBe(y);
    }
  });

  test('lineChanges falls back to one replaceChange for a huge changed middle', () => {
    const lines = (tag: string) => Array.from({ length: 2001 }, (_, i) => `${tag}${i}\n`).join('');
    const from = 'head\n' + lines('a') + 'tail\n';
    const to = 'head\n' + lines('b') + 'tail\n';
    expect(lineChanges(from, to)).toEqual([replaceChange(from, to)]);
  });

  test('replaceChange keeps the common prefix and suffix', () => {
    expect(replaceChange('abcdef', 'abXYef')).toEqual({ from: 2, to: 4, insert: 'XY' });
    expect(replaceChange('abc', 'abc')).toEqual({ from: 3, to: 3, insert: '' });
    expect(replaceChange('', 'new')).toEqual({ from: 0, to: 0, insert: 'new' });
    expect(replaceChange('old', '')).toEqual({ from: 0, to: 3, insert: '' });
    expect(replaceChange('aaa', 'aa')).toEqual({ from: 2, to: 3, insert: '' });
    for (const [x, y] of [
      ['aba', 'abba'],
      ['hello world', 'help world'],
      ['xyz', 'abc'],
    ]) {
      const c = replaceChange(x, y);
      expect(x.slice(0, c.from) + c.insert + x.slice(c.to)).toBe(y);
    }
  });

  test('songTitle: meta.title, else the file name', () => {
    expect(songTitle(PA, A)).toBe('Alpha');
    expect(songTitle(PB, B)).toBe('b');
    expect(songTitle(PC, C)).toBe('Gamma');
    expect(songTitle(PC, `const meta = { title: 'Gamma' `)).toBe('c'); // header syntax error
    expect(songTitle(PB, null)).toBe('b');
    expect(songTitle('songs/my song.js', `const meta = { title: '' };`)).toBe('my song');
    expect(songTitle(PA, `const meta = { title: 7 };`)).toBe('a');
  });
});

// ---------------------------------------------------------------- init

describe('init', () => {
  test('reopens the last project', async () => {
    const h = await opened();
    expect(h.fake.calls).toContain('last');
    expect(h.st().project?.name).toBe('Proj');
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
    expect(h.st().error).toBeNull();
  });

  test('with no last project, project stays null', async () => {
    const h = await opened({ last: null });
    expect(h.st().project).toBeNull();
    expect(h.st().songs).toEqual([]);
    expect(h.st().active).toBeNull();
    expect(h.title()).toBe('strudeland');
  });

  test('a failing project_last sets error', async () => {
    const h = setup();
    h.fake.lastFails = true;
    await h.session.init();
    expect(h.st().project).toBeNull();
    expect(h.st().error).toBe('recents are broken');
  });

  test('outside Tauri it does nothing and reports projects unavailable', async () => {
    const h = setup({ tauri: false });
    await h.session.init();
    h.session.docChanged(); // the editor exists in the browser too
    expect(h.fake.calls).toEqual([]);
    expect(h.st().available).toBe(false);
    expect(h.st().recents).toEqual([]);
    expect(h.st().project).toBeNull();
  });

  test('in Tauri, projects are available and init loads the recents', async () => {
    const h = setup({ last: null });
    h.fake.recents = [{ path: '/q', name: 'Other' }];
    expect(h.st().available).toBe(true);
    await h.session.init();
    expect(h.st().recents).toEqual([{ path: '/q', name: 'Other' }]);
  });

  test('the recents load even when project_last fails', async () => {
    const h = setup();
    h.fake.lastFails = true;
    h.fake.recents = [{ path: '/q', name: 'Other' }];
    await h.session.init();
    expect(h.st().error).toBe('recents are broken');
    expect(h.st().recents).toHaveLength(1);
  });

  test('a failing recent_list keeps the list empty and sets no error', async () => {
    const h = setup({ last: null });
    h.fake.recentsFail = true;
    await h.session.init();
    expect(h.st().recents).toEqual([]);
    expect(h.st().error).toBeNull();
  });

  test('the recents are refreshed after an open', async () => {
    const h = await opened({ last: null });
    h.fake.recents = [{ path: '/q', name: 'Other' }];
    await h.session.openRecent('/q');
    expect(h.st().recents).toEqual([{ path: '/q', name: 'Other' }]);
  });

  test('registers the close guard once, even when called twice', async () => {
    const h = await opened();
    await h.session.init();
    expect(h.fake.calls.filter((c) => c === 'onCloseRequested')).toHaveLength(1);
  });

  test('a view attached after init gets the active song', async () => {
    const h = await opened({ withView: false });
    expect(h.st().active).toBe(PA);
    const view = new EditorView({ parent: document.body });
    h.session.attachView(view);
    expect(view.state.doc.toString()).toBe(A);
  });

  test('without a view, edits live in the stash; a detached view hands its state back', async () => {
    const h = await opened();
    h.type('// one');
    h.session.attachView(undefined);
    await h.session.switchSong(PB);
    expect(h.item(PA)?.dirty).toBe(true);
    const view = new EditorView({ parent: document.body });
    h.session.attachView(view);
    expect(view.state.doc.toString()).toBe(B);
    await h.session.switchSong(PA);
    expect(view.state.doc.toString()).toBe(A + '// one');
  });

  test('read paths do not touch the view', async () => {
    const h = await opened();
    const state = h.view!.state;
    h.session.docChanged();
    expect(await h.session.guardUnsaved()).toBe(true);
    expect(h.st().songs).toHaveLength(3);
    expect(h.view!.state).toBe(state);
  });
});

// ---------------------------------------------------------------- open

describe('open', () => {
  test('the first sorted song is active; titles from meta or the file name', async () => {
    const h = await opened();
    expect(h.st().songs.map((s) => [s.path, s.title])).toEqual([
      [PA, 'Alpha'],
      [PB, 'b'],
      [PC, 'Gamma'],
    ]);
    expect(h.st().songs.every((s) => !s.dirty && s.conflict === null)).toBe(true);
    // every song is read once at open, for its title
    expect(h.fake.calls.filter((c) => c.startsWith('read:'))).toEqual([
      `read:${PA}`,
      `read:${PB}`,
      `read:${PC}`,
    ]);
  });

  test('a failed read gives the file name as title and sets error', async () => {
    const h = setup();
    h.fake.failRead.add(PB);
    await h.session.init();
    expect(h.item(PB)?.title).toBe('b');
    expect(h.st().error).toContain('permission denied');
    expect(h.st().active).toBe(PA);
  });

  test('if the first song cannot be read, the next readable one is active', async () => {
    const h = setup();
    h.fake.failRead.add(PA);
    await h.session.init();
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe(B);
    expect(h.item(PA)?.title).toBe('a');
  });

  test('a song whose read failed is retried on switch', async () => {
    const h = setup();
    h.fake.failRead.add(PB);
    await h.session.init();
    await h.session.switchSong(PB);
    expect(h.st().active).toBe(PA); // still failing: stay
    expect(h.st().error).toContain('permission denied');
    h.fake.failRead.delete(PB);
    await h.session.switchSong(PB);
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe(B);
  });

  test('opening stops playback', async () => {
    const h = await opened();
    expect(h.rt.stops()).toBe(1);
    await h.session.openRecent('/q');
    expect(h.rt.stops()).toBe(2);
  });

  test('a project with zero songs has no active song', async () => {
    const h = await opened({ last: '/empty' });
    expect(h.st().project?.name).toBe('Empty');
    expect(h.st().songs).toEqual([]);
    expect(h.st().active).toBeNull();
    expect(h.doc()).toBe('');
  });

  test('switching to a zero-song project blanks the editor and drops its history', async () => {
    const h = await opened();
    h.type('x');
    await h.session.save();
    await h.session.openRecent('/empty');
    expect(h.doc()).toBe('');
    expect(undoDepth(h.view!.state)).toBe(0);
  });

  test('the guard runs before switching projects; cancel aborts with no API calls', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.confirm.push('cancel');
    h.fake.calls.length = 0;
    await h.session.openRecent('/q');
    expect(h.fake.calls).toEqual(['confirm:1']);
    expect(h.st().project?.root).toBe('/p');
    expect(h.doc()).toBe(A + '// mine');
    expect(h.rt.stops()).toBe(1);
  });

  test('openProject picks, then guards; cancel does not open', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.picks.push('/q');
    h.fake.confirm.push('cancel');
    h.fake.calls.length = 0;
    await h.session.openProject();
    expect(h.fake.calls).toEqual(['pickFolder:Open project', 'confirm:1']);
    expect(h.st().project?.root).toBe('/p');
  });

  test('openProject: a cancelled picker does nothing', async () => {
    const h = await opened();
    h.fake.calls.length = 0;
    await h.session.openProject();
    expect(h.fake.calls).toEqual(['pickFolder:Open project']);
    expect(h.st().project?.root).toBe('/p');
  });

  test('openProject opens the picked folder', async () => {
    const h = await opened();
    h.fake.picks.push('/q');
    await h.session.openProject();
    expect(h.fake.calls).toContain('open:/q');
    expect(h.st().project?.name).toBe('Other');
    expect(h.st().active).toBe('songs/q.js');
    expect(h.doc()).toBe(Q);
  });

  test('guard discard proceeds and drops the old project', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.confirm.push('discard');
    await h.session.openRecent('/q');
    expect(h.st().project?.root).toBe('/q');
    expect(h.fake.disk.get('/p')!.files.get(PA)).toBe(A); // not saved
  });

  test('a failing folder picker sets error and returns null', async () => {
    const h = await opened({ last: null });
    h.fake.failPick = true;
    const askName = mock(async () => 'unused');
    await h.session.newProject(askName);
    expect(askName).not.toHaveBeenCalled();
    expect(h.st().error).toBe('dialog unavailable');
  });

  test('newProject picks a folder, asks for its name, then opens the new project', async () => {
    const h = await opened({ last: null });
    h.fake.picks.push('/new');
    const askName = mock(async (_dir: string) => 'Fresh');
    await h.session.newProject(askName);
    expect(askName).toHaveBeenCalledWith('/new');
    expect(h.fake.calls).toContain('pickFolder:New project folder');
    expect(h.fake.calls).toContain('create:/new:Fresh');
    expect(h.st().project).toMatchObject({ root: '/new', name: 'Fresh' });
    expect(h.st().active).toBe('songs/main.js');
    expect(h.item('songs/main.js')?.title).toBe('main');
  });

  test('newProject offers to preserve a deleted active song even when its buffer is clean', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(h.item(PA)).toMatchObject({ conflict: 'deleted', dirty: false });

    h.fake.picks.push('/new');
    h.fake.confirm.push('cancel');
    await h.session.newProject(async () => 'Fresh');
    expect(h.st().project?.root).toBe('/p');
    expect(h.doc()).toBe(A);
    expect(h.fake.calls).not.toContain('create:/new:Fresh');

    h.fake.picks.push('/new');
    h.fake.confirm.push('save');
    await h.session.newProject(async () => 'Fresh');
    expect(h.fake.confirmCounts).toEqual([1, 1]);
    expect(h.fake.disk.get('/p')?.files.get(PA)).toBe(A);
    expect(h.st().project?.root).toBe('/new');
  });

  test('openRecent opens the path', async () => {
    const h = await opened({ last: null });
    await h.session.openRecent('/q');
    expect(h.fake.calls).toContain('open:/q');
    expect(h.st().project?.root).toBe('/q');
  });

  test('an open error keeps the previous project and playback', async () => {
    const h = await opened();
    h.type('x');
    const songs = h.st().songs;
    h.fake.confirm.push('discard');
    await h.session.openRecent('/missing');
    expect(h.st().error).toContain('not a strudeland project');
    expect(h.st().project?.root).toBe('/p');
    expect(h.st().songs).toEqual(songs);
    expect(h.doc()).toBe(A + 'x');
    expect(h.rt.stops()).toBe(1);
  });

  test('a create error keeps the previous project', async () => {
    const h = await opened();
    h.fake.failOpen.add('/full');
    h.fake.picks.push('/full');
    await h.session.newProject(async () => 'X');
    expect(h.st().error).toContain('already contains files');
    expect(h.st().project?.root).toBe('/p');
  });

  test('the guard, a store update and a late save during a slow load do not throw', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdSaves = hold.promise;
    const saving = h.session.save();
    h.fake.holdSaves = null;
    h.fake.confirm.push('discard');
    const reads = deferred();
    h.fake.holdReads = reads.promise;
    const opening = h.session.openRecent('/q');
    await flush();
    expect(await h.session.guardUnsaved()).toBe(true);
    h.fake.failPick = true;
    await h.session.newProject(async () => null); // the picker fails and emits
    expect(h.st()).toMatchObject({ project: { root: '/q' }, songs: [], active: null });
    hold.resolve();
    expect(await saving).toBe(false);
    h.fake.holdReads = null;
    reads.resolve();
    await opening;
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js']);
    expect(h.doc()).toBe(Q);
  });

  test('subscribes to changes once, before the first open', async () => {
    const h = setup();
    await h.session.init();
    await h.session.init();
    await h.session.openRecent('/q');
    const calls = h.fake.calls.filter((c) => c === 'onChange' || c.startsWith('last'));
    expect(calls).toEqual(['onChange', 'last', 'last']);
  });

  test('an event emitted while the songs are read is applied after the install', async () => {
    const h = await opened();
    const reads = deferred();
    h.fake.holdReads = reads.promise;
    const opening = h.session.openRecent('/q');
    await flush(); // project_open returned, its reads are pending
    h.fake.holdReads = null;
    h.fake.files().set('songs/r.js', B); // after the listing
    h.fake.emit({ added: ['songs/r.js'] });
    reads.resolve();
    await opening;
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js', 'songs/r.js']);
    expect(h.st().error).toBeNull();
  });

  test('an event emitted while project_open runs belongs to the new project', async () => {
    const h = await opened();
    const gate = deferred();
    const open = h.api.open;
    h.api.open = async (dir) => {
      const info = await open(dir);
      h.fake.files().set('songs/r.js', B);
      h.fake.emit({ added: ['songs/r.js'] }); // the new watcher, before the command returns
      h.fake.files().delete('songs/r.js');
      await gate.promise;
      h.fake.files().set('songs/r.js', B);
      return info;
    };
    const opening = h.session.openRecent('/q');
    await flush();
    gate.resolve();
    await opening;
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js', 'songs/r.js']);
  });

  test("an old watcher's event delivered while project_open runs is dropped by root", async () => {
    const h = await opened();
    const open = h.api.open;
    h.api.open = async (dir) => {
      const info = await open(dir);
      h.fake.files().set('songs/stray.js', B); // exists in /q, but the event is /p's
      h.fake.emit({ root: '/p', added: ['songs/stray.js'] });
      return info;
    };
    await h.session.openRecent('/q');
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js']);
    expect(h.fake.calls).not.toContain('read:songs/stray.js');
  });

  test('an event of the previous project queued behind an open is dropped', async () => {
    const h = await opened();
    const reads = deferred();
    h.fake.holdReads = reads.promise;
    h.fake.files().set(PA, A + '// one');
    h.fake.emit({ changed: [PA] }); // being applied: its read is held
    await flush();
    h.fake.holdReads = null;
    const opening = h.session.openRecent('/q'); // queued behind it (after the guard)
    await flush();
    expect(h.fake.calls).not.toContain('open:/q');
    h.fake.emit({ added: [PB] }); // still /p's, but runs after /q is open
    reads.resolve();
    await opening;
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js']);
    expect(h.st().error).toBeNull();
    expect(h.fake.calls.filter((c) => c === `read:${PB}`)).toHaveLength(1); // at the first open
  });

  test('a failing watcher subscription sets error but the project opens', async () => {
    const h = setup();
    h.fake.failOnChange = true;
    await h.session.init();
    expect(h.st().project?.root).toBe('/p');
    expect(h.st().error).toContain('watch failed');
  });

  test('a failed subscription is retried by the next init', async () => {
    const h = setup();
    h.fake.failOnChange = true;
    await h.session.init();
    h.fake.failOnChange = false;
    await h.session.init();
    expect(h.st().error).toBeNull();
    h.fake.files().set(PA, A + '// outside');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '// outside');
  });
});

// ---------------------------------------------------------------- switch

describe('switch', () => {
  test('undo history survives a round trip', async () => {
    const h = await opened();
    h.type('// edit');
    await h.session.switchSong(PB);
    expect(h.doc()).toBe(B);
    await h.session.switchSong(PA);
    expect(h.doc()).toBe(A + '// edit');
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
  });

  test('highlights do not come back with a stashed state', async () => {
    const h = await opened();
    const at = A.indexOf('"a"') + 1;
    updateMiniLocations(h.view!, [[at, at + 1]]);
    highlightMiniLocations(h.view!, 0, [located(at, at + 1)]);
    expect(highlighted(h.view!)).toEqual([[at, at + 1]]);
    await h.session.switchSong(PB);
    await h.session.switchSong(PA);
    expect(highlighted(h.view!)).toEqual([]);
    highlightMiniLocations(h.view!, 0.5, [located(at, at + 1)]);
    expect(highlighted(h.view!)).toEqual([]);
  });

  test('the first visit builds the state from the disk text, later visits reuse it', async () => {
    const h = await opened();
    expect(h.created).toEqual([A]);
    await h.session.switchSong(PB);
    expect(h.created).toEqual([A, B]);
    await h.session.switchSong(PA);
    await h.session.switchSong(PB);
    expect(h.created).toEqual([A, B]);
    // no re-read on switch: the text from open is used
    expect(h.fake.calls.filter((c) => c === `read:${PB}`)).toHaveLength(1);
  });

  test('switching does not stop or evaluate', async () => {
    const h = await opened();
    h.rt.setStarted(true);
    await h.session.switchSong(PB);
    expect(h.rt.stops()).toBe(1);
    expect(h.rt.evaluated).toEqual([]);
  });

  test('switching to an unknown path or the active song is a no-op', async () => {
    const h = await opened();
    const before = h.st();
    const state = h.view!.state;
    h.fake.calls.length = 0;
    await h.session.switchSong('songs/nope.js');
    await h.session.switchSong(PA);
    expect(h.st()).toBe(before);
    expect(h.view!.state).toBe(state);
    expect(h.fake.calls).toEqual([]);
  });

  test('metaError and active follow the switch', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    expect(h.st().active).toBe(PB);
    expect(h.st().metaError).toBeNull();
  });
});

// ---------------------------------------------------------------- prepareState / onShown

describe('prepareState and onShown', () => {
  /** A session whose keymap follows `pref.vim`, as the app wires it. */
  async function keyed(project = '/p') {
    const pref = { vim: false };
    const shown: { doc: string; vim: boolean }[] = [];
    const prepared: string[] = [];
    const h = await opened({
      last: project,
      vim: () => pref.vim,
      prepareState: (s) => (prepared.push(s.doc.toString()), withKeymap(s, pref.vim)),
      onShown: (v) =>
        void shown.push({
          doc: v.state.doc.toString(),
          vim: keymapCompartment.get(v.state) === keymapFor(true),
        }),
    });
    const vimOn = () => keymapCompartment.get(h.view!.state) === keymapFor(true);
    return { ...h, pref, shown, prepared, vimOn };
  }

  test('a stashed state shown after a toggle has the new keymap', async () => {
    const h = await keyed();
    await h.session.switchSong(PB);
    expect(h.vimOn()).toBe(false);
    h.pref.vim = true;
    expect(setKeymap(h.view!, true)).toBe(true);
    await h.session.switchSong(PA);
    expect(h.doc()).toBe(A);
    expect(h.vimOn()).toBe(true);
    h.pref.vim = false;
    setKeymap(h.view!, false);
    await h.session.switchSong(PB);
    expect(h.vimOn()).toBe(false);
  });

  test('a song first shown after a toggle starts with the new keymap', async () => {
    const h = await keyed();
    h.pref.vim = true;
    setKeymap(h.view!, true);
    await h.session.switchSong(PC);
    expect(h.vimOn()).toBe(true);
  });

  test('the stashed state keeps its edits and undo history through prepareState', async () => {
    const h = await keyed();
    h.type('// one');
    await h.session.switchSong(PB);
    h.pref.vim = true;
    await h.session.switchSong(PA);
    expect(h.vimOn()).toBe(true);
    expect(h.doc()).toBe(A + '// one');
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
  });

  test('onShown runs after the state is attached, with the prepared state', async () => {
    const h = await keyed();
    expect(h.shown).toEqual([{ doc: A, vim: false }]);
    h.pref.vim = true;
    await h.session.switchSong(PB);
    expect(h.shown.at(-1)).toEqual({ doc: B, vim: true });
    expect(h.prepared.at(-1)).toBe(B);
  });

  test('onShown and prepareState are not called on the early return', async () => {
    const h = await keyed();
    const shownBefore = h.shown.length;
    const preparedBefore = h.prepared.length;
    await h.session.switchSong(PA); // already shown
    h.session.attachView(h.view); // same view
    expect(h.shown.length).toBe(shownBefore);
    expect(h.prepared.length).toBe(preparedBefore);
  });

  test('the blank state goes through prepareState and onShown once', async () => {
    const h = await keyed('/empty');
    expect(h.st().active).toBeNull();
    expect(h.shown).toEqual([{ doc: '', vim: false }]);
    const shownBefore = h.shown.length;
    h.pref.vim = true;
    // Already blank: nothing is attached again, so neither runs.
    await h.session.switchSong('songs/nope.js');
    expect(h.shown.length).toBe(shownBefore);
  });

  test('a newly attached view gets the prepared state and onShown', async () => {
    const h = await keyed();
    h.session.attachView(undefined);
    h.pref.vim = true;
    const view = new EditorView({ parent: document.body });
    h.session.attachView(view);
    expect(view.state.doc.toString()).toBe(A);
    expect(keymapCompartment.get(view.state)).toBe(keymapFor(true));
    expect(h.shown.at(-1)).toEqual({ doc: A, vim: true });
    view.destroy();
  });

  test('without the deps, shown states are attached as they are', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    expect(keymapCompartment.get(h.view!.state)).toBe(keymapFor(false));
  });
});

// ---------------------------------------------------------------- dirty

describe('dirty', () => {
  test('set by an edit, cleared by typing back to the disk text', async () => {
    const h = await opened();
    h.type('!!');
    expect(h.item(PA)?.dirty).toBe(true);
    h.backspace(2);
    expect(h.doc()).toBe(A);
    expect(h.item(PA)?.dirty).toBe(false);
  });

  test('tracked per song, including inactive ones', async () => {
    const h = await opened();
    h.type('x');
    await h.session.switchSong(PB);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.item(PB)?.dirty).toBe(false);
    h.type('y');
    expect(h.item(PB)?.dirty).toBe(true);
    expect(h.item(PC)?.dirty).toBe(false);
  });
});

// ---------------------------------------------------------------- save

describe('save', () => {
  test('writes the doc and clears dirty', async () => {
    const h = await opened();
    h.type('// saved');
    expect(await h.session.save()).toBe(true);
    expect(h.fake.files().get(PA)).toBe(A + '// saved');
    expect(h.item(PA)?.dirty).toBe(false);
    h.type('!');
    expect(h.item(PA)?.dirty).toBe(true); // compared against the new disk text
  });

  test('a failed save returns false and sets error', async () => {
    const h = await opened();
    h.type('x');
    h.fake.failSave.add(PA);
    expect(await h.session.save()).toBe(false);
    expect(h.st().error).toContain('disk full');
    expect(h.item(PA)?.dirty).toBe(true);
    h.fake.failSave.delete(PA);
    expect(await h.session.save()).toBe(true);
    expect(h.st().error).toBeNull();
  });

  test('saving an unchanged song still writes', async () => {
    const h = await opened();
    h.fake.calls.length = 0;
    expect(await h.session.save()).toBe(true);
    expect(h.fake.calls).toEqual([`save:${PA}`]);
  });

  test('every song and info command passes the open project root', async () => {
    const h = await opened();
    const roots: string[] = [];
    const read = h.api.readSong;
    h.api.readSong = (root, path) => (roots.push(root), read(root, path));
    const save = h.api.saveSong;
    h.api.saveSong = (root, path, text) => (roots.push(root), save(root, path, text));
    await h.session.openRecent('/q');
    await h.session.save();
    expect(roots).toEqual(['/q', '/q']);
  });

  test('a save resolving after a project switch does not settle into the new project', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdSaves = hold.promise;
    const saving = h.session.save();
    h.fake.holdSaves = null;
    h.fake.confirm.push('discard'); // still dirty: the save has not resolved
    await h.session.openRecent('/one'); // also has songs/a.js
    hold.resolve();
    expect(await saving).toBe(false);
    expect(h.fake.disk.get('/p')!.files.get(PA)).toBe(A + '// mine');
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.doc()).toBe(A);
    h.type('!');
    expect(h.item(PA)?.dirty).toBe(true); // still compared against /one's text
    h.backspace(1);
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.st().error).toBeNull();
  });

  test('a stale save rejected with "the project changed" is silent', async () => {
    const h = await opened();
    h.type('// mine');
    const gate = deferred();
    const save = h.api.saveSong;
    h.api.saveSong = async (root, path, text) => {
      await gate.promise;
      return save(root, path, text);
    };
    const saving = h.session.save();
    h.fake.confirm.push('discard');
    await h.session.openRecent('/one');
    gate.resolve();
    expect(await saving).toBe(false);
    expect(h.fake.disk.get('/one')!.files.get(PA)).toBe(A);
    expect(h.st().error).toBeNull();
    expect(h.item(PA)?.dirty).toBe(false);
  });

  test('a song read resolving after a project switch is dropped', async () => {
    const h = setup();
    h.fake.failRead.add(PB);
    await h.session.init();
    h.fake.failRead.delete(PB);
    const gate = deferred();
    h.fake.holdReads = gate.promise;
    const switching = h.session.switchSong(PB); // its open read failed: read again now
    expect(h.fake.calls.filter((c) => c === `read:${PB}`)).toHaveLength(2);
    h.fake.holdReads = null;
    await h.session.openRecent('/one');
    h.fake.disk.get('/one')!.files.set(PB, 'other');
    gate.resolve();
    await switching;
    expect(h.st().active).toBe(PA);
    expect(h.st().songs.map((s) => s.path)).toEqual([PA]);
    expect(h.st().error).toBeNull();
  });

  test('no-op without an active song', async () => {
    const h = await opened({ last: '/empty' });
    h.fake.calls.length = 0;
    expect(await h.session.save()).toBe(false);
    expect(h.fake.calls).toEqual([]);
    expect(h.st().error).toBeNull();
  });
});

describe('format on save', () => {
  const steps = Array(31).fill('c4').concat('d4').join(' ');
  const source = `$: note("${steps}").s('sine')\n`;

  function replaceSong(h: Awaited<ReturnType<typeof opened>>, text: string) {
    const view = h.view!;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
  }

  test('the default writes eligible source byte for byte', async () => {
    const h = await opened();
    replaceSong(h, source);
    expect(await h.session.save()).toBe(true);
    expect(h.fake.files().get(PA)).toBe(source);
  });

  test('explicit save formats a clean song in one undo step and maps live locations', async () => {
    const h = setup({ formatOnSave: () => true });
    h.fake.disk.get('/p')!.files.set(PA, source);
    await h.session.init();
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([source]); // clean Play does not format
    const view = h.view!;
    const note = source.indexOf('d4');
    view.dispatch({ selection: { anchor: note, head: note + 2 } });
    updateMiniLocations(view, [[note, note + 2]]);
    h.rt.setStarted(true);

    expect(await h.session.save()).toBe(true);
    const formatted = h.doc();
    expect(formatted).toContain('note(`\n');
    expect(formatted).toEndWith("\n`)\n  .s('sine')\n");
    expect(h.fake.files().get(PA)).toBe(formatted);
    const newNote = formatted.indexOf('d4');
    expect(view.state.selection.main.from).toBe(newNote);
    expect(view.state.selection.main.to).toBe(newNote + 2);
    highlightMiniLocations(view, 0, [located(note, note + 2)]);
    expect(highlighted(view)).toContainEqual([newNote, newNote + 2]);
    expect(h.rt.evaluated).toEqual([source]);
    expect(h.item(PA)?.dirty).toBe(false);
    expect(await h.session.save()).toBe(true);
    expect(undoDepth(view.state)).toBe(1);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(source);
    expect(h.item(PA)?.dirty).toBe(true);
  });

  test('evaluates the formatted snapshot through an in-flight watcher echo', async () => {
    const h = await opened({ formatOnSave: () => true });
    replaceSong(h, source);
    const hold = deferred();
    h.fake.holdSaves = hold.promise;
    const saving = h.session.saveAndEvaluate();
    const formatted = h.doc();
    expect(formatted).toContain('note(`\n');
    expect(formatted).toEndWith("\n`)\n  .s('sine')\n");
    h.type('// typed later');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBeNull();
    hold.resolve();
    await saving;
    expect(h.fake.files().get(PA)).toBe(formatted);
    expect(h.rt.evaluated).toEqual([formatted]);
    expect(h.doc()).toBe(formatted + '// typed later');
    expect(h.item(PA)?.dirty).toBe(true);
  });

  test('control saves format stashed state and leave the visible song alone', async () => {
    const h = await opened({ formatOnSave: () => true });
    await h.session.switchSong(PB);
    replaceSong(h, source);
    await h.session.switchSong(PA);
    expect(await h.session.saveSongs({ path: PB })).toEqual({ ok: true, saved: [PB] });
    expect(h.doc()).toBe(A);
    await h.session.switchSong(PB);
    expect(h.doc()).toContain('note(`\n');
    expect(h.doc()).toEndWith("\n`)\n  .s('sine')\n");
    expect(h.doc()).toBe(h.fake.files().get(PB)!);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(source);
  });

  test.each([false, true])(
    'a save waiting for a failed rename keeps its snapshot (later edit: %s)',
    async (edited) => {
      let enabled = true;
      const h = await opened({ formatOnSave: () => enabled });
      replaceSong(h, source);
      const entered = deferred();
      const release = deferred();
      h.api.renameSong = async () => {
        entered.resolve();
        await release.promise;
        throw new Error('rename failed');
      };
      const renaming = rejection(h.session.renameSong(PA, 'rename-target'));
      await entered.promise;
      const saving = h.session.save();
      enabled = false;
      if (edited) h.type('// typed while waiting');
      release.resolve();

      expect(await renaming).toEqual(new Error('rename failed'));
      expect(await saving).toBe(true);
      if (edited) {
        expect(h.fake.files().get(PA)).toBe(source);
        expect(h.doc()).toBe(source + '// typed while waiting');
        expect(h.item(PA)?.dirty).toBe(true);
      } else {
        expect(h.doc()).toContain('note(`\n');
        expect(h.doc()).toBe(h.fake.files().get(PA)!);
        expect(h.item(PA)?.dirty).toBe(false);
      }
    },
  );

  test('a failed save leaves an undoable buffer and editing errors can still be saved', async () => {
    const h = await opened({ formatOnSave: () => true });
    replaceSong(h, source);
    h.fake.failSave.add(PA);
    expect(await h.session.save()).toBe(false);
    expect(h.doc()).toContain('note(`\n');
    expect(h.fake.files().get(PA)).toBe(A);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(source);
    const incomplete = source + 'note(';
    replaceSong(h, incomplete);
    h.fake.failSave.delete(PA);
    expect(await h.session.save()).toBe(true);
    expect(h.fake.files().get(PA)).toBe(incomplete);
  });
});

// ---------------------------------------------------------------- saveAndEvaluate

describe('saveAndEvaluate', () => {
  test('saves a dirty song, then evaluates the doc as it was at key press', async () => {
    const h = await opened();
    h.type('// a');
    const pressed = h.session.saveAndEvaluate();
    h.type('// typed after');
    await pressed;
    expect(h.fake.files().get(PA)).toBe(A + '// a');
    expect(h.rt.evaluated).toEqual([A + '// a']);
    expect(h.item(PA)?.dirty).toBe(true);
  });

  test('a clean song is evaluated without saving', async () => {
    const h = await opened();
    h.fake.calls.length = 0;
    await h.session.saveAndEvaluate();
    expect(h.fake.calls).toEqual([]);
    expect(h.rt.evaluated).toEqual([A]);
  });

  test('a failed save does not evaluate', async () => {
    const h = await opened();
    h.type('x');
    h.fake.failSave.add(PA);
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([]);
    expect(h.st().error).toContain('disk full');
  });

  test('zero songs does nothing', async () => {
    const h = await opened({ last: '/empty' });
    h.fake.calls.length = 0;
    await h.session.saveAndEvaluate();
    expect(h.fake.calls).toEqual([]);
    expect(h.rt.evaluated).toEqual([]);
  });
});

// ---------------------------------------------------------------- change events

describe('change events', () => {
  test('our own save echo is ignored', async () => {
    const h = await opened();
    h.type('x');
    await h.session.save();
    const depth = undoDepth(h.view!.state);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.fake.calls.filter((c) => c === `read:${PA}`)).toHaveLength(2);
    expect(h.doc()).toBe(A + 'x');
    expect(undoDepth(h.view!.state)).toBe(depth);
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });

  test('an echo that arrives before the save resolves is ignored', async () => {
    const h = await opened();
    h.type('x');
    const gate = deferred();
    h.fake.holdSaves = gate.promise;
    const saving = h.session.save();
    h.fake.emit({ changed: [PA] }); // the file already holds our text
    await flush();
    expect(h.item(PA)?.conflict).toBeNull();
    expect(h.doc()).toBe(A + 'x');
    gate.resolve();
    expect(await saving).toBe(true);
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });

  test('a clean active song is replaced as one undoable step and re-evaluated when started', async () => {
    const h = await opened();
    h.rt.setStarted(true);
    const next = A.replace('"a"', '"aa"');
    h.fake.files().set(PA, next);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(next);
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
    expect(h.rt.evaluated).toEqual([next]);
    expect(undoDepth(h.view!.state)).toBe(1);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
  });

  test('a reload right after typing is its own undo step', async () => {
    const h = await opened();
    h.type('x');
    await h.session.save();
    h.fake.files().set(PA, A + 'xy');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + 'xy');
    h.undoView();
    expect(h.doc()).toBe(A + 'x');
  });

  test('a clean active song that is not playing is not evaluated', async () => {
    const h = await opened();
    h.fake.files().set(PA, A + '// more');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '// more');
    expect(h.rt.evaluated).toEqual([]);
  });

  test('a dirty active song gets a changed conflict and keeps its text', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, A + '// theirs');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '// mine');
    expect(h.item(PA)).toMatchObject({ dirty: true, conflict: 'changed' });
  });

  test('keepMine clears the conflict, stays dirty, and the next save overwrites disk', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, A + '// theirs');
    h.fake.emit({ changed: [PA] });
    await flush();
    h.session.keepMine();
    expect(h.item(PA)).toMatchObject({ dirty: true, conflict: null });
    // the disk text is now theirs: typing back to the old disk text is still dirty
    h.backspace('// mine'.length);
    expect(h.item(PA)?.dirty).toBe(true);
    h.type('// mine');
    await h.session.save();
    expect(h.fake.files().get(PA)).toBe(A + '// mine');
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });

  test('takeDisk replaces the doc undoably and clears dirty', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, A + '// theirs');
    h.fake.emit({ changed: [PA] });
    await flush();
    h.session.takeDisk();
    expect(h.doc()).toBe(A + '// theirs');
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
    h.undoView();
    expect(h.doc()).toBe(A + '// mine');
  });

  test('keepMine and takeDisk without a conflict do nothing', async () => {
    const h = await opened();
    h.type('x');
    const before = h.st();
    h.session.keepMine();
    h.session.takeDisk();
    expect(h.st()).toBe(before);
    expect(h.doc()).toBe(A + 'x');
  });

  test('a disk change equal to the dirty doc resolves without a conflict', async () => {
    const h = await opened();
    h.type('x');
    h.fake.files().set(PA, A + 'x');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });

  test('a conflict clears when the disk goes back to the saved text', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, A + '// theirs');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBe('changed');
    h.fake.files().set(PA, A); // e.g. git checkout
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)).toMatchObject({ dirty: true, conflict: null });
    expect(h.doc()).toBe(A + '// mine');
    // the stale pending text is gone: takeDisk does nothing, typing back to A is clean
    h.session.takeDisk();
    expect(h.doc()).toBe(A + '// mine');
    h.backspace('// mine'.length);
    expect(h.item(PA)?.dirty).toBe(false);
  });

  test('a second change while in conflict updates the stashed disk text', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, A + '// one');
    h.fake.emit({ changed: [PA] });
    await flush();
    h.fake.files().set(PA, A + '// two');
    h.fake.emit({ changed: [PA] });
    await flush();
    h.session.takeDisk();
    expect(h.doc()).toBe(A + '// two');
  });

  test('an inactive clean song is updated with its undo history intact', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// typed');
    await h.session.save();
    await h.session.switchSong(PA);
    h.rt.setStarted(true);
    h.fake.files().set(PB, B + '// typed// reloaded');
    h.fake.emit({ changed: [PB] });
    await flush();
    expect(h.rt.evaluated).toEqual([]); // not the active song
    expect(h.doc()).toBe(A);
    expect(h.item(PB)).toMatchObject({ dirty: false, conflict: null });
    await h.session.switchSong(PB);
    expect(h.doc()).toBe(B + '// typed// reloaded');
    h.undoView(); // one step back: before the reload
    expect(h.doc()).toBe(B + '// typed');
    h.undoView();
    expect(h.doc()).toBe(B);
  });

  test('an inactive song never shown picks up the new text and title', async () => {
    const h = await opened();
    const next = `const meta = { title: 'Bee' };\n` + B;
    h.fake.files().set(PB, next);
    h.fake.emit({ changed: [PB] });
    await flush();
    expect(h.item(PB)?.title).toBe('Bee');
    await h.session.switchSong(PB);
    expect(h.doc()).toBe(next);
    expect(undoDepth(h.view!.state)).toBe(0);
  });

  test('an inactive dirty song gets a conflict; takeDisk applies to its stashed state', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// mine');
    await h.session.switchSong(PA);
    h.fake.files().set(PB, B + '// theirs');
    h.fake.emit({ changed: [PB] });
    await flush();
    expect(h.item(PB)).toMatchObject({ dirty: true, conflict: 'changed' });
    expect(h.doc()).toBe(A);
    h.session.takeDisk(PB);
    expect(h.item(PB)).toMatchObject({ dirty: false, conflict: null });
    expect(h.doc()).toBe(A); // the view still shows the active song
    await h.session.switchSong(PB);
    expect(h.doc()).toBe(B + '// theirs');
    h.undoView();
    expect(h.doc()).toBe(B + '// mine');
  });

  test('keepMine on an inactive song keeps its stashed text dirty', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// mine');
    await h.session.switchSong(PA);
    h.fake.files().set(PB, B + '// theirs');
    h.fake.emit({ changed: [PB] });
    await flush();
    h.session.keepMine(PB);
    expect(h.item(PB)).toMatchObject({ dirty: true, conflict: null });
  });

  test('a read failure during reload sets error and keeps the doc', async () => {
    const h = await opened();
    h.fake.failRead.add(PA);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.st().error).toContain('permission denied');
    expect(h.doc()).toBe(A);
    expect(h.item(PA)?.conflict).toBeNull();
  });

  test('rapid double events converge on the final disk text', async () => {
    const h = await opened();
    h.fake.files().set(PA, A + '1');
    h.fake.emit({ changed: [PA] });
    h.fake.files().set(PA, A + '12');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '12');
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });

  test('interleaved events (one read per state) converge too', async () => {
    const h = await opened();
    h.fake.files().set(PA, A + '1');
    h.fake.emit({ changed: [PA] });
    await flush();
    h.fake.files().set(PA, A + '12');
    h.fake.emit({ changed: [PA] });
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '12');
    expect(undoDepth(h.view!.state)).toBe(2);
    expect(h.item(PA)).toMatchObject({ dirty: false, conflict: null });
  });
});

// ---------------------------------------------------------------- line breaks

describe('CRLF songs', () => {
  const crlf = (text: string) => text.replace(/\n/g, '\r\n');

  test('toLF turns CRLF and lone CR into LF', () => {
    expect(toLF('a\r\nb\rc\nd\r\n')).toBe('a\nb\nc\nd\n');
    expect(toLF('plain')).toBe('plain');
  });

  async function openedCrlf() {
    const h = setup({ last: '/crlf' });
    h.fake.disk.set('/crlf', {
      name: 'Crlf',
      files: new Map([
        [PA, crlf(A)],
        [PB, crlf(B)],
      ]),
    });
    await h.session.init();
    await flush();
    return h;
  }

  test('a CRLF song opens clean, also when shown later', async () => {
    const h = await openedCrlf();
    expect(h.doc()).toBe(A);
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.item(PA)?.title).toBe('Alpha');
    await h.session.switchSong(PB);
    expect(h.item(PB)?.dirty).toBe(false);
  });

  test('an outside CRLF edit to a clean song reloads instead of conflicting', async () => {
    const h = await openedCrlf();
    h.fake.files().set(PA, crlf(A + '// outside\n'));
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBeNull();
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.doc()).toBe(A + '// outside\n');
  });

  test('a CRLF touch with the same text is a no-op; saves write LF', async () => {
    const h = await openedCrlf();
    h.type('// mine');
    h.fake.emit({ changed: [PA] }); // disk still holds crlf(A)
    await flush();
    expect(h.item(PA)?.conflict).toBeNull();
    await h.session.save();
    expect(h.fake.files().get(PA)).toBe(A + '// mine');
  });
});

// ---------------------------------------------------------------- added / removed

describe('added and removed', () => {
  test('an added song appears at its sorted position with its title', async () => {
    const h = await opened();
    h.fake.files().set('songs/ab.js', `const meta = { title: 'Between' };\n`);
    h.fake.emit({ added: ['songs/ab.js'] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, 'songs/ab.js', PB, PC]);
    expect(h.item('songs/ab.js')).toMatchObject({ title: 'Between', dirty: false, conflict: null });
    expect(h.st().active).toBe(PA);
    await h.session.switchSong('songs/ab.js');
    expect(h.doc()).toBe(`const meta = { title: 'Between' };\n`);
  });

  test('an added song that cannot be read is listed by file name with an error', async () => {
    const h = await opened();
    h.fake.files().set('songs/z.js', 'x');
    h.fake.failRead.add('songs/z.js');
    h.fake.emit({ added: ['songs/z.js'] });
    await flush();
    expect(h.item('songs/z.js')?.title).toBe('z');
    expect(h.st().error).toContain('permission denied');
  });

  test('a changed event for an unknown path adds it', async () => {
    const h = await opened();
    h.fake.files().set('songs/d.js', '$: s("d")\n');
    h.fake.emit({ changed: ['songs/d.js'] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC, 'songs/d.js']);
  });

  test('a clean inactive song that is removed is dropped', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    await h.session.switchSong(PA);
    h.fake.files().delete(PB);
    h.fake.emit({ removed: [PB] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PC]);
  });

  test('the active song, removed, becomes a deleted conflict', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(h.item(PA)).toMatchObject({ conflict: 'deleted', dirty: false });
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
  });

  test('a dirty inactive song, removed, becomes a deleted conflict', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('x');
    await h.session.switchSong(PA);
    h.fake.files().delete(PB);
    h.fake.emit({ removed: [PB] });
    await flush();
    expect(h.item(PB)).toMatchObject({ conflict: 'deleted', dirty: true });
  });

  test('recreateDeleted writes the doc and clears the conflict', async () => {
    const h = await opened();
    h.type('// keep');
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    await h.session.recreateDeleted();
    expect(h.fake.files().get(PA)).toBe(A + '// keep');
    expect(h.item(PA)).toMatchObject({ conflict: null, dirty: false });
  });

  test('a failed recreate keeps the conflict and sets error', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    h.fake.failSave.add(PA);
    await h.session.recreateDeleted();
    expect(h.item(PA)?.conflict).toBe('deleted');
    expect(h.st().error).toContain('disk full');
  });

  test('closeDeleted drops the song and switches to the first remaining one', async () => {
    const h = await opened();
    h.type('x');
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    await h.session.closeDeleted();
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC]);
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe(B);
    expect(await h.session.guardUnsaved()).toBe(true); // the discarded edit no longer counts
  });

  test('closeDeleted without a deleted conflict does nothing', async () => {
    const h = await opened();
    await h.session.closeDeleted();
    await h.session.recreateDeleted();
    expect(h.st().songs).toHaveLength(3);
    expect(h.fake.calls.some((c) => c.startsWith('save:'))).toBe(false);
  });

  test('removing the last song, then closing it, gives the zero-songs state', async () => {
    const h = await opened({ last: '/one' });
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBe('deleted');
    await h.session.closeDeleted();
    expect(h.st().songs).toEqual([]);
    expect(h.st().active).toBeNull();
    expect(h.doc()).toBe('');
    expect(h.title()).toBe('One');
  });

  test('a deleted song recreated on disk clears the conflict', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    h.fake.files().set(PA, A);
    h.fake.emit({ added: [PA] });
    await flush();
    expect(h.item(PA)).toMatchObject({ conflict: null, dirty: false });
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC]);
  });

  test('the list order comes from song_list, not a local sort', async () => {
    const h = await opened();
    // Rust sorts byte-wise; serve a different order to show the session takes it as given.
    h.api.listSongs = async () => [PC, 'songs/d.js', PB, PA].map((path) => ({ path }));
    h.fake.files().set('songs/d.js', B);
    h.fake.emit({ added: ['songs/d.js'] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PC, 'songs/d.js', PB, PA]);
  });

  test('song_list is asked only when a song was added', async () => {
    const h = await opened();
    h.fake.files().set(PB, B + '// x');
    h.fake.emit({ changed: [PB] });
    await flush();
    expect(h.fake.calls).not.toContain('listSongs');
    h.fake.files().set('songs/d.js', B);
    h.fake.emit({ added: ['songs/d.js'] });
    await flush();
    expect(h.fake.calls.filter((c) => c === 'listSongs')).toHaveLength(1);
  });

  test('a failing song_list keeps the song at the end and sets error', async () => {
    const h = await opened();
    h.api.listSongs = async () => {
      throw 'listing failed';
    };
    h.fake.files().set('songs/ab.js', B);
    h.fake.emit({ added: ['songs/ab.js'] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC, 'songs/ab.js']);
    expect(h.st().error).toContain('listing failed');
  });

  test('a deleted conflict keeps its place when another song is added', async () => {
    const h = await opened();
    h.type('x');
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    h.fake.files().set('songs/ab.js', B);
    h.fake.emit({ added: ['songs/ab.js'] });
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, 'songs/ab.js', PB, PC]);
    expect(h.item(PA)?.conflict).toBe('deleted');
  });

  test('the songs of one event are read in parallel, then applied in order', async () => {
    const h = await opened();
    const gate = deferred();
    const read = h.api.readSong;
    let pending = 0;
    let maxPending = 0;
    h.api.readSong = async (root, path) => {
      pending++;
      maxPending = Math.max(maxPending, pending);
      await gate.promise;
      pending--;
      return read(root, path);
    };
    h.fake.files().set(PB, B + '// b');
    h.fake.files().set('songs/d.js', B);
    h.fake.emit({ changed: [PB], added: ['songs/d.js'] });
    await flush();
    expect(maxPending).toBe(2);
    gate.resolve();
    await flush();
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC, 'songs/d.js']);
    await h.session.switchSong(PB);
    expect(h.doc()).toBe(B + '// b');
  });

  test('removed for an unknown path is ignored', async () => {
    const h = await opened();
    h.fake.emit({ removed: ['songs/zz.js'] });
    await flush();
    expect(h.st().songs).toHaveLength(3);
  });
});

// ---------------------------------------------------------------- manifest

describe('manifest', () => {
  test('a manifest change re-reads project_info and updates the name', async () => {
    const h = await opened();
    h.fake.disk.get('/p')!.name = 'Renamed';
    h.fake.calls.length = 0;
    h.fake.emit({ manifest: true });
    await flush();
    // project_info only: no reopen, no new watcher, no recents update
    expect(h.fake.calls).toEqual(['info']);
    expect(h.st().project?.name).toBe('Renamed');
    expect(h.title()).toBe('Renamed — Alpha');
  });

  test('a failing project_info sets error and keeps the state', async () => {
    const h = await opened();
    h.type('x');
    h.fake.failInfo = true;
    h.fake.emit({ manifest: true });
    await flush();
    expect(h.st().error).toContain('reloading project.json failed: project.json: expected value');
    expect(h.st().project?.name).toBe('Proj');
    expect(h.doc()).toBe(A + 'x');
    expect(h.st().songs).toHaveLength(3);
  });
});

// ---------------------------------------------------------------- createSong

describe('createSong', () => {
  test('creates, lists in order and switches to it', async () => {
    const h = await opened();
    await h.session.createSong('b2');
    expect(h.fake.calls).toContain('createSong:b2');
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, 'songs/b2.js', PC]);
    expect(h.st().active).toBe('songs/b2.js');
    expect(h.item('songs/b2.js')?.title).toBe('b2');
    expect(h.doc()).toContain("title: 'b2'");
  });

  test('the watcher echo of a created song does not duplicate it', async () => {
    const h = await opened();
    await h.session.createSong('n');
    h.fake.emit({ added: ['songs/n.js'] });
    await flush();
    expect(h.st().songs.filter((s) => s.path === 'songs/n.js')).toHaveLength(1);
    expect(h.item('songs/n.js')?.conflict).toBeNull();
  });

  // Decision: an API error rejects with an Error carrying the Rust message, for the song list's
  // inline field to show. The session `error` is not set, so it does not show twice.
  test('an invalid name rejects with the API message and leaves state alone', async () => {
    const h = await opened();
    const before = h.st();
    await expect(h.session.createSong('a/b')).rejects.toThrow('invalid song name "a/b"');
    await expect(h.session.createSong('a')).rejects.toThrow('song songs/a.js already exists');
    expect(h.st()).toBe(before);
    expect(h.st().error).toBeNull();
  });

  test('without a project it rejects', async () => {
    const h = await opened({ last: null });
    await expect(h.session.createSong('x')).rejects.toThrow('no project is open');
    expect(h.fake.calls).not.toContain('createSong:x');
  });

  test('from zero songs, the new song becomes active', async () => {
    const h = await opened({ last: '/empty' });
    await h.session.createSong('first');
    expect(h.st().active).toBe('songs/first.js');
    expect(h.doc()).toContain("title: 'first'");
    expect(h.title()).toBe('Empty — first');
  });

  test('resolves to the created path', async () => {
    const h = await opened();
    expect(await h.session.createSong('b2')).toBe('songs/b2.js');
  });

  test('resolves null when the project changed mid-call', async () => {
    const h = await opened();
    const gate = deferred();
    const create = h.api.createSong;
    h.api.createSong = async (root, stem) => {
      await gate.promise;
      return create(root, stem);
    };
    const creating = h.session.createSong('n');
    await h.session.openRecent('/q');
    gate.resolve();
    expect(await creating).toBeNull();
    expect(h.st().project?.root).toBe('/q');
    expect(h.st().songs.map((s) => s.path)).toEqual(['songs/q.js']);
    expect(h.st().error).toBeNull();
  });

  test('a library error rejects with the ProjectError and its kind', async () => {
    const h = await opened();
    expect(await rejection(h.session.createSong('a'))).toMatchObject({ kind: 'songExists' });
  });
});

// ---------------------------------------------------------------- renameSong

describe('renameSong', () => {
  const PZ = 'songs/z.js';

  test('moves the song with its dirty buffer, undo history and Claude mark', async () => {
    const h = await opened();
    h.type('// mine\n');
    h.session.externalEdit(PA, A + '// mine\n// claude\n');
    const depth = undoDepth(h.view!.state);
    expect(await h.session.renameSong(PA, 'z')).toEqual({ ok: true, from: PA, to: PZ });
    expect(h.fake.files().has(PA)).toBe(false);
    expect(h.fake.files().get(PZ)).toBe(A);
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC, PZ]);
    expect(h.st().active).toBe(PZ);
    expect(h.item(PZ)).toMatchObject({ title: 'Alpha', dirty: true, conflict: null });
    expect(h.doc()).toBe(A + '// mine\n// claude\n');
    expect(undoDepth(h.view!.state)).toBe(depth);
    expect(hasClaudeMark(h.view!.state)).toBe(true);
    expect(h.st().notice).toMatchObject({ path: PZ });
    h.undoView();
    expect(h.doc()).toBe(A + '// mine\n');
    expect(await h.session.save()).toBe(true);
    expect(h.fake.files().get(PZ)).toBe(A + '// mine\n');
    expect(h.fake.files().has(PA)).toBe(false);
  });

  test('a song without meta.title takes the new file name as its title', async () => {
    const h = await opened();
    expect(await h.session.renameSong(PB, 'b2')).toEqual({ ok: true, from: PB, to: 'songs/b2.js' });
    expect(h.item('songs/b2.js')).toMatchObject({ title: 'b2', dirty: false });
    expect(h.st().active).toBe(PA); // not switched to
    expect(h.doc()).toBe(A);
  });

  test('the evaluated song follows, with its title recomputed', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    await h.session.saveAndEvaluate();
    expect(h.st().evaluated).toEqual({ path: PB, title: 'b' });
    await h.session.renameSong(PB, 'n');
    expect(h.st().evaluated).toEqual({ path: 'songs/n.js', title: 'n' });
    expect(h.st().active).toBe('songs/n.js');
    await h.session.switchSong(PA);
    await h.session.saveAndEvaluate();
    await h.session.renameSong(PA, 'x');
    expect(h.st().evaluated).toEqual({ path: 'songs/x.js', title: 'Alpha' });
  });

  test('a changed conflict and its disk text move with the song', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, 'other');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBe('changed');
    await h.session.renameSong(PA, 'z');
    h.fake.emit({ removed: [PA], added: [PZ] });
    await flush();
    expect(h.item(PZ)).toMatchObject({ conflict: 'changed', dirty: true });
    h.session.takeDisk();
    expect(h.doc()).toBe('other');
    expect(h.item(PZ)).toMatchObject({ conflict: null, dirty: false });
  });

  for (const when of ['before', 'after'] as const) {
    test(`the watcher echo ${when} the API resolves leaves one clean entry`, async () => {
      const h = await opened();
      h.type('// mine'); // a removed echo would make a dirty song a deleted conflict
      const hold = deferred();
      h.fake.holdRenames = hold.promise;
      const renaming = h.session.renameSong(PA, 'z');
      await flush();
      if (when === 'before') {
        h.fake.emit({ removed: [PA], added: [PZ] });
        await flush(); // queued behind the rename, so it cannot apply yet
      }
      hold.resolve();
      expect(await renaming).toMatchObject({ ok: true, to: PZ });
      if (when === 'after') h.fake.emit({ removed: [PA], added: [PZ] });
      await flush();
      expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC, PZ]);
      expect(h.item(PZ)).toMatchObject({ conflict: null, dirty: true });
      expect(h.st().active).toBe(PZ);
      expect(h.doc()).toBe(A + '// mine');
    });
  }

  test('the list keeps the disk order', async () => {
    const h = await opened();
    await h.session.renameSong(PA, 'bb');
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, 'songs/bb.js', PC]);
  });

  test('a deleted conflict gives deleted and calls nothing', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(await h.session.renameSong(PA, 'z')).toEqual({ ok: false, reason: 'deleted' });
    expect(h.fake.calls.some((c) => c.startsWith('rename:'))).toBe(false);
  });

  test('an unknown song gives missing and calls nothing', async () => {
    const h = await opened();
    expect(await h.session.renameSong('songs/nope.js', 'z')).toEqual({
      ok: false,
      reason: 'missing',
    });
    expect(h.fake.calls.some((c) => c.startsWith('rename:'))).toBe(false);
  });

  test('without a project it gives missing', async () => {
    const h = await opened({ last: null });
    expect(await h.session.renameSong(PA, 'z')).toEqual({ ok: false, reason: 'missing' });
  });

  test('library errors reject with their kind and leave the state alone', async () => {
    const h = await opened();
    const before = h.st();
    expect(await rejection(h.session.renameSong(PA, 'b'))).toMatchObject({
      kind: 'songExists',
      message: 'song songs/b.js already exists',
    });
    expect(await rejection(h.session.renameSong(PA, 'a/b'))).toMatchObject({
      kind: 'invalidName',
    });
    h.fake.files().delete(PC); // gone on disk, no event yet
    expect(await rejection(h.session.renameSong(PC, 'cc'))).toMatchObject({
      kind: 'songNotFound',
    });
    expect(h.st()).toBe(before);
    expect(h.st().error).toBeNull();
  });

  test('a name held by a song kept only in the editor is songExists, without an API call', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] }); // active: kept as a deleted conflict
    await flush();
    expect(await rejection(h.session.renameSong(PB, 'a'))).toMatchObject({ kind: 'songExists' });
    expect(h.fake.calls.some((c) => c.startsWith('rename:'))).toBe(false);
    expect(h.item(PA)?.conflict).toBe('deleted');
  });

  test('a project-changed rejection gives projectChanged and touches nothing', async () => {
    const h = await opened();
    h.api.renameSong = async () => {
      throw new ProjectError('the project changed', 'projectChanged');
    };
    expect(await h.session.renameSong(PA, 'z')).toEqual({ ok: false, reason: 'projectChanged' });
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC]);
    expect(h.st().active).toBe(PA);
  });

  test('Cmd-S during a held rename writes nothing and the buffer stays dirty under the new path', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdRenames = hold.promise;
    const renaming = h.session.renameSong(PA, 'z');
    await flush();
    expect(h.fake.calls).toContain(`rename:${PA}:z`);
    const saving = h.session.save();
    await flush();
    hold.resolve();
    expect(await saving).toBe(false);
    await renaming;
    expect(h.fake.calls.some((c) => c.startsWith('save:'))).toBe(false);
    expect(h.fake.files().has(PA)).toBe(false);
    expect(h.fake.files().get(PZ)).toBe(A);
    expect(h.item(PZ)?.dirty).toBe(true);
    expect(h.st().error).toBeNull();
  });

  test('a rename during an in-flight save waits for it', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdSaves = hold.promise;
    const saving = h.session.save();
    h.fake.holdSaves = null;
    const renaming = h.session.renameSong(PA, 'z');
    await flush();
    expect(h.fake.calls.some((c) => c.startsWith('rename:'))).toBe(false);
    hold.resolve();
    expect(await saving).toBe(true);
    expect(await renaming).toMatchObject({ ok: true, to: PZ });
    expect(h.fake.files().get(PZ)).toBe(A + '// mine');
    expect(h.fake.files().has(PA)).toBe(false);
    expect(h.item(PZ)?.dirty).toBe(false);
  });
});

// ---------------------------------------------------------------- deleteSong

describe('deleteSong', () => {
  const deletes = (h: { fake: { calls: string[] } }) =>
    h.fake.calls.filter((c) => c.startsWith('delete:'));

  test('a clean delete drops the active song and activates the first loadable one', async () => {
    const h = await opened();
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: true,
      stopped: false,
    });
    expect(h.fake.files().has(PA)).toBe(false);
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC]);
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe(B);
  });

  test('deleting an inactive song keeps the active one', async () => {
    const h = await opened();
    await h.session.deleteSong(PB, { discard: false });
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PC]);
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
  });

  test('a dirty song without discard is unsaved and calls nothing', async () => {
    const h = await opened();
    h.type('x');
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: false,
      reason: 'unsaved',
    });
    expect(deletes(h)).toEqual([]);
    expect(h.item(PA)?.dirty).toBe(true);
  });

  test('a dirty song with discard is deleted', async () => {
    const h = await opened();
    h.type('x');
    expect(await h.session.deleteSong(PA, { discard: true })).toMatchObject({ ok: true });
    expect(deletes(h)).toEqual([`delete:${PA}`]);
    expect(h.item(PA)).toBeUndefined();
    expect(await h.session.guardUnsaved()).toBe(true);
  });

  test('a deleted conflict needs discard, then drops it without an API call', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: false,
      reason: 'unsaved',
    });
    expect(await h.session.deleteSong(PA, { discard: true })).toEqual({ ok: true, stopped: false });
    expect(deletes(h)).toEqual([]);
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC]);
    expect(h.st().active).toBe(PB);
  });

  test('deleting the playing evaluated song stops it and clears evaluated', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    h.rt.setStarted(true);
    const stops = h.rt.stops();
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({ ok: true, stopped: true });
    expect(h.rt.stops()).toBe(stops + 1);
    expect(h.st().evaluated).toBeNull();
  });

  test('deleting the evaluated song while stopped reports stopped false', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: true,
      stopped: false,
    });
    expect(h.st().evaluated).toBeNull();
  });

  test('deleting another song leaves playback alone', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    h.rt.setStarted(true);
    const stops = h.rt.stops();
    expect(await h.session.deleteSong(PB, { discard: false })).toEqual({
      ok: true,
      stopped: false,
    });
    expect(h.rt.stops()).toBe(stops);
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
  });

  test('deleting the last song leaves no active song', async () => {
    const h = await opened({ last: '/one' });
    expect(await h.session.deleteSong(PA, { discard: false })).toMatchObject({ ok: true });
    expect(h.st().songs).toEqual([]);
    expect(h.st().active).toBeNull();
    expect(h.doc()).toBe('');
  });

  for (const when of ['before', 'after'] as const) {
    test(`the watcher echo ${when} the API resolves is a no-op`, async () => {
      const h = await opened();
      const hold = deferred();
      h.fake.holdDeletes = hold.promise;
      const deleting = h.session.deleteSong(PA, { discard: false });
      await flush();
      if (when === 'before') {
        h.fake.emit({ removed: [PA] });
        await flush(); // queued behind the rename, so it cannot apply yet
      }
      hold.resolve();
      expect(await deleting).toMatchObject({ ok: true });
      if (when === 'after') h.fake.emit({ removed: [PA] });
      await flush();
      expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC]);
      expect(h.st().active).toBe(PB);
      expect(h.st().error).toBeNull();
    });
  }

  test('Cmd-S during a held delete returns false and writes nothing', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdDeletes = hold.promise;
    const deleting = h.session.deleteSong(PA, { discard: true });
    await flush();
    const saving = h.session.save();
    await flush();
    hold.resolve();
    expect(await saving).toBe(false);
    await deleting;
    expect(h.fake.calls.some((c) => c.startsWith('save:'))).toBe(false);
    expect(h.fake.files().has(PA)).toBe(false);
    expect(h.st().error).toBeNull();
  });

  test('an unknown song gives missing', async () => {
    const h = await opened();
    expect(await h.session.deleteSong('songs/nope.js', { discard: true })).toEqual({
      ok: false,
      reason: 'missing',
    });
    expect(deletes(h)).toEqual([]);
  });

  test('a library error rejects with its kind and keeps the song', async () => {
    const h = await opened();
    h.api.deleteSong = async () => {
      throw new ProjectError('permission denied', 'other');
    };
    expect(await rejection(h.session.deleteSong(PB, { discard: false }))).toMatchObject({
      kind: 'other',
    });
    expect(h.item(PB)).toBeDefined();
  });

  test('a file already gone on disk counts as deleted', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    h.fake.files().delete(PA); // gone on disk, no event yet
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: true,
      stopped: false,
    });
    expect(h.st().songs.map((s) => s.path)).toEqual([PB, PC]);
    expect(h.st().active).toBe(PB);
    expect(h.st().evaluated).toBeNull();
  });

  test('deleting the active song while stopped resets the transport for the next one', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.deleteSong(PA, { discard: false });
    expect(h.st().active).toBe(PB);
    expect(h.rt.resets()).toEqual([0]);
  });

  test('deleting the active song while another plays leaves the transport alone', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    await h.session.saveAndEvaluate(); // B plays
    h.rt.setStarted(true);
    await h.session.switchSong(PA);
    h.rt.resets().length = 0;
    await h.session.deleteSong(PA, { discard: false });
    expect(h.rt.resets()).toEqual([]);
  });

  test('deleting an inactive song leaves the transport alone', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.deleteSong(PB, { discard: false });
    expect(h.rt.resets()).toEqual([]);
  });

  test('a project-changed rejection gives projectChanged and keeps the song', async () => {
    const h = await opened();
    h.api.deleteSong = async () => {
      throw new ProjectError('the project changed', 'projectChanged');
    };
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: false,
      reason: 'projectChanged',
    });
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PB, PC]);
  });
});

// ---------------------------------------------------------------- saveSongs

describe('saveSongs', () => {
  const saves = (h: { fake: { calls: string[] } }) =>
    h.fake.calls.filter((c) => c.startsWith('save:'));

  test('a dirty song is written', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// b');
    await h.session.switchSong(PA);
    expect(await h.session.saveSongs({ path: PB })).toEqual({ ok: true, saved: [PB] });
    expect(h.fake.files().get(PB)).toBe(B + '// b');
    expect(h.item(PB)?.dirty).toBe(false);
  });

  test('a clean song writes nothing', async () => {
    const h = await opened();
    expect(await h.session.saveSongs({ path: PA })).toEqual({ ok: true, saved: [] });
    expect(saves(h)).toEqual([]);
  });

  test('a deleted conflict is recreated', async () => {
    const h = await opened();
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(await h.session.saveSongs({ path: PA })).toEqual({ ok: true, saved: [PA] });
    expect(h.fake.files().get(PA)).toBe(A);
    expect(h.item(PA)?.conflict).toBeNull();
  });

  test('all writes only the songs that need saving, in list order', async () => {
    const h = await opened();
    await h.session.switchSong(PC);
    h.type('// c');
    await h.session.switchSong(PA);
    h.type('// a');
    expect(await h.session.saveSongs({ all: true })).toEqual({ ok: true, saved: [PA, PC] });
    expect(saves(h)).toEqual([`save:${PA}`, `save:${PC}`]);
    expect(h.fake.confirmCounts).toEqual([]);
    expect(h.st().songs.every((s) => !s.dirty)).toBe(true);
  });

  test('all with nothing dirty saves nothing', async () => {
    const h = await opened();
    expect(await h.session.saveSongs({ all: true })).toEqual({ ok: true, saved: [] });
    expect(saves(h)).toEqual([]);
  });

  test('a failing write ends the call and keeps the earlier saves', async () => {
    const h = await opened();
    await h.session.switchSong(PC);
    h.type('// c');
    await h.session.switchSong(PA);
    h.type('// a');
    h.fake.failSave.add(PC);
    const out = await h.session.saveSongs({ all: true });
    expect(out).toMatchObject({ ok: false, reason: 'failed', saved: [PA], failedPath: PC });
    expect(out.ok === false && out.reason === 'failed' && out.error).toContain('disk full');
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.item(PC)?.dirty).toBe(true);
  });

  test('an unknown song gives missing', async () => {
    const h = await opened();
    expect(await h.session.saveSongs({ path: 'songs/nope.js' })).toEqual({
      ok: false,
      reason: 'missing',
    });
  });

  test('a song renamed while its save waited gives missing and writes nothing', async () => {
    const h = await opened();
    h.type('// mine');
    const hold = deferred();
    h.fake.holdRenames = hold.promise;
    const renaming = h.session.renameSong(PA, 'z');
    await flush();
    const saving = h.session.saveSongs({ path: PA });
    hold.resolve();
    expect(await saving).toEqual({ ok: false, reason: 'missing' });
    await renaming;
    expect(saves(h)).toEqual([]);
  });
});

// ---------------------------------------------------------------- openPath

describe('openPath', () => {
  test('opens another project and installs it', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    const stops = h.rt.stops();
    h.fake.recents = [{ path: '/q', name: 'Other' }];
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: true,
      alreadyOpen: false,
    });
    expect(h.st().project?.root).toBe('/q');
    expect(h.st().active).toBe('songs/q.js');
    expect(h.doc()).toBe(Q);
    expect(h.st().evaluated).toBeNull();
    expect(h.rt.stops()).toBe(stops + 1);
    expect(h.st().recents).toEqual([{ path: '/q', name: 'Other' }]);
    expect(h.fake.confirmCounts).toEqual([]);
  });

  test('the open project, by its canonical root, is alreadyOpen and keeps edits even with discard', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.rootOf['/x/../p'] = '/p';
    h.fake.calls.length = 0;
    expect(await h.session.openPath('/x/../p', { discard: true })).toEqual({
      ok: true,
      alreadyOpen: true,
    });
    expect(h.fake.calls).toEqual(['rootOf:/x/../p']);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.doc()).toBe(A + '// mine');
  });

  test('dirty songs without discard give unsaved with their paths and open nothing', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// b');
    await h.session.switchSong(PA);
    h.type('// a');
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'unsaved',
      dirty: [PA, PB],
    });
    expect(h.fake.calls).not.toContain('open:/q');
    expect(h.fake.confirmCounts).toEqual([]);
    expect(h.st().project?.root).toBe('/p');
  });

  test('a deleted conflict counts as unsaved', async () => {
    const h = await opened();
    h.fake.files().delete(PA); // the active song is kept in memory with a deleted conflict
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(h.item(PA)?.dirty).toBe(false);
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'unsaved',
      dirty: [PA],
    });
    expect(h.st().project?.root).toBe('/p');
    expect(await h.session.openPath('/q', { discard: true })).toMatchObject({ ok: true });
  });

  test('busy when a UI open begins while the root is looked up', async () => {
    const h = await opened();
    const hold = deferred();
    h.fake.holdRootOf = hold.promise;
    const control = h.session.openPath('/q', { discard: false });
    await flush();
    h.fake.holdRootOf = null;
    const ui = h.session.openRecent('/one'); // raises uiOpening, queues behind the control open
    await flush();
    hold.resolve();
    expect(await control).toEqual({ ok: false, reason: 'busy' });
    await ui;
    expect(h.st().project?.root).toBe('/one');
    expect(h.fake.calls).not.toContain('open:/q');
  });

  test('with discard the edits are dropped and it opens', async () => {
    const h = await opened();
    h.type('// a');
    expect(await h.session.openPath('/q', { discard: true })).toMatchObject({ ok: true });
    expect(h.st().project?.root).toBe('/q');
    expect(h.fake.disk.get('/p')!.files.get(PA)).toBe(A);
  });

  test('a failing open gives failed and leaves everything as it was', async () => {
    const h = await opened();
    h.type('// a');
    await h.session.saveAndEvaluate();
    h.type('!');
    const stops = h.rt.stops();
    const before = h.st();
    h.fake.failOpen.add('/q');
    const out = await h.session.openPath('/q', { discard: true });
    expect(out).toMatchObject({ ok: false, reason: 'failed' });
    expect(out.ok === false && out.reason === 'failed' && out.error).toContain(
      'not a strudeland project',
    );
    expect(h.st()).toEqual(before);
    expect(h.st().error).toBeNull();
    expect(h.rt.stops()).toBe(stops);
    expect(h.doc()).toBe(A + '// a!');
  });

  test('a folder that is not a project fails at the root lookup', async () => {
    const h = await opened();
    const out = await h.session.openPath('/nowhere', { discard: false });
    expect(out).toMatchObject({ ok: false, reason: 'failed' });
    expect(out.ok === false && out.reason === 'failed' && out.error).toContain(
      'not a strudeland project',
    );
    expect(h.fake.calls).not.toContain('open:/nowhere');
    expect(h.st().error).toBeNull();
  });

  test('works with no project open', async () => {
    const h = await opened({ last: null });
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: true,
      alreadyOpen: false,
    });
    expect(h.fake.calls.some((c) => c.startsWith('rootOf:'))).toBe(false);
    expect(h.st().project?.root).toBe('/q');
  });

  test('busy while a UI open waits on the folder picker, not after', async () => {
    const h = await opened();
    const hold = deferred();
    h.fake.holdPicks = hold.promise;
    const ui = h.session.openProject();
    await flush();
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'busy',
    });
    hold.resolve(); // cancelled picker
    await ui;
    expect(await h.session.openPath('/q', { discard: false })).toMatchObject({ ok: true });
  });

  test('control open is busy throughout the new-project picker and name prompt', async () => {
    const h = await opened();
    const pick = deferred();
    const name = Promise.withResolvers<string | null>();
    h.fake.holdPicks = pick.promise;
    h.fake.picks.push('/new');
    const askName = mock(() => name.promise);
    const ui = h.session.newProject(askName);
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'busy',
    });
    pick.resolve();
    await flush();
    expect(askName).toHaveBeenCalledWith('/new');
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'busy',
    });
    name.resolve(null);
    await ui;
    expect(h.fake.calls.some((call) => call.startsWith('create:'))).toBe(false);
    expect(await h.session.openPath('/q', { discard: false })).toMatchObject({ ok: true });
  });

  test('busy while a UI open waits on the unsaved modal, not after', async () => {
    const h = await opened();
    h.type('x');
    const hold = deferred();
    h.fake.holdConfirms = hold.promise;
    h.fake.confirm.push('cancel');
    const ui = h.session.openRecent('/one');
    await flush();
    expect(h.fake.confirmCounts).toEqual([1]);
    expect(await h.session.openPath('/q', { discard: true })).toEqual({
      ok: false,
      reason: 'busy',
    });
    hold.resolve();
    await ui;
    expect(h.st().project?.root).toBe('/p');
    expect(await h.session.openPath('/q', { discard: true })).toMatchObject({ ok: true });
  });

  test('busy while a UI open loads the songs', async () => {
    const h = await opened();
    const hold = deferred();
    h.fake.holdReads = hold.promise;
    const ui = h.session.openRecent('/one');
    await flush();
    expect(await h.session.openPath('/q', { discard: false })).toEqual({
      ok: false,
      reason: 'busy',
    });
    hold.resolve();
    await ui;
    expect(h.st().project?.root).toBe('/one');
  });

  test('a control open queued before a UI open starts is busy once the UI open began', async () => {
    const h = await opened();
    const hold = deferred();
    h.fake.holdReads = hold.promise;
    h.fake.files().set(PB, 'changed');
    h.fake.emit({ changed: [PB] }); // holds the serial queue in its read
    await flush();
    const control = h.session.openPath('/q', { discard: false }); // queued behind it
    const ui = h.session.openRecent('/one'); // no dirty songs: raises uiOpening, queues
    hold.resolve();
    h.fake.holdReads = null;
    expect(await control).toEqual({ ok: false, reason: 'busy' });
    await ui;
    expect(h.st().project?.root).toBe('/one');
  });
});

// ---------------------------------------------------------------- guard

describe('unsaved guard (window close)', () => {
  test('application quit and window close share one unsaved prompt', async () => {
    const h = await opened();
    h.type('// dirty');
    const choice = Promise.withResolvers<'save' | 'discard' | 'cancel'>();
    h.api.confirmUnsaved = () => choice.promise;
    const quit = h.session.guardShutdown();
    expect(await h.closeWindow()).toBe(true);
    choice.resolve('cancel');
    expect(await quit).toBe(false);
    expect(await h.session.guardShutdown()).toBe(false);
  });

  test('an active operation blocks window close before the unsaved prompt', async () => {
    const h = setup();
    let exporting = true;
    await h.session.init(() => !exporting);
    h.type('x');
    h.fake.confirm.push('discard');
    expect(await h.closeWindow()).toBe(true);
    expect(h.fake.confirmCounts).toEqual([]);

    exporting = false;
    expect(await h.closeWindow()).toBe(false);
    expect(h.fake.confirmCounts).toEqual([1]);
  });

  test('a close approved by the unsaved guard rechecks for an export started during saving', async () => {
    const h = setup();
    let exporting = false;
    await h.session.init(() => !exporting);
    h.type('x');
    h.fake.confirm.push('save');
    const saving = deferred();
    h.fake.holdSaves = saving.promise;
    const close = h.closeWindow();
    await flush();
    expect(h.fake.confirmCounts).toEqual([1]);

    exporting = true;
    saving.resolve();
    expect(await close).toBe(true);
    expect(h.fake.files().get(PA)).toBe(A + 'x');
  });

  test('no dirty songs: close allowed without a prompt', async () => {
    const h = await opened();
    expect(await h.closeWindow()).toBe(false);
    expect(h.fake.confirmCounts).toEqual([]);
  });

  test('cancel prevents the close', async () => {
    const h = await opened();
    h.type('x');
    h.fake.confirm.push('cancel');
    expect(await h.closeWindow()).toBe(true);
  });

  test('discard allows the close without saving', async () => {
    const h = await opened();
    h.type('x');
    h.fake.confirm.push('discard');
    expect(await h.closeWindow()).toBe(false);
    expect(h.fake.calls.some((c) => c.startsWith('save:'))).toBe(false);
  });

  test('save writes the active and the inactive dirty songs, then allows', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// b');
    await h.session.switchSong(PA);
    h.type('// a');
    h.fake.confirm.push('save');
    expect(await h.closeWindow()).toBe(false);
    expect(h.fake.confirmCounts).toEqual([2]);
    expect(h.fake.files().get(PA)).toBe(A + '// a');
    expect(h.fake.files().get(PB)).toBe(B + '// b');
    expect(h.fake.files().get(PC)).toBe(C);
    expect(h.st().songs.every((s) => !s.dirty)).toBe(true);
  });

  test('any save failure prevents the close', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// b');
    await h.session.switchSong(PA);
    h.type('// a');
    h.fake.failSave.add(PB);
    h.fake.confirm.push('save');
    expect(await h.closeWindow()).toBe(true);
    expect(h.fake.files().get(PA)).toBe(A + '// a'); // the others are still saved
    expect(h.st().error).toContain('disk full');
  });

  test('the count is the number of dirty songs', async () => {
    const h = await opened();
    h.type('x');
    h.fake.confirm.push('cancel');
    await h.closeWindow();
    await h.session.switchSong(PB);
    h.type('y');
    await h.session.switchSong(PC);
    h.type('z');
    h.fake.confirm.push('cancel');
    await h.closeWindow();
    expect(h.fake.confirmCounts).toEqual([1, 3]);
  });

  test('a failing prompt counts as cancel', async () => {
    const h = await opened();
    h.type('x');
    h.api.confirmUnsaved = async () => {
      throw 'dialog failed';
    };
    expect(await h.session.guardUnsaved()).toBe(false);
  });
});

// ---------------------------------------------------------------- store updates

describe('store updates', () => {
  test('typing publishes only when the title, dirty flag or metaError changes', async () => {
    const h = await opened();
    const states: unknown[] = [];
    const unsubscribe = h.session.store.subscribe((s) => states.push(s));
    h.type('x');
    expect(states).toHaveLength(2);
    expect(h.title()).toBe('Proj — Alpha ●');
    h.type('y');
    expect(states).toHaveLength(2);
    h.backspace(2);
    expect(states).toHaveLength(3);
    expect(h.title()).toBe('Proj — Alpha');
    unsubscribe();
  });

  test('a keystroke only rebuilds the active song', async () => {
    const h = await opened();
    const [, b, c] = h.st().songs;
    h.type('x');
    expect(h.st().songs[1]).toBe(b);
    expect(h.st().songs[2]).toBe(c);
  });

  test('a meta title change updates the title as typed', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    expect(h.title()).toBe('Proj — b');
    h.view!.dispatch({ changes: { from: 0, insert: `const meta = { title: 'Beta' };\n` } });
    expect(h.title()).toBe('Proj — Beta ●');
    expect(h.item(PB)?.title).toBe('Beta');
  });

  test('a clean reload never shows the song dirty', async () => {
    const h = await opened();
    const dirty: boolean[] = [];
    const unsubscribe = h.session.store.subscribe((s) => dirty.push(s.songs[0].dirty));
    h.fake.files().set(PA, A + '// more');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(A + '// more');
    expect(dirty.every((d) => !d)).toBe(true);
    unsubscribe();
  });
});

// ---------------------------------------------------------------- metaError

describe('metaError', () => {
  test('set with line:column for an invalid header, cleared on fix', async () => {
    const h = await opened();
    // `title: 1` at line 1; the value starts at 0-based column 22, shown 1-based
    h.view!.dispatch({
      changes: { from: 0, to: A.indexOf('\n'), insert: `const meta = { title: 1 };` },
    });
    expect(h.st().metaError).toContain('meta.title must be a string');
    expect(h.st().metaError).toContain('(1:23)');
    h.view!.dispatch({
      changes: { from: 0, to: h.doc().indexOf('\n'), insert: `const meta = { title: 'ok' };` },
    });
    expect(h.st().metaError).toBeNull();
  });

  test('cleared on switch to a song without one', async () => {
    const h = await opened();
    h.view!.dispatch({
      changes: { from: 0, to: A.indexOf('\n'), insert: `const meta = { bpm: 'fast' };` },
    });
    expect(h.st().metaError).toContain('meta.bpm');
    await h.session.switchSong(PB);
    expect(h.st().metaError).toBeNull();
    await h.session.switchSong(PA);
    expect(h.st().metaError).toContain('meta.bpm');
  });

  test('a syntax error is not a metaError', async () => {
    const h = await opened();
    await h.session.switchSong(PC);
    expect(h.st().metaError).toBeNull();
  });

  test('a body syntax error while typing keeps the title', async () => {
    const h = await opened();
    h.type('$: s(');
    expect(h.item(PA)?.title).toBe('Alpha');
    expect(h.st().metaError).toBeNull();
  });
});

// ---------------------------------------------------------------- structure and evaluated

const SECTIONED = `const meta = {
  title: 'Alpha',
  bpm: 90,
  meter: '6/8',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
  ],
};
$: song(meta, { intro: s("a"), body: s("b") })
`;
const SIXEIGHT = { cps: cpsFor(90, { num: 6, den: 8 }), meter: { num: 6, den: 8 } };
const DEFAULT_TEMPO = { cps: 0.5, meter: { num: 4, den: 4 }, engine: 'superdough' as const };

/** Replace the whole doc of the shown song, as typing would. */
const replaceAll = (h: Awaited<ReturnType<typeof opened>>, text: string) =>
  h.view!.dispatch({ changes: { from: 0, to: h.view!.state.doc.length, insert: text } });

describe('structure', () => {
  test("the active song's static structure, updated as the header is typed", async () => {
    const h = await opened();
    expect(h.st().structure).toEqual(buildStructure({ title: 'Alpha' }));
    replaceAll(h, SECTIONED);
    const s = h.st().structure!;
    expect(s.bpm).toBe(90);
    expect(s.meter).toEqual({ num: 6, den: 8 });
    expect(s.sections.map((x) => x.name)).toEqual(['intro', 'body']);
    expect(s.loopIndex).toBe(1);
  });

  test('typing in the body keeps the same structure object and does not republish', async () => {
    const h = await opened();
    replaceAll(h, SECTIONED);
    const before = h.st().structure;
    const states: unknown[] = [];
    const unsubscribe = h.session.store.subscribe((s) => states.push(s));
    h.type('// x');
    h.type('y');
    expect(states).toHaveLength(1); // still dirty, same title: nothing new to publish
    expect(h.st().structure).toBe(before);
    unsubscribe();
  });

  test('a header with a meta error has the default structure', async () => {
    const h = await opened();
    replaceAll(h, SECTIONED.replace("meter: '6/8'", "meter: 'six'"));
    expect(h.st().metaError).toContain('meta.meter');
    expect(h.st().structure).toEqual(buildStructure({}));
  });

  test('follows the active song, and is null without one', async () => {
    const h = await opened();
    replaceAll(h, SECTIONED);
    await h.session.switchSong(PB);
    expect(h.st().structure).toEqual(buildStructure({}));
    await h.session.switchSong(PA);
    expect(h.st().structure?.bpm).toBe(90);
    const empty = await opened({ last: '/empty' });
    expect(empty.st().structure).toBeNull();
  });
});

describe('evaluateSong', () => {
  test("Cmd-Enter evaluates with the header's cps and meter", async () => {
    const h = await opened();
    replaceAll(h, SECTIONED);
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([SECTIONED]);
    expect(h.rt.options[0]!.cps).toBeCloseTo(SIXEIGHT.cps, 12);
    expect(h.rt.options[0]!.meter).toEqual(SIXEIGHT.meter);
  });

  test('meta.seed is passed to the runtime, and omitted without one', async () => {
    const h = await opened();
    replaceAll(h, A.replace("title: 'Alpha'", "title: 'Alpha', seed: 7"));
    await h.session.saveAndEvaluate();
    expect(h.rt.options[0]!.seed).toBe(7);
    replaceAll(h, A.replace("title: 'Alpha'", "title: 'Alpha', seed: 0"));
    await h.session.saveAndEvaluate();
    expect(h.rt.options[1]!.seed).toBe(0);
    replaceAll(h, A);
    await h.session.saveAndEvaluate();
    expect(h.rt.options[2]!.seed).toBeUndefined();
  });

  test('a bad meta.seed is a meta error and is not evaluated', async () => {
    const h = await opened();
    replaceAll(h, A.replace("title: 'Alpha'", "title: 'Alpha', seed: '7'"));
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([]);
    expect(h.st().metaError).toContain('meta.seed must be an integer between');
  });

  test('a song without a header evaluates at the defaults', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    await h.session.saveAndEvaluate();
    expect(h.rt.options).toEqual([DEFAULT_TEMPO]);
  });

  test('hot reload evaluates with the new header tempo', async () => {
    const h = await opened();
    h.rt.setStarted(true);
    h.fake.files().set(PA, SECTIONED);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.rt.evaluated).toEqual([SECTIONED]);
    expect(h.rt.options[0]!.meter).toEqual(SIXEIGHT.meter);
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
  });

  test('a meta error: saved, but not evaluated', async () => {
    const h = await opened();
    const bad = A.replace("title: 'Alpha'", "title: 'Alpha', bpm: 'fast'");
    replaceAll(h, bad);
    await h.session.saveAndEvaluate();
    expect(h.fake.files().get(PA)).toBe(bad);
    expect(h.rt.evaluated).toEqual([]);
    expect(h.st().evaluated).toBeNull();
    expect(h.st().metaError).toContain('meta.bpm');
  });

  test('a meta error from disk is not hot-reloaded into the runtime', async () => {
    const h = await opened();
    h.rt.setStarted(true);
    const bad = A.replace("title: 'Alpha'", "title: 'Alpha', loops: 2");
    h.fake.files().set(PA, bad);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.doc()).toBe(bad);
    expect(h.rt.evaluated).toEqual([]);
  });
});

describe('transport reset', () => {
  test('switching songs while stopped resets the transport', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    expect(h.rt.resets()).toEqual([]);
    await h.session.switchSong(PB);
    expect(h.rt.resets()).toEqual([0]);
  });

  test('switching songs while playing leaves the playing song its transport', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    h.rt.setStarted(true);
    await h.session.switchSong(PB);
    expect(h.rt.resets()).toEqual([]);
  });

  test('a project change resets the transport', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.openRecent('/q');
    expect(h.rt.resets()).toEqual([0]);
  });

  test('a different song taking over playback resets before it evaluates', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.saveAndEvaluate(); // A
    h.rt.setStarted(true);
    await h.session.switchSong(PB);
    await h.session.saveAndEvaluate(); // B takes over
    expect(h.rt.evaluated).toEqual([A, B]);
    expect(h.rt.resets()).toEqual([1]); // after A's evaluate, before B's
  });

  test('re-evaluating the playing song keeps its transport', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.saveAndEvaluate();
    h.rt.setStarted(true);
    await h.session.saveAndEvaluate();
    expect(h.rt.resets()).toEqual([]);
  });

  test('Play after arming while stopped keeps the armed start, also after a switch', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    await h.session.saveAndEvaluate(); // first Play of A: nothing to reset
    expect(h.rt.resets()).toEqual([]);
    await h.session.switchSong(PB); // stopped: reset here, then the user arms on B
    const n = h.rt.resets().length;
    await h.session.saveAndEvaluate(); // Play B: evaluated was A, but B is not taking over
    expect(h.rt.resets().length).toBe(n);
  });

  test('a start armed while stopped is cleared when a meta error refuses the evaluate', async () => {
    const h = await opened();
    h.rt.resets().length = 0; // the open's own reset
    replaceAll(h, A.replace("title: 'Alpha'", "title: 'Alpha', bpm: 'fast'"));
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([]);
    expect(h.rt.resets()).toEqual([0]);
  });
});

describe('evaluated', () => {
  test('set after a successful evaluate, with the title at that time', async () => {
    const h = await opened();
    expect(h.st().evaluated).toBeNull();
    await h.session.saveAndEvaluate();
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
    await h.session.switchSong(PB);
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' }); // switching is not playing
    await h.session.saveAndEvaluate();
    expect(h.st().evaluated).toEqual({ path: PB, title: 'b' });
  });

  test('a failed evaluate leaves it as it was', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    await h.session.switchSong(PB);
    h.rt.setResult(false);
    await h.session.saveAndEvaluate();
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
  });

  test('cleared when the project changes', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    await h.session.openRecent('/q');
    expect(h.st().project?.name).toBe('Other');
    expect(h.st().evaluated).toBeNull();
  });

  test('an evaluate that resolves after the project changed does not set it', async () => {
    const h = await opened();
    const gate = deferred();
    h.rt.hold(gate.promise);
    const pressed = h.session.saveAndEvaluate();
    await h.session.openRecent('/q');
    gate.resolve();
    await pressed;
    expect(h.st().evaluated).toBeNull();
  });
});

describe('applyMeta', () => {
  test('one undoable edit: marks dirty, updates the structure, is neither saved nor evaluated', async () => {
    const h = await opened();
    h.fake.calls.length = 0;
    expect(undoDepth(h.view!.state)).toBe(0);
    expect(h.session.applyMeta('bpm', 90, h.session.activeText()!)).toBe(true);
    expect(h.doc()).toBe(A.replace("title: 'Alpha'", "title: 'Alpha', bpm: 90"));
    expect(undoDepth(h.view!.state)).toBe(1);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.st().structure?.bpm).toBe(90);
    expect(h.fake.calls).toEqual([]);
    expect(h.rt.evaluated).toEqual([]);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
    expect(h.item(PA)?.dirty).toBe(false);
    expect(h.st().structure?.bpm).toBe(120);
  });

  test('section bars in a sectioned header', async () => {
    const h = await opened();
    replaceAll(h, SECTIONED);
    expect(h.session.applyMeta('sections[1].bars', 8, h.session.activeText()!)).toBe(true);
    expect(h.st().structure?.sections[1]!.bars).toBe(8);
    expect(h.session.applyMeta('loops', 3, h.session.activeText()!)).toBe(true);
    expect(h.st().structure?.loops).toBe(3);
  });

  test('false when the doc changed since it was read', async () => {
    const h = await opened();
    const seen = h.session.activeText()!;
    h.type('x');
    expect(h.session.applyMeta('bpm', 90, seen)).toBe(false);
    expect(h.doc()).toBe(A + 'x');
  });

  test('false when the writer refuses', async () => {
    const h = await opened();
    replaceAll(h, A.replace("title: 'Alpha'", 'bpm: 90, bpm: 100'));
    const depth = undoDepth(h.view!.state);
    expect(h.session.applyMeta('bpm', 120, h.session.activeText()!)).toBe(false);
    expect(h.session.applyMeta('sections[0].bars', 2, h.session.activeText()!)).toBe(false);
    expect(undoDepth(h.view!.state)).toBe(depth);
  });

  test('false without an active song', async () => {
    const h = await opened({ last: '/empty' });
    expect(h.session.activeText()).toBeNull();
    expect(h.session.applyMeta('bpm', 90, '')).toBe(false);
  });

  test('works on a song held without a view, and publishes', async () => {
    const h = await opened({ withView: false });
    expect(h.session.activeText()).toBe(A);
    expect(h.session.applyMeta('meter', '3/4', A)).toBe(true);
    expect(h.session.activeText()).toBe(
      A.replace("title: 'Alpha'", "title: 'Alpha', meter: '3/4'"),
    );
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.st().structure?.meter).toEqual({ num: 3, den: 4 });
  });

  test('successive commits from one focused input each apply (the seen doc follows the edit)', async () => {
    const h = await opened();
    let seen = h.session.activeText()!; // focus
    let r = commitInput(h.session, 'bpm', '90', {}, seen);
    expect(r.outcome).toEqual({ kind: 'applied' });
    seen = r.seen;
    r = commitInput(h.session, 'bpm', '95', {}, seen);
    expect(r.outcome).toEqual({ kind: 'applied' });
    expect(h.st().structure?.bpm).toBe(95);
  });

  test('a commit after the doc changed is stale, and the next one from the same focus applies', async () => {
    const h = await opened();
    const seen = h.session.activeText()!;
    h.type('// x');
    const r = commitInput(h.session, 'bpm', '90', {}, seen);
    expect(r.outcome).toEqual({ kind: 'stale' });
    expect(h.st().structure?.bpm).toBe(120);
    expect(commitInput(h.session, 'bpm', '90', {}, r.seen).outcome).toEqual({ kind: 'applied' });
  });

  test('a commit the writer refuses, or an invalid value, changes nothing', async () => {
    const h = await opened();
    replaceAll(h, A.replace("title: 'Alpha'", 'bpm: 90, bpm: 100'));
    const seen = h.session.activeText()!;
    expect(commitInput(h.session, 'bpm', '120', {}, seen)).toEqual({
      outcome: { kind: 'refused' },
      seen,
    });
    const bad = commitInput(h.session, 'bpm', '5', {}, seen);
    expect(bad.outcome.kind).toBe('invalid');
    expect(bad.seen).toBe(seen);
    expect(h.session.activeText()).toBe(seen);
  });

  test('activeText is the editor text of the active song', async () => {
    const h = await opened();
    h.type('// t');
    expect(h.session.activeText()).toBe(A + '// t');
  });
});

// ---------------------------------------------------------------- control-socket support

const A2 = A.replace('s("a")', 's("a").fast(2)');

describe('externalEdit', () => {
  test('one undo step after typing; undo restores the dirty text', async () => {
    const h = await opened();
    h.type('// mine');
    const result = h.session.externalEdit(PA, A2);
    expect(result).toEqual({ changed: true, dirty: true, conflict: null });
    expect(h.doc()).toBe(A2);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A + '// mine');
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
  });

  test('separated line edits mark only those lines; undo is one step', async () => {
    const h = await opened();
    const lines = Array.from({ length: 100 }, (_, i) => `// line ${i + 1}`);
    const before = lines.join('\n') + '\n';
    h.session.externalEdit(PA, before);
    const after = lines
      .map((l, i) => (i === 1 || i === 79 ? l + ' changed' : l))
      .join('\n')
      .concat('\n');
    h.session.externalEdit(PA, after);
    expect(h.doc()).toBe(after);
    expect(markedLines(h.view!.state)).toEqual([2, 80]);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(before);
  });

  test('appending to a song without a trailing newline marks only the new line', async () => {
    const h = await opened();
    h.session.externalEdit(PA, '$: s("a")');
    h.session.externalEdit(PA, '$: s("a")\n$: s("b")');
    expect(markedLines(h.view!.state)).toEqual([2]);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe('$: s("a")');
  });

  test('typing right after it is a separate undo step', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    h.type('x'); // well inside newGroupDelay
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A2);
    expect(h.undoView()).toBe(true);
    expect(h.doc()).toBe(A);
  });

  test('marks dirty, does not save or evaluate, and publishes', async () => {
    const h = await opened();
    h.fake.calls.length = 0;
    const seen: unknown[] = [];
    const unsub = h.session.store.subscribe((s) => seen.push(s));
    seen.length = 0;
    h.session.externalEdit(PA, A2);
    unsub();
    expect(seen.length).toBeGreaterThan(0);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.fake.calls).toEqual([]);
    expect(h.rt.evaluated).toEqual([]);
    expect(h.fake.files().get(PA)).toBe(A);
  });

  test('identical text is a no-op: no history, no mark, no publish', async () => {
    const h = await opened();
    const before = h.st();
    const state = h.view!.state;
    expect(h.session.externalEdit(PA, A)).toEqual({ changed: false, dirty: false, conflict: null });
    expect(h.view!.state).toBe(state);
    expect(undoDepth(h.view!.state)).toBe(0);
    expect(hasClaudeMark(h.view!.state)).toBe(false);
    expect(h.st()).toBe(before);
  });

  test("'missing' for a non-active or unknown path, and without a project", async () => {
    const h = await opened();
    expect(h.session.externalEdit(PB, 'x')).toBe('missing');
    expect(h.session.externalEdit('songs/nope.js', 'x')).toBe('missing');
    expect(h.doc()).toBe(A);
    const none = await opened({ last: null });
    expect(none.session.externalEdit(PA, 'x')).toBe('missing');
  });

  test("'stale' on an expect mismatch, success on a match", async () => {
    const h = await opened();
    h.type('// mine');
    expect(h.session.externalEdit(PA, A2, { expect: A })).toBe('stale');
    expect(h.doc()).toBe(A + '// mine');
    expect(h.session.externalEdit(PA, A2, { expect: A + '// mine' })).toEqual({
      changed: true,
      dirty: true,
      conflict: null,
    });
    expect(h.doc()).toBe(A2);
  });

  test('dirty and conflict reflect a pending disk change', async () => {
    const h = await opened();
    h.type('// mine');
    h.fake.files().set(PA, Q);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(h.item(PA)?.conflict).toBe('changed');
    expect(h.session.externalEdit(PA, A2)).toEqual({
      changed: true,
      dirty: true,
      conflict: 'changed',
    });
  });

  test('editing back to the disk text reports clean', async () => {
    const h = await opened();
    h.type('// mine');
    expect(h.session.externalEdit(PA, A)).toEqual({ changed: true, dirty: false, conflict: null });
  });

  test('works on a song held without a view, and publishes', async () => {
    const h = await opened({ withView: false });
    expect(h.session.externalEdit(PA, A2)).toEqual({ changed: true, dirty: true, conflict: null });
    expect(h.session.activeText()).toBe(A2);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.st().notice).toEqual({ kind: 'claude-edit', path: PA, title: 'Alpha' });
  });
});

describe('notice', () => {
  test('set by externalEdit, cleared by typing', async () => {
    const h = await opened();
    expect(h.st().notice).toBeNull();
    h.session.externalEdit(PA, A2);
    expect(h.st().notice).toEqual({ kind: 'claude-edit', path: PA, title: 'Alpha' });
    h.type('x');
    expect(h.st().notice).toBeNull();
  });

  test('cleared by undo, not restored by redo', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    h.undoView();
    expect(h.st().notice).toBeNull();
    redo({ state: h.view!.state, dispatch: (tr) => h.view!.dispatch(tr) });
    expect(h.doc()).toBe(A2);
    expect(h.st().notice).toBeNull();
  });

  test('follows a title the edit changes', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A.replace('Alpha', 'Beta'));
    expect(h.st().notice?.title).toBe('Beta');
  });

  test('hidden on a switch, shown again on switching back', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    await h.session.switchSong(PB);
    expect(h.st().notice).toBeNull();
    await h.session.switchSong(PA);
    expect(h.st().notice).toEqual({ kind: 'claude-edit', path: PA, title: 'Alpha' });
  });

  test('null after a project change', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    h.fake.confirm.push('discard');
    await h.session.openRecent('/q');
    expect(h.st().project?.name).toBe('Other');
    expect(h.st().notice).toBeNull();
  });

  test('a new store object only when the notice changes', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    const before = h.st();
    h.session.docChanged(); // nothing changed
    expect(h.st()).toBe(before);
  });
});

describe('evaluateBuffer', () => {
  test('evaluates the buffer without saving and reports ok', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A2);
    h.fake.calls.length = 0;
    h.rt.patch({ started: true, audio: 'running' });
    expect(await h.session.evaluateBuffer(PA)).toEqual({
      ok: true,
      song: PA,
      title: 'Alpha',
      started: true,
      audio: 'running',
      error: null,
    });
    expect(h.rt.evaluated).toEqual([A2]);
    expect(h.fake.calls).toEqual([]);
    expect(h.item(PA)?.dirty).toBe(true);
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
  });

  test("autostart false reaches the runtime with meta's tempo; the default passes none", async () => {
    const h = await opened();
    await h.session.evaluateBuffer(PA, { autostart: false });
    await h.session.evaluateBuffer(PA);
    await h.session.evaluateBuffer(PA, { autostart: true });
    expect(h.rt.options).toEqual([
      {
        cps: 0.5,
        meter: { num: 4, den: 4 },
        seed: undefined,
        engine: 'superdough',
        autostart: false,
      },
      { cps: 0.5, meter: { num: 4, den: 4 }, seed: undefined, engine: 'superdough' },
      { cps: 0.5, meter: { num: 4, den: 4 }, seed: undefined, engine: 'superdough' },
    ]);
  });

  test('autostart false with a meta error evaluates nothing', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A.replace("title: 'Alpha'", "title: 'Alpha', bpm: 'fast'"));
    const out = await h.session.evaluateBuffer(PA, { autostart: false });
    expect(h.rt.options).toEqual([]);
    expect(out.ok).toBe(false);
    expect(out.error).toBe(`meta: ${h.st().metaError}`);
  });

  test('ok with audio suspended reports started false', async () => {
    const h = await opened();
    const out = await h.session.evaluateBuffer(PA);
    expect(out).toMatchObject({ ok: true, started: false, audio: 'suspended', error: null });
  });

  test("a runtime error carries the runtime's evalError", async () => {
    const h = await opened();
    h.rt.setResult(false);
    h.rt.patch({ evalError: 'Unexpected token' });
    expect(await h.session.evaluateBuffer(PA)).toMatchObject({
      ok: false,
      song: PA,
      title: 'Alpha',
      error: 'Unexpected token',
    });
    expect(h.st().evaluated).toBeNull();
  });

  test("a failure without an evalError says 'evaluate failed'", async () => {
    const h = await opened();
    h.rt.setResult(false);
    expect((await h.session.evaluateBuffer(PA)).error).toBe('evaluate failed');
  });

  test('a meta error is not evaluated and is reported with the status-bar text', async () => {
    const h = await opened();
    h.session.externalEdit(PA, A.replace("title: 'Alpha'", "title: 'Alpha', bpm: 'fast'"));
    const out = await h.session.evaluateBuffer(PA);
    expect(h.rt.evaluated).toEqual([]);
    expect(out.ok).toBe(false);
    expect(out.error).toBe(`meta: ${h.st().metaError}`);
    expect(out.error).toContain('meta.bpm');
  });

  test('never throws: a throwing runtime and an unknown path', async () => {
    const h = await opened();
    h.rt.runtime.evaluate = async () => {
      throw new Error('boom');
    };
    const err = console.error;
    console.error = () => {};
    try {
      expect(await h.session.evaluateBuffer(PA)).toMatchObject({
        ok: false,
        error: 'evaluate failed',
      });
    } finally {
      console.error = err;
    }
    expect(await h.session.evaluateBuffer('songs/nope.js')).toMatchObject({
      ok: false,
      song: 'songs/nope.js',
      title: 'nope',
      error: 'evaluate failed',
    });
  });
});

describe('peekSong', () => {
  test('a read song: its stashed edits and dirty, without switching', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    h.type('// b');
    await h.session.switchSong(PA);
    const state = h.view!.state;
    expect(await h.session.peekSong(PB)).toEqual({ text: B + '// b', dirty: true });
    expect(await h.session.peekSong(PC)).toEqual({ text: C, dirty: false });
    expect(h.st().active).toBe(PA);
    expect(h.view!.state).toBe(state);
  });

  test('the active song: its live text', async () => {
    const h = await opened();
    h.type('x');
    expect(await h.session.peekSong(PA)).toEqual({ text: A + 'x', dirty: true });
  });

  test('loads an unread song without showing it', async () => {
    const h = setup();
    h.fake.failRead.add(PB);
    await h.session.init();
    expect(h.item(PB)?.title).toBe('b');
    h.fake.failRead.delete(PB);
    expect(await h.session.peekSong(PB)).toEqual({ text: B, dirty: false });
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
    expect(h.created).toContain(B);
  });

  test('null for an unreadable or unknown song; active unchanged', async () => {
    const h = setup();
    h.fake.failRead.add(PB);
    await h.session.init();
    expect(await h.session.peekSong(PB)).toBeNull();
    expect(await h.session.peekSong('songs/nope.js')).toBeNull();
    expect(h.st().active).toBe(PA);
  });
});

describe('shouldAutoEval', () => {
  test('only while this song is the one playing', () => {
    expect(shouldAutoEval(true, PA, PA)).toBe(true);
    expect(shouldAutoEval(true, PB, PA)).toBe(false); // another song plays
    expect(shouldAutoEval(false, PA, PA)).toBe(false); // stopped
    expect(shouldAutoEval(false, null, PA)).toBe(false); // nothing evaluated
    expect(shouldAutoEval(true, null, PA)).toBe(false);
  });
});

// ---------------------------------------------------------------- beforeProjectChange

describe('beforeProjectChange', () => {
  /** A hook that logs into the fake's call list and waits for `gate` when given. */
  function hook(log: string[], gate?: Promise<void>) {
    return async () => {
      log.push('hook');
      await gate;
      log.push('hook done');
    };
  }

  test('awaited before another project opens, after the unsaved guard', async () => {
    const gate = deferred();
    const log: string[] = [];
    const h = await opened({ beforeProjectChange: hook(log, gate.promise) });
    expect(log).toEqual([]); // the first open (init) has no project to leave
    h.type('x');
    h.fake.confirm.push('discard');
    const opening = h.session.openRecent('/q');
    await flush();
    expect(h.fake.confirmCounts).toEqual([1]);
    expect(log).toEqual(['hook']);
    expect(h.fake.calls).not.toContain('open:/q');
    expect(h.st().project?.root).toBe('/p');
    gate.resolve();
    await opening;
    expect(log).toEqual(['hook', 'hook done']);
    expect(h.st().project?.root).toBe('/q');
  });

  test('newProject and the open picker call it too', async () => {
    const log: string[] = [];
    const h = await opened({ beforeProjectChange: hook(log) });
    h.fake.picks.push('/new');
    await h.session.newProject(async () => 'Fresh');
    expect(log).toEqual(['hook', 'hook done']);
    h.fake.picks.push('/q');
    await h.session.openProject();
    expect(log).toEqual(['hook', 'hook done', 'hook', 'hook done']);
    expect(h.st().project?.root).toBe('/q');
  });

  test('not called without an open project or when the guard cancels', async () => {
    const log: string[] = [];
    const h = await opened({ last: null, beforeProjectChange: hook(log) });
    await h.session.openRecent('/p');
    expect(log).toEqual([]);
    h.type('x');
    h.fake.confirm.push('cancel');
    await h.session.openRecent('/q');
    expect(log).toEqual([]);
    expect(h.st().project?.root).toBe('/p');
  });

  test('openPath calls it for another project, not for the open one or an unsaved refusal', async () => {
    const log: string[] = [];
    const h = await opened({ beforeProjectChange: hook(log) });
    h.fake.rootOf['/x/../p'] = '/p';
    await h.session.openPath('/x/../p', { discard: false });
    expect(log).toEqual([]);
    h.type('x');
    await h.session.openPath('/q', { discard: false });
    expect(log).toEqual([]);
    expect(await h.session.openPath('/q', { discard: true })).toEqual({
      ok: true,
      alreadyOpen: false,
    });
    expect(log).toEqual(['hook', 'hook done']);
  });

  test('a failing hook is logged and the switch goes on', async () => {
    const h = await opened({
      beforeProjectChange: async () => {
        throw new Error('nope');
      },
    });
    const errors: unknown[] = [];
    const orig = console.error;
    console.error = (e: unknown) => void errors.push(e);
    try {
      await h.session.openRecent('/q');
    } finally {
      console.error = orig;
    }
    expect(h.st().project?.root).toBe('/q');
    expect(errors.map((e) => (e as Error).message)).toEqual(['nope']);
  });
});

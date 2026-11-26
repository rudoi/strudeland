import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, mock, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createSession } = await import('../src/lib/project/session');
const { createEditorState } = await import('../src/lib/editor/setup');
const { createProjectActions } = await import('../src/lib/project/actions');
const { createShell, allowed } = await import('../src/lib/shell');
const { EditorView } = await import('@codemirror/view');
const { get, writable } = await import('svelte/store');
import type { ExportApi, ExportResult } from '../src/lib/export/api';
import type { createExportDialog } from '../src/lib/export/controller';
import type { Session, SessionState } from '../src/lib/project/session';
import type { Gate, Modal, ShellAction } from '../src/lib/shell';
import { fakeProjectApi, flush } from './helpers/fake-project-api';
import { fakeShellPorts } from './helpers/fake-shell-ports';
import { stubRuntime } from './helpers/stub-runtime';

const A = `const meta = { title: 'Alpha', bpm: 120 };\n$: s("a")\n`;
const PA = 'songs/a.js';
const PROJECTS = {
  '/p': { name: 'Proj', files: { [PA]: A } },
  '/q': { name: 'Other', files: { [PA]: A } },
  '/empty': { name: 'Empty', files: {} },
};
const audio = { role: 'single', path: '/p/renders/a.wav', exists: false } as const;
const result = {
  files: [{ role: 'single', path: audio.path, godotLoopOffset: null }],
  warnings: [],
} satisfies ExportResult;

/** A real session (not yet initialised: only `shell.start()` calls `init`), the fake ports. */
function setup(opts: { tauri?: boolean } = {}) {
  const { api, fake } = fakeProjectApi(PROJECTS);
  fake.last = '/p';
  const rt = stubRuntime({ audio: 'running', live: true });
  const view = new EditorView({ parent: document.body });
  let session!: Session;
  session = createSession({
    api,
    runtime: rt.runtime,
    createState: (doc: string) =>
      createEditorState(doc, rt.runtime, { onDocChange: () => session.docChanged() }),
    isTauri: () => opts.tauri ?? true,
  });
  session.attachView(view);
  const projectSession = {
    openProject: mock(async () => {}),
    openRecent: mock(async (_path: string) => {}),
    newProject: mock(async (_askName: (dir: string) => Promise<string | null>) => {}),
  };
  const projectActions = createProjectActions(projectSession);
  const exportApi = {
    preview: mock<ExportApi['preview']>(async () => [audio]),
    pickFolder: mock<ExportApi['pickFolder']>(async () => null),
    start: mock<ExportApi['start']>(async () => result),
  };
  const ports = fakeShellPorts();
  let focused: HTMLElement | null = null;
  const shell = createShell({
    session,
    projectActions,
    exportApi,
    quit: ports.quit,
    menu: ports.menu,
    activeElement: () => focused,
  });
  const st = () => get(shell.store);
  const type = (text: string) =>
    view.dispatch({
      changes: { from: view.state.doc.length, insert: text },
      userEvent: 'input.type',
    });
  /** Runs the real session's window-close handler; true when it prevented the close. */
  const closeWindow = async () => {
    let prevented = false;
    await fake.closeHandlers[0]({ preventDefault: () => void (prevented = true) });
    return prevented;
  };
  /** Opens the export dialog through the menu and returns its controller. */
  const openExport = () => {
    ports.emitMenu('exportAudio');
    const modal = st().modal;
    if (modal?.kind !== 'export') throw new Error('export did not open');
    return modal.controller;
  };
  return {
    api,
    fake,
    session,
    view,
    projectSession,
    projectActions,
    exportApi,
    ports,
    shell,
    st,
    type,
    closeWindow,
    openExport,
    focus: (el: HTMLElement | null) => void (focused = el),
  };
}

/** Started, the last project open with its song active, both listeners attached. */
async function started(opts: { tauri?: boolean } = {}) {
  const h = setup(opts);
  h.shell.start();
  await flush();
  return h;
}

type ExportDialog = ReturnType<typeof createExportDialog>;

const button = () => document.createElement('button');

type ShellSession = Parameters<typeof createShell>[0]['session'];

/** The dialog's preview is ready, so `start` exports (the song has no sections: give a length). */
async function previewed(controller: ExportDialog) {
  controller.select({ bars: '4' });
  await flush();
  expect(get(controller.store).preview.kind).toBe('ready');
}

describe('quit', () => {
  test('1: quit while exporting waits, then guards unsaved, then replies approved', async () => {
    const h = await started();
    h.type('// dirty');
    const controller = h.openExport();
    await previewed(controller);
    const exportDone = Promise.withResolvers<ExportResult>();
    h.exportApi.start.mockReturnValueOnce(exportDone.promise);
    const running = controller.start();

    const order: string[] = [];
    const requestQuit = controller.requestQuit;
    controller.requestQuit = async () => {
      const ok = await requestQuit();
      order.push(`requestQuit:${ok}`);
      return ok;
    };
    const confirm = h.api.confirmUnsaved;
    h.api.confirmUnsaved = (count) => {
      order.push(`confirm:${count}`);
      return confirm(count);
    };
    const reply = h.ports.quit.reply;
    h.ports.quit.reply = (id, approved) => {
      order.push(`reply:${id}:${approved}`);
      return reply(id, approved);
    };
    const finishQuit = controller.finishQuit;
    controller.finishQuit = () => {
      order.push('finishQuit');
      finishQuit();
    };
    h.fake.confirm.push('save');

    const quit = h.shell.onQuit(1);
    expect(get(controller.store).quit).toBe('ask');
    controller.answerQuit('wait');
    expect(get(controller.store).quit).toBe('waiting');
    exportDone.resolve(result);
    await running;
    expect(get(controller.store).quit).toBe('result');
    controller.answerQuit('continue');
    await quit;

    expect(order).toEqual(['requestQuit:true', 'confirm:1', 'reply:1:true', 'finishQuit']);
    expect(get(controller.store).quit).toBeNull();
    expect(h.shell.quitting).toBe(false);
    expect(h.st().quitting).toBe(false);
    expect(h.fake.files().get(PA)).toBe(A + '// dirty');
  });

  test('2: menu actions during quit are ignored', async () => {
    const h = await started();
    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    h.fake.confirm.push('cancel');
    const quit = h.shell.onQuit(1);
    await flush();
    expect(h.fake.confirmCounts).toEqual([1]);

    for (const action of ['exportAudio', 'settings', 'openProject', 'newProject'] as const) {
      h.ports.emitMenu(action);
      expect(h.st().modal).toBeNull();
    }
    expect(h.projectSession.openProject).not.toHaveBeenCalled();
    expect(h.projectSession.newProject).not.toHaveBeenCalled();

    prompt.resolve();
    await quit;
    expect(h.ports.log.at(-1)).toBe('reply:1:false');
  });

  test('3: the newSong shortcut while export is open does nothing', async () => {
    const h = await started();
    h.openExport();
    expect(h.shell.shortcut('newSong')).toBe(false);
    expect(h.st().modal?.kind).toBe('export');
  });

  test('4: a second quit replies false while the first is held; the first then completes', async () => {
    const h = await started();
    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    h.fake.confirm.push('discard');
    const first = h.shell.onQuit(1);
    await flush();
    await h.shell.onQuit(2);
    expect(h.ports.log.filter((l) => l.startsWith('reply'))).toEqual(['reply:2:false']);
    expect(h.shell.quitting).toBe(true);

    prompt.resolve();
    await first;
    expect(h.ports.log.filter((l) => l.startsWith('reply'))).toEqual([
      'reply:2:false',
      'reply:1:true',
    ]);
    expect(h.shell.quitting).toBe(false);
  });

  test('5: a throwing reply still clears quitting and runs finishQuit', async () => {
    const h = await started();
    const controller = h.openExport();
    await previewed(controller);
    h.ports.replyFails = true;
    let error: unknown;
    try {
      await h.shell.onQuit(1);
    } catch (e) {
      error = e;
    }
    expect((error as Error).message).toBe('reply failed');
    expect(h.ports.log.at(-1)).toBe('reply:1:true');
    expect(h.shell.quitting).toBe(false);
    expect(h.st().quitting).toBe(false);
    expect(get(controller.store).quit).toBeNull();
  });

  test('15: quitting is observable synchronously', async () => {
    const h = await started();
    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    h.fake.confirm.push('discard');
    const quit = h.shell.onQuit(1);
    expect(h.shell.quitting).toBe(true);
    expect(h.st().quitting).toBe(true);
    await flush();
    expect(h.shell.quitting).toBe(true);

    prompt.resolve();
    await quit;
    expect(h.ports.log.at(-1)).toBe('reply:1:true');
    expect(h.shell.quitting).toBe(false);
    expect(h.st().quitting).toBe(false);
  });

  test('a native quit request reaches onQuit through the registered listener', async () => {
    const h = await started();
    h.ports.emitQuit(7);
    await flush();
    expect(h.ports.log.at(-1)).toBe('reply:7:true');
  });
});

describe('window close', () => {
  test('7: canCloseWindow is false while quitting, exporting or a quit is pending', async () => {
    const h = await started();
    expect(h.shell.canCloseWindow()).toBe(true);

    // Quitting, with no export open.
    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    h.fake.confirm.push('cancel');
    const quit = h.shell.onQuit(1);
    expect(h.shell.canCloseWindow()).toBe(false);
    prompt.resolve();
    await quit;
    h.fake.holdConfirms = null;
    expect(h.shell.canCloseWindow()).toBe(true);

    // A running export.
    const controller = h.openExport();
    await previewed(controller);
    expect(h.shell.canCloseWindow()).toBe(true);
    const exportDone = Promise.withResolvers<ExportResult>();
    h.exportApi.start.mockReturnValueOnce(exportDone.promise);
    const running = controller.start();
    expect(h.shell.canCloseWindow()).toBe(false);
    // Through the real session's close handler: blocked silently, no prompt.
    const prompts = h.fake.confirmCounts.length;
    expect(await h.closeWindow()).toBe(true);
    expect(h.fake.confirmCounts.length).toBe(prompts);
    exportDone.resolve(result);
    await running;
    expect(h.shell.canCloseWindow()).toBe(true);

    // A pending quit: settling before finishQuit, outside the shell's own quit.
    const settled = controller.requestQuit();
    expect(get(controller.store).quit).toBe('result');
    controller.answerQuit('continue');
    expect(await settled).toBe(true);
    expect(get(controller.store).quit).toBe('settling');
    expect(h.shell.canCloseWindow()).toBe(false);
    controller.finishQuit();
    expect(h.shell.canCloseWindow()).toBe(true);
    // Allowed: the real handler runs the unsaved prompt.
    h.fake.confirm.push('discard');
    expect(await h.closeWindow()).toBe(false);
    expect(h.fake.confirmCounts.length).toBe(prompts + 1);
  });
});

describe('gate table', () => {
  const ACTIONS: ShellAction[] = [
    'newProject',
    'openProject',
    'settings',
    'exportAudio',
    'newSong',
    'openRecent',
    'instruments',
  ];
  const base: Gate = {
    quitting: false,
    modal: false,
    busy: false,
    ready: true,
    project: true,
    active: true,
    naming: false,
  };
  type Flip = 'quitting' | 'modal' | 'busy' | 'not ready' | 'no project' | 'no active' | 'naming';
  const FLIPS: Record<Flip, Partial<Gate>> = {
    quitting: { quitting: true },
    modal: { modal: true },
    busy: { busy: true },
    'not ready': { ready: false },
    'no project': { project: false },
    'no active': { active: false },
    naming: { naming: true },
  };
  /** The flips that refuse each action; every other flip leaves it allowed. */
  const REFUSED: Record<ShellAction, Flip[]> = {
    newProject: ['quitting', 'modal', 'not ready'],
    openProject: ['quitting', 'modal', 'not ready'],
    settings: ['quitting', 'modal', 'busy'],
    exportAudio: ['quitting', 'modal', 'busy', 'not ready', 'no project', 'no active'],
    newSong: ['quitting', 'modal', 'no project', 'naming'],
    openRecent: [],
    instruments: ['modal', 'no project'],
  };

  test('8: every action against each condition flipped alone', () => {
    for (const action of ACTIONS) {
      expect(allowed(action, base)).toBe(true);
      for (const [flip, change] of Object.entries(FLIPS) as [Flip, Partial<Gate>][]) {
        const want = !REFUSED[action].includes(flip);
        expect({ action, flip, allowed: allowed(action, { ...base, ...change }) }).toEqual({
          action,
          flip,
          allowed: want,
        });
      }
    }
  });

  test('8: openRecent is allowed with every gate false or true; settings is allowed while not ready', () => {
    const all = (v: boolean): Gate => ({
      quitting: v,
      modal: v,
      busy: v,
      ready: v,
      project: v,
      active: v,
      naming: v,
    });
    expect(allowed('openRecent', all(false))).toBe(true);
    expect(allowed('openRecent', all(true))).toBe(true);
    expect(allowed('settings', { ...base, ready: false, project: false, active: false })).toBe(
      true,
    );
  });

  test('8: each verb with each modal kind open is refused or acts per its row', async () => {
    const kinds: Modal['kind'][] = ['settings', 'instruments', 'export'];
    for (const kind of kinds) {
      const h = await started();
      if (kind === 'settings') h.shell.openSettings(button());
      else if (kind === 'instruments') h.shell.openInstruments(button());
      else h.openExport();
      const open = h.st().modal;
      expect(open?.kind).toBe(kind);

      h.shell.projectAction('newProject');
      h.shell.projectAction('openProject');
      h.ports.emitMenu('newProject');
      h.ports.emitMenu('openProject');
      expect(h.shell.shortcut('openProject')).toBe(false);
      expect(h.projectSession.newProject).not.toHaveBeenCalled();
      expect(h.projectSession.openProject).not.toHaveBeenCalled();
      h.shell.openSettings(button());
      h.ports.emitMenu('settings');
      h.ports.emitMenu('exportAudio');
      h.shell.openInstruments(button());
      expect(h.shell.shortcut('newSong')).toBe(false);
      expect(h.st().modal).toBe(open);

      // openRecent is not gated.
      h.shell.openRecent('/q');
      expect(h.projectSession.openRecent).toHaveBeenCalledWith('/q');
      h.shell.destroy();
    }
  });

  test('8: with no modal, each verb acts', async () => {
    const h = await started();
    h.shell.projectAction('newProject');
    await flush();
    h.ports.emitMenu('openProject');
    await flush();
    expect(h.projectSession.newProject).toHaveBeenCalledTimes(1);
    expect(h.projectSession.openProject).toHaveBeenCalledTimes(1);
    expect(h.shell.shortcut('newSong')).toBe(true);
    h.shell.openInstruments(button());
    expect(h.st().modal?.kind).toBe('instruments');
    h.shell.close('instruments');
    h.ports.emitMenu('settings');
    expect(h.st().modal?.kind).toBe('settings');
    h.shell.close('settings');
    h.ports.emitMenu('exportAudio');
    expect(h.st().modal?.kind).toBe('export');
  });

  test('8: busy refuses settings and export, not project actions (run drops them) or newSong', async () => {
    const h = await started();
    const open = Promise.withResolvers<void>();
    h.projectSession.openProject.mockImplementationOnce(() => open.promise);
    h.shell.projectAction('openProject');
    expect(h.projectActions.isBusy()).toBe(true);
    h.ports.emitMenu('settings');
    h.shell.openSettings(button());
    h.ports.emitMenu('exportAudio');
    expect(h.st().modal).toBeNull();
    expect(h.shell.shortcut('newSong')).toBe(true);
    h.shell.projectAction('newProject');
    expect(h.projectSession.newProject).not.toHaveBeenCalled();
    open.resolve();
    await flush();
    h.ports.emitMenu('settings');
    expect(h.st().modal?.kind).toBe('settings');
  });
});

describe('modals', () => {
  test('10: the instrument browser is dropped on a root change, kept on the same root', async () => {
    const h = await started();
    const opener = button();
    h.shell.openInstruments(opener);
    const open = h.st().modal;
    expect(open).toMatchObject({ kind: 'instruments', root: '/p', opener });
    h.type('x');
    await flush();
    expect(h.st().modal).toBe(open);

    h.fake.confirm.push('discard');
    await h.session.openRecent('/q');
    await flush();
    expect(get(h.session.store).project?.root).toBe('/q');
    expect(h.st().modal).toBeNull();
  });

  test('11: close returns where focus goes, and ignores a kind that is not open', async () => {
    const h = await started();
    const opener = button();
    h.shell.openSettings(opener);
    expect(h.shell.close('instruments')).toBeNull();
    expect(h.shell.close('export')).toBeNull();
    expect(h.st().modal?.kind).toBe('settings');
    expect(h.shell.close('settings')).toBe(opener);
    expect(h.st().modal).toBeNull();
    expect(h.shell.close('settings')).toBeNull();

    h.shell.openSettings(null);
    expect(h.shell.close('settings')).toBeNull();

    h.shell.openInstruments(opener);
    expect(h.shell.close('instruments')).toBe(opener);
    h.shell.openInstruments(opener);
    expect(h.shell.close('instruments', true)).toBe('editor');

    h.openExport();
    expect(h.shell.close('settings')).toBeNull();
    expect(h.st().modal?.kind).toBe('export');
    expect(h.shell.close('export')).toBeNull();
    expect(h.st().modal).toBeNull();
  });

  test('12: openInstruments with another modal open does nothing', async () => {
    const h = await started();
    h.shell.openSettings(null);
    const settings = h.st().modal;
    h.shell.openInstruments(button());
    expect(h.st().modal).toBe(settings);
    h.shell.close('settings');

    const controller = h.openExport();
    h.shell.openInstruments(button());
    expect(h.st().modal).toMatchObject({ kind: 'export', controller });
  });

  test('14: the menu settings opener is what activeElement returns at the event', async () => {
    const h = await started();
    const el = button();
    h.focus(el);
    h.ports.emitMenu('settings');
    expect(h.st().modal).toEqual({ kind: 'settings', opener: el, restore: el });
    h.shell.close('settings');
    h.focus(null);
    h.ports.emitMenu('settings');
    expect(h.st().modal).toEqual({ kind: 'settings', opener: null, restore: null });
  });
});

describe('shortcuts', () => {
  test('13: openProject runs when allowed and not when blocked, returning false', async () => {
    const h = await started();
    expect(h.shell.shortcut('openProject')).toBe(false);
    await flush();
    expect(h.projectSession.openProject).toHaveBeenCalledTimes(1);

    h.shell.openSettings(null);
    expect(h.shell.shortcut('openProject')).toBe(false);
    h.shell.close('settings');

    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    const quit = h.shell.onQuit(1);
    expect(h.shell.shortcut('openProject')).toBe(false);
    prompt.resolve();
    await quit;
    await flush();
    expect(h.projectSession.openProject).toHaveBeenCalledTimes(1);
  });

  test('13: newSong needs a project and no naming; busy and not ready do not block it', async () => {
    const h = setup({ tauri: false });
    h.shell.start();
    await flush();
    expect(h.st().ready).toBe(false);
    expect(h.shell.shortcut('newSong')).toBe(false); // no project (not available)

    const s = await started();
    expect(s.shell.shortcut('newSong')).toBe(true);
    const busy = Promise.withResolvers<void>();
    s.projectSession.openProject.mockImplementationOnce(() => busy.promise);
    s.shell.projectAction('openProject');
    expect(s.projectActions.isBusy()).toBe(true);
    expect(s.shell.shortcut('newSong')).toBe(true);
    busy.resolve();
    await flush();

    s.projectSession.newProject.mockImplementationOnce(async (askName) => {
      await askName('/music/new');
    });
    s.shell.projectAction('newProject');
    await flush();
    expect(get(s.projectActions.folder)).toBe('/music/new');
    expect(s.shell.shortcut('newSong')).toBe(false);

    // Not ready but with a project: a stub session that never finishes init.
    const store = writable({ ...get(s.session.store) });
    const stub: ShellSession = {
      store,
      init: () => new Promise<void>(() => {}),
      guardShutdown: async () => true,
      instrumentSelection: () => null,
      activeText: () => null,
    };
    const ports = fakeShellPorts();
    const shell = createShell({
      session: stub,
      projectActions: createProjectActions(s.projectSession),
      exportApi: s.exportApi,
      quit: ports.quit,
      menu: ports.menu,
      activeElement: () => null,
    });
    shell.start();
    expect(get(shell.store).ready).toBe(false);
    expect(shell.shortcut('newSong')).toBe(true);
  });
});

describe('start, the menu sync and destroy', () => {
  test('6: export menu availability', async () => {
    const h = setup();
    const attach = Promise.withResolvers<void>();
    h.ports.holdMenuListen = attach.promise;
    h.shell.start();
    await flush();
    expect(get(h.session.store).active).toBe(PA);
    expect(h.ports.log).toEqual(['menu.listen']);

    attach.resolve();
    await flush();
    expect(h.ports.log).toEqual(['menu.listen', 'quit.listen', 'exportAvailable:true']);

    const sent = () => h.ports.log.filter((l) => l.startsWith('exportAvailable'));
    h.type('x'); // an unchanged value sends nothing
    await flush();
    expect(sent()).toEqual(['exportAvailable:true']);

    h.fake.confirm.push('discard');
    await h.session.openRecent('/empty');
    await flush();
    expect(get(h.session.store).active).toBeNull();
    expect(sent()).toEqual(['exportAvailable:true', 'exportAvailable:false']);

    await h.session.openRecent('/p');
    await flush();
    expect(sent()).toEqual([
      'exportAvailable:true',
      'exportAvailable:false',
      'exportAvailable:true',
    ]);
  });

  test('6: the first send after attach is false when no song is open', async () => {
    const h = setup();
    h.fake.last = '/empty';
    h.shell.start();
    await flush();
    expect(h.ports.log).toEqual(['menu.listen', 'quit.listen', 'exportAvailable:false']);
  });

  test('6: not ready never attaches and never sends', async () => {
    const h = setup({ tauri: false });
    h.shell.start();
    await flush();
    expect(h.st().ready).toBe(false);
    expect(h.ports.log).toEqual([]);
  });

  test('7: start hands canCloseWindow to the session close hook', async () => {
    const h = await started();
    expect(h.fake.closeHandlers.length).toBe(1);
    h.type('x');
    const prompt = Promise.withResolvers<void>();
    h.fake.holdConfirms = prompt.promise;
    h.fake.confirm.push('cancel', 'discard');
    const quit = h.shell.onQuit(1);
    await flush();
    const prompts = h.fake.confirmCounts.length;
    // Quitting: the window close is refused without a second prompt.
    expect(await h.closeWindow()).toBe(true);
    expect(h.fake.confirmCounts.length).toBe(prompts);
    prompt.resolve();
    await quit;
    h.fake.holdConfirms = null;
    expect(await h.closeWindow()).toBe(false);
  });

  test('9: stop before the menu listen resolves unregisters it; quit.listen is never called', async () => {
    const h = setup();
    const attach = Promise.withResolvers<void>();
    h.ports.holdMenuListen = attach.promise;
    const stop = h.shell.start();
    await flush();
    stop();
    attach.resolve();
    await flush();
    expect(h.ports.log).toEqual(['menu.listen', 'menu.unlisten']);
  });

  test('9: stop before the quit listen resolves unregisters it; a later quit is not handled', async () => {
    const h = setup();
    const attach = Promise.withResolvers<void>();
    h.ports.holdQuitListen = attach.promise;
    const stop = h.shell.start();
    await flush();
    stop();
    attach.resolve();
    await flush();
    expect(h.ports.log).toEqual([
      'menu.listen',
      'quit.listen',
      'exportAvailable:true',
      'menu.unlisten',
      'quit.unlisten',
    ]);
    h.ports.emitQuit(1);
    h.ports.emitMenu('settings');
    await flush();
    expect(h.ports.log.some((l) => l.startsWith('reply'))).toBe(false);
    expect(h.st().modal).toBeNull();
  });

  test('9: stop after both listeners attached unregisters both, once', async () => {
    const h = setup();
    const stop = h.shell.start();
    await flush();
    expect(h.shell.start()).toBe(stop); // start runs once
    stop();
    stop();
    expect(h.ports.log).toEqual([
      'menu.listen',
      'quit.listen',
      'exportAvailable:true',
      'menu.unlisten',
      'quit.unlisten',
    ]);
    h.ports.emitQuit(1);
    await flush();
    expect(h.ports.log.some((l) => l.startsWith('reply'))).toBe(false);
  });

  test('9: stop before init resolves: no listen at all, ready stays false', async () => {
    const init = Promise.withResolvers<void>();
    const store = writable({ available: true, project: null, active: null } as SessionState);
    const stub: ShellSession = {
      store,
      init: () => init.promise,
      guardShutdown: async () => true,
      instrumentSelection: () => null,
      activeText: () => null,
    };
    const ports = fakeShellPorts();
    const actions = createProjectActions({
      openProject: async () => {},
      openRecent: async () => {},
      newProject: async () => {},
    });
    const shell = createShell({
      session: stub,
      projectActions: actions,
      exportApi: {} as ExportApi,
      quit: ports.quit,
      menu: ports.menu,
      activeElement: () => null,
    });
    const stop = shell.start();
    stop();
    init.resolve();
    await flush();
    expect(ports.log).toEqual([]);
    expect(get(shell.store).ready).toBe(false);
  });

  test('16: ready is true when menu.listen is called, and menu.listen resolves before quit.listen', async () => {
    const h = setup();
    const seen: boolean[] = [];
    h.ports.onMenuListen = () => seen.push(h.st().ready);
    const order: string[] = [];
    const menuListen = h.ports.menu.listen;
    h.ports.menu.listen = async (handle) => {
      order.push('menu.listen called');
      const stop = await menuListen(handle);
      order.push('menu.listen resolved');
      return stop;
    };
    const quitListen = h.ports.quit.listen;
    h.ports.quit.listen = (handle) => {
      order.push('quit.listen called');
      return quitListen(handle);
    };
    h.shell.start();
    await flush();
    expect(seen).toEqual([true]);
    expect(order).toEqual(['menu.listen called', 'menu.listen resolved', 'quit.listen called']);
    expect(h.st().ready).toBe(true);
  });

  test('17: a double destroy is safe and a later store change sends nothing', async () => {
    const h = await started();
    h.shell.destroy();
    h.shell.destroy();
    expect(h.ports.log.filter((l) => l.endsWith('unlisten'))).toEqual([
      'menu.unlisten',
      'quit.unlisten',
    ]);
    const logged = h.ports.log.length;
    h.fake.confirm.push('discard');
    await h.session.openRecent('/empty');
    await flush();
    expect(get(h.session.store).active).toBeNull();
    expect(h.ports.log.length).toBe(logged);
  });
});

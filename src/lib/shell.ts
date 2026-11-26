/**
 * The app shell: what the window does around the editor. It decides which one dialog is open
 * (settings, the instrument browser, export) and where focus goes back to when it closes, whether
 * a Quit is being decided and in what order it asks, whether the window may close (the predicate
 * it hands `session.init`), whether a menu item, shortcut or welcome button acts now (one gate
 * table), and whether the native Export Audio item is enabled. It also registers the menu and
 * Quit listeners.
 *
 * Menu items are never disabled for a dialog, a Quit or a busy action: those actions are ignored.
 * Only Export Audio is greyed out, until a song is open.
 *
 * Every dependency is called through its object at call time. The shell reads no DOM: an opener
 * is an opaque element, the menu's settings opener comes from `activeElement`, and focusing (after
 * `tick()`) stays in `App.svelte`. Tauri is reached only through the two ports; `shell-ports.ts`
 * is their Tauri side and `tests/helpers/fake-shell-ports.ts` their fake.
 */
import { get, writable, type Readable } from 'svelte/store';
import type { ExportApi } from './export/api';
import { captureExportSong, createExportDialog, type ExportSong } from './export/controller';
import type { createProjectActions } from './project/actions';
import type { ProjectAction, ProjectMenuAction } from './project/api';
import type { Session, SessionState } from './project/session';
import type { AppShortcut } from './project/ui';

/** Native Quit: its requests and the reply to each. */
export interface QuitPort {
  listen(handle: (id: number) => void): Promise<() => void>;
  reply(id: number, approved: boolean): Promise<void>;
}

/** The native project menu: its action events and the Export Audio item's availability. */
export interface MenuPort {
  listen(handle: (action: ProjectMenuAction) => void): Promise<() => void>;
  setExportAvailable(available: boolean): Promise<void>;
}

/** What the shell uses of the session. */
type ShellSession = Pick<
  Session,
  'init' | 'store' | 'guardShutdown' | 'instrumentSelection' | 'activeText'
>;

type ShellProjectActions = Pick<
  ReturnType<typeof createProjectActions>,
  'run' | 'openRecent' | 'isBusy' | 'folder'
>;

/**
 * The one open dialog. NewProject is not one: it follows `projectActions.folder`. `restore` is
 * where focus goes when it closes without applying (export restores focus itself on unmount).
 */
export type Modal =
  | { kind: 'settings'; restore: HTMLElement | null }
  | {
      kind: 'instruments';
      root: string;
      selection: ReturnType<Session['instrumentSelection']>;
      restore: HTMLButtonElement;
    }
  | {
      kind: 'export';
      song: ExportSong;
      controller: ReturnType<typeof createExportDialog>;
      restore: null;
    };

export interface ShellState {
  /** The session opened and projects are available. Never goes back to false. */
  ready: boolean;
  /** A Quit is being decided. */
  quitting: boolean;
  modal: Modal | null;
}

/** Where focus goes after a dialog closes: the editor, an element, or nowhere (null). */
export type FocusTarget = 'editor' | HTMLElement | null;

export type ShellAction =
  | 'newProject'
  | 'openProject'
  | 'settings'
  | 'exportAudio'
  | 'newSong'
  | 'openRecent'
  | 'instruments';

export interface Gate {
  quitting: boolean;
  modal: boolean;
  busy: boolean;
  ready: boolean;
  project: boolean;
  active: boolean;
  naming: boolean;
}

/**
 * Whether `action` acts now. The rows differ on purpose: each keeps the preconditions its caller
 * had before the shell existed (project actions drop a busy call themselves, `openRecent` was
 * never gated, the newSong shortcut ignores busy and ready). One row is new: `instruments` adds
 * `!g.modal`. It is unreachable today (every dialog uses `showModal`, so the instruments button
 * cannot be clicked under one), and it keeps the one modal slot from orphaning an export
 * controller. Whether the rows should agree is an open decision; this table is the one place to
 * change them.
 */
export function allowed(action: ShellAction, g: Gate): boolean {
  switch (action) {
    case 'newProject':
    case 'openProject':
      return !g.quitting && !g.modal && g.ready;
    case 'settings':
      return !g.quitting && !g.modal && !g.busy;
    case 'exportAudio':
      return !g.quitting && !g.modal && !g.busy && g.ready && g.project && g.active;
    case 'newSong':
      return !g.quitting && !g.modal && g.project && !g.naming;
    case 'openRecent':
      return true;
    case 'instruments':
      return !g.modal && g.project;
  }
}

export interface Shell {
  store: Readable<ShellState>;
  /** The store's `quitting`, read synchronously; true as soon as `onQuit` starts. */
  readonly quitting: boolean;
  /** Opens the session, then registers the menu and Quit listeners. Returns the stop. */
  start(): () => void;
  menu(action: ProjectMenuAction): void;
  /** The project shortcut runs here; true when the caller should open the new-song form. */
  shortcut(kind: AppShortcut): boolean;
  projectAction(action: ProjectAction): void;
  openRecent(path: string): void;
  openSettings(opener: HTMLElement | null): void;
  openInstruments(opener: HTMLButtonElement): void;
  /**
   * Closes `kind` if it is the open dialog and says where focus goes; null otherwise, except that
   * an applied instruments close is always 'editor'.
   */
  close(kind: Modal['kind'], applied?: boolean): FocusTarget;
  canCloseWindow(): boolean;
  onQuit(id: number): Promise<void>;
  destroy(): void;
}

export function createShell(deps: {
  session: ShellSession;
  projectActions: ShellProjectActions;
  exportApi: ExportApi;
  quit: QuitPort;
  menu: MenuPort;
  activeElement: () => HTMLElement | null;
}): Shell {
  const { session, projectActions, exportApi, quit, menu } = deps;

  // Every field is set before the session subscription, whose first callback runs at once.
  let ready = false;
  let quitting = false;
  let modal: Modal | null = null;
  let snapshot!: SessionState;
  let started = false;
  let stopped = false;
  let unlistenMenu: (() => void) | undefined;
  let unlistenQuit: (() => void) | undefined;
  let lastSent: boolean | undefined;
  let syncPending = false;
  const state = writable<ShellState>({ ready, quitting, modal });
  const emit = () => state.set({ ready, quitting, modal });

  let unsubscribe: (() => void) | undefined = session.store.subscribe((next) => {
    snapshot = next;
    if (modal?.kind === 'instruments' && modal.root !== next.project?.root) {
      modal = null;
      emit();
    }
    scheduleSync();
  });

  const exportAvailable = () => ready && snapshot.project !== null && snapshot.active !== null;

  /** One send per microtask, only of a changed value, as a Svelte effect would flush it. */
  function scheduleSync() {
    if (syncPending) return;
    syncPending = true;
    queueMicrotask(() => {
      syncPending = false;
      // Unset until the menu attaches; stop() clears it.
      if (unlistenMenu === undefined) return;
      const available = exportAvailable();
      if (available === lastSent) return;
      lastSent = available;
      void menu.setExportAvailable(available).catch(console.error);
    });
  }

  function gate(): Gate {
    return {
      quitting,
      modal: modal !== null,
      busy: projectActions.isBusy(),
      ready,
      project: snapshot.project !== null,
      active: snapshot.active !== null,
      naming: get(projectActions.folder) !== null,
    };
  }
  const can = (action: ShellAction) => allowed(action, gate());

  function setModal(next: Modal | null) {
    modal = next;
    emit();
  }

  function setQuitting(next: boolean) {
    quitting = next;
    emit();
  }

  function projectAction(action: ProjectAction) {
    if (can(action)) void projectActions.run(action);
  }

  function openSettings(opener: HTMLElement | null) {
    if (can('settings')) setModal({ kind: 'settings', restore: opener });
  }

  function openExport() {
    if (!can('exportAudio')) return;
    const song = captureExportSong(snapshot, session.activeText());
    if (song) {
      setModal({
        kind: 'export',
        song,
        controller: createExportDialog(song, exportApi),
        restore: null,
      });
    }
  }

  function onMenu(action: ProjectMenuAction) {
    if (action === 'settings') openSettings(deps.activeElement());
    else if (action === 'exportAudio') openExport();
    else projectAction(action);
  }

  function canCloseWindow(): boolean {
    return !quitting && (modal?.kind === 'export' ? modal.controller.canClose() : true);
  }

  async function onQuit(id: number): Promise<void> {
    if (quitting) {
      await quit.reply(id, false);
      return;
    }
    setQuitting(true);
    const controller = modal?.kind === 'export' ? modal.controller : undefined;
    let approved = false;
    try {
      if (!controller || (await controller.requestQuit())) {
        approved = await session.guardShutdown();
      }
    } finally {
      try {
        await quit.reply(id, approved);
      } finally {
        controller?.finishQuit();
        setQuitting(false);
      }
    }
  }

  function stop() {
    if (stopped) return;
    stopped = true;
    unlistenMenu?.();
    unlistenMenu = undefined;
    unlistenQuit?.();
    unlistenQuit = undefined;
  }

  return {
    store: { subscribe: state.subscribe },
    get quitting() {
      return quitting;
    },

    start() {
      if (started) return stop;
      started = true;
      // Reopen the last project before allowing the menu or shortcut to switch it.
      void session
        .init(canCloseWindow)
        .then(async () => {
          if (stopped || !snapshot.available) return;
          ready = true;
          emit();
          const stopMenu = await menu.listen(onMenu);
          if (stopped) {
            stopMenu();
            return;
          }
          unlistenMenu = stopMenu;
          scheduleSync();
          const stopQuit = await quit.listen((id) => {
            if (!stopped) void onQuit(id).catch(console.error);
          });
          if (stopped) stopQuit();
          else unlistenQuit = stopQuit;
        })
        .catch(console.error);
      return stop;
    },

    menu: onMenu,

    shortcut(kind) {
      if (kind === 'openProject') {
        projectAction('openProject');
        return false;
      }
      return can('newSong');
    },

    projectAction,

    openRecent(path) {
      if (can('openRecent')) void projectActions.openRecent(path);
    },

    openSettings,

    openInstruments(opener) {
      if (!can('instruments') || !snapshot.project) return;
      setModal({
        kind: 'instruments',
        root: snapshot.project.root,
        selection: session.instrumentSelection(),
        restore: opener,
      });
    },

    close(kind, applied = false) {
      const open = modal;
      // Applying an instrument always focuses the editor, even when the browser was already
      // dropped by a root change (as the old closeInstruments(true) did).
      if (open?.kind !== kind) return kind === 'instruments' && applied ? 'editor' : null;
      setModal(null);
      return applied ? 'editor' : open.restore;
    },

    canCloseWindow,
    onQuit,

    destroy() {
      stop();
      unsubscribe?.();
      unsubscribe = undefined;
    },
  };
}

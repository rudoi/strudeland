import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { message, open } from '@tauri-apps/plugin-dialog';
import { CommandError, narrowKind, toCommandError } from '../ipc/command-error';
import { invokeCommand } from '../ipc/invoke';
import type { InstrumentSnapshot } from '../strudel/instruments';

// The OS seam of the project feature. Everything that talks to Tauri (invoke, events, dialogs,
// the window) lives in `createTauriProjectApi`; `session.ts` only sees the `ProjectApi` type, so it
// runs under `bun test` with a fake.

/** Mirrors the Rust `SongEntry` (camelCase over IPC). */
export interface SongEntry {
  /** "songs/x.js": relative to the project root, forward slashes. */
  path: string;
}
/** Mirrors the Rust `ProjectInfo`. `root` is the canonical path. */
export interface ProjectInfo {
  root: string;
  name: string;
  songs: SongEntry[];
}
export interface InstrumentEntry {
  id: string;
  fingerprint: string;
}

/** A null root identifies the global library copy. */
export interface InstrumentVersion extends InstrumentEntry {
  root: string | null;
}

export interface InstrumentBrowserApi {
  libraryEntries(): Promise<InstrumentEntry[]>;
  projectEntries(root: string): Promise<InstrumentEntry[]>;
  instrumentDefinition(version: InstrumentVersion): Promise<InstrumentSnapshot[string]>;
  instrumentPreview(version: InstrumentVersion): Promise<Uint8Array>;
  writeInstrumentPin(
    root: string,
    id: string,
    libraryFingerprint: string,
    pinFingerprint: string | null,
  ): Promise<InstrumentEntry>;
}

export interface RecentProject {
  path: string;
  name: string;
}
/** Payload of the `project://changed` event. Each list is sorted. */
export interface ProjectChange {
  /** The watched project's root (same string as `ProjectInfo.root`). */
  root: string;
  songs: { changed: string[]; added: string[]; removed: string[] };
  manifest: boolean;
  /** True iff a memo `.wav` or `.json` under `recordings/` changed. */
  recordings: boolean;
  instruments: boolean;
}
const PROJECT_ERROR_KINDS = [
  'exists',
  'notFound',
  'invalidName',
  'projectChanged',
  'other',
] as const;
export type ProjectErrorKind = (typeof PROJECT_ERROR_KINDS)[number];

/** The structured rejection of the song commands that create, rename and delete. */
export class ProjectError extends CommandError<ProjectErrorKind> {
  constructor(message: string, kind: ProjectErrorKind, options?: ErrorOptions) {
    super(message, kind, options);
    this.name = 'ProjectError';
  }
}

export type UnsavedChoice = 'save' | 'discard' | 'cancel';

/**
 * The exact rejection of the root-taking commands when `root` is not the open project (it was
 * switched meanwhile). A caller with a stale project drops it silently.
 */
export const PROJECT_CHANGED = 'the project changed';

/**
 * The song and info commands take `root`, the `ProjectInfo.root` of the project the caller
 * believes is open, and reject with `PROJECT_CHANGED` when it is not.
 */
export interface ProjectApi extends InstrumentBrowserApi {
  create(dir: string, name: string): Promise<ProjectInfo>;
  open(dir: string): Promise<ProjectInfo>;
  /**
   * Re-reads project.json of the open project (no watcher restart, recents untouched). Rejects with
   * a `CommandError` if the manifest is invalid.
   */
  info(root: string): Promise<ProjectInfo>;
  instruments(root: string, pin?: InstrumentEntry): Promise<InstrumentSnapshot>;
  last(): Promise<ProjectInfo | null>;
  recents(): Promise<RecentProject[]>;
  /** Sorted byte-wise by path. */
  listSongs(root: string): Promise<SongEntry[]>;
  readSong(root: string, path: string): Promise<string>;
  saveSong(root: string, path: string, contents: string): Promise<void>;
  /** Rejects with a `ProjectError`. */
  createSong(root: string, stem: string): Promise<string>;
  /** Rejects with a `ProjectError`. Resolves to the new relative path. */
  renameSong(root: string, path: string, stem: string): Promise<string>;
  /** Rejects with a `ProjectError`. */
  deleteSong(root: string, path: string): Promise<void>;
  /**
   * The canonical root of the project folder `dir`, as `ProjectInfo.root` spells it. Opens
   * nothing. Rejects with a `CommandError` if `dir` is not a project.
   */
  projectRootOf(dir: string): Promise<string>;
  /** Returns an unlisten function. */
  onChange(handler: (change: ProjectChange) => void): Promise<() => void>;
  /** Native folder picker; null when cancelled. */
  pickFolder(title: string): Promise<string | null>;
  /** The three-way prompt; `count` is the number of dirty songs. */
  confirmUnsaved(count: number): Promise<UnsavedChoice>;
  setWindowTitle(title: string): Promise<void>;
  /**
   * The handler may be async; the window closes after it resolves unless it called
   * `preventDefault()`. Returns an unlisten function.
   */
  onCloseRequested(
    handler: (e: { preventDefault(): void }) => void | Promise<void>,
  ): Promise<() => void>;
}

export const PROJECT_CHANGED_EVENT = 'project://changed';
export const PROJECT_MENU_EVENT = 'project://menu';
export type ProjectAction = 'newProject' | 'openProject';
export type ProjectMenuAction = ProjectAction | 'exportAudio' | 'settings';

export function isProjectMenuAction(payload: unknown): payload is ProjectMenuAction {
  return (
    payload === 'newProject' ||
    payload === 'openProject' ||
    payload === 'exportAudio' ||
    payload === 'settings'
  );
}

export function onProjectMenu(handler: (action: ProjectMenuAction) => void): Promise<() => void> {
  return listen<unknown>(
    PROJECT_MENU_EVENT,
    ({ payload }) => {
      if (isProjectMenuAction(payload)) handler(payload);
    },
    { target: 'main' },
  );
}

/** Button labels of the unsaved-changes prompt. */
export const UNSAVED_BUTTONS = { yes: 'Save all', no: 'Discard', cancel: 'Cancel' } as const;

export function unsavedMessage(count: number): string {
  return count === 1 ? '1 song has unsaved changes.' : `${count} songs have unsaved changes.`;
}

/** The pressed button's label to a choice. Anything else (a dismissed dialog) is `'cancel'`. */
export function unsavedChoice(result: unknown): UnsavedChoice {
  if (result === UNSAVED_BUTTONS.yes) return 'save';
  if (result === UNSAVED_BUTTONS.no) return 'discard';
  return 'cancel';
}

/** The folder picker's result: one directory, else null (cancelled, or an unexpected shape). */
export function pickedFolder(result: unknown): string | null {
  if (typeof result === 'string') return result;
  if (Array.isArray(result) && typeof result[0] === 'string') return result[0];
  return null;
}

/**
 * A rejection of a song command to a `ProjectError`: the Rust `CommandError` (`{ kind, message }`)
 * with its kind narrowed to the song kinds (any other is `other`), else whatever was thrown as
 * text with kind `other`.
 */
export function toProjectError(e: unknown): ProjectError {
  if (e instanceof ProjectError) return e;
  const c = toCommandError(e);
  return new ProjectError(c.message, narrowKind(c.kind, PROJECT_ERROR_KINDS, 'other'), {
    cause: c.cause,
  });
}

async function songInvoke(cmd: string, args: Record<string, unknown>): Promise<unknown> {
  try {
    return await invokeCommand(cmd, args);
  } catch (e) {
    throw toProjectError(e);
  }
}

/** The real implementation (invoke, listen, dialog plugin, getCurrentWindow). */
export function createTauriProjectApi(): ProjectApi {
  // Invoke argument keys are camelCase in JS; Tauri maps them to the Rust snake_case names.
  return {
    create: (dir, name) => invokeCommand('project_create', { dir, name }) as Promise<ProjectInfo>,
    open: (dir) => invokeCommand('project_open', { dir }) as Promise<ProjectInfo>,
    info: (root) => invokeCommand('project_info', { root }) as Promise<ProjectInfo>,
    instruments: (root, pin) =>
      invokeCommand('project_instruments', { root, pin }) as Promise<InstrumentSnapshot>,
    libraryEntries: () => invokeCommand('instrument_library_entries') as Promise<InstrumentEntry[]>,
    projectEntries: (root) =>
      invokeCommand('project_instrument_entries', { root }) as Promise<InstrumentEntry[]>,
    instrumentDefinition: (version) =>
      invokeCommand('instrument_definition', {
        root: version.root,
        id: version.id,
        fingerprint: version.fingerprint,
      }) as Promise<InstrumentSnapshot[string]>,
    instrumentPreview: async (version) =>
      new Uint8Array(
        (await invokeCommand('instrument_preview', {
          root: version.root,
          id: version.id,
          fingerprint: version.fingerprint,
        })) as ArrayBuffer,
      ),
    writeInstrumentPin: (root, id, libraryFingerprint, pinFingerprint) =>
      invokeCommand('project_write_instrument_pin', {
        root,
        id,
        libraryFingerprint,
        pinFingerprint,
      }) as Promise<InstrumentEntry>,
    last: () => invokeCommand('project_last') as Promise<ProjectInfo | null>,
    recents: () => invokeCommand('recent_list') as Promise<RecentProject[]>,
    listSongs: (root) => invokeCommand('song_list', { root }) as Promise<SongEntry[]>,
    readSong: (root, path) => invokeCommand('song_read', { root, path }) as Promise<string>,
    saveSong: (root, path, contents) =>
      invokeCommand('song_save', { root, path, contents }) as Promise<void>,
    createSong: (root, stem) => songInvoke('song_create', { root, stem }) as Promise<string>,
    renameSong: (root, path, stem) =>
      songInvoke('song_rename', { root, path, stem }) as Promise<string>,
    deleteSong: (root, path) => songInvoke('song_delete', { root, path }) as Promise<void>,
    projectRootOf: (dir) => invokeCommand('project_root_of', { dir }) as Promise<string>,
    onChange: (handler) =>
      listen<ProjectChange>(PROJECT_CHANGED_EVENT, (event) => handler(event.payload)),
    pickFolder: async (title) => pickedFolder(await open({ directory: true, title })),
    confirmUnsaved: async (count) =>
      unsavedChoice(
        await message(unsavedMessage(count), {
          title: 'Unsaved changes',
          kind: 'warning',
          buttons: { ...UNSAVED_BUTTONS },
        }),
      ),
    setWindowTitle: (title) => getCurrentWindow().setTitle(title),
    onCloseRequested: (handler) => getCurrentWindow().onCloseRequested(handler),
  };
}

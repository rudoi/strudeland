import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { message, open } from '@tauri-apps/plugin-dialog';
import { kindedError } from '../command-error';
import { errorMessage } from '../errors';

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
}
export type ProjectErrorKind =
  'songExists' | 'songNotFound' | 'invalidName' | 'projectChanged' | 'other';

/** The structured rejection of the song commands that create, rename and delete. */
export class ProjectError extends Error {
  constructor(
    message: string,
    readonly kind: ProjectErrorKind,
  ) {
    super(message);
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
export interface ProjectApi {
  create(dir: string, name: string): Promise<ProjectInfo>;
  open(dir: string): Promise<ProjectInfo>;
  /**
   * Re-reads project.json of the open project (no watcher restart, recents untouched). Rejects with
   * the error string if the manifest is invalid.
   */
  info(root: string): Promise<ProjectInfo>;
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
   * nothing. Rejects with the error string if `dir` is not a project.
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
 * A rejection of a song command to a `ProjectError`: the Rust `SongError` object
 * (`{ kind, message }`), else whatever was thrown as text with kind `other`.
 */
export function toProjectError(e: unknown): ProjectError {
  if (e instanceof ProjectError) return e;
  const k = kindedError(e);
  if (k && typeof k.kind === 'string')
    return new ProjectError(k.message, k.kind as ProjectErrorKind);
  return new ProjectError(errorMessage(e), 'other');
}

async function songInvoke<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toProjectError(e);
  }
}

/** The real implementation (invoke, listen, dialog plugin, getCurrentWindow). */
export function createTauriProjectApi(): ProjectApi {
  // Invoke argument keys are camelCase in JS; Tauri maps them to the Rust snake_case names.
  return {
    create: (dir, name) => invoke<ProjectInfo>('project_create', { dir, name }),
    open: (dir) => invoke<ProjectInfo>('project_open', { dir }),
    info: (root) => invoke<ProjectInfo>('project_info', { root }),
    last: () => invoke<ProjectInfo | null>('project_last'),
    recents: () => invoke<RecentProject[]>('recent_list'),
    listSongs: (root) => invoke<SongEntry[]>('song_list', { root }),
    readSong: (root, path) => invoke<string>('song_read', { root, path }),
    saveSong: (root, path, contents) => invoke<void>('song_save', { root, path, contents }),
    createSong: (root, stem) => songInvoke<string>('song_create', { root, stem }),
    renameSong: (root, path, stem) => songInvoke<string>('song_rename', { root, path, stem }),
    deleteSong: (root, path) => songInvoke<void>('song_delete', { root, path }),
    projectRootOf: (dir) => invoke<string>('project_root_of', { dir }),
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

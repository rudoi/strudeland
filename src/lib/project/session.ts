import { isolateHistory } from '@codemirror/commands';
import { Text, type ChangeSpec, type EditorState, type TransactionSpec } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { get, writable, type Readable } from 'svelte/store';
import type { EvalOutcome } from '../control/protocol';
import { instrumentApplication } from '../editor/apply-instrument';
import { claudeEdit, hasClaudeMark } from '../editor/claude-mark';
import { formatSong } from '../editor/format-song';
import { attachState, pianoRollEditLocation, pianoRollEditHighlight } from '../editor/visuals';
import type { RollPatch, SourceSpan } from '../piano-roll/types';
import type { InstrumentSnapshot } from '../strudel/instruments';
import type { Runtime } from '../strudel/runtime';
import { sameStructure, type SongStructure } from '../strudel/structure';
import { readSongHeader, type MetaError } from './song-header';
import { setMetaValue, type MetaPath } from './meta-write';
import { stripSongExt } from './ui';
import {
  PROJECT_CHANGED,
  ProjectError,
  type ProjectApi,
  type ProjectChange,
  type ProjectInfo,
  type RecentProject,
  type UnsavedChoice,
} from './api';
import { errorMessage } from '../errors';

/** Why a failed evaluate failed: the song header, the instrument pins, or the evaluate itself. */
export type EvalErrorKind = 'meta' | 'instruments' | 'eval';

/** The wire outcome plus its failure kind (null exactly when `ok`); never sent over the socket. */
export type SessionEvalOutcome = EvalOutcome & { errorKind: EvalErrorKind | null };

export interface SongItem {
  /** "songs/x.js" */
  path: string;
  /** meta.title, else the file name without ".js". */
  title: string;
  dirty: boolean;
  conflict: 'changed' | 'deleted' | null;
}

export interface SessionState {
  /** Projects need the desktop app; false in a plain browser. */
  available: boolean;
  /** The recent projects (already pruned by the backend); loaded in `init`, refreshed after an open. */
  recents: RecentProject[];
  project: ProjectInfo | null;
  /** Same order as the disk listing (sorted by file name, byte-wise). */
  songs: SongItem[];
  /** A path in `songs`, or null (no project, or zero songs). */
  active: string | null;
  /** Last failed operation, shown in the status bar. */
  error: string | null;
  /** MetaError message for the active song, with line:column. */
  metaError: string | null;
  /** The active song's structure, read from its header as typed; null without an active song. */
  structure: SongStructure | null;
  /** The song the last successful evaluate played (kept on Stop, cleared on project change). */
  evaluated: { path: string; title: string } | null;
  /** Derived: the active song still shows the mark of an external (Claude) edit. */
  notice: { kind: 'claude-edit'; path: string; title: string } | null;
}

export interface ExternalEditResult {
  changed: boolean;
  /** Read from the song after the edit. */
  dirty: boolean;
  conflict: 'changed' | 'deleted' | null;
}

export type RenameOutcome =
  | { ok: true; from: string; to: string }
  | { ok: false; reason: 'missing' | 'deleted' | 'projectChanged' };
export type DeleteOutcome =
  { ok: true; stopped: boolean } | { ok: false; reason: 'missing' | 'unsaved' | 'projectChanged' };
export type SaveTarget = { path: string } | { all: true };
export type SaveOutcome =
  | { ok: true; saved: string[] }
  | { ok: false; reason: 'missing' | 'projectChanged' }
  | { ok: false; reason: 'failed'; saved: string[]; failedPath: string; error: string };
export type OpenOutcome =
  | { ok: true; alreadyOpen: boolean }
  | { ok: false; reason: 'unsaved'; dirty: string[] }
  | { ok: false; reason: 'busy' }
  | { ok: false; reason: 'failed'; error: string };

export interface InstrumentSelection {
  root: string;
  song: string;
  from: number;
  to: number;
  expect: string;
}

export interface PianoRollSnapshot {
  root: string;
  path: string;
  revision: number;
  selectionRevision: number;
  sourceRevision: number;
  selection: SourceSpan | null;
  code: string;
}

export interface PianoRollCommit {
  snapshot: PianoRollSnapshot;
  target?: SourceSpan;
  targetCaret?: number;
  spans: readonly SourceSpan[];
}

export type InstrumentApplyRequest = Omit<InstrumentSelection, 'song'> & {
  id: string;
  song?: string;
  pinFingerprint?: string;
};

export interface InstrumentApplyResult {
  song: string;
  from: number;
  to: number;
}

export class InstrumentApplyError extends Error {
  constructor(
    readonly kind: 'project' | 'song' | 'stale' | 'invalid',
    message: string,
  ) {
    super(message);
  }
}

export interface Session {
  readonly store: Readable<SessionState>;
  readonly pianoRollRevision: Readable<number>;
  pianoRollChanged(selected?: boolean): void;
  pianoRollSnapshot(): PianoRollSnapshot | null;
  pianoRollMatchesLivePins(): boolean;
  editPianoRoll(
    snapshot: PianoRollSnapshot,
    patches: readonly RollPatch[],
    target?: SourceSpan,
  ): PianoRollCommit | null;
  clearPianoRollFeedback(): void;
  revealPianoRoll(snapshot: PianoRollSnapshot, span: SourceSpan): void;
  /** Reopens the last project and installs the window close guard. */
  init(canCloseWindow?: () => boolean): Promise<void>;
  /** Show the active song in `view` (or detach with undefined). The live state moves with it. */
  attachView(view: EditorView | undefined): void;
  /** Pick a folder, collect a name (null to cancel), then create and open the project. */
  newProject(askName: (folder: string) => Promise<string | null>): Promise<void>;
  /** pickFolder, then open. */
  openProject(): Promise<void>;
  openRecent(path: string): Promise<void>;
  switchSong(path: string): Promise<void>;
  /**
   * Create, add to the list, switch to it. Resolves to the new path, or null when the project
   * changed meanwhile. Rejects with the API's `ProjectError` (e.g. an invalid name) so the UI can
   * show it inline, or an Error without a project; `error` is not set for this.
   */
  createSong(stem: string): Promise<string | null>;
  /**
   * Rename a song on disk and move its editor state (text, undo history, marks, dirty buffer,
   * conflict) to the new path; `active` and `evaluated` follow. Does not switch to it. Rejects with
   * the API's `ProjectError` (`songExists`, `invalidName`, `songNotFound`).
   */
  renameSong(path: string, stem: string): Promise<RenameOutcome>;
  /**
   * Delete a song from disk and drop it; stops playback if it was the evaluated song. `unsaved`
   * when it needs saving and `discard` is false. A file already gone on disk counts as deleted.
   * Rejects with the API's `ProjectError`.
   */
  deleteSong(path: string, opts: { discard: boolean }): Promise<DeleteOutcome>;
  /** Save one song or every song that needs saving, without prompting. `save()` is unchanged. */
  saveSongs(target: SaveTarget): Promise<SaveOutcome>;
  /**
   * Open the project at `path` without a modal: `busy` while a UI open is in progress, `unsaved`
   * with the songs that need saving (dirty, or a `deleted` conflict) unless `discard`, `alreadyOpen` (nothing touched) for the open project. A
   * failed open leaves everything as it was and does not set `error`.
   */
  openPath(path: string, opts: { discard: boolean }): Promise<OpenOutcome>;
  /** Active song; false and `error` set on failure. False without an active song. */
  save(): Promise<boolean>;
  /** Cmd-Enter: save if dirty, then evaluate the captured source with any save formatting. */
  saveAndEvaluate(): Promise<void>;
  /** Resolve a `changed` conflict by keeping the editor text. Defaults to the active song. */
  keepMine(path?: string): void;
  /** Resolve a `changed` conflict by taking the disk text (undoable). Defaults to the active song. */
  takeDisk(path?: string): void;
  /** "Save to recreate" for the active song's `deleted` conflict. */
  recreateDeleted(): Promise<void>;
  /** "Close" for the active song's `deleted` conflict: drop it and switch away. */
  closeDeleted(): Promise<void>;
  /** The editor calls this when the shown doc changed. */
  docChanged(): void;
  /**
   * Set a `meta` header value of the active song as one undoable edit (marks it dirty; not saved or
   * evaluated). True iff it dispatched: false without an active song, when its text is no longer
   * `seenDoc`, or when the header cannot be edited safely.
   */
  applyMeta(path: MetaPath, value: number | string, seenDoc: string): boolean;
  /**
   * Replace the active song's text with `text` as one isolated, marked undo step (marks it dirty; not
   * saved or evaluated). `'missing'` unless `path` is the active song with an editor state; `'stale'`
   * when `opts.expect` differs from its text. Identical text changes nothing. `text` and `expect`
   * are normalized to LF.
   */
  externalEdit(
    path: string,
    text: string,
    opts?: { expect?: string },
  ): ExternalEditResult | 'stale' | 'missing';
  /**
   * Evaluate a song's current editor text without saving it. Never throws. `autostart: false`
   * installs the pattern without starting playback (`Runtime.start` starts it).
   */
  evaluateBuffer(path: string, opts?: { autostart?: boolean }): Promise<SessionEvalOutcome>;
  /** Loads the song if needed (ensureLoaded) without showing it; its live text + dirty, or null if it cannot be read. */
  peekSong(path: string): Promise<{ text: string; dirty: boolean } | null>;
  /** Snapshot a usable selection before focus moves to the browser. */
  instrumentSelection(): InstrumentSelection | null;
  /** Wrap an expression in the active buffer as one undo step. */
  applyInstrument(request: InstrumentApplyRequest): Promise<InstrumentApplyResult>;
  /** The active song's editor text, or null without one. */
  activeText(): string | null;
  /** True if the caller may proceed (no dirty songs, discard, or every save succeeded). */
  guardUnsaved(): Promise<boolean>;
  /** Allows one unsaved decision at a time across window close and application Quit. */
  guardShutdown(): Promise<boolean>;
}

export interface SessionDeps {
  api: ProjectApi;
  runtime: Runtime;
  /** setup.ts `createEditorState`, bound to the runtime and hooks. */
  createState: (doc: string) => EditorState;
  isTauri: () => boolean;
  /** Whether a save should format eligible mini-notation in the editor. */
  formatOnSave?: () => boolean;
  /**
   * Awaited before another project is opened or created while one is open (after the unsaved
   * guard), so whatever belongs to the open project (a recording take) can finish first. Errors
   * are logged; the switch goes on.
   */
  beforeProjectChange?: () => Promise<void>;
  /** Applied to every state `show()` attaches (a song's stashed state or the blank one). */
  prepareState?: (state: EditorState) => EditorState;
  /** Called at the end of `show()` after a state was attached (not on its early return). */
  onShown?: (view: EditorView) => void;
}

/** What the song list shows for a song, cached until its doc or disk text changes. */
interface Derived {
  /** The doc this was computed from; null for a song without an editor state. */
  doc: Text | null;
  title: string;
  dirty: boolean;
  /** The header's error, or null when it reads cleanly. */
  headerError: MetaError | null;
  /** From the header; `buildStructure({})` when it has a MetaError. */
  structure: SongStructure;
}

/** Per-song memory. */
interface Song {
  /** Last text read from or written to disk; null if it could not be read yet. */
  diskText: string | null;
  /** `diskText` as a Text, built on first dirty check. */
  diskDoc: Text | null;
  /** Created on first switch. While the song is shown, the view's state is the live one. */
  editorState: EditorState | null;
  /** The disk text that raised a `changed` conflict. */
  diskPending: string | null;
  conflict: 'changed' | 'deleted' | null;
  derived: Derived | null;
}

const DEFAULT_TITLE = 'strudeland';

/** A promise and the function that resolves it. */
function settled(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
}

/** An error (or any thrown value) as a message string. */
export { errorMessage };

/** "songs/main.js" to "main". */
export const fileStem = (path: string): string =>
  stripSongExt(path.slice(path.lastIndexOf('/') + 1));

const titleFrom = (path: string, title: unknown): string =>
  typeof title === 'string' && title.trim() ? title : fileStem(path);

/** CRLF and lone CR line breaks to LF. */
export const toLF = (text: string): string => text.replace(/\r\n?/g, '\n');

const newSong = (diskText: string | null): Song => ({
  diskText,
  diskDoc: null,
  editorState: null,
  diskPending: null,
  conflict: null,
  derived: null,
});

/** The disk now holds `text` and the song agrees with it: no pending text, no conflict. */
function settle(song: Song, text: string): void {
  song.diskText = text;
  song.diskDoc = null;
  song.diskPending = null;
  song.conflict = null;
  song.derived = null;
}

/** meta.title, else the file name without ".js". */
export function songTitle(path: string, code: string | null): string {
  const header = code === null ? null : readSongHeader(code);
  return titleFrom(path, header?.ok ? header.title : undefined);
}

/** The window title for a session state. */
export function windowTitle(state: SessionState): string {
  if (!state.project) return DEFAULT_TITLE;
  const item = state.songs.find((s) => s.path === state.active);
  if (!item) return state.project.name;
  return `${state.project.name} — ${item.title}${item.dirty ? ' ●' : ''}`;
}

/** The smallest single change turning `from` into `to` (common prefix and suffix kept). */
export function replaceChange(
  from: string,
  to: string,
): { from: number; to: number; insert: string } {
  let start = 0;
  const max = Math.min(from.length, to.length);
  while (start < max && from.charCodeAt(start) === to.charCodeAt(start)) start++;
  let end = 0;
  while (
    end < max - start &&
    from.charCodeAt(from.length - 1 - end) === to.charCodeAt(to.length - 1 - end)
  ) {
    end++;
  }
  return { from: start, to: from.length - end, insert: to.slice(start, to.length - end) };
}

/** Above this many cells (changed old lines × changed new lines), `lineChanges` does not diff. */
const MAX_DIFF_CELLS = 4_000_000;

/** `text` as lines, each with its `\n` (the last one may lack it); `''` has none. */
function splitLines(text: string): string[] {
  return text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
}

/** The start offset of each line, then the text's length. */
function lineStarts(lines: readonly string[]): number[] {
  const out = [0];
  for (const l of lines) out.push(out[out.length - 1] + l.length);
  return out;
}

/**
 * The changes turning `from` into `to`, one per run of changed lines (an LCS over lines), each
 * trimmed to its changed characters, so edits far apart stay separate. Falls back to one
 * `replaceChange` when the changed middle is too large to diff (`MAX_DIFF_CELLS`).
 */
export function lineChanges(
  from: string,
  to: string,
): { from: number; to: number; insert: string }[] {
  if (from === to) return [];
  const a = splitLines(from);
  const b = splitLines(to);
  let p = 0;
  while (p < a.length && p < b.length && a[p] === b[p]) p++;
  let q = 0;
  while (q < a.length - p && q < b.length - p && a[a.length - 1 - q] === b[b.length - 1 - q]) q++;
  const n = a.length - p - q;
  const m = b.length - p - q;
  if (n * m > MAX_DIFF_CELLS) return [replaceChange(from, to)];
  // lcs[i * w + j]: the LCS length of a[p+i..] and b[p+j..] (at most 2000 here, so 16 bits do).
  const w = m + 1;
  const lcs = new Uint16Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * w + j] =
        a[p + i] === b[p + j]
          ? lcs[(i + 1) * w + j + 1] + 1
          : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    }
  }
  const startA = lineStarts(a);
  const startB = lineStarts(b);
  const out: { from: number; to: number; insert: string }[] = [];
  let hunk: [number, number] | null = null; // where the current run of changed lines started
  const close = (i: number, j: number) => {
    if (!hunk) return;
    const fa = startA[p + hunk[0]];
    const c = replaceChange(
      from.slice(fa, startA[p + i]),
      to.slice(startB[p + hunk[1]], startB[p + j]),
    );
    if (c.from !== c.to || c.insert)
      out.push({ from: fa + c.from, to: fa + c.to, insert: c.insert });
    hunk = null;
  };
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[p + i] === b[p + j]) {
      close(i, j);
      i++;
      j++;
      continue;
    }
    hunk ??= [i, j];
    if (j >= m || (i < n && lcs[(i + 1) * w + j] >= lcs[i * w + j + 1])) i++;
    else j++;
  }
  close(n, m);
  return out;
}

/**
 * `known` in the order of the disk listing `listed`. Known paths the listing lacks (songs kept
 * with a `deleted` conflict) stay right after the path they followed before; listed paths that
 * are not known are left out.
 */
export function mergeOrder(listed: readonly string[], known: readonly string[]): string[] {
  const knownSet = new Set(known);
  const out = listed.filter((p) => knownSet.has(p));
  const placed = new Set(out);
  let prev: string | null = null;
  for (const path of known) {
    if (!placed.has(path)) {
      out.splice(prev === null ? 0 : out.indexOf(prev) + 1, 0, path);
      placed.add(path);
    }
    prev = path;
  }
  return out;
}

/**
 * Whether a control-socket `code.set` with `eval: "auto"` evaluates: only while the edited song is
 * the one playing. Stricter than a hot reload, which lets the active song take playback over.
 */
export function shouldAutoEval(
  started: boolean,
  evaluatedPath: string | null,
  path: string,
): boolean {
  return started && evaluatedPath === path;
}

const sameNotice = (a: SessionState['notice'], b: SessionState['notice']): boolean =>
  a === b || (a !== null && b !== null && a.path === b.path && a.title === b.title);

const sameItem = (a: SongItem, b: SongItem): boolean =>
  a.path === b.path && a.title === b.title && a.dirty === b.dirty && a.conflict === b.conflict;

function sameState(a: SessionState, b: SessionState): boolean {
  return (
    a.available === b.available &&
    a.recents === b.recents &&
    a.project === b.project &&
    a.active === b.active &&
    a.error === b.error &&
    a.metaError === b.metaError &&
    a.structure === b.structure &&
    a.evaluated === b.evaluated &&
    sameNotice(a.notice, b.notice) &&
    a.songs.length === b.songs.length &&
    a.songs.every((s, i) => sameItem(s, b.songs[i]))
  );
}

export function createSession(deps: SessionDeps): Session {
  const { api, runtime, createState, isTauri } = deps;

  const available = isTauri();
  let recents: RecentProject[] = [];
  let project: ProjectInfo | null = null;
  let instruments: InstrumentSnapshot | undefined;
  let instrumentError: string | null = null;
  let order: string[] = [];
  let songs = new Map<string, Song>();
  let active: string | null = null;
  let error: string | null = null;
  /**
   * What the last successful evaluate played: the published `song` (identity-stable, so
   * `sameState` sees no change until it moves) and the text's `meta.title`, to retitle it on a rename.
   */
  let evaluated: {
    song: NonNullable<SessionState['evaluated']>;
    metaTitle: unknown;
    instruments: InstrumentSnapshot | undefined;
  } | null = null;
  /**
   * The open project's generation, unique per open attempt (`nextGeneration`). Results and change
   * events that belong to an older one are dropped.
   */
  let generation = 0;
  let nextGeneration = 0;
  /**
   * While an open waits for `project_open`: the generation it will install. The new project's
   * watcher already runs then, so events arriving meanwhile belong to it.
   */
  let opening: number | null = null;
  let watching = false;
  let closeHookRegistered = false;
  /**
   * Paths being written right now: the `text` written in generation `gen`, so its watcher echo is
   * ignored even before the save resolves (only in that generation), and `done`, which settles when
   * the write ends (a write of an older project is still waited for).
   */
  const inflight = new Map<string, { text: string; gen: number; done: Promise<void> }>();
  /**
   * Paths being renamed or deleted right now, to a promise that settles once the song has moved or
   * been dropped. A write waits for it, so a Cmd-S cannot recreate the old file.
   */
  const moving = new Map<string, Promise<void>>();
  /** UI opens in progress (picker, unsaved modal, load); a control open meanwhile is `busy`. */
  let uiOpening = 0;

  let view: EditorView | undefined;
  /** The song whose state the view holds (its live state), or null. */
  let shown: string | null = null;
  /** The view shows no song of ours (a blank state, or the one it was created with). */
  let blanked = false;
  let rollRevision = 0;
  let rollSelectionRevision = 0;
  let rollSourceRevision = 0;
  const pianoRollRevision = writable(rollRevision);
  function pianoRollChanged(selected = false, sourceChanged = !selected): void {
    if (selected) rollSelectionRevision++;
    if (sourceChanged) rollSourceRevision++;
    pianoRollRevision.set(++rollRevision);
  }

  function matchesPianoRoll(snapshot: PianoRollSnapshot): boolean {
    return (
      snapshot.revision === rollRevision &&
      snapshot.root === project?.root &&
      snapshot.path === active &&
      active === shown &&
      !opening &&
      uiOpening === 0 &&
      songs.get(snapshot.path)?.conflict !== 'deleted'
    );
  }

  function pianoRollSnapshot(): PianoRollSnapshot | null {
    if (!view || !project || !active || shown !== active) return null;
    const { doc, selection } = view.state;
    const { from, to } = selection.main;
    return {
      root: project.root,
      path: active,
      revision: rollRevision,
      selectionRevision: rollSelectionRevision,
      sourceRevision: rollSourceRevision,
      selection: selection.ranges.length === 1 ? { from, to } : null,
      code: doc.toString(),
    };
  }

  // ---- view ----

  /** Move the live state of the shown song back into its stash. */
  function stash(): void {
    if (shown !== null && view) {
      const song = songs.get(shown);
      if (song) song.editorState = view.state;
    }
    shown = null;
  }

  /** Make the view show the active song (stashing whatever song it showed before). */
  function show(): void {
    if (!view || (shown !== null && shown === active)) return;
    stash();
    const state = active ? songs.get(active)?.editorState : null;
    const prepare = deps.prepareState ?? ((s: EditorState) => s);
    if (active && state) {
      attachState(view, prepare(state));
      shown = active;
      blanked = false;
    } else if (!blanked) {
      // Nothing to show: drop the previous song (and its undo history) from the hidden editor.
      attachState(view, prepare(createState('')));
      blanked = true;
    } else return;
    pianoRollChanged(true, true);
    deps.onShown?.(view);
  }

  // ---- reads (never touch the view) ----

  /** The live state of a song: the view's for the shown song, else the stash. */
  function stateOf(path: string): EditorState | null {
    if (view && shown === path) return view.state;
    return songs.get(path)?.editorState ?? null;
  }

  /** Current editor text of a song, or its disk text if it has no editor state yet. */
  function textOf(path: string): string | null {
    const st = stateOf(path);
    return st ? st.doc.toString() : (songs.get(path)?.diskText ?? null);
  }

  /** Title, dirty and headerError of a song; recomputed only when its doc or disk text changed. */
  function derive(path: string): Derived {
    const song = songs.get(path)!;
    const doc = stateOf(path)?.doc ?? null;
    if (song.derived && song.derived.doc === doc) return song.derived;
    let dirty = false;
    if (doc) {
      if (song.diskText === null) dirty = true;
      else {
        song.diskDoc ??= Text.of(song.diskText.split('\n'));
        dirty = doc !== song.diskDoc && !doc.eq(song.diskDoc);
      }
    }
    // A clean doc reads as the disk text, so it needs no toString.
    const text = doc && dirty ? doc.toString() : song.diskText;
    // No text reads as an empty file: the file name, no error and the default structure.
    const header = readSongHeader(text ?? '');
    const title = titleFrom(path, header.ok ? header.title : undefined);
    const headerError = header.ok ? null : header.error;
    let structure = header.structure;
    // Keep the previous object while the structure is unchanged, so typing does not republish it.
    const prev = song.derived?.structure;
    if (prev && sameStructure(prev, structure)) structure = prev;
    song.derived = { doc, title, dirty, headerError, structure };
    return song.derived;
  }

  const isDirty = (path: string): boolean => derive(path).dirty;

  // ---- store ----

  /** The notice for a song: set while its state carries a Claude-edit mark. */
  function noticeOf(path: string | null): SessionState['notice'] {
    const st = path === null ? null : stateOf(path);
    if (path === null || !st || !hasClaudeMark(st)) return null;
    return { kind: 'claude-edit', path, title: derive(path).title };
  }

  function itemOf(path: string): SongItem {
    const { title, dirty } = derive(path);
    return { path, title, dirty, conflict: songs.get(path)!.conflict };
  }

  function snapshot(): SessionState {
    return {
      available,
      recents,
      project,
      songs: order.map(itemOf),
      active,
      error,
      metaError: active ? (derive(active).headerError?.located ?? null) : null,
      structure: active ? derive(active).structure : null,
      evaluated: evaluated?.song ?? null,
      notice: noticeOf(active),
    };
  }

  let current = snapshot();
  const state = writable(current);

  function publish(next: SessionState): void {
    if (sameState(current, next)) return;
    current = next;
    state.set(next);
  }

  const emit = (): void => publish(snapshot());

  // ---- serial queue: project loads and change events run one at a time, in order ----

  let queue: Promise<unknown> = Promise.resolve();
  function serial<T>(fn: () => Promise<T>): Promise<T> {
    const run = queue.then(fn);
    queue = run.catch(() => {});
    return run;
  }

  // ---- disk ----

  /**
   * Read a song with its line breaks normalized to LF. This is the one place text arrives from
   * disk: CodeMirror turns CRLF into LF, so `diskText` must too, or a CRLF song would never be
   * clean and an outside edit would always conflict. Saves write the editor's LF text.
   */
  async function readText(root: string, path: string): Promise<string> {
    return toLF(await api.readSong(root, path));
  }

  /**
   * Optionally format and write the captured text, returning the saved source or null on failure.
   * A pending rename/delete must settle before we can edit the buffer or write the file.
   */
  async function write(path: string, text: string): Promise<string | null> {
    const gen = generation;
    const song = songs.get(path);
    const shouldFormat = deps.formatOnSave?.();
    while (moving.has(path)) await moving.get(path);
    if (!project || gen !== generation || !song || songs.get(path) !== song) return null;
    const state = shouldFormat ? stateOf(path) : null;
    if (state?.doc.toString() === text) {
      const edits = formatSong(text);
      if (edits.length > 0) {
        const formatted = apply(path, {
          changes: edits,
          userEvent: 'format',
          annotations: isolateHistory.of('full'),
        });
        text = formatted?.toString() ?? text;
      }
    }
    const done = settled();
    const entry = { text, gen, done: done.promise };
    inflight.set(path, entry);
    try {
      await api.saveSong(project.root, path, text);
    } catch (e) {
      if (gen !== generation && errorMessage(e) === PROJECT_CHANGED) return null;
      error = errorMessage(e);
      emit();
      return null;
    } finally {
      if (inflight.get(path) === entry) inflight.delete(path);
      done.resolve();
    }
    if (gen !== generation) return null;
    settle(song, text); // a save overwrites the disk and re-creates a deleted file
    error = null;
    emit();
    return text;
  }

  /** Read a song not read yet and give it an editor state. False (and `error` set) if it fails. */
  async function ensureLoaded(path: string): Promise<boolean> {
    const song = songs.get(path);
    if (!song || !project) return false;
    if (song.diskText === null) {
      const gen = generation;
      let text: string;
      try {
        text = await readText(project.root, path);
      } catch (e) {
        if (gen !== generation) return false;
        error = `${fileStem(path)}: ${errorMessage(e)}`;
        emit();
        return false;
      }
      if (gen !== generation) return false; // the project changed while it was read
      settle(song, text);
    }
    song.editorState ??= createState(song.diskText!);
    return true;
  }

  /** Make the first song that has been read active (none if there is none) and show it. */
  function activateFirstLoadable(): void {
    active = order.find((p) => songs.get(p)!.diskText !== null) ?? null;
    if (active) {
      const song = songs.get(active)!;
      song.editorState ??= createState(song.diskText!);
    }
    show();
  }

  function addSong(path: string, diskText: string | null): void {
    songs.set(path, newSong(diskText));
    order.push(path);
  }

  function dropSong(path: string): void {
    songs.delete(path);
    order = order.filter((p) => p !== path);
    if (shown === path) shown = null;
  }

  /** Put `order` in the disk listing's order. On failure the order stays and `error` is set. */
  async function reorder(): Promise<void> {
    if (!project) return;
    const gen = generation;
    let listed: string[];
    try {
      listed = (await api.listSongs(project.root)).map((s) => s.path);
    } catch (e) {
      if (gen === generation) error = `listing songs failed: ${errorMessage(e)}`;
      return;
    }
    if (gen !== generation) return;
    order = mergeOrder(
      listed,
      order.filter((p) => songs.has(p)),
    );
  }

  async function refreshRecents(): Promise<void> {
    try {
      recents = await api.recents();
    } catch (e) {
      console.error(e);
    }
  }

  // ---- projects ----

  async function install(info: ProjectInfo, gen: number): Promise<void> {
    generation = gen;
    runtime.stop(); // playback always stops when the project changes
    runtime.transport.reset();
    project = info;
    instruments = undefined;
    instrumentError = null;
    // Empty while the songs are read; `order` and `songs` are only ever swapped in together, so
    // every reader (guard, emit, a late write) sees a consistent pair meanwhile.
    songs = new Map();
    order = [];
    active = null;
    shown = null; // the view still holds the old project's song; show() replaces it below
    blanked = false;
    evaluated = null;

    const paths = info.songs.map((s) => s.path);
    const [reads] = await Promise.all([
      Promise.allSettled(paths.map((path) => readText(info.root, path))),
      refreshInstruments(info.root),
    ]);
    const failed: string[] = [];
    const loaded = new Map<string, Song>();
    paths.forEach((path, i) => {
      const r = reads[i];
      if (r.status === 'rejected') failed.push(`${fileStem(path)}: ${errorMessage(r.reason)}`);
      loaded.set(path, newSong(r.status === 'fulfilled' ? r.value : null));
    });
    songs = loaded;
    order = paths;
    error = failed.length ? failed.join('; ') : null;
    // A song that failed stays in the list and retries on switch.
    activateFirstLoadable();
    emit();
  }

  /** Guard (if a project is open), then open through `load`, install the result, refresh recents. */
  async function openWith(load: () => Promise<ProjectInfo | null>): Promise<void> {
    await uiOpen(() => guardAndLoad(load));
  }

  /** Run a UI open step with `uiOpening` raised, so a control open meanwhile is refused. */
  async function uiOpen<T>(fn: () => Promise<T>): Promise<T> {
    uiOpening++;
    try {
      return await fn();
    } finally {
      uiOpening--;
    }
  }

  async function pickProjectFolder(title: string): Promise<string | null> {
    try {
      return await api.pickFolder(title);
    } catch (e) {
      error = errorMessage(e);
      emit();
      return null;
    }
  }

  async function guardAndLoad(load: () => Promise<ProjectInfo | null>): Promise<void> {
    if (project && !(await guardUnsaved())) return;
    if (project) await beforeChange();
    await serial(async () => {
      const r = await loadAndInstall(load);
      if (r.ok) return;
      error = r.error;
      emit();
      await refreshRecents();
      emit();
    });
  }

  /**
   * Inside `serial()`. Open through `load` as a new generation, install the result (none if `load`
   * gives null), refresh recents. A failed `load` returns its error and does neither.
   */
  async function loadAndInstall(
    load: () => Promise<ProjectInfo | null>,
  ): Promise<{ ok: true } | { ok: false; error: string }> {
    const gen = ++nextGeneration;
    let info: ProjectInfo | null;
    opening = gen;
    try {
      info = await load();
    } catch (e) {
      return { ok: false, error: errorMessage(e) };
    } finally {
      opening = null;
    }
    if (info) await install(info, gen);
    await refreshRecents();
    emit();
    return { ok: true };
  }

  // ---- change events ----

  /**
   * The `project://changed` handler, subscribed once before the first open. An event is stamped
   * with the project it arrived for and queued behind any open in progress, so one that arrives
   * while a project's songs are read is applied once they are in; one stamped with a project that
   * is no longer open is dropped.
   */
  function onDiskChange(change: ProjectChange): void {
    const gen = opening ?? generation;
    void serial(() => (gen === generation ? applyChange(change) : Promise.resolve()));
  }

  async function applyChange(change: ProjectChange): Promise<void> {
    const { changed, added, removed } = change.songs;
    // An event from a previous project's watcher (delivered while the next one was opening).
    if (!project || change.root !== project.root) return;
    const { root } = project;
    const paths = [...changed, ...added];
    // Reconciling a playing song can evaluate it immediately. Load pins from the
    // same watcher batch before that evaluate captures a snapshot.
    const [reads] = await Promise.all([
      Promise.allSettled(paths.map((path) => readText(root, path))),
      change.instruments ? refreshInstruments(root) : undefined,
    ]);
    let inserted = false;
    paths.forEach((path, i) => {
      if (reconcile(path, reads[i])) inserted = true;
    });
    removed.forEach(diskRemoved);
    if (inserted) await reorder();
    if (change.manifest && project) {
      try {
        project = await api.info(root);
      } catch (e) {
        error = `reloading project.json failed: ${errorMessage(e)}`;
      }
    }
    emit();
  }

  function acceptInstrumentSnapshot(snapshot: InstrumentSnapshot): void {
    instruments = Object.keys(snapshot).length ? snapshot : undefined;
    pianoRollChanged();
    if (instrumentError !== null && error === instrumentError) error = null;
    instrumentError = null;
  }

  /** Called in the same serialized project queue as installation and other watcher reads. */
  async function refreshInstruments(root: string): Promise<void> {
    try {
      acceptInstrumentSnapshot(await api.instruments(root));
    } catch (e) {
      instruments = undefined;
      pianoRollChanged();
      const message = `reading instruments failed: ${errorMessage(e)}`;
      if (instrumentError !== null && error === instrumentError) error = message;
      instrumentError = message;
    }
  }

  /** A song was changed or added on disk and read. True if it was new to the list. */
  function reconcile(path: string, read: PromiseSettledResult<string>): boolean {
    const song = songs.get(path);
    if (read.status === 'rejected') {
      error = `${fileStem(path)}: ${errorMessage(read.reason)}`;
      if (song) return false;
      addSong(path, null);
      return true;
    }
    const text = read.value;
    if (!song) {
      addSong(path, text);
      return true;
    }
    if (song.conflict === 'deleted') song.conflict = null; // it exists again
    if (text === song.diskText) {
      // A no-op touch, or the disk went back to the saved text: a pending `changed` conflict is moot.
      if (song.conflict === 'changed') {
        song.conflict = null;
        song.diskPending = null;
      }
      return false;
    }
    const w = inflight.get(path);
    if (w?.gen === generation && text === w.text) return false; // our own save, settled when it resolves
    const st = stateOf(path);
    if (song.diskText === null || !st) {
      // Never shown: nothing to reconcile with.
      settle(song, text);
      return false;
    }
    const doc = st.doc.toString();
    if (doc === text) {
      // The editor already holds what the disk now says.
      settle(song, text);
    } else if (doc === song.diskText) {
      // Settle first, so the view's doc-change callback already sees a clean song.
      settle(song, text);
      replaceDoc(path, text);
      if (path === active && get(runtime.store).started) void evaluateSong(path, text);
    } else {
      song.conflict = 'changed';
      song.diskPending = text;
    }
    return false;
  }

  function diskRemoved(path: string): void {
    const song = songs.get(path);
    if (!song) return;
    if (path === active || isDirty(path)) {
      song.conflict = 'deleted';
      song.diskPending = null;
    } else {
      dropSong(path);
    }
  }

  /** Apply a transaction to the view or stash and return its resulting document. */
  function apply(path: string, spec: TransactionSpec): Text | null {
    const state = stateOf(path);
    if (!state) return null;
    const transaction = state.update(spec);
    if (view && shown === path) {
      view.dispatch(transaction);
    } else {
      songs.get(path)!.editorState = transaction.state;
    }
    return transaction.newDoc;
  }

  /**
   * Replace a song's doc with `text` as one undoable step (a `reload` unless `extras` says
   * otherwise), its changes computed by `diff`. True if the doc changed.
   */
  function replaceDoc(
    path: string,
    text: string,
    extras: Pick<TransactionSpec, 'userEvent' | 'annotations'> = { userEvent: 'reload' },
    diff: (from: string, to: string) => ChangeSpec = replaceChange,
  ): boolean {
    const st = stateOf(path);
    if (!st) return false;
    const doc = st.doc.toString();
    if (doc === text) return false;
    apply(path, { changes: diff(doc, text), ...extras });
    return true;
  }

  // ---- evaluate ----

  /**
   * Evaluate a song's text with its header's tempo. A header with a MetaError (already shown in
   * the status bar) is not evaluated. True iff the runtime evaluated it.
   */
  async function evaluateSong(path: string, text: string, autostart = true): Promise<boolean> {
    const header = readSongHeader(text);
    const started = get(runtime.store).started;
    if (!header.ok) {
      // Nothing will start, so a start armed while stopped must not linger as "will start at".
      if (!started) runtime.transport.reset();
      return false;
    }
    if (instrumentError !== null) {
      if (!started) runtime.transport.reset();
      error = instrumentError;
      emit();
      return false;
    }
    // A different song taking over playback must not inherit the playing song's transport. While
    // stopped the transport belongs to the active song (reset on every switch), so a start armed
    // before Play is kept.
    if (started && evaluated?.song.path !== path) runtime.transport.reset();
    const gen = generation;
    const opts = header.options;
    if (instruments !== undefined) opts.instruments = instruments;
    const ok = await runtime.evaluate(text, autostart ? opts : { ...opts, autostart: false });
    if (ok && gen === generation) {
      evaluated = {
        song: { path, title: titleFrom(path, header.title) },
        metaTitle: header.title,
        instruments: opts.instruments,
      };
      emit();
    }
    return ok;
  }

  async function beforeChange(): Promise<void> {
    try {
      await deps.beforeProjectChange?.();
    } catch (e) {
      console.error(e);
    }
  }

  // ---- unsaved guard ----

  async function guardUnsaved(): Promise<boolean> {
    const dirty = order.filter(needsSave);
    if (dirty.length === 0) return true;
    let choice: UnsavedChoice;
    try {
      choice = await api.confirmUnsaved(dirty.length);
    } catch (e) {
      console.error(e);
      return false;
    }
    if (choice === 'discard') return true;
    if (choice !== 'save') return false;
    let ok = true;
    for (const path of dirty) {
      const text = textOf(path);
      if (text === null || (await write(path, text)) === null) ok = false;
    }
    return ok;
  }

  let guardingShutdown = false;

  async function guardShutdown(): Promise<boolean> {
    if (guardingShutdown) return false;
    guardingShutdown = true;
    try {
      return await guardUnsaved();
    } finally {
      guardingShutdown = false;
    }
  }

  // ---- public ----

  async function switchSong(path: string): Promise<void> {
    if (path === active || !songs.has(path)) return;
    if (!(await ensureLoaded(path))) return;
    if (!songs.has(path)) return; // removed while it was read
    active = path;
    activeTransport();
    show();
    emit();
  }

  /** Call after `active` changes. While stopped the transport is the active song's: an armed
   * start does not carry over to the next one. */
  function activeTransport(): void {
    if (!get(runtime.store).started) runtime.transport.reset();
  }

  /** The song's edits would be lost without a save: dirty, or kept with a `deleted` conflict. */
  const needsSave = (path: string): boolean =>
    isDirty(path) || songs.get(path)?.conflict === 'deleted';

  /** Wait for every write of `path` in flight. */
  async function writesDone(path: string): Promise<void> {
    while (inflight.has(path)) await inflight.get(path)!.done;
  }

  /** Run `fn` with `path` marked as moving; writes of it wait until `fn` has settled. */
  async function whileMoving<T>(path: string, fn: () => Promise<T>): Promise<T> {
    const done = settled();
    moving.set(path, done.promise);
    try {
      return await fn();
    } finally {
      moving.delete(path);
      done.resolve();
    }
  }

  const isProjectChanged = (e: unknown): boolean =>
    e instanceof ProjectError && e.kind === 'projectChanged';

  /** Inside `serial()`. */
  async function renameTask(path: string, stem: string): Promise<RenameOutcome> {
    if (!songs.has(path)) return { ok: false, reason: 'missing' };
    await writesDone(path);
    const song = songs.get(path);
    if (!song || !project) return { ok: false, reason: 'missing' };
    if (song.conflict === 'deleted') return { ok: false, reason: 'deleted' };
    // A song kept only in memory (a `deleted` conflict) may hold the target name: the library sees
    // no file there and would rename over the buffer the editor still has.
    const target = `songs/${stem}.js`;
    if (target !== path && songs.has(target)) {
      throw new ProjectError(`song ${target} already exists`, 'songExists');
    }
    const gen = generation;
    const { root } = project;
    const to = await whileMoving(path, async () => {
      let next: string;
      try {
        next = await api.renameSong(root, path, stem);
      } catch (e) {
        if (gen !== generation || isProjectChanged(e)) return null;
        throw e;
      }
      if (gen !== generation) return null;
      // Moved before `moving` is released, so a waiting write sees the old path gone.
      songs.delete(path);
      songs.set(next, song);
      order = order.map((p) => (p === path ? next : p));
      song.derived = null; // the title may follow the file name
      if (active === path) active = next;
      if (shown === path) shown = next;
      if (evaluated?.song.path === path) {
        const { metaTitle } = evaluated;
        evaluated = { ...evaluated, song: { path: next, title: titleFrom(next, metaTitle) } };
      }
      return next;
    });
    if (to === null) return { ok: false, reason: 'projectChanged' };
    await reorder();
    emit();
    return { ok: true, from: path, to };
  }

  /** Inside `serial()`. */
  async function deleteTask(path: string, discard: boolean): Promise<DeleteOutcome> {
    if (!songs.has(path)) return { ok: false, reason: 'missing' };
    await writesDone(path);
    const song = songs.get(path);
    if (!song || !project) return { ok: false, reason: 'missing' };
    if (needsSave(path) && !discard) return { ok: false, reason: 'unsaved' };
    if (song.conflict === 'deleted') {
      dropSong(path); // no file to delete
    } else {
      const gen = generation;
      const { root } = project;
      const dropped = await whileMoving(path, async () => {
        try {
          await api.deleteSong(root, path);
        } catch (e) {
          if (gen !== generation || isProjectChanged(e)) return false;
          // Already gone on disk (the watcher's event not yet applied): the goal is met.
          if (!(e instanceof ProjectError && e.kind === 'songNotFound')) throw e;
        }
        if (gen !== generation) return false;
        dropSong(path); // before `moving` is released, so a waiting write writes nothing
        return true;
      });
      if (!dropped) return { ok: false, reason: 'projectChanged' };
    }
    if (active === path) {
      activateFirstLoadable();
      activeTransport();
    }
    let stopped = false;
    if (evaluated?.song.path === path) {
      stopped = get(runtime.store).started;
      runtime.stop();
      evaluated = null;
    }
    emit();
    return { ok: true, stopped };
  }

  async function saveSongs(target: SaveTarget): Promise<SaveOutcome> {
    const gen = generation;
    const all = 'all' in target;
    if (!all && !songs.has(target.path)) return { ok: false, reason: 'missing' };
    const paths = all ? order.filter(needsSave) : needsSave(target.path) ? [target.path] : [];
    const saved: string[] = [];
    for (const path of paths) {
      const text = textOf(path);
      if (text === null) continue; // never read: nothing of ours to write
      if ((await write(path, text)) !== null) {
        saved.push(path);
        continue;
      }
      if (gen !== generation) return { ok: false, reason: 'projectChanged' };
      if (!songs.has(path)) {
        // Renamed or deleted while the write waited.
        if (all) continue;
        return { ok: false, reason: 'missing' };
      }
      return {
        ok: false,
        reason: 'failed',
        saved,
        failedPath: path,
        error: error ?? 'save failed',
      };
    }
    return { ok: true, saved };
  }

  /** Inside `serial()`. */
  async function openTask(path: string, discard: boolean): Promise<OpenOutcome> {
    if (uiOpening > 0) return { ok: false, reason: 'busy' };
    if (project) {
      let root: string;
      try {
        root = await api.projectRootOf(path);
      } catch (e) {
        return { ok: false, reason: 'failed', error: errorMessage(e) };
      }
      if (root === project.root) return { ok: true, alreadyOpen: true };
      // A UI open may have begun while the root was looked up.
      if (uiOpening > 0) return { ok: false, reason: 'busy' };
    }
    const dirty = order.filter(needsSave);
    if (dirty.length > 0 && !discard) return { ok: false, reason: 'unsaved', dirty };
    if (project) await beforeChange();
    const r = await loadAndInstall(() => api.open(path));
    if (!r.ok) return { ok: false, reason: 'failed', error: r.error };
    return { ok: true, alreadyOpen: false };
  }

  /** The song a conflict action applies to, if it has a `changed` conflict. */
  function changedConflict(path: string | null): Song | null {
    const song = path === null ? undefined : songs.get(path);
    return song?.conflict === 'changed' ? song : null;
  }

  return {
    store: { subscribe: state.subscribe },

    async init(canCloseWindow) {
      if (!available) return;
      if (!closeHookRegistered) {
        closeHookRegistered = true;
        try {
          await api.onCloseRequested(async (e) => {
            // An export can start while the unsaved-changes guard writes one or more songs.
            if (
              canCloseWindow?.() === false ||
              !(await guardShutdown()) ||
              canCloseWindow?.() === false
            ) {
              e.preventDefault();
            }
          });
        } catch (e) {
          console.error(e);
        }
      }
      let watchError: string | null = null;
      if (!watching) {
        watching = true;
        try {
          await api.onChange(onDiskChange);
        } catch (e) {
          watching = false; // the next init tries again
          watchError = `watching the project failed: ${errorMessage(e)}`;
        }
      }
      await openWith(() => api.last());
      if (watchError) {
        error = error ? `${error}; ${watchError}` : watchError;
        emit();
      }
    },

    attachView(next) {
      if (next === view) return;
      stash();
      view = next;
      blanked = true; // a newly attached view shows none of our songs
      show();
      if (!next) pianoRollChanged();
    },

    pianoRollRevision: { subscribe: pianoRollRevision.subscribe },
    pianoRollChanged,
    pianoRollSnapshot,
    pianoRollMatchesLivePins() {
      return (
        evaluated !== null && instrumentError === null && evaluated.instruments === instruments
      );
    },
    editPianoRoll(snapshot, patches, target) {
      if (!view || !matchesPianoRoll(snapshot) || view.state.doc.toString() !== snapshot.code) {
        return null;
      }
      if (!patches.length) return null;
      const changes = view.state.changes(patches);
      const spans: SourceSpan[] = [];
      changes.iterChangedRanges((_from, _to, from, to) => spans.push({ from, to }));
      if (!spans.length) return null;
      const reveal = (patches.find((patch) => patch.reveal) ?? patches.at(-1)!).from;
      const primary = changes.mapPos(reveal, -1);
      const { from, to } = view.viewport;
      const mappedTarget = target && {
        from: changes.mapPos(target.from, -1),
        to: changes.mapPos(target.to, 1),
      };
      const targetCaret = target && changes.mapPos(target.from, 1);
      apply(snapshot.path, {
        changes,
        effects: [
          EditorView.scrollIntoView(primary, {
            y: reveal < from || reveal > to ? 'center' : 'nearest',
          }),
          pianoRollEditLocation.of(spans),
        ],
        userEvent: 'input.pianoRoll',
        annotations: isolateHistory.of('full'),
      });
      const current = pianoRollSnapshot();
      return current && { snapshot: current, target: mappedTarget, targetCaret, spans };
    },
    clearPianoRollFeedback() {
      if (view?.state.field(pianoRollEditHighlight, false)?.size) {
        view.dispatch({ effects: pianoRollEditLocation.of([]) });
      }
    },
    revealPianoRoll(snapshot, { from, to }) {
      if (
        !view ||
        !matchesPianoRoll(snapshot) ||
        from < 0 ||
        to < from ||
        to > view.state.doc.length
      ) {
        return;
      }
      apply(snapshot.path, { selection: { anchor: from, head: to }, scrollIntoView: true });
      view.focus();
    },

    newProject: (askName) =>
      uiOpen(async () => {
        const dir = await pickProjectFolder('New project folder');
        if (dir === null) return;
        const name = await askName(dir);
        if (name !== null) await guardAndLoad(() => api.create(dir, name));
      }),

    openProject: () =>
      uiOpen(async () => {
        const dir = await pickProjectFolder('Open project');
        if (dir !== null) await guardAndLoad(() => api.open(dir));
      }),

    openRecent: (path) => openWith(() => api.open(path)),

    switchSong,

    async createSong(stem) {
      if (!project) throw new Error('no project is open');
      const gen = generation;
      const { root } = project;
      let path: string;
      let text: string;
      try {
        path = await api.createSong(root, stem);
        text = await readText(root, path);
      } catch (e) {
        if (gen !== generation) return null; // the project changed meanwhile
        throw e instanceof ProjectError ? e : new Error(errorMessage(e));
      }
      if (gen !== generation) return null; // the project changed meanwhile
      // The watcher's `added` may have listed it already.
      if (!songs.has(path)) {
        addSong(path, text);
        await reorder();
        if (gen !== generation) return null;
      }
      error = null;
      await switchSong(path);
      return path;
    },

    renameSong: (path, stem) => serial(() => renameTask(path, stem)),

    deleteSong: (path, opts) => serial(() => deleteTask(path, opts.discard)),

    saveSongs,

    async openPath(path, opts) {
      if (uiOpening > 0) return { ok: false, reason: 'busy' };
      return serial(() => openTask(path, opts.discard));
    },

    async save() {
      if (!active) return false;
      const path = active;
      return (await write(path, textOf(path)!)) !== null;
    },

    async saveAndEvaluate() {
      if (!active) return;
      const path = active;
      const doc = textOf(path)!; // read when the key is pressed
      const source = isDirty(path) ? await write(path, doc) : doc;
      if (source !== null) await evaluateSong(path, source);
    },

    keepMine(path) {
      const song = changedConflict(path ?? active);
      if (!song) return;
      // The disk now holds the other text: keep it as diskText so the song stays dirty.
      if (song.diskPending !== null) settle(song, song.diskPending);
      else song.conflict = null;
      emit();
    },

    takeDisk(path) {
      const target = path ?? active;
      const song = changedConflict(target);
      if (!song || song.diskPending === null) return;
      const text = song.diskPending;
      settle(song, text);
      replaceDoc(target!, text);
      emit();
    },

    async recreateDeleted() {
      if (!active || songs.get(active)?.conflict !== 'deleted') return;
      await write(active, textOf(active)!);
    },

    async closeDeleted() {
      if (!active || songs.get(active)?.conflict !== 'deleted') return;
      dropSong(active);
      activateFirstLoadable();
      emit();
    },

    docChanged() {
      if (!active) return;
      const i = current.songs.findIndex((s) => s.path === active);
      if (i < 0) return emit();
      const item = itemOf(active);
      const { headerError, structure } = derive(active);
      const metaError = headerError?.located ?? null;
      const notice = noticeOf(active);
      if (
        sameItem(current.songs[i], item) &&
        current.metaError === metaError &&
        current.structure === structure &&
        sameNotice(current.notice, notice)
      ) {
        return;
      }
      const items = current.songs.slice();
      items[i] = item;
      publish({ ...current, songs: items, metaError, structure, notice });
    },

    applyMeta(path, value, seenDoc) {
      if (!active || !stateOf(active)) return false;
      const text = textOf(active);
      if (text !== seenDoc) return false;
      const edit = setMetaValue(text, path, value);
      if (!edit) return false;
      apply(active, { changes: edit, userEvent: 'input.meta' });
      emit(); // a stashed state has no view listener to report the change
      return true;
    },

    instrumentSelection() {
      if (!project || !active || !view || shown !== active) return null;
      const state = view.state;
      if (state.selection.ranges.length !== 1) return null;
      const { from, to } = state.selection.main;
      const expect = state.doc.toString();
      try {
        instrumentApplication(expect, from, to, 'instrument');
      } catch {
        return null;
      }
      return { root: project.root, song: active, from, to, expect };
    },

    async applyInstrument({ root, song, id, from, to, expect, pinFingerprint }) {
      const gen = generation;
      const checkProject = () => {
        if (project?.root !== root || opening !== null || generation !== gen) {
          throw new InstrumentApplyError('project', 'The requested project is not open.');
        }
        if (uiOpening > 0) {
          throw new InstrumentApplyError('project', 'A project change is in progress. Try again.');
        }
      };
      checkProject();
      const path = active;
      const checkSong = () => {
        if (!path || active !== path || (song !== undefined && song !== path)) {
          throw new InstrumentApplyError('song', 'Open the requested song in the editor.');
        }
        const text = textOf(path);
        if (text === null || songs.get(path)?.conflict === 'deleted') {
          throw new InstrumentApplyError('song', 'Open an available song in the editor.');
        }
        if (text !== expect) {
          throw new InstrumentApplyError('stale', 'The song changed since you read it.');
        }
        return path;
      };
      checkSong();
      let edit;
      try {
        edit = instrumentApplication(expect, from, to, id);
      } catch (error) {
        throw new InstrumentApplyError('invalid', errorMessage(error));
      }
      return serial(async () => {
        checkProject();
        checkSong();
        const pin = pinFingerprint === undefined ? undefined : { id, fingerprint: pinFingerprint };
        const snapshot = await api.instruments(root, pin);
        // A UI open may have checked for unsaved edits before entering the project queue.
        checkProject();
        const target = checkSong();
        if (!Object.hasOwn(snapshot, id)) {
          throw new InstrumentApplyError(
            'invalid',
            `Pin '${id}' to the project before applying it.`,
          );
        }
        // The next evaluation uses the same pins as Apply, even before the watcher fires.
        acceptInstrumentSnapshot(snapshot);
        const end = edit.from + edit.insert.length;
        apply(target, {
          changes: edit,
          selection: { anchor: edit.from, head: end },
          userEvent: 'input.instrument',
          annotations: isolateHistory.of('full'),
        });
        emit();
        return { song: target, from: edit.from, to: end };
      });
    },

    externalEdit(path, text, opts) {
      const st = path === active ? stateOf(path) : null;
      if (!st) return 'missing';
      if (opts?.expect !== undefined && toLF(opts.expect) !== st.doc.toString()) return 'stale';
      // 'claude' never joins the event before it; isolateHistory keeps typing from joining after.
      // Line-level changes, so the mark and the undo cover only the lines Claude changed.
      const changed = replaceDoc(
        path,
        toLF(text),
        { userEvent: 'claude', annotations: [claudeEdit.of(true), isolateHistory.of('full')] },
        lineChanges,
      );
      if (changed) emit(); // a stashed state has no view listener to report the change
      return {
        changed,
        dirty: isDirty(path),
        conflict: songs.get(path)?.conflict ?? null,
      };
    },

    async evaluateBuffer(path, opts) {
      const text = textOf(path);
      // Title and headerError of the text being evaluated, read before the evaluate awaits.
      const d = songs.has(path) ? derive(path) : null;
      const pinsError = instrumentError;
      let ok = false;
      try {
        ok = text !== null && (await evaluateSong(path, text, opts?.autostart ?? true));
      } catch (e) {
        console.error(e);
      }
      const rs = get(runtime.store);
      let error: string | null = null;
      let errorKind: EvalErrorKind | null = null;
      if (!ok) {
        if (d?.headerError) {
          error = d.headerError.reported;
          errorKind = 'meta';
        } else if (pinsError) {
          error = pinsError;
          errorKind = 'instruments';
        } else {
          error = rs.evalError ?? 'evaluate failed';
          errorKind = 'eval';
        }
      }
      return {
        ok,
        song: path,
        title: d?.title ?? fileStem(path),
        started: rs.started,
        audio: rs.audio,
        error,
        errorKind,
      };
    },

    async peekSong(path) {
      const song = songs.get(path);
      if (!song) return null;
      const unread = song.diskText === null;
      if (!(await ensureLoaded(path))) return null;
      if (songs.get(path) !== song) return null; // removed, or the project changed, while it was read
      if (unread) emit(); // its title now comes from the text
      return { text: textOf(path)!, dirty: isDirty(path) };
    },

    activeText: () => (active ? textOf(active) : null),

    guardUnsaved,
    guardShutdown,
  };
}

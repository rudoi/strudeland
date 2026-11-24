import {
  PROJECT_CHANGED,
  ProjectError,
  type ProjectApi,
  type ProjectChange,
  type ProjectInfo,
  type RecentProject,
  type UnsavedChoice,
} from '../../src/lib/project/api';
import type { InstrumentSnapshot } from '../../src/lib/strudel/instruments';

export const tick = () => new Promise((r) => setTimeout(r, 0));

/** Let queued change events and their reads finish. */
export const flush = async () => {
  for (let i = 0; i < 5; i++) await tick();
};

export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
}

interface FakeProject {
  name: string;
  files: Map<string, string>;
}

/** Byte-wise by UTF-8, the order Rust lists songs in. */
const byBytes = (a: string, b: string) => Buffer.compare(Buffer.from(a), Buffer.from(b));

const songExists = (path: string) => new ProjectError(`song ${path} already exists`, 'songExists');
const songNotFound = (path: string) => new ProjectError(`song ${path} not found`, 'songNotFound');
const checkStem = (stem: string) => {
  if (!/^[A-Za-z0-9 _-]{1,64}$/.test(stem) || stem !== stem.trim()) {
    throw new ProjectError(
      `invalid song name ${JSON.stringify(stem)} (use letters, digits, space, - and _; 1 to 64 characters)`,
      'invalidName',
    );
  }
};

/**
 * An in-memory ProjectApi over `projects` (root to name and files). Errors are thrown as strings, like
 * Tauri's invoke, except the create, rename and delete song commands, which throw `ProjectError`. Every call is logged in `fake.calls`.
 */
export function fakeProjectApi(
  projects: Record<string, { name: string; files: Record<string, string> }>,
) {
  const disk = new Map<string, FakeProject>(
    Object.entries(projects).map(([root, p]) => [
      root,
      { name: p.name, files: new Map(Object.entries(p.files)) },
    ]),
  );
  const calls: string[] = [];
  const titles: string[] = [];
  const handlers = new Set<(c: ProjectChange) => void>();
  const allHandlers: ((c: ProjectChange) => void)[] = [];
  const closeHandlers: ((e: { preventDefault(): void }) => void | Promise<void>)[] = [];
  const confirmCounts: number[] = [];
  const fake = {
    calls,
    titles,
    disk,
    allHandlers,
    closeHandlers,
    confirmCounts,
    current: null as string | null,
    last: null as string | null,
    lastFails: false,
    recents: [] as RecentProject[],
    recentsFail: false,
    failRead: new Set<string>(),
    failSave: new Set<string>(),
    failOpen: new Set<string>(),
    failInfo: false,
    instrumentSnapshots: new Map<string, InstrumentSnapshot | string>(),
    failOnChange: false,
    failPick: false,
    confirm: [] as UnsavedChoice[],
    picks: [] as (string | null)[],
    /** While set, readSong waits for this before reading. */
    holdReads: null as Promise<void> | null,
    /** While set, saveSong writes the file, then waits for this before resolving. */
    holdSaves: null as Promise<void> | null,
    /** While set, renameSong renames the file, then waits for this before resolving. */
    holdRenames: null as Promise<void> | null,
    /** While set, deleteSong deletes the file, then waits for this before resolving. */
    holdDeletes: null as Promise<void> | null,
    /** While set, pickFolder waits for this before answering. */
    holdPicks: null as Promise<void> | null,
    /** While set, confirmUnsaved waits for this before answering. */
    holdConfirms: null as Promise<void> | null,
    /** While set, projectRootOf waits for this before answering. */
    holdRootOf: null as Promise<void> | null,
    /** Directory to canonical root for projectRootOf; a directory not listed maps to itself. */
    rootOf: {} as Record<string, string>,
    files(): Map<string, string> {
      return disk.get(fake.current!)!.files;
    },
    projectInfo(root: string): ProjectInfo {
      const p = disk.get(root)!;
      const songs = [...p.files.keys()].sort(byBytes).map((path) => ({ path }));
      return { root, name: p.name, songs };
    },
    /** An external edit (Claude, git, another editor) followed by its watcher event. */
    emit(
      change: Partial<ProjectChange['songs']> & {
        manifest?: boolean;
        recordings?: boolean;
        instruments?: boolean;
        root?: string;
      },
    ) {
      const payload: ProjectChange = {
        root: change.root ?? fake.current ?? '',
        songs: {
          changed: change.changed ?? [],
          added: change.added ?? [],
          removed: change.removed ?? [],
        },
        manifest: change.manifest ?? false,
        recordings: change.recordings ?? false,
        instruments: change.instruments ?? false,
      };
      handlers.forEach((h) => h(payload));
    },
  };
  /** Like the Rust commands: `root` must be the open project. */
  const check = (root: string) => {
    if (root !== fake.current) throw PROJECT_CHANGED;
  };
  /** `check` for the song commands, which reject with a `ProjectError`. */
  const songCheck = (root: string) => {
    if (root !== fake.current) throw new ProjectError(PROJECT_CHANGED, 'projectChanged');
  };
  const api: ProjectApi = {
    async create(dir, name) {
      calls.push(`create:${dir}:${name}`);
      if (fake.failOpen.has(dir)) throw `${dir} already contains files`;
      disk.set(dir, {
        name,
        files: new Map([['songs/main.js', `const meta = { title: 'main' };\n`]]),
      });
      fake.current = dir;
      return fake.projectInfo(dir);
    },
    async open(dir) {
      calls.push(`open:${dir}`);
      if (fake.failOpen.has(dir) || !disk.has(dir)) {
        throw `${dir} is not a strudeland project (no project.json)`;
      }
      fake.current = dir;
      return fake.projectInfo(dir);
    },
    async info(root) {
      calls.push('info');
      check(root);
      if (fake.failInfo) throw 'project.json: expected value at line 1 column 1';
      return fake.projectInfo(fake.current!);
    },
    async instruments(root) {
      check(root);
      const pins = fake.instrumentSnapshots.get(root);
      if (typeof pins === 'string') throw pins;
      return pins ?? {};
    },
    async last() {
      calls.push('last');
      if (fake.lastFails) throw 'recents are broken';
      if (!fake.last) return null;
      fake.current = fake.last;
      return fake.projectInfo(fake.last);
    },
    async recents() {
      calls.push('recents');
      if (fake.recentsFail) throw 'recents unreadable';
      return fake.recents;
    },
    async listSongs(root) {
      calls.push('listSongs');
      check(root);
      return fake.projectInfo(fake.current!).songs;
    },
    async readSong(root, path) {
      calls.push(`read:${path}`);
      if (fake.holdReads) await fake.holdReads;
      check(root);
      if (fake.failRead.has(path)) throw `${path}: permission denied`;
      const text = fake.files().get(path);
      if (text === undefined) throw `song ${path} not found`;
      return text;
    },
    async saveSong(root, path, contents) {
      calls.push(`save:${path}`);
      check(root);
      if (fake.failSave.has(path)) throw `${path}: disk full`;
      fake.files().set(path, contents);
      if (fake.holdSaves) await fake.holdSaves;
    },
    async createSong(root, stem) {
      calls.push(`createSong:${stem}`);
      songCheck(root);
      checkStem(stem);
      const path = `songs/${stem}.js`;
      if (fake.files().has(path)) throw songExists(path);
      fake.files().set(path, `const meta = {\n  title: '${stem}',\n  bpm: 120,\n};\n`);
      return path;
    },
    async renameSong(root, path, stem) {
      calls.push(`rename:${path}:${stem}`);
      songCheck(root);
      checkStem(stem);
      const files = fake.files();
      const text = files.get(path);
      if (text === undefined) throw songNotFound(path);
      const next = `songs/${stem}.js`;
      if (files.has(next)) throw songExists(next);
      files.delete(path);
      files.set(next, text);
      if (fake.holdRenames) await fake.holdRenames;
      return next;
    },
    async deleteSong(root, path) {
      calls.push(`delete:${path}`);
      songCheck(root);
      if (!fake.files().delete(path)) throw songNotFound(path);
      if (fake.holdDeletes) await fake.holdDeletes;
    },
    async projectRootOf(dir) {
      calls.push(`rootOf:${dir}`);
      if (fake.holdRootOf) await fake.holdRootOf;
      const root = fake.rootOf[dir] ?? dir;
      if (!disk.has(root)) throw `${dir} is not a strudeland project (no project.json)`;
      return root;
    },
    async onChange(handler) {
      calls.push('onChange');
      if (fake.failOnChange) throw 'watch failed';
      handlers.add(handler);
      allHandlers.push(handler);
      return () => void handlers.delete(handler);
    },
    async pickFolder(title) {
      calls.push(`pickFolder:${title}`);
      if (fake.holdPicks) await fake.holdPicks;
      if (fake.failPick) throw 'dialog unavailable';
      return fake.picks.shift() ?? null;
    },
    async confirmUnsaved(count) {
      calls.push(`confirm:${count}`);
      confirmCounts.push(count);
      if (fake.holdConfirms) await fake.holdConfirms;
      return fake.confirm.shift() ?? 'cancel';
    },
    async setWindowTitle(title) {
      titles.push(title);
    },
    async onCloseRequested(handler) {
      calls.push('onCloseRequested');
      closeHandlers.push(handler);
      return () => {};
    },
  };
  return { api, fake };
}

/**
 * The memo library of the open project: the list (`recording_list`, newest first), playback
 * through the memo player, rename and delete. It follows the session's project, refreshes after a
 * take is saved and on a watcher change under `recordings/`, and drops a result that arrives after
 * a project switch. Talks to Tauri only through `RecordingApi`.
 */
import { get, writable, type Readable } from 'svelte/store';
import type { MemoPlayer, MemoPlayerState, PlayHow } from '../audio/memo-player';
import type { ProjectChange } from '../project/api';
import { errorMessage } from '../errors';
import type { SongStructure } from '../strudel/structure';
import type { RecordingApi } from './api';
import { memoAudible, songTempoOf, sortMemos, takeRunning, type SongTempo } from './memo-ui';
import type { RecordingEntry, RecordingPhase, RecordingUiState } from './types';

export interface MemosState {
  /** Newest first. Empty until the open project's list loads. */
  items: RecordingEntry[];
  /** The open project's list has loaded at least once. */
  loaded: boolean;
  /** The last refresh failed. */
  error: string | null;
}

export interface Memos {
  readonly store: Readable<MemosState>;
  /** The memo player's state. */
  readonly player: Readable<MemoPlayerState>;
  /**
   * Re-reads the open project's list. Idempotent; never rejects. One listing runs at a time: a
   * call during one waits for it and then one more (shared by every call made meanwhile).
   */
  refresh(): Promise<void>;
  /** Plays a listed memo (nothing while a take runs). Call from a user gesture. */
  play(stem: string, how: PlayHow): Promise<void>;
  /** Stops playback and clears a playback error. */
  stopPlayback(): void;
  /** Resolves to the new stem; throws the `RecordingError`. */
  rename(stem: string, next: string): Promise<string>;
  /** Throws the `RecordingError` (after refreshing the list). */
  remove(stem: string): Promise<void>;
  destroy(): void;
}

/** What the player needs from the library: the open project's bytes and the song's tempo. */
export interface PlayerIo {
  readBytes(stem: string): Promise<Uint8Array>;
  songTempo(): SongTempo | null;
}

export interface MemosDeps {
  api: Pick<RecordingApi, 'list' | 'rename' | 'delete' | 'read'>;
  session: {
    readonly store: Readable<{
      project: { root: string } | null;
      active: string | null;
      structure: SongStructure | null;
    }>;
  };
  /** The project API's `onChange` (its own subscription). */
  onChange?: (handler: (change: ProjectChange) => void) => Promise<() => void>;
  /** The recording controller: playback waits for a take, and a saved take refreshes the list. */
  recording?: { readonly store: Readable<RecordingUiState> };
  createPlayer: (io: PlayerIo) => MemoPlayer;
}

export function createMemos(deps: MemosDeps): Memos {
  const { api, session } = deps;
  const rootNow = (): string | null => get(session.store).project?.root ?? null;

  const player = deps.createPlayer({
    readBytes(stem) {
      const root = rootNow();
      if (root === null) return Promise.reject(new Error('no project is open'));
      return api.read(root, stem);
    },
    songTempo() {
      const s = get(session.store);
      if (s.active === null || s.structure === null) return null;
      return songTempoOf(s.structure);
    },
  });

  let state: MemosState = { items: [], loaded: false, error: null };
  const store = writable(state);
  const patch = (p: Partial<MemosState>): void => {
    state = { ...state, ...p };
    store.set(state);
  };

  /** Bumped by every refresh and project switch; an older list is dropped. */
  let gen = 0;
  let root: string | null = null;
  let destroyed = false;
  const offs: (() => void)[] = [];

  async function load(): Promise<void> {
    const g = ++gen;
    const r = root;
    if (r === null || destroyed) return;
    try {
      const list = await api.list(r);
      if (g !== gen || r !== root || destroyed) return;
      patch({ items: sortMemos(list), loaded: true, error: null });
    } catch (e) {
      if (g !== gen || r !== root || destroyed) return;
      patch({ error: errorMessage(e) });
    }
  }

  /** The listing under way, and the one queued behind it. */
  let inFlight: Promise<void> | null = null;
  let trailing: Promise<void> | null = null;

  function refresh(): Promise<void> {
    if (inFlight === null) {
      const p: Promise<void> = load().finally(() => {
        if (inFlight === p) inFlight = null;
      });
      return (inFlight = p);
    }
    if (trailing === null) {
      const p: Promise<void> = inFlight.then(() => {
        if (trailing === p) trailing = null;
        return refresh();
      });
      trailing = p;
    }
    return trailing;
  }

  /** Stops playback when `stem` is the memo loading or playing. */
  function stopIfPlaying(stem: string): void {
    const p = get(player.store);
    if (memoAudible(p) && p.stem === stem) player.stop();
  }

  let subscribed = false;
  offs.push(
    session.store.subscribe((s) => {
      const next = s.project?.root ?? null;
      if (subscribed && next === root) return;
      const switched = subscribed;
      subscribed = true;
      root = next;
      gen++;
      // The old project's listing is dropped by `gen`: the new one need not wait for it.
      inFlight = null;
      trailing = null;
      if (switched) player.stop();
      patch({ items: [], loaded: false, error: null });
      void refresh();
    }),
  );

  if (deps.recording) {
    let phase: RecordingPhase | null = null;
    offs.push(
      deps.recording.store.subscribe((s) => {
        const prev = phase;
        phase = s.phase;
        // A stop's own watcher event may lag.
        if (prev === 'stopping' && s.phase !== 'stopping') void refresh();
      }),
    );
  }

  if (deps.onChange) {
    const onChange = deps.onChange;
    void (async () => {
      try {
        const off = await onChange((change: ProjectChange) => {
          if (change.recordings === true && change.root === root) void refresh();
        });
        if (destroyed) off();
        else offs.push(off);
      } catch (e) {
        console.error(e);
      }
    })();
  }

  const busy = (): boolean => {
    const rec = deps.recording ? get(deps.recording.store) : null;
    return takeRunning(rec?.phase ?? null);
  };

  return {
    store: { subscribe: store.subscribe },
    player: player.store,
    refresh,
    async play(stem, how) {
      if (destroyed || busy()) return;
      const item = state.items.find((m) => m.stem === stem);
      if (!item) return;
      await player.play(item, how);
    },
    stopPlayback: () => player.stop(),
    async rename(stem, next) {
      const r = root;
      if (r === null) throw new Error('no project is open');
      stopIfPlaying(stem);
      const stemNow = await api.rename(r, stem, next);
      await refresh();
      return stemNow;
    },
    async remove(stem) {
      const r = root;
      if (r === null) throw new Error('no project is open');
      stopIfPlaying(stem);
      try {
        await api.delete(r, stem);
      } finally {
        await refresh();
      }
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      gen++;
      player.dispose();
      offs.splice(0).forEach((f) => f());
    },
  };
}

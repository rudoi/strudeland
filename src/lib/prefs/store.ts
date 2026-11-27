import { writable, type Readable } from 'svelte/store';
import { errorMessage } from '../errors';
import { DEFAULT_PREFS, type Prefs, type PrefsApi } from './api';

export interface PrefsState extends Prefs {
  /** The message of the last failed save; cleared by the next good save. */
  error: string | null;
}

export interface PrefsStore {
  store: Readable<PrefsState>;
  /** The current value, synchronously. */
  vim(): boolean;
  auditionNotes(): boolean;
  formatOnSave(): boolean;
  /** The current value, synchronously. */
  limiter(): boolean;
  /** Whether this session set `key`; a value the user chose before the load wins over a migration. */
  edited(key: keyof Prefs): boolean;
  /**
   * Whether a load succeeded and read a good (or missing) file. False before the load, after a
   * failed one, and when a bad file gave the defaults: only then may a save be made unasked.
   */
  loadedClean(): boolean;
  /** Loads the initial preference snapshot. Never rejects. */
  load(): Promise<void>;
  /** Resolves whether the save landed; never rejects. */
  setVim(on: boolean): Promise<boolean>;
  /** Resolves whether the save landed; never rejects. */
  setAuditionNotes(on: boolean): Promise<boolean>;
  /** Resolves whether the save landed; never rejects. */
  setFormatOnSave(on: boolean): Promise<boolean>;
  /** Resolves whether the save landed; never rejects. */
  setPianoRollOpen(on: boolean): Promise<boolean>;
  /** Resolves whether the save landed; never rejects. */
  setLimiter(on: boolean): Promise<boolean>;
}

export function createPrefs(api: PrefsApi | null): PrefsStore {
  const state = writable<PrefsState>({ ...DEFAULT_PREFS, error: null });
  // One subscription keeps the synchronous accessors current.
  let current: Prefs = DEFAULT_PREFS;
  state.subscribe((s) => (current = s));
  let edits: Partial<Prefs> = {};
  let initial: Promise<Prefs> | undefined;
  let clean = false;
  let chain: Promise<void> = Promise.resolve();

  function loadInitial(): Promise<Prefs> {
    if (!api) return Promise.resolve(DEFAULT_PREFS);
    return (initial ??= (async () => {
      const { prefs: saved, clean: wasClean } = await api.get();
      clean = wasClean;
      state.update((s) => ({ ...s, ...saved, ...edits }));
      return saved;
    })().catch((e) => {
      initial = undefined;
      throw e;
    }));
  }

  function set(key: keyof Prefs, on: boolean): Promise<boolean> {
    const snapshot = { ...edits, [key]: on };
    edits = snapshot;
    let saved: Promise<boolean> = Promise.resolve(false);
    if (api) {
      // Queue each snapshot before notifying subscribers. Wait for untouched fields to load.
      saved = chain.then(async () => {
        try {
          // A save can join the startup read. Retry if that read fails before writing.
          const loaded = await (initial?.catch(() => loadInitial()) ?? loadInitial());
          await api.set({ ...loaded, ...snapshot });
          state.update((s) => ({ ...s, error: null }));
          return true;
        } catch (e) {
          state.update((s) => ({ ...s, error: errorMessage(e) }));
          return false;
        }
      });
      chain = saved.then(() => {});
    }
    state.update((s) => ({ ...s, [key]: on }));
    return saved;
  }

  return {
    store: { subscribe: state.subscribe },
    vim: () => current.vim,
    auditionNotes: () => current.auditionNotes,
    formatOnSave: () => current.formatOnSave,
    limiter: () => current.limiter,
    edited: (key) => key in edits,
    loadedClean: () => clean,
    async load() {
      try {
        await loadInitial();
      } catch (e) {
        console.error('could not load preferences', e);
      }
    },
    setVim: (on) => set('vim', on),
    setAuditionNotes: (on) => set('auditionNotes', on),
    setFormatOnSave: (on) => set('formatOnSave', on),
    setPianoRollOpen: (on) => set('pianoRollOpen', on),
    setLimiter: (on) => set('limiter', on),
  };
}

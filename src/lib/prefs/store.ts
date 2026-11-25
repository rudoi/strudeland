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
  /** Loads the initial preference snapshot. Never rejects. */
  load(): Promise<void>;
  /** Never rejects. */
  setVim(on: boolean): Promise<void>;
  /** Never rejects. */
  setAuditionNotes(on: boolean): Promise<void>;
  /** Never rejects. */
  setFormatOnSave(on: boolean): Promise<void>;
  /** Never rejects. */
  setPianoRollOpen(on: boolean): Promise<void>;
}

export function createPrefs(api: PrefsApi | null): PrefsStore {
  const state = writable<PrefsState>({ ...DEFAULT_PREFS, error: null });
  // One subscription keeps the synchronous accessors current.
  let current: Prefs = DEFAULT_PREFS;
  state.subscribe((s) => (current = s));
  let edits: Partial<Prefs> = {};
  let initial: Promise<Prefs> | undefined;
  let chain: Promise<void> = Promise.resolve();

  function loadInitial(): Promise<Prefs> {
    if (!api) return Promise.resolve(DEFAULT_PREFS);
    return (initial ??= (async () => {
      const saved = await api.get();
      state.update((s) => ({ ...s, ...saved, ...edits }));
      return saved;
    })().catch((e) => {
      initial = undefined;
      throw e;
    }));
  }

  function set(key: keyof Prefs, on: boolean): Promise<void> {
    const snapshot = { ...edits, [key]: on };
    edits = snapshot;
    if (api) {
      // Queue each snapshot before notifying subscribers. Wait for untouched fields to load.
      chain = chain.then(async () => {
        try {
          // A save can join the startup read. Retry if that read fails before writing.
          const saved = await (initial?.catch(() => loadInitial()) ?? loadInitial());
          await api.set({ ...saved, ...snapshot });
          state.update((s) => ({ ...s, error: null }));
        } catch (e) {
          state.update((s) => ({ ...s, error: errorMessage(e) }));
        }
      });
    }
    const saved = chain;
    state.update((s) => ({ ...s, [key]: on }));
    return saved;
  }

  return {
    store: { subscribe: state.subscribe },
    vim: () => current.vim,
    auditionNotes: () => current.auditionNotes,
    formatOnSave: () => current.formatOnSave,
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
  };
}

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
  /** Loads the initial preference snapshot. Never rejects. */
  load(): Promise<void>;
  /** Never rejects. */
  setVim(on: boolean): Promise<void>;
  /** Never rejects. */
  setAuditionNotes(on: boolean): Promise<void>;
}

export function createPrefs(api: PrefsApi | null): PrefsStore {
  const state = writable<PrefsState>({ ...DEFAULT_PREFS, error: null });
  // One subscription keeps both synchronous accessors current.
  let current: Prefs = DEFAULT_PREFS;
  state.subscribe((s) => (current = s));
  let edits: Partial<Prefs> = {};
  let initial: Promise<Prefs> | undefined;
  let chain: Promise<void> = Promise.resolve();

  function loadInitial(): Promise<Prefs> {
    return (initial ??= (async () => {
      if (!api) return DEFAULT_PREFS;
      try {
        const saved = await api.get();
        state.update((s) => ({ ...s, ...saved, ...edits }));
        return saved;
      } catch (e) {
        console.error('could not load preferences', e);
        return DEFAULT_PREFS;
      }
    })());
  }

  function set(key: keyof Prefs, on: boolean): Promise<void> {
    const snapshot = { ...edits, [key]: on };
    edits = snapshot;
    if (api) {
      // Queue each snapshot before notifying subscribers. Wait for untouched fields to load.
      chain = chain.then(async () => {
        try {
          await api.set({ ...(await loadInitial()), ...snapshot });
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
    async load() {
      await loadInitial();
    },
    setVim: (on) => set('vim', on),
    setAuditionNotes: (on) => set('auditionNotes', on),
  };
}

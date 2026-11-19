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
  /** Never rejects. */
  load(): Promise<void>;
  /** Never rejects. */
  setVim(on: boolean): Promise<void>;
}

export function createPrefs(api: PrefsApi | null): PrefsStore {
  const state = writable<PrefsState>({ ...DEFAULT_PREFS, error: null });
  // One subscription for the store's lifetime, so vim() reads without subscribing per call.
  let current = DEFAULT_PREFS.vim;
  state.subscribe((s) => (current = s.vim));
  let touched = false;
  let chain: Promise<void> = Promise.resolve();

  return {
    store: { subscribe: state.subscribe },
    vim: () => current,
    async load() {
      if (!api) return;
      try {
        const prefs = await api.get();
        if (!touched) state.update((s) => ({ ...s, vim: prefs.vim }));
      } catch (e) {
        console.error('could not load preferences', e);
      }
    },
    setVim(on) {
      touched = true;
      state.update((s) => ({ ...s, vim: on }));
      if (!api) return Promise.resolve();
      const run = async () => {
        try {
          await api.set({ vim: on });
          state.update((s) => ({ ...s, error: null }));
        } catch (e) {
          state.update((s) => ({ ...s, error: errorMessage(e) }));
        }
      };
      const next = chain.then(run);
      chain = next;
      return next;
    },
  };
}

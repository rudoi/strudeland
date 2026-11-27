import { invokeCommand } from '../ipc/invoke';

// The only module that calls Tauri for app preferences. Mirrors the Rust `Prefs`.
export interface Prefs {
  vim: boolean;
  auditionNotes: boolean;
  formatOnSave: boolean;
  pianoRollOpen: boolean;
  limiter: boolean;
}
export const DEFAULT_PREFS: Prefs = {
  vim: false,
  auditionNotes: false,
  formatOnSave: false,
  pianoRollOpen: false,
  limiter: false,
};

export interface PrefsApi {
  /** Best effort on the Rust side: a missing or corrupt file gives the defaults. */
  get(): Promise<Prefs>;
  /** Replaces the whole prefs object. */
  set(prefs: Prefs): Promise<void>;
}

export function createTauriPrefsApi(): PrefsApi {
  return {
    get: () => invokeCommand('prefs_get') as Promise<Prefs>,
    set: (prefs) => invokeCommand('prefs_set', { prefs }) as Promise<void>,
  };
}

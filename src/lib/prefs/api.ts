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

/**
 * What `prefs_get` returns. `clean` is false when a bad file (unreadable, corrupt, another
 * schema) gave the defaults; a missing file is clean. Only `prefs` is ever sent back.
 */
export interface PrefsLoad {
  prefs: Prefs;
  clean: boolean;
}

export interface PrefsApi {
  /** Best effort on the Rust side: a missing or corrupt file gives the defaults. */
  get(): Promise<PrefsLoad>;
  /** Replaces the whole prefs object. */
  set(prefs: Prefs): Promise<void>;
}

export function createTauriPrefsApi(): PrefsApi {
  return {
    get: () => invokeCommand('prefs_get') as Promise<PrefsLoad>,
    set: (prefs) => invokeCommand('prefs_set', { prefs }) as Promise<void>,
  };
}

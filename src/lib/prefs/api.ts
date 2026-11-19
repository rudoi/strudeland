import { invoke } from '@tauri-apps/api/core';

// The only module that calls Tauri for app preferences. Mirrors the Rust `Prefs`.
export interface Prefs {
  vim: boolean;
}
export const DEFAULT_PREFS: Prefs = { vim: false };

export interface PrefsApi {
  /** Best effort on the Rust side: a missing or corrupt file gives the defaults. */
  get(): Promise<Prefs>;
  /** Replaces the whole prefs object. */
  set(prefs: Prefs): Promise<void>;
}

export function createTauriPrefsApi(): PrefsApi {
  return {
    get: () => invoke<Prefs>('prefs_get'),
    set: (prefs) => invoke<void>('prefs_set', { prefs }),
  };
}

import { DEFAULT_PREFS, type Prefs, type PrefsApi } from '../../src/lib/prefs/api';

export interface FakePrefsApi extends PrefsApi {
  /** Every prefs object passed to `set`, in order of completion. */
  saved: Prefs[];
  failGet: boolean;
  failSet: boolean;
  /** The next `get` waits for the returned `resolve`. */
  deferGet(): { resolve: () => void };
}

export function fakePrefsApi(initial: Prefs = DEFAULT_PREFS): FakePrefsApi {
  let current = { ...initial };
  let gate: Promise<void> | null = null;
  const api: FakePrefsApi = {
    saved: [],
    failGet: false,
    failSet: false,
    deferGet() {
      let resolve!: () => void;
      gate = new Promise<void>((r) => (resolve = r));
      return { resolve };
    },
    async get() {
      const g = gate;
      gate = null;
      const snapshot = { ...current };
      if (g) await g;
      if (api.failGet) throw new Error('read failed');
      return snapshot;
    },
    async set(prefs) {
      if (api.failSet) throw new Error('write failed');
      current = { ...prefs };
      api.saved.push({ ...prefs });
    },
  };
  return api;
}

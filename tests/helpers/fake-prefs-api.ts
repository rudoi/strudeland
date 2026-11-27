import { DEFAULT_PREFS, type Prefs, type PrefsApi } from '../../src/lib/prefs/api';

export interface FakePrefsApi extends PrefsApi {
  /** Every prefs object passed to `set`, in order of completion. */
  saved: Prefs[];
  failGet: boolean;
  failSet: boolean;
  /** What the next `get` reports as `clean`; a bad file on the Rust side gives the defaults and false. */
  clean: boolean;
  /** The next `get` waits for the returned `resolve`. */
  deferGet(): { resolve: () => void };
}

export function fakePrefsApi(initial: Partial<Prefs> = {}): FakePrefsApi {
  let current = { ...DEFAULT_PREFS, ...initial };
  let gate: Promise<void> | null = null;
  const api: FakePrefsApi = {
    saved: [],
    failGet: false,
    failSet: false,
    clean: true,
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
      return api.clean
        ? { prefs: snapshot, clean: true }
        : { prefs: { ...DEFAULT_PREFS }, clean: false };
    },
    async set(prefs) {
      if (api.failSet) throw new Error('write failed');
      current = { ...prefs };
      api.saved.push({ ...prefs });
    },
  };
  return api;
}

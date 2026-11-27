import { describe, expect, it, spyOn } from 'bun:test';
import { get } from 'svelte/store';
import { migrateLimiter } from '../src/lib/prefs/migrate';
import { createPrefs, type PrefsStore } from '../src/lib/prefs/store';
import { fakePrefsApi } from './helpers/fake-prefs-api';
import { flush } from './helpers/fake-project-api';

describe('prefs store', () => {
  it('keeps a piano roll toggle made before saved preferences arrive', async () => {
    const api = fakePrefsApi({ vim: true });
    const gate = api.deferGet();
    const prefs = createPrefs(api);
    const loaded = prefs.load();
    const saved = prefs.setPianoRollOpen(true);
    expect(get(prefs.store).pianoRollOpen).toBe(true);
    gate.resolve();
    await Promise.all([loaded, saved]);
    expect(get(prefs.store)).toMatchObject({ pianoRollOpen: true, vim: true });
    expect(api.saved[0]).toMatchObject({ pianoRollOpen: true, vim: true });
  });

  it('starts with the defaults', () => {
    const p = createPrefs(fakePrefsApi());
    expect(p.vim()).toBe(false);
    expect(p.auditionNotes()).toBe(false);
    expect(p.formatOnSave()).toBe(false);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: false,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
      error: null,
    });
  });

  it('loads from the api', async () => {
    const p = createPrefs(
      fakePrefsApi({ vim: true, auditionNotes: true, formatOnSave: true, pianoRollOpen: false }),
    );
    await p.load();
    expect(p.vim()).toBe(true);
    expect(p.auditionNotes()).toBe(true);
    expect(p.formatOnSave()).toBe(true);
    expect(get(p.store)).toEqual({
      vim: true,
      auditionNotes: true,
      formatOnSave: true,
      pianoRollOpen: false,
      limiter: false,
      error: null,
    });
  });

  it('works in memory without an api', async () => {
    const p = createPrefs(null);
    await p.load();
    await p.setVim(true);
    await p.setAuditionNotes(true);
    await p.setFormatOnSave(true);
    expect(p.vim()).toBe(true);
    expect(p.auditionNotes()).toBe(true);
    expect(p.formatOnSave()).toBe(true);
    expect(get(p.store).error).toBeNull();
  });

  it('shows defaults after a failed startup load and reads saved Vim again before writing', async () => {
    const api = fakePrefsApi({ vim: true });
    api.failGet = true;
    const log = spyOn(console, 'error').mockImplementation(() => {});
    const p = createPrefs(api);
    try {
      await p.load();
      expect(log).toHaveBeenCalled();
    } finally {
      log.mockRestore();
    }
    expect(p.vim()).toBe(false);
    expect(get(p.store).error).toBeNull();
    api.failGet = false;
    await p.setAuditionNotes(true);
    expect(p.vim()).toBe(true);
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: true, formatOnSave: false, pianoRollOpen: false, limiter: false },
    ]);
  });

  it('retries a failed startup load that a pending save was waiting on', async () => {
    const api = fakePrefsApi({ vim: true });
    let rejectRead!: (e: Error) => void;
    const read = new Promise<never>((_, reject) => (rejectRead = reject));
    const getSaved = api.get;
    api.get = () => {
      api.get = getSaved;
      return read;
    };
    const log = spyOn(console, 'error').mockImplementation(() => {});
    const p = createPrefs(api);
    try {
      const loading = p.load();
      const saving = p.setAuditionNotes(true);
      await flush();
      expect(api.saved).toEqual([]);
      rejectRead(new Error('read failed'));
      await Promise.all([loading, saving]);
    } finally {
      log.mockRestore();
    }
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: true, formatOnSave: false, pianoRollOpen: false, limiter: false },
    ]);
    expect(get(p.store).error).toBeNull();
  });

  it('skips writes during read failure and preserves optimistic edits for the next save', async () => {
    const api = fakePrefsApi({ vim: true });
    api.failGet = true;
    const p = createPrefs(api);
    const saving = p.setAuditionNotes(true);
    expect(p.auditionNotes()).toBe(true);
    await saving;
    expect(api.saved).toEqual([]);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: true,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
      error: 'read failed',
    });

    api.failGet = false;
    await p.setVim(false);
    expect(api.saved).toEqual([
      {
        vim: false,
        auditionNotes: true,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
    ]);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: true,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
      error: null,
    });
  });

  it('saves and updates the store at once', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    const done = p.setVim(true);
    expect(p.vim()).toBe(true);
    await done;
    expect(api.saved).toEqual([
      {
        vim: true,
        auditionNotes: false,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
    ]);
  });

  it('setting audition before an explicit load retains the saved Vim preference', async () => {
    const api = fakePrefsApi({ vim: true });
    const p = createPrefs(api);
    const done = p.setAuditionNotes(true);
    expect(p.auditionNotes()).toBe(true);
    await done;
    expect(p.vim()).toBe(true);
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: true, formatOnSave: false, pianoRollOpen: false, limiter: false },
    ]);
    const reopened = createPrefs(api);
    await reopened.load();
    expect(reopened.auditionNotes()).toBe(true);
  });

  it('sets error on a failed save, keeps the value, and clears it on the next good save', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    api.failSet = true;
    await p.setVim(true);
    expect(p.vim()).toBe(true);
    expect(get(p.store).error).toBe('write failed');
    api.failSet = false;
    await p.setVim(false);
    expect(get(p.store).error).toBeNull();
    expect(api.saved).toEqual([
      {
        vim: false,
        auditionNotes: false,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
    ]);
  });

  it('keeps both edits while loading and writes them in order once the saved prefs are known', async () => {
    const api = fakePrefsApi({
      vim: true,
      auditionNotes: true,
      formatOnSave: false,
      pianoRollOpen: false,
    });
    const gate = api.deferGet();
    const p = createPrefs(api);
    const loading = p.load();
    const vimSave = p.setVim(false);
    const auditionSave = p.setAuditionNotes(false);
    await flush();
    expect(api.saved).toEqual([]);
    gate.resolve();
    await loading;
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: false,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
      error: null,
    });
    await Promise.all([vimSave, auditionSave]);
    expect(api.saved).toEqual([
      {
        vim: false,
        auditionNotes: true,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
      {
        vim: false,
        auditionNotes: false,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
    ]);
  });

  it('saves in order, last wins', async () => {
    const api = fakePrefsApi();
    const order: boolean[] = [];
    const set = api.set;
    api.set = async (prefs) => {
      // the first save is slower than the second would be if they overlapped
      if (prefs.vim) await flush();
      order.push(prefs.vim);
      return set(prefs);
    };
    const p = createPrefs(api);
    const a = p.setVim(true);
    const b = p.setVim(false);
    await Promise.all([a, b]);
    expect(order).toEqual([true, false]);
    expect(api.saved.at(-1)).toEqual({
      vim: false,
      auditionNotes: false,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
    });
  });

  it('saves alternating toggles as complete ordered snapshots after hydration', async () => {
    const api = fakePrefsApi({ vim: true, auditionNotes: true });
    const gate = api.deferGet();
    const set = api.set;
    api.set = async (prefs) => {
      if (prefs.vim) await flush();
      return set(prefs);
    };
    const p = createPrefs(api);
    const loading = p.load();
    const enableFormat = p.setFormatOnSave(true);
    expect(p.formatOnSave()).toBe(true);
    const disableVim = p.setVim(false);
    const disableFormat = p.setFormatOnSave(false);
    expect(p.formatOnSave()).toBe(false);
    await flush();
    expect(api.saved).toEqual([]);
    gate.resolve();
    await Promise.all([loading, enableFormat, disableVim, disableFormat]);
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: true, formatOnSave: true, pianoRollOpen: false, limiter: false },
      { vim: false, auditionNotes: true, formatOnSave: true, pianoRollOpen: false, limiter: false },
      {
        vim: false,
        auditionNotes: true,
        formatOnSave: false,
        pianoRollOpen: false,
        limiter: false,
      },
    ]);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: true,
      formatOnSave: false,
      pianoRollOpen: false,
      limiter: false,
      error: null,
    });
  });

  it('a failed save does not block later saves', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    const set = api.set;
    api.set = (prefs) => (prefs.auditionNotes ? set(prefs) : Promise.reject(new Error('nope')));
    const a = p.setVim(true);
    const b = p.setAuditionNotes(true);
    await Promise.all([a, b]);
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: true, formatOnSave: false, pianoRollOpen: false, limiter: false },
    ]);
  });

  it('sets the limiter and updates the store at once', async () => {
    const api = fakePrefsApi({ vim: true });
    const p = createPrefs(api);
    await p.load();
    const done = p.setLimiter(true);
    expect(p.limiter()).toBe(true);
    expect(get(p.store).limiter).toBe(true);
    await done;
    expect(api.saved).toEqual([
      { vim: true, auditionNotes: false, formatOnSave: false, pianoRollOpen: false, limiter: true },
    ]);
  });

  it('knows which keys this session set', async () => {
    const p = createPrefs(fakePrefsApi());
    expect(p.edited('limiter')).toBe(false);
    expect(p.edited('vim')).toBe(false);
    await p.setLimiter(false);
    expect(p.edited('limiter')).toBe(true);
    expect(p.edited('vim')).toBe(false);
    expect(p.edited('auditionNotes')).toBe(false);
  });

  it('resolves whether the save landed', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    expect(await p.setVim(true)).toBe(true);
    api.failSet = true;
    expect(await p.setAuditionNotes(true)).toBe(false);
    expect(await p.setLimiter(true)).toBe(false);
    expect(await createPrefs(null).setLimiter(true)).toBe(false);
  });
});

describe('limiter migration', () => {
  const KEY = 'strudeland.limiter';

  function fakeStorage(value?: string) {
    const map = new Map<string, string>();
    if (value !== undefined) map.set(KEY, value);
    const storage = {
      map,
      removed: 0,
      getItem: (key: string) => map.get(key) ?? null,
      removeItem: (key: string) => {
        storage.removed++;
        map.delete(key);
      },
    };
    return storage;
  }

  async function loaded(initial: Parameters<typeof fakePrefsApi>[0] = {}) {
    const api = fakePrefsApi(initial);
    const prefs = createPrefs(api);
    await prefs.load();
    return { api, prefs };
  }

  it("moves '1' into prefs and removes the key", async () => {
    const { api, prefs } = await loaded();
    const storage = fakeStorage('1');
    await migrateLimiter(prefs, () => storage);
    expect(prefs.limiter()).toBe(true);
    expect(api.saved.map((s) => s.limiter)).toEqual([true]);
    expect(storage.map.has(KEY)).toBe(false);
  });

  it("moves '0' over a loaded limiter and removes the key", async () => {
    const { api, prefs } = await loaded({ limiter: true });
    expect(prefs.limiter()).toBe(true);
    const storage = fakeStorage('0');
    await migrateLimiter(prefs, () => storage);
    expect(prefs.limiter()).toBe(false);
    expect(api.saved.map((s) => s.limiter)).toEqual([false]);
    expect(storage.map.has(KEY)).toBe(false);
  });

  it('does nothing when the key is absent', async () => {
    const { api, prefs } = await loaded();
    const storage = fakeStorage();
    await migrateLimiter(prefs, () => storage);
    expect(api.saved).toEqual([]);
    expect(storage.removed).toBe(0);
  });

  it('leaves a value other than 1 or 0', async () => {
    const { api, prefs } = await loaded();
    const storage = fakeStorage('yes');
    await migrateLimiter(prefs, () => storage);
    expect(api.saved).toEqual([]);
    expect(prefs.limiter()).toBe(false);
    expect(storage.map.get(KEY)).toBe('yes');
  });

  it('does nothing when getItem throws', async () => {
    const { api, prefs } = await loaded();
    const storage = {
      getItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('not reached');
      },
    };
    await migrateLimiter(prefs, () => storage);
    expect(api.saved).toEqual([]);
  });

  it('does nothing when the storage accessor throws', async () => {
    const { api, prefs } = await loaded();
    await migrateLimiter(prefs, () => {
      throw new Error('denied');
    });
    expect(api.saved).toEqual([]);
  });

  it('does nothing without storage', async () => {
    const { api, prefs } = await loaded();
    await migrateLimiter(prefs, () => undefined);
    expect(api.saved).toEqual([]);
  });

  it('keeps the key when the save fails', async () => {
    const { api, prefs } = await loaded();
    api.failSet = true;
    const storage = fakeStorage('1');
    await migrateLimiter(prefs, () => storage);
    expect(prefs.limiter()).toBe(true);
    expect(get(prefs.store).error).toBe('write failed');
    expect(api.saved).toEqual([]);
    expect(storage.map.get(KEY)).toBe('1');
  });

  async function toggledBeforeLoad(): Promise<{
    api: ReturnType<typeof fakePrefsApi>;
    prefs: PrefsStore;
    toggle: Promise<boolean>;
  }> {
    const api = fakePrefsApi();
    const gate = api.deferGet();
    const prefs = createPrefs(api);
    const loading = prefs.load();
    const toggle = prefs.setLimiter(false);
    gate.resolve();
    await loading;
    return { api, prefs, toggle };
  }

  it('keeps a toggle made before the load and removes the key', async () => {
    const { api, prefs, toggle } = await toggledBeforeLoad();
    const storage = fakeStorage('1');
    await migrateLimiter(prefs, () => storage);
    await toggle;
    expect(prefs.limiter()).toBe(false);
    expect(api.saved.map((s) => s.limiter)).toEqual([false, false]);
    expect(api.saved[0]).toEqual(api.saved[1]);
    expect(storage.map.has(KEY)).toBe(false);
  });

  it('keeps the key when a toggle made before the load fails to save', async () => {
    const api = fakePrefsApi();
    api.failSet = true;
    const gate = api.deferGet();
    const prefs = createPrefs(api);
    const loading = prefs.load();
    const toggle = prefs.setLimiter(false);
    gate.resolve();
    await loading;
    const storage = fakeStorage('1');
    await migrateLimiter(prefs, () => storage);
    expect(await toggle).toBe(false);
    expect(prefs.limiter()).toBe(false);
    expect(get(prefs.store).error).toBe('write failed');
    expect(api.saved).toEqual([]);
    expect(storage.map.get(KEY)).toBe('1');
  });

  it('logs and does not throw when removeItem throws after a good save', async () => {
    const { api, prefs } = await loaded();
    const storage = {
      getItem: () => '1',
      removeItem: () => {
        throw new Error('read only');
      },
    };
    const log = spyOn(console, 'error').mockImplementation(() => {});
    try {
      await migrateLimiter(prefs, () => storage);
      expect(log).toHaveBeenCalledTimes(1);
      expect(log.mock.calls[0][0]).toBe('could not remove the old limiter preference');
    } finally {
      log.mockRestore();
    }
    expect(api.saved.map((s) => s.limiter)).toEqual([true]);
  });
});

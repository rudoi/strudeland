import { describe, expect, it, spyOn } from 'bun:test';
import { get } from 'svelte/store';
import { createPrefs } from '../src/lib/prefs/store';
import { fakePrefsApi } from './helpers/fake-prefs-api';
import { flush } from './helpers/fake-project-api';

describe('prefs store', () => {
  it('starts with the defaults', () => {
    const p = createPrefs(fakePrefsApi());
    expect(p.vim()).toBe(false);
    expect(p.auditionNotes()).toBe(false);
    expect(p.formatOnSave()).toBe(false);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: false,
      formatOnSave: false,
      error: null,
    });
  });

  it('loads from the api', async () => {
    const p = createPrefs(fakePrefsApi({ vim: true, auditionNotes: true, formatOnSave: true }));
    await p.load();
    expect(p.vim()).toBe(true);
    expect(p.auditionNotes()).toBe(true);
    expect(p.formatOnSave()).toBe(true);
    expect(get(p.store)).toEqual({
      vim: true,
      auditionNotes: true,
      formatOnSave: true,
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
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true, formatOnSave: false }]);
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
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true, formatOnSave: false }]);
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
      error: 'read failed',
    });

    api.failGet = false;
    await p.setVim(false);
    expect(api.saved).toEqual([{ vim: false, auditionNotes: true, formatOnSave: false }]);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: true,
      formatOnSave: false,
      error: null,
    });
  });

  it('saves and updates the store at once', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    const done = p.setVim(true);
    expect(p.vim()).toBe(true);
    await done;
    expect(api.saved).toEqual([{ vim: true, auditionNotes: false, formatOnSave: false }]);
  });

  it('setting audition before an explicit load retains the saved Vim preference', async () => {
    const api = fakePrefsApi({ vim: true });
    const p = createPrefs(api);
    const done = p.setAuditionNotes(true);
    expect(p.auditionNotes()).toBe(true);
    await done;
    expect(p.vim()).toBe(true);
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true, formatOnSave: false }]);
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
    expect(api.saved).toEqual([{ vim: false, auditionNotes: false, formatOnSave: false }]);
  });

  it('keeps both edits while loading and writes them in order once the saved prefs are known', async () => {
    const api = fakePrefsApi({ vim: true, auditionNotes: true, formatOnSave: false });
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
      error: null,
    });
    await Promise.all([vimSave, auditionSave]);
    expect(api.saved).toEqual([
      { vim: false, auditionNotes: true, formatOnSave: false },
      { vim: false, auditionNotes: false, formatOnSave: false },
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
    expect(api.saved.at(-1)).toEqual({ vim: false, auditionNotes: false, formatOnSave: false });
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
      { vim: true, auditionNotes: true, formatOnSave: true },
      { vim: false, auditionNotes: true, formatOnSave: true },
      { vim: false, auditionNotes: true, formatOnSave: false },
    ]);
    expect(get(p.store)).toEqual({
      vim: false,
      auditionNotes: true,
      formatOnSave: false,
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
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true, formatOnSave: false }]);
  });
});

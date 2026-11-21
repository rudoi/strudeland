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
    expect(get(p.store)).toEqual({ vim: false, auditionNotes: false, error: null });
  });

  it('loads from the api', async () => {
    const p = createPrefs(fakePrefsApi({ vim: true, auditionNotes: true }));
    await p.load();
    expect(p.vim()).toBe(true);
    expect(p.auditionNotes()).toBe(true);
    expect(get(p.store)).toEqual({ vim: true, auditionNotes: true, error: null });
  });

  it('works in memory without an api', async () => {
    const p = createPrefs(null);
    await p.load();
    await p.setVim(true);
    await p.setAuditionNotes(true);
    expect(p.vim()).toBe(true);
    expect(p.auditionNotes()).toBe(true);
    expect(get(p.store).error).toBeNull();
  });

  it('keeps the defaults when the load fails, without rejecting', async () => {
    const api = fakePrefsApi({ vim: true });
    api.failGet = true;
    const log = spyOn(console, 'error').mockImplementation(() => {});
    const p = createPrefs(api);
    await p.load();
    expect(p.vim()).toBe(false);
    expect(get(p.store).error).toBeNull();
    expect(log).toHaveBeenCalled();
    log.mockRestore();
  });

  it('saves and updates the store at once', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    const done = p.setVim(true);
    expect(p.vim()).toBe(true);
    await done;
    expect(api.saved).toEqual([{ vim: true, auditionNotes: false }]);
  });

  it('setting audition before an explicit load retains the saved Vim preference', async () => {
    const api = fakePrefsApi({ vim: true });
    const p = createPrefs(api);
    const done = p.setAuditionNotes(true);
    expect(p.auditionNotes()).toBe(true);
    await done;
    expect(p.vim()).toBe(true);
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true }]);
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
    expect(api.saved).toEqual([{ vim: false, auditionNotes: false }]);
  });

  it('keeps both edits while loading and writes them in order once the saved prefs are known', async () => {
    const api = fakePrefsApi({ vim: true, auditionNotes: true });
    const gate = api.deferGet();
    const p = createPrefs(api);
    const loading = p.load();
    const vimSave = p.setVim(false);
    const auditionSave = p.setAuditionNotes(false);
    await flush();
    expect(api.saved).toEqual([]);
    gate.resolve();
    await loading;
    expect(get(p.store)).toEqual({ vim: false, auditionNotes: false, error: null });
    await Promise.all([vimSave, auditionSave]);
    expect(api.saved).toEqual([
      { vim: false, auditionNotes: true },
      { vim: false, auditionNotes: false },
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
    expect(api.saved.at(-1)).toEqual({ vim: false, auditionNotes: false });
  });

  it('a failed save does not block later saves', async () => {
    const api = fakePrefsApi();
    const p = createPrefs(api);
    const set = api.set;
    api.set = (prefs) => (prefs.auditionNotes ? set(prefs) : Promise.reject(new Error('nope')));
    const a = p.setVim(true);
    const b = p.setAuditionNotes(true);
    await Promise.all([a, b]);
    expect(api.saved).toEqual([{ vim: true, auditionNotes: true }]);
  });
});

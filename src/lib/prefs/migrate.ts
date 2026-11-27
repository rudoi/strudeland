/**
 * The one-time move of the old `localStorage` limiter value into prefs; the only module that
 * touches `localStorage`. Once no installed build has the key, delete this file and its call.
 */
import type { PrefsStore } from './store';

const KEY = 'strudeland.limiter';

/** The webview's `localStorage`; the app passes it to `migrateLimiter`. */
export const webStorage = (): Storage | undefined => globalThis.localStorage;

/**
 * Moves the old `localStorage` limiter value into prefs once. Call it after `prefs.load()`
 * settled. It saves only after a clean load, so a bad `prefs.json` that gave the defaults is
 * never overwritten unasked, and a failed load skips it. After an unclean or failed load it does
 * nothing at all, not even in memory (the store's state stays the loaded prefs plus the user's
 * edits, which every save writes), and the key stays for a later launch. The key is removed only
 * after a save landed; a toggle made before the load wins over the stored value. Never rejects.
 */
export async function migrateLimiter(
  prefs: PrefsStore,
  storage: () => Pick<Storage, 'getItem' | 'removeItem'> | undefined,
): Promise<void> {
  let store: Pick<Storage, 'getItem' | 'removeItem'> | undefined;
  let value: string | null;
  try {
    store = storage();
    value = store?.getItem(KEY) ?? null;
  } catch {
    return;
  }
  if (!store || value === null) return;
  if (value !== '1' && value !== '0') return;
  if (!prefs.loadedClean()) return;
  // The user already chose this session: save that value again so a failed save keeps the key.
  const on = prefs.edited('limiter') ? prefs.limiter() : value === '1';
  if (!(await prefs.setLimiter(on))) return;
  try {
    store.removeItem(KEY);
  } catch (e) {
    console.error('could not remove the old limiter preference', e);
  }
}

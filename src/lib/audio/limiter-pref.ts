/** The safety limiter preference: one setting per machine, in `localStorage`. */

const KEY = 'strudeland.limiter';

/** Off when nothing is stored or storage throws. */
export function loadLimiter(): boolean {
  try {
    return localStorage.getItem(KEY) === '1';
  } catch {
    return false;
  }
}

export function saveLimiter(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? '1' : '0');
  } catch {
    // Private window or blocked storage: the preference lasts until the page closes.
  }
}

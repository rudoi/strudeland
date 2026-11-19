import { describe, expect, test } from 'bun:test';
import { followExternal } from '../src/components/ui/link';

function click(prevented = false) {
  const e = {
    defaultPrevented: prevented,
    preventDefault() {
      e.defaultPrevented = true;
    },
  };
  return e;
}

describe('followExternal', () => {
  test('opens the href outside and stops the webview navigating', () => {
    const opened: string[] = [];
    const e = click();
    expect(followExternal(e, 'https://strudel.cc', (u) => void opened.push(u))).toBe(true);
    expect(e.defaultPrevented).toBe(true);
    expect(opened).toEqual(['https://strudel.cc']);
  });

  test('leaves a click a handler already prevented alone', () => {
    const opened: string[] = [];
    expect(followExternal(click(true), 'https://x.test', (u) => void opened.push(u))).toBe(false);
    expect(opened).toEqual([]);
  });

  test('does not wait for or throw on a rejected open', async () => {
    const e = click();
    let rejected = false;
    const open = () =>
      Promise.reject(new Error('no opener')).catch(() => {
        rejected = true;
      });
    expect(followExternal(e, 'https://x.test', open)).toBe(true);
    await Promise.resolve();
    expect(rejected).toBe(true);
  });
});

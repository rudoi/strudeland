import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { resetResolveProbe, resolveColor } from '../src/lib/theme/resolve';
import { catOf } from '../src/lib/theme/tones';

describe('resolveColor without a document', () => {
  test('returns transparent', () => {
    const g = globalThis as { document?: unknown };
    const saved = Object.getOwnPropertyDescriptor(g, 'document');
    if (saved) delete g.document;
    try {
      expect(typeof document).toBe('undefined');
      expect(resolveColor('--accent')).toBe('transparent');
    } finally {
      if (saved) Object.defineProperty(g, 'document', saved);
    }
  });
});

describe('resolveColor under happy-dom', () => {
  beforeAll(() => {
    GlobalRegistrator.register();
    resetResolveProbe();
  });
  afterAll(async () => {
    resetResolveProbe();
    await GlobalRegistrator.unregister();
  });

  const probes = () =>
    [...document.documentElement.children].filter(
      (el) => el.getAttribute('aria-hidden') === 'true',
    );

  test('an empty computed value is transparent', () => {
    expect(resolveColor('--no-such-token')).toBe('transparent');
  });

  test('a computed value is returned trimmed', () => {
    const real = window.getComputedStyle;
    window.getComputedStyle = (() => ({ color: '  rgb(1, 2, 3) ' })) as never;
    try {
      expect(resolveColor('--accent')).toBe('rgb(1, 2, 3)');
    } finally {
      window.getComputedStyle = real;
    }
  });

  test('the probe reads the token through var() with a transparent fallback', () => {
    let seen = '';
    const real = window.getComputedStyle;
    window.getComputedStyle = ((el: HTMLElement) => {
      seen = el.style.color;
      return { color: '' };
    }) as never;
    try {
      resolveColor('--scope-trace');
    } finally {
      window.getComputedStyle = real;
    }
    // Without the fallback a missing token would inherit the root colour in a real browser.
    expect(seen).toBe('var(--scope-trace, transparent)');
  });

  test('the probe element is created once', () => {
    resetResolveProbe();
    const before = probes().length;
    resolveColor('--accent');
    resolveColor('--focus');
    resolveColor('--warning');
    expect(probes().length).toBe(before + 1);
  });

  test('a removed probe is recreated', () => {
    resolveColor('--accent');
    probes().forEach((el) => el.remove());
    resolveColor('--accent');
    expect(probes().length).toBe(1);
  });
});

describe('catOf', () => {
  test('cycles 1, 2, 3, 4, 1', () => {
    expect([0, 1, 2, 3, 4, 5, 8].map(catOf)).toEqual([1, 2, 3, 4, 1, 2, 1]);
  });

  test('a negative index stays in range', () => {
    expect([-1, -4, -5].map(catOf)).toEqual([4, 1, 4]);
  });
});

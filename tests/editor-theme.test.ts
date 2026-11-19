import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { SETTINGS, createEditorState } = await import('../src/lib/editor/setup');
const { STRUDELAND_THEME, registerStrudelandSettings, strudelandSettings, strudelandTheme } =
  await import('../src/lib/editor/theme');
const { settings, theme, themes } = await import('@strudel/codemirror');
const { EditorView } = await import('@codemirror/view');
import { stubRuntime } from './helpers/stub-runtime';

const KEYS = [
  'background',
  'gutterBackground',
  'lineBackground',
  'foreground',
  'caret',
  'selection',
  'selectionMatch',
  'lineHighlight',
  'gutterForeground',
  'muted',
];

/** A resolver that echoes the token, so each value shows which token it came from. */
const echo = (token: string) => `resolved(${token})`;

describe('the theme', () => {
  test('is registered as themes.strudeland once setup is imported', () => {
    expect(STRUDELAND_THEME).toBe('strudeland');
    expect(themes.strudeland).toBe(strudelandTheme);
    expect(theme('strudeland')).toBe(strudelandTheme);
  });

  test('SETTINGS.theme names it', () => {
    expect(SETTINGS.theme).toBe('strudeland');
  });

  test('an editor state built from the settings uses it, as a dark theme', () => {
    const state = createEditorState('note("c")', stubRuntime().runtime);
    expect(state.facet(EditorView.darkTheme)).toBe(true);
    const view = new EditorView({ state, parent: document.body });
    try {
      expect(view.dom.classList.contains('cm-editor')).toBe(true);
    } finally {
      view.destroy();
    }
  });
});

describe('strudelandSettings', () => {
  test('has exactly the ten keys Strudel reads, and no light key', () => {
    const s = strudelandSettings(echo);
    expect(Object.keys(s).sort()).toEqual([...KEYS].sort());
    expect('light' in s).toBe(false);
  });

  test('no value is a CSS variable', () => {
    for (const value of Object.values(strudelandSettings(echo)))
      expect(value).not.toContain('var(');
  });

  test('maps each key to its token; the backgrounds stay transparent', () => {
    expect(strudelandSettings(echo)).toEqual({
      background: 'transparent',
      gutterBackground: 'transparent',
      lineBackground: 'transparent',
      foreground: 'resolved(--accent)',
      caret: 'resolved(--editor-cursor)',
      selection: 'resolved(--editor-selection)',
      selectionMatch: 'resolved(--editor-selection)',
      lineHighlight: 'resolved(--editor-active-line)',
      gutterForeground: 'resolved(--editor-line-number)',
      muted: 'resolved(--text-faint)',
    });
  });

  test('defaults to resolveColor (under happy-dom every token resolves to a string)', () => {
    const s = strudelandSettings();
    expect(Object.keys(s).sort()).toEqual([...KEYS].sort());
    for (const value of Object.values(s)) {
      expect(typeof value).toBe('string');
      expect(value.length).toBeGreaterThan(0);
      expect(value).not.toContain('var(');
    }
  });
});

describe('registerStrudelandSettings', () => {
  test('sets settings.strudeland and is idempotent', () => {
    delete settings.strudeland;
    registerStrudelandSettings(echo);
    const first = { ...settings.strudeland };
    expect(first).toEqual(strudelandSettings(echo));
    registerStrudelandSettings(echo);
    expect(settings.strudeland).toEqual(first);
    expect(themes.strudeland).toBe(strudelandTheme);
  });

  test('a later call overwrites with fresh values', () => {
    registerStrudelandSettings(echo);
    registerStrudelandSettings(() => 'fresh');
    expect(settings.strudeland.foreground).toBe('fresh');
    expect(settings.strudeland.background).toBe('transparent');
  });

  test('leaves the other themes alone', () => {
    const before = settings.strudelTheme;
    registerStrudelandSettings(echo);
    expect(settings.strudelTheme).toBe(before);
  });
});

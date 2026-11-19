/**
 * The strudeland CodeMirror theme. Colours are `var(--…)` strings naming tokens from
 * `src/styles/tokens.css`, so the theme itself needs no DOM and is registered with Strudel at module
 * load. Strudel's settings object (written onto `:root` with `!important` and handed to
 * `@strudel/draw`'s painters) must hold real colours, so it is resolved later, at editor creation.
 */
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { tags as t } from '@lezer/highlight';
import { settings, themes } from '@strudel/codemirror';
import { resolveColor } from '../theme/resolve';

/** The name the theme and its settings are registered under in `@strudel/codemirror`. */
export const STRUDELAND_THEME = 'strudeland';

const editorTheme = EditorView.theme(
  {
    '&': {
      backgroundColor: 'transparent',
      color: 'var(--syntax-text)',
    },
    '.cm-scroller': {
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-code)',
      lineHeight: 'var(--lh-code)',
    },
    '.cm-content': {
      caretColor: 'var(--editor-cursor)',
    },
    '.cm-cursor, .cm-dropCursor': {
      borderLeftColor: 'var(--editor-cursor)',
    },
    '.cm-gutters': {
      backgroundColor: 'transparent',
      color: 'var(--editor-line-number)',
      border: 'none',
    },
    '.cm-activeLine': {
      backgroundColor: 'var(--editor-active-line)',
    },
    '.cm-activeLineGutter': {
      backgroundColor: 'var(--editor-active-line)',
      color: 'var(--editor-line-number-active)',
    },
    // The same specificity as the base theme's focused rule, so this wins without `!important`.
    '& .cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground':
      {
        background: 'var(--editor-selection)',
      },
    '.cm-selectionMatch, &.cm-focused .cm-matchingBracket, .cm-matchingBracket': {
      backgroundColor: 'var(--editor-selection)',
    },
  },
  { dark: true },
);

/** The Lezer tags Strudel's `strudel-theme.mjs` styles, mapped to the syntax tokens. */
const highlightStyle = HighlightStyle.define([
  { tag: [t.atom, t.bool, t.special(t.variableName)], color: 'var(--syntax-number)' },
  { tag: t.labelName, color: 'var(--syntax-keyword)' },
  { tag: t.keyword, color: 'var(--syntax-keyword)' },
  { tag: t.operator, color: 'var(--syntax-operator)' },
  { tag: t.typeName, color: 'var(--syntax-fn)' },
  { tag: t.number, color: 'var(--syntax-number)' },
  { tag: t.definition(t.variableName), color: 'var(--syntax-fn)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--syntax-string)' },
  { tag: t.comment, color: 'var(--syntax-comment)', fontStyle: 'italic' },
  { tag: t.variableName, color: 'var(--syntax-text)' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: 'var(--syntax-fn)' },
  { tag: t.tagName, color: 'var(--syntax-keyword)' },
  { tag: t.bracket, color: 'var(--syntax-punct)' },
  { tag: t.meta, color: 'var(--syntax-keyword)' },
  { tag: [t.attributeName, t.propertyName], color: 'var(--syntax-property)' },
  { tag: t.className, color: 'var(--syntax-fn)' },
  { tag: t.invalid, color: 'var(--syntax-text)' },
  { tag: [t.unit, t.punctuation], color: 'var(--syntax-punct)' },
]);

export const strudelandTheme: Extension = [editorTheme, syntaxHighlighting(highlightStyle)];

// Registered at module load: `extensions.theme('strudeland')` looks it up when the state is built.
themes[STRUDELAND_THEME] = strudelandTheme;

/** The colours Strudel writes onto `:root` and hands to `@strudel/draw`, keyed as Strudel names them. */
export interface StrudelandSettings {
  background: string;
  gutterBackground: string;
  lineBackground: string;
  foreground: string;
  caret: string;
  selection: string;
  selectionMatch: string;
  lineHighlight: string;
  gutterForeground: string;
  muted: string;
}

/**
 * Strudel's settings for the theme, every value a resolved colour (never `var(`): `@strudel/draw`
 * paints with them on a canvas. `foreground` is the hap outline and the painters' default colour.
 */
export function strudelandSettings(
  resolve: (token: string) => string = resolveColor,
): StrudelandSettings {
  return {
    background: 'transparent',
    gutterBackground: 'transparent',
    lineBackground: 'transparent',
    foreground: resolve('--accent'),
    caret: resolve('--editor-cursor'),
    selection: resolve('--editor-selection'),
    selectionMatch: resolve('--editor-selection'),
    lineHighlight: resolve('--editor-active-line'),
    gutterForeground: resolve('--editor-line-number'),
    muted: resolve('--text-faint'),
  };
}

/**
 * Sets `settings.strudeland` to freshly resolved colours. Idempotent. Call after the stylesheets
 * apply and before `initTheme('strudeland')` (`activateTheme` ignores a repeated name).
 */
export function registerStrudelandSettings(resolve?: (token: string) => string): void {
  settings[STRUDELAND_THEME] = strudelandSettings(resolve);
}

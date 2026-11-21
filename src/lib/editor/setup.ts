import { claudeMark } from './claude-mark';
import { inspectNoteAudition, type NoteAuditionEdit } from './note-audition';
import { registerStrudelandSettings, STRUDELAND_THEME } from './theme';
import { exitInsert } from './vim-mode';
import { javascript, javascriptLanguage } from '@codemirror/lang-javascript';
import { Compartment, EditorState, Prec, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, keymap } from '@codemirror/view';
import { extensions, flash, initTheme, sliderPlugin, widgetPlugin } from '@strudel/codemirror';
import { basicSetup } from '@strudel/codemirror/basicSetup.mjs';
import type { Runtime } from '../strudel/runtime';

/** Fixed editor settings (we never read or write @strudel/codemirror's localStorage atom). */
export const SETTINGS = {
  theme: STRUDELAND_THEME,
  isLineNumbersDisplayed: true,
  isBracketClosingEnabled: true,
  isAutoCompletionEnabled: true,
  isTooltipEnabled: true,
  isPatternHighlightingEnabled: true,
  isFlashEnabled: true,
} as const;

/** Cmd-Enter: run `evaluate`, then flash. Not awaited: CodeMirror commands are sync. */
export const evaluateCommand = (evaluate: (view: EditorView) => void) => (view: EditorView) => {
  evaluate(view);
  flash(view);
  return true;
};

/** Evaluate the whole document through the runtime and start playing. */
export const runtimeEvaluate = (runtime: Runtime) => (view: EditorView) => {
  void runtime.evaluate(view.state.doc.toString());
};

/** Cmd-.: stop. */
export const stopCommand = (runtime: Runtime) => () => {
  runtime.stop();
  return true;
};

/** Optional callbacks a host (the project session) wires into the editor. */
export interface EditorHooks {
  /** Mod-s. */
  onSave?: () => void;
  /** Mod-Enter / Ctrl-Enter, in place of evaluating the document directly. */
  onEvaluate?: () => void;
  /** Any transaction that changes the document. */
  onDocChange?: () => void;
  /** Whether to inspect editor updates for an audition; read before parsing. */
  auditionEnabled?: () => boolean;
  /** A typed pitch or delimiter, or null when the edit invalidates a pending pitch. */
  onAuditionEdit?: (edit: NoteAuditionEdit | null) => void;
  /** Whether new states start with vim keybindings; read for each new state. Default: off. */
  vim?: () => boolean;
}

/** Holds the keymap (vim or CodeMirror's default) of every editor state, so it can be switched. */
export const keymapCompartment = new Compartment();

let vimKeymap: Extension | undefined;
let defaultKeymapExt: Extension | undefined;

/** The keymap compartment's content; the same object for the same `vim`, so identity compares. */
export function keymapFor(vim: boolean): Extension {
  if (vim) return (vimKeymap ??= extensions.keybindings('vim'));
  return (defaultKeymapExt ??= extensions.keybindings('codemirror'));
}

/** `state` with the given keymap; the same object when it already has it. */
export function withKeymap(state: EditorState, vim: boolean): EditorState {
  const want = keymapFor(vim);
  if (keymapCompartment.get(state) === want) return state;
  return state.update({ effects: keymapCompartment.reconfigure(want) }).state;
}

/**
 * Switch the view's keymap. False (and nothing done) when it already has it. Leaving vim from
 * insert mode exits insert mode first, so vim's insert-mode listeners do not outlive the plugin.
 */
export function setKeymap(view: EditorView, vim: boolean): boolean {
  const want = keymapFor(vim);
  if (keymapCompartment.get(view.state) === want) return false;
  if (!vim) exitInsert(view);
  view.dispatch({ effects: keymapCompartment.reconfigure(want) });
  return true;
}

export function strudelKeymap(runtime: Runtime, hooks?: EditorHooks): Extension {
  const onEvaluate = hooks?.onEvaluate;
  const evaluate = evaluateCommand(onEvaluate ? () => onEvaluate() : runtimeEvaluate(runtime));
  const stop = stopCommand(runtime);
  const onSave = hooks?.onSave;
  return Prec.highest(
    keymap.of([
      { key: 'Mod-Enter', run: evaluate },
      { key: 'Ctrl-Enter', run: evaluate },
      { key: 'Mod-.', run: stop, preventDefault: true },
      { key: 'Ctrl-.', run: stop, preventDefault: true },
      ...(onSave ? [{ key: 'Mod-s', run: () => (onSave(), true), preventDefault: true }] : []),
    ]),
  );
}

export function createExtensions(runtime: Runtime, hooks?: EditorHooks): Extension[] {
  const e = extensions;
  return [
    e.theme(SETTINGS.theme),
    keymapCompartment.of(keymapFor(hooks?.vim?.() ?? false)),
    e.isLineNumbersDisplayed(SETTINGS.isLineNumbersDisplayed),
    e.isBracketClosingEnabled(SETTINGS.isBracketClosingEnabled),
    e.isAutoCompletionEnabled(SETTINGS.isAutoCompletionEnabled),
    e.isTooltipEnabled(SETTINGS.isTooltipEnabled),
    e.isPatternHighlightingEnabled(SETTINGS.isPatternHighlightingEnabled),
    e.isFlashEnabled(SETTINGS.isFlashEnabled),
    basicSetup,
    javascript(),
    javascriptLanguage.data.of({
      closeBrackets: { brackets: ['(', '[', '{', "'", '"', '<'] },
    }),
    sliderPlugin,
    widgetPlugin,
    drawSelection({ cursorBlinkRate: 0 }),
    strudelKeymap(runtime, hooks),
    ...(hooks?.onDocChange ? [docChangeListener(hooks.onDocChange)] : []),
    ...(hooks?.onAuditionEdit
      ? [
          EditorView.updateListener.of((update) => {
            if (!hooks.auditionEnabled?.() || (!update.docChanged && !update.selectionSet)) return;
            const [transaction] = update.transactions;
            hooks.onAuditionEdit?.(
              update.docChanged && update.transactions.length === 1
                ? inspectNoteAudition(transaction)
                : null,
            );
          }),
        ]
      : []),
    claudeMark,
  ];
}

function docChangeListener(onDocChange: () => void): Extension {
  return EditorView.updateListener.of((update) => {
    if (update.docChanged) onDocChange();
  });
}

/** A state with the Strudel extensions, so songs can be swapped into one view with `setState`. */
export function createEditorState(doc: string, runtime: Runtime, hooks?: EditorHooks): EditorState {
  return EditorState.create({ doc, extensions: createExtensions(runtime, hooks) });
}

/** Needs `document`. Call exactly once per window, before the first EditorView exists. */
export function createEditor(
  parent: HTMLElement,
  doc: string,
  runtime: Runtime,
  hooks?: EditorHooks,
): EditorView {
  // Resolved now, at mount: the stylesheets apply by then, so the probe reads real colours.
  registerStrudelandSettings();
  initTheme(SETTINGS.theme);
  return new EditorView({ state: createEditorState(doc, runtime, hooks), parent });
}

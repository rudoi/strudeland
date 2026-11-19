import type { EditorView } from '@codemirror/view';
import { getCM, Vim, type CodeMirrorV } from '@replit/codemirror-vim';
import { writable, type Readable } from 'svelte/store';
import type { Tone } from '../theme/tones';

export type VimModeName = 'normal' | 'insert' | 'visual' | 'replace';
export interface VimModeState {
  mode: VimModeName;
  /** Visual only: unset for charwise. */
  subMode?: 'linewise' | 'blockwise';
}

export interface VimMode {
  /** `null` means vim is off (or no view). */
  store: Readable<VimModeState | null>;
  /**
   * Follow `view`'s vim mode. Detaches from the previous view first. `undefined` or a view without
   * vim sets the store to null. Call again after every state swap or keymap change: each builds a
   * new vim adapter.
   */
  attach(view: EditorView | undefined): void;
  destroy(): void;
}

type ModeChange = { mode?: string; subMode?: string };

/** The view's vim adapter, or null when the view has no vim. */
export function vimAdapter(view: EditorView): CodeMirrorV | null {
  return getCM(view) as unknown as CodeMirrorV | null;
}

/** Leave insert mode (keeping the cursor) if the view's vim is in it; otherwise nothing. */
export function exitInsert(view: EditorView): void {
  const cm = vimAdapter(view);
  if (cm?.state.vim?.insertMode) Vim.exitInsertMode(cm, true);
}

function fromEvent(e: ModeChange): VimModeState {
  switch (e.mode) {
    case 'insert':
      return { mode: 'insert' };
    case 'replace':
      return { mode: 'replace' };
    case 'visual':
      return e.subMode === 'linewise' || e.subMode === 'blockwise'
        ? { mode: 'visual', subMode: e.subMode }
        : { mode: 'visual' };
    default:
      return { mode: 'normal' };
  }
}

/** The mode of a freshly attached adapter (no event fires when the plugin starts). */
function seed(cm: CodeMirrorV): VimModeState {
  const vim = cm.state.vim;
  if (vim?.insertMode) return fromEvent({ mode: 'insert' });
  if (!vim?.visualMode) return fromEvent({ mode: 'normal' });
  const subMode = vim.visualLine ? 'linewise' : vim.visualBlock ? 'blockwise' : undefined;
  return fromEvent({ mode: 'visual', subMode });
}

export function createVimMode(): VimMode {
  const state = writable<VimModeState | null>(null);
  let detach: (() => void) | null = null;
  const release = () => {
    const d = detach;
    detach = null;
    d?.();
  };
  return {
    store: { subscribe: state.subscribe },
    attach(view) {
      release();
      const cm = view ? vimAdapter(view) : null;
      if (!cm) return state.set(null);
      const onChange = (e: ModeChange) => state.set(fromEvent(e));
      cm.on('vim-mode-change', onChange);
      detach = () => cm.off('vim-mode-change', onChange);
      state.set(seed(cm));
    },
    destroy() {
      release();
      state.set(null);
    },
  };
}

const SUB_LABEL: Record<NonNullable<VimModeState['subMode']>, string> = {
  linewise: 'v-line',
  blockwise: 'v-block',
};
const TONES: Record<VimModeName, Tone> = {
  normal: 'neutral',
  insert: 'accent',
  visual: 'warning',
  replace: 'danger',
};

/** The status bar chip for a vim mode; `null` is vim off. */
export function vimChip(mode: VimModeState | null): { label: string; tone: Tone; lit: boolean } {
  if (!mode) return { label: 'vim', tone: 'neutral', lit: false };
  const label = mode.subMode ? SUB_LABEL[mode.subMode] : mode.mode;
  return { label: `vim · ${label}`, tone: TONES[mode.mode], lit: true };
}

let writeHandler: (() => void) | null = null;
let writeDefined = false;

/**
 * Bind vim's `:w` (`write`) to `save`. `write` is defined page-globally once, on the first call,
 * and dispatches to the current handler; the last call wins. Returns a function that clears the
 * handler if it is still `save`'s, leaving `:w` a no-op, which is what vim's own `:w` does here (it
 * calls `CodeMirror.commands.save`, which nothing defines).
 */
export function defineVimWrite(save: () => void): () => void {
  if (!writeDefined) {
    Vim.defineEx('write', 'w', () => writeHandler?.());
    writeDefined = true;
  }
  const handler = () => save();
  writeHandler = handler;
  return () => {
    if (writeHandler === handler) writeHandler = null;
  };
}

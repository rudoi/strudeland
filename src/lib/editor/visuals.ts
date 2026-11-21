import { StateEffect, StateField, type EditorState } from '@codemirror/state';
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view';
import {
  highlightMiniLocations,
  updateMiniLocations,
  updateSliderWidgets,
  updateWidgets,
} from '@strudel/codemirror';
import { cleanupDraw, Drawer, getDrawContext } from '@strudel/draw';
import type { RuntimeHooks } from '../strudel/runtime';
import type { Hap } from '../strudel/types';
import { exitInsert } from './vim-mode';

interface Widget {
  type: string;
}
type DrawTime = [number, number];
/** A `.pianoroll()`/`.punchcard()`/… painter registered through `Pattern.onPaint`. */
type Painter = (ctx: unknown, time: number, haps: Hap[], drawTime: DrawTime) => void;
type OnDraw = (haps: Hap[], time: number, drawer: unknown, painters: Painter[]) => void;

export interface DrawerLike {
  setDrawTime(t: DrawTime): void;
  invalidate(scheduler: unknown): void;
  start(scheduler: unknown): void;
  stop(): void;
}

export interface VisualsOptions {
  getView: () => EditorView | undefined;
  getScheduler: () => unknown;
  /** Test seams; default to @strudel/draw's Drawer, shared canvas and cleanup. */
  createDrawer?: (onDraw: OnDraw, drawTime: DrawTime) => DrawerLike;
  getDrawContext?: () => unknown;
  cleanupDraw?: (clearScreen: boolean) => void;
}

/** Painters need history and lookahead; without any, only the present moment is drawn. */
export const PAINTER_DRAW_TIME: DrawTime = [-2, 2];

const auditionLocation = StateEffect.define<{ from: number; to: number } | null>();
const soundingNote = Decoration.mark({
  class: 'cm-audition-note',
  attributes: { style: 'outline: solid 2px var(--foreground)' },
});
export const auditionHighlight = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    if (transaction.docChanged) return Decoration.none;
    for (const effect of transaction.effects) {
      if (!effect.is(auditionLocation)) continue;
      const range = effect.value;
      return range ? Decoration.set(soundingNote.range(range.from, range.to)) : Decoration.none;
    }
    return value;
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** Outline an unsaved note without replacing the evaluated song's mini locations. */
export function highlightAudition(view: EditorView, range: { from: number; to: number }) {
  view.dispatch({ effects: auditionLocation.of(range) });
  return () => {
    if (view.state.field(auditionHighlight, false)?.size) {
      view.dispatch({ effects: auditionLocation.of(null) });
    }
  };
}

/** Drop mini-notation highlights, e.g. after swapping a song's state into the view. */
export function clearHighlights(view: EditorView): void {
  updateMiniLocations(view, []);
}

/**
 * Swap `state` into `view` and drop the highlights it carried from when it was last shown. The
 * drawer keeps running and finds no locations until the next evaluate. Vim leaves insert mode
 * first: `setState` destroys its adapter without removing the insert-mode key listener it put on
 * the content element, which the view keeps.
 */
export function attachState(view: EditorView, state: EditorState): void {
  exitInsert(view);
  view.setState(state);
  clearHighlights(view);
}

/** Runtime hooks that drive highlighting, painters, sliders and widgets in the editor. */
export function createVisuals(opts: VisualsOptions): RuntimeHooks {
  const createDrawer = opts.createDrawer ?? ((cb, t) => new Drawer(cb, t) as DrawerLike);
  let drawTime: DrawTime = [0, 0];
  let drawContext: unknown;
  const cleanup = opts.cleanupDraw ?? cleanupDraw;

  const drawer = createDrawer((haps, time, _drawer, painters) => {
    const view = opts.getView();
    if (view) {
      highlightMiniLocations(
        view,
        time,
        haps.filter((h) => h.isActive(time)),
      );
    }
    if (painters.length) {
      // The canvas is created on first use, so patterns without visuals never add one.
      drawContext ??= (opts.getDrawContext ?? getDrawContext)();
      painters.forEach((paint) => paint(drawContext, time, haps, drawTime));
    }
  }, drawTime);

  return {
    beforeEval: () => {
      cleanup(true);
    },
    afterEval: ({ pattern, meta }) => {
      const view = opts.getView();
      if (view) {
        const widgets: Widget[] = meta?.widgets ?? [];
        updateSliderWidgets(
          view,
          widgets.filter((w) => w.type === 'slider'),
        );
        updateWidgets(
          view,
          widgets.filter((w) => w.type !== 'slider'),
        );
        updateMiniLocations(view, meta?.miniLocations ?? []);
      }
      drawTime = pattern.getPainters().length ? PAINTER_DRAW_TIME : [0, 0];
      drawer.setDrawTime(drawTime);
      drawer.invalidate(opts.getScheduler());
    },
    onToggle: (started) => {
      if (started) {
        drawer.start(opts.getScheduler());
      } else {
        drawer.stop();
        const view = opts.getView();
        if (view) clearHighlights(view);
        cleanup(true);
      }
    },
  };
}

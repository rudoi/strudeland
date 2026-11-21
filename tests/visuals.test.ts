import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { EditorView } = await import('@codemirror/view');
const {
  auditionHighlight,
  attachState,
  clearHighlights,
  createVisuals,
  highlightAudition,
  PAINTER_DRAW_TIME,
} = await import('../src/lib/editor/visuals');
const { EditorState } = await import('@codemirror/state');
const { highlightExtension, highlightMiniLocations, updateMiniLocations } =
  await import('@strudel/codemirror');

function setup() {
  const log: string[] = [];
  let onDraw: any;
  const drawer = {
    drawTime: [0, 0] as [number, number],
    setDrawTime(t: [number, number]) {
      this.drawTime = t;
      log.push(`drawTime:${t}`);
    },
    invalidate: () => void log.push('invalidate'),
    start: () => void log.push('start'),
    stop: () => void log.push('stop'),
  };
  const view = new EditorView({ doc: 'note("c e")', parent: document.body });
  const ctx = { canvas: true };
  const hooks = createVisuals({
    getView: () => view,
    getScheduler: () => ({}),
    createDrawer: (cb) => ((onDraw = cb), drawer),
    getDrawContext: () => ctx,
    cleanupDraw: () => void log.push('cleanup'),
  });
  return { log, drawer, view, ctx, hooks, draw: (...a: any[]) => onDraw(...a) };
}
const pattern = (painters: number) => ({ getPainters: () => new Array(painters).fill(0) });
const hap = (active: boolean) => ({ value: {}, isActive: () => active });

test('afterEval without painters keeps the present-only draw window', () => {
  const { hooks, drawer, log, view } = setup();
  hooks.afterEval!({ code: '', pattern: pattern(0), meta: { miniLocations: [], widgets: [] } });
  expect(drawer.drawTime).toEqual([0, 0]);
  expect(log).toContain('invalidate');
  view.destroy();
});

test('afterEval with painters widens the draw window, and painters receive frames', () => {
  const { hooks, drawer, ctx, draw, view } = setup();
  hooks.afterEval!({ code: '', pattern: pattern(1), meta: {} });
  expect(drawer.drawTime).toEqual(PAINTER_DRAW_TIME);
  const calls: any[] = [];
  const haps = [hap(true), hap(false)];
  draw(haps, 1.5, null, [(...a: any[]) => calls.push(a)]);
  expect(calls).toEqual([[ctx, 1.5, haps, PAINTER_DRAW_TIME]]);
  view.destroy();
});

test('frames without painters never create a draw context', () => {
  let created = 0;
  const view = new EditorView({ doc: '', parent: document.body });
  let onDraw: any;
  createVisuals({
    getView: () => view,
    getScheduler: () => ({}),
    createDrawer: (cb) => (
      (onDraw = cb),
      { setDrawTime() {}, invalidate() {}, start() {}, stop() {} }
    ),
    getDrawContext: () => (created++, {}),
  });
  onDraw([hap(true)], 0, null, []);
  expect(created).toBe(0);
  view.destroy();
});

test('onToggle starts and stops the drawer', () => {
  const { hooks, log, view } = setup();
  hooks.onToggle!(true);
  hooks.onToggle!(false);
  expect(log.filter((l) => ['start', 'stop', 'cleanup'].includes(l))).toEqual([
    'start',
    'stop',
    'cleanup',
  ]);
  view.destroy();
});

/** Ranges currently drawn by the mini-notation highlight decorations. */
function highlighted(view: InstanceType<typeof EditorView>): [number, number][] {
  const out: [number, number][] = [];
  for (const source of view.state.facet(EditorView.decorations)) {
    const set = typeof source === 'function' ? source(view) : source;
    for (let it = set.iter(); it.value; it.next()) out.push([it.from, it.to]);
  }
  return out;
}
const located = (start: number, end: number) => ({
  value: {},
  whole: { begin: 0 },
  context: { locations: [{ start, end }] },
});

test('unsaved audition outlines the exact token and leaves playback locations available', () => {
  const doc = 'note("c3 g3")';
  const view = new EditorView({
    doc,
    extensions: [highlightExtension, auditionHighlight],
    parent: document.body,
  });
  updateMiniLocations(view, [[6, 8]]);

  const clear = highlightAudition(view, { from: 9, to: 11 });
  const outlined = view.dom.querySelector<HTMLElement>('.cm-audition-note');
  expect(outlined?.textContent).toBe('g3');
  const auditionStyle = outlined?.getAttribute('style');
  expect(auditionStyle).toContain('var(--foreground)');
  clear();
  expect(view.dom.querySelector('.cm-audition-note')).toBeNull();

  highlightMiniLocations(view, 0, [located(6, 8)]);
  expect(highlighted(view)).toEqual([[6, 8]]);
  const playbackMark = view.contentDOM.querySelector('span[style*="outline"]');
  expect(playbackMark?.textContent).toBe('c3');
  expect(playbackMark?.getAttribute('style')).toBe(auditionStyle);
  highlightAudition(view, { from: 9, to: 11 });
  view.dispatch({ changes: { from: 9, to: 11, insert: 'b3' } });
  expect(view.dom.querySelector('.cm-audition-note')).toBeNull();
  expect(highlighted(view)).toEqual([[6, 8]]);
  view.destroy();
});

test('a restored song does not show an audition that ended while it was stashed', () => {
  const state = (doc: string) =>
    EditorState.create({ doc, extensions: [highlightExtension, auditionHighlight] });
  const view = new EditorView({ state: state('note("g3")'), parent: document.body });
  const endAudition = highlightAudition(view, { from: 6, to: 8 });
  const stashed = view.state;

  attachState(view, state('note("c3")'));
  endAudition();
  attachState(view, stashed);
  expect(view.dom.querySelector('.cm-audition-note')).toBeNull();
  expect(highlighted(view)).toEqual([]);
  view.destroy();
});

test('delimiters and unrelated edits retain the outline, while extending the note clears it', () => {
  const view = new EditorView({
    doc: 'note("g3")',
    extensions: auditionHighlight,
    parent: document.body,
  });
  const mark = () => view.contentDOM.querySelector('.cm-audition-note')?.textContent;
  highlightAudition(view, { from: 6, to: 8 });
  view.dispatch({ changes: { from: 8, insert: ' ' } });
  expect(mark()).toBe('g3');
  view.dispatch({ changes: { from: 0, insert: '// draft\n' } });
  expect(mark()).toBe('g3');
  view.dispatch({ changes: { from: 17, insert: '0' } });
  expect(view.state.doc.toString()).toContain('g30');
  expect(mark()).toBeUndefined();

  view.dispatch({ changes: { from: 17, to: 18, insert: '' } });
  highlightAudition(view, { from: 15, to: 17 });
  view.dispatch({ changes: { from: 17, insert: '.5' } });
  expect(view.state.doc.toString()).toContain('g3.5');
  expect(mark()).toBeUndefined();
  view.destroy();
});

test('clearHighlights drops the mini-notation locations', () => {
  const view = new EditorView({
    doc: 'note("c e")',
    extensions: highlightExtension,
    parent: document.body,
  });
  updateMiniLocations(view, [
    [6, 7],
    [8, 9],
  ]);
  highlightMiniLocations(view, 0, [located(6, 7), located(8, 9)]);
  expect(highlighted(view)).toEqual([
    [6, 7],
    [8, 9],
  ]);

  clearHighlights(view);
  expect(highlighted(view)).toEqual([]);
  // Later frames find nothing to highlight in the cleared state.
  highlightMiniLocations(view, 0.5, [located(6, 7)]);
  expect(highlighted(view)).toEqual([]);
  view.destroy();
});

test('clearHighlights on a view without highlighting is a no-op', () => {
  const view = new EditorView({ doc: 'x', parent: document.body });
  expect(() => clearHighlights(view)).not.toThrow();
  expect(view.state.doc.toString()).toBe('x');
  view.destroy();
});

test('onToggle(false) clears the highlights', () => {
  const view = new EditorView({
    doc: 'note("c")',
    extensions: highlightExtension,
    parent: document.body,
  });
  const hooks = createVisuals({
    getView: () => view,
    getScheduler: () => ({}),
    createDrawer: () => ({ setDrawTime() {}, invalidate() {}, start() {}, stop() {} }),
    cleanupDraw: () => {},
  });
  updateMiniLocations(view, [[6, 7]]);
  highlightMiniLocations(view, 0, [located(6, 7)]);
  expect(highlighted(view)).toEqual([[6, 7]]);
  hooks.onToggle!(false);
  highlightMiniLocations(view, 0, [located(6, 7)]);
  expect(highlighted(view)).toEqual([]);
  view.destroy();
});

test('attachState swaps the state in and drops the highlights it carried', () => {
  const view = new EditorView({
    doc: 'note("c e")',
    extensions: highlightExtension,
    parent: document.body,
  });
  updateMiniLocations(view, [[6, 7]]);
  highlightMiniLocations(view, 0, [located(6, 7)]);
  const stashed = view.state;
  view.setState(EditorState.create({ doc: 'other', extensions: highlightExtension }));
  attachState(view, stashed);
  expect(view.state.doc.toString()).toBe('note("c e")');
  highlightMiniLocations(view, 0.5, [located(6, 7)]);
  expect(highlighted(view)).toEqual([]);
  view.destroy();
});

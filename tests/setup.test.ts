import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createEditor, evaluateCommand, runtimeEvaluate, stopCommand } =
  await import('../src/lib/editor/setup');
const { keymap } = await import('@codemirror/view');
import type { Runtime } from '../src/lib/strudel/runtime';

function fakeRuntime() {
  const calls: string[] = [];
  return {
    calls,
    runtime: {
      scheduler: {},
      drawingScheduler: {},
      store: { subscribe: () => () => {} },
      evaluate: async (c: string) => (calls.push('eval:' + c), true),
      stop: () => void calls.push('stop'),
      resumeAudio: async () => true,
      canAudition: () => true,
      prepareAudition: async () => true,
      start: async () => true,
      cycleTime: () => null,
      transport: {
        store: { subscribe: () => () => {} },
        startFrom: () => {},
        loopSection: () => {},
        loopRange: () => {},
        playEnding: () => {},
        reset: () => {},
      },
      position: () => ({ cycle: 0, located: null }),
      query: () => null,
      readScope: () => false,
      drainLevels: () => null,
      levels: () => null,
      setLimiter: () => {},
    } satisfies Runtime,
  };
}

test('commands call the runtime', () => {
  const { calls, runtime } = fakeRuntime();
  const view = createEditor(document.body, 'note("c")', runtime);
  expect(evaluateCommand(runtimeEvaluate(runtime))(view)).toBe(true);
  expect(stopCommand(runtime)()).toBe(true);
  expect(calls).toEqual(['eval:note("c")', 'stop']);
  view.destroy();
});

test('evaluateCommand runs the given action, then reports handled', () => {
  const { runtime } = fakeRuntime();
  const view = createEditor(document.body, 'x', runtime);
  const ran: string[] = [];
  expect(evaluateCommand((v) => void ran.push(v.state.doc.toString()))(view)).toBe(true);
  expect(ran).toEqual(['x']);
  view.destroy();
});

/** Count the dispatches `run` makes: `flash` dispatches the highlight at once. */
function dispatches(view: { dispatch: (...a: never[]) => void }, run: () => void): number {
  const original = view.dispatch;
  let count = 0;
  view.dispatch = (...a: never[]) => {
    count++;
    original.apply(view, a);
  };
  try {
    run();
  } finally {
    view.dispatch = original;
  }
  return count;
}

test('evaluateCommand flashes unless the action returns exactly false', () => {
  const { runtime } = fakeRuntime();
  const view = createEditor(document.body, 'x', runtime);
  expect(dispatches(view, () => evaluateCommand(() => false)(view))).toBe(0);
  expect(dispatches(view, () => evaluateCommand(() => true)(view))).toBe(1);
  expect(dispatches(view, () => evaluateCommand(() => {})(view))).toBe(1);
  expect(evaluateCommand(() => false)(view)).toBe(true);
  view.destroy();
});

test('Mod-. and Ctrl-. call onStop when given, in place of the runtime', () => {
  const { calls, runtime } = fakeRuntime();
  const stops: string[] = [];
  const view = createEditor(document.body, 'x', runtime, { onStop: () => void stops.push('s') });
  const keys = view.state.facet(keymap).flat();
  for (const key of ['Mod-.', 'Ctrl-.']) {
    expect(keys.find((b) => b.key === key)!.run!(view)).toBe(true);
  }
  expect(stops).toEqual(['s', 's']);
  expect(calls).toEqual([]);
  expect(stopCommand(runtime, () => void stops.push('t'))()).toBe(true);
  expect(stops).toEqual(['s', 's', 't']);
  expect(calls).toEqual([]);
  view.destroy();
});

test('createEditor registers the strudeland settings before activating the theme', async () => {
  const { settings } = await import('@strudel/codemirror');
  delete settings.strudeland;
  const view = createEditor(document.body, '', fakeRuntime().runtime);
  expect(Object.keys(settings.strudeland)).toContain('foreground');
  for (const value of Object.values<string>(settings.strudeland))
    expect(value).not.toContain('var(');
  view.destroy();
});

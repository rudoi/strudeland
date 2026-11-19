import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createEditor, evaluateCommand, runtimeEvaluate, stopCommand } =
  await import('../src/lib/editor/setup');
import type { Runtime } from '../src/lib/strudel/runtime';

function fakeRuntime() {
  const calls: string[] = [];
  return {
    calls,
    runtime: {
      scheduler: {},
      store: { subscribe: () => () => {} },
      evaluate: async (c: string) => (calls.push('eval:' + c), true),
      stop: () => void calls.push('stop'),
      resumeAudio: async () => true,
      start: async () => true,
      cycleTime: () => null,
      transport: {
        store: { subscribe: () => () => {} },
        startFrom: () => {},
        loopSection: () => {},
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

test('createEditor registers the strudeland settings before activating the theme', async () => {
  const { settings } = await import('@strudel/codemirror');
  delete settings.strudeland;
  const view = createEditor(document.body, '', fakeRuntime().runtime);
  expect(Object.keys(settings.strudeland)).toContain('foreground');
  for (const value of Object.values<string>(settings.strudeland))
    expect(value).not.toContain('var(');
  view.destroy();
});

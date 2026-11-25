// The committed headless host bundle's entry (`src/host/main.ts`), evaluated in a bare `node:vm`
// context: no DOM, no timers that fire, a zero clock and a throwing `Math.random`, as the embedded
// runtime gives it. Each test gets a fresh context, as each host request does. Results come back
// through the slots `__strudelandResult` and `__strudelandJob`. Strings in song code are single
// quoted where they must stay strings (the transpiler turns "..." into mini-notation).
import { afterAll, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import vm from 'node:vm';
import { HOST_JS, REPO_ROOT } from '../scripts/host/build';
import { chunkRequest, renderChunkPlans } from '../src/lib/render/chunks';
import type { RenderRequest } from '../src/lib/render/contract';
import { loggedError, loggerMessage } from '../src/lib/render/logged-errors';

const HOST_SOURCE = readFileSync(join(REPO_ROOT, HOST_JS), 'utf8');
const script = new vm.Script(HOST_SOURCE, { filename: 'host.js' });

const REQUEST = {
  layout: 'linear',
  fromBar: 0,
  toBar: 2,
  overlapBars: 0,
  lookbackSec: 'auto',
  mode: 'mix',
  sampleRate: 48000,
};
const SONG = 'note("c e").s("sine")';

interface Host {
  console: { level: string; text: string }[];
  call(fn: 'planJob' | 'planRender' | 'query' | 'inspect', ...args: unknown[]): Promise<any>;
  get(name: string): any;
}

/** A fresh context with the bundle loaded. */
function host(): Host {
  const logged: { level: string; text: string }[] = [];
  const sink =
    (level: string) =>
    (...a: unknown[]) =>
      void logged.push({ level, text: a.map(String).join(' ') });
  const ctx = vm.createContext({
    console: {
      log: sink('log'),
      info: sink('info'),
      debug: sink('debug'),
      warn: sink('warn'),
      error: sink('error'),
    },
    performance: { now: () => 0 },
    setTimeout: () => 0,
    clearTimeout: () => {},
  });
  vm.runInContext('Math.random = () => { throw new Error("Math.random is not available"); };', ctx);
  script.runInContext(ctx);
  ctx.__args = undefined;
  return {
    console: logged,
    get: (name) => (ctx as any)[name],
    async call(fn, ...args) {
      (ctx as any).__args = args;
      const ret = vm.runInContext(`strudelandHost.${fn}(...__args)`, ctx);
      expect(ret).toBeUndefined();
      // Drain microtasks (the embedder does the same); nothing waits on a timer.
      for (let i = 0; i < 100 && (ctx as any).__strudelandResult === undefined; i++) {
        await new Promise((r) => setImmediate(r));
      }
      return JSON.parse(vm.runInContext('JSON.stringify(__strudelandResult ?? null)', ctx));
    },
  };
}

test('the bundle loads with no DOM and installs the entry only', () => {
  const h = host();
  expect(vm.runInContext('typeof window + typeof document', vm.createContext({}))).toBe(
    'undefinedundefined',
  );
  expect(typeof h.get('strudelandHost').planJob).toBe('function');
  expect(typeof h.get('strudelandHost').planRender).toBe('function');
  expect(typeof h.get('strudelandHost').query).toBe('function');
  expect(typeof h.get('strudelandHost').inspect).toBe('function');
  expect(h.get('__strudelandResult')).toBeUndefined();
});

test('planJob: the job text in __strudelandJob and its info in the slot', async () => {
  const h = host();
  const r = await h.call('planJob', SONG, JSON.stringify(REQUEST));
  expect(r.ok).toBe(true);
  const info = JSON.parse(r.value);
  const json: string = h.get('__strudelandJob');
  expect(info.jsonLength).toBe(json.length);
  expect(info.cps).toBe(0.5);
  expect(info.events.map((e: any) => e.note)).toEqual(['c', 'e', 'c', 'e']);
  expect(info.errors).toEqual([]);
  const job = JSON.parse(json);
  expect(job.schema).toBe(2);
  expect(job.sampleRate).toBe(48000);
  expect(/^[\x20-\x7e]*$/.test(json)).toBe(true);
});

test('planJob: shims, stubs and the widgets plan without a DOM', async () => {
  const h = host();
  const code = [
    'await samples({ x: "x.wav" })',
    'aliasBank("a", "b")',
    'note("c e").s("sine")._scope().pianoroll().markcss("color:red").gain(slider(0.5, 0, 1))',
  ].join('\n');
  const r = await h.call('planJob', code, JSON.stringify(REQUEST));
  expect(r.ok).toBe(true);
  const info = JSON.parse(r.value);
  expect(info.events).toHaveLength(4);
  expect(info.errors).toEqual([]);
});

test('planJob: a console.warn while planning is captured into info.errors', async () => {
  const h = host();
  const r = await h.call('planJob', `console.warn('careful'); ${SONG}`, JSON.stringify(REQUEST));
  expect(r.ok).toBe(true);
  expect(JSON.parse(r.value).errors).toEqual([{ kind: 'other', message: 'careful' }]);
  expect(h.console).toContainEqual({ level: 'warn', text: 'careful' });
});

/** A song whose inner query throws: core's `queryArc` logs `[query] error: …` and answers []. */
const QUERY_ERROR_SONG = [
  "note('c').withValue(() => { throw new Error('inner query broke'); }).queryArc(0, 1);",
  SONG,
].join('\n');

test('planJob: a failure core logs is captured into info.errors as the page captures it', async () => {
  const r = await host().call('planJob', QUERY_ERROR_SONG, JSON.stringify(REQUEST));
  expect(r.ok).toBe(true);
  const info = JSON.parse(r.value);
  expect(info.events.map((e: any) => e.note)).toEqual(['c', 'e', 'c', 'e']);
  expect(info.errors).toEqual(await pageCaptures(() => QUERY_ERROR_PATTERN().queryArc(0, 1)));
  expect(info.errors).toEqual([{ kind: 'other', message: '[query] error: inner query broke' }]);
});

test('planJob: other console.log output and non-failure log lines are not captured', async () => {
  const h = host();
  const code = `console.log('an error, but not the logger'); logger('[x] fine'); ${SONG}`;
  const r = await h.call('planJob', code, JSON.stringify(REQUEST));
  expect(JSON.parse(r.value).errors).toEqual([]);
  expect(h.console).toContainEqual({ level: 'log', text: 'an error, but not the logger' });
});

test('planJob: a logger failure known only by its type is page-only (documented)', async () => {
  // `scale` logs this with type 'error'; console.log does not carry the type and the text does
  // not match the rule, so the host does not see it as a failure.
  const h = host();
  const r = await h.call('planJob', `s('bd').scale('C:major')`, JSON.stringify(REQUEST));
  expect(r.ok).toBe(true);
  expect(JSON.parse(r.value).errors).toEqual([]);
  expect(
    h.console.some((l) => l.text.startsWith("%c[tonal] Invalid value format for 'scale'")),
  ).toBe(true);
});

test('planJob: a bad request is kind plan', async () => {
  for (const bad of ['{', '', 'null', '[1]', '3']) {
    const r = await host().call('planJob', SONG, bad);
    expect({ bad, ok: r.ok, kind: r.kind }).toEqual({ bad, ok: false, kind: 'plan' });
    expect(r.error).toStartWith('bad request: ');
  }
});

test('planJob: meta, eval and plan failures carry their kinds', async () => {
  const req = JSON.stringify(REQUEST);
  const meta = await host().call('planJob', 'const meta = { bpm: \'x\' };\nnote("c")', req);
  expect([meta.ok, meta.kind]).toEqual([false, 'meta']);
  expect(meta.error).toStartWith('meta: ');
  const evalErr = await host().call('planJob', 'note("c").nope()', req);
  // The message is the engine's own (JavaScriptCore here, the embedded runtime in the crate).
  expect([evalErr.ok, evalErr.kind]).toEqual([false, 'eval']);
  expect(evalErr.error).toContain('nope is not a function');
  const empty = await host().call('planJob', '   ', req);
  expect(empty.kind).toBe('eval');
  const random = await host().call('planJob', 'Math.random(); note("c")', req);
  expect(random).toEqual({ ok: false, kind: 'eval', error: 'Math.random is not available' });
});

test('planJob: a request without the window fields fails in the plan step', async () => {
  const r = await host().call('planJob', SONG, '{}');
  expect(r.ok).toBe(false);
  expect(r.kind).toBe('plan');
});

const renderRequest = (length: object, o: object = {}) =>
  JSON.stringify({ length, mode: 'mix', sampleRate: 44100, ...o });

test('planRender: a project snapshot contributes labeled kit voices to the engine job', async () => {
  const h = host();
  const code = 'note("c2").apply(instruments.drums({ gain: 0.25 }))';
  const request = renderRequest(
    { kind: 'length', bars: 1 },
    {
      instruments: {
        drums: {
          schema: 1,
          parameters: { gain: 0.5 },
          voices: [
            {
              name: 'kick',
              chain: [
                { control: 's', value: 'sine' },
                { control: 'gain', value: { param: 'gain' } },
              ],
            },
            { name: 'hat', chain: [{ control: 's', value: 'white' }] },
          ],
        },
      },
    },
  );
  expect(await h.call('planRender', code, request)).toEqual({ ok: true, value: 'ok' });
  const planned = JSON.parse(h.get('__strudelandRender'));
  expect(JSON.parse(planned.chunks[0].json).values).toEqual([
    { note: 'c2', s: 'sine', gain: 0.25, label: 'drums/kick' },
    { note: 'c2', s: 'white', label: 'drums/hat' },
  ]);
});

test('planRender: the plan as ASCII JSON in __strudelandRender, ok in the slot', async () => {
  const h = host();
  const code = `const meta = { bpm: 120 };\nnote("c e").s("sine").room(0.3).gain("<1 0.5>")`;
  const r = await h.call('planRender', code, renderRequest({ kind: 'length', bars: 20 }));
  expect(r).toEqual({ ok: true, value: 'ok' });
  const text: string = h.get('__strudelandRender');
  expect(/^[\x20-\x7e]*$/.test(text)).toBe(true);
  const planned = JSON.parse(text);
  expect(planned.window).toEqual({ fromBar: 0, toBar: 20, layout: 'linear' });
  expect(planned.cps).toBe(0.5);
  expect(planned.structure.bpm).toBe(120);
  expect(planned.errors).toEqual([]);
  expect('events' in planned).toBe(false);
  expect(planned.chunks.length).toBeGreaterThan(1);
  for (const c of planned.chunks) {
    expect(JSON.parse(c.json).schema).toBe(2);
    expect(c.plan.c1).toBe(c.plan.toBar);
  }
  expect(h.get('__strudelandJob')).toBeUndefined();
});

test('planRender: a non-ASCII string in the song is escaped in the plan', async () => {
  const h = host();
  const code = `console.warn('café'); note("c").s("sine").color('café')`;
  const r = await h.call('planRender', code, renderRequest({ kind: 'length', bars: 2 }));
  expect(r.ok).toBe(true);
  const text: string = h.get('__strudelandRender');
  expect(/^[\x20-\x7e]*$/.test(text)).toBe(true);
  const planned = JSON.parse(text);
  expect(planned.errors).toEqual([{ kind: 'other', message: 'café' }]);
  // The job text inside is already ASCII; it decodes to exactly what pageJob wrote.
  expect(planned.chunks[0].json).toContain('caf\\u00e9');
  expect(JSON.parse(planned.chunks[0].json).values[0].color).toBe('café');
});

test('planRender: includeEvents adds the schedule events', async () => {
  const h = host();
  const r = await h.call(
    'planRender',
    SONG,
    renderRequest({ kind: 'length', bars: 2 }, { includeEvents: true }),
  );
  expect(r.ok).toBe(true);
  const planned = JSON.parse(h.get('__strudelandRender'));
  expect(planned.events.map((e: any) => e.note)).toEqual(['c', 'e', 'c', 'e']);
});

test('planRender: a length error arrives as kind meta with its flag', async () => {
  const h = host();
  const r = await h.call('planRender', SONG, renderRequest({ kind: 'clip' }));
  expect(r).toEqual({
    ok: false,
    kind: 'meta',
    error: 'this song has no sections; pass --length N to say how many bars to render',
    flag: '--length',
  });
  expect(h.get('__strudelandRender')).toBeUndefined();
  const section = await host().call(
    'planRender',
    SONG,
    renderRequest({ kind: 'section', name: 'a' }),
  );
  expect([section.kind, section.flag]).toEqual(['meta', '--section']);
  const loops = await host().call('planRender', SONG, renderRequest({ kind: 'clip', loops: 0 }));
  expect([loops.kind, loops.flag]).toEqual(['meta', '--loops']);
});

test('planRender: other failures carry no flag', async () => {
  const meta = await host().call(
    'planRender',
    'const meta = { bpm: \'x\' };\nnote("c")',
    renderRequest({ kind: 'length', bars: 2 }),
  );
  expect([meta.ok, meta.kind, 'flag' in meta]).toEqual([false, 'meta', false]);
  const evalErr = await host().call(
    'planRender',
    'note("c").nope()',
    renderRequest({ kind: 'length', bars: 2 }),
  );
  expect([evalErr.kind, 'flag' in evalErr]).toEqual(['eval', false]);
});

test('planRender: a bad request is kind plan', async () => {
  for (const bad of ['{', '', 'null', '[1]', '3']) {
    const r = await host().call('planRender', SONG, bad);
    expect({ bad, ok: r.ok, kind: r.kind }).toEqual({ bad, ok: false, kind: 'plan' });
    expect(r.error).toStartWith('bad request: ');
  }
  const cases: [string, string][] = [
    [renderRequest({ kind: 'length', bars: 2 }, { mode: 'stems' }), 'mode: stems'],
    [renderRequest({ kind: 'length', bars: 2 }, { sampleRate: 48000 }), 'sampleRate'],
    [renderRequest({ kind: 'length', bars: 0 }), 'length.bars'],
    [renderRequest({ kind: 'nope' }), 'unknown length kind nope'],
    ['{}', 'mode'],
  ];
  for (const [bad, needle] of cases) {
    const r = await host().call('planRender', SONG, bad);
    expect({ bad, kind: r.kind, flag: r.flag }).toEqual({ bad, kind: 'plan', flag: undefined });
    expect(r.error).toStartWith('bad request: ');
    expect(r.error).toContain(needle);
  }
});

/** Corpus songs with several chunks: every chunk planRender plans equals planJob's job for the
 *  same chunk request, both through the bundle (the host-side half of the batch parity). */
test.each([
  ['song-linear', 0, 12, 'clip'],
  ['reverb-long', 0, 20, 'linear'],
  ['late-feedback', 136, 160, 'linear'],
  ['pad-seam', 0, 20, 'linear'],
] as const)(
  "planRender: %s's chunk jobs equal planJob's",
  async (name, fromBar, toBar, layout) => {
    const code = readFileSync(join(REPO_ROOT, 'tests/render/corpus', `${name}.js`), 'utf8');
    const h = host();
    const r = await h.call(
      'planRender',
      code,
      renderRequest({ kind: 'window', fromBar, toBar, layout }, { includeEvents: true }),
    );
    expect(r).toEqual({ ok: true, value: 'ok' });
    const planned = JSON.parse(h.get('__strudelandRender'));
    const plans = renderChunkPlans({
      fromBar,
      toBar,
      events: planned.events,
      cps: planned.cps,
      overlapBars: 0,
    });
    expect(planned.chunks).toHaveLength(plans.length);
    expect(plans.length).toBeGreaterThan(1);
    const base: RenderRequest = {
      code,
      layout,
      fromBar,
      toBar,
      overlapBars: 0,
      lookbackSec: 0,
      mode: 'mix',
      sampleRate: 44100,
    };
    for (const [i, plan] of plans.entries()) {
      const { code: _code, ...request } = chunkRequest(base, plan);
      const one = host();
      const p = await one.call('planJob', code, JSON.stringify(request));
      expect(p.ok).toBe(true);
      expect({ name, i, json: planned.chunks[i].json }).toEqual({
        name,
        i,
        json: one.get('__strudelandJob'),
      });
      expect(planned.chunks[i].plan).toEqual({
        ...JSON.parse(p.value).plan,
        fromBar: plan.contentStartBar,
        toBar: plan.contentEndBar,
        lookbackSec: plan.lookbackSec,
      });
    }
  },
  60_000,
);

test('query: the events of [from, to) in the layout', async () => {
  const r = await host().call('query', SONG, 0, 1, 'linear');
  expect(r.ok).toBe(true);
  const events = JSON.parse(r.value);
  expect(events.map((e: any) => [e.tSec, e.note])).toEqual([
    [0, 'c'],
    [1, 'e'],
  ]);
  const clip = await host().call('query', SONG, 1, 2, 'clip');
  expect(JSON.parse(clip.value).map((e: any) => e.tSec)).toEqual([2, 3]);
});

test('query: the console is left as it is (its output is the events only)', async () => {
  const h = host();
  const r = await h.call('query', `globalThis.__warn = console.warn; ${SONG}`, 0, 1, 'linear');
  expect(r.ok).toBe(true);
  expect(h.get('__warn')).toBe(h.get('console').warn);
});

test('query: a bad window is kind plan, message bad window', async () => {
  const windows: [unknown, unknown][] = [
    [1, 1],
    [2, 1],
    [NaN, 1],
    [0, NaN],
    [0, Infinity],
    [-Infinity, 0],
    [0, 4096.5],
    [-1e308, 1e308],
    ['0', 1],
  ];
  for (const [from, to] of windows) {
    const r = await host().call('query', SONG, from, to, 'linear');
    expect({ from, to, r }).toEqual({
      from,
      to,
      r: { ok: false, kind: 'plan', error: 'bad window' },
    });
  }
  const widest = await host().call('query', SONG, -10, 4086, 'linear');
  expect(widest.ok).toBe(true);
});

test('query: an unknown layout, a meta error and an eval error', async () => {
  const layout = await host().call('query', SONG, 0, 1, 'loop');
  expect(layout).toEqual({ ok: false, kind: 'plan', error: 'bad request: unknown layout loop' });
  const meta = await host().call('query', "const meta = { meter: '9/x' };", 0, 1, 'linear');
  expect(meta.kind).toBe('meta');
  const evalErr = await host().call('query', 'nope()', 0, 1, 'clip');
  expect(evalErr.kind).toBe('eval');
});

// ---- the entry's helpers, imported directly (restore the shims' replacements after import)

const core = await import('@strudel/core');
const { importHostRealModules, snapshotOwnProperties } =
  await import('./helpers/prototype-snapshot');
await importHostRealModules();
// Load the widget and audio methods before host shims can replace them. Otherwise the
// snapshot has no real method to restore for a later test file.
await import('@strudel/codemirror');
await import('@strudel/webaudio');
const restorePrototype = snapshotOwnProperties(core.Pattern.prototype);
const hadHost = 'strudelandHost' in globalThis;
const entry = await import('../src/host/main').finally(restorePrototype);
afterAll(() => {
  if (!hadHost) delete (globalThis as any).strudelandHost;
});

const QUERY_ERROR_PATTERN = () =>
  core.note('c').withValue(() => {
    throw new Error('inner query broke');
  });

/** What the render page captures while `body` runs: its `strudel.log` listener's entries (the
 *  page's `loggedError` on each event), with a stand-in `document` for core's logger. */
async function pageCaptures(body: () => void): Promise<unknown[]> {
  const g = globalThis as any;
  const had = 'document' in g;
  const prev = g.document;
  const doc = new EventTarget();
  const out: unknown[] = [];
  doc.addEventListener('strudel.log', (e) => {
    const detail = (e as CustomEvent).detail ?? {};
    const err = loggedError(String(detail.message ?? ''), detail.type);
    if (err) out.push(err);
  });
  g.document = doc;
  const { log } = console;
  console.log = () => {};
  try {
    body();
  } finally {
    console.log = log;
    if (had) g.document = prev;
    else delete g.document;
  }
  return out;
}

/** A fake `createRuntime` result: a store and the three methods the adapter calls. */
function fakeRuntime(evaluate: () => Promise<boolean>, state: Record<string, unknown> = {}) {
  const s = { evalError: null, structure: null, cps: 0.5, seed: undefined, ...state };
  return {
    store: { subscribe: (run: (v: any) => void) => (run(s), () => {}) },
    evaluate,
    query: (from: number, to: number) => [from, to],
  } as any;
}

test('hostAdapter: a rejected evaluate is a failed one with its message', async () => {
  const rt = entry.hostAdapter(fakeRuntime(() => Promise.reject(new Error('prebake failed'))));
  expect(await rt.evaluate('x', {})).toBe(false);
  expect(rt.evalError()).toBe('prebake failed');
});

test('hostAdapter: evalError, state and query read the runtime', async () => {
  let ok = false;
  const rt = entry.hostAdapter(
    fakeRuntime(async () => ok, { evalError: 'boom', cps: 0.75, seed: 3, structure: { s: 1 } }),
  );
  expect(await rt.evaluate('x', {})).toBe(false);
  expect(rt.evalError()).toBe('boom');
  ok = true;
  expect(await rt.evaluate('x', {})).toBe(true);
  expect(rt.state()).toEqual({ structure: { s: 1 } as any, cps: 0.75, seed: 3 });
  expect(rt.query(1, 2, [])).toEqual([1, 2] as any);
  // A rejection's message lasts until the next evaluate.
  const rejecting = entry.hostAdapter(fakeRuntime(() => Promise.reject('plain')));
  expect(await rejecting.evaluate('x', {})).toBe(false);
  expect(rejecting.evalError()).toBe('plain');
});

test('current reads a store synchronously and unsubscribes', () => {
  let subscribed = 0;
  const store = {
    subscribe(run: (v: number) => void) {
      subscribed++;
      run(7);
      return () => void subscribed--;
    },
  };
  expect(entry.current(store)).toBe(7);
  expect(subscribed).toBe(0);
});

test('isQueryWindow', () => {
  expect(entry.isQueryWindow(0, 1)).toBe(true);
  expect(entry.isQueryWindow(-5, -4)).toBe(true);
  expect(entry.isQueryWindow(0, entry.MAX_QUERY_CYCLES)).toBe(true);
  expect(entry.isQueryWindow(0.5, entry.MAX_QUERY_CYCLES + 0.5)).toBe(true);
  for (const [a, b] of [
    [0, entry.MAX_QUERY_CYCLES + 1e-9],
    [1, 1],
    [1, 0],
    [NaN, 1],
    [0, Infinity],
    [-Infinity, Infinity],
    [-0, 0],
    ['0', 1],
    [0, null],
    [undefined, 1],
  ] as [unknown, unknown][]) {
    expect({ a, b, ok: entry.isQueryWindow(a, b) }).toEqual({ a, b, ok: false });
  }
});

test('captureConsole classifies warnings and errors, forwards them, and restores', () => {
  const seen: unknown[][] = [];
  const { error, warn } = console;
  console.error = (...a: unknown[]) => void seen.push(['error', ...a]);
  console.warn = (...a: unknown[]) => void seen.push(['warn', ...a]);
  try {
    const captured: any[] = [];
    const restore = entry.captureConsole(captured);
    console.warn('could not load AudioWorklet', 1);
    console.error(new Error('it broke'));
    console.error('hap 3 failed');
    restore();
    console.warn('after');
    expect(captured).toEqual([
      { kind: 'worklet', message: 'could not load AudioWorklet 1' },
      { kind: 'other', message: 'it broke' },
      { kind: 'hap', message: 'hap 3 failed' },
    ]);
    expect(seen.map((x) => x[0])).toEqual(['warn', 'error', 'error', 'warn']);
  } finally {
    console.error = error;
    console.warn = warn;
  }
});

test("captureConsole captures the failures core's logger reports through console.log", () => {
  const seen: unknown[][] = [];
  const { log } = console;
  console.log = (...a: unknown[]) => void seen.push(a);
  try {
    const captured: any[] = [];
    const restore = entry.captureConsole(captured);
    console.log('%c[query] error: x', 'color:red');
    console.log('%c[eval] code updated', 'color:red');
    console.log('plain error line');
    console.log('%c[core] Modulation type q not found', 'color:red', 'extra');
    restore();
    console.log('%c[query] error: after', 'color:red');
    expect(captured).toEqual([{ kind: 'other', message: '[query] error: x' }]);
    expect(seen).toHaveLength(5);
    expect(seen[0]).toEqual(['%c[query] error: x', 'color:red']);
  } finally {
    console.log = log;
  }
});

test("loggerMessage reads core's logger calls and nothing else", () => {
  const seen: unknown[][] = [];
  const { log } = console;
  console.log = (...a: unknown[]) => void seen.push(a);
  try {
    core.logger('loggerMessage probe ' + Math.random());
  } finally {
    console.log = log;
  }
  expect(seen).toHaveLength(1);
  expect(loggerMessage(seen[0]!)).toBe((seen[0]![0] as string).slice(2));
  expect(loggerMessage(seen[0]!)).toStartWith('loggerMessage probe ');
  expect(loggerMessage(['%cx'])).toBeNull();
  expect(loggerMessage(['x', 'y'])).toBeNull();
  expect(loggerMessage([1, 'y'])).toBeNull();
});

test('importing the entry installs globalThis.strudelandHost', () => {
  expect((globalThis as any).strudelandHost).toBe(entry.strudelandHost);
});

function inspection(host: Host, code: string, options: object): ReturnType<Host['call']> {
  return host.call('inspect', code, JSON.stringify({ view: 'song', from: 0, to: 1, ...options }));
}

function selection(code: string, expression: string) {
  const from = code.indexOf(expression);
  return { from, to: from + expression.length };
}

test('inspect: capture a reused transformed expression once with exact source contributors', async () => {
  const expression = 'motif.transpose(12).s("sine")';
  const code = `// 🎹
const motif = note("c4 e4 f4");
const transformed = ${expression};
s("bd")`;
  const r = await inspection(host(), code, {
    view: 'expression',
    expression: selection(code, expression),
  });
  expect(r.ok).toBe(true);
  const { notes, truncated } = JSON.parse(r.value);
  expect(truncated).toBe(false);
  expect(notes.map(({ pitch, start, end, sound }: any) => [pitch, start, end, sound])).toEqual([
    [72, '0', '1/3', 'sine'],
    [76, '1/3', '2/3', 'sine'],
    [77, '2/3', '1', 'sine'],
  ]);
  expect(notes[0].contributors).toContainEqual(selection(code, 'c4'));
  expect(notes[0].contributors).toContainEqual(selection(code, 'sine'));
});

test('inspect: label capture and arranged mode agree on explicit seeded pitches', async () => {
  const expression = 'note("c4 e4 f4").degradeBy(0.2)';
  const code = `const meta = { seed: 42 };
lead: ${expression}`;
  const selected = await inspection(host(), code, {
    view: 'expression',
    to: 4,
    expression: selection(code, expression),
  });
  const song = await inspection(host(), code, { to: 4 });
  expect(selected.ok).toBe(true);
  expect(song.ok).toBe(true);
  const events = (r: any) =>
    JSON.parse(r.value).notes.map(({ pitch, start, end }: any) => [pitch, start, end]);
  expect(events(selected)).toEqual(events(song));
  expect(events(song).length).toBeGreaterThan(0);
});

test('inspect: preserve continuations and duplicate voices without inventing pitch from sample indices', async () => {
  const code = 'stack(note("c4").slow(2), note("c4").slow(2), s("bd").n(3))';
  const r = await inspection(host(), code, { from: 1, to: 2 });
  expect(r.ok).toBe(true);
  const { notes } = JSON.parse(r.value);
  expect(notes).toHaveLength(2);
  expect(notes[0].id).not.toBe(notes[1].id);
  expect(
    notes.map(({ pitch, start, end, visibleStart, visibleEnd }: any) => [
      pitch,
      start,
      end,
      visibleStart,
      visibleEnd,
    ]),
  ).toEqual([
    [60, '0', '2', '1', '2'],
    [60, '0', '2', '1', '2'],
  ]);
});

test('inspect: distinguish unavailable or skipped expressions from an empty pitched window', async () => {
  const expression = 'note("c4")';
  const code = `null?.play(${expression}); s("bd")`;
  const r = await inspection(host(), code, {
    view: 'expression',
    expression: selection(code, expression),
  });
  expect(r).toMatchObject({ ok: false, kind: 'plan' });
  const silent = await inspection(host(), code, {});
  expect(silent.ok).toBe(true);
  expect(JSON.parse(silent.value)).toEqual({ notes: [], truncated: false });
});

test('inspect: respect octave controls and fall back to a note from a zero frequency', async () => {
  const code = 'stack(note("c4").octave(1), freq(440).octave(-1), note("e4").freq(0))';
  const r = await inspection(host(), code, {});
  expect(r.ok).toBe(true);
  const { notes } = JSON.parse(r.value);
  expect(notes.map(({ pitch }: { pitch: number }) => pitch)).toEqual([72, 57, 64]);
});

test('inspect: reject a wide window and report dense note truncation', async () => {
  const code = 'note("c4*2000")';
  expect(await inspection(host(), code, { to: 17 })).toMatchObject({ ok: false, kind: 'plan' });
  const r = await inspection(host(), code, {});
  expect(r.ok).toBe(true);
  const { notes, truncated } = JSON.parse(r.value);
  expect(truncated).toBe(true);
  expect(notes).toHaveLength(1024);
});

test('inspect: cooked mini offsets retain pitches but omit uncertain raw source links', async () => {
  const code = String.raw`note("c4\ne4")`;
  const r = await inspection(host(), code, {});
  expect(r.ok).toBe(true);
  const { notes } = JSON.parse(r.value);
  expect(notes.map(({ pitch }: any) => pitch)).toEqual([60, 64]);
  expect(notes.every(({ contributors }: any) => contributors === undefined)).toBe(true);
});

test('inspect: pinned instruments supply the current sounding voice', async () => {
  const expr = 'note("c4").apply(instruments.lead())';
  const r = await inspection(host(), expr, {
    view: 'expression',
    expression: selection(expr, expr),
    instruments: {
      lead: {
        schema: 1,
        parameters: {},
        voices: [{ name: 'melody', chain: [{ control: 's', value: 'pulse' }] }],
      },
    },
  });
  expect(r.ok).toBe(true);
  expect(JSON.parse(r.value).notes).toMatchObject([{ pitch: 60, sound: 'pulse' }]);
});

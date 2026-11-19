import { describe, expect, test } from 'bun:test';
import {
  createMasterTap,
  MAX_WORKLET_CHANNELS,
  type MasterController,
  type MasterNodeOptions,
} from '../src/lib/audio/master-tap';
import { MASTER_PROCESSOR } from '../src/lib/audio/master-dsp';
import { flush } from './helpers/fake-project-api';

function deferred() {
  let resolve!: () => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}

/** A fake graph whose nodes record `connect(dest)` / `disconnect(dest)` edges. */
function fakeGraph() {
  const edges = new Set<string>();
  const names = new Map<object, string>();
  const name = (n: unknown) => names.get(n as object) ?? '?';
  const node = (label: string, context: unknown) => {
    const n = {
      context,
      connect(dest: unknown) {
        edges.add(`${label}->${name(dest)}`);
        return dest;
      },
      disconnect(dest?: unknown) {
        if (dest === undefined) {
          for (const e of [...edges]) if (e.startsWith(`${label}->`)) edges.delete(e);
          return;
        }
        const e = `${label}->${name(dest)}`;
        if (!edges.has(e)) throw new Error('InvalidAccessError: not connected');
        edges.delete(e);
      },
    };
    names.set(n, label);
    return n;
  };
  let contexts = 0;
  const makeContext = (opts: { sampleRate?: number; channels?: number } = {}) => {
    const id = ++contexts;
    const loads: { url: string; d: ReturnType<typeof deferred> }[] = [];
    const context: any = {
      sampleRate: opts.sampleRate ?? 48000,
      audioWorklet: {
        addModule(url: string) {
          const d = deferred();
          loads.push({ url, d });
          return d.promise;
        },
      },
    };
    context.destination = Object.assign(node(`dest${id}`, context), {
      channelCount: opts.channels ?? 2,
    });
    let gains = 0;
    /** A destinationGain already connected to the destination, as superdough makes it. */
    const newGain = () => {
      const g = node(`gain${id}.${++gains}`, context);
      g.connect(context.destination);
      return g;
    };
    return { context, loads, newGain, id };
  };
  const created: {
    context: unknown;
    name: string;
    options: MasterNodeOptions;
    posted: unknown[];
    port: { onmessage: ((e: { data: unknown }) => void) | null };
  }[] = [];
  let nodes = 0;
  let failCreate = false;
  const createNode = (context: any, procName: string, options: MasterNodeOptions) => {
    if (failCreate) throw new Error('InvalidStateError: no processor');
    const n = node(`worklet${++nodes}`, context);
    const posted: unknown[] = [];
    const port = {
      onmessage: null as ((e: { data: unknown }) => void) | null,
      postMessage: (m: unknown) => void posted.push(m),
    };
    created.push({ context, name: procName, options, posted, port });
    return Object.assign(n, { port });
  };
  return {
    edges: () => [...edges].sort(),
    makeContext,
    createNode,
    created,
    failCreate: () => void (failCreate = true),
  };
}

function setup(opts: { running?: boolean } = {}) {
  const g = fakeGraph();
  const ctx = g.makeContext();
  let running = opts.running ?? true;
  let gain: any = ctx.newGain();
  let controllerCalls = 0;
  let throwController = false;
  const tap = createMasterTap({
    isRunning: () => running,
    getController: (): MasterController => {
      controllerCalls++;
      if (throwController) throw new Error('controller nope');
      return { output: { destinationGain: gain } };
    },
    moduleUrl: 'data:master',
    createNode: g.createNode as any,
  });
  return {
    g,
    ctx,
    tap,
    gain: () => gain,
    setGain: (x: any) => void (gain = x),
    setRunning: (on: boolean) => void (running = on),
    controllerCalls: () => controllerCalls,
    throwController: () => void (throwController = true),
    /** Resolves the oldest pending addModule of `c`. */
    load: async (c = ctx) => {
      c.loads.shift()!.d.resolve();
      await flush();
    },
  };
}

describe('master tap', () => {
  test('nothing happens before audio runs (no controller, no module)', () => {
    const s = setup({ running: false });
    s.tap.ensure();
    s.tap.setLimiter(true);
    expect(s.controllerCalls()).toBe(0);
    expect(s.ctx.loads).toHaveLength(0);
    expect(s.tap.available()).toBe(true);
    expect(s.tap.latencySec()).toBe(0);
    expect(s.g.edges()).toEqual(['gain1.1->dest1']);
  });

  test('the direct edge stays until addModule resolves, then gain -> worklet -> destination', async () => {
    const s = setup();
    s.tap.ensure();
    s.tap.ensure(); // one load per context
    expect(s.ctx.loads).toEqual([{ url: 'data:master', d: expect.anything() }]);
    await flush();
    expect(s.g.edges()).toEqual(['gain1.1->dest1']);
    expect(s.tap.available()).toBe(true); // loading counts as available
    expect(s.tap.latencySec()).toBe(0);
    await s.load();
    expect(s.g.edges()).toEqual(['gain1.1->worklet1', 'worklet1->dest1']);
    expect(s.g.created).toHaveLength(1);
    expect(s.g.created[0]!.name).toBe(MASTER_PROCESSOR);
    expect(s.g.created[0]!.options).toEqual({
      channelCount: 2,
      channelCountMode: 'explicit',
      channelInterpretation: 'discrete',
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    // nothing changed: a reference compare, no new node or edge
    s.tap.ensure();
    expect(s.g.created).toHaveLength(1);
    expect(s.g.edges()).toEqual(['gain1.1->worklet1', 'worklet1->dest1']);
  });

  test('a multichannel destination is not downmixed', async () => {
    const g = fakeGraph();
    const ctx = g.makeContext({ channels: 6 });
    const gain = ctx.newGain();
    const tap = createMasterTap({
      isRunning: () => true,
      getController: () => ({ output: { destinationGain: gain as any } }),
      moduleUrl: 'u',
      createNode: g.createNode as any,
    });
    tap.ensure();
    ctx.loads[0]!.d.resolve();
    await flush();
    expect(g.created[0]!.options.channelCount).toBe(6);
    expect(g.created[0]!.options.outputChannelCount).toEqual([6]);
  });

  test('more channels than a worklet output can carry keep the direct edge, logged once', () => {
    const g = fakeGraph();
    const warn = console.warn;
    const warned: unknown[] = [];
    console.warn = (...a: unknown[]) => void warned.push(a);
    try {
      let ctx = g.makeContext({ channels: MAX_WORKLET_CHANNELS + 1 });
      let gain = ctx.newGain();
      const tap = createMasterTap({
        isRunning: () => true,
        getController: () => ({ output: { destinationGain: gain as any } }),
        moduleUrl: 'u',
        createNode: g.createNode as any,
      });
      tap.ensure();
      expect(ctx.loads).toHaveLength(0); // no module, no node constructor to throw
      expect(g.created).toHaveLength(0);
      expect(tap.available()).toBe(false);
      expect(tap.latencySec()).toBe(0);
      expect(g.edges()).toEqual(['gain1.1->dest1']);
      // a second such context: still unavailable, not logged again
      ctx = g.makeContext({ channels: 64 });
      gain = ctx.newGain();
      tap.ensure();
      expect(tap.available()).toBe(false);
      expect(warned).toHaveLength(1);
    } finally {
      console.warn = warn;
    }
  });

  test('exactly the worklet limit is attached', async () => {
    const g = fakeGraph();
    const ctx = g.makeContext({ channels: MAX_WORKLET_CHANNELS });
    const gain = ctx.newGain();
    const tap = createMasterTap({
      isRunning: () => true,
      getController: () => ({ output: { destinationGain: gain as any } }),
      moduleUrl: 'u',
      createNode: g.createNode as any,
    });
    await Promise.all([tap.ready(), (async () => ctx.loads[0]!.d.resolve())()]);
    expect(g.created[0]!.options.outputChannelCount).toEqual([MAX_WORKLET_CHANNELS]);
    expect(tap.available()).toBe(true);
  });

  test('ready() resolves once the worklet is attached', async () => {
    const s = setup();
    let done = false;
    const ready = s.tap.ready().then(() => void (done = true));
    await flush();
    expect(done).toBe(false);
    expect(s.g.edges()).toEqual(['gain1.1->dest1']);
    s.ctx.loads.shift()!.d.resolve();
    await ready;
    expect(s.g.edges()).toEqual(['gain1.1->worklet1', 'worklet1->dest1']);
    expect(s.tap.latencySec()).toBe(72 / 48000);
    await s.tap.ready(); // attached: at once, no new load
    expect(s.ctx.loads).toHaveLength(0);
  });

  test('ready() resolves on a failed load, before audio runs, and after its timeout', async () => {
    const failing = setup();
    const ready = failing.tap.ready();
    const err = console.error;
    console.error = () => {};
    failing.ctx.loads.shift()!.d.reject(new Error('addModule failed'));
    await ready;
    console.error = err;
    expect(failing.tap.available()).toBe(false);
    expect(failing.g.edges()).toEqual(['gain1.1->dest1']);

    const idle = setup({ running: false });
    await idle.tap.ready();
    expect(idle.ctx.loads).toHaveLength(0);

    const hung = setup();
    await hung.tap.ready(5); // the module never loads: the direct edge stays
    expect(hung.g.edges()).toEqual(['gain1.1->dest1']);
    expect(hung.tap.available()).toBe(true);
  });

  test('a failed addModule keeps the direct edge and is unavailable', async () => {
    const s = setup();
    s.tap.ensure();
    const err = console.error;
    console.error = () => {};
    s.ctx.loads.shift()!.d.reject(new Error('addModule failed'));
    await flush();
    console.error = err;
    expect(s.tap.available()).toBe(false);
    expect(s.g.edges()).toEqual(['gain1.1->dest1']);
    s.tap.ensure();
    expect(s.ctx.loads).toHaveLength(0); // no retry on the same context
    expect(s.tap.latencySec()).toBe(0);
  });

  test('a failed node construction keeps the direct edge and is unavailable', async () => {
    const s = setup();
    s.g.failCreate();
    s.tap.ensure();
    const err = console.error;
    console.error = () => {};
    await s.load();
    console.error = err;
    expect(s.tap.available()).toBe(false);
    expect(s.g.edges()).toEqual(['gain1.1->dest1']);
  });

  test('after a reset() only the new gain feeds the worklet (not heard twice)', async () => {
    const s = setup();
    s.tap.ensure();
    await s.load();
    // superdough's reset(): the old gain is disconnected from everything, a new one is connected
    // straight to the destination
    s.gain().disconnect();
    s.setGain(null);
    s.tap.ensure(); // mid-reset: nothing to do
    s.setGain(s.ctx.newGain());
    expect(s.g.edges()).toEqual(['gain1.2->dest1', 'worklet1->dest1']);
    s.tap.ensure();
    expect(s.g.edges()).toEqual(['gain1.2->worklet1', 'worklet1->dest1']);
    expect(s.g.created).toHaveLength(1); // the worklet link persists per context
  });

  test('a replaced gain that is still connected is moved off the worklet', async () => {
    const s = setup();
    s.tap.ensure();
    await s.load();
    s.setGain(s.ctx.newGain());
    s.tap.ensure();
    expect(s.g.edges()).toEqual(['gain1.2->worklet1', 'worklet1->dest1']);
  });

  test('a replaced context gets its own module, node and the limiter preference', async () => {
    const s = setup();
    s.tap.setLimiter(true);
    s.tap.ensure();
    await s.load();
    expect(s.g.created[0]!.posted).toEqual([{ limiter: true }]);
    const other = s.g.makeContext({ sampleRate: 44100 });
    s.setGain(other.newGain());
    s.tap.ensure();
    expect(s.tap.latencySec()).toBe(0); // detached while the new module loads
    expect(other.loads).toHaveLength(1);
    await s.load(other);
    expect(s.g.created).toHaveLength(2);
    expect(s.g.created[1]!.context).toBe(other.context);
    expect(s.g.created[1]!.posted).toEqual([{ limiter: true }]);
    expect(s.g.edges()).toContain('gain2.1->worklet2');
    expect(s.g.edges()).toContain('worklet2->dest2');
    expect(s.g.edges()).not.toContain('gain2.1->dest2');
    expect(s.tap.latencySec()).toBe(66 / 44100);
  });

  test('a load that resolves after its context was replaced is dropped', async () => {
    const s = setup();
    s.tap.ensure();
    const other = s.g.makeContext();
    s.setGain(other.newGain());
    s.tap.ensure();
    await s.load(); // the first context's module
    expect(s.g.created).toHaveLength(0);
    await s.load(other);
    expect(s.g.created).toHaveLength(1);
    expect(s.g.created[0]!.context).toBe(other.context);
  });

  test('setLimiter before attach is posted on attach, after attach straight away', async () => {
    const s = setup();
    s.tap.setLimiter(true);
    s.tap.setLimiter(false);
    s.tap.setLimiter(true);
    s.tap.ensure();
    await s.load();
    const posted = s.g.created[0]!.posted;
    expect(posted).toEqual([{ limiter: true }]);
    s.tap.setLimiter(false);
    expect(posted).toEqual([{ limiter: true }, { limiter: false }]);
  });

  test('latencySec is 0 detached and the lookahead attached', async () => {
    const s = setup();
    expect(s.tap.latencySec()).toBe(0);
    s.tap.ensure();
    expect(s.tap.latencySec()).toBe(0);
    await s.load();
    expect(s.tap.latencySec()).toBe(72 / 48000);
  });

  test('levels from the port reach every listener; the remover and junk messages', async () => {
    const s = setup();
    const a: unknown[] = [];
    const b: unknown[] = [];
    const offA = s.tap.onLevels((f) => void a.push(f));
    s.tap.onLevels((f) => void b.push(f));
    s.tap.onLevels(() => {
      throw new Error('a bad listener');
    });
    s.tap.ensure();
    await s.load();
    const frame = { peak: [0.5, 0.25], rms: [0.1, 0.1], overs: [0, 0], limitDb: 0 };
    const port = s.g.created[0]!.port;
    const err = console.error;
    console.error = () => {};
    try {
      port.onmessage!({ data: frame });
      offA();
      port.onmessage!({ data: { ...frame, limitDb: -2 } });
      port.onmessage!({ data: 'junk' });
      port.onmessage!({ data: null });
    } finally {
      console.error = err;
    }
    expect(a).toEqual([frame]);
    expect(b).toEqual([frame, { ...frame, limitDb: -2 }]);
  });

  test('ensure never throws: a throwing controller, a failing connect restores the direct edge', async () => {
    const s = setup();
    s.tap.ensure();
    await s.load();
    const g2 = s.ctx.newGain();
    s.setGain(g2);
    const err = console.error;
    console.error = () => {};
    try {
      // the worklet refuses the new gain: it stays on the destination and is retried
      const realConnect = g2.connect;
      let first = true;
      (g2 as any).connect = (dest: unknown) => {
        if (first) {
          first = false;
          throw new Error('InvalidAccessError');
        }
        return realConnect(dest);
      };
      expect(() => s.tap.ensure()).not.toThrow();
      expect(s.g.edges()).toContain('gain1.2->dest1');
      expect(s.g.edges()).not.toContain('gain1.2->worklet1');
      s.tap.ensure();
      expect(s.g.edges()).toContain('gain1.2->worklet1');
      expect(s.g.edges()).not.toContain('gain1.2->dest1');
      s.throwController();
      expect(() => s.tap.ensure()).not.toThrow();
    } finally {
      console.error = err;
    }
  });

  test('a context without audioWorklet fails quietly', async () => {
    const tap = createMasterTap({
      isRunning: () => true,
      getController: () =>
        ({ output: { destinationGain: { context: {}, connect() {}, disconnect() {} } } }) as any,
    });
    const err = console.error;
    console.error = () => {};
    try {
      expect(() => tap.ensure()).not.toThrow();
      await flush();
    } finally {
      console.error = err;
    }
    expect(tap.available()).toBe(false);
  });
});

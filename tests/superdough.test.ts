import { describe, expect, test } from 'bun:test';
import type { LevelFrame } from '../src/lib/levels/levels';
import { createMasterTap, type MasterTapDeps } from '../src/lib/audio/master-tap';
import { createSuperdoughBackend } from '../src/lib/audio/superdough';
import { fakeAudioGraph } from './helpers/fake-audio-graph';

test('superdough backend resume false when suspended', async () => {
  const ctx = { state: 'suspended', currentTime: 1, resume: async () => {} };
  let created = 0;
  const be = createSuperdoughBackend({
    getContext: () => (created++, ctx),
    init: async () => {},
    output: () => {},
    setSeed: () => {},
  });
  expect(created).toBe(0);
  expect(be.isRunning()).toBe(false);
  expect(be.getTime()).toBe(0);
  expect(created).toBe(0);
  expect(await be.resume()).toBe(false);
  expect(be.isRunning()).toBe(false);
  ctx.state = 'running';
  expect(await be.resume()).toBe(true);
  expect(be.getTime()).toBe(1);
});

test('superdough backend init retries after a failure', async () => {
  let calls = 0;
  const be = createSuperdoughBackend({
    getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
    init: async () => {
      if (++calls === 1) throw new Error('addModule failed');
    },
    output: () => {},
    setSeed: () => {},
  });
  await expect(be.init()).rejects.toThrow('addModule failed');
  await be.init();
  expect(calls).toBe(2);
});

test('superdough backend forwards output(hap, t, cps) unchanged', async () => {
  const seen: unknown[][] = [];
  const be = createSuperdoughBackend({
    getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
    init: async () => {},
    output: (...args) => {
      seen.push(args);
      return Promise.resolve();
    },
    setSeed: () => {},
  });
  const hap = { value: { s: 'bd' } } as any;
  await be.output(hap, 1.25, 0.5);
  expect(seen).toEqual([[hap, 1.25, 0.5]]);
});

test('superdough backend output passes a rejection through', async () => {
  const be = createSuperdoughBackend({
    getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
    init: async () => {},
    output: async () => {
      throw new Error('sound bd not found');
    },
    setSeed: () => {},
  });
  await expect(Promise.resolve(be.output({} as any, 0, 1))).rejects.toThrow('not found');
});

test('superdough backend setSeed passes the seed through', () => {
  const seeds: number[] = [];
  const be = createSuperdoughBackend({
    getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
    init: async () => {},
    output: () => {},
    setSeed: (seed) => void seeds.push(seed),
  });
  be.setSeed(9);
  be.setSeed(-1.5);
  be.setSeed(0);
  expect(seeds).toEqual([9, -1.5, 0]);
});

describe('superdough backend graph()', () => {
  const setup = () => {
    const ctx = { state: 'running', currentTime: 0, resume: async () => {} };
    const nodeContext = { name: 'ctx' };
    let gain: any = { context: nodeContext, name: 'a' };
    let calls = 0;
    let fail = false;
    const be = createSuperdoughBackend({
      getContext: () => ctx,
      init: async () => {},
      output: () => {},
      setSeed: () => {},
      getController: () => {
        calls++;
        if (fail) throw new Error('controller nope');
        return { output: { destinationGain: gain } };
      },
    });
    return {
      be,
      ctx,
      nodeContext,
      calls: () => calls,
      setGain: (g: any) => void (gain = g),
      fail: () => void (fail = true),
    };
  };

  test('null before audio runs, without asking superdough (that would create a context)', () => {
    const s = setup();
    expect(s.be.graph!()).toBeNull();
    expect(s.calls()).toBe(0);
  });

  test('the destination gain and its context once running', async () => {
    const s = setup();
    await s.be.resume();
    const before = s.calls(); // resume() let the master tap look once
    const g = s.be.graph!()!;
    expect(g.destination as unknown).toEqual({ context: s.nodeContext, name: 'a' });
    expect(g.context as unknown).toBe(s.nodeContext);
    expect(s.calls()).toBe(before + 1);
  });

  test('null mid-reset (no destinationGain), then the replaced node', async () => {
    const s = setup();
    await s.be.resume();
    s.setGain(null);
    expect(s.be.graph!()).toBeNull();
    const other = { context: { name: 'ctx2' }, name: 'b' };
    s.setGain(other);
    expect(s.be.graph!()!.destination as unknown).toBe(other);
    expect(s.be.graph!()!.context as unknown).toBe(other.context);
  });

  test('null when the context is suspended again or superdough throws', async () => {
    const s = setup();
    await s.be.resume();
    s.ctx.state = 'suspended';
    expect(s.be.graph!()).toBeNull();
    s.ctx.state = 'running';
    s.fail();
    expect(s.be.graph!()).toBeNull();
  });

  test('null without getController', async () => {
    const be = createSuperdoughBackend({
      getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
      init: async () => {},
      output: () => {},
      setSeed: () => {},
    });
    await be.resume();
    expect(be.graph!()).toBeNull();
  });
});

describe('superdough backend master tap', () => {
  const setup = (state = 'running') => {
    const log: string[] = [];
    const ctx = { state, currentTime: 0, resume: async () => {} };
    let tapDeps: MasterTapDeps | null = null;
    const listeners: ((f: LevelFrame) => void)[] = [];
    const be = createSuperdoughBackend({
      getContext: () => ctx,
      init: async () => {},
      output: () => void log.push('output'),
      setSeed: () => {},
      getController: () => null,
      createMasterTap: (deps) => {
        tapDeps = deps;
        return {
          ensure: () => void log.push('ensure'),
          ready: async () => void log.push('ready'),
          onLevels: (cb) => {
            listeners.push(cb);
            return () => void log.push('off');
          },
          setLimiter: (on) => void log.push(`limiter ${on}`),
          available: () => false,
          latencySec: () => 0.0015,
        };
      },
    });
    return { be, log, ctx, tapDeps: () => tapDeps!, listeners };
  };

  test('output() repairs the wiring before the hap plays', async () => {
    const s = setup();
    await s.be.output({} as any, 0, 1);
    await s.be.output({} as any, 0, 1);
    expect(s.log).toEqual(['ensure', 'output', 'ensure', 'output']);
  });

  test('resume() and readScope() call ensure', async () => {
    const s = setup();
    await s.be.resume();
    expect(s.log).toEqual(['ensure']);
    s.be.readScope!(new Float32Array(4));
    expect(s.log).toEqual(['ensure', 'ensure']);
  });

  test('the tap gets the running check and the controller', async () => {
    const s = setup('suspended');
    expect(s.tapDeps().isRunning()).toBe(false);
    await s.be.resume();
    expect(s.tapDeps().isRunning()).toBe(false);
    s.ctx.state = 'running';
    expect(s.tapDeps().isRunning()).toBe(true);
    expect(s.tapDeps().getController()).toBeNull();
  });

  test('onLevels, setLimiter, limiterAvailable and masterLatencySec forward', () => {
    const s = setup();
    const got: LevelFrame[] = [];
    const off = s.be.onLevels!((f) => void got.push(f));
    const frame: LevelFrame = { peak: [1, 1], rms: [0.5, 0.5], overs: [3, 0], limitDb: -1 };
    s.listeners[0]!(frame);
    expect(got).toEqual([frame]);
    off();
    s.be.setLimiter!(true);
    s.be.setLimiter!(false);
    expect(s.log).toEqual(['off', 'limiter true', 'limiter false']);
    expect(s.be.limiterAvailable!()).toBe(false);
    expect(s.be.masterLatencySec!()).toBe(0.0015);
  });

  test('init() waits for the master tap before resolving, every time', async () => {
    const s = setup();
    await s.be.init();
    await s.be.init();
    expect(s.log).toEqual(['ready', 'ready']);
  });

  test('the first play after resume starts on the worklet, not the direct edge', async () => {
    // the real tap over a fake graph whose addModule resolves later
    const edges = new Set<string>();
    const label = new Map<object, string>();
    const node = (name: string, extra: object = {}) => {
      const n: any = {
        ...extra,
        connect: (d: object) => void edges.add(`${name}->${label.get(d)}`),
        disconnect: (d: object) => {
          if (!edges.delete(`${name}->${label.get(d)}`)) throw new Error('not connected');
        },
      };
      label.set(n, name);
      return n;
    };
    let loaded!: () => void;
    const context: any = {
      sampleRate: 48000,
      audioWorklet: { addModule: () => new Promise<void>((r) => (loaded = r)) },
    };
    context.destination = node('dest', { channelCount: 2 });
    const gain = node('gain', { context });
    gain.connect(context.destination);
    const ctx = { state: 'running', currentTime: 0, resume: async () => {} };
    const be = createSuperdoughBackend({
      getContext: () => ctx,
      init: async () => {},
      output: () => {},
      setSeed: () => {},
      getController: () => ({ output: { destinationGain: gain } }),
      createMasterTap: (deps) =>
        createMasterTap({
          ...deps,
          moduleUrl: 'u',
          createNode: () =>
            node('worklet', { port: { onmessage: null, postMessage: () => {} } }) as any,
        }),
    });
    // what the runtime does on a gesture: resume, then init, then play
    await be.resume();
    let initDone = false;
    const init = be.init().then(() => void (initDone = true));
    await new Promise((r) => setTimeout(r, 5));
    expect(initDone).toBe(false); // still waiting for the module
    expect(be.masterLatencySec!()).toBe(0);
    loaded();
    await init;
    expect([...edges].sort()).toEqual(['gain->worklet', 'worklet->dest']);
    expect(be.masterLatencySec!()).toBe(72 / 48000);
    await be.output({} as any, 0, 1);
    expect([...edges].sort()).toEqual(['gain->worklet', 'worklet->dest']);
  });

  test('a failed module load still lets init() resolve (the direct edge stays)', async () => {
    const context: any = {
      sampleRate: 48000,
      destination: { channelCount: 2 },
      audioWorklet: { addModule: () => Promise.reject(new Error('nope')) },
    };
    const gain: any = { context, connect() {}, disconnect() {} };
    const be = createSuperdoughBackend({
      getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
      init: async () => {},
      output: () => {},
      setSeed: () => {},
      getController: () => ({ output: { destinationGain: gain } }),
    });
    const err = console.error;
    console.error = () => {};
    try {
      await be.resume();
      await be.init();
    } finally {
      console.error = err;
    }
    expect(be.limiterAvailable!()).toBe(false);
    expect(be.masterLatencySec!()).toBe(0);
  });

  test('the default tap stays out of the way without a controller', async () => {
    const be = createSuperdoughBackend({
      getContext: () => ({ state: 'running', currentTime: 0, resume: async () => {} }),
      init: async () => {},
      output: () => {},
      setSeed: () => {},
    });
    await be.resume();
    await be.output({} as any, 0, 1);
    expect(be.limiterAvailable!()).toBe(true);
    expect(be.masterLatencySec!()).toBe(0);
  });
});

describe('fakeAudioGraph', () => {
  test('records nodes, connects, schedules and automation; settable clock and latencies', () => {
    const f = fakeAudioGraph({ currentTime: 1, baseLatency: 0.01 });
    const ctx = f.graph.context;
    expect(ctx.currentTime).toBe(1);
    f.setTime(2);
    expect(ctx.currentTime).toBe(2);
    expect(ctx.baseLatency).toBe(0.01);
    expect('outputLatency' in ctx).toBe(false);
    f.setLatency({ outputLatency: 0.03 });
    expect(ctx.outputLatency).toBe(0.03);
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.frequency.value = 900;
    osc.connect(gain);
    gain.connect(f.graph.destination);
    gain.gain.setValueAtTime(0, 2);
    osc.start(2);
    osc.stop(2.1);
    expect(f.oscillators()[0]).toMatchObject({
      frequency: { value: 900 },
      starts: [2],
      stops: [2.1],
    });
    expect(f.gains()[0]!.automation).toEqual([['setValueAtTime', 0, 2]]);
    expect(f.destination.connects).toEqual([f.gains()[0]]);
    expect(ctx.getOutputTimestamp()).toEqual({ contextTime: 0, performanceTime: 0 });
    f.setOutputTimestamp('missing');
    expect('getOutputTimestamp' in ctx).toBe(false);
  });
});

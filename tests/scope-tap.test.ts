import { describe, expect, test } from 'bun:test';
import { createScopeTap, type ScopeController } from '../src/lib/audio/scope-tap';
import { createSuperdoughBackend } from '../src/lib/audio/superdough';
import { createRuntime } from '../src/lib/strudel/runtime';
import type { AudioBackend } from '../src/lib/audio/backend';
import { fakeRuntime } from './helpers/fake-runtime';

function fakeAnalyser() {
  return {
    fftSize: 0,
    reads: 0,
    getFloatTimeDomainData(buf: Float32Array) {
      this.reads++;
      buf.fill(0.25);
    },
  };
}
type FakeAnalyser = ReturnType<typeof fakeAnalyser>;

type FakeContext = { createAnalyser(): FakeAnalyser };

function fakeContext(analysers: FakeAnalyser[]): FakeContext {
  return {
    createAnalyser: () => {
      const a = fakeAnalyser();
      analysers.push(a);
      return a;
    },
  };
}

/** A node on `context`; `connect` throws (as a cross-context connect does) when `throwOnConnect`. */
function fakeNode(
  name: string,
  log: string[],
  context: FakeContext,
  opts: { throwOnDisconnect?: boolean; throwOnConnect?: boolean } = {},
) {
  return {
    connected: [] as FakeAnalyser[],
    connect(dest: FakeAnalyser) {
      log.push(`connect ${name}`);
      if (opts.throwOnConnect) throw new Error('InvalidAccessError');
      this.connected.push(dest);
    },
    disconnect: () => {
      log.push(`disconnect ${name}`);
      if (opts.throwOnDisconnect) throw new Error('not connected');
    },
    context,
  };
}

function setup(opts: { running?: boolean } = {}) {
  const log: string[] = [];
  const analysers: FakeAnalyser[] = [];
  let running = opts.running ?? true;
  let controllerCalls = 0;
  const context = fakeContext(analysers);
  let controller: ScopeController = { output: { destinationGain: fakeNode('a', log, context) } };
  const tap = createScopeTap({
    isRunning: () => running,
    getController: () => (controllerCalls++, controller),
  });
  return {
    tap,
    log,
    analysers,
    controllerCalls: () => controllerCalls,
    setRunning: (r: boolean) => void (running = r),
    setController: (c: ScopeController) => void (controller = c),
    /** A node on the first context, or on `ctx`. */
    node: (
      name: string,
      opts: { throwOnDisconnect?: boolean; throwOnConnect?: boolean; ctx?: FakeContext } = {},
    ) => fakeNode(name, log, opts.ctx ?? context, opts),
    newContext: () => fakeContext(analysers),
  };
}

describe('createScopeTap', () => {
  test('getController is never called while not running', () => {
    const s = setup({ running: false });
    const buf = new Float32Array(2048);
    for (let i = 0; i < 3; i++) expect(s.tap.read(buf)).toBe(false);
    expect(s.controllerCalls()).toBe(0);
    expect(s.log).toEqual([]);
    expect(buf.every((v) => v === 0)).toBe(true);
    s.setRunning(true);
    expect(s.tap.read(buf)).toBe(true);
    expect(s.controllerCalls()).toBe(1);
    s.setRunning(false);
    expect(s.tap.read(buf)).toBe(false);
    expect(s.controllerCalls()).toBe(1);
  });

  test('connects once and creates one analyser with fftSize 2048', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    for (let i = 0; i < 4; i++) expect(s.tap.read(buf)).toBe(true);
    expect(s.log).toEqual(['connect a']);
    expect(s.analysers).toHaveLength(1);
    expect(s.analysers[0]!.fftSize).toBe(2048);
    expect(s.analysers[0]!.reads).toBe(4);
  });

  test('fills the buffer', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    expect(s.tap.read(buf)).toBe(true);
    expect(buf.every((v) => v === 0.25)).toBe(true);
  });

  test('reconnects after destinationGain changes, disconnecting the old node', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.tap.read(buf);
    s.setController({ output: { destinationGain: s.node('b') } });
    expect(s.tap.read(buf)).toBe(true);
    s.tap.read(buf);
    expect(s.log).toEqual(['connect a', 'disconnect a', 'connect b']);
    expect(s.analysers).toHaveLength(1); // the analyser is reused
  });

  test('a throwing disconnect is tolerated', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.setController({ output: { destinationGain: s.node('a', { throwOnDisconnect: true }) } });
    s.tap.read(buf);
    s.setController({ output: { destinationGain: s.node('b') } });
    expect(s.tap.read(buf)).toBe(true);
    expect(s.log).toEqual(['connect a', 'disconnect a', 'connect b']);
  });

  test('a null destinationGain or a null controller returns false', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.setController({ output: { destinationGain: null } });
    expect(s.tap.read(buf)).toBe(false);
    s.setController(null);
    expect(s.tap.read(buf)).toBe(false);
    expect(s.log).toEqual([]);
    expect(s.analysers).toHaveLength(0);
  });

  test('a node on a replaced AudioContext gets a new analyser from that context', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.tap.read(buf);
    const b = s.node('b', { ctx: s.newContext() });
    s.setController({ output: { destinationGain: b } });
    expect(s.tap.read(buf)).toBe(true);
    expect(s.log).toEqual(['connect a', 'disconnect a', 'connect b']);
    expect(s.analysers).toHaveLength(2);
    expect(s.analysers[1]!.fftSize).toBe(2048);
    expect(b.connected).toEqual([s.analysers[1]!]);
    expect(s.analysers[1]!.reads).toBe(1);
    expect(s.analysers[0]!.reads).toBe(1); // the old one is no longer read
  });

  test('a throwing connect returns false, never throws, and is retried on the next read', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    const bad = s.node('x', { throwOnConnect: true });
    s.setController({ output: { destinationGain: bad } });
    expect(() => s.tap.read(buf)).not.toThrow();
    expect(s.tap.read(buf)).toBe(false);
    expect(s.log).toEqual(['connect x', 'connect x']);
    s.setController({ output: { destinationGain: s.node('y') } });
    expect(s.tap.read(buf)).toBe(true);
    expect(s.log).toEqual(['connect x', 'connect x', 'connect y']); // x never became the source, so it is not disconnected
  });

  test('a throwing controller or analyser read returns false', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.tap.read(buf);
    s.analysers[0]!.getFloatTimeDomainData = () => {
      throw new Error('closed');
    };
    expect(s.tap.read(buf)).toBe(false);
    const t = createScopeTap({
      isRunning: () => true,
      getController: () => {
        throw new Error('no context');
      },
    });
    expect(t.read(buf)).toBe(false);
  });

  test('after a reset nulls the node, the recreated node is connected', () => {
    const s = setup();
    const buf = new Float32Array(2048);
    s.tap.read(buf);
    s.setController({ output: { destinationGain: null } });
    expect(s.tap.read(buf)).toBe(false);
    s.setController({ output: { destinationGain: s.node('c') } });
    expect(s.tap.read(buf)).toBe(true);
    expect(s.log).toEqual(['connect a', 'disconnect a', 'connect c']);
  });
});

describe('readScope', () => {
  const ctx = () => ({ state: 'running', currentTime: 0, resume: async () => {} });
  const base = { init: async () => {}, output: () => {}, setSeed: () => {} };

  test('the superdough backend delegates to the tap once running', async () => {
    const log: string[] = [];
    const analysers: FakeAnalyser[] = [];
    let calls = 0;
    const node = fakeNode('out', log, fakeContext(analysers));
    const be = createSuperdoughBackend({
      ...base,
      getContext: ctx,
      // The fake has only what the tap reads.
      getController: () => (calls++, { output: { destinationGain: node as never } }),
    });
    const buf = new Float32Array(2048);
    expect(be.readScope!(buf)).toBe(false); // no context yet: not running
    expect(calls).toBe(0);
    await be.resume();
    expect(be.readScope!(buf)).toBe(true);
    expect(buf[0]).toBe(0.25);
    expect(log).toEqual(['connect out']);
  });

  test('a superdough backend built without getController returns false', async () => {
    const be = createSuperdoughBackend({ ...base, getContext: ctx });
    await be.resume();
    expect(be.readScope!(new Float32Array(2048))).toBe(false);
  });

  test('runtime.readScope returns the backend result', () => {
    const seen: Float32Array[] = [];
    const backend: AudioBackend = {
      id: 'superdough',
      init: async () => {},
      resume: async () => true,
      isRunning: () => true,
      getTime: () => 0,
      setSeed: () => {},
      output: () => {},
      readScope: (buf) => (seen.push(buf), true),
    };
    const rt = createRuntime({ backends: { superdough: backend }, prebake: async () => {} });
    const buf = new Float32Array(4);
    expect(rt.readScope(buf)).toBe(true);
    expect(seen).toEqual([buf]);
  });

  test('a backend without readScope gives false', () => {
    expect(fakeRuntime().rt.readScope(new Float32Array(4))).toBe(false);
  });
});

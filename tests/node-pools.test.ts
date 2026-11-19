import { afterAll, afterEach, beforeAll, describe, expect, test } from 'bun:test';
import {
  getNodeFromPool,
  getWorkletFromPool,
  isPoolable,
  releaseNodeToPool,
  setNodePooling,
} from 'superdough/nodePools.mjs';
import { stubGlobals } from './helpers/stub-globals';

// releaseNodeToPool, getNodeFromPool and releaseAudioNode test nodes with instanceof against Web
// Audio classes, which bun does not have. Fake nodes are AudioNodes; fake worklets also
// AudioWorkletNodes.
class FakeAudioNode {}
class FakeWorkletNode extends FakeAudioNode {}
let restore: () => void;
beforeAll(() => {
  restore = stubGlobals(
    {
      AudioNode: FakeAudioNode,
      AudioWorkletNode: FakeWorkletNode,
      AudioScheduledSourceNode: class {},
      AudioParam: class {},
    },
    { onlyIfMissing: true },
  );
});
afterAll(() => restore());
afterEach(() => setNodePooling(true));

let nextKey = 0;
/** A fresh pool key per test, so tests never see each other's pooled nodes. */
const key = () => `test-${nextKey++}`;

function fakeNode() {
  return Object.assign(Object.create(FakeAudioNode.prototype), {
    disconnects: 0,
    disconnect() {
      this.disconnects++;
    },
    context: { currentTime: 0 },
  });
}

describe('node pool', () => {
  test('pooling is on by default and reuses a released node', () => {
    expect(setNodePooling(true)).toBe(true); // on by default; returns the previous setting
    const k = key();
    const a = getNodeFromPool(k, fakeNode);
    releaseNodeToPool(a);
    expect(a.disconnects).toBe(1);
    expect(getNodeFromPool(k, fakeNode)).toBe(a);
  });

  test('off: every request builds a new node, still tagged as poolable', () => {
    expect(setNodePooling(false)).toBe(true);
    expect(setNodePooling(false)).toBe(false);
    const k = key();
    let built = 0;
    const factory = () => (built++, fakeNode());
    const a = getNodeFromPool(k, factory);
    releaseNodeToPool(a);
    const b = getNodeFromPool(k, factory);
    expect(b).not.toBe(a);
    expect(built).toBe(2);
    expect(isPoolable(a)).toBe(true);
    expect(a.disconnects).toBeGreaterThan(0);
  });

  test('turning pooling off empties the pool', () => {
    const k = key();
    const a = getNodeFromPool(k, fakeNode);
    releaseNodeToPool(a);
    setNodePooling(false);
    setNodePooling(true);
    expect(getNodeFromPool(k, fakeNode)).not.toBe(a);
  });

  test('a node released while off is not reused after turning it back on', () => {
    const k = key();
    setNodePooling(false);
    const a = getNodeFromPool(k, fakeNode);
    releaseNodeToPool(a);
    setNodePooling(true);
    expect(getNodeFromPool(k, fakeNode)).not.toBe(a);
  });
});

describe('getWorkletFromPool', () => {
  function fakeWorklet(options: unknown) {
    const messages: unknown[] = [];
    const ends: [number, number][] = [];
    return Object.assign(Object.create(FakeWorkletNode.prototype), {
      ...fakeNode(),
      options,
      messages,
      ends,
      port: { postMessage: (m: unknown) => messages.push(m) },
      parameters: {
        get: (name: string) =>
          name === 'end'
            ? { value: 1, setValueAtTime: (v: number, t: number) => ends.push([v, t]) }
            : undefined,
        forEach: () => {},
      },
    });
  }

  test('a new node gets the payload in processorOptions and no message', () => {
    const k = key();
    const payload = { seed: 7 };
    const node = getWorkletFromPool(k, fakeWorklet, payload);
    expect(node.options).toEqual({ processorOptions: payload });
    expect(node.messages).toEqual([]);
  });

  test('a pooled node gets the payload in an initialize message', () => {
    const k = key();
    const a = getWorkletFromPool(k, fakeWorklet, { seed: 1 });
    releaseNodeToPool(a);
    const b = getWorkletFromPool(k, fakeWorklet, { seed: 2 });
    expect(b).toBe(a);
    expect(b.messages).toEqual([{ type: 'initialize', payload: { seed: 2 } }]);
  });

  test('with pooling off every node is new and seeded at construction', () => {
    setNodePooling(false);
    const k = key();
    const a = getWorkletFromPool(k, fakeWorklet, { seed: 1 });
    releaseNodeToPool(a);
    const b = getWorkletFromPool(k, fakeWorklet, { seed: 2 });
    expect(b).not.toBe(a);
    expect(b.options).toEqual({ processorOptions: { seed: 2 } });
    expect(b.messages).toEqual([]);
  });

  test('with pooling off a released worklet is told to end, as releaseAudioNode does', () => {
    setNodePooling(false);
    const node = getWorkletFromPool(key(), fakeWorklet, {});
    releaseNodeToPool(node);
    expect(node.ends).toEqual([[0, 0]]);
  });

  test('with pooling on a released worklet keeps running for reuse', () => {
    const node = getWorkletFromPool(key(), fakeWorklet, {});
    releaseNodeToPool(node);
    expect(node.ends).toEqual([]);
  });
});

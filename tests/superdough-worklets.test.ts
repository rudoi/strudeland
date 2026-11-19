import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mulberry32 } from 'superdough/rng.mjs';
import { stubGlobals } from './helpers/stub-globals';

// worklets.mjs runs in an AudioWorkletGlobalScope: give it the globals it reads at load and at
// construction, then reach the processors through registerProcessor.
const processors = new Map<string, any>();
let restore: () => void;

beforeAll(async () => {
  restore = stubGlobals({
    AudioWorkletProcessor: class {
      port: { onmessage?: (e: { data: unknown }) => void } = {};
    },
    registerProcessor: (name: string, cls: any) => processors.set(name, cls),
    sampleRate: 48000,
    currentTime: 0,
  });
  await import('superdough/worklets.mjs');
});
afterAll(() => restore());

const take = (f: () => number, n = 8) => Array.from({ length: n }, () => f());
const SEED = 123456789;
const FRAMES = [new Float32Array([0, 1, 0, -1])];

const cases = [
  { name: 'supersaw-oscillator', payload: { seed: SEED } },
  {
    name: 'wavetable-oscillator-processor',
    payload: { seed: SEED, key: 'test-table', frameLen: 4, frames: FRAMES },
  },
];

describe.each(cases)('$name', ({ name, payload }) => {
  test('is seeded from processorOptions at construction', () => {
    const p = new (processors.get(name))({ processorOptions: payload });
    expect(take(p.random)).toEqual(take(mulberry32(SEED)));
    expect(p.phase).toEqual([]);
  });

  test('construction and the initialize message give the same stream', () => {
    const byOptions = new (processors.get(name))({ processorOptions: payload });
    const byMessage = new (processors.get(name))({});
    byMessage.port.onmessage({ data: { type: 'initialize', payload } });
    expect(take(byMessage.random)).toEqual(take(byOptions.random));
  });

  test('without options it keeps Math.random', () => {
    expect(new (processors.get(name))().random).toBe(Math.random);
    expect(new (processors.get(name))({}).random).toBe(Math.random);
  });
});

test('wavetable frames arrive with processorOptions', () => {
  const p = new (processors.get('wavetable-oscillator-processor'))({
    processorOptions: { key: 'test-frames', frameLen: 4, frames: FRAMES },
  });
  expect(p.table).toBe(FRAMES);
  expect(p.frameLen).toBe(4);
});

import { afterAll, beforeAll, expect, test } from 'bun:test';
import reverbGen from 'superdough/reverbGen.mjs';
import { stubGlobals } from './helpers/stub-globals';

// reverb.mjs extends BaseAudioContext.prototype at load when AudioContext exists; bun has neither.
// Other test files load it through superdough's index without them, so this file loads its own
// instance (the query makes a separate module) after defining the globals.
const originalGenerate = reverbGen.generateReverb;
let pending: { decayTime: number; done: (buffer: unknown) => void }[] = [];
let Context: any;
let restore: () => void;

beforeAll(async () => {
  Context = class {
    createConvolver() {
      return { buffer: null };
    }
  };
  restore = stubGlobals({ BaseAudioContext: Context, AudioContext: Context });
  reverbGen.generateReverb = (params: any, done: (buffer: unknown) => void) => {
    pending.push({ decayTime: params.decayTime, done });
  };
  await import(`${Bun.resolveSync('superdough/reverb.mjs', import.meta.dir)}?with-context`);
});
afterAll(() => {
  reverbGen.generateReverb = originalGenerate;
  restore();
});

const finish = (decayTime: number) => {
  const i = pending.findIndex((p) => p.decayTime === decayTime);
  pending.splice(i, 1)[0]!.done(`ir ${decayTime}`);
};

test('reverb.mjs extended the context prototype', () => {
  expect(typeof Context.prototype.createReverb).toBe('function');
});

test('an IR that finishes after a later request does not replace it', () => {
  pending = [];
  const conv = new Context().createReverb(5);
  conv.generate(8);
  expect(pending.map((p) => p.decayTime)).toEqual([5, 8]);
  finish(8);
  finish(5);
  expect(conv.buffer).toBe('ir 8');
  expect(conv.duration).toBe(8);
});

test('IRs that finish in request order are each applied, ending with the latest', () => {
  pending = [];
  const conv = new Context().createReverb(5);
  conv.generate(8);
  finish(5);
  expect(conv.buffer).toBe('ir 5');
  finish(8);
  expect(conv.buffer).toBe('ir 8');
});

test('a stream of requests is not starved: a finished IR newer than the buffer applies', () => {
  pending = [];
  const conv = new Context().createReverb(1);
  conv.generate(2);
  finish(1);
  conv.generate(3);
  finish(2);
  expect(conv.buffer).toBe('ir 2');
  conv.generate(4);
  finish(4);
  finish(3);
  expect(conv.buffer).toBe('ir 4');
});

test('each convolver counts its own requests', () => {
  pending = [];
  const a = new Context().createReverb(3);
  const b = new Context().createReverb(4);
  finish(3);
  finish(4);
  expect(a.buffer).toBe('ir 3');
  expect(b.buffer).toBe('ir 4');
});

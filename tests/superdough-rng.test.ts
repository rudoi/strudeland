import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  getAudioSeed,
  hashSeed,
  makeRandom,
  mulberry32,
  randomFromSeed,
  setAudioSeed,
  voiceSeed,
} from 'superdough/rng.mjs';
import { setAudioContext } from 'superdough/audioContext.mjs';
import { getNoiseBuffer } from 'superdough/noise.mjs';
import { applyFM } from 'superdough/helpers.mjs';
import { Orbit } from 'superdough/superdoughoutput.mjs';
import reverbGen from 'superdough/reverbGen.mjs';
import { buildSamples } from 'superdough/zzfx_fork.mjs';

const SUPERDOUGH = resolve(import.meta.dir, '../vendor/strudel/packages/superdough');
const SAMPLE_RATE = 8000;

/** Enough of an AudioContext for buffer generation: createBuffer and sampleRate. */
function fakeContext() {
  return {
    state: 'running',
    sampleRate: SAMPLE_RATE,
    currentTime: 0,
    close() {},
    createBuffer(numberOfChannels: number, length: number, sampleRate: number) {
      const channels = Array.from({ length: numberOfChannels }, () => new Float32Array(length));
      return { numberOfChannels, length, sampleRate, getChannelData: (i: number) => channels[i] };
    },
  };
}

const take = (f: () => number, n = 16) => Array.from({ length: n }, () => f());
const samples = (buffer: any) => Array.from(buffer.getChannelData(0) as Float32Array);

let seedBefore: number | undefined;
beforeAll(() => {
  seedBefore = getAudioSeed();
  setAudioContext(fakeContext());
});
afterAll(() => setAudioSeed(seedBefore));

describe('mulberry32', () => {
  test('same seed gives the same sequence', () => {
    expect(take(mulberry32(42))).toEqual(take(mulberry32(42)));
  });
  test('different seeds differ', () => {
    expect(take(mulberry32(1))).not.toEqual(take(mulberry32(2)));
  });
  test('values are in [0, 1)', () => {
    for (const v of take(mulberry32(123), 10000)) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });
});

describe('hashSeed', () => {
  test('known values (32-bit FNV-1a over typed parts)', () => {
    expect(hashSeed()).toBe(2166136261);
    expect(hashSeed(1)).toBe(1255242637);
    expect(hashSeed(7, 'noise', 'white', 48000)).toBe(207077153);
  });
  test('order and type sensitive', () => {
    expect(hashSeed(1)).not.toBe(hashSeed('1'));
    expect(hashSeed(1, 2)).not.toBe(hashSeed(2, 1));
  });
  test('returns an unsigned 32-bit integer', () => {
    for (const parts of [[0], [-1], ['x', 1.5], [1e9, 'a', true as any]]) {
      const h = hashSeed(...parts);
      expect(Number.isInteger(h)).toBe(true);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(2 ** 32);
    }
  });
});

describe('unseeded', () => {
  test('every source falls back to Math.random', () => {
    setAudioSeed(undefined);
    expect(makeRandom('x')).toBe(Math.random);
    expect(randomFromSeed(undefined)).toBe(Math.random);
    expect(voiceSeed(0, { s: 'bd' })).toBeUndefined();
  });
  test('randomFromSeed with a number is seeded', () => {
    expect(randomFromSeed(3)).not.toBe(Math.random);
    expect(take(randomFromSeed(3))).toEqual(take(mulberry32(3)));
  });
  test('makeRandom with a seed depends on seed and key only', () => {
    setAudioSeed(4);
    expect(take(makeRandom('a', 1))).toEqual(take(makeRandom('a', 1)));
    expect(take(makeRandom('a', 1))).not.toEqual(take(makeRandom('a', 2)));
    setAudioSeed(5);
    const five = take(makeRandom('a', 1));
    setAudioSeed(4);
    expect(take(makeRandom('a', 1))).not.toEqual(five);
  });
});

describe('noise', () => {
  test('white is identical across two fresh caches for a fixed seed', () => {
    setAudioSeed(7);
    const first = getNoiseBuffer('white');
    expect(getNoiseBuffer('white')).toBe(first); // cached
    setAudioSeed(8);
    setAudioSeed(7); // a seed change empties the cache
    const second = getNoiseBuffer('white');
    expect(second).not.toBe(first);
    expect(samples(second)).toEqual(samples(first));
  });
  test('generating pink first does not change white', () => {
    setAudioSeed(11);
    const white = samples(getNoiseBuffer('white'));
    setAudioSeed(12);
    setAudioSeed(11);
    getNoiseBuffer('pink');
    getNoiseBuffer('brown');
    expect(samples(getNoiseBuffer('white'))).toEqual(white);
  });
  test('changing the seed empties the cache and changes the buffer', () => {
    setAudioSeed(1);
    const one = getNoiseBuffer('white');
    setAudioSeed(2);
    const two = getNoiseBuffer('white');
    expect(two).not.toBe(one);
    expect(samples(two)).not.toEqual(samples(one));
  });
  test('setting the same seed again keeps the cache', () => {
    setAudioSeed(3);
    const buf = getNoiseBuffer('pink');
    setAudioSeed(3);
    expect(getNoiseBuffer('pink')).toBe(buf);
  });
  test('crackle: same voice seed identical, different voice seeds differ, never cached', () => {
    setAudioSeed(7);
    const a = getNoiseBuffer('crackle', 20, 100);
    const b = getNoiseBuffer('crackle', 20, 100);
    expect(b).not.toBe(a);
    expect(samples(b)).toEqual(samples(a));
    expect(samples(getNoiseBuffer('crackle', 20, 101))).not.toEqual(samples(a));
    expect(samples(a).some((v) => v !== 0)).toBe(true);
  });
});

describe('reverb', () => {
  const ir = (decayTime: number) => {
    let out: any;
    reverbGen.generateReverb(
      {
        audioContext: fakeContext(),
        numChannels: 2,
        decayTime,
        fadeInTime: 0.01,
        lpFreqStart: 0,
        lpFreqEnd: 0,
      },
      (b: any) => (out = b),
    );
    return [samples(out), Array.from(out.getChannelData(1) as Float32Array)];
  };
  test('same params give the same IR regardless of what was generated before', () => {
    setAudioSeed(7);
    const first = ir(0.1);
    ir(0.2);
    getNoiseBuffer('white');
    expect(ir(0.1)).toEqual(first);
    // the two channels are not copies of each other
    expect(first[0]).not.toEqual(first[1]);
  });
  test('a different decay time differs', () => {
    setAudioSeed(7);
    expect(ir(0.1)[0].slice(0, 100)).not.toEqual(ir(0.11)[0].slice(0, 100));
  });
  test('a different seed differs', () => {
    setAudioSeed(7);
    const seven = ir(0.1);
    setAudioSeed(8);
    expect(ir(0.1)).not.toEqual(seven);
  });
});

describe('orbit reverb', () => {
  /** An orbit on a context whose createReverb counts how often an IR is generated. */
  function orbit() {
    const node = () => ({ connect: (n: any) => n, disconnect() {} });
    let generated = 0;
    const ac = {
      createReverb(...args: any[]) {
        const conv: any = { ...node() };
        conv.generate = (duration: any, fade: any, lp: any, dim: any, ir: any) => {
          generated++;
          Object.assign(conv, { duration, fade, lp, dim, ir });
        };
        conv.generate(...args);
        return conv;
      },
    };
    const g = globalThis as any;
    const before = g.GainNode;
    g.GainNode = function () {
      return node();
    };
    try {
      return { o: new Orbit(ac), generated: () => generated };
    } finally {
      g.GainNode = before;
    }
  }
  test('the same room settings reuse the IR; a seed change regenerates it once', () => {
    setAudioSeed(7);
    const { o, generated } = orbit();
    o.getReverb(1, 0.1, 15000, 1000);
    o.getReverb(1, 0.1, 15000, 1000);
    expect(generated()).toBe(1);
    setAudioSeed(8);
    o.getReverb(1, 0.1, 15000, 1000);
    expect(generated()).toBe(2);
    o.getReverb(1, 0.1, 15000, 1000);
    expect(generated()).toBe(2);
    setAudioSeed(8); // the same seed again is no change
    o.getReverb(1, 0.1, 15000, 1000);
    expect(generated()).toBe(2);
  });
});

describe('fm modulator noise', () => {
  /** The buffer of an fm modulator with `fmwave` crackle for a voice seed. */
  function crackleModulator(voiceSeed: number) {
    const node = () => ({ connect: (n: any) => n, start() {}, stop() {}, gain: { value: 0 } });
    const ac: any = {
      ...fakeContext(),
      createBufferSource: () => ({ ...node(), buffer: null, loop: false }),
      createGain: () => node(),
    };
    setAudioContext(ac);
    try {
      const param = { value: 440 };
      const { nodes } = applyFM(param, { fmi: 1, fmwave: 'crackle', _voiceSeed: voiceSeed }, 0);
      return samples(nodes.fm_1[0].buffer);
    } finally {
      setAudioContext(fakeContext());
    }
  }
  test('crackle follows the voice seed', () => {
    setAudioSeed(7);
    const a = crackleModulator(42);
    expect(a).toEqual(samples(getNoiseBuffer('crackle', 2, 42)));
    expect(crackleModulator(42)).toEqual(a);
    expect(crackleModulator(43)).not.toEqual(a);
  });
});

describe('zzfx', () => {
  const params = [0.25, 0.5, 220, 0, 0.05, 0.05, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0];
  test('the same voice seed gives identical samples', () => {
    expect(buildSamples(...params, randomFromSeed(3))).toEqual(
      buildSamples(...params, randomFromSeed(3)),
    );
  });
  test('a different voice seed changes the randomized frequency', () => {
    expect(buildSamples(...params, randomFromSeed(3))).not.toEqual(
      buildSamples(...params, randomFromSeed(4)),
    );
  });
  test('a short parameter array still gets its defaults', () => {
    const short = Array.from({ length: 20 }, (_, i) => [0.25, 0.1][i]);
    const out = buildSamples(...short, randomFromSeed(3));
    expect(out.length).toBeGreaterThan(0);
    expect(out.every((v: number) => Number.isFinite(v))).toBe(true);
    expect(out).toEqual(buildSamples(...short, randomFromSeed(3)));
  });
});

describe('voiceSeed', () => {
  test('the same (cycle, value) twice gives different seeds', () => {
    setAudioSeed(7);
    const a = voiceSeed(1, { s: 'bd' });
    const b = voiceSeed(1, { s: 'bd' });
    expect(typeof a).toBe('number');
    expect(b).not.toBe(a);
  });
  test('the occurrence counter restarts when the cycle changes', () => {
    setAudioSeed(9);
    setAudioSeed(7); // fresh counter
    const fresh = [voiceSeed(2, { s: 'bd' }), voiceSeed(2, { s: 'bd' })];
    setAudioSeed(9);
    setAudioSeed(7);
    voiceSeed(1, { s: 'bd' });
    voiceSeed(1, { s: 'bd' });
    voiceSeed(1, { s: 'bd' });
    expect([voiceSeed(2, { s: 'bd' }), voiceSeed(2, { s: 'bd' })]).toEqual(fresh);
  });
  test('the seed depends on the audio seed', () => {
    setAudioSeed(1);
    const one = voiceSeed(0, { s: 'bd' });
    setAudioSeed(2);
    expect(voiceSeed(0, { s: 'bd' })).not.toBe(one);
  });
  const first = (value: object, cycle = 3) => {
    setAudioSeed(100);
    setAudioSeed(7);
    return voiceSeed(cycle, value);
  };
  test('a value differing in one key gives a different seed', () => {
    expect(first({ s: 'bd', n: 1 })).not.toBe(first({ s: 'bd', n: 2 }));
    expect(first({ s: 'bd', pan: [0, 1] })).not.toBe(first({ s: 'bd', pan: [1, 0] }));
  });
  test('the onset cycle matters', () => {
    expect(first({ s: 'bd' }, 3)).not.toBe(first({ s: 'bd' }, 3.5));
  });
  test('underscore keys, undefined and functions do not change it', () => {
    const base = first({ s: 'bd', n: 1 });
    expect(first({ s: 'bd', n: 1, _voiceSeed: 99, _x: 'y' })).toBe(base);
    expect(first({ s: 'bd', n: 1, gain: undefined })).toBe(base);
    expect(first({ s: 'bd', n: 1, onTrigger: () => {} })).toBe(base);
  });
  test('key order does not matter, also inside nested objects', () => {
    expect(first({ s: 'bd', n: 1 })).toBe(first({ n: 1, s: 'bd' }));
    expect(first({ x: { a: 1, b: [2, { c: 3, d: 4 }] } })).toBe(
      first({ x: { b: [2, { d: 4, c: 3 }], a: 1 } }),
    );
  });
});

test('guard: Math.random( in superdough only in rng.mjs and the chainID line', () => {
  const hits: string[] = [];
  for (const name of readdirSync(SUPERDOUGH).filter((f) => f.endsWith('.mjs'))) {
    readFileSync(join(SUPERDOUGH, name), 'utf8')
      .split('\n')
      .forEach((line, i) => {
        if (!line.includes('Math.random(')) return;
        if (name === 'rng.mjs') return;
        if (name === 'superdough.mjs' && /^\s*const chainID = /.test(line)) return;
        hits.push(`${name}:${i + 1}: ${line.trim()}`);
      });
  }
  expect(hits).toEqual([]);
});

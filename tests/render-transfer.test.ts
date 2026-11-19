import { afterEach, describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';
import {
  decodeChannel,
  HarnessError,
  isFatalHarnessError,
  mapBatched,
  pageError,
  RendererStalledError,
  resolveStatic,
  withTimeout,
} from '../scripts/render/transfer';
import { dedupeErrors, encodeChannel, isErrorLog, SLICE_BYTES } from '../src/render/pcm';

const b64 = (f: Float32Array) =>
  Buffer.from(f.buffer, f.byteOffset, f.byteLength).toString('base64');

describe('decodeChannel', () => {
  test('round trips a Float32Array at an odd byteOffset of a pooled buffer', () => {
    const values = new Float32Array([0, 1, -1, 0.5, 1e-7, -0.25]);
    const out = decodeChannel(b64(values));
    expect(out.byteOffset).toBe(0);
    expect(Array.from(out)).toEqual(Array.from(values));
    // A Buffer whose byteOffset is odd still decodes correctly.
    const backing = Buffer.alloc(values.byteLength + 1);
    Buffer.from(values.buffer).copy(backing, 1);
    const odd = backing.subarray(1);
    expect(odd.byteOffset % 4).not.toBe(0);
    expect(Array.from(decodeChannel(odd.toString('base64')))).toEqual(Array.from(values));
  });

  test('empty input is an empty channel', () => {
    expect(decodeChannel('').length).toBe(0);
  });

  test('rejects a length that is not a multiple of 4', () => {
    expect(() => decodeChannel(Buffer.from([1, 2, 3]).toString('base64'))).toThrow(
      /not a multiple of 4/,
    );
  });
});

describe('encodeChannel (page side)', () => {
  test('round trips through decodeChannel, across several slices', () => {
    const n = Math.ceil((SLICE_BYTES * 3) / 4) + 7; // more than three slices, uneven tail
    const f = new Float32Array(n);
    for (let i = 0; i < n; i++) f[i] = Math.sin(i * 0.01) * (i % 7) - 3;
    const s = encodeChannel(f);
    expect(s).toBe(b64(f));
    expect(decodeChannel(s)).toEqual(f);
  });

  test('a subarray encodes only its own bytes', () => {
    const f = new Float32Array([9, 1, 2, 9]).subarray(1, 3);
    expect(Array.from(decodeChannel(encodeChannel(f)))).toEqual([1, 2]);
  });

  test('empty channel', () => {
    expect(encodeChannel(new Float32Array(0))).toBe('');
  });

  test('slices are a multiple of 3 bytes, so no padding lands mid-string', () => {
    expect(SLICE_BYTES % 3).toBe(0);
  });
});

describe('page error helpers', () => {
  test('dedupeErrors keeps the first of each kind and message, in order', () => {
    const a = { kind: 'hap' as const, message: 'x', bar: 1 };
    const b = { kind: 'other' as const, message: 'x' };
    const c = { kind: 'hap' as const, message: 'x', bar: 2 };
    expect(dedupeErrors([a, b, c, b])).toEqual([a, b]);
    expect(dedupeErrors([])).toEqual([]);
  });

  test('isErrorLog: error type and failure text, not the worklet success line', () => {
    expect(isErrorLog('anything', 'error')).toBe(true);
    expect(isErrorLog('[cyclist] error: boom', undefined)).toBe(true);
    expect(isErrorLog('sound x not found! Is it loaded?', undefined)).toBe(true);
    expect(isErrorLog('could not load AudioWorklet effects', 'warning')).toBe(true);
    expect(isErrorLog('[superdough] AudioWorklets loaded', undefined)).toBe(false);
    expect(isErrorLog('[superdough] ready', undefined)).toBe(false);
  });

  test('pageError: worklet text is a worklet error, the rest other', () => {
    expect(pageError('[console] AudioWorklet processor crashed')).toEqual({
      kind: 'worklet',
      message: '[console] AudioWorklet processor crashed',
    });
    expect(pageError('[page error] boom').kind).toBe('other');
  });
});

describe('resolveStatic', () => {
  const root = resolve('/srv/dist-render');

  test('accepts paths inside root', () => {
    expect(resolveStatic(root, '/assets/a.js')).toBe(resolve(root, 'assets/a.js'));
    expect(resolveStatic(root, '/index.html')).toBe(resolve(root, 'index.html'));
    expect(resolveStatic(root, '/a%20b.wav')).toBe(resolve(root, 'a b.wav'));
    expect(resolveStatic(root, '/..foo.js')).toBe(resolve(root, '..foo.js'));
  });

  test('rejects escapes, the root itself and undecodable paths', () => {
    expect(resolveStatic(root, '/../x')).toBeNull();
    expect(resolveStatic(root, '/assets/../../x')).toBeNull();
    expect(resolveStatic(root, '/..%2fx')).toBeNull();
    expect(resolveStatic(root, '/%2e%2e/x')).toBeNull();
    expect(resolveStatic(root, '/')).toBeNull();
    expect(resolveStatic(root, '/%E0%A4%A')).toBeNull();
    expect(resolveStatic(root, '/a%00b')).toBeNull();
  });
});

describe('withTimeout and harness errors', () => {
  test('resolves with the value', async () => {
    expect(await withTimeout(Promise.resolve(7), 50, 'x')).toBe(7);
  });

  test('rejects with RendererStalledError (a stalled HarnessError) after ms', async () => {
    const never = new Promise<never>(() => {});
    const err = await withTimeout(never, 5, 'render kickoff').catch((e) => e);
    expect(err).toBeInstanceOf(RendererStalledError);
    expect(err).toBeInstanceOf(HarnessError);
    expect(err.code).toBe('stalled');
    expect(err.message).toContain('render kickoff');
  });

  test('passes through a rejection of the wrapped promise', async () => {
    const err = await withTimeout(Promise.reject(new Error('boom')), 1000, 'x').catch((e) => e);
    expect(err.message).toBe('boom');
    expect(err).not.toBeInstanceOf(HarnessError);
  });

  test('clears the timer: no late rejection after the value', async () => {
    const orig = globalThis.clearTimeout;
    const cleared: unknown[] = [];
    globalThis.clearTimeout = ((id: any) => {
      cleared.push(id);
      orig(id);
    }) as typeof clearTimeout;
    try {
      await withTimeout(Promise.resolve(1), 1000, 'x');
      await withTimeout(new Promise<never>(() => {}), 1, 'y').catch(() => {});
    } finally {
      globalThis.clearTimeout = orig;
    }
    expect(cleared.length).toBe(2);
    expect(cleared.every((id) => id !== undefined)).toBe(true);
  });

  test('HarnessError keeps its code and message', () => {
    for (const code of [
      'chromium-missing',
      'build-missing',
      'eval-failed',
      'render-failed',
      'stalled',
    ] as const) {
      const e = new HarnessError(code, `m ${code}`);
      expect(e).toBeInstanceOf(Error);
      expect(e.code).toBe(code);
      expect(e.message).toBe(`m ${code}`);
      expect(e.name).toBe('HarnessError');
    }
  });
});

describe('isFatalHarnessError', () => {
  test('only the environment codes are fatal', () => {
    const fatal = (code: ConstructorParameters<typeof HarnessError>[0]) =>
      isFatalHarnessError(new HarnessError(code, ''));
    expect(fatal('chromium-missing')).toBe(true);
    expect(fatal('build-missing')).toBe(true);
    expect(fatal('eval-failed')).toBe(false);
    expect(fatal('render-failed')).toBe(false);
    expect(fatal('stalled')).toBe(false);
    expect(isFatalHarnessError(new RendererStalledError('x'))).toBe(false);
  });
});

describe('mapBatched', () => {
  test('at most n in flight, results in order', async () => {
    let active = 0;
    let max = 0;
    const out = await mapBatched([5, 4, 3, 2, 1, 0, 6], 3, async (v, i) => {
      max = Math.max(max, ++active);
      await new Promise((r) => setTimeout(r, v));
      active--;
      return `${i}:${v}`;
    });
    expect(out).toEqual(['0:5', '1:4', '2:3', '3:2', '4:1', '5:0', '6:6']);
    expect(max).toBe(3);
  });
  test('empty input and a bad batch size', async () => {
    expect(await mapBatched([], 2, async () => 1)).toEqual([]);
    await expect(mapBatched([1], 0, async () => 1)).rejects.toThrow('batch size');
  });
  test('a rejection rejects the whole map and starts no later batch', async () => {
    const started: number[] = [];
    const p = mapBatched([0, 1, 2, 3], 2, async (v) => {
      started.push(v);
      if (v === 1) throw new Error('boom');
      return v;
    });
    await expect(p).rejects.toThrow('boom');
    expect(started).toEqual([0, 1]);
  });
});

describe('render page guard', () => {
  const original = (globalThis as any).AudioContext;
  afterEach(() => {
    (globalThis as any).AudioContext = original;
  });

  test('importing it makes new AudioContext() throw', async () => {
    const { GUARD_MESSAGE } = await import('../src/render/guard');
    expect(() => new (globalThis as any).AudioContext()).toThrow(GUARD_MESSAGE);
    expect(GUARD_MESSAGE).toContain('before setAudioContext');
  });
});

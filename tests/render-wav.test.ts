import { describe, expect, test } from 'bun:test';
import { decodeWav, encodeWav, WavError } from '../src/lib/render/wav';
import { noiseSeeded } from './helpers/render-signals';

const ascii = (b: Uint8Array, off: number, n: number) =>
  String.fromCharCode(...b.subarray(off, off + n));

function expectBitExact(a: Float32Array[], b: Float32Array[]): void {
  expect(b.length).toBe(a.length);
  for (let c = 0; c < a.length; c++) {
    expect(b[c]!.length).toBe(a[c]!.length);
    for (let i = 0; i < a[c]!.length; i++) {
      if (!Object.is(a[c]![i], b[c]![i])) throw new Error(`ch ${c} sample ${i} differs`);
    }
  }
}

/** Build a WAV from raw chunks after the RIFF/WAVE header. */
function riff(...chunks: { id: string; body: Uint8Array; size?: number }[]): Uint8Array {
  const parts: number[] = [];
  const push = (s: string) => {
    for (const ch of s) parts.push(ch.charCodeAt(0));
  };
  const u32 = (v: number) => parts.push(v & 255, (v >>> 8) & 255, (v >>> 16) & 255, v >>> 24);
  push('RIFF');
  u32(0);
  push('WAVE');
  for (const c of chunks) {
    push(c.id);
    u32(c.size ?? c.body.length);
    parts.push(...c.body);
    if (c.body.length % 2) parts.push(0);
  }
  return new Uint8Array(parts);
}

function fmtBody(tag: number, ch: number, sr: number, bits: number, ext = false): Uint8Array {
  const b = new Uint8Array(ext ? 40 : 16);
  const v = new DataView(b.buffer);
  v.setUint16(0, ext ? 0xfffe : tag, true);
  v.setUint16(2, ch, true);
  v.setUint32(4, sr, true);
  v.setUint32(8, (sr * ch * bits) / 8, true);
  v.setUint16(12, (ch * bits) / 8, true);
  v.setUint16(14, bits, true);
  if (ext) {
    v.setUint16(16, 22, true);
    v.setUint16(18, bits, true);
    v.setUint32(20, 3, true);
    v.setUint16(24, tag, true);
  }
  return b;
}

function f32Body(interleaved: number[]): Uint8Array {
  const b = new Uint8Array(interleaved.length * 4);
  const v = new DataView(b.buffer);
  interleaved.forEach((x, i) => v.setFloat32(i * 4, x, true));
  return b;
}

describe('encodeWav / decodeWav f32', () => {
  test('bit-exact for mono, stereo and 6 channels with NaN and 1.5', () => {
    for (const nch of [1, 2, 6]) {
      const chans = Array.from({ length: nch }, (_, c) => {
        const x = noiseSeeded(c + 1, 257, 1.2);
        x[3] = NaN;
        x[4] = 1.5;
        x[5] = -Infinity;
        x[6] = -0;
        return x;
      });
      const d = decodeWav(encodeWav(chans, 44100, 'f32'));
      expect(d.format).toBe('f32');
      expect(d.sampleRate).toBe(44100);
      expectBitExact(chans, d.channels);
    }
  });

  test('lengths 0, 1, 3 and 4411', () => {
    for (const n of [0, 1, 3, 4411]) {
      const chans = [noiseSeeded(n, n), noiseSeeded(n + 9, n)];
      expectBitExact(chans, decodeWav(encodeWav(chans, 48000, 'f32')).channels);
    }
  });

  test('the bulk f32 path equals a per-sample DataView read at any byte offset', () => {
    /** The reference decode: interleaved little-endian floats from byte 44 of `b`. */
    const reference = (b: Uint8Array, nch: number): Float32Array[] => {
      const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
      const frames = Math.floor((b.length - 44) / (4 * nch));
      const out = Array.from({ length: nch }, () => new Float32Array(frames));
      for (let i = 0, off = 44; i < frames; i++) {
        for (let c = 0; c < nch; c++, off += 4) out[c]![i] = v.getFloat32(off, true);
      }
      return out;
    };
    for (const nch of [1, 2, 3, 6]) {
      const chans = Array.from({ length: nch }, (_, c) => noiseSeeded(c + 7, 1031, 1.5));
      chans[0]![1] = Infinity;
      chans[0]![2] = -0;
      const wav = encodeWav(chans, 44100, 'f32');
      for (const shift of [0, 1, 2, 3]) {
        const buf = new Uint8Array(wav.length + shift);
        buf.set(wav, shift);
        const b = buf.subarray(shift);
        expectBitExact(reference(b, nch), decodeWav(b).channels);
      }
      // a data chunk cut mid-frame keeps the whole frames only
      const cut = wav.subarray(0, wav.length - 2);
      const d = decodeWav(cut).channels;
      expect(d[0]!.length).toBe(1030);
      expectBitExact(reference(cut, nch), d);
    }
  });

  test('stereo f32 header bytes', () => {
    const b = encodeWav([new Float32Array(10), new Float32Array(10)], 44100, 'f32');
    const v = new DataView(b.buffer);
    expect(b.length).toBe(44 + 80);
    expect(ascii(b, 0, 4)).toBe('RIFF');
    expect(v.getUint32(4, true)).toBe(36 + 80);
    expect(ascii(b, 8, 8)).toBe('WAVEfmt ');
    expect(v.getUint32(16, true)).toBe(16);
    expect(v.getUint16(20, true)).toBe(3);
    expect(v.getUint16(22, true)).toBe(2);
    expect(v.getUint32(24, true)).toBe(44100);
    expect(v.getUint32(28, true)).toBe(44100 * 2 * 4);
    expect(v.getUint16(32, true)).toBe(8);
    expect(v.getUint16(34, true)).toBe(32);
    expect(ascii(b, 36, 4)).toBe('data');
    expect(v.getUint32(40, true)).toBe(80);
  });
});

describe('encodeWav / decodeWav i16', () => {
  test('round trip within 1/16384', () => {
    const x = noiseSeeded(7, 1000, 0.99);
    const d = decodeWav(encodeWav([x], 44100, 'i16'));
    expect(d.format).toBe('i16');
    const b = new DataView(encodeWav([x], 44100, 'i16').buffer);
    expect(b.getUint16(20, true)).toBe(1);
    expect(b.getUint16(34, true)).toBe(16);
    for (let i = 0; i < x.length; i++) {
      expect(Math.abs(d.channels[0]![i]! - x[i]!)).toBeLessThanOrEqual(1 / 16384);
    }
  });

  test('clamps +-2.0 to +-32767/32768', () => {
    const d = decodeWav(encodeWav([new Float32Array([2, -2, NaN])], 44100, 'i16'));
    expect(d.channels[0]![0]).toBeCloseTo(32767 / 32768, 7);
    expect(d.channels[0]![1]).toBeCloseTo(-32767 / 32768, 7);
    expect(d.channels[0]![2]).toBe(0);
  });
});

describe('decodeWav chunk handling', () => {
  test('odd-sized LIST chunk before data', () => {
    const bytes = riff(
      { id: 'fmt ', body: fmtBody(3, 1, 22050, 32) },
      { id: 'LIST', body: new Uint8Array([1, 2, 3]) },
      { id: 'data', body: f32Body([0.25, -0.5]) },
    );
    const d = decodeWav(bytes);
    expect(d.sampleRate).toBe(22050);
    expect(Array.from(d.channels[0]!)).toEqual([0.25, -0.5]);
  });

  test('extensible header', () => {
    const bytes = riff(
      { id: 'fmt ', body: fmtBody(3, 2, 44100, 32, true) },
      { id: 'data', body: f32Body([0.25, -0.5, 0.125, 1]) },
    );
    const d = decodeWav(bytes);
    expect(d.format).toBe('f32');
    expect(Array.from(d.channels[0]!)).toEqual([0.25, 0.125]);
    expect(Array.from(d.channels[1]!)).toEqual([-0.5, 1]);
  });

  test('truncated data is clamped and the partial frame dropped', () => {
    const full = encodeWav([new Float32Array([0.1, 0.2, 0.3]), new Float32Array(3)], 44100, 'f32');
    const d = decodeWav(full.subarray(0, full.length - 6));
    expect(d.channels[0]!.length).toBe(2);
    expect(d.channels[0]![1]).toBeCloseTo(0.2, 6);
  });

  test('streamed data size 0xFFFFFFFF reads the bytes present', () => {
    const bytes = riff(
      { id: 'fmt ', body: fmtBody(3, 1, 44100, 32) },
      { id: 'data', body: f32Body([0.5, 0.75]), size: 0xffffffff },
    );
    expect(Array.from(decodeWav(bytes).channels[0]!)).toEqual([0.5, 0.75]);
  });
});

describe('errors', () => {
  const err = (f: () => unknown, prefix: string) => {
    expect(f).toThrow(WavError);
    expect(f).toThrow(new RegExp(`^${prefix}`));
  };

  test('decode errors', () => {
    err(() => decodeWav(new Uint8Array(0)), 'not a RIFF/WAVE file');
    const rifx = encodeWav([new Float32Array(2)], 44100, 'f32');
    rifx[3] = 'X'.charCodeAt(0);
    err(() => decodeWav(rifx), 'not a RIFF/WAVE file');
    err(
      () => decodeWav(riff({ id: 'fmt ', body: fmtBody(3, 1, 44100, 32) })),
      'missing data chunk',
    );
    err(() => decodeWav(riff({ id: 'data', body: f32Body([0]) })), 'missing fmt chunk');
    err(() => decodeWav(riff()), 'missing fmt chunk');
    err(
      () =>
        decodeWav(
          riff(
            { id: 'fmt ', body: fmtBody(1, 1, 44100, 8) },
            { id: 'data', body: new Uint8Array(4) },
          ),
        ),
      'unsupported format 1/8',
    );
    err(
      () =>
        decodeWav(
          riff(
            { id: 'fmt ', body: fmtBody(3, 1, 44100, 64) },
            { id: 'data', body: new Uint8Array(8) },
          ),
        ),
      'unsupported format 3/64',
    );
  });

  test('encode errors', () => {
    err(() => encodeWav([], 44100, 'f32'), 'no channels');
    err(
      () => encodeWav([new Float32Array(2), new Float32Array(3)], 44100, 'f32'),
      'channel length mismatch',
    );
    err(() => encodeWav([new Float32Array(2)], 0, 'f32'), 'invalid sample rate');
    err(() => encodeWav([new Float32Array(2)], 44100.5, 'i16'), 'invalid sample rate');
    // Checked before allocating, so a fake length is enough.
    const huge = { length: 2 ** 29 } as unknown as Float32Array;
    err(() => encodeWav([huge, huge], 44100, 'f32'), 'data larger than 4 GiB');
  });
});

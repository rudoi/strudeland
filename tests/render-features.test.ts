import { describe, expect, test } from 'bun:test';
import {
  extractFeatures,
  FeaturesError,
  parseFeatures,
  serializeFeatures,
  sha256Hex,
} from '../src/lib/render/features';
import { encodeWav } from '../src/lib/render/wav';
import { bursts, mixResult, stereo } from './helpers/render-results';
import { SR, silence, sine } from './helpers/render-signals';

const hex = (s: string) => sha256Hex(new TextEncoder().encode(s));

describe('sha256Hex', () => {
  test('known vectors', () => {
    expect(hex('')).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(hex('abc')).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(hex('abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq')).toBe(
      '248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1',
    );
  });
  test('padding boundaries (55, 56, 63, 64 bytes of a)', () => {
    expect(hex('a'.repeat(55))).toBe(
      '9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318',
    );
    expect(hex('a'.repeat(56))).toBe(
      'b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a',
    );
    expect(hex('a'.repeat(64))).toBe(
      'ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb',
    );
  });
  test('one million a', () => {
    expect(sha256Hex(new Uint8Array(1_000_000).fill(97))).toBe(
      'cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0',
    );
  });
});

describe('extractFeatures', () => {
  const sig = bursts(2);
  const render = () => mixResult(stereo(sig));

  test('shape and rounding', () => {
    const f = extractFeatures(render());
    expect(f.schema).toBe(1);
    expect(f.sampleRate).toBe(SR);
    expect(f.frames).toBe(2 * SR);
    expect(f.windowSec).toBe(2);
    for (const b of Object.values(f.bandsDb)) expect(b.length).toBe(20);
    expect(f.onset.length).toBe(199);
    expect(f.pcmSha256).toMatch(/^[0-9a-f]{64}$/);
    expect(f.peak).toBe(Math.round(f.peak * 1e6) / 1e6);
    expect(f.peakDb).toBe(Math.round(f.peakDb * 100) / 100);
    expect(f.lufs).not.toBeNull();
    for (const v of f.bandsDb.mid) expect(v).toBe(Math.round(v * 100) / 100);
  });

  test('a -20 dBFS 1 kHz sine lands in mid', () => {
    const f = extractFeatures(mixResult(stereo(sine(1000, 0.1, 2))));
    const mean = (a: number[]) => a.slice(2).reduce((s, v) => s + v, 0) / (a.length - 2);
    expect(mean(f.bandsDb.mid)).toBeGreaterThan(-24);
    expect(mean(f.bandsDb.mid)).toBeLessThan(-22);
    // The bandpass is a single biquad, so the neighbouring bands see the tone through its skirts.
    expect(mean(f.bandsDb.sub)).toBeLessThan(mean(f.bandsDb.mid) - 15);
    expect(mean(f.bandsDb.low)).toBeLessThan(mean(f.bandsDb.mid) - 7);
    expect(mean(f.bandsDb.high)).toBeLessThan(mean(f.bandsDb.mid) - 7);
    expect(f.lufs!).toBeCloseTo(-20, 0);
  });

  test('silence', () => {
    const f = extractFeatures(mixResult(stereo(silence(2))));
    expect(f.lufs).toBeNull();
    expect(f.peakDb).toBe(-100);
    for (const b of Object.values(f.bandsDb)) expect(b.every((v) => v === -100)).toBe(true);
  });

  test('window excludes lookback and overlap', () => {
    const head = SR; // 1 s lookback
    const tail = SR;
    const make = (fill: number) => {
      const ch = [0, 1].map(() => {
        const x = new Float32Array(head + 2 * SR + tail);
        x.set(sig, head);
        x.fill(fill, 0, head);
        x.fill(fill, head + 2 * SR);
        return x;
      });
      return mixResult(ch, { head });
    };
    const a = extractFeatures(make(0.5));
    const b = extractFeatures(make(-0.25));
    expect(b).toEqual(a);
    expect(a.pcmSha256).toBe(extractFeatures(render()).pcmSha256);
  });

  test('hash equals the data chunk hash of the f32 wav', () => {
    const r = render();
    const wav = encodeWav(r.channels, SR, 'f32');
    const dv = new DataView(wav.buffer, wav.byteOffset, wav.byteLength);
    let off = 12;
    let data: Uint8Array | null = null;
    while (off + 8 <= wav.length) {
      const id = String.fromCharCode(...wav.subarray(off, off + 4));
      const size = dv.getUint32(off + 4, true);
      if (id === 'data') {
        data = wav.subarray(off + 8, off + 8 + size);
        break;
      }
      off += 8 + size + (size & 1);
    }
    expect(data).not.toBeNull();
    expect(extractFeatures(r).pcmSha256).toBe(sha256Hex(data!));
  });

  test('a one-sample change changes only the hash', () => {
    const a = extractFeatures(render());
    const ch = stereo(sig.slice());
    ch[0]![100] = ch[0]![100]! + 1e-7;
    const b = extractFeatures(mixResult(ch));
    expect(b.pcmSha256).not.toBe(a.pcmSha256);
    expect({ ...b, pcmSha256: '' }).toEqual({ ...a, pcmSha256: '' });
  });

  test('errors', () => {
    expect(() => extractFeatures(mixResult(stereo(sig), { mode: 'stems' }))).toThrow(RangeError);
    expect(() => extractFeatures(mixResult(stereo(sig), { ok: false }))).toThrow(RangeError);
    const nan = stereo(sig.slice());
    nan[1]![5] = NaN;
    expect(() => extractFeatures(mixResult(nan))).toThrow('non-finite samples');
    expect(() => extractFeatures(mixResult(stereo(sig), { toBar: 2 }))).toThrow(RangeError);
  });
});

describe('serialize and parse', () => {
  const f = extractFeatures(mixResult(stereo(bursts(2))));

  test('round trip is identical and stable', () => {
    const json = serializeFeatures(f);
    expect(json.endsWith('}\n')).toBe(true);
    expect(parseFeatures(json)).toEqual(f);
    expect(serializeFeatures(parseFeatures(json))).toBe(json);
    expect(JSON.parse(json)).toEqual(f);
  });

  test('null lufs round trips', () => {
    const s = extractFeatures(mixResult(stereo(silence(2))));
    expect(parseFeatures(serializeFeatures(s)).lufs).toBeNull();
  });

  const bad = (mutate: (o: Record<string, any>) => void) => {
    const o = JSON.parse(serializeFeatures(f));
    mutate(o);
    return JSON.stringify(o);
  };
  test('rejects malformed input', () => {
    expect(() => parseFeatures('nope')).toThrow(FeaturesError);
    expect(() => parseFeatures('[]')).toThrow(FeaturesError);
    expect(() => parseFeatures(bad((o) => (o.schema = 2)))).toThrow('schema');
    expect(() => parseFeatures(bad((o) => (o.pcmSha256 = 'abc')))).toThrow('pcmSha256');
    expect(() => parseFeatures(bad((o) => (o.pcmSha256 = 'A'.repeat(64))))).toThrow(FeaturesError);
    expect(() => parseFeatures(bad((o) => o.bandsDb.low.pop()))).toThrow('unequal');
    expect(() => parseFeatures(bad((o) => (o.onset = ['x'])))).toThrow('onset');
    expect(() => parseFeatures(bad((o) => delete o.bandsDb.mid))).toThrow('bandsDb.mid');
    expect(() => parseFeatures(bad((o) => (o.frames = 'x')))).toThrow('frames');
    expect(() => parseFeatures(bad((o) => (o.lufs = 'x')))).toThrow('lufs');
    expect(() => parseFeatures(bad((o) => (o.bandsDb = null)))).toThrow('bandsDb');
  });
});

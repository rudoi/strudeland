// The JS master meter and limiter on the cases of `crates/strudeland-live/src/master.rs`'s unit
// tests, at the worklet's usual rate (48 kHz: a lookahead of 72 frames).
import { describe, expect, test } from 'bun:test';
import {
  CEILING_DB,
  createMasterDsp,
  createSilenceGate,
  GAIN_SNAP,
  isSilentFrame,
  lookaheadFrames,
  LEVEL_EVERY_SEC,
  LOOKAHEAD_SEC,
  type MasterDsp,
  RELEASE_SEC,
} from '../src/lib/audio/master-dsp';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

describe('the constants match master.rs', () => {
  const rs = readFileSync(join(import.meta.dir, '../crates/strudeland-live/src/master.rs'), 'utf8');
  const player = readFileSync(
    join(import.meta.dir, '../crates/strudeland-live/src/player.rs'),
    'utf8',
  );
  const rsConst = (name: string): number => {
    const m = new RegExp(`const ${name}: f64 = ([-0-9.e]+);`).exec(rs);
    if (!m) throw new Error(`${name} not found in master.rs`);
    return Number(m[1]);
  };
  test.each([
    ['CEILING_DB', CEILING_DB],
    ['LOOKAHEAD_SEC', LOOKAHEAD_SEC],
    ['RELEASE_SEC', RELEASE_SEC],
    ['GAIN_SNAP', GAIN_SNAP],
  ] as const)('%s', (name, value) => {
    expect(value as number).toBe(rsConst(name));
  });
  test("LEVEL_EVERY_SEC is the player's SCOPE_EVERY", () => {
    const m = /SCOPE_EVERY: Duration = Duration::from_millis\((\d+)\)/.exec(player);
    expect(m).not.toBeNull();
    expect(LEVEL_EVERY_SEC * 1000).toBeCloseTo(Number(m![1]), 9);
  });
});

const RATE = 48000;
const LA = 72;
const BLOCK = 128;
const C = Math.pow(10, CEILING_DB / 20);
const f32 = Math.fround;

/** Planar stereo of `frames` frames from `f(i) = [l, r]`. */
function stereo(frames: number, f: (i: number) => [number, number]): [Float32Array, Float32Array] {
  const l = new Float32Array(frames);
  const r = new Float32Array(frames);
  for (let i = 0; i < frames; i++) [l[i], r[i]] = f(i);
  return [l, r];
}

function sine(frames: number, amp: number) {
  return stereo(frames, (i) => {
    const v = f32(amp * Math.sin((2 * Math.PI * 440 * i) / RATE));
    return [v, -0.5 * v];
  });
}

/** Runs `input` through `dsp` in 128-frame blocks, `extra` blocks of silence after it. */
function run(dsp: MasterDsp, input: Float32Array[], extra = 0): Float32Array[] {
  const len = input[0]!.length;
  const blocks = Math.ceil(len / BLOCK) + extra;
  const out = input.map(() => new Float32Array(blocks * BLOCK));
  for (let k = 0; k < blocks; k++) {
    const block = input.map((ch) => {
      const b = new Float32Array(BLOCK);
      b.set(ch.subarray(Math.min(k * BLOCK, len), Math.min((k + 1) * BLOCK, len)));
      return b;
    });
    dsp.process(block, block);
    block.forEach((b, ch) => out[ch]!.set(b, k * BLOCK));
  }
  return out;
}

const peakOf = (chs: Float32Array[]) =>
  chs.reduce((p, ch) => ch.reduce((q, s) => Math.max(q, Math.abs(s)), p), 0);

describe('constants', () => {
  test('the lookahead per rate and the ceiling', () => {
    expect(lookaheadFrames(RATE)).toBe(LA);
    expect(lookaheadFrames(44100)).toBe(66);
    expect(lookaheadFrames(RATE)).toBe(Math.round(LOOKAHEAD_SEC * RATE));
    const d = createMasterDsp(RATE);
    expect(d.lookahead).toBe(LA);
    expect(Math.abs(d.ceiling - 0.891251)).toBeLessThan(1e-6);
  });

  test('a bad rate or channel count throws', () => {
    expect(() => createMasterDsp(NaN)).toThrow();
    expect(() => createMasterDsp(100)).toThrow();
    expect(() => createMasterDsp(RATE, 0)).toThrow();
  });
});

describe('level frames', () => {
  test('peak, rms and overs of a block', () => {
    const d = createMasterDsp(RATE);
    // left alternates 0.5 and -1.0; right is a constant 0.25
    const x = stereo(BLOCK, (i) => [i % 2 === 0 ? 0.5 : -1.0, 0.25]);
    d.process(
      x,
      x.map(() => new Float32Array(BLOCK)),
    );
    const f = d.takeFrame()!;
    expect(f.peak).toEqual([1, 0.25]);
    expect(f.rms).toEqual([f32(Math.sqrt((0.25 + 1) / 2)), 0.25]);
    expect(f.overs).toEqual([BLOCK / 2, 0]);
    expect(f.limitDb).toBe(0);
  });

  test('a frame covers every block since the last', () => {
    const d = createMasterDsp(RATE);
    const a = stereo(BLOCK, (i) => [i === 0 ? 0.5 : 0, 0]);
    d.process(a, a);
    const b = stereo(BLOCK, (i) => [0, i === 0 ? 2 : 0]);
    d.process(b, b);
    expect(d.pendingFrames()).toBe(2 * BLOCK);
    const f = d.takeFrame()!;
    expect(f.peak).toEqual([0.5, 2]);
    expect(f.overs).toEqual([0, 1]);
    const n = 2 * BLOCK;
    expect(f.rms).toEqual([f32(Math.sqrt(0.25 / n)), f32(Math.sqrt(4 / n))]);
  });

  test('identical channels give identical levels; one channel fills both', () => {
    const d = createMasterDsp(RATE);
    const x = stereo(BLOCK, (i) => {
      const v = f32(Math.sin(i / 40));
      return [v, v];
    });
    run(d, x);
    const f = d.takeFrame()!;
    expect(f.peak[0]).toBe(f.peak[1]);
    expect(f.rms[0]).toBe(f.rms[1]);
    expect(f.overs[0]).toBe(f.overs[1]);

    const mono = createMasterDsp(RATE, 1);
    const m = [x[0]];
    mono.process(m, [new Float32Array(BLOCK)]);
    expect(mono.takeFrame()).toEqual(f);
  });

  test('silence is a frame of zeros', () => {
    const d = createMasterDsp(RATE);
    const x = stereo(BLOCK, () => [0, 0]);
    d.process(x, x);
    expect(d.takeFrame()).toEqual({ peak: [0, 0], rms: [0, 0], overs: [0, 0], limitDb: 0 });
  });

  test('a missing input (nothing connected) measures as silence', () => {
    const d = createMasterDsp(RATE);
    const out = [new Float32Array(BLOCK).fill(9), new Float32Array(BLOCK).fill(9)];
    d.process([], out);
    expect(d.takeFrame()).toEqual({ peak: [0, 0], rms: [0, 0], overs: [0, 0], limitDb: 0 });
    expect(out.every((ch) => ch.every((s) => s === 0))).toBe(true);
  });

  test('non-finite samples are overs of peak 1, every field finite', () => {
    for (const on of [false, true]) {
      const d = createMasterDsp(RATE);
      d.setEnabled(on);
      const x = stereo(BLOCK, () => [0, 0]);
      x[0][0] = NaN;
      x[1][0] = Infinity;
      x[1][1] = -Infinity;
      d.process(x, x);
      const f = d.takeFrame()!;
      expect(f.peak).toEqual([1, 1]);
      expect(f.overs).toEqual([1, 2]);
      expect(f.rms).toEqual([f32(Math.sqrt(1 / BLOCK)), f32(Math.sqrt(2 / BLOCK))]);
      expect(Number.isFinite(f.limitDb)).toBe(true);
      expect(JSON.stringify(f)).not.toContain('null');
    }
  });

  test('takeFrame drains and is null when nothing was processed', () => {
    const d = createMasterDsp(RATE);
    expect(d.takeFrame()).toBeNull();
    const x = stereo(BLOCK, () => [0.5, 0.5]);
    d.process(x, x);
    expect(d.takeFrame()).not.toBeNull();
    expect(d.pendingFrames()).toBe(0);
    expect(d.takeFrame()).toBeNull();
  });

  test('levels are measured before the limiter', () => {
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    run(d, sine(4 * BLOCK, 2));
    const f = d.takeFrame()!;
    expect(f.peak[0]).toBeGreaterThan(1.99);
    expect(f.overs[0]).toBeGreaterThan(0);
    expect(f.limitDb).toBeLessThan(-6);
  });
});

describe('limiter', () => {
  test('below the ceiling the output is the input delayed, bit for bit', () => {
    for (const on of [false, true]) {
      const d = createMasterDsp(RATE);
      d.setEnabled(on);
      const x = sine(10 * BLOCK, 0.8);
      const y = run(d, x, 1);
      for (let ch = 0; ch < 2; ch++) {
        expect(y[ch]!.subarray(0, LA).every((s) => s === 0)).toBe(true);
        expect(y[ch]!.subarray(LA, LA + x[ch]!.length)).toEqual(x[ch]!);
      }
      expect(d.takeFrame()!.limitDb).toBe(0);
    }
  });

  test('disabled passes a NaN through', () => {
    const d = createMasterDsp(RATE);
    const x = stereo(BLOCK, () => [0, 0]);
    x[0][5] = NaN;
    const y = run(d, x, 1);
    expect(Number.isNaN(y[0]![5 + LA])).toBe(true);
  });

  test('enabled replaces a NaN with zero and keeps the gain finite', () => {
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    const x = sine(4 * BLOCK, 0.5);
    x[0][5] = NaN;
    const y = run(d, x, 1);
    expect(y[0]![5 + LA]).toBe(0);
    expect(y.every((ch) => ch.every(Number.isFinite))).toBe(true);
    // the rest passes unchanged: the NaN asked for no gain reduction
    expect(y[0]![6 + LA]).toBe(x[0][6]!);
  });

  test('enabled output never exceeds the ceiling (a loud sine, impulses)', () => {
    const loud = sine(40 * BLOCK, 4);
    const impulse = stereo(40 * BLOCK, () => [0, 0]);
    impulse[0][1000] = 8;
    impulse[1][1001] = -3;
    impulse[0][1500] = 1.2;
    for (const x of [loud, impulse]) {
      const d = createMasterDsp(RATE);
      d.setEnabled(true);
      const peak = peakOf(run(d, x, 2));
      expect(peak).toBeLessThanOrEqual(f32(C));
      expect(peak).toBeGreaterThan(0.5 * C);
    }
  });

  test('the linked gain covers every channel of a multichannel output', () => {
    const d = createMasterDsp(RATE, 4);
    d.setEnabled(true);
    const x = [0, 1, 2, 3].map(() => new Float32Array(10 * BLOCK));
    x[3]![500] = 5; // only channel 3 is loud
    x[0]![500] = 0.5;
    const y = run(d, x, 1);
    expect(peakOf(y)).toBeLessThanOrEqual(f32(C));
    expect(y[0]![500 + LA]).toBeLessThan(0.5 * (C / 5) + 1e-6);
  });

  test('the gain stays under every frame’s requirement', () => {
    const x = stereo(30 * BLOCK, (i) => {
      const v = f32(0.6 * Math.sin(i / 7) + (i % 397 === 0 ? 3 : 0));
      return [v, f32(0.2 * v)];
    });
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    const n = x[0].length;
    const peaks = Array.from({ length: n }, (_, i) =>
      Math.max(Math.abs(x[0][i]!), Math.abs(x[1][i]!)),
    );
    const gains = peaks.map((p) => d.gain(p));
    for (let k = 0; k < n - LA; k++) {
      const req = peaks[k]! <= C ? 1 : C / peaks[k]!;
      if (gains[k + LA]! > req) throw new Error(`frame ${k}: ${gains[k + LA]} > ${req}`);
    }
  });

  test('the sliding minimum covers the last LA + 1 frames', () => {
    const d = createMasterDsp(RATE);
    const reqs = Array.from({ length: 500 }, (_, i) => 0.5 + (0.5 * ((i * 37) % 101)) / 101);
    reqs.forEach((r, n) => {
      const got = d.slidingMin(r);
      const want = Math.min(...reqs.slice(Math.max(0, n - LA), n + 1));
      if (got !== want) throw new Error(`frame ${n}: ${got} != ${want}`);
    });
  });

  test('the release is monotone after a peak and returns to exactly 1', () => {
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    const gains = [d.gain(4)];
    for (let i = 0; i < RATE; i++) gains.push(d.gain(0.1));
    let lowest = 0;
    gains.forEach((g, i) => {
      if (g < gains[lowest]!) lowest = i;
    });
    expect(lowest).toBe(LA); // the ramp reaches its floor after the lookahead
    expect(gains[lowest]!).toBeLessThan(0.25);
    for (let i = LA + 1; i < gains.length; i++) {
      if (gains[i]! < gains[i - 1]!) throw new Error(`${gains[i - 1]} then ${gains[i]}`);
    }
    expect(gains[gains.length - 1]).toBe(1);
    // the attack is a ramp, not a step
    for (let i = 1; i <= LA; i++) expect(gains[i - 1]! - gains[i]!).toBeLessThan(0.02);
  });

  test('the gain snaps to 1, except at the over’s own output frame', () => {
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    const tiny = f32(C * (1 + 5e-7));
    expect(tiny).toBeGreaterThan(C);
    const req = C / tiny;
    expect(1 - req).toBeLessThan(GAIN_SNAP);
    for (let n = 0; n < 300; n++) {
      const g = d.gain(n === 0 ? tiny : 0);
      expect(g).toBe(n === LA ? req : 1);
    }
  });

  test('toggling does not move audio', () => {
    const x = stereo(12 * BLOCK, (i) => (i % 300 === 5 ? [0.5, -0.5] : [0, 0]));
    const want = run(createMasterDsp(RATE), x, 1);
    const d = createMasterDsp(RATE);
    const got = x.map(() => new Float32Array(13 * BLOCK));
    for (let k = 0; k < 13; k++) {
      d.setEnabled(k % 2 === 1);
      const block = x.map((ch) => {
        const b = new Float32Array(BLOCK);
        b.set(ch.subarray(Math.min(k * BLOCK, ch.length), Math.min((k + 1) * BLOCK, ch.length)));
        return b;
      });
      d.process(block, block);
      block.forEach((b, ch) => got[ch]!.set(b, k * BLOCK));
    }
    expect(got).toEqual(want as Float32Array<ArrayBuffer>[]);
  });

  test('enabling mid-stream limits from the toggle on', () => {
    // a loud steady signal: the frames already in the delay line went in unlimited, yet every
    // sample out from the toggle on is under the ceiling
    const x = sine(20 * BLOCK, 3);
    for (const at of [1, 5, 9]) {
      const d = createMasterDsp(RATE);
      let peak = 0;
      for (let k = 0; k < 20; k++) {
        if (k === at) d.setEnabled(true);
        const block = x.map((ch) => ch.slice(k * BLOCK, (k + 1) * BLOCK));
        d.process(block, block);
        if (k >= at) peak = Math.max(peak, peakOf(block));
        else expect(peakOf(block)).toBeGreaterThan(C); // unlimited before the toggle
      }
      expect(peak).toBeLessThanOrEqual(f32(C));
      expect(peak).toBeGreaterThan(0.5 * C);
    }
  });

  test('disabled output is the delayed input even over the ceiling', () => {
    const d = createMasterDsp(RATE);
    const x = sine(10 * BLOCK, 3);
    const y = run(d, x, 1);
    for (let ch = 0; ch < 2; ch++) {
      expect(y[ch]!.subarray(LA, LA + x[ch]!.length)).toEqual(x[ch]!);
    }
    expect(d.takeFrame()!.limitDb).toBe(0); // off reports no limiting
  });

  test('resetStream clears the delay line and keeps the rest', () => {
    const d = createMasterDsp(RATE);
    d.setEnabled(true);
    const a = stereo(BLOCK, () => [0.5, 0.5]);
    d.process(a, a);
    d.resetStream();
    const b = stereo(BLOCK, () => [0, 0]);
    d.process(b, b);
    expect(b.every((ch) => ch.every((s) => s === 0))).toBe(true);
    expect(d.enabled()).toBe(true);
    expect(d.takeFrame()!.peak).toEqual([0.5, 0.5]); // the accumulator survives
  });
});

describe('the silence gate', () => {
  const silent = { peak: [0, 0], rms: [0, 0], overs: [0, 0], limitDb: 0 } as const;
  const sound = { peak: [0.5, 0.2], rms: [0.3, 0.1], overs: [0, 0], limitDb: 0 } as const;
  const frame = (f: typeof silent | typeof sound) => ({
    peak: [...f.peak] as [number, number],
    rms: [...f.rms] as [number, number],
    overs: [...f.overs] as [number, number],
    limitDb: f.limitDb,
  });

  test('a frame is silent only with no peak, no overs and no limiting', () => {
    expect(isSilentFrame(frame(silent))).toBe(true);
    expect(isSilentFrame(frame(sound))).toBe(false);
    expect(isSilentFrame({ ...frame(silent), peak: [0, 1e-9] })).toBe(false);
    expect(isSilentFrame({ ...frame(silent), overs: [0, 1] })).toBe(false);
    expect(isSilentFrame({ ...frame(silent), limitDb: -0.5 })).toBe(false);
  });

  test('the first silent frame after sound is posted, the rest are skipped until sound', () => {
    const gate = createSilenceGate();
    const seq = [silent, silent, sound, sound, silent, silent, silent, sound, silent, silent];
    expect(seq.map((f) => gate(frame(f)))).toEqual([
      true, // the first frame is always posted
      false,
      true,
      true,
      true, // the meter falls to silence
      false,
      false,
      true,
      true,
      false,
    ]);
  });

  test('a dsp idling in silence posts one frame through the gate', () => {
    const d = createMasterDsp(RATE);
    const gate = createSilenceGate();
    let posts = 0;
    const loud = sine(BLOCK, 0.5);
    for (let k = 0; k < 40; k++) {
      const block = k < 4 ? loud.map((ch) => ch.slice()) : stereo(BLOCK, () => [0, 0]);
      d.process(block, block);
      const f = d.takeFrame();
      if (f && gate(f)) posts++;
    }
    // four loud frames (levels are measured at the input), the first silent one, then nothing
    expect(posts).toBe(5);
  });
});

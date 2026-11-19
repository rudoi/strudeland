/**
 * The corpus one-shots (`tests/render/corpus/samples/{kick,hat,tone}.wav`): deterministic synthesis,
 * mono, 22050 Hz (the offline context resamples them), 16-bit PCM. `makeSamples()` has no side
 * effects; running this file writes the three files. A test compares the committed bytes.
 *
 *   bun scripts/render/make-samples.ts
 */

import { mulberry32 } from 'superdough/rng.mjs';
import { encodeWav } from '../../src/lib/render/wav';

export const SAMPLE_RATE = 22050;

const framesOf = (seconds: number) => Math.round(seconds * SAMPLE_RATE);

/** Sine with instantaneous frequency 45 + 105·exp(-t/0.03) Hz, amplitude 0.9·exp(-t/0.09), 0.2 s. */
export function kick(): Float32Array {
  const out = new Float32Array(framesOf(0.2));
  let phase = 0;
  for (let i = 0; i < out.length; i++) {
    const t = i / SAMPLE_RATE;
    out[i] = 0.9 * Math.exp(-t / 0.09) * Math.sin(phase);
    phase += (2 * Math.PI * (45 + 105 * Math.exp(-t / 0.03))) / SAMPLE_RATE;
  }
  return out;
}

/** mulberry32(0x5eed) white noise, high-passed as x[n] - 0.7·x[n-1], amplitude 0.6·exp(-t/0.015),
 *  0.1 s. */
export function hat(): Float32Array {
  const out = new Float32Array(framesOf(0.1));
  const rnd: () => number = mulberry32(0x5eed);
  let prev = 0;
  for (let i = 0; i < out.length; i++) {
    const t = i / SAMPLE_RATE;
    const x = rnd() * 2 - 1;
    out[i] = 0.6 * Math.exp(-t / 0.015) * (x - 0.7 * prev);
    prev = x;
  }
  return out;
}

/** 220 Hz with the 2nd and 3rd partials, 5 ms linear attack, exp(-t/0.12) decay, 0.4 s. */
export function tone(): Float32Array {
  const out = new Float32Array(framesOf(0.4));
  for (let i = 0; i < out.length; i++) {
    const t = i / SAMPLE_RATE;
    const w = 2 * Math.PI * t;
    const partials = (Math.sin(w * 220) + 0.4 * Math.sin(w * 440) + 0.2 * Math.sin(w * 660)) / 1.6;
    out[i] = 0.5 * partials * Math.min(1, t / 0.005) * Math.exp(-t / 0.12);
  }
  return out;
}

/** File name to WAV bytes, in the order of `corpus.json` `samples`. */
export function makeSamples(): Record<string, Uint8Array> {
  return {
    'kick.wav': encodeWav([kick()], SAMPLE_RATE, 'i16'),
    'hat.wav': encodeWav([hat()], SAMPLE_RATE, 'i16'),
    'tone.wav': encodeWav([tone()], SAMPLE_RATE, 'i16'),
  };
}

if (import.meta.main) {
  const { resolve } = await import('node:path');
  const dir = resolve(import.meta.dir, '../../tests/render/corpus/samples');
  for (const [file, bytes] of Object.entries(makeSamples())) {
    await Bun.write(resolve(dir, file), bytes);
    console.log(`${file}  ${bytes.length} bytes`);
  }
}

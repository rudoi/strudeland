/** Builds synthetic mix RenderResults for the features and parity tests. */

import type { RenderRequest, RenderResult } from '../../src/lib/render/contract';
import { noiseSeeded, SR } from './render-signals';

export function mixResult(
  channels: Float32Array[],
  opts: {
    cps?: number;
    fromBar?: number;
    toBar?: number;
    head?: number;
    mode?: 'mix' | 'stems';
    ok?: boolean;
  } = {},
): RenderResult {
  const cps = opts.cps ?? 0.5;
  const head = opts.head ?? 0;
  const request: RenderRequest = {
    code: '',
    layout: 'linear',
    fromBar: opts.fromBar ?? 0,
    toBar: opts.toBar ?? 1,
    overlapBars: 0,
    lookbackSec: 'auto',
    mode: opts.mode ?? 'mix',
    sampleRate: 44100,
  };
  const frames = channels[0]!.length;
  return {
    channels,
    sampleRate: SR,
    frames,
    windowStartFrame: head,
    lookbackSecUsed: head / SR,
    bufferStartSec: 0,
    cps,
    events: [],
    stemPairs: 1,
    errors: [],
    ok: opts.ok ?? true,
    timing: { evalMs: 0, renderMs: 0, totalMs: 0 },
    request,
  };
}

/** Seeded noise bursts with an exponential decay every `everySec`, broadband. */
export function bursts(sec: number, everySec = 0.5, amp = 0.3): Float32Array {
  const n = Math.round(sec * SR);
  const noise = noiseSeeded(7, n, amp);
  const every = Math.round(everySec * SR);
  for (let i = 0; i < n; i++) noise[i] = noise[i]! * Math.exp(-(i % every) / (0.06 * SR));
  return noise;
}

export function stereo(x: Float32Array): Float32Array[] {
  return [x, x.slice()];
}

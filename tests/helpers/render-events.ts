/** A RenderEvent with every field filled; override what a test cares about. */

import type { RenderEvent } from '../../src/lib/render/schedule';

export function ev(o: Partial<RenderEvent> = {}): RenderEvent {
  return {
    tSec: 0,
    durSec: 1,
    bar: 0,
    s: 'sine',
    orbit: 1,
    freq: null,
    note: null,
    gain: 0.8,
    attack: 0,
    decay: 0,
    sustain: null,
    release: 0,
    penv: 0,
    cutoff: null,
    room: null,
    delay: null,
    stochastic: false,
    fxRelease: 0,
    compressor: false,
    busSends: [],
    busReads: [],
    ...o,
  };
}

export const room = (size: number): RenderEvent['room'] => ({
  amount: 0.5,
  size,
  fade: 0.1,
  lp: 15000,
  dim: 1000,
});

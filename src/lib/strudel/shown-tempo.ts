import type { Tempo } from './structure';

/**
 * The tempo a readout keeps after sampling `sampled`: `current` itself when cps, bpm and the
 * meter object are unchanged (a bar's meter is the structure's own object, so it is compared by
 * identity), else `sampled`. Keeping the same object lets a frame-rate sample skip re-rendering.
 */
export const nextShownTempo = (current: Tempo, sampled: Tempo): Tempo =>
  sampled.cps === current.cps && sampled.bpm === current.bpm && sampled.meter === current.meter
    ? current
    : sampled;

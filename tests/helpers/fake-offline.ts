/**
 * Real Strudel haps and a recording fake of `OfflineDeps` for `renderOffline`, shared by the
 * planning tests (`render-plan`, `render-engine-job`). The fake context pauses where it is told,
 * records every suspend, superdough call and orbit creation, and renders an empty buffer.
 */

import * as core from '@strudel/core';
import * as mini from '@strudel/mini';
import type { OfflineDeps, RenderedBuffer } from '../../src/lib/audio/offline';
import type { RenderRequest } from '../../src/lib/render/contract';

await core.evalScope(core, mini);

const { Fraction, Hap, TimeSpan } = core as any;

/** A real Hap with whole [begin, end) and, unless given, part = whole. */
export function hap(
  begin: number,
  end: number,
  value: unknown,
  opts: { partBegin?: number; context?: object; noWhole?: boolean } = {},
): any {
  const whole = opts.noWhole ? undefined : new TimeSpan(Fraction(begin), Fraction(end));
  const part = new TimeSpan(Fraction(opts.partBegin ?? begin), Fraction(end));
  return new Hap(whole, part, value, opts.context ?? {});
}

export const req = (o: Partial<RenderRequest> = {}): RenderRequest => ({
  code: '',
  layout: 'linear',
  fromBar: 0,
  toBar: 2,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: 44100,
  ...o,
});

export interface SuperdoughCall {
  value: any;
  t: number;
  dur: number;
  cps: number;
  cycle: number;
}

/** A fake that renders instantly: every suspend resolves when the render reaches it. */
export function recordingDeps() {
  const suspends: number[] = [];
  const calls: SuperdoughCall[] = [];
  const orbits: [unknown, number[]][] = [];
  const contexts: { channels: number; frames: number }[] = [];
  let seed: number | null = null;
  let ctx: any;

  const deps: OfflineDeps = {
    createContext(channels, frames, sampleRate) {
      contexts.push({ channels, frames });
      const pauses = new Map<number, () => void>();
      let finish!: (b: RenderedBuffer) => void;
      const buffer: RenderedBuffer = {
        numberOfChannels: channels,
        length: frames,
        getChannelData: () => new Float32Array(frames),
      };
      const advance = () => {
        const next = [...pauses.keys()].filter((t) => t >= ctx.currentTime).sort((a, b) => a - b);
        if (next.length > 0) {
          const t = next[0]!;
          const resolve = pauses.get(t)!;
          pauses.delete(t);
          ctx.currentTime = t;
          resolve();
        } else {
          ctx.currentTime = frames / sampleRate;
          finish(buffer);
        }
      };
      ctx = {
        currentTime: 0,
        suspend(t: number) {
          suspends.push(t);
          return new Promise<void>((r) => pauses.set(t, r));
        },
        startRendering() {
          const p = new Promise<RenderedBuffer>((r) => (finish = r));
          setTimeout(advance, 0);
          return p;
        },
        resume() {
          setTimeout(advance, 0);
          return Promise.resolve();
        },
      };
      return ctx;
    },
    setContext: () => {},
    createController: () => ({ getOrbit: (n, ch) => void orbits.push([n, ch]) }),
    setController: () => {},
    initAudio: async () => {},
    setSeed: (s) => void (seed = s),
    setPooling: () => true,
    installPatches: () => ({ settle: async () => 0, uninstall: () => {} }),
    preloadSamples: async () => {},
    sampleUrl: () => null,
    superdough: (value, t, dur, cps, cycle) => {
      calls.push({ value, t, dur, cps, cycle });
      return undefined;
    },
    now: () => 0,
  };
  return { deps, suspends, calls, orbits, contexts, seed: () => seed };
}

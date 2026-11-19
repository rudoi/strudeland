import { getAudioContext, initAudio, webaudioOutput } from '@strudel/webaudio';
import { getSuperdoughAudioController } from 'superdough';
import { setAudioSeed } from 'superdough/rng.mjs';
import type { AudioBackend, AudioGraph } from './backend';
import { createMasterTap, type MasterGain, type MasterTap, type MasterTapDeps } from './master-tap';
import { createScopeTap, type TapNode } from './scope-tap';

interface Ctx {
  state: string;
  currentTime: number;
  resume(): Promise<void>;
}
/** superdough's audio controller, as far as this backend and the taps read it: the master output's
 *  gain, a node on the live `AudioContext` (null mid-`reset()`). */
type SuperdoughController = {
  output: {
    destinationGain: (TapNode & MasterGain & AudioNode & { readonly context: AudioContext }) | null;
  };
} | null;

interface Deps {
  getContext: () => Ctx;
  init: () => Promise<void>;
  output: AudioBackend['output'];
  setSeed: (seed: number) => void;
  /** superdough's audio controller (for the scope tap); it creates an AudioContext when none exists. */
  getController?: () => SuperdoughController;
  /** Builds the master tap (meter and limiter worklet); default `createMasterTap`. */
  createMasterTap?: (deps: MasterTapDeps) => MasterTap;
}

const defaultDeps: Deps = {
  getContext: getAudioContext,
  init: initAudio,
  output: webaudioOutput,
  setSeed: setAudioSeed,
  getController: getSuperdoughAudioController,
};

export function createSuperdoughBackend(deps: Deps = defaultDeps): AudioBackend {
  // Not created until the first resume()/init(): getAudioContext() creates the
  // AudioContext lazily, and one created outside a user gesture starts suspended.
  let ctx: Ctx | undefined;
  let initPromise: Promise<void> | undefined;
  const isRunning = () => ctx !== undefined && ctx.state === 'running';
  const getController = deps.getController ?? (() => null);
  const tap = createScopeTap({ isRunning, getController });
  const master = (deps.createMasterTap ?? createMasterTap)({ isRunning, getController });
  return {
    id: 'superdough',
    async init() {
      initPromise ??= deps
        .init()
        .then(() => {
          ctx ??= deps.getContext();
        })
        .catch((e) => {
          initPromise = undefined;
          throw e;
        });
      await initPromise;
      // The runtime awaits init() after every resume and before playing: the master worklet is
      // attached (or has failed) before the first hap, never rewired under a playing take.
      await master.ready();
    },
    async resume() {
      ctx ??= deps.getContext();
      await ctx.resume();
      master.ensure();
      return ctx.state === 'running';
    },
    isRunning,
    getTime: () => ctx?.currentTime ?? 0,
    setSeed: (seed) => deps.setSeed(seed),
    output: (...args) => {
      // A reset() made a new destinationGain: repair the edge before this hap's voice connects.
      master.ensure();
      return deps.output(...args);
    },
    readScope: (buf) => {
      master.ensure();
      return tap.read(buf);
    },
    onLevels: (cb) => master.onLevels(cb),
    setLimiter: (on) => master.setLimiter(on),
    limiterAvailable: () => master.available(),
    masterLatencySec: () => master.latencySec(),
    graph(): AudioGraph | null {
      // getController() would create an AudioContext outside a gesture: never before running.
      if (!isRunning()) return null;
      try {
        const node = deps.getController?.()?.output.destinationGain ?? null;
        if (!node) return null; // mid-reset()
        return { context: node.context, destination: node };
      } catch {
        return null;
      }
    },
  };
}

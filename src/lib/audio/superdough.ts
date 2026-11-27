import { getAudioContext, initAudio, webaudioOutput } from '@strudel/webaudio';
import { getSuperdoughAudioController, superdough } from 'superdough';
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
  output(hap: any, t, cps, timing) {
    if (!timing) return webaudioOutput(hap, t, cps);
    hap.ensureObjectValue();
    // Superdough writes duration and its seed into the value. The logical hap can
    // also be routed to a custom callback that still needs its authored duration.
    return superdough(
      { ...hap.value },
      t,
      timing.durationSec,
      cps,
      Number(hap.whole.begin),
      timing.phaseSec,
    );
  },
  setSeed: setAudioSeed,
  getController: getSuperdoughAudioController,
};

/**
 * The superdough backend, with what only it has: the Web Audio graph and the master stage's
 * latency. The graph feeds previews, the metronome, the output clock and memo playback; the
 * master latency feeds the take's output delay. Previews play here even while the Rust engine
 * plays the song.
 */
export interface SuperdoughBackend extends AudioBackend {
  /**
   * The live graph, or null unless audio runs (it never creates the context) or while the output
   * is being rebuilt. Never throws. Callers call it each time they need a node and never keep the
   * result: the destination and the context can be replaced.
   */
  graph(): AudioGraph | null;
  /** The delay the master stage adds to what is heard, in seconds (0 when none). */
  masterLatencySec(): number;
}

export function createSuperdoughBackend(deps: Deps = defaultDeps): SuperdoughBackend {
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

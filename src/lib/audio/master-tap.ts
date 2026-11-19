/**
 * Inserts the `master-meter` worklet (`master-worklet.ts`) between superdough's master
 * `destinationGain` and the context's destination, so everything heard on superdough is metered
 * and, when the limiter is on, limited. Modelled on `scope-tap.ts`: it rewires from our side, so no
 * vendored file is patched. Imported only by `superdough.ts` (never by offline renders).
 *
 * The module is added on the first `ensure()` with a running context; the rewire happens only
 * after `addModule` resolves. `ready()` (awaited by the backend's `init()`, which the runtime
 * awaits after every resume and before the first play) waits for that, so audio does not start on
 * the direct edge and get rewired mid-playback. Until then, and if loading or the node fails (or
 * the destination has more channels than an AudioWorkletNode output can carry), the direct
 * `destinationGain -> destination` edge stays. On every new `destinationGain` (a `reset()`
 * creates one already connected to the destination) exactly that edge is removed and the gain is
 * connected to the worklet; the worklet's own link to the destination is made once per context.
 */
import type { LevelFrame } from '../levels/levels';
import { lookaheadFrames, MASTER_PROCESSOR } from './master-dsp';
import masterWorkletUrl from './master-worklet?audioworklet';

export interface MasterContext {
  audioWorklet: { addModule(url: string): Promise<void> };
  destination: { channelCount: number };
  sampleRate: number;
}
/** The worklet node, as far as the tap uses it. */
export interface MasterNode {
  connect(dest: unknown): unknown;
  port: { postMessage(msg: unknown): void; onmessage: ((e: { data: unknown }) => void) | null };
}
export interface MasterGain {
  connect(dest: unknown): unknown;
  disconnect(dest: unknown): unknown;
  readonly context: MasterContext;
}
/** The part of superdough's audio controller the tap reads. */
export type MasterController = { output: { destinationGain: MasterGain | null } } | null;

export interface MasterNodeOptions {
  channelCount: number;
  channelCountMode: 'explicit';
  channelInterpretation: 'discrete';
  numberOfInputs: 1;
  numberOfOutputs: 1;
  outputChannelCount: [number];
}

export interface MasterTapDeps {
  /** Checked first: while false the tap never touches superdough. */
  isRunning: () => boolean;
  /** superdough's `getSuperdoughAudioController`; it creates an AudioContext when none exists. */
  getController: () => MasterController;
  /** The worklet module's URL (default: the bundled `master-worklet.ts`). */
  moduleUrl?: string;
  /** Creates the worklet node (default: `new AudioWorkletNode`). */
  createNode?: (context: MasterContext, name: string, options: MasterNodeOptions) => MasterNode;
}

/** The most channels an AudioWorkletNode output may have (Web Audio's limit). */
export const MAX_WORKLET_CHANNELS = 32;
/** How long `ready()` waits for a module load before giving up (the direct edge stays meanwhile). */
export const READY_TIMEOUT_MS = 2000;

export interface MasterTap {
  /**
   * Repairs the wiring when superdough's output changed; the only rewiring entry point. Does
   * nothing until audio runs, never throws, and is a reference compare when nothing changed.
   */
  ensure(): void;
  /**
   * `ensure()`, then waits until the current context's worklet is attached or has failed (at most
   * `timeoutMs`). Never rejects; resolves at once before audio runs or when nothing is loading.
   */
  ready(timeoutMs?: number): Promise<void>;
  /** Subscribes to level frames; returns the remover. */
  onLevels(cb: (frame: LevelFrame) => void): () => void;
  /** The limiter preference; posted to the current node and every new one. */
  setLimiter(on: boolean): void;
  /** False once the module or the node failed on the current context (no levels, no limiter). */
  available(): boolean;
  /** The delay the worklet adds (its lookahead), 0 while no worklet is attached. */
  latencySec(): number;
}

const defaultCreateNode: NonNullable<MasterTapDeps['createNode']> = (context, name, options) =>
  new AudioWorkletNode(context as unknown as BaseAudioContext, name, options) as MasterNode;

const isFrame = (d: unknown): d is LevelFrame => {
  const f = d as LevelFrame | null;
  return (
    typeof f === 'object' &&
    f !== null &&
    Array.isArray(f.peak) &&
    Array.isArray(f.rms) &&
    Array.isArray(f.overs) &&
    typeof f.limitDb === 'number'
  );
};

interface Attachment {
  context: MasterContext;
  status: 'loading' | 'ready' | 'failed';
  node: MasterNode | null;
  /** Settles (never rejects) once the load is over, attached or failed. */
  settled: Promise<void>;
}

export function createMasterTap(deps: MasterTapDeps): MasterTap {
  const url = deps.moduleUrl ?? masterWorkletUrl;
  const createNode = deps.createNode ?? defaultCreateNode;
  const listeners = new Set<(frame: LevelFrame) => void>();
  let limiter = false;
  /** The latest context's worklet state. */
  let current: Attachment | null = null;
  /** The gain currently feeding `current.node`. */
  let source: MasterGain | null = null;
  let warnedChannels = false;

  const emit = (e: { data: unknown }) => {
    if (!isFrame(e.data)) return;
    const frame = e.data;
    for (const cb of listeners) {
      try {
        cb(frame);
      } catch (err) {
        console.error(err);
      }
    }
  };

  const load = (a: Attachment): Promise<void> => {
    const channels = a.context.destination?.channelCount ?? 2;
    if (channels > MAX_WORKLET_CHANNELS) {
      // The node's constructor would throw: keep the direct edge rather than drop channels.
      a.status = 'failed';
      if (!warnedChannels) {
        warnedChannels = true;
        console.warn(
          `the master meter is off: the output has ${channels} channels, more than an audio worklet can carry (${MAX_WORKLET_CHANNELS})`,
        );
      }
      return Promise.resolve();
    }
    let loaded: Promise<void>;
    try {
      loaded = Promise.resolve(a.context.audioWorklet.addModule(url));
    } catch (err) {
      loaded = Promise.reject(err); // no audioWorklet (an old WebKit) fails like a bad module
    }
    return loaded
      .then(() => {
        if (current !== a) return; // the context was replaced meanwhile
        const node = createNode(a.context, MASTER_PROCESSOR, {
          channelCount: channels,
          channelCountMode: 'explicit',
          channelInterpretation: 'discrete',
          numberOfInputs: 1,
          numberOfOutputs: 1,
          outputChannelCount: [channels],
        });
        node.port.onmessage = emit;
        node.port.postMessage({ limiter });
        node.connect(a.context.destination);
        a.node = node;
        a.status = 'ready';
        tap.ensure();
      })
      .catch((err) => {
        if (current !== a) return;
        a.status = 'failed';
        console.error('the master worklet did not load:', err);
      });
  };

  const tap: MasterTap = {
    ensure() {
      // getController() would create an AudioContext outside a gesture: never before running.
      if (!deps.isRunning()) return;
      try {
        const gain = deps.getController()?.output.destinationGain ?? null;
        if (!gain || gain === source) return; // mid-reset(), or nothing changed
        const context = gain.context;
        if (!current || current.context !== context) {
          // A new (or replaced) context: its own module and node. The old gain feeds the old
          // context's node, which goes with that context.
          source = null;
          const a: Attachment = {
            context,
            status: 'loading',
            node: null,
            settled: Promise.resolve(),
          };
          current = a;
          a.settled = load(a);
          return;
        }
        if (current.status !== 'ready' || !current.node) return; // the direct edge stays
        const node = current.node;
        if (source) {
          try {
            source.disconnect(node);
          } catch {
            // reset() already disconnected the old gain from everything.
          }
          source = null;
        }
        try {
          gain.disconnect(context.destination);
        } catch {
          // Not connected to the destination: nothing to remove.
        }
        try {
          gain.connect(node);
        } catch (err) {
          // Never leave the master silent: put the direct edge back and retry on the next call.
          gain.connect(context.destination);
          throw err;
        }
        source = gain;
      } catch {
        // A failed rewire leaves `source` null, so the next call tries again.
      }
    },
    async ready(timeoutMs = READY_TIMEOUT_MS) {
      tap.ensure();
      const a = current;
      if (!a || a.status !== 'loading') return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const timeout = new Promise<void>((resolve) => (timer = setTimeout(resolve, timeoutMs)));
      try {
        await Promise.race([a.settled, timeout]);
      } finally {
        clearTimeout(timer);
      }
    },
    onLevels(cb) {
      listeners.add(cb);
      return () => void listeners.delete(cb);
    },
    setLimiter(on) {
      limiter = on;
      try {
        current?.node?.port.postMessage({ limiter: on });
      } catch {
        // A closed context's port: the next node gets the preference.
      }
    },
    available: () => current?.status !== 'failed',
    latencySec() {
      if (!current?.node || !source) return 0;
      const rate = current.context.sampleRate;
      return lookaheadFrames(rate) / rate;
    },
  };
  return tap;
}

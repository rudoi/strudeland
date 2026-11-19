/**
 * A fake audio graph (no Web Audio): a context with a settable clock, latencies and output stamp
 * whose `createOscillator`/`createGain`/`createBufferSource` record every node, its schedule and its
 * automation, `createBuffer` buffers that keep what is copied into them, and a destination that
 * records what connects to it.
 */
import type { AudioGraph } from '../../src/lib/audio/backend';

export interface FakeNode {
  kind: 'oscillator' | 'gain' | 'source';
  /** What this node was connected to, in order. */
  connects: unknown[];
  disconnected: boolean;
}
export interface FakeOscillator extends FakeNode {
  kind: 'oscillator';
  type: string;
  frequency: { value: number };
  starts: number[];
  stops: number[];
  onended: (() => void) | null;
  /** Fires `onended`, as the context does after `stop(t)`. */
  end(): void;
}
export interface FakeGain extends FakeNode {
  kind: 'gain';
  /** Automation calls in order: [method, value, time]. */
  automation: [method: string, value: number, time: number][];
}

export interface FakeBuffer {
  numberOfChannels: number;
  length: number;
  sampleRate: number;
  duration: number;
  /** What `copyToChannel` copied, per channel. */
  data: (Float32Array | null)[];
}
export interface FakeSource extends FakeNode {
  kind: 'source';
  buffer: FakeBuffer | null;
  /** `start(when, offset)` calls. */
  starts: [when: number, offset: number | undefined][];
  stops: number;
  onended: (() => void) | null;
  /** Fires `onended`, as the context does when the buffer ends or after `stop()`. */
  end(): void;
}

export interface FakeGraphOptions {
  currentTime?: number;
  /** Absent when undefined (as on a context without the property). */
  baseLatency?: number;
  outputLatency?: number;
  /** What `getOutputTimestamp()` returns; `'missing'` removes the method, `'throw'` makes it throw. */
  outputTimestamp?: AudioTimestamp | 'missing' | 'throw';
}

export function fakeAudioGraph(opts: FakeGraphOptions = {}) {
  const nodes: (FakeOscillator | FakeGain | FakeSource)[] = [];
  const buffers: FakeBuffer[] = [];
  const destination = { connects: [] as unknown[] };
  const base = (kind: FakeNode['kind']) => ({
    kind,
    connects: [] as unknown[],
    disconnected: false,
    connect(dest: any) {
      this.connects.push(dest);
      if (dest === destinationNode) destination.connects.push(this);
      return dest;
    },
    disconnect() {
      this.disconnected = true;
    },
  });
  const destinationNode = { name: 'destination' };
  const context: Record<string, any> = {
    currentTime: opts.currentTime ?? 0,
    createOscillator() {
      const osc = {
        ...base('oscillator'),
        type: 'sine',
        frequency: { value: 440 },
        starts: [] as number[],
        stops: [] as number[],
        onended: null as (() => void) | null,
        start(t: number) {
          this.starts.push(t);
        },
        stop(t: number) {
          this.stops.push(t);
        },
        end() {
          this.onended?.();
        },
      } as FakeOscillator & { connect: any; disconnect: any };
      nodes.push(osc);
      return osc;
    },
    createBuffer(numberOfChannels: number, length: number, sampleRate: number) {
      if (length <= 0) throw new Error('NotSupportedError: length');
      const buf = {
        numberOfChannels,
        length,
        sampleRate,
        duration: length / sampleRate,
        data: Array.from({ length: numberOfChannels }, () => null) as (Float32Array | null)[],
        copyToChannel(src: Float32Array, ch: number) {
          this.data[ch] = new Float32Array(src);
        },
      };
      buffers.push(buf);
      return buf;
    },
    createBufferSource() {
      const src = {
        ...base('source'),
        buffer: null as FakeBuffer | null,
        starts: [] as [number, number | undefined][],
        stops: 0,
        onended: null as (() => void) | null,
        start(when: number, offset?: number) {
          this.starts.push([when, offset]);
        },
        stop() {
          this.stops++;
        },
        end() {
          this.onended?.();
        },
      } as FakeSource & { connect: any; disconnect: any };
      nodes.push(src);
      return src;
    },
    createGain() {
      const automation: FakeGain['automation'] = [];
      const param = (method: string) => (value: number, time: number) =>
        void automation.push([method, value, time]);
      const gain = {
        ...base('gain'),
        automation,
        gain: {
          value: 1,
          setValueAtTime: param('setValueAtTime'),
          linearRampToValueAtTime: param('linearRampToValueAtTime'),
          exponentialRampToValueAtTime: param('exponentialRampToValueAtTime'),
        },
      } as FakeGain & { connect: any; disconnect: any };
      nodes.push(gain);
      return gain;
    },
  };
  if (opts.baseLatency !== undefined) context.baseLatency = opts.baseLatency;
  if (opts.outputLatency !== undefined) context.outputLatency = opts.outputLatency;
  let stamp = opts.outputTimestamp ?? { contextTime: 0, performanceTime: 0 };
  const installStamp = () => {
    if (stamp === 'missing') delete context.getOutputTimestamp;
    else
      context.getOutputTimestamp = () => {
        if (stamp === 'throw') throw new Error('InvalidStateError');
        return stamp;
      };
  };
  installStamp();
  const graph: AudioGraph = {
    context: context as unknown as AudioContext,
    destination: destinationNode as unknown as AudioNode,
  };
  return {
    graph,
    /** Every node created, in order. */
    nodes,
    oscillators: () => nodes.filter((n): n is FakeOscillator => n.kind === 'oscillator'),
    gains: () => nodes.filter((n): n is FakeGain => n.kind === 'gain'),
    sources: () => nodes.filter((n): n is FakeSource => n.kind === 'source'),
    /** Every buffer created, in order. */
    buffers,
    /** The nodes connected to the destination, in order. */
    destination,
    setTime: (t: number) => void (context.currentTime = t),
    setLatency: (l: { baseLatency?: number; outputLatency?: number }) => {
      for (const k of ['baseLatency', 'outputLatency'] as const) {
        if (k in l) {
          if (l[k] === undefined) delete context[k];
          else context[k] = l[k];
        }
      }
    },
    setOutputTimestamp: (s: AudioTimestamp | 'missing' | 'throw') => {
      stamp = s;
      installStamp();
    },
  };
}

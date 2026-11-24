import { writable } from 'svelte/store';
import { errorMessage } from '../errors';
import type { InstrumentVersion } from '../project/api';
import { decodeWav } from '../render/wav';
import type { AudioGraph } from './backend';

type PreviewState =
  { status: 'idle' | 'loading' | 'playing' } | { status: 'error'; message: string };

export function createInstrumentPreview(deps: {
  eligible(): boolean;
  prepare(): Promise<boolean>;
  graph(): AudioGraph | null;
  render(version: InstrumentVersion): Promise<Uint8Array>;
}) {
  const store = writable<PreviewState>({ status: 'idle' });
  let generation = 0;
  let source: AudioBufferSourceNode | null = null;

  function stop() {
    generation++;
    if (source) {
      source.onended = null;
      source.disconnect();
      // A source can also be retired after start() failed.
      try {
        source.stop();
      } catch {}
      source = null;
    }
    store.set({ status: 'idle' });
  }

  async function play(version: InstrumentVersion) {
    stop();
    const request = generation;
    const busy = 'stop playback and recording to preview';
    try {
      if (!deps.eligible()) throw new Error(busy);
      store.set({ status: 'loading' });
      // Begin unlocking Web Audio synchronously inside the preview gesture.
      const ready = await deps.prepare();
      if (request !== generation) return;
      if (!ready) throw new Error('could not start preview audio');
      if (!deps.eligible()) throw new Error(busy);
      const bytes = await deps.render(version);
      if (request !== generation) return;
      if (!deps.eligible()) throw new Error(busy);
      const graph = deps.graph();
      if (!graph) throw new Error('audio output is unavailable');
      const { channels, sampleRate } = decodeWav(bytes);
      const { context, destination } = graph;
      const buffer = context.createBuffer(channels.length, channels[0].length, sampleRate);
      channels.forEach((channel, index) =>
        buffer.copyToChannel(channel as Float32Array<ArrayBuffer>, index),
      );
      source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(destination);
      source.onended = () => {
        if (request === generation) stop();
      };
      source.start(context.currentTime);
      store.set({ status: 'playing' });
    } catch (e) {
      if (request !== generation) return;
      stop();
      store.set({ status: 'error', message: errorMessage(e) });
    }
  }

  return { store: { subscribe: store.subscribe }, play, stop };
}

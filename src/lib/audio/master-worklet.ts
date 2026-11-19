/**
 * The `master-meter` AudioWorklet: superdough's master output passes through it (inserted by
 * `master-tap.ts`). A thin processor over `master-dsp.ts`: it measures and (when on) limits each
 * block in place, posts a `LevelFrame` whenever at least `LEVEL_EVERY_SEC` of frames have
 * accumulated (checked at the end of each block; a silent frame after a silent one is not
 * posted, so an idle context posts nothing), and takes `{ limiter: boolean }` messages.
 * Bundled into a data URL by vite-plugin-bundle-audioworklet (`./master-worklet?audioworklet` in
 * `master-tap.ts`).
 */

import {
  createMasterDsp,
  createSilenceGate,
  LEVEL_EVERY_SEC,
  MASTER_PROCESSOR,
  type MasterDsp,
} from './master-dsp';

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(name: string, ctor: unknown): void;
declare const sampleRate: number;

class MasterMeterProcessor extends AudioWorkletProcessor {
  private readonly dsp: MasterDsp;
  private readonly every: number;
  private readonly shouldPost = createSilenceGate();

  constructor(options: { outputChannelCount?: number[] }) {
    super();
    this.dsp = createMasterDsp(sampleRate, options.outputChannelCount?.[0] ?? 2);
    this.every = Math.max(1, Math.round(LEVEL_EVERY_SEC * sampleRate));
    this.port.onmessage = (e: MessageEvent) => {
      const on = (e.data as { limiter?: unknown } | null)?.limiter;
      if (typeof on === 'boolean') this.dsp.setEnabled(on);
    };
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const output = outputs[0];
    if (!output || output.length === 0) return true;
    this.dsp.process(inputs[0] ?? [], output);
    if (this.dsp.pendingFrames() >= this.every) {
      const frame = this.dsp.takeFrame();
      if (frame && this.shouldPost(frame)) this.port.postMessage(frame);
    }
    return true;
  }
}

registerProcessor(MASTER_PROCESSOR, MasterMeterProcessor);

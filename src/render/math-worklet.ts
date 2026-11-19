/**
 * The `v8-math-processor` AudioWorklet of the render page's `math` kind: on its first
 * `process()` call it evaluates every group (`evalMathGroup`, in the worklet's V8), posts the
 * outputs (or `{ error }`) once, and returns false. Bundled into a data URL by
 * vite-plugin-bundle-audioworklet (`./math-worklet?audioworklet` in `main.ts`).
 */

import { evalMathGroup } from './math';
import type { MathGroupInput } from './protocol';

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(name: string, ctor: unknown): void;

class V8MathProcessor extends AudioWorkletProcessor {
  private readonly groups: MathGroupInput[];
  private done = false;

  constructor(options: { processorOptions: { groups: MathGroupInput[] } }) {
    super();
    this.groups = options.processorOptions.groups;
  }

  process(): boolean {
    if (this.done) return false;
    this.done = true;
    try {
      this.port.postMessage({ outputs: this.groups.map(evalMathGroup) });
    } catch (e) {
      this.port.postMessage({ error: e instanceof Error ? e.message : String(e) });
    }
    return false;
  }
}

registerProcessor('v8-math-processor', V8MathProcessor);

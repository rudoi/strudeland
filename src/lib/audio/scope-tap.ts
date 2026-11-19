/**
 * A read-only tap on superdough's master output: an `AnalyserNode` fed from the output's
 * `destinationGain`. The analyser has no outputs and is not in superdough's `analysers` registry, so
 * it cannot change what is heard. Imported only by `superdough.ts` and, for `SCOPE_FFT_SIZE`, the
 * scope clock (never by offline renders).
 */

interface TapContext {
  createAnalyser(): TapAnalyser;
}
interface TapAnalyser {
  fftSize: number;
  getFloatTimeDomainData(buf: Float32Array): void;
}
export interface TapNode {
  connect(dest: TapAnalyser): unknown;
  disconnect(dest: TapAnalyser): unknown;
  context: TapContext;
}
/** The part of superdough's audio controller the tap reads. */
export type ScopeController = { output: { destinationGain: TapNode | null } } | null;

export interface ScopeTapDeps {
  /** Checked first: while false the tap never touches superdough. */
  isRunning: () => boolean;
  /** superdough's `getSuperdoughAudioController`; it creates an AudioContext when none exists. */
  getController: () => ScopeController;
}

export interface ScopeTap {
  /**
   * Fills `buf` with the latest output samples; false when there is no signal source. Never throws
   * (it runs inside an animation frame).
   */
  read(buf: Float32Array): boolean;
}

/** The analyser size; the scope clock's buffer is this long (`SCOPE_BUFFER_SIZE`). */
export const SCOPE_FFT_SIZE = 2048;

export function createScopeTap(deps: ScopeTapDeps): ScopeTap {
  let analyser: TapAnalyser | null = null;
  /** The context `analyser` was created from: a node can feed it only from the same context. */
  let analyserContext: TapContext | null = null;
  let source: TapNode | null = null;
  const detach = () => {
    if (source && analyser) {
      try {
        source.disconnect(analyser);
      } catch {
        // The old node is already disconnected (reset() disconnects everything).
      }
    }
    source = null;
  };
  return {
    read(buf) {
      // getController() would create an AudioContext outside a gesture: never before running.
      if (!deps.isRunning()) return false;
      try {
        const node = deps.getController()?.output.destinationGain ?? null;
        if (!node) return false; // mid-reset()
        if (node !== source) {
          detach();
          // A replaced AudioContext needs a new analyser: connect() cannot cross contexts.
          if (!analyser || analyserContext !== node.context) {
            analyser = node.context.createAnalyser();
            analyser.fftSize = SCOPE_FFT_SIZE;
            analyserContext = node.context;
          }
          node.connect(analyser);
          source = node;
        }
        analyser!.getFloatTimeDomainData(buf);
        return true;
      } catch {
        // A failed connect leaves `source` null, so the next read tries again.
        return false;
      }
    },
  };
}

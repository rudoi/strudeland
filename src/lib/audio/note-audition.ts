import { midiToFreq } from '@strudel/core';
import { getSound } from 'superdough';
import { auditionSounds, isAuditionSound } from './audition-sound';
import type { AudioGraph } from './backend';

export interface AuditionRequest {
  pitch: number;
  sound: string;
  controls: Readonly<Record<string, number>>;
  fallbackToTriangle?: boolean;
}

interface SynthHandle {
  node: AudioNode;
  stop(when: number): void;
}

type SynthTrigger = (
  when: number,
  value: Record<string, number>,
  ended: () => void,
) => SynthHandle | Promise<SynthHandle>;

const builtinTriggers = new Map<string, SynthTrigger>();

/** Called immediately after prebake registers the built-ins, before song evaluation. */
export function captureAuditionSynths(): void {
  builtinTriggers.clear();
  for (const name of auditionSounds) {
    const trigger = getSound(name)?.onTrigger;
    if (trigger) builtinTriggers.set(name, trigger);
  }
}

function registeredTrigger(name: string): SynthTrigger | undefined {
  const builtin = builtinTriggers.get(name);
  // A song can replace even a stock registry name with a custom sound.
  return getSound(name)?.onTrigger === builtin ? builtin : undefined;
}

const GATE_SECONDS = 0.16;
const ENVELOPE_MAX = { attack: GATE_SECONDS, decay: GATE_SECONDS, sustain: 1, release: 0.12 };

export function createNoteAuditionPlayer(deps: {
  graph(): AudioGraph | null;
  lookup?: (name: string) => SynthTrigger | undefined;
}) {
  const lookup = deps.lookup ?? registeredTrigger;
  let cancelVoice: (() => void) | undefined;

  function stop(): void {
    cancelVoice?.();
  }

  async function play(
    { pitch, sound, controls, fallbackToTriangle }: AuditionRequest,
    onSounding?: (fallback: boolean) => (() => void) | undefined,
  ): Promise<void> {
    stop();
    let graph: AudioGraph | null = null;
    let output: GainNode | undefined;
    let handle: SynthHandle | undefined;
    let clearHighlight: (() => void) | undefined;
    function ended(): void {
      if (cancelVoice === cancel) cancelVoice = undefined;
      output?.disconnect();
      const clear = clearHighlight;
      clearHighlight = undefined;
      clear?.();
    }
    function cancel(): void {
      ended();
      try {
        if (graph) handle?.stop(graph.context.currentTime);
      } catch {
        // The source may already have ended. Its output has been disconnected.
      }
    }
    try {
      if (!Number.isInteger(pitch) || pitch < 0 || pitch > 127) return;
      const selected = isAuditionSound(sound) ? lookup(sound.toLowerCase()) : undefined;
      const fallback = !selected && fallbackToTriangle === true;
      // The fallback uses the captured built-in even when the song replaces the registry entry.
      const trigger = selected ?? (fallback ? builtinTriggers.get('triangle') : undefined);
      graph = deps.graph();
      if (!trigger || !graph) return;
      // Superdough treats note: 0 as a missing note. Frequency also covers C-1.
      const value: Record<string, number> = { freq: midiToFreq(pitch), duration: GATE_SECONDS };
      for (const [key, max] of Object.entries(ENVELOPE_MAX)) {
        const n = controls[key];
        if (Number.isFinite(n)) value[key] = Math.min(max, Math.max(0, n));
      }
      cancelVoice = cancel;
      handle = await trigger(graph.context.currentTime, value, ended);
      if (cancelVoice !== cancel || deps.graph()?.destination !== graph.destination) {
        cancel();
        return;
      }
      output = graph.context.createGain();
      const gain = Number.isFinite(controls.gain) ? controls.gain : 1;
      output.gain.value = 0.5 * Math.min(1, Math.max(0, gain));
      handle.node.connect(output);
      output.connect(graph.destination);
      if (output.gain.value > 0) clearHighlight = onSounding?.(fallback);
    } catch {
      // Preview is best effort while the graph is being rebuilt or a worklet is unavailable.
      cancel();
    }
  }

  return { play, stop };
}

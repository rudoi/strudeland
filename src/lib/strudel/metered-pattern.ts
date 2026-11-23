import { Fraction, Hap, Pattern, State, TimeSpan } from '@strudel/core';
import type { HapTiming } from '../audio/backend';
import { beatsPerBar, type SongStructure } from './structure';
import type { LayoutClock } from './timing';

export const MIXED_CPS_ERROR = 'mixed-meter songs use meta.bpm and cannot change CPS in the code';

interface Scheduled {
  hap: any;
  cps: number;
  timing: HapTiming;
}

// Cyclist must see physical spans to calculate target times. Outputs, custom
// callbacks and seed derivation receive the logical hap that produced them.
const scheduled = Symbol('metered schedule');
export const meteredSchedule = (hap: any): Scheduled | undefined => hap[scheduled];

/** Query the final composed pattern in musical cycles. Every piece sees its own
 * CPS, including independently stacked patterns and outer transformations. */
export function queryMetered(
  pattern: any,
  state: any,
  structure: SongStructure,
  clock: LayoutClock,
  referenceCps: number,
): any[] {
  const end = Number(state.span.end);
  let from = Number(state.span.begin);
  const haps: any[] = [];
  do {
    const to = Math.min(end, clock.nextMeterBoundary(from));
    const localCps = (referenceCps * structure.beatsPerBar) / beatsPerBar(clock.meterAt(from));
    const piece = new State(new TimeSpan(from, to), { ...state.controls, _cps: localCps });
    const result = pattern.query(piece);
    if (structure.meterChanges?.length && result.some((h: any) => h.value?.cps !== undefined)) {
      throw new Error(MIXED_CPS_ERROR);
    }
    for (const h of result) haps.push(h);
    if (to >= end) return haps;
    if (to <= from) throw new Error('meter clock did not advance');
    from = to;
  } while (true);
}

/** Adapt only the Cyclist-facing pattern. A transport edit changes the supplied
 * clock for future queries, without changing any already scheduled hap. */
export function meteredPlayback(
  pattern: any,
  structure: SongStructure,
  getClock: () => LayoutClock | null,
  phaseOffset: () => number,
  onError: (error: unknown) => void,
): any {
  return new Pattern((state: any) => {
    const clock = getClock();
    if (!clock) return pattern.query(state);
    const cps = state.controls._cps ?? structure.cps;
    const span = state.span.withTime((t: any) => clock.toCycle(Number(t)));
    try {
      return queryMetered(pattern, state.setSpan(span), structure, clock, cps).map((h) => {
        const spanToClock = (s: any) => s.withTime((t: any) => clock.toClock(Number(t)));
        if (!h.whole)
          return new Hap(undefined, spanToClock(h.part), h.value, h.context, h.stateful);
        const onset = Number(h.whole.begin);
        const at = clock.toClock(onset);
        const localCps = (cps * structure.beatsPerBar) / beatsPerBar(clock.meterAt(onset));
        const timing = {
          phaseSec: phaseOffset() + at / cps,
          durationSec: (clock.toClock(Number(h.whole.begin.add(h.duration))) - at) / cps,
        };
        let context = h.context;
        if (context.onSchedule) {
          // Custom outputs receive the usual (hap, targetTime, cps). They compute
          // seconds as hap.duration / cps, including when the gate crosses a meter change.
          const callbackHap = Object.create(h, {
            duration: { value: Fraction(timing.durationSec * localCps) },
          });
          context = {
            ...context,
            onSchedule: (_h: any, t: number) => h.context.onSchedule(callbackHap, t, localCps),
          };
        }
        const mapped = new Hap(
          spanToClock(h.whole),
          spanToClock(h.part),
          h.value,
          context,
          h.stateful,
        );
        mapped[scheduled] = { hap: h, cps: localCps, timing };
        return mapped;
      });
    } catch (error) {
      // Pattern.queryArc catches exceptions. Report through the runtime as well.
      onError(error);
      throw error;
    }
  });
}

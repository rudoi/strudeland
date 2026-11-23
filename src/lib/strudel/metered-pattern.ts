import { Fraction, Hap, Pattern, State, TimeSpan } from '@strudel/core';
import type { HapTiming } from '../audio/backend';
import { cpsAtMeter, type SongStructure } from './structure';
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
  const mixedMeter = !!structure.meterChanges?.length;
  const painters = state.controls.painters;
  const registered = mixedMeter && painters ? new Map<unknown, number>() : undefined;
  do {
    const to = Math.min(end, clock.nextMeterBoundary(from));
    const localCps = cpsAtMeter(referenceCps, structure.meter, clock.meterAt(from));
    const piece = new State(new TimeSpan(from, to), {
      ...state.controls,
      _cps: localCps,
      _mixedMeter: mixedMeter,
    });
    if (registered) piece.controls.painters = [];
    const result = pattern.query(piece);
    if (registered) {
      // A painter belongs to the whole draw window. Preserve multiple copies
      // registered together, adding only new occurrences from later pieces.
      const occurrences = new Map<unknown, number>();
      for (const painter of piece.controls.painters) {
        const count = (occurrences.get(painter) ?? 0) + 1;
        occurrences.set(painter, count);
        if (count > (registered.get(painter) ?? 0)) {
          painters.push(painter);
          registered.set(painter, count);
        }
      }
    }
    if (mixedMeter && result.some((h: any) => h.value?.cps !== undefined)) {
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
    const toReference = (t: any) => clock.toClock(Number(t));
    try {
      return queryMetered(pattern, state.setSpan(span), structure, clock, cps).map((h) => {
        if (!h.whole)
          return new Hap(undefined, h.part.withTime(toReference), h.value, h.context, h.stateful);
        const onset = Number(h.whole.begin);
        const at = clock.toClock(onset);
        const localCps = cpsAtMeter(cps, structure.meter, clock.meterAt(onset));
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
          h.whole.withTime(toReference),
          h.part.withTime(toReference),
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

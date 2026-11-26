/**
 * The live clock: where the audible playhead is on the song's layout while tempo and meter edits
 * are still in the scheduler's lookahead, and the cps Strudel's `getCps` reads. A hot edit starts
 * at the scheduler's query horizon, so audio between the audible playhead and that horizon still
 * follows the previous structure, map and tempo; the clock keeps those as its edit history and
 * walks back through them.
 *
 * It does not own the segments (the transport does; the runtime passes them in) or the engines
 * (the runtime routes them). It reads the scheduler only through `ClockPort` and imports no
 * Strudel.
 */
import { layoutClock, type LayoutClock } from './timing';
import {
  buildStructure,
  cpsAtMeter,
  meterString,
  type Meter,
  type SongStructure,
} from './structure';
import type { Segment } from './transport';

/** What the clock reads from the scheduler (Cyclist), live at every call. */
export interface ClockPort {
  /** `sch.started`. */
  readonly started: boolean;
  /** `sch.now()`. */
  now(): number;
  /** `sch.lastBegin`, raw. */
  readonly lastBegin: number;
  /** `sch.lastEnd`, raw. */
  readonly lastEnd: number;
  /** `sch.lastTick`. */
  readonly lastTick: number;
  /** `sch.clock.duration`. */
  readonly tickDuration: number;
  /** `sch.getTime()`. */
  getTime(): number;
  /** `sch.cps`. */
  readonly cps: number;
}

/** One live edit still in the lookahead: the map, tempo and reference meter before it. */
interface ClockHistoryEntry {
  /** The edit's anchor: where the old structure stops governing. */
  end: { cycle: number; clock: number };
  /** The layout map before the edit. */
  map: LayoutClock | null;
  /** The scheduler's cps before the edit. */
  cps: number;
  /** The old structure's meter. */
  referenceMeter?: Meter;
}

/** The clock's whole state, held by reference (never copied). */
export interface ClockSnapshot {
  structure: SongStructure | null;
  map: LayoutClock | null;
  history: readonly ClockHistoryEntry[];
  phaseOffsetSec: number;
}

export interface LiveClock {
  /** The live structure: the one the last evaluate installed. */
  readonly structure: SongStructure | null;
  /** The layout map from layout cycles to reference clock; null when they coincide. */
  readonly map: LayoutClock | null;
  /** phase = offset + reference clock / CPS, rebased when an evaluate changes the tempo. */
  readonly phaseOffsetSec: number;
  /** Reference clock to layout cycle, through the map that governed that point. */
  toCycle(reference: number): number;
  /** Layout cycle to reference clock, through the map that governed that point. */
  toClock(cycle: number): number;
  /** The reference clock now sounding, walked back across the edits in the lookahead. */
  audibleReference(): number;
  /** toCycle(audibleReference()), unclamped: the layout cycle now sounding; Strudel's getTime. */
  audibleCycle(): number;
  /** max(0, audibleCycle()): the audible playhead. */
  now(): number;
  /** The cps now sounding, at the meter of the bar now sounding; Strudel's getCps. */
  cpsNow(): number;
  /** The scheduler's query horizon: { cycle: toCycle(Number(lastEnd)), clock: Number(lastEnd) }. */
  anchor(): { cycle: number; clock: number };
  /**
   * Rebuilds the map for `segments` (anchored at `anchor`), or clears it when the structure has no
   * meter changes and the anchor maps a cycle to itself. Does not keep `segments`.
   */
  setLayout(segments: readonly Segment[], anchor?: { cycle: number; clock: number }): void;
  /** An evaluate's install: anchor, history push, phase rebase, structure, layout. Returns the anchor. */
  edit(e: {
    /** The structure the evaluate registered. */
    structure: SongStructure | null;
    /** The evaluate's meter. */
    meter: Meter;
    /** The evaluate's cps. */
    nextCps: number;
    /** Whether the evaluate's code called setcps/setcpm. */
    codeSetCps: boolean;
    segments: readonly Segment[];
  }): { cycle: number; clock: number } | undefined;
  /** The stopped install: phase offset 0, history empty. Keeps structure and map. */
  reset(): void;
  /** The four fields, by reference. */
  snapshot(): ClockSnapshot;
  /** Assigns the four fields from `s` and nothing else. */
  restore(s: ClockSnapshot): void;
  /** Assigns map, history and phase offset from `s`, keeping the live structure. */
  resume(s: Pick<ClockSnapshot, 'map' | 'history' | 'phaseOffsetSec'>): void;
}

/** Reads nothing from `port` while constructing. */
export function createLiveClock(port: ClockPort): LiveClock {
  let structure: SongStructure | null = null;
  let map: LayoutClock | null = null;
  let history: readonly ClockHistoryEntry[] = [];
  let phaseOffsetSec = 0;

  const toCycle = (reference: number) => {
    const previous = history.find((h) => reference < h.end.clock);
    return (previous ? previous.map : map)?.toCycle(reference) ?? reference;
  };
  const toClock = (cycle: number) => {
    const previous = history.find((h) => cycle < h.end.cycle);
    return (previous ? previous.map : map)?.toClock(cycle) ?? cycle;
  };
  const audibleReference = (): number => {
    const latest = history.at(-1);
    const current = port.now();
    if (
      !port.started ||
      !latest ||
      (port.lastBegin >= latest.end.clock && current >= latest.end.clock)
    ) {
      return current;
    }
    // Cyclist.now places lastBegin at lastTick + one tick duration. The end of
    // that query is one tick later. Walk back from it at the tempos already
    // scheduled, including multiple edits within the lookahead window.
    let reference = Number(port.lastEnd);
    let seconds = port.lastTick + 2 * port.tickDuration - port.getTime();
    let cps = port.cps;
    for (let i = history.length - 1; i >= 0; i--) {
      const previous = history[i]!;
      const sinceEdit = (reference - previous.end.clock) / cps;
      if (seconds <= sinceEdit) break;
      seconds -= sinceEdit;
      reference = previous.end.clock;
      cps = previous.cps;
    }
    return reference - seconds * cps;
  };
  const cpsNow = (): number => {
    if (!port.started || (!map && history.length === 0)) return port.cps;
    const reference = audibleReference();
    const previous = history.find((h) => reference < h.end.clock);
    const at = previous ? previous.map : map;
    const base = previous ? previous.referenceMeter : structure?.meter;
    const cps = previous?.cps ?? port.cps;
    return base && at ? cpsAtMeter(cps, base, at.meterAt(at.toCycle(reference))) : cps;
  };
  const setLayout = (next: readonly Segment[], anchor?: { cycle: number; clock: number }) => {
    const needsClock = structure?.meterChanges?.length || (anchor && anchor.cycle !== anchor.clock);
    map = structure && needsClock ? layoutClock(structure, next, anchor) : null;
  };
  const anchor = () => {
    const reference = Number(port.lastEnd);
    return { cycle: toCycle(reference), clock: reference };
  };
  const audibleCycle = (): number => toCycle(audibleReference());
  const now = (): number => Math.max(0, audibleCycle());
  const edit: LiveClock['edit'] = (e) => {
    const a = port.started ? anchor() : undefined;
    if (a && (map || e.structure?.meterChanges?.length)) {
      const audible = audibleReference();
      const next = history.filter((h) => h.end.clock > audible);
      if (next.at(-1)?.end.clock !== a.clock) {
        next.push({ end: a, map, cps: port.cps, referenceMeter: structure?.meter });
      }
      history = next;
    }
    if (a && !e.codeSetCps) {
      phaseOffsetSec += a.clock / port.cps - a.clock / e.nextCps;
    }
    structure =
      e.structure ??
      (a && a.cycle !== a.clock ? buildStructure({ meter: meterString(e.meter) }) : null);
    setLayout(e.segments, a);
    return a;
  };

  return {
    get structure() {
      return structure;
    },
    get map() {
      return map;
    },
    get phaseOffsetSec() {
      return phaseOffsetSec;
    },
    toCycle,
    toClock,
    audibleReference,
    audibleCycle,
    now,
    cpsNow,
    anchor,
    setLayout,
    edit,
    reset: () => {
      phaseOffsetSec = 0;
      history = [];
    },
    snapshot: () => ({ structure, map, history, phaseOffsetSec }),
    restore: (s) => {
      structure = s.structure;
      map = s.map;
      history = s.history;
      phaseOffsetSec = s.phaseOffsetSec;
    },
    resume: (s) => {
      map = s.map;
      history = s.history;
      phaseOffsetSec = s.phaseOffsetSec;
    },
  };
}

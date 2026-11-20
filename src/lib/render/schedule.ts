/**
 * The render schedule: hap values plus their whole timespans, turned into plain events with
 * superdough's defaults filled in. The harness gates compare audio against these events. Pure.
 */

import { SAMPLE_RATE } from './contract';
import { LOOKBACK_MARGIN_SEC } from './thresholds';

export interface RenderEvent {
  /** Onset, layout seconds = hap.whole.begin / cps. */
  tSec: number;
  /** Whole duration / cps (not the clipped hap part). */
  durSec: number;
  /** floor(whole.begin) (layout bar). */
  bar: number;
  /** Sound name ('' when absent). */
  s: string;
  /** Default 1. */
  orbit: number;
  /** Hz, from freq or note; null for unpitched (samples, noise, drums). */
  freq: number | null;
  /** Raw value.note (n is NOT used: it is a sample index for samples). */
  note: number | string | null;
  /** gain (default 0.8) times velocity (default 1). */
  gain: number;
  /** Seconds, default 0. */
  attack: number;
  /** Default 0. */
  decay: number;
  sustain: number | null;
  /** Default 0.01. */
  release: number;
  /** Default 0. */
  penv: number;
  cutoff: number | null;
  /** Reverb: null when room is unset or <= 0. */
  room: { amount: number; size: number; fade: number; lp: number; dim: number } | null;
  /** Delay: null when delay is unset or <= 0. */
  delay: { amount: number; timeSec: number; feedback: number } | null;
  /** Kabelsalat voice: `value.workletSrc !== undefined`, the only unseeded waveform. */
  stochastic: boolean;
  /** Seconds, default 0: top-level `FXrelease` (superdough's `fullRelease = max(release,
   *  FXrelease ?? 0)`); may be negative or large, `deriveLookback` handles it. */
  fxRelease: number;
  /** A compressor on the hap value or any `FX` entry (`compressor !== undefined`, so `0` and
   *  `null` count, as in superdough). */
  compressor: boolean;
  /** Buses this voice sends to (`String(value.bus)` when `bus != null`); at most one. */
  busSends: string[];
  /** Buses this voice reads, in first-seen order, de-duplicated: the bus sound's `n ?? 0`, then
   *  every `bmod` bus of the hap value and of each `FX` entry. Over-approximates (a `bmod` whose
   *  target is missing is still listed); that only lengthens the lookback. */
  busReads: string[];
}

export interface EventSource {
  value: Record<string, unknown>;
  wholeBeginCycles: number;
  wholeDurCycles: number;
}

/** A finite number from a number or numeric string, else null. */
function num(x: unknown): number | null {
  let n: number;
  if (typeof x === 'number') n = x;
  else if (typeof x === 'string' && x.trim() !== '') n = Number(x);
  else return null;
  return Number.isFinite(n) ? n : null;
}

const CHROMAS: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };
const NOTE_RE = /^([a-g])([#bs]*)(-?\d+)?$/i;

/** MIDI number of a note name (superdough `noteToMidi`, default octave 3), or null. */
function noteNameToMidi(name: string): number | null {
  const m = NOTE_RE.exec(name.trim());
  if (!m) return null;
  const [, pc, acc, oct] = m;
  let semi = CHROMAS[pc!.toLowerCase()]!;
  for (const ch of acc!) semi += ch === 'b' ? -1 : 1;
  const octave = oct === undefined ? 3 : Number(oct);
  return (octave + 1) * 12 + semi;
}

const midiToFreq = (midi: number): number => 440 * Math.pow(2, (midi - 69) / 12);

function freqOf(v: Record<string, unknown>): number | null {
  if (typeof v.freq === 'number' && Number.isFinite(v.freq)) return v.freq;
  if (typeof v.note === 'number') return Number.isFinite(v.note) ? midiToFreq(v.note) : null;
  if (typeof v.note === 'string') {
    const midi = noteNameToMidi(v.note);
    return midi === null ? null : midiToFreq(midi);
  }
  return null;
}

const isObject = (x: unknown): x is Record<string, unknown> => x !== null && typeof x === 'object';

/** The object entries of `v.FX` (superdough iterates `[...value.FX, value]`); anything else is
 *  ignored here, since the engine refuses such a job anyway. */
const fxEntries = (v: Record<string, unknown>): Record<string, unknown>[] =>
  Array.isArray(v.FX) ? v.FX.filter(isObject) : [];

/** The modulator ids of a `bmod` value: its iterable `__ids` (a `Set` on the page), else none. */
function modIds(x: unknown): unknown[] {
  if (!isObject(x)) return [];
  const ids: unknown = x.__ids;
  if (
    !isObject(ids) ||
    typeof (ids as { [Symbol.iterator]?: unknown })[Symbol.iterator] !== 'function'
  ) {
    return [];
  }
  try {
    return [...(ids as unknown as Iterable<unknown>)];
  } catch {
    return [];
  }
}

/** `String(x)` as superdough's property-key lookup sees it, or null when it throws (an object
 *  without a usable `toString`); malformed input never throws here. */
function busKey(x: unknown): string | null {
  try {
    return String(x);
  } catch {
    return null;
  }
}

const isPropertyKey = (x: unknown): x is PropertyKey =>
  typeof x === 'string' || typeof x === 'number' || typeof x === 'symbol';

function* modConfigs(mod: unknown): Generator<Record<string, unknown>> {
  if (!isObject(mod)) return;
  for (const id of modIds(mod)) {
    if (!isPropertyKey(id)) continue;
    const config = (mod as Record<PropertyKey, unknown>)[id];
    if (isObject(config)) yield config;
  }
}

/** `getBus` keys this voice reads: the bus sound's `n ?? 0`, then each `bmod` config's `bus` on
 *  the value and every `FX` entry (superdough's `connectBusModulator`). */
function busReadsOf(v: Record<string, unknown>): string[] {
  const reads = new Set<string>();
  const add = (x: unknown): void => {
    const k = busKey(x);
    if (k !== null) reads.add(k);
  };
  if (v.s === 'bus') add(v.n ?? 0);
  for (const e of [v, ...fxEntries(v)]) {
    for (const config of modConfigs(e.bmod)) add(config.bus);
  }
  return [...reads];
}

/** The bus a voice sends to: superdough's `if (bus != null) getBus(bus)`. */
function busSendsOf(bus: unknown): string[] {
  if (bus == null) return [];
  const k = busKey(bus);
  return k === null ? [] : [k];
}

/** Numeric controls as read by the engine (controls.rs js_to_number). Only reverb also
 *  converts arrays through their string representation (js_to_number_full). */
function engineNum(value: unknown): number | null {
  if (value === null || typeof value === 'boolean') return Number(value);
  if (typeof value === 'string') return num(Number(value));
  return num(value);
}

function roomNum(value: unknown): number | null {
  return Array.isArray(value) ? num(Number(value)) : engineNum(value);
}

const FILTERS = [
  ['cutoff', 'resonance', 'lp'],
  ['hcutoff', 'hresonance', 'hp'],
  ['bandf', 'bandq', 'bp'],
] as const;
const FILTER_MAX_TAIL_SEC = 30; // biquad.rs MAX_TAIL_TIME

/** Clearance for export after a filtered voice's amp envelope closes. The static estimate
 *  allows six Q-weighted periods at the nearer of DC and Nyquist, for both oscillatory and
 *  overdamped poles. The engine takes Q in dB for low/high pass and linear Q for bandpass.
 *  The engine caps a biquad's tail time at 30 seconds. */
function exportFilterTailSec(v: Record<string, unknown>): number {
  const entries = [v, ...fxEntries(v)];
  let seconds = 0;
  let hasFilter = false;
  let implicitSeconds = 0;
  for (const entry of entries) {
    // The vowel's parallel formants have six Q-weighted periods under one second (vowel.rs).
    if (entry.vowel !== undefined) implicitSeconds += 1;
    if (entry.phaserrate !== undefined && (engineNum(entry.phaserdepth ?? undefined) ?? 0.75) > 0)
      implicitSeconds += LOOKBACK_MARGIN_SEC; // the phaser's swept notch

    for (const [frequency, resonance, family] of FILTERS) {
      if (entry[frequency] === undefined) continue;
      hasFilter = true;
      const hz = engineNum(entry[frequency]) ?? 0;
      const rawQ = engineNum(entry[resonance]) ?? 1;
      const q = family === 'bp' ? rawQ : 10 ** (rawQ / 20);
      let lowHz = hz;
      let highHz = hz;
      // The filter envelope sweeps +/- octaves around its anchor, capped at 20 kHz.
      if (
        ['env', 'attack', 'decay', 'sustain', 'release'].some(
          (k) => entry[family + k] !== undefined,
        )
      ) {
        const octaves = Math.abs(engineNum(entry[family + 'env']) ?? 1);
        const offset = octaves * (engineNum(entry.fanchor) ?? 0);
        lowHz = Math.min(20000, hz * 2 ** -offset);
        highHz = Math.min(20000, hz * 2 ** (octaves - offset));
      }
      if (
        ['rate', 'sync', 'depth', 'depthfrequency', 'shape', 'skew'].some(
          (k) => entry[family + k] !== undefined,
        )
      ) {
        // A built-in filter LFO can sweep down to 30 Hz. With an envelope too, the two add.
        lowHz += Math.min(0, 30 - hz);
        highHz += Math.max(0, 20000 - hz);
      }
      const edgeHz = Math.min(lowHz, SAMPLE_RATE / 2 - highHz);
      // A static cutoff at DC or Nyquist degenerates to a pass or silence. A sweep passing
      // through that point can approach it arbitrarily closely.
      const rings = q > 0 && highHz > 0 && lowHz < SAMPLE_RATE / 2;
      if (rings)
        seconds += Math.max(LOOKBACK_MARGIN_SEC, (6 * Math.max(q, 1 / q)) / Math.max(0, edgeHz));
    }
  }
  if (!hasFilter && !implicitSeconds) return 0;
  // Generic modulators may target a different FX entry and drive its cutoff or Q beyond the
  // static estimate. This scan includes only filter targets, so gain modulation stays cheap.
  for (const entry of entries) {
    for (const mod of [entry.lfo, entry.env, entry.bmod]) {
      for (const config of modConfigs(mod)) {
        const control = typeof config.control === 'string' ? config.control.split('_')[0] : null;
        if (
          control === 'vowel' ||
          control === 'phasercenter' ||
          control === 'phaserdepth' ||
          FILTERS.some(([f, q]) => control === f || control === q)
        )
          return FILTER_MAX_TAIL_SEC;
      }
    }
  }
  // The 24 dB setting cascades each stage twice (numeric ftype wraps around three settings).
  const doubled =
    v.ftype === '24db' || (typeof v.ftype === 'number' && ((v.ftype % 3) + 3) % 3 >= 2);
  return Math.min(FILTER_MAX_TAIL_SEC, implicitSeconds + seconds * (doubled ? 2 : 1));
}

function toEvent(src: EventSource, cps: number, engineTiming = false): RenderEvent {
  const raw: unknown = src.value;
  let v: Record<string, unknown>;
  if (typeof raw === 'string') v = { s: raw };
  else if (raw !== null && typeof raw === 'object') v = raw as Record<string, unknown>;
  else v = {};

  const timingNum = engineTiming ? engineNum : num;
  const roomAmount = engineTiming ? roomNum(v.room) : num(v.room);
  const delayAmount = timingNum(v.delay);
  const note = typeof v.note === 'number' || typeof v.note === 'string' ? v.note : null;

  return {
    tSec: src.wholeBeginCycles / cps,
    durSec: src.wholeDurCycles / cps,
    bar: Math.floor(src.wholeBeginCycles),
    s: typeof v.s === 'string' ? v.s : '',
    orbit: timingNum(v.orbit ?? undefined) ?? 1,
    freq: freqOf(v),
    note,
    gain: (num(v.gain) ?? 0.8) * (num(v.velocity) ?? 1),
    attack: num(v.attack) ?? 0,
    decay: num(v.decay) ?? 0,
    sustain: num(v.sustain),
    release: timingNum(v.release) ?? 0.01,
    penv: num(v.penv) ?? 0,
    cutoff: num(v.cutoff),
    room:
      roomAmount !== null && roomAmount > 0
        ? {
            amount: roomAmount,
            size:
              (engineTiming
                ? roomNum(v.roomsize)
                : (num(v.roomsize) ?? num(v.size) ?? num(v.rsize))) ?? 2,
            fade: num(v.roomfade) ?? 0.1,
            lp: num(v.roomlp) ?? 15000,
            dim: num(v.roomdim) ?? 1000,
          }
        : null,
    delay:
      delayAmount !== null && delayAmount > 0
        ? {
            amount: delayAmount,
            timeSec:
              timingNum(v.delaytime ?? undefined) ?? (timingNum(v.delaysync) ?? 3 / 16) / cps,
            feedback: timingNum(v.delayfeedback) ?? 0.5,
          }
        : null,
    stochastic: v.workletSrc !== undefined,
    fxRelease: timingNum(v.FXrelease) ?? 0,
    compressor: v.compressor !== undefined || fxEntries(v).some((e) => e.compressor !== undefined),
    busSends: busSendsOf(v.bus),
    busReads: busReadsOf(v),
  };
}

/** Events for `sources`, sorted by tSec ascending (ties keep input order). Throws RangeError
 *  unless cps is positive and finite. */
export function eventsFromValues(sources: EventSource[], cps: number): RenderEvent[] {
  if (!Number.isFinite(cps) || cps <= 0) throw new RangeError(`invalid cps ${cps}`);
  return sources.map((s) => toEvent(s, cps)).sort((a, b) => a.tSec - b.tSec);
}

/** The export's lifetime model follows the engine duration and numeric coercions. Keep the
 *  schedule reported by includeEvents (and used by the regular parity gates) as authored. */
export function exportTimingEvent(
  value: Record<string, unknown>,
  onset: number,
  duration: number,
  cps: number,
) {
  const src = { value, wholeBeginCycles: onset, wholeDurCycles: duration };
  const djf = Math.fround(engineNum(value.djf ?? undefined) ?? 0.5);
  return {
    ...toEvent(src, cps, true),
    filterTailSec: exportFilterTailSec(value),
    djFilter: djf < 0.49 || djf > 0.51,
  };
}

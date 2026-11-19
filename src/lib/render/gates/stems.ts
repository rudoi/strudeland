/**
 * The stems gate: each orbit renders to its own channel pair, so a voice missing in a bar leaves
 * its pair without onsets where the schedule says it plays. Every fast-attack note is scored by
 * the strongest energy rise on its pair near its onset, gain-normalized; scores from every chunk
 * are pooled and judged against the orbit's median over the whole song. Pure (ported from the
 * earlier render harness's second version of stems scoring).
 */

import { ampToDb, mixToMono, onsetEnvelope } from '../dsp';
import type { RenderEvent } from '../schedule';
import {
  STEMS_EDGE_SEC,
  STEMS_HOP_SEC,
  STEMS_MAX_ATTACK_SEC,
  STEMS_MAX_PAIRS,
  STEMS_MIN_GAIN,
  STEMS_PRE_SEC,
} from '../thresholds';
import { roundDb, roundSec, type GateResult, type Json } from './types';

/** Stereo pair of an orbit (superdough.mjs:529): orbit k >= 1 on pair (k - 1) mod pairs, orbit
 *  0 on pair 0 (so orbit 0 and orbit 1 share it). */
export function orbitPair(orbit: number, pairs: number = STEMS_MAX_PAIRS): number {
  return orbit <= 0 ? 0 : (orbit - 1) % pairs;
}

export interface PairVerdict {
  pair: number;
  orbits: number[];
  verifiable: boolean;
  reason?: string;
  /** Fast-attack notes scored on the pair (0 when not verifiable). */
  notes: number;
}

export interface StemNoteScore {
  pair: number;
  orbit: number;
  /** Absolute layout bar: floor(tSec / barSec + 1e-6). */
  bar: number;
  tSec: number;
  /** Strongest onset near the note divided by its gain. */
  score: number;
}

export interface StemOnsetMiss {
  pair: number;
  orbit: number;
  bar: number;
  tSec: number;
  /** The bar's best score relative to the orbit's typical score, dB. */
  relDb: number;
}

export interface StemsScoreOptions {
  /** Layout seconds of frame 0 of `channels`; default 0. */
  offsetSec?: number;
  /** Default STEMS_MAX_ATTACK_SEC. */
  maxAttack?: number;
  /** Default STEMS_MIN_GAIN. */
  minGain?: number;
}

/** Scores every fast-attack note on a single-orbit pair. `channels` holds 2 x pairs stem
 *  channels. Pairs shared by aliasing, or with no fast-attack notes, are returned unverifiable.
 *  Throws RangeError unless there is an even number (>= 2) of equal-length channels. */
export function stemsOnsetScores(
  channels: Float32Array[],
  sampleRate: number,
  events: RenderEvent[],
  barSec: number,
  opts: StemsScoreOptions = {},
): { pairs: PairVerdict[]; notes: StemNoteScore[] } {
  if (channels.length < 2 || channels.length % 2 !== 0) {
    throw new RangeError(`stems need an even number of channels >= 2, got ${channels.length}`);
  }
  const pairs = channels.length / 2;
  const offset = opts.offsetSec ?? 0;
  const maxAttack = opts.maxAttack ?? STEMS_MAX_ATTACK_SEC;
  const minGain = opts.minGain ?? STEMS_MIN_GAIN;
  // onsetEnvelope rounds the hop to whole frames (221 at 44.1 kHz, not 220.5), so the hop used
  // to index the envelope is that rounded one; STEMS_HOP_SEC itself drifts 7 hops over 16 s.
  const hop = Math.max(1, Math.round(STEMS_HOP_SEC * sampleRate)) / sampleRate;
  const env: Float32Array[] = [];
  for (let p = 0; p < pairs; p++) {
    env.push(onsetEnvelope(mixToMono([channels[2 * p]!, channels[2 * p + 1]!]), sampleRate, hop));
  }
  const len = env[0]!.length;
  const span = len * hop;
  const lo = Math.round(STEMS_PRE_SEC / hop);
  const hi = Math.round(STEMS_EDGE_SEC / hop);

  const inWindow = events.filter(
    (e) => e.tSec >= offset && e.tSec - offset < span && e.gain >= minGain,
  );
  const orbitsByPair = new Map<number, Set<number>>();
  for (const e of inWindow) {
    const p = orbitPair(e.orbit, pairs);
    const set = orbitsByPair.get(p) ?? new Set<number>();
    set.add(e.orbit);
    orbitsByPair.set(p, set);
  }

  const verdicts: PairVerdict[] = [];
  const notes: StemNoteScore[] = [];
  for (const [p, set] of [...orbitsByPair.entries()].sort((a, b) => a[0] - b[0])) {
    const orbits = [...set].sort((a, b) => a - b);
    if (orbits.length > 1) {
      verdicts.push({ pair: p, orbits, verifiable: false, reason: 'shared by aliasing', notes: 0 });
      continue;
    }
    const fast = inWindow.filter((e) => orbitPair(e.orbit, pairs) === p && e.attack <= maxAttack);
    if (fast.length === 0) {
      verdicts.push({
        pair: p,
        orbits,
        verifiable: false,
        reason: 'no fast-attack notes',
        notes: 0,
      });
      continue;
    }
    verdicts.push({ pair: p, orbits, verifiable: true, notes: fast.length });
    const e = env[p]!;
    for (const n of fast) {
      const c = Math.round((n.tSec - offset) / hop);
      // A note in the last 40 ms has its rise in the next window: scoring it here reads silence.
      if (c + hi > len - 1) continue;
      let best = 0;
      for (let k = Math.max(0, c - lo); k <= Math.min(len - 1, c + hi); k++) {
        if (e[k]! > best) best = e[k]!;
      }
      notes.push({
        pair: p,
        orbit: n.orbit,
        bar: Math.floor(n.tSec / barSec + 1e-6),
        tSec: n.tSec,
        score: best / Math.max(n.gain, 1e-3),
      });
    }
  }
  return { pairs: verdicts, notes };
}

/** One verdict per pair seen in any chunk: verifiable if any chunk verified it (notes summed),
 *  else the reason of the first chunk that had one. Sorted by pair. */
export function mergePairVerdicts(perChunk: PairVerdict[][]): PairVerdict[] {
  const byPair = new Map<number, PairVerdict>();
  for (const list of perChunk) {
    for (const v of list) {
      const cur = byPair.get(v.pair);
      if (!cur) {
        byPair.set(v.pair, { ...v, orbits: [...v.orbits] });
        continue;
      }
      cur.orbits = [...new Set([...cur.orbits, ...v.orbits])].sort((a, b) => a - b);
      if (v.verifiable) {
        cur.notes = (cur.verifiable ? cur.notes : 0) + v.notes;
        cur.verifiable = true;
        delete cur.reason;
      } else if (!cur.verifiable && cur.reason === undefined && v.reason !== undefined) {
        cur.reason = v.reason;
      }
    }
  }
  return [...byPair.values()].sort((a, b) => a.pair - b.pair);
}

/** Flags an (orbit, bar) when none of the orbit's notes in that bar rose to within `missDb` of
 *  the orbit's typical (median) score. Pass a whole song's pooled scores: a median over one short
 *  chunk can be unstable, as seen in calibration with a 3-bar final chunk. An orbit with one note
 *  cannot miss. */
export function judgeStemScores(
  notes: StemNoteScore[],
  missDb: number,
): { tested: number; misses: StemOnsetMiss[] } {
  const byOrbit = new Map<number, StemNoteScore[]>();
  for (const n of notes) {
    const list = byOrbit.get(n.orbit) ?? [];
    list.push(n);
    byOrbit.set(n.orbit, list);
  }
  const misses: StemOnsetMiss[] = [];
  let tested = 0;
  for (const [orbit, ns] of byOrbit) {
    const sorted = ns.map((n) => n.score).sort((a, b) => a - b);
    const typical = sorted[Math.floor(sorted.length / 2)]!;
    if (!(typical > 0)) continue;
    const floor = typical * Math.pow(10, -Math.abs(missDb) / 20);
    const best = new Map<number, StemNoteScore>();
    for (const n of ns) {
      const cur = best.get(n.bar);
      if (!cur || n.score > cur.score) best.set(n.bar, n);
    }
    for (const [bar, n] of best) {
      tested++;
      if (n.score < floor) {
        misses.push({
          pair: n.pair,
          orbit,
          bar,
          tSec: roundSec(n.tSec),
          relDb: roundDb(ampToDb((n.score + 1e-12) / typical, 0)),
        });
      }
    }
  }
  misses.sort((a, b) => a.bar - b.bar || a.pair - b.pair);
  return { tested, misses };
}

const MAX_LISTED = 8;

function pairJson(v: PairVerdict): Json {
  return {
    pair: v.pair,
    orbits: v.orbits,
    verifiable: v.verifiable,
    reason: v.reason ?? null,
    notes: v.notes,
  };
}

/** The stems verdict from merged pair verdicts and the pooled judgement. */
export function stemsGate(
  pairs: PairVerdict[],
  judged: { tested: number; misses: StemOnsetMiss[] },
): GateResult {
  const warnings = pairs
    .filter((p) => !p.verifiable)
    .map((p) => `pair ${p.pair} (orbits ${p.orbits.join(', ')}) not verified: ${p.reason ?? ''}`);
  const measurements = {
    pairs: pairs.map(pairJson),
    tested: judged.tested,
    misses: judged.misses.length,
  };
  const reasons: string[] = [];
  let verdict: GateResult['verdict'];
  if (judged.misses.length > 0) {
    verdict = 'fail';
    for (const m of judged.misses.slice(0, MAX_LISTED)) {
      reasons.push(`orbit ${m.orbit} missing in bar ${m.bar}: best onset ${m.relDb} dB`);
    }
    if (judged.misses.length > MAX_LISTED) reasons.push(`+${judged.misses.length - MAX_LISTED}`);
  } else if (judged.tested === 0) {
    verdict = 'unverifiable';
    reasons.push('no testable stem: every pair aliased or without fast-attack notes');
  } else {
    verdict = 'pass';
  }
  return { gate: 'stems', verdict, measurements, reasons, warnings };
}

/**
 * The pure parts of `engine:refs` (`refs.ts`): argument parsing, which fixtures carry refs, ref
 * file names and texts, the superdough reference envelope of a fixture chunk, `index.json`, the
 * size budgets, merging a partial regeneration into the committed set, the diff summary, the
 * V8 math reference file, the measured-floor rule, and the evidence of the measure cases
 * (`measure-suspend-quantization`, `measure-feedback-echo`, `measure-worklet-start`,
 * `measure-worklet-teardown`). No Playwright, Strudel or file-system access, so `bun test` covers
 * it (`tests/engine-refs.test.ts`).
 */

import type { FixtureEntry } from '../../src/lib/render/corpus';
import type { EngineJob, Provenance } from '../../src/lib/render/engine-job';
import { pauseFrameOf, RENDER_QUANTUM as QUANTUM } from '../../src/lib/render/plan';
import { sortedKeys } from '../json-text';
import { sha256 } from '../sha256';
import { nullResidualDb } from '../render/probes/calibrate';
import {
  ENGINE_CORE_EVIDENCE_DIR,
  ENGINE_EFFECTS_EVIDENCE_DIR,
} from '../render/probes/evidence-path';
import type { MathGroupInput } from '../../src/render/protocol';
import {
  FIXTURE_PREFIX,
  MATH_CASE,
  MATH_FILE,
  type CaseKind,
  type MathProvenance,
} from './provenance';

export { MATH_CASE, MATH_FILE };

export const SR = 44100;
/** The refs directory, relative to the repo root. */
export const REFS_DIR = 'crates/strudeland-engine/tests/refs';
export const INDEX_FILE = 'index.json';
export const THRESHOLDS_FILE = 'thresholds.json';
export const INDEX_SCHEMA = 2;
const ENVELOPE_SCHEMA = 1;
/** 10 ms at 44100 Hz. */
const ENVELOPE_WINDOW_FRAMES = 441;
/** The dB value of a digitally silent window. */
export const SILENCE_DB = -200;

/** All `*.job.json` plus `*.ref.json` files together (05 section 1). */
export const BUDGET_JOB_BYTES = 8 * 1024 * 1024;
/** All `*.wav` files together. */
export const BUDGET_WAV_BYTES = 40 * 1024 * 1024;
/** `v8-math.json` on its own. */
export const MATH_MAX_BYTES = 1024 * 1024;
/** A micro WAV lasts at most 0.5 s ... */
const MICRO_MAX_FRAMES = 22_050;
/** ... except the long-prefixed cases (delay and reverb tails, IR swaps, the measure cases'
 *  pauses and feedback, compressor warm-up, duck recoveries, djf on the orbit, FX chains, orbit
 *  modulators): up to 2 s. */
const MICRO_LONG_MAX_FRAMES = 88_200;
export const LONG_PREFIXES = [
  'delay-',
  'reverb-',
  'measure-',
  'compressor-',
  'duck-',
  'djf-',
  'fx-chain-',
  'mod-orbit-',
];

/** Fixtures kept with refs on purpose although unsupported: they must fail with their expected
 *  signatures (`fixtures.rs`). `noise-seed` (zzfx) keeps a real refusal path tested once the
 *  others play. */
const KEPT_UNSUPPORTED = ['noise-seed'];

/** The measure case whose Chromium render becomes `evidence/measure-suspend-quantization.json`. */
export const SUSPEND_CASE = 'measure-suspend-quantization';
/** The measure case whose Chromium render becomes `evidence/measure-feedback-echo.json`. */
export const FEEDBACK_CASE = 'measure-feedback-echo';

// ---- arguments

export interface RefsOptions {
  write: boolean;
  names: string[];
  help: boolean;
}

export const REFS_USAGE = `usage: bun run engine:refs [--write] [name...]

Renders every micro case (tests/render/engine-micro/<name>.js) and every corpus fixture with refs
in Chromium, evaluates the V8 math reference (case ${MATH_CASE}: ${MATH_FILE}, twice, in two
fresh pages), and compares the result with ${REFS_DIR}/. Without --write it prints the diff
summary and writes nothing (exit 1 when something differs); with --write it writes the changed
files and index.json. Names (micro cases, fixture names or ${MATH_CASE}) restrict the run to
those cases; the other committed refs are kept (a schema 1 index.json needs a full run first).
Exit 0 current or written, 1 differs or over budget or a case failed, 2 bad arguments or harness
error.`;

export function parseRefsArgs(argv: string[]): RefsOptions | { error: string } {
  const o: RefsOptions = { write: false, names: [], help: false };
  for (const a of argv) {
    if (a === '--write') o.write = true;
    else if (a === '--help' || a === '-h') o.help = true;
    else if (a.startsWith('-')) return { error: `unknown flag ${a}` };
    else if (!/^[a-z0-9][a-z0-9-]*$/.test(a)) return { error: `invalid case name ${a}` };
    else if (!o.names.includes(a)) o.names.push(a);
  }
  return o;
}

/** Supported targeted fixtures and the named exceptions get reference jobs and envelopes.
 *  External fixtures are checked by `render:check --engine rust` without storing refs. */
export const fixtureHasRefs = (f: FixtureEntry): boolean =>
  (f.rust === 'supported' && f.kind !== 'external') || KEPT_UNSUPPORTED.includes(f.name);

export interface CaseSelection {
  micro: string[];
  fixtures: string[];
  /** The V8 math reference: on a full run, or when `v8-math` is named. */
  math: boolean;
}

/** Resolves the requested names (all cases when none) against the micro sources and the corpus.
 *  A name may not be both, nor a fixture without refs, nor unknown. */
export function selectCases(
  names: string[],
  microNames: string[],
  corpus: FixtureEntry[],
): CaseSelection | { error: string } {
  const withRefs = corpus.filter(fixtureHasRefs).map((f) => f.name);
  if (microNames.includes(MATH_CASE)) {
    return { error: `${MATH_CASE} is the V8 math reference; no micro case may take the name` };
  }
  if (corpus.some((f) => f.name === MATH_CASE)) {
    return { error: `${MATH_CASE} is the V8 math reference; no corpus fixture may take the name` };
  }
  for (const m of microNames) {
    if (m.startsWith(FIXTURE_PREFIX)) {
      return { error: `micro case ${m} may not start with ${FIXTURE_PREFIX}` };
    }
    if (corpus.some((f) => f.name === m)) {
      return { error: `${m} is both a micro case and a corpus fixture` };
    }
  }
  if (names.length === 0) {
    return { micro: [...microNames].sort(), fixtures: withRefs.sort(), math: true };
  }
  const micro: string[] = [];
  const fixtures: string[] = [];
  let math = false;
  for (const n of names) {
    if (n === MATH_CASE) math = true;
    else if (microNames.includes(n)) micro.push(n);
    else if (withRefs.includes(n)) fixtures.push(n);
    else if (corpus.some((f) => f.name === n)) {
      return { error: `fixture ${n} has no engine refs (external or unsupported fixture)` };
    } else return { error: `unknown micro case or fixture ${n}` };
  }
  return { micro, fixtures, math };
}

// ---- file names and texts

export const microJobFile = (name: string): string => `${name}.job.json`;
export const microWavFile = (name: string): string => `${name}.wav`;
export const fixtureJobFile = (fixture: string, mode: 'mix' | 'stems', k: number): string =>
  `${FIXTURE_PREFIX}${fixture}--${mode}-${k}.job.json`;
export const fixtureEnvelopeFile = (fixture: string, mode: 'mix' | 'stems', k: number): string =>
  `${FIXTURE_PREFIX}${fixture}--${mode}-${k}.ref.json`;

const encoder = new TextEncoder();
export const utf8 = (text: string): Uint8Array => encoder.encode(text);

/** A ref job: the unmutated job with `provenance` as its last top-level field, compact JSON (the
 *  job JSON is ASCII-only) and a newline. */
export function jobFileText(job: EngineJob, provenance: Provenance): string {
  const { provenance: _, ...rest } = job;
  return `${JSON.stringify({ ...rest, provenance })}\n`;
}

// ---- reference envelope

/** `x` rounded to `decimals` places (half away from zero); `-0` becomes `0`. */
export function roundTo(x: number, decimals: number): number {
  const f = 10 ** decimals;
  const r = (Math.sign(x) * Math.round(Math.abs(x) * f)) / f;
  return r === 0 ? 0 : r;
}

/** RMS of `x[from, to)` in dB (`20*log10(rms)`), `SILENCE_DB` for digital silence or an empty
 *  range, rounded to 0.01. A non-finite sample gives NaN (the caller refuses it). */
export function rmsDb(x: Float32Array, from: number, to: number): number {
  let sum = 0;
  for (let i = from; i < to; i++) sum += x[i]! * x[i]!;
  if (Number.isNaN(sum)) return NaN;
  if (to <= from || sum === 0) return SILENCE_DB;
  return Math.max(SILENCE_DB, roundTo(10 * Math.log10(sum / (to - from)), 2));
}

/** Largest absolute sample, rounded to 6 decimals. */
export function peakOf(x: Float32Array): number {
  let p = 0;
  for (const v of x) {
    const a = Math.abs(v);
    if (!(a <= p)) p = a;
  }
  return roundTo(p, 6);
}

export interface RefEnvelope {
  schema: 1;
  windowFrames: number;
  frames: number;
  channels: number;
  /** Per channel, per 10 ms window `[441 w, min(441 (w + 1), frames))`. */
  rmsDb: number[][];
  peak: number[];
}

/** The reference envelope of a superdough chunk render (the full buffer, lookback and overlap
 *  included). Throws on no channels, unequal lengths or a non-finite sample. */
export function referenceEnvelope(channels: Float32Array[]): RefEnvelope {
  if (channels.length === 0) throw new RangeError('no channels');
  const frames = channels[0]!.length;
  if (channels.some((c) => c.length !== frames)) throw new RangeError('channel length mismatch');
  const windows = Math.ceil(frames / ENVELOPE_WINDOW_FRAMES);
  const rms = channels.map((c) =>
    Array.from({ length: windows }, (_, w) =>
      rmsDb(c, w * ENVELOPE_WINDOW_FRAMES, Math.min((w + 1) * ENVELOPE_WINDOW_FRAMES, frames)),
    ),
  );
  const peak = channels.map(peakOf);
  if (rms.some((c) => c.some(Number.isNaN)) || peak.some((p) => !Number.isFinite(p))) {
    throw new RangeError('non-finite sample in the reference render');
  }
  return {
    schema: ENVELOPE_SCHEMA,
    windowFrames: ENVELOPE_WINDOW_FRAMES,
    frames,
    channels: channels.length,
    rmsDb: rms,
    peak,
  };
}

export const envelopeFileText = (e: RefEnvelope): string => `${JSON.stringify(e)}\n`;

// ---- index

export interface IndexCase {
  kind: CaseKind;
  /** A `MathProvenance` for the math case, a job `Provenance` otherwise. */
  provenance: Provenance | MathProvenance;
  /** The job files; empty for the math case. */
  jobs: string[];
  /** Fixture cases: the `*.ref.json` files, in `jobs` order; null otherwise. */
  envelopes: string[] | null;
  /** Micro cases: the WAV; null otherwise. */
  wav: string | null;
  /** The math case: its data file (`v8-math.json`); null otherwise. */
  data: string | null;
  /** A micro case declared `floor` in `thresholds.json`: its measured Chromium floor in dB
   *  (rounded to 0.01). Absent on every other case. */
  floorDb?: number;
}

export interface RefsIndex {
  schema: 2;
  files: Record<string, { sha256: string; bytes: number }>;
  cases: Record<string, IndexCase>;
}

/** The index of a refs directory: every file (sorted) and every case (sorted). */
export function buildIndex(
  files: ReadonlyMap<string, Uint8Array>,
  cases: Record<string, IndexCase>,
): RefsIndex {
  return {
    schema: INDEX_SCHEMA,
    files: sortedKeys(
      Object.fromEntries(
        [...files].map(([name, bytes]) => [name, { sha256: sha256(bytes), bytes: bytes.length }]),
      ),
    ),
    cases: sortedKeys(cases),
  };
}

export const indexText = (index: RefsIndex): string => `${JSON.stringify(index, null, 2)}\n`;

/** The ref files a case owns. */
export const caseFiles = (c: IndexCase): string[] => [
  ...c.jobs,
  ...(c.envelopes ?? []),
  ...(c.wav ? [c.wav] : []),
  ...(c.data ? [c.data] : []),
];

/** Why a run may not go ahead against the committed index: a partial `--write` against a schema
 *  1 index would mix schemas. `null` when it may. */
export function partialRunRefusal(
  committedIndex: { schema: number } | null,
  full: boolean,
  write: boolean,
): string | null {
  if (full || !write || committedIndex === null || committedIndex.schema === INDEX_SCHEMA) {
    return null;
  }
  return `index.json is schema ${committedIndex.schema}: run \`engine:refs --write\` without names`;
}

// ---- budgets

const isJobLike = (name: string): boolean =>
  name.endsWith('.job.json') || name.endsWith('.ref.json');
const isWav = (name: string): boolean => name.endsWith('.wav');

export const microFrameLimit = (name: string): number =>
  LONG_PREFIXES.some((p) => name.startsWith(p)) ? MICRO_LONG_MAX_FRAMES : MICRO_MAX_FRAMES;

export interface Totals {
  jobBytes: number;
  wavBytes: number;
  files: number;
}

export function totalsOf(files: ReadonlyMap<string, Uint8Array>): Totals {
  let jobBytes = 0;
  let wavBytes = 0;
  for (const [name, bytes] of files) {
    if (isJobLike(name)) jobBytes += bytes.length;
    else if (isWav(name)) wavBytes += bytes.length;
  }
  return { jobBytes, wavBytes, files: files.size };
}

const mib = (n: number): string => `${(n / (1024 * 1024)).toFixed(2)} MiB`;

export const formatTotals = (t: Totals): string =>
  `${t.files} files: jobs+envelopes ${mib(t.jobBytes)} of ${mib(BUDGET_JOB_BYTES)}, ` +
  `WAVs ${mib(t.wavBytes)} of ${mib(BUDGET_WAV_BYTES)}`;

/** Every budget the final refs set breaks (empty when within budget). `wavFrames` holds the frame
 *  count of each micro WAV by case name. */
export function budgetProblems(
  files: ReadonlyMap<string, Uint8Array>,
  wavFrames: ReadonlyMap<string, number>,
): string[] {
  const problems: string[] = [];
  const t = totalsOf(files);
  if (t.jobBytes > BUDGET_JOB_BYTES) {
    problems.push(`jobs and envelopes take ${t.jobBytes} bytes, over ${BUDGET_JOB_BYTES}`);
  }
  if (t.wavBytes > BUDGET_WAV_BYTES) {
    problems.push(`WAVs take ${t.wavBytes} bytes, over ${BUDGET_WAV_BYTES}`);
  }
  const math = files.get(MATH_FILE);
  if (math && math.length > MATH_MAX_BYTES) {
    problems.push(`${MATH_FILE} takes ${math.length} bytes, over ${MATH_MAX_BYTES}`);
  }
  for (const [name, frames] of [...wavFrames].sort()) {
    const limit = microFrameLimit(name);
    if (frames > limit) problems.push(`${name}: ${frames} frames, over ${limit}`);
  }
  return problems;
}

// ---- merge and diff

export interface GeneratedCase {
  name: string;
  entry: IndexCase;
  files: Map<string, Uint8Array>;
}

/** The refs set after a run: a full run (no names) is exactly the generated cases; a partial run
 *  replaces the generated cases (dropping their old files) and keeps every other committed case. */
export function mergeRefs(
  committedFiles: ReadonlyMap<string, Uint8Array>,
  committedIndex: RefsIndex | null,
  generated: GeneratedCase[],
  full: boolean,
): { files: Map<string, Uint8Array>; index: RefsIndex } {
  const cases: Record<string, IndexCase> = {};
  const files = new Map<string, Uint8Array>();
  if (!full && committedIndex) {
    const replaced = new Set(generated.map((g) => g.name));
    for (const [name, c] of Object.entries(committedIndex.cases)) {
      if (replaced.has(name)) continue;
      cases[name] = c;
      for (const f of caseFiles(c)) {
        const bytes = committedFiles.get(f);
        if (bytes) files.set(f, bytes);
      }
    }
  }
  for (const g of generated) {
    cases[g.name] = g.entry;
    for (const [f, bytes] of g.files) files.set(f, bytes);
  }
  return { files, index: buildIndex(files, cases) };
}

const bytesEqual = (a: Uint8Array, b: Uint8Array): boolean =>
  a.length === b.length && a.every((v, i) => v === b[i]);

export interface RefsDiff {
  added: string[];
  changed: string[];
  removed: string[];
  unchanged: number;
}

export function diffRefs(
  before: ReadonlyMap<string, Uint8Array>,
  after: ReadonlyMap<string, Uint8Array>,
): RefsDiff {
  const d: RefsDiff = { added: [], changed: [], removed: [], unchanged: 0 };
  for (const [name, bytes] of after) {
    const old = before.get(name);
    if (!old) d.added.push(name);
    else if (!bytesEqual(old, bytes)) d.changed.push(name);
    else d.unchanged++;
  }
  for (const name of before.keys()) if (!after.has(name)) d.removed.push(name);
  d.added.sort();
  d.changed.sort();
  d.removed.sort();
  return d;
}

export const hasChanges = (d: RefsDiff): boolean =>
  d.added.length + d.changed.length + d.removed.length > 0;

/** The printed summary: one line per added, changed or removed file with its sizes. */
export function formatDiff(
  d: RefsDiff,
  before: ReadonlyMap<string, Uint8Array>,
  after: ReadonlyMap<string, Uint8Array>,
): string[] {
  const size = (m: ReadonlyMap<string, Uint8Array>, n: string) => m.get(n)?.length ?? 0;
  return [
    ...d.added.map((n) => `  added    ${n} (${size(after, n)} bytes)`),
    ...d.changed.map((n) => `  changed  ${n} (${size(before, n)} -> ${size(after, n)} bytes)`),
    ...d.removed.map((n) => `  removed  ${n} (${size(before, n)} bytes)`),
    `  ${d.added.length} added, ${d.changed.length} changed, ${d.removed.length} removed, ` +
      `${d.unchanged} unchanged`,
  ];
}

// ---- measure-suspend-quantization

/** `TimeToSampleFrame(t, sr, RoundUp)` (Blink `audio_utilities.cc`): round to the nearest 1/1024
 *  frame (half away from zero), then ceil. */
export function timeToSampleFrameRoundUp(t: number, sr: number = SR): number {
  const x = t * sr * 1024;
  return Math.ceil((Math.sign(x) * Math.round(Math.abs(x))) / 1024);
}

/** The frame where an oscillator note scheduled at `t` (with a gain envelope ramping up from 0
 *  at `t`) is first non-zero, per `research/chromium.md`: the source starts at
 *  `TimeToSampleFrame(t, RoundUp)`. With a negative sub-sample offset the k-rate oscillator's
 *  first sample has a non-zero phase, and the envelope at that frame (evaluated at time
 *  `frame / sr`) is above 0 when `frame / sr > t`, so that frame sounds. With an offset of 0 (t
 *  on a frame) the phase and the envelope are 0 there, and with a tiny positive offset the first
 *  frame is skipped, so the next frame is the first. */
export function expectedFirstSounding(t: number, sr: number = SR): number {
  const start = timeToSampleFrameRoundUp(t, sr);
  return t * sr - start < 0 && start / sr > t ? start : start + 1;
}

/** First frame at or after `from` where any channel is non-zero; -1 when none before `to`. */
export function firstNonZero(channels: Float32Array[], from: number, to: number): number {
  for (let i = Math.max(0, from); i < to; i++) {
    if (channels.some((c) => c[i] !== undefined && c[i] !== 0)) return i;
  }
  return -1;
}

export interface SuspendEvidence {
  case: string;
  rule: string;
  pauses: {
    suspendSec: number;
    truncatedFrame: number;
    pauseFrame: number;
    truncatedIsMultipleOf128: boolean;
  }[];
  haps: {
    pause: number;
    t: number;
    tFrames: number;
    startFrame: number;
    expectedFirstSoundingFrame: number;
    observedFirstSoundingFrame: number;
    afterPauseFrame: boolean;
    agrees: boolean;
  }[];
  agrees: boolean;
}

/** How many frames before a hap's expected start the search for its first sound begins; the
 *  case's notes are far enough apart that this range is silent. */
const ONSET_SEARCH_FRAMES = 256;

/** The evidence of the `measure-suspend-quantization` case from its job and Chromium render:
 *  every pause frame (truncate, round up to 128) and, for every hap, the frame where it first
 *  sounds against `expectedFirstSounding`. `agrees` is false when any hap sounds elsewhere or
 *  before its pause frame. */
export function suspendEvidence(job: EngineJob, channels: Float32Array[]): SuspendEvidence {
  const pauses = job.pauses.map((p) => {
    const truncatedFrame = Math.trunc(p.suspendSec * SR);
    return {
      suspendSec: p.suspendSec,
      truncatedFrame,
      pauseFrame: pauseFrameOf(p.suspendSec, SR),
      truncatedIsMultipleOf128: truncatedFrame % 128 === 0,
    };
  });
  const frames = channels[0]?.length ?? 0;
  const haps = job.pauses.flatMap((p, j) =>
    p.haps.map((h) => {
      const expected = expectedFirstSounding(h.t);
      const observed = firstNonZero(channels, expected - ONSET_SEARCH_FRAMES, frames);
      const afterPauseFrame = observed >= pauses[j]!.pauseFrame;
      return {
        pause: j,
        t: h.t,
        tFrames: h.t * SR,
        startFrame: timeToSampleFrameRoundUp(h.t),
        expectedFirstSoundingFrame: expected,
        observedFirstSoundingFrame: observed,
        afterPauseFrame,
        agrees: observed === expected && afterPauseFrame,
      };
    }),
  );
  return {
    case: SUSPEND_CASE,
    rule:
      'pause frame = round_up_128(trunc(suspendSec * 44100)); a hap scheduled in a pause first ' +
      'sounds at its own start frame TimeToSampleFrame(t, RoundUp) (the next frame when t is on ' +
      'a frame), never at the pause frame',
    pauses,
    haps,
    agrees: haps.length > 0 && haps.every((h) => h.agrees),
  };
}

// ---- measure-feedback-echo

/** The extra frames each loop through a feedback cycle adds (`research/chromium.md`, Delay). */
export const FEEDBACK_EXTRA_FRAMES = 128;
/** How many echoes the evidence checks. */
const FEEDBACK_ECHOES = 4;
/** Zero frames that separate two bursts. */
const BURST_GAP_FRAMES = 256;

/** The first frame of every run of non-zero samples (any channel) that follows at least `gap`
 *  zero frames (or the start). */
export function burstOnsets(channels: Float32Array[], gap: number = BURST_GAP_FRAMES): number[] {
  const frames = channels[0]?.length ?? 0;
  const onsets: number[] = [];
  let zeros = gap;
  for (let i = 0; i < frames; i++) {
    if (channels.some((c) => c[i] !== 0)) {
      if (zeros >= gap) onsets.push(i);
      zeros = 0;
    } else {
      zeros++;
    }
  }
  return onsets;
}

/** The peak magnitude in `[from, to)` over every channel. */
export function peakIn(channels: Float32Array[], from: number, to: number): number {
  let peak = 0;
  for (const c of channels) {
    for (let i = Math.max(0, from); i < Math.min(to, c.length); i++) {
      peak = Math.max(peak, Math.abs(c[i]!));
    }
  }
  return peak;
}

export interface FeedbackEvidence {
  case: string;
  rule: string;
  delaytime: number;
  feedback: number;
  delayFrames: number;
  dryOnsetFrame: number;
  echoes: {
    k: number;
    expectedFrame: number;
    observedFrame: number;
    peakRatio: number;
    expectedPeakRatio: number;
    agrees: boolean;
  }[];
  agrees: boolean;
}

/** The evidence of the `measure-feedback-echo` case: its first hap's `delaytime`/`delayfeedback`,
 *  the delay in frames as the a-rate kernel computes it (`f32(f32(delaytime) * 44100)`), the
 *  dry burst's onset `t0`, and for k = 1..4 the onset of burst k against `t0 + k*D + (k-1)*128`
 *  plus its peak relative to the first echo (`feedback^(k-1)`, within 1e-6). */
export function feedbackEvidence(job: EngineJob, channels: Float32Array[]): FeedbackEvidence {
  const hap = job.pauses.flatMap((p) => p.haps)[0];
  const value = (hap ? job.values[hap.v] : undefined) as Record<string, unknown> | undefined;
  const delaytime = Number(value?.delaytime);
  const feedback = Number(value?.delayfeedback);
  const delayFrames = Math.fround(Math.fround(delaytime) * SR);
  const onsets = burstOnsets(channels);
  const dryOnsetFrame = onsets[0] ?? -1;
  const burst = (i: number) =>
    onsets[i] === undefined ? 0 : peakIn(channels, onsets[i]!, onsets[i + 1] ?? Infinity);
  const first = burst(1);
  const echoes = Array.from({ length: FEEDBACK_ECHOES }, (_, j) => {
    const k = j + 1;
    const expectedFrame = dryOnsetFrame + k * delayFrames + (k - 1) * FEEDBACK_EXTRA_FRAMES;
    const observedFrame = onsets[k] ?? -1;
    const peakRatio = first === 0 ? 0 : burst(k) / first;
    const expectedPeakRatio = Math.fround(feedback) ** (k - 1);
    return {
      k,
      expectedFrame,
      observedFrame,
      peakRatio,
      expectedPeakRatio,
      agrees:
        dryOnsetFrame >= 0 &&
        observedFrame === expectedFrame &&
        Math.abs(peakRatio - expectedPeakRatio) <= 1e-6,
    };
  });
  return {
    case: FEEDBACK_CASE,
    rule:
      'echo k of a burst at t0 through a delay of D frames with feedback g lands at ' +
      't0 + k*D + (k-1)*128 with amplitude g^(k-1) relative to echo 1 (the feedback input ' +
      "reads the delay's output of the previous render quantum)",
    delaytime,
    feedback,
    delayFrames,
    dryOnsetFrame,
    echoes,
    agrees: Number.isInteger(delayFrames) && echoes.every((e) => e.agrees),
  };
}

// ---- V8 math reference

/** One group of `v8-math.json`: the inputs and V8's outputs, all f64 bit strings. */
export interface MathReferenceGroup extends MathGroupInput {
  out: string[];
}

/** `v8-math.json`: UTF-8, `{"schema":1,"provenance":{…},"groups":[` then one group object per
 *  line (`{"id","fn","xs","ys","out"}`), then `]}` and one trailing newline. */
export function mathFileText(provenance: MathProvenance, groups: MathReferenceGroup[]): string {
  const lines = groups.map((g) =>
    JSON.stringify({ id: g.id, fn: g.fn, xs: g.xs, ys: g.ys, out: g.out }),
  );
  return (
    `{"schema":1,\n"provenance":${JSON.stringify(provenance)},\n"groups":[\n` +
    `${lines.join(',\n')}\n]}\n`
  );
}

/** Joins the grid with one evaluation's outputs: the same ids in the same order, one output per
 *  input. Throws when they disagree. */
export function mathGroups(
  inputs: MathGroupInput[],
  outputs: { id: string; out: string[] }[],
): MathReferenceGroup[] {
  if (outputs.length !== inputs.length) {
    throw new RangeError(`${outputs.length} output groups for ${inputs.length} input groups`);
  }
  return inputs.map((g, i) => {
    const o = outputs[i]!;
    if (o.id !== g.id) throw new RangeError(`output group ${i} is ${o.id}, expected ${g.id}`);
    if (o.out.length !== g.xs.length) {
      throw new RangeError(`${g.id}: ${o.out.length} outputs for ${g.xs.length} inputs`);
    }
    if (o.out.some((h) => !/^[0-9a-f]{16}$/.test(h))) {
      throw new RangeError(`${g.id}: an output is not a 16-digit f64 bit string`);
    }
    return { ...g, out: o.out };
  });
}

/** The first difference between two evaluations of the grid, or null when they are identical:
 *  V8 math must be deterministic. */
export function mathOutputsDiffer(
  a: { id: string; out: string[] }[],
  b: { id: string; out: string[] }[],
): string | null {
  if (a.length !== b.length) return `${a.length} groups against ${b.length}`;
  for (let i = 0; i < a.length; i++) {
    const [x, y] = [a[i]!, b[i]!];
    if (x.id !== y.id) return `group ${i}: ${x.id} against ${y.id}`;
    if (x.out.length !== y.out.length) return `${x.id}: ${x.out.length} against ${y.out.length}`;
    const j = x.out.findIndex((h, k) => h !== y.out[k]);
    if (j >= 0) return `${x.id}[${j}]: ${x.out[j]} against ${y.out[j]}`;
  }
  return null;
}

// ---- measured-floor mode

/** Chromium renders of a floor case. */
export const FLOOR_RUNS = 8;
/** How far above its committed floor a new render may be and still count as unchanged. */
export const FLOOR_MARGIN_DB = 6;
/** A floor above this is a defect, not a summing order (a sanity bound; replaced, with the
 *  evidence cited, if a legitimate floor ever measures above it). */
export const FLOOR_MAX_DB = -100;
/** The only micro cases whose Chromium reference varies run to run (hash-set summing order at a
 *  junction with three or more sounding inputs). A third needs the plan and this list changed. */
export const FLOOR_CASES: readonly string[] = ['vowel-formants', 'mod-lfo-note'];

/** `10*log10(sum((a - b)^2) / sum(a^2))` over every sample of every channel (`micro.rs`
 *  `residual_db`): `-Infinity` when equal, `Infinity` when only the reference `a` is silent, NaN
 *  when a sample is NaN. Throws when the shapes differ. */
export function residualDb(a: Float32Array[], b: Float32Array[]): number {
  if (a.length !== b.length || a.some((c, i) => c.length !== b[i]!.length)) {
    throw new RangeError('residualDb: the buffers have different shapes');
  }
  // With equal shapes `nullResidualDb` is null only for a NaN sample.
  return nullResidualDb(a, b) ?? NaN;
}

export interface CommittedFloorCase {
  /** The committed job file's bytes. */
  jobBytes: Uint8Array;
  /** The committed WAV's channels. */
  wav: Float32Array[];
  floorDb: number;
}

export type FloorSettlement =
  | { status: 'unchanged'; floorDb: number; worstDb: number }
  | { status: 'changed'; wav: Float32Array[]; floorDb: number; reason: string }
  | { error: string };

/** The measured-floor rule shared by the dry run and `--write`. `renders` are the `FLOOR_RUNS`
 *  Chromium renders of this run (render 1 first). The fresh floor is the largest residual of
 *  renders 2..8 against render 1, rounded to 0.01 dB; all 7 `-Infinity` (the case is
 *  deterministic) and a floor above `FLOOR_MAX_DB` are errors. The case is unchanged (the
 *  committed WAV and floor kept) iff it is committed, its job bytes equal `jobText`, and every
 *  render's residual against the committed WAV is at most `committed floorDb + FLOOR_MARGIN_DB`;
 *  otherwise it changes to render 1 and the fresh floor. */
export function settleFloorCase(
  jobText: string,
  renders: Float32Array[][],
  committed: CommittedFloorCase | null,
): FloorSettlement {
  if (renders.length !== FLOOR_RUNS) {
    return { error: `a floor case needs ${FLOOR_RUNS} renders, got ${renders.length}` };
  }
  const first = renders[0]!;
  const residuals = renders.slice(1).map((r) => residualDb(first, r));
  if (residuals.some(Number.isNaN)) return { error: 'a render has a NaN sample' };
  if (residuals.every((r) => r === -Infinity)) {
    return {
      error: `case is declared floor but its ${FLOOR_RUNS} renders are identical; remove the floor`,
    };
  }
  const floorDb = roundTo(Math.max(...residuals), 2);
  if (floorDb > FLOOR_MAX_DB) {
    return {
      error: `floor ${floorDb} dB: a floor above ${FLOOR_MAX_DB} dB is a defect, not a summing order`,
    };
  }
  if (committed === null) {
    return { status: 'changed', wav: first, floorDb, reason: 'first generation' };
  }
  if (!bytesEqual(committed.jobBytes, utf8(jobText))) {
    return { status: 'changed', wav: first, floorDb, reason: 'the job changed' };
  }
  const shapeOk = (r: Float32Array[]) =>
    r.length === committed.wav.length && r.every((c, i) => c.length === committed.wav[i]!.length);
  if (!renders.every(shapeOk)) {
    return { status: 'changed', wav: first, floorDb, reason: 'the render shape changed' };
  }
  const worstDb = Math.max(...renders.map((r) => residualDb(committed.wav, r)));
  const bound = committed.floorDb + FLOOR_MARGIN_DB;
  if (!(worstDb <= bound)) {
    return {
      status: 'changed',
      wav: first,
      floorDb,
      reason: `a render is ${roundTo(worstDb, 2)} dB from the committed WAV, over ${bound} dB`,
    };
  }
  return { status: 'unchanged', floorDb: committed.floorDb, worstDb };
}

// ---- measure-worklet-start

/** The measure case whose Chromium render becomes `evidence/measure-worklet-start.json`. */
export const WORKLET_START_CASE = 'measure-worklet-start';

/** The quantum a worklet source with `begin = t` first runs in: the smallest `q` whose start time
 *  `q * 128 / 44100` is strictly greater than `Math.fround(t)` (the f32 param value; JS
 *  `!(currentTime <= begin)`). */
export function expectedWorkletStartQuantum(t: number): number {
  const begin = Math.fround(t);
  let q = Math.max(0, Math.floor((begin * SR) / QUANTUM) - 1);
  while (!((q * QUANTUM) / SR > begin)) q++;
  return q;
}

export interface WorkletStartEvidence {
  case: string;
  rule: string;
  haps: {
    t: number;
    tF32: number;
    tFrames: number;
    offsetInQuantum: number;
    expectedQuantum: number;
    observedQuantum: number;
    agrees: boolean;
  }[];
  agrees: boolean;
}

/** The evidence of `measure-worklet-start`: for every hap, the quantum its first non-zero output
 *  sample falls in (searching from 256 frames before the expected quantum's first frame) against
 *  `expectedWorkletStartQuantum`. */
export function workletStartEvidence(
  job: EngineJob,
  channels: Float32Array[],
): WorkletStartEvidence {
  const frames = channels[0]?.length ?? 0;
  const haps = job.pauses
    .flatMap((p) => p.haps)
    .map((h) => {
      const tFrames = h.t * SR;
      const expectedQuantum = expectedWorkletStartQuantum(h.t);
      const first = firstNonZero(channels, expectedQuantum * QUANTUM - ONSET_SEARCH_FRAMES, frames);
      const observedQuantum = first < 0 ? -1 : Math.floor(first / QUANTUM);
      return {
        t: h.t,
        tF32: Math.fround(h.t),
        tFrames,
        offsetInQuantum: roundTo(tFrames - QUANTUM * Math.floor(tFrames / QUANTUM), 2),
        expectedQuantum,
        observedQuantum,
        agrees: observedQuantum === expectedQuantum,
      };
    });
  return {
    case: WORKLET_START_CASE,
    rule:
      'a worklet source with begin = t first runs (and first sounds) in the first quantum q ' +
      'with q * 128 / 44100 > f32(t) (JS !(currentTime <= begin) on the float32 param)',
    haps,
    agrees: haps.length > 0 && haps.every((h) => h.agrees),
  };
}

// ---- measure-worklet-teardown

/** The measure case whose Chromium render becomes `evidence/measure-worklet-teardown.json`. */
export const WORKLET_TEARDOWN_CASE = 'measure-worklet-teardown';

/** The frame a ladder's output is zero from when its source stops at `stopFrame`:
 *  `ceil(stopFrame / 128) * 128`, the engine's `worklet::first_absent_after_stop` (the source
 *  finishes in the quantum holding its last frame, the disconnect cascades in that quantum's
 *  post-render tasks, and the ladder sees no input from the next quantum). The capture uses a stop
 *  frame that is not a multiple of 128, where this rule and "the quantum after the one holding
 *  the stop frame" agree. */
export const ladderTeardownCutFrame = (stopFrame: number): number =>
  Math.ceil(stopFrame / QUANTUM) * QUANTUM;

/** Below this, the ring decayed on its own before the cut: the cut is not observed. */
const RING_MIN_PEAK = 1e-6;

export interface WorkletTeardownEvidence {
  case: string;
  rule: string;
  t: number;
  duration: number;
  release: number;
  stopFrame: number;
  /** The capture must avoid this: there the rules "ceil" and "quantum after" would differ. */
  stopFrameIsMultipleOf128: boolean;
  expectedCutFrame: number;
  observedCutFrame: number;
  ringPeakBeforeCut: number;
  agrees: boolean;
}

/** The evidence of `measure-worklet-teardown` from its one hap: the oscillator's stop frame
 *  `TimeToSampleFrame(t + duration + release + 0.01, RoundUp)` (`release` from the hap's value,
 *  required), the expected cut, the observed cut (one past the last non-zero frame of any
 *  channel) and the ring's peak over the 128 frames before it. */
export function workletTeardownEvidence(
  job: EngineJob,
  channels: Float32Array[],
): WorkletTeardownEvidence {
  const haps = job.pauses.flatMap((p) => p.haps);
  if (haps.length !== 1)
    throw new RangeError(`${WORKLET_TEARDOWN_CASE} needs one hap, got ${haps.length}`);
  const h = haps[0]!;
  const value = job.values[h.v] as Record<string, unknown> | undefined;
  const release = value?.release;
  if (typeof release !== 'number' || !Number.isFinite(release)) {
    throw new RangeError(`${WORKLET_TEARDOWN_CASE}: the hap must set a numeric release`);
  }
  const stopFrame = timeToSampleFrameRoundUp(h.t + h.duration + release + 0.01);
  const expectedCutFrame = ladderTeardownCutFrame(stopFrame);
  const frames = channels[0]?.length ?? 0;
  let last = -1;
  for (let i = frames - 1; i >= 0 && last < 0; i--) {
    if (channels.some((c) => c[i] !== 0)) last = i;
  }
  const observedCutFrame = last + 1;
  const ringPeakBeforeCut = peakIn(channels, observedCutFrame - QUANTUM, observedCutFrame);
  return {
    case: WORKLET_TEARDOWN_CASE,
    rule:
      "a ladder's output is zero from ceil(stopFrame / 128) * 128 (the quantum after its " +
      "source's stop frame), the ring before it kept",
    t: h.t,
    duration: h.duration,
    release,
    stopFrame,
    stopFrameIsMultipleOf128: stopFrame % QUANTUM === 0,
    expectedCutFrame,
    observedCutFrame,
    ringPeakBeforeCut,
    agrees:
      stopFrame % QUANTUM !== 0 &&
      observedCutFrame === expectedCutFrame &&
      observedCutFrame < frames &&
      ringPeakBeforeCut >= RING_MIN_PEAK,
  };
}

// ---- measure cases

/** A measure case: the evidence its Chromium render gives and the directory it is written to. */
export interface MeasureCase {
  dir: string;
  evidence: (job: EngineJob, channels: Float32Array[]) => { agrees: boolean };
}

/** Every measure case by name. */
export const MEASURE_CASES: ReadonlyMap<string, MeasureCase> = new Map([
  [SUSPEND_CASE, { dir: ENGINE_CORE_EVIDENCE_DIR, evidence: suspendEvidence }],
  [FEEDBACK_CASE, { dir: ENGINE_CORE_EVIDENCE_DIR, evidence: feedbackEvidence }],
  [WORKLET_START_CASE, { dir: ENGINE_EFFECTS_EVIDENCE_DIR, evidence: workletStartEvidence }],
  [WORKLET_TEARDOWN_CASE, { dir: ENGINE_EFFECTS_EVIDENCE_DIR, evidence: workletTeardownEvidence }],
]);

// ---- worklet determinism

/** The micro-case name prefixes `worklet-determinism.probe.ts` renders by default (every case
 *  that runs a worklet processor). */
export const WORKLET_CASE_PREFIXES: readonly string[] = [
  'supersaw-',
  'pulse-',
  'coarse-',
  'crush-',
  'shape-',
  'distort-',
  'transient-',
  'ladder-',
  'djf-',
  'lfo-',
  'env-',
  'tremolo-',
  'phaser-',
  'filter-lfo-',
  'mod-',
  'measure-worklet-',
];

export const isWorkletCase = (name: string): boolean =>
  WORKLET_CASE_PREFIXES.some((p) => name.startsWith(p));

export interface DeterminismCase {
  name: string;
  /** sha256 of each render's f32 samples, channel after channel. */
  hashes: string[];
  bitEqual: boolean;
  /** The largest residual of renders 2..n against render 1 (`null` when all are equal). */
  worstResidualDb: number | null;
  /** The first frame where any render differs from render 1 (`null` when all are equal). */
  firstDiffFrame: number | null;
}

const channelBytes = (channels: Float32Array[]): Uint8Array => {
  const out = new Uint8Array(channels.reduce((n, c) => n + c.byteLength, 0));
  let at = 0;
  for (const c of channels) {
    out.set(new Uint8Array(c.buffer, c.byteOffset, c.byteLength), at);
    at += c.byteLength;
  }
  return out;
};

/** Compares the repeated renders of one case. Throws on fewer than two renders or different
 *  shapes. */
export function determinismOf(name: string, renders: Float32Array[][]): DeterminismCase {
  if (renders.length < 2) throw new RangeError(`${name}: needs at least two renders`);
  const first = renders[0]!;
  const hashes = renders.map((r) => sha256(channelBytes(r)));
  const bitEqual = hashes.every((h) => h === hashes[0]);
  let worst: number | null = null;
  let firstDiff: number | null = null;
  for (const r of renders.slice(1)) {
    const db = residualDb(first, r);
    if (db !== -Infinity && (worst === null || db > worst || Number.isNaN(db))) worst = db;
    for (let ch = 0; ch < first.length; ch++) {
      const [a, b] = [first[ch]!, r[ch]!];
      for (let i = 0; i < a.length && (firstDiff === null || i < firstDiff); i++) {
        if (!Object.is(a[i], b[i])) {
          firstDiff = i;
          break;
        }
      }
    }
  }
  return {
    name,
    hashes,
    bitEqual,
    worstResidualDb: worst === null ? null : roundTo(worst, 2),
    firstDiffFrame: firstDiff,
  };
}

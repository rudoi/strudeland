/**
 * The render corpus manifest (`tests/render/corpus.json`): types and validation. `validateCorpus`
 * collects every problem (it never throws) against an injected `CorpusIo`, so tests run on fakes
 * and `render:check` on the real directory. Pure: no fs, Strudel, DOM or Playwright imports.
 */

import { readSongHeader } from '../project/song-header';
import type { SongStructure } from '../strudel/structure';
import type { SegmentLayout } from './contract';
import { GATE_NAMES, type GateName } from './gates/types';

export const MAX_WINDOW_BARS = 24;
/** `seam` needs a window longer than this many bars. */
const SEAM_MIN_WINDOW_BARS = 8;
const FIXTURE_NAME = /^[a-z0-9][a-z0-9-]*$/;

export type FixtureKind = 'external' | 'targeted';
export type RustSupport = 'supported' | 'unsupported';

export interface FixtureEntry {
  name: string;
  file: string;
  /** External fixtures use the render harness without adding committed engine references. */
  kind: FixtureKind;
  /** [from, to) in bars of `layout`. */
  windowBars: [number, number];
  layout: SegmentLayout;
  gates: GateName[];
  /** Minimum chunk count the plan must reach; required with (and only with) `seam`. */
  expectChunks?: number;
  /** The song uses a kabelsalat voice (00-module-map decision 4): twin runs in tolerance mode. */
  stochastic: boolean;
  /** The song calls `samples(`; `render:check` then passes the corpus samples. */
  usesSamples: boolean;
  /** Whether the Rust engine plays every hap (`render:check --engine rust` checks it agrees
   *  with the schedule-time detection; superdough ignores it). */
  rust: RustSupport;
  tags: string[];
  notes?: string;
  expect?: { unverifiable?: GateName[]; clip?: boolean };
}

export interface Corpus {
  schema: 1;
  samples: string[];
  fixtures: FixtureEntry[];
}

export interface CorpusIo {
  /** Song text of `tests/render/corpus/<file>`, or null if missing. */
  readSong(file: string): string | null;
  /** Whether `tests/render/corpus/samples/<file>` exists. */
  sampleExists(file: string): boolean;
  /** Every `.js` file name in `tests/render/corpus/`. */
  listSongs(): string[];
}

export interface CorpusError {
  /** Like `fixtures[3].windowBars`; '' for the manifest itself. */
  path: string;
  message: string;
}

const TOP_KEYS = ['schema', 'samples', 'fixtures'];
const FIXTURE_KEYS = [
  'name',
  'file',
  'kind',
  'windowBars',
  'layout',
  'gates',
  'expectChunks',
  'stochastic',
  'usesSamples',
  'rust',
  'tags',
  'notes',
  'expect',
];
const EXPECT_KEYS = ['unverifiable', 'clip'];

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const isStringArray = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((x) => typeof x === 'string');
const isGate = (v: unknown): v is GateName =>
  typeof v === 'string' && (GATE_NAMES as readonly string[]).includes(v);

/** Validate a parsed `corpus.json`. `corpus` is null whenever `errors` is non-empty. */
export function validateCorpus(
  raw: unknown,
  io: CorpusIo,
): { corpus: Corpus | null; errors: CorpusError[] } {
  const errors: CorpusError[] = [];
  const err = (path: string, message: string) => errors.push({ path, message });
  const unknownKeys = (obj: Record<string, unknown>, allowed: string[], path: string) => {
    for (const k of Object.keys(obj)) if (!allowed.includes(k)) err(path, `unknown key '${k}'`);
  };

  if (!isObject(raw) || raw.schema !== 1) {
    err('schema', 'schema must be 1');
    if (!isObject(raw)) return { corpus: null, errors };
  }
  unknownKeys(raw, TOP_KEYS, '');

  // samples
  const samples = raw.samples;
  if (!isStringArray(samples)) {
    err('samples', 'samples must be an array of file names');
  } else {
    const seen = new Set<string>();
    samples.forEach((file, i) => {
      if (seen.has(file)) err(`samples[${i}]`, `duplicate sample '${file}'`);
      seen.add(file);
      if (!io.sampleExists(file)) err(`samples[${i}]`, `sample '${file}' not found`);
    });
  }
  const haveSamples = isStringArray(samples) && samples.length > 0;

  // fixtures
  const fixtures = raw.fixtures;
  const used = new Set<string>();
  const names = new Set<string>();
  if (!Array.isArray(fixtures) || fixtures.length === 0) {
    err('fixtures', 'fixtures must be a non-empty array');
  } else {
    fixtures.forEach((fx, i) => {
      const path = `fixtures[${i}]`;
      if (!isObject(fx)) {
        err(path, 'fixture must be an object');
        return;
      }
      validateFixture(fx, path);
    });
  }

  // every song file is used
  for (const file of io.listSongs()) {
    if (!used.has(file)) err('fixtures', `song '${file}' is not in corpus.json`);
  }

  return { corpus: errors.length === 0 ? (raw as unknown as Corpus) : null, errors };

  function validateFixture(fx: Record<string, unknown>, path: string): void {
    unknownKeys(fx, FIXTURE_KEYS, path);

    // 2. name
    if (typeof fx.name !== 'string' || !FIXTURE_NAME.test(fx.name)) {
      err(`${path}.name`, `name must match ${FIXTURE_NAME}`);
    } else if (names.has(fx.name)) {
      err(`${path}.name`, `duplicate fixture name '${fx.name}'`);
    } else {
      names.add(fx.name);
    }

    // 7. kind, tags, notes
    if (fx.kind !== 'external' && fx.kind !== 'targeted') {
      err(`${path}.kind`, `kind must be 'external' or 'targeted'`);
    }
    if (!isStringArray(fx.tags)) err(`${path}.tags`, 'tags must be an array of strings');
    if (fx.notes !== undefined && typeof fx.notes !== 'string') {
      err(`${path}.notes`, 'notes must be a string');
    }

    // layout
    const layout = fx.layout;
    const layoutOk = layout === 'linear' || layout === 'clip';
    if (!layoutOk) err(`${path}.layout`, `layout must be 'linear' or 'clip'`);

    // 3. file, meta, structure
    let song: string | null = null;
    let structure: SongStructure | null = null;
    if (typeof fx.file !== 'string' || !fx.file.endsWith('.js')) {
      err(`${path}.file`, 'file must be a .js file name');
    } else {
      used.add(fx.file);
      song = io.readSong(fx.file);
      if (song === null) {
        err(`${path}.file`, `song file '${fx.file}' not found`);
      } else {
        const header = readSongHeader(song);
        if (header.ok) structure = header.structure;
        else err(`${path}.file`, `meta: ${header.error.message}`);
      }
    }

    // 4. windowBars
    const w = fx.windowBars;
    let windowLen: number | null = null;
    if (
      !Array.isArray(w) ||
      w.length !== 2 ||
      !Number.isInteger(w[0]) ||
      !Number.isInteger(w[1]) ||
      (w[0] as number) < 0 ||
      (w[0] as number) >= (w[1] as number)
    ) {
      err(`${path}.windowBars`, 'windowBars must be [from, to] integers with 0 <= from < to');
    } else {
      const [from, to] = w as [number, number];
      windowLen = to - from;
      if (windowLen > MAX_WINDOW_BARS) {
        err(`${path}.windowBars`, `window longer than ${MAX_WINDOW_BARS} bars`);
      }
      if (structure !== null && layoutOk) {
        if (structure.sections.length > 0) {
          if (layout === 'linear' && to > structure.totalBars) {
            err(`${path}.windowBars`, `window ends after the song (${structure.totalBars} bars)`);
          }
          if (layout === 'clip') {
            if (structure.loopIndex === null) {
              err(`${path}.layout`, `layout 'clip' needs a loop section`);
            } else if (to > structure.clipBars) {
              err(`${path}.windowBars`, `window ends after the song (${structure.clipBars} bars)`);
            }
          }
        } else if (layout === 'clip') {
          err(`${path}.layout`, `layout 'clip' needs a loop section`);
        }
      }
    }

    // 5. gates, expectChunks
    const gates = fx.gates;
    const gateList: GateName[] = [];
    if (!Array.isArray(gates) || gates.length === 0) {
      err(`${path}.gates`, 'gates must be a non-empty array');
    } else {
      gates.forEach((g, j) => {
        if (!isGate(g)) err(`${path}.gates[${j}]`, `unknown gate '${String(g)}'`);
        else if (gateList.includes(g)) err(`${path}.gates[${j}]`, `duplicate gate '${g}'`);
        else gateList.push(g);
      });
    }
    const hasSeam = gateList.includes('seam');
    const ec = fx.expectChunks;
    if (hasSeam) {
      const ecOk = Number.isInteger(ec) && (ec as number) >= 2;
      const windowOk = windowLen === null || windowLen > SEAM_MIN_WINDOW_BARS;
      if (!ecOk || !windowOk) {
        err(
          `${path}.gates`,
          `seam needs a window over ${SEAM_MIN_WINDOW_BARS} bars and expectChunks >= 2`,
        );
      }
    } else if (ec !== undefined) {
      err(`${path}.expectChunks`, 'expectChunks needs the seam gate');
    }

    // 6. stochastic, usesSamples
    if (typeof fx.stochastic !== 'boolean') {
      err(`${path}.stochastic`, 'stochastic must be a boolean');
    }
    if (typeof fx.usesSamples !== 'boolean') {
      err(`${path}.usesSamples`, 'usesSamples must be a boolean');
    } else {
      if (song !== null && fx.usesSamples !== song.includes('samples(')) {
        err(`${path}.usesSamples`, 'usesSamples does not match the song');
      }
      if (fx.usesSamples && !haveSamples) {
        err(`${path}.usesSamples`, 'usesSamples needs a non-empty samples list');
      }
    }

    // rust
    if (fx.rust !== 'supported' && fx.rust !== 'unsupported') {
      err(`${path}.rust`, `rust must be 'supported' or 'unsupported'`);
    }

    // 7. expect
    if (fx.expect !== undefined) {
      const ex = fx.expect;
      if (!isObject(ex)) {
        err(`${path}.expect`, 'expect must be an object');
      } else {
        unknownKeys(ex, EXPECT_KEYS, `${path}.expect`);
        if (ex.unverifiable !== undefined) {
          if (!Array.isArray(ex.unverifiable)) {
            err(`${path}.expect.unverifiable`, 'expect.unverifiable must be an array');
          } else {
            ex.unverifiable.forEach((g, j) => {
              if (!gateList.includes(g as GateName)) {
                err(
                  `${path}.expect.unverifiable[${j}]`,
                  `expected-unverifiable gate '${String(g)}' is not in gates`,
                );
              }
            });
          }
        }
        if (ex.clip !== undefined && typeof ex.clip !== 'boolean') {
          err(`${path}.expect.clip`, 'expect.clip must be a boolean');
        }
      }
    }
  }
}

/** `corpus.samples` as the `RenderRequest.samples` map (file stem to file name). */
export function samplesMap(corpus: Corpus): Record<string, string> {
  return Object.fromEntries(corpus.samples.map((f) => [f.replace(/\.wav$/, ''), f]));
}

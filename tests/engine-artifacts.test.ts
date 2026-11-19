/**
 * Staleness and budget checks of the committed engine references
 * (`crates/strudeland-engine/tests/refs/`, written only by `bun run engine:refs --write`). No
 * browser: hashes, sizes, provenance and file sets only.
 */

import { describe, expect, test } from 'bun:test';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import corpusJson from './render/corpus.json';
import { validateEngineJob, type EngineJob } from '../src/lib/render/engine-job';
import { decodeWav } from '../src/lib/render/wav';
import {
  caseKind,
  fixtureCase,
  isFresh,
  isMathFresh,
  mathProvenanceOf,
  MICRO_DIR,
  provenanceOf,
  type MathProvenance,
} from '../scripts/engine/provenance';
import {
  budgetProblems,
  caseFiles,
  fixtureHasRefs,
  FLOOR_CASES,
  FLOOR_MAX_DB,
  INDEX_FILE,
  MATH_CASE,
  MATH_FILE,
  MATH_MAX_BYTES,
  microJobFile,
  microWavFile,
  REFS_DIR,
  THRESHOLDS_FILE,
  type RefEnvelope,
  type RefsIndex,
} from '../scripts/engine/refs-lib';
import type { FixtureEntry } from '../src/lib/render/corpus';
import type { Provenance } from '../src/lib/render/engine-job';
import { generateVectors, readVectors, VECTORS_DIR } from '../scripts/engine/vectors';
import { sha256 } from '../scripts/sha256';
import { isBinaryFn } from '../src/render/math';
import { VERIFICATION_SET } from '../scripts/engine/v8-math-cases';

const ROOT = resolve(import.meta.dir, '..');
const REFS = join(ROOT, REFS_DIR);
const REGENERATE = 'run `bun run engine:refs --write`';

const index = JSON.parse(readFileSync(join(REFS, INDEX_FILE), 'utf8')) as RefsIndex;
interface Threshold {
  maxResidualDb: number;
  evidence: string;
  floor?: { runs: number; reason: string };
  platform?: string;
}
const thresholds = JSON.parse(readFileSync(join(REFS, THRESHOLDS_FILE), 'utf8')) as {
  schema: number;
  cases: Record<string, Threshold>;
  fixtures: Record<string, unknown>;
};
const EVIDENCE = /^plans\/(0010-rust-engine-core|0011-engine-effects)\/evidence\/.+\.(json|md)$/;
/** The micro cases judged on macOS only (decision 16); the compressor stage adds them. */
const PLATFORM_CASES: string[] = [
  'compressor-fresh',
  'compressor-params',
  'compressor-stereo',
  'compressor-warmup',
];
const fixtures = (corpusJson as unknown as { fixtures: FixtureEntry[] }).fixtures;

const refFiles = (): string[] =>
  readdirSync(REFS).filter((f) => f !== INDEX_FILE && f !== THRESHOLDS_FILE);
const bytesOf = (f: string) => new Uint8Array(readFileSync(join(REFS, f)));
const jsonOf = <T>(f: string): T => JSON.parse(readFileSync(join(REFS, f), 'utf8')) as T;
const microSources = (): string[] =>
  readdirSync(join(ROOT, MICRO_DIR))
    .filter((f) => f.endsWith('.js'))
    .map((f) => f.slice(0, -3))
    .sort();
const cases = Object.entries(index.cases);

describe('engine refs index', () => {
  test('schema 2', () => {
    expect(index.schema).toBe(2);
  });

  test('every listed file exists with its recorded sha256 and size', () => {
    for (const [name, rec] of Object.entries(index.files)) {
      const path = join(REFS, name);
      expect(existsSync(path), `${name} is missing: ${REGENERATE}`).toBe(true);
      const bytes = bytesOf(name);
      expect(bytes.length, `${name} size: ${REGENERATE}`).toBe(rec.bytes);
      expect(sha256(bytes), `${name} sha256: ${REGENERATE}`).toBe(rec.sha256);
    }
  });

  test('no file is unlisted', () => {
    const unlisted = refFiles().filter((f) => !(f in index.files));
    expect(unlisted, `unlisted ref files: ${REGENERATE}`).toEqual([]);
  });

  test('every file belongs to exactly one case and every case file is listed', () => {
    const owners = new Map<string, string[]>();
    for (const [name, c] of cases) {
      for (const f of caseFiles(c)) owners.set(f, [...(owners.get(f) ?? []), name]);
    }
    for (const f of Object.keys(index.files)) {
      expect(owners.get(f)?.length ?? 0, `${f} belongs to no case or several`).toBe(1);
    }
    for (const f of owners.keys()) expect(f in index.files, `${f} not listed`).toBe(true);
  });

  test('case kinds and shapes agree with their names', () => {
    for (const [name, c] of cases) {
      expect(c.kind, name).toBe(caseKind(name));
      if (c.kind === 'math') {
        expect(name).toBe(MATH_CASE);
        expect(c).toMatchObject({ jobs: [], envelopes: null, wav: null, data: MATH_FILE });
        expect('floorDb' in c, name).toBe(false);
      } else if (c.kind === 'micro') {
        expect(c.jobs, name).toEqual([microJobFile(name)]);
        expect(c.wav, name).toBe(microWavFile(name));
        expect(c.envelopes, name).toBeNull();
        expect(c.data, name).toBeNull();
      } else {
        expect(c.data, name).toBeNull();
        expect('floorDb' in c, name).toBe(false);
        expect(c.wav, name).toBeNull();
        expect(c.jobs.length, name).toBeGreaterThan(0);
        expect(c.envelopes?.length, name).toBe(c.jobs.length);
        for (const [k, j] of c.jobs.entries()) {
          expect(c.envelopes![k], name).toBe(j.replace(/\.job\.json$/, '.ref.json'));
          expect(j.startsWith(`${name}--`), name).toBe(true);
        }
      }
    }
  });
});

describe('engine refs provenance', () => {
  test('every job file embeds its case provenance', () => {
    for (const [, c] of cases) {
      for (const j of c.jobs) {
        expect(jsonOf<EngineJob>(j).provenance, `${j}: ${REGENERATE}`).toEqual(
          c.provenance as Provenance,
        );
      }
    }
  });

  test('every case is fresh (all fields but platform)', () => {
    for (const [name, c] of cases) {
      const { fresh, differs } =
        c.kind === 'math'
          ? isMathFresh(c.provenance as MathProvenance, mathProvenanceOf(ROOT))
          : isFresh(c.provenance as Provenance, provenanceOf(ROOT, name));
      expect(fresh, `${name} is stale (${differs.join(', ')} differ): ${REGENERATE}`).toBe(true);
    }
  });
});

describe('engine refs budgets', () => {
  test('WAVs <= 40 MiB, jobs plus envelopes <= 8 MiB, v8-math.json <= 1 MiB, micro WAV lengths', () => {
    const files = new Map(refFiles().map((f) => [f, bytesOf(f)]));
    const frames = new Map<string, number>();
    for (const [name, c] of cases) {
      if (c.wav) frames.set(name, decodeWav(files.get(c.wav)!).channels[0]!.length);
    }
    expect(budgetProblems(files, frames)).toEqual([]);
  });
});

describe('micro cases', () => {
  const micro = cases.filter(([, c]) => c.kind === 'micro').map(([n]) => n);

  test('every micro source has refs and a threshold, and vice versa', () => {
    const sources = microSources();
    expect(sources.length).toBeGreaterThan(0);
    expect([...micro].sort(), REGENERATE).toEqual(sources);
    expect(Object.keys(thresholds.cases).sort()).toEqual(sources);
  });

  test('every job is valid and its WAV is stereo f32 at 44100 with the job frames', () => {
    for (const name of micro) {
      const job = jsonOf<EngineJob>(microJobFile(name));
      expect(validateEngineJob(job).errors, name).toEqual([]);
      const wav = decodeWav(bytesOf(microWavFile(name)));
      expect(wav.format, name).toBe('f32');
      expect(wav.sampleRate, name).toBe(44100);
      expect(wav.channels.length, name).toBe(2);
      expect(wav.channels[0]!.length, name).toBe(job.frames);
    }
  });

  test('every threshold cites an existing evidence file (schema 2, both plan directories)', () => {
    expect(thresholds.schema).toBe(2);
    for (const [name, t] of Object.entries(thresholds.cases)) {
      expect(t.maxResidualDb, name).toBeLessThan(0);
      expect(t.evidence, name).toMatch(EVIDENCE);
      expect(existsSync(join(ROOT, t.evidence)), `${name}: ${t.evidence}`).toBe(true);
    }
    expect('plans/0011-engine-effects/evidence/x.json').toMatch(EVIDENCE);
    expect('plans/0012-other/evidence/x.json').not.toMatch(EVIDENCE);
  });

  test('floor cases: declared iff measured, one of FLOOR_CASES, above their floor', () => {
    for (const [name, t] of Object.entries(thresholds.cases)) {
      const c = index.cases[name];
      expect(t.floor !== undefined, `${name}: floor iff floorDb`).toBe(c?.floorDb !== undefined);
      if (!t.floor) continue;
      expect(FLOOR_CASES, name).toContain(name);
      expect(t.floor.runs, name).toBe(8);
      const m = /\((\d+) sounding inputs\)$/.exec(t.floor.reason);
      expect(Number(m?.[1] ?? 0), `${name}: reason`).toBeGreaterThanOrEqual(3);
      expect(c!.floorDb!, name).toBeLessThanOrEqual(FLOOR_MAX_DB);
      expect(t.maxResidualDb, name).toBeGreaterThan(c!.floorDb!);
    }
  });

  test.each(['vowel-formants', 'mod-lfo-note'])('%s is a declared floor case', (name) => {
    const t = thresholds.cases[name];
    const c = index.cases[name];
    expect(FLOOR_CASES).toContain(name);
    expect(t?.floor?.runs).toBe(8);
    const m = /\((\d+) sounding inputs\)$/.exec(t?.floor?.reason ?? '');
    expect(Number(m?.[1] ?? 0)).toBeGreaterThanOrEqual(3);
    expect(typeof c?.floorDb).toBe('number');
    expect(c!.floorDb!).toBeLessThanOrEqual(FLOOR_MAX_DB);
    expect(t!.maxResidualDb).toBeGreaterThan(c!.floorDb!);
    expect(t!.maxResidualDb).toBeLessThan(0);
  });

  test('final form: the floor cases are vowel-formants and mod-lfo-note', () => {
    expect([...FLOOR_CASES].sort()).toEqual(['mod-lfo-note', 'vowel-formants']);
  });

  test('final form: every threshold key is a known one (no case is skipped)', () => {
    const known = ['evidence', 'expectErrors', 'floor', 'maxResidualDb', 'platform'];
    for (const [name, t] of Object.entries(thresholds.cases)) {
      for (const k of Object.keys(t)) expect(known, `${name}: ${k}`).toContain(k);
    }
  });

  test('the floor cases are exactly the declared ones', () => {
    const floors = Object.entries(thresholds.cases)
      .filter(([, t]) => t.floor !== undefined)
      .map(([n]) => n)
      .sort();
    expect(floors).toEqual([...FLOOR_CASES].sort());
  });

  test('platform gating marks exactly the compressor cases, as "macos"', () => {
    const gated = Object.entries(thresholds.cases)
      .filter(([, t]) => t.platform !== undefined)
      .map(([n, t]) => {
        expect(t.platform, n).toBe('macos');
        return n;
      });
    expect(gated.sort()).toEqual(PLATFORM_CASES);
  });

  test('the fixtures section has the four tolerances', () => {
    for (const k of ['floorDb', 'windowMeanDb', 'windowP95Db', 'peakDb']) {
      expect(typeof thresholds.fixtures[k], k).toBe('number');
    }
  });

  test('every threshold is calibrated: no interim or placeholder marker is left', () => {
    for (const [name, t] of Object.entries(thresholds.cases)) {
      expect(Object.keys(t).sort(), name).not.toContain('interim');
      expect(Object.keys(t).sort(), name).not.toContain('placeholder');
    }
    expect(Object.keys(thresholds.fixtures)).not.toContain('placeholder');
    expect(Object.keys(thresholds.fixtures)).not.toContain('interim');
    const evidence = thresholds.fixtures.evidence as string;
    expect(evidence).toMatch(/^plans\/0010-rust-engine-core\/evidence\/.+\.json$/);
    expect(existsSync(join(ROOT, evidence)), evidence).toBe(true);
  });
});

describe('fixture refs', () => {
  const fixtureCases = cases.filter(([, c]) => c.kind === 'fixture');

  test('every fixture case is a corpus fixture that has refs', () => {
    for (const [name] of fixtureCases) {
      const f = fixtures.find((x) => fixtureCase(x.name) === name);
      expect(f, name).toBeDefined();
      expect(fixtureHasRefs(f!), name).toBe(true);
    }
  });

  test('every fixture with refs has its jobs and envelopes', () => {
    const withRefs = fixtures.filter(fixtureHasRefs).map((f) => fixtureCase(f.name));
    expect(fixtureCases.map(([name]) => name).sort()).toEqual(withRefs.sort());
    for (const [name, c] of fixtureCases) {
      expect(c.jobs.length, name).toBeGreaterThan(0);
      expect(c.envelopes?.length, name).toBe(c.jobs.length);
      for (const [k, j] of c.jobs.entries()) {
        expect(existsSync(join(REFS, j)), j).toBe(true);
        expect(c.envelopes![k], j).toBe(j.replace(/\.job\.json$/, '.ref.json'));
        expect(existsSync(join(REFS, c.envelopes![k]!)), c.envelopes![k]).toBe(true);
      }
    }
  });

  test('noise-seed is the only unsupported fixture kept with refs', () => {
    const kept = fixtures.filter((f) => f.rust === 'unsupported' && fixtureHasRefs(f));
    expect(kept.map((f) => f.name)).toEqual(['noise-seed']);
    expect(fixtures.find((f) => f.name === 'duck')?.rust).toBe('supported');
  });

  test('the new synth and effect fixtures have refs', () => {
    const names = fixtureCases.map(([name]) => name);
    for (const f of [
      'voice-osc',
      'voice-mod',
      'voice-noise',
      'voice-filter',
      'voice-level',
      'fx-delay',
      'fx-room',
      'duck',
      'worklets',
      'song-linear',
      'song-clip',
      'voice-worklets',
      'fx-native',
      'fx-mod',
      'fx-bus',
      'fx-chain',
    ]) {
      expect(names, f).toContain(fixtureCase(f));
    }
  });

  test('every job is valid and its envelope matches it', () => {
    for (const [name, c] of fixtureCases) {
      for (const [k, j] of c.jobs.entries()) {
        const job = jsonOf<EngineJob>(j);
        expect(validateEngineJob(job).errors, j).toEqual([]);
        const env = jsonOf<RefEnvelope>(c.envelopes![k]!);
        expect(env.schema, j).toBe(1);
        expect(env.windowFrames, j).toBe(441);
        expect(env.frames, j).toBe(job.frames);
        expect(env.channels, j).toBe(job.channels.count);
        expect(env.rmsDb.length, j).toBe(env.channels);
        expect(env.peak.length, j).toBe(env.channels);
        for (const w of env.rmsDb) expect(w.length, j).toBe(Math.ceil(env.frames / 441));
      }
      expect(
        c.jobs.some((j) => j.includes('--mix-')),
        name,
      ).toBe(true);
    }
  });

  test('stems variants exist exactly for fixtures with the stems gate', () => {
    for (const [name, c] of fixtureCases) {
      const f = fixtures.find((x) => fixtureCase(x.name) === name)!;
      const stems = c.jobs.filter((j) => j.includes('--stems-')).length;
      if (f.gates.includes('stems')) expect(stems, name).toBe(c.jobs.length / 2);
      else expect(stems, name).toBe(0);
    }
  });
});

describe('V8 math reference', () => {
  const entry = index.cases[MATH_CASE];
  const text = readFileSync(join(REFS, MATH_FILE), 'utf8');
  const ref = JSON.parse(text) as {
    schema: number;
    provenance: MathProvenance;
    groups: { id: string; fn: string; xs: string[]; ys: string[] | null; out: string[] }[];
  };

  test('listed in the index, at most 1 MiB', () => {
    expect(entry?.kind).toBe('math');
    expect(MATH_FILE in index.files).toBe(true);
    expect(index.files[MATH_FILE]!.bytes).toBeLessThanOrEqual(MATH_MAX_BYTES);
  });

  test('groups are well formed', () => {
    expect(ref.schema).toBe(1);
    expect(text.endsWith(']}\n')).toBe(true);
    const hex = /^[0-9a-f]{16}$/;
    expect(new Set(ref.groups.map((g) => g.id)).size).toBe(ref.groups.length);
    for (const g of ref.groups) {
      expect(g.out.length, g.id).toBe(g.xs.length);
      expect(g.ys === null, g.id).toBe(!isBinaryFn(g.fn));
      if (g.ys) expect(g.ys.length, g.id).toBe(g.xs.length);
      for (const h of [...g.xs, ...(g.ys ?? []), ...g.out]) expect(h, g.id).toMatch(hex);
    }
  });

  test('has groups for every function the host runs natively and for **', () => {
    const fns = new Set(ref.groups.map((g) => g.fn));
    for (const fn of [...VERIFICATION_SET, 'powOp']) expect(fns.has(fn), fn).toBe(true);
    for (const fn of VERIFICATION_SET) {
      expect(
        ref.groups.some((g) => g.id === `${fn}-main` && g.fn === fn),
        `${fn}-main`,
      ).toBe(true);
      expect(
        ref.groups.some((g) => g.id === `${fn}-special` && g.fn === fn),
        `${fn}-special`,
      ).toBe(true);
    }
  });

  test('its provenance equals the index entry and is fresh', () => {
    expect(ref.provenance).toEqual(entry!.provenance as MathProvenance);
    const { fresh, differs } = isMathFresh(ref.provenance, mathProvenanceOf(ROOT));
    expect(fresh, `stale (${differs.join(', ')}): ${REGENERATE}`).toBe(true);
  });

  test('v8-math-choice.json covers the ten functions with a chosen rate of 1', () => {
    const path = join(ROOT, 'plans/0011-engine-effects/evidence/v8-math-choice.json');
    expect(existsSync(path)).toBe(true);
    const choice = JSON.parse(readFileSync(path, 'utf8')) as {
      functions: Record<
        string,
        {
          samples: number;
          libm: { rate: number };
          std: { rate: number };
          port: { rate: number } | null;
          chosen: string;
        }
      >;
    };
    expect(Object.keys(choice.functions).sort()).toEqual(
      ['cos', 'cosh', 'exp', 'expm1', 'log1p', 'pow', 'round', 'sin', 'tanh', 'to_int32'].sort(),
    );
    for (const [name, f] of Object.entries(choice.functions)) {
      expect(f.chosen, name).not.toBe('std');
      expect(f.samples, name).toBeGreaterThan(0);
      // the host pow and tanh are judged on macOS only, where the reference was generated
      if (f.chosen === 'host' && process.platform !== 'darwin') continue;
      const rate = f.chosen === 'libm' ? f.libm.rate : f.port?.rate;
      expect(rate, `${name}: chosen ${f.chosen}`).toBe(1);
    }
    expect(choice.functions.pow!.chosen).toBe('host');
    expect(choice.functions.tanh!.chosen).toBe('host');
  });
});

describe('engine vectors', () => {
  const REGENERATE_VECTORS = 'run `bun run engine:vectors --write`';

  test('the committed vectors are exactly what the generator writes', () => {
    const committed = readVectors(join(ROOT, VECTORS_DIR));
    const generated = generateVectors();
    expect([...committed.keys()].sort(), `vector files: ${REGENERATE_VECTORS}`).toEqual(
      [...generated.keys()].sort(),
    );
    const decoder = new TextDecoder();
    for (const [name, text] of generated) {
      expect(decoder.decode(committed.get(name)), `${name} is stale: ${REGENERATE_VECTORS}`).toBe(
        text,
      );
    }
  });
});

import { fileURLToPath } from 'node:url';
import { describe, expect, test } from 'bun:test';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import type { RenderRequest } from '../src/lib/render/contract';
import { validateCorpus, type Corpus, type CorpusIo } from '../src/lib/render/corpus';
import type { GateResult } from '../src/lib/render/gates/types';
import {
  judgeMutation,
  MUTATIONS,
  validateMutations,
  type MutationSpec,
} from '../scripts/render/mutations';

const DIR = fileURLToPath(new URL('./render/corpus/', import.meta.url));
const MANIFEST = fileURLToPath(new URL('./render/corpus.json', import.meta.url));
const fsIo: CorpusIo = {
  readSong: (f) => (existsSync(DIR + f) ? readFileSync(DIR + f, 'utf8') : null),
  sampleExists: (f) => existsSync(`${DIR}samples/${f}`),
  listSongs: () => readdirSync(DIR).filter((f) => f.endsWith('.js')),
};
const realCorpus = (): Corpus => {
  const { corpus, errors } = validateCorpus(JSON.parse(readFileSync(MANIFEST, 'utf8')), fsIo);
  expect(errors).toEqual([]);
  return corpus!;
};

const spec = (name: string): MutationSpec => MUTATIONS.find((m) => m.name === name)!;
const result = (verdict: GateResult['verdict'], reasons: string[] = []): GateResult => ({
  gate: 'stems',
  verdict,
  measurements: {},
  reasons,
  warnings: [],
});

const REQ: RenderRequest = {
  code: 'x',
  layout: 'linear',
  fromBar: 8,
  toBar: 16,
  overlapBars: 1,
  lookbackSec: 6,
  mode: 'mix',
  sampleRate: 44100,
};

describe('judgeMutation', () => {
  const s = spec('stems-orbit-dropped');
  test('caught when the mutated gate fails', () => {
    expect(judgeMutation(s, result('pass'), result('fail', ['orbit 4 missing in bar 1']))).toEqual({
      name: 'stems-orbit-dropped',
      fixture: 'stems-5',
      gate: 'stems',
      status: 'caught',
      detail: 'orbit 4 missing in bar 1',
    });
  });
  test('missed when the mutated gate passes or is unverifiable', () => {
    for (const v of ['pass', 'unverifiable'] as const) {
      const m = judgeMutation(s, result('pass'), result(v));
      expect(m.status).toBe('missed');
      expect(m.detail).toBe(`gate stems returned ${v}; the known defect was not detected`);
    }
  });
  test('skipped when the control did not pass or did not run', () => {
    for (const v of ['fail', 'unverifiable'] as const) {
      expect(judgeMutation(s, result(v), result('fail'))).toMatchObject({
        status: 'skipped',
        detail: 'control did not pass',
      });
    }
    expect(judgeMutation(s, undefined, result('fail'))).toMatchObject({
      status: 'skipped',
      detail: 'control did not run',
    });
  });
  test('error without a mutated result, or with a render error', () => {
    expect(judgeMutation(s, result('pass'), undefined)).toMatchObject({
      status: 'error',
      detail: 'the mutated render produced no gate result',
    });
    expect(
      judgeMutation(s, result('pass'), undefined, 'mutated render failed: eval: x'),
    ).toMatchObject({
      status: 'error',
      detail: 'mutated render failed: eval: x',
    });
  });
});

describe('validateMutations', () => {
  test('the real corpus carries every mutation fixture and gate', () => {
    expect(validateMutations(realCorpus())).toEqual([]);
  });
  test('a missing fixture or gate is a problem', () => {
    const c = realCorpus();
    const without: Corpus = { ...c, fixtures: c.fixtures.filter((f) => f.name !== 'pad-seam') };
    expect(validateMutations(without)).toEqual([
      `mutation 'seam-lookback-0': fixture 'pad-seam' is not in the corpus`,
    ]);
    const noGate: Corpus = {
      ...c,
      fixtures: c.fixtures.map((f) =>
        f.name === 'bass-line' ? { ...f, gates: ['twin', 'mix'] } : f,
      ),
    };
    expect(validateMutations(noGate)).toEqual([
      `mutation 'bass-dropped': fixture 'bass-line' does not list gate 'bass'`,
    ]);
  });
  test('duplicate names are problems', () => {
    const c = realCorpus();
    const s = spec('bass-dropped');
    expect(validateMutations(c, [s, s])).toEqual([`mutation 'bass-dropped': duplicate name`]);
  });
});

describe('MUTATIONS', () => {
  test('the fixed set', () => {
    expect(MUTATIONS.map((m) => [m.name, m.fixture, m.gate])).toEqual([
      ['seam-lookback-0', 'pad-seam', 'seam'],
      ['stems-orbit-dropped', 'stems-5', 'stems'],
      ['bass-dropped', 'bass-line', 'bass'],
    ]);
  });
  test('seam-lookback-0 zeroes the lookback and keeps the rest', () => {
    expect(spec('seam-lookback-0').apply(REQ)).toEqual({ ...REQ, lookbackSec: 0 });
  });
  test('stems-orbit-dropped drops orbit 4 in bars 1, 4 and 6', () => {
    const stemsReq: RenderRequest = { ...REQ, mode: 'stems', overlapBars: 0 };
    expect(spec('stems-orbit-dropped').apply(stemsReq)).toEqual({
      ...stemsReq,
      mutation: { kind: 'dropOrbit', orbit: 4, bars: [1, 4, 6] },
    });
  });
  test('bass-dropped drops notes below 130 Hz in three adjacent bars', () => {
    expect(spec('bass-dropped').apply(REQ)).toEqual({
      ...REQ,
      mutation: { kind: 'dropNotesBelowHz', hz: 130, bars: [3, 4, 5] },
    });
  });
  test('apply does not mutate its input', () => {
    const copy = { ...REQ };
    for (const m of MUTATIONS) m.apply(copy);
    expect(copy).toEqual(REQ);
  });
});

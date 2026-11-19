import { describe, expect, test } from 'bun:test';
import type { FixtureEntry } from '../src/lib/render/corpus';
import type { EngineJob, Provenance } from '../src/lib/render/engine-job';
import { pauseFrameOf } from '../src/lib/render/plan';
import {
  burstOnsets,
  FEEDBACK_CASE,
  FEEDBACK_EXTRA_FRAMES,
  feedbackEvidence,
  peakIn,
  budgetProblems,
  BUDGET_JOB_BYTES,
  BUDGET_WAV_BYTES,
  buildIndex,
  caseFiles,
  diffRefs,
  expectedFirstSounding,
  firstNonZero,
  fixtureEnvelopeFile,
  fixtureHasRefs,
  fixtureJobFile,
  formatDiff,
  formatTotals,
  hasChanges,
  indexText,
  jobFileText,
  mergeRefs,
  microFrameLimit,
  microJobFile,
  microWavFile,
  parseRefsArgs,
  peakOf,
  referenceEnvelope,
  rmsDb,
  roundTo,
  selectCases,
  SILENCE_DB,
  suspendEvidence,
  timeToSampleFrameRoundUp,
  totalsOf,
  utf8,
  determinismOf,
  expectedWorkletStartQuantum,
  FLOOR_CASES,
  FLOOR_MARGIN_DB,
  FLOOR_MAX_DB,
  FLOOR_RUNS,
  INDEX_SCHEMA,
  isWorkletCase,
  ladderTeardownCutFrame,
  LONG_PREFIXES,
  MATH_CASE,
  MATH_FILE,
  MATH_MAX_BYTES,
  mathFileText,
  mathGroups,
  mathOutputsDiffer,
  partialRunRefusal,
  residualDb,
  settleFloorCase,
  WORKLET_START_CASE,
  WORKLET_TEARDOWN_CASE,
  workletStartEvidence,
  workletTeardownEvidence,
  type GeneratedCase,
  type IndexCase,
} from '../scripts/engine/refs-lib';
import type { MathProvenance } from '../scripts/engine/provenance';

const PROV: Provenance = {
  schema: 2,
  sourceSha256: 'aa',
  strudelCommit: 'bb',
  strudelPatchesSha256: 'cc',
  plannerSha256: 'dd',
  playwright: '1.64.1',
  chromium: '156',
  platform: 'darwin-arm64',
};

function fixture(o: Partial<FixtureEntry>): FixtureEntry {
  return {
    name: 'f',
    file: 'f.js',
    kind: 'targeted',
    windowBars: [0, 4],
    layout: 'linear',
    gates: ['twin', 'mix'],
    stochastic: false,
    usesSamples: false,
    rust: 'supported',
    tags: [],
    ...o,
  };
}

function job(o: Partial<EngineJob> = {}): EngineJob {
  return {
    schema: 2,
    sampleRate: 44100,
    frames: 1000,
    seed: 0,
    cps: 1,
    channels: { count: 2, multiChannelOrbits: false },
    orbits: [{ orbit: 1, channels: [0, 1] }],
    pauses: [{ suspendSec: 0, haps: [] }],
    values: [],
    ...o,
  };
}

describe('parseRefsArgs', () => {
  test('defaults, --write, names (deduplicated), help', () => {
    expect(parseRefsArgs([])).toEqual({ write: false, names: [], help: false });
    expect(parseRefsArgs(['--write', 'osc-sine', 'sine-basic', 'osc-sine'])).toEqual({
      write: true,
      names: ['osc-sine', 'sine-basic'],
      help: false,
    });
    expect(parseRefsArgs(['-h'])).toMatchObject({ help: true });
    expect(parseRefsArgs(['--help'])).toMatchObject({ help: true });
  });

  test('unknown flags and odd names are errors', () => {
    expect(parseRefsArgs(['--update'])).toEqual({ error: 'unknown flag --update' });
    expect(parseRefsArgs(['../x'])).toEqual({ error: 'invalid case name ../x' });
    expect(parseRefsArgs(['Osc'])).toEqual({ error: 'invalid case name Osc' });
  });
});

describe('fixtureHasRefs and selectCases', () => {
  const corpus = [
    fixture({ name: 'bass-line' }),
    fixture({ name: 'external-song', kind: 'external' }),
    fixture({ name: 'duck', rust: 'unsupported' }),
    fixture({ name: 'noise-seed', rust: 'unsupported' }),
    fixture({ name: 'samples', rust: 'unsupported' }),
  ];

  test('supported targeted fixtures and named exceptions have refs', () => {
    expect(corpus.filter(fixtureHasRefs).map((f) => f.name)).toEqual(['bass-line', 'noise-seed']);
    expect(fixtureHasRefs(fixture({ name: 'noise-seed', rust: 'unsupported' }))).toBe(true);
    expect(fixtureHasRefs(fixture({ name: 'duck', rust: 'unsupported' }))).toBe(false);
    expect(fixtureHasRefs(fixture({ name: 'duck' }))).toBe(true);
    expect(fixtureHasRefs(fixture({ name: 'worklets', rust: 'unsupported' }))).toBe(false);
    expect(fixtureHasRefs(fixture({ name: 'samples', rust: 'unsupported' }))).toBe(false);
  });

  test('no names selects everything with refs, sorted', () => {
    expect(selectCases([], ['osc-sine', 'adsr-defaults'], corpus)).toEqual({
      micro: ['adsr-defaults', 'osc-sine'],
      fixtures: ['bass-line', 'noise-seed'],
      math: true,
    });
  });

  test('names resolve to micro cases or fixtures with refs', () => {
    expect(selectCases(['noise-seed', 'osc-sine'], ['osc-sine'], corpus)).toEqual({
      micro: ['osc-sine'],
      fixtures: ['noise-seed'],
      math: false,
    });
  });

  test('v8-math is selected on a full run or by name, and reserved', () => {
    expect(MATH_CASE).toBe('v8-math');
    expect(selectCases(['v8-math'], ['osc-sine'], corpus)).toEqual({
      micro: [],
      fixtures: [],
      math: true,
    });
    expect(selectCases(['v8-math', 'osc-sine'], ['osc-sine'], corpus)).toMatchObject({
      micro: ['osc-sine'],
      math: true,
    });
    expect(selectCases([], ['v8-math'], corpus)).toEqual({
      error: 'v8-math is the V8 math reference; no micro case may take the name',
    });
    expect(selectCases([], [], [...corpus, fixture({ name: 'v8-math' })])).toEqual({
      error: 'v8-math is the V8 math reference; no corpus fixture may take the name',
    });
    expect(parseRefsArgs(['--write', 'v8-math'])).toEqual({
      write: true,
      names: ['v8-math'],
      help: false,
    });
  });

  test('fixtures without refs, unknown names and clashing micro names are errors', () => {
    expect(selectCases(['external-song'], [], corpus)).toEqual({
      error: 'fixture external-song has no engine refs (external or unsupported fixture)',
    });
    expect(selectCases(['nope'], [], corpus)).toEqual({
      error: 'unknown micro case or fixture nope',
    });
    expect(selectCases(['duck'], [], corpus)).toEqual({
      error: 'fixture duck has no engine refs (external or unsupported fixture)',
    });
    expect(selectCases([], ['noise-seed'], corpus)).toEqual({
      error: 'noise-seed is both a micro case and a corpus fixture',
    });
    expect(selectCases([], ['fixture-x'], corpus)).toEqual({
      error: 'micro case fixture-x may not start with fixture-',
    });
  });
});

describe('file names and job text', () => {
  test('names', () => {
    expect(microJobFile('osc-sine')).toBe('osc-sine.job.json');
    expect(microWavFile('osc-sine')).toBe('osc-sine.wav');
    expect(fixtureJobFile('stems-5', 'stems', 0)).toBe('fixture-stems-5--stems-0.job.json');
    expect(fixtureEnvelopeFile('pad-seam', 'mix', 2)).toBe('fixture-pad-seam--mix-2.ref.json');
  });

  test('the job file carries the provenance last, replacing any old one', () => {
    const text = jobFileText({ ...job(), provenance: { ...PROV, chromium: 'old' } }, PROV);
    expect(text.endsWith('}\n')).toBe(true);
    expect(text.split('\n').length).toBe(2);
    const parsed = JSON.parse(text);
    expect(parsed.provenance).toEqual(PROV);
    expect(Object.keys(parsed).at(-1)).toBe('provenance');
    expect({ ...parsed, provenance: undefined }).toEqual({ ...job(), provenance: undefined });
  });
});

describe('envelope', () => {
  test('roundTo rounds half away from zero and drops -0', () => {
    expect(roundTo(1.005, 2)).toBe(1);
    expect(roundTo(1.0051, 2)).toBe(1.01);
    expect(roundTo(-2.345, 1)).toBe(-2.3);
    expect(roundTo(-2.35, 1)).toBe(-2.4);
    expect(Object.is(roundTo(-0.0001, 2), 0)).toBe(true);
  });

  test('rmsDb of a full-scale square is 0 dB, of half amplitude -6.02 dB', () => {
    const sq = Float32Array.from({ length: 441 }, (_, i) => (i % 2 ? 1 : -1));
    expect(rmsDb(sq, 0, 441)).toBe(0);
    const half = sq.map((v) => v / 2);
    expect(rmsDb(half, 0, 441)).toBe(-6.02);
  });

  test('rmsDb: silence and empty ranges are -200, very quiet clamps to -200, NaN propagates', () => {
    const z = new Float32Array(10);
    expect(rmsDb(z, 0, 10)).toBe(SILENCE_DB);
    expect(rmsDb(z, 5, 5)).toBe(SILENCE_DB);
    expect(rmsDb(Float32Array.of(1e-30), 0, 1)).toBe(SILENCE_DB);
    expect(rmsDb(Float32Array.of(NaN, 1), 0, 2)).toBeNaN();
  });

  test('peakOf takes the absolute maximum, rounded to 6 decimals', () => {
    expect(peakOf(Float32Array.of(0.1, -0.75, 0.5))).toBe(0.75);
    expect(peakOf(Float32Array.of(0.12345678))).toBe(0.123457);
    expect(peakOf(new Float32Array(0))).toBe(0);
    expect(peakOf(Float32Array.of(Infinity))).toBe(Infinity);
  });

  test('referenceEnvelope windows 441 frames, the last window short', () => {
    const frames = 1000;
    const a = new Float32Array(frames).fill(0.5);
    const b = new Float32Array(frames);
    b[900] = 1;
    const e = referenceEnvelope([a, b]);
    expect(e).toMatchObject({ schema: 1, windowFrames: 441, frames, channels: 2 });
    expect(e.rmsDb[0]).toEqual([-6.02, -6.02, -6.02]);
    // window 2 is frames [882, 1000): one full-scale sample in 118
    expect(e.rmsDb[1]).toEqual([-200, -200, roundTo(10 * Math.log10(1 / 118), 2)]);
    expect(e.peak).toEqual([0.5, 1]);
  });

  test('referenceEnvelope refuses no channels, unequal lengths and non-finite samples', () => {
    expect(() => referenceEnvelope([])).toThrow(/no channels/);
    expect(() => referenceEnvelope([new Float32Array(3), new Float32Array(4)])).toThrow(/mismatch/);
    expect(() => referenceEnvelope([Float32Array.of(0, NaN)])).toThrow(/non-finite/);
    expect(() => referenceEnvelope([Float32Array.of(0, Infinity)])).toThrow(/non-finite/);
  });
});

const micro = (name: string): IndexCase => ({
  kind: 'micro',
  provenance: PROV,
  jobs: [microJobFile(name)],
  envelopes: null,
  wav: microWavFile(name),
  data: null,
});

const MPROV: MathProvenance = {
  schema: 1,
  inputsSha256: 'ee',
  playwright: '1.64.1',
  chromium: '156',
  platform: 'darwin-arm64',
};

const mathCase = (): IndexCase => ({
  kind: 'math',
  provenance: MPROV,
  jobs: [],
  envelopes: null,
  wav: null,
  data: MATH_FILE,
});

function gen(name: string, payload: string): GeneratedCase {
  return {
    name,
    entry: micro(name),
    files: new Map([
      [microJobFile(name), utf8(`${payload}-job`)],
      [microWavFile(name), utf8(`${payload}-wav`)],
    ]),
  };
}

describe('index', () => {
  test('lists every file with sha256 and size, sorted, and every case', () => {
    const files = new Map([
      ['b.wav', utf8('abc')],
      ['a.job.json', utf8('')],
    ]);
    const index = buildIndex(files, { b: micro('b') });
    expect(Object.keys(index.files)).toEqual(['a.job.json', 'b.wav']);
    expect(index.files['b.wav']).toEqual({
      sha256: 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
      bytes: 3,
    });
    expect(index.files['a.job.json']!.bytes).toBe(0);
    expect(index.schema).toBe(2);
    expect(INDEX_SCHEMA).toBe(2);
    expect(indexText(index).endsWith('}\n')).toBe(true);
    expect(JSON.parse(indexText(index))).toEqual(index);
  });

  test('caseFiles lists jobs, envelopes and the WAV', () => {
    expect(caseFiles(micro('x'))).toEqual(['x.job.json', 'x.wav']);
    expect(
      caseFiles({
        kind: 'fixture',
        provenance: PROV,
        jobs: ['j'],
        envelopes: ['e'],
        wav: null,
        data: null,
      }),
    ).toEqual(['j', 'e']);
    expect(caseFiles(mathCase())).toEqual(['v8-math.json']);
  });

  test('a schema 1 index refuses a partial --write only', () => {
    const msg = 'index.json is schema 1: run `engine:refs --write` without names';
    expect(partialRunRefusal({ schema: 1 }, false, true)).toBe(msg);
    expect(partialRunRefusal({ schema: 1 }, true, true)).toBeNull();
    expect(partialRunRefusal({ schema: 1 }, false, false)).toBeNull();
    expect(partialRunRefusal({ schema: 2 }, false, true)).toBeNull();
    expect(partialRunRefusal(null, false, true)).toBeNull();
  });
});

describe('budgets', () => {
  test('micro frame limits: 0.5 s, or 2 s for the long prefixes', () => {
    expect(LONG_PREFIXES).toEqual([
      'delay-',
      'reverb-',
      'measure-',
      'compressor-',
      'duck-',
      'djf-',
      'fx-chain-',
      'mod-orbit-',
    ]);
    for (const p of LONG_PREFIXES) expect(microFrameLimit(`${p}x`), p).toBe(88200);
    expect(microFrameLimit('osc-sine')).toBe(22050);
    expect(microFrameLimit('adsr-delay')).toBe(22050);
    expect(microFrameLimit('mod-lfo-note')).toBe(22050);
    expect(microFrameLimit('fx-chain')).toBe(22050);
  });

  test('the WAV budget is 40 MiB and v8-math.json has its own 1 MiB cap', () => {
    expect(BUDGET_WAV_BYTES).toBe(40 * 1024 * 1024);
    expect(MATH_MAX_BYTES).toBe(1024 * 1024);
    const at = (n: number) => new Map([[MATH_FILE, new Uint8Array(n)]]);
    expect(budgetProblems(at(MATH_MAX_BYTES), new Map())).toEqual([]);
    expect(budgetProblems(at(MATH_MAX_BYTES + 1), new Map())).toEqual([
      `v8-math.json takes ${MATH_MAX_BYTES + 1} bytes, over ${MATH_MAX_BYTES}`,
    ]);
    // v8-math.json counts toward neither the job nor the WAV total
    expect(totalsOf(at(10))).toEqual({ jobBytes: 0, wavBytes: 0, files: 1 });
  });

  test('totals count jobs with envelopes and WAVs apart; index and others are not counted', () => {
    const files = new Map([
      ['a.job.json', new Uint8Array(10)],
      ['a.ref.json', new Uint8Array(5)],
      ['a.wav', new Uint8Array(100)],
      ['other.txt', new Uint8Array(7)],
    ]);
    const t = totalsOf(files);
    expect(t).toEqual({ jobBytes: 15, wavBytes: 100, files: 4 });
    expect(formatTotals(t)).toBe(
      '4 files: jobs+envelopes 0.00 MiB of 8.00 MiB, WAVs 0.00 MiB of 40.00 MiB',
    );
  });

  test('within budget: no problems', () => {
    const files = new Map([['a.wav', new Uint8Array(BUDGET_WAV_BYTES)]]);
    expect(budgetProblems(files, new Map([['osc-sine', 22050]]))).toEqual([]);
  });

  test('every broken budget is reported', () => {
    const files = new Map([
      ['a.job.json', new Uint8Array(BUDGET_JOB_BYTES)],
      ['b.ref.json', new Uint8Array(1)],
      ['a.wav', new Uint8Array(BUDGET_WAV_BYTES + 1)],
    ]);
    const wavs = new Map([
      ['osc-sine', 22051],
      ['delay-x', 88201],
      ['reverb-y', 88200],
    ]);
    expect(budgetProblems(files, wavs)).toEqual([
      `jobs and envelopes take ${BUDGET_JOB_BYTES + 1} bytes, over ${BUDGET_JOB_BYTES}`,
      `WAVs take ${BUDGET_WAV_BYTES + 1} bytes, over ${BUDGET_WAV_BYTES}`,
      'delay-x: 88201 frames, over 88200',
      'osc-sine: 22051 frames, over 22050',
    ]);
  });
});

describe('mergeRefs', () => {
  const committed = mergeRefs(new Map(), null, [gen('a', 'old'), gen('b', 'old')], true);

  test('a full run is exactly the generated cases', () => {
    const next = mergeRefs(committed.files, committed.index, [gen('a', 'new')], true);
    expect([...next.files.keys()].sort()).toEqual(['a.job.json', 'a.wav']);
    expect(Object.keys(next.index.cases)).toEqual(['a']);
  });

  test('a partial run replaces the generated cases and keeps the others', () => {
    const next = mergeRefs(committed.files, committed.index, [gen('a', 'new')], false);
    expect([...next.files.keys()].sort()).toEqual(['a.job.json', 'a.wav', 'b.job.json', 'b.wav']);
    expect(new TextDecoder().decode(next.files.get('a.wav'))).toBe('new-wav');
    expect(new TextDecoder().decode(next.files.get('b.wav'))).toBe('old-wav');
    expect(Object.keys(next.index.cases)).toEqual(['a', 'b']);
  });

  test('a partial run drops files the regenerated case no longer has', () => {
    const fewer: GeneratedCase = {
      name: 'a',
      entry: { ...micro('a'), jobs: [] },
      files: new Map([['a.wav', utf8('w')]]),
    };
    const next = mergeRefs(committed.files, committed.index, [fewer], false);
    expect(next.files.has('a.job.json')).toBe(false);
  });

  test('a partial run keeps the math case and a floor case as committed', () => {
    const math: GeneratedCase = {
      name: MATH_CASE,
      entry: mathCase(),
      files: new Map([[MATH_FILE, utf8('{}')]]),
    };
    const floor: GeneratedCase = { ...gen('f', 'old'), entry: { ...micro('f'), floorDb: -120 } };
    const base = mergeRefs(new Map(), null, [math, floor, gen('a', 'old')], true);
    const next = mergeRefs(base.files, base.index, [gen('a', 'new')], false);
    expect(next.index.cases[MATH_CASE]).toEqual(mathCase());
    expect(next.index.cases.f!.floorDb).toBe(-120);
    expect(next.files.has(MATH_FILE)).toBe(true);
  });

  test('a partial run without a committed index keeps nothing else', () => {
    const next = mergeRefs(committed.files, null, [gen('a', 'new')], false);
    expect(Object.keys(next.index.cases)).toEqual(['a']);
  });
});

describe('diff', () => {
  const before = new Map([
    ['same', utf8('x')],
    ['changed', utf8('old')],
    ['gone', utf8('bye')],
  ]);
  const after = new Map([
    ['same', utf8('x')],
    ['changed', utf8('newer')],
    ['new', utf8('hi')],
  ]);

  test('added, changed, removed and unchanged', () => {
    const d = diffRefs(before, after);
    expect(d).toEqual({ added: ['new'], changed: ['changed'], removed: ['gone'], unchanged: 1 });
    expect(hasChanges(d)).toBe(true);
    expect(hasChanges(diffRefs(before, before))).toBe(false);
  });

  test('the summary names each file with its sizes', () => {
    expect(formatDiff(diffRefs(before, after), before, after)).toEqual([
      '  added    new (2 bytes)',
      '  changed  changed (3 -> 5 bytes)',
      '  removed  gone (3 bytes)',
      '  1 added, 1 changed, 1 removed, 1 unchanged',
    ]);
  });
});

describe('suspend quantization', () => {
  test('TimeToSampleFrame rounds to 1/1024 frame, then up', () => {
    expect(timeToSampleFrameRoundUp(0)).toBe(0);
    expect(timeToSampleFrameRoundUp(1)).toBe(44100);
    expect(timeToSampleFrameRoundUp(100.5 / 44100)).toBe(101);
    // within 1/2048 frame above a whole frame rounds down to it
    expect(timeToSampleFrameRoundUp((100 + 1 / 4096) / 44100)).toBe(100);
    expect(timeToSampleFrameRoundUp((100 + 1 / 1000) / 44100)).toBe(101);
  });

  test('pause frames truncate, then round up to 128', () => {
    // The one quantization the job validator and the suspend evidence share (plan.ts).
    expect(pauseFrameOf(0, 44100)).toBe(0);
    expect(pauseFrameOf(129.9 / 44100, 44100)).toBe(256);
    expect(pauseFrameOf(256 / 44100, 44100)).toBe(256);
    expect(pauseFrameOf(0.4358299595141701, 44100)).toBe(19328);
  });

  test('first sounding frame: the start frame, or the next one on or just past a frame', () => {
    expect(expectedFirstSounding(0)).toBe(1);
    expect(expectedFirstSounding(13390.69 / 44100)).toBe(13391);
    expect(expectedFirstSounding(500 / 44100)).toBe(501);
    expect(expectedFirstSounding((500 + 1 / 4096) / 44100)).toBe(501);
  });

  test('firstNonZero scans every channel inside the range', () => {
    const a = new Float32Array(10);
    const b = new Float32Array(10);
    b[6] = -0.1;
    a[8] = 0.2;
    expect(firstNonZero([a, b], 0, 10)).toBe(6);
    expect(firstNonZero([a, b], 7, 10)).toBe(8);
    expect(firstNonZero([a, b], -5, 6)).toBe(-1);
    expect(firstNonZero([a, b], 9, 20)).toBe(-1);
  });

  function evidenceJob(ts: number[][]): EngineJob {
    return job({
      frames: 4000,
      pauses: ts.map((haps, j) => ({
        suspendSec: j === 0 ? 0 : (j * 1000.3) / 44100,
        haps: haps.map((t, i) => ({ t, duration: 0.01, cycle: j, v: 0, event: i })),
      })),
    });
  }

  function sounding(starts: number[]): Float32Array[] {
    const c = new Float32Array(4000);
    for (const s of starts) for (let i = s; i < s + 50; i++) c[i] = 0.5;
    return [c, c];
  }

  test('agrees when every hap sounds where Chromium is expected to start it', () => {
    const j = evidenceJob([[0], [2000.5 / 44100]]);
    const e = suspendEvidence(j, sounding([1, 2001]));
    expect(e.pauses[1]).toEqual({
      suspendSec: 1000.3 / 44100,
      truncatedFrame: 1000,
      pauseFrame: 1024,
      truncatedIsMultipleOf128: false,
    });
    expect(e.haps.map((h) => [h.expectedFirstSoundingFrame, h.observedFirstSoundingFrame])).toEqual(
      [
        [1, 1],
        [2001, 2001],
      ],
    );
    expect(e.agrees).toBe(true);
  });

  test('disagrees on a late, early or missing onset', () => {
    const j = evidenceJob([[0], [2000.5 / 44100]]);
    expect(suspendEvidence(j, sounding([1, 2002])).agrees).toBe(false);
    expect(suspendEvidence(j, sounding([1, 2000])).agrees).toBe(false);
    const missing = suspendEvidence(j, sounding([1]));
    expect(missing.haps[1]!.observedFirstSoundingFrame).toBe(-1);
    expect(missing.agrees).toBe(false);
  });

  test('a hap sounding before its pause frame disagrees; no haps never agrees', () => {
    // a hap at frame 600 scheduled in the pause at frame 1024 (impossible in Chromium)
    const j = evidenceJob([[], [600.5 / 44100]]);
    const e = suspendEvidence(j, sounding([601]));
    expect(e.haps[0]!.afterPauseFrame).toBe(false);
    expect(e.agrees).toBe(false);
    expect(suspendEvidence(evidenceJob([[]]), sounding([])).agrees).toBe(false);
  });
});

describe('feedback echo', () => {
  test('burstOnsets finds runs after a gap of zeros on any channel', () => {
    const a = new Float32Array(2000);
    const b = new Float32Array(2000);
    a[0] = 1;
    a[100] = 1; // within the gap: same burst
    b[500] = -1;
    a[1200] = 0.5;
    expect(burstOnsets([a, b])).toEqual([0, 500, 1200]);
    expect(burstOnsets([a, b], 50)).toEqual([0, 100, 500, 1200]);
    expect(burstOnsets([new Float32Array(10)])).toEqual([]);
    expect(burstOnsets([])).toEqual([]);
  });

  test('peakIn is the largest magnitude over every channel in the range', () => {
    const a = Float32Array.from([0, 0.5, -2, 0]);
    const b = Float32Array.from([1, 0, 0, 3]);
    expect(peakIn([a, b], 0, 3)).toBe(2);
    expect(peakIn([a, b], 0, Infinity)).toBe(3);
    expect(peakIn([a, b], -4, 1)).toBe(1);
  });

  function echoJob(delaytime: number, feedback: number): EngineJob {
    return job({
      frames: 12000,
      pauses: [{ suspendSec: 0, haps: [{ t: 0, duration: 0.01, cycle: 0, v: 0, event: 0 }] }],
      values: [{ s: 'one', delay: 1, delaytime, delayfeedback: feedback }],
    });
  }

  function echoes(at: number[], peaks: number[]): Float32Array[] {
    const c = new Float32Array(12000);
    at.forEach((f, i) => {
      for (let j = 0; j < 100; j++) c[f + j] = peaks[i]! * (j === 50 ? 1 : 0.1);
    });
    return [c, c];
  }

  test('agrees when echo k lands at t0 + k*D + (k-1)*128 with g^(k-1)', () => {
    const d = 2205;
    const at = [1, ...[1, 2, 3, 4].map((k) => 1 + k * d + (k - 1) * FEEDBACK_EXTRA_FRAMES)];
    const e = feedbackEvidence(echoJob(0.05, 0.5), echoes(at, [1, 1, 0.5, 0.25, 0.125]));
    expect(e.case).toBe(FEEDBACK_CASE);
    expect(e.delayFrames).toBe(2205);
    expect(e.dryOnsetFrame).toBe(1);
    expect(e.echoes.map((x) => x.observedFrame)).toEqual([2206, 4539, 6872, 9205]);
    expect(e.echoes.map((x) => x.peakRatio)).toEqual([1, 0.5, 0.25, 0.125]);
    expect(e.agrees).toBe(true);
  });

  test('disagrees on a misplaced or missing echo, a wrong amplitude or a fractional delay', () => {
    const d = 2205;
    const plain = [1, 2206, 4411, 6616, 8821]; // k*D, no extra quantum
    expect(
      feedbackEvidence(echoJob(0.05, 0.5), echoes(plain, [1, 1, 0.5, 0.25, 0.125])).agrees,
    ).toBe(false);
    const at = [1, ...[1, 2, 3, 4].map((k) => 1 + k * d + (k - 1) * 128)];
    expect(feedbackEvidence(echoJob(0.05, 0.5), echoes(at, [1, 1, 0.5, 0.3, 0.125])).agrees).toBe(
      false,
    );
    const missing = feedbackEvidence(echoJob(0.05, 0.5), echoes(at.slice(0, 3), [1, 1, 0.5]));
    expect(missing.echoes[2]!.observedFrame).toBe(-1);
    expect(missing.agrees).toBe(false);
    // 0.0625 s is 2756.25 frames: the rule needs a whole number of frames.
    expect(
      feedbackEvidence(echoJob(0.0625, 0.5), echoes(at, [1, 1, 0.5, 0.25, 0.125])).agrees,
    ).toBe(false);
    expect(feedbackEvidence(job(), [new Float32Array(10)]).agrees).toBe(false);
  });
});

describe('V8 math file', () => {
  const inputs = [
    { id: 'a', fn: 'sin' as const, xs: ['3ff0000000000000'], ys: null },
    { id: 'b', fn: 'pow' as const, xs: ['4000000000000000'], ys: ['3ff0000000000000'] },
  ];
  const outputs = [
    { id: 'a', out: ['3feaed548f090cee'] },
    { id: 'b', out: ['4000000000000000'] },
  ];

  test('mathGroups joins inputs with outputs', () => {
    expect(mathGroups(inputs, outputs)).toEqual([
      { ...inputs[0]!, out: outputs[0]!.out },
      { ...inputs[1]!, out: outputs[1]!.out },
    ]);
  });

  test('mathGroups refuses mismatched groups', () => {
    expect(() => mathGroups(inputs, outputs.slice(1))).toThrow(/1 output groups for 2/);
    expect(() => mathGroups(inputs, [outputs[1]!, outputs[0]!])).toThrow(/expected a/);
    expect(() => mathGroups(inputs, [{ id: 'a', out: [] }, outputs[1]!])).toThrow(/0 outputs/);
    expect(() => mathGroups(inputs, [{ id: 'a', out: ['XYZ'] }, outputs[1]!])).toThrow(/16-digit/);
  });

  test('mathFileText: schema, provenance, one group per line, a trailing newline', () => {
    const text = mathFileText(MPROV, mathGroups(inputs, outputs));
    const lines = text.split('\n');
    expect(lines[0]).toBe('{"schema":1,');
    expect(lines[1]).toBe(`"provenance":${JSON.stringify(MPROV)},`);
    expect(lines[2]).toBe('"groups":[');
    expect(JSON.parse(lines[3]!.replace(/,$/, ''))).toEqual({ ...inputs[0], out: outputs[0]!.out });
    expect(lines[5]).toBe(']}');
    expect(text.endsWith(']}\n')).toBe(true);
    const parsed = JSON.parse(text);
    expect(parsed.schema).toBe(1);
    expect(parsed.groups[1].ys).toEqual(['3ff0000000000000']);
    expect(parsed.groups[0].ys).toBeNull();
  });

  test('mathOutputsDiffer names the first difference', () => {
    expect(mathOutputsDiffer(outputs, outputs)).toBeNull();
    expect(mathOutputsDiffer(outputs, outputs.slice(1))).toBe('2 groups against 1');
    expect(mathOutputsDiffer(outputs, [outputs[1]!, outputs[0]!])).toBe('group 0: a against b');
    expect(mathOutputsDiffer(outputs, [{ id: 'a', out: ['3feaed548f090cef'] }, outputs[1]!])).toBe(
      'a[0]: 3feaed548f090cee against 3feaed548f090cef',
    );
    expect(mathOutputsDiffer(outputs, [{ id: 'a', out: [] }, outputs[1]!])).toBe('a: 1 against 0');
  });
});

describe('residualDb', () => {
  const a = [Float32Array.of(1, -1, 0.5), Float32Array.of(0, 0, 0)];

  test('equal buffers are -Infinity, silence too', () => {
    expect(residualDb(a, a)).toBe(-Infinity);
    const z = [new Float32Array(3)];
    expect(residualDb(z, z)).toBe(-Infinity);
  });

  test('a silent reference against sound is Infinity', () => {
    expect(residualDb([new Float32Array(2)], [Float32Array.of(0, 1e-9)])).toBe(Infinity);
  });

  test('the dB ratio over every channel', () => {
    const half = a.map((c) => c.map((v) => v / 2));
    expect(residualDb(a, half)).toBeCloseTo(20 * Math.log10(0.5), 12);
  });

  test('NaN propagates; different shapes throw', () => {
    expect(residualDb(a, [Float32Array.of(1, NaN, 0.5), a[1]!])).toBeNaN();
    expect(() => residualDb(a, [a[0]!])).toThrow(/different shapes/);
    expect(() => residualDb(a, [a[0]!, new Float32Array(2)])).toThrow(/different shapes/);
  });
});

describe('settleFloorCase', () => {
  const base = Float32Array.from({ length: 64 }, (_, i) => Math.sin(i / 3));
  // render k differs from render 0 by a tiny amount in one sample: about -150 dB
  const render = (k: number, eps = 1e-7): Float32Array[] => {
    const c = base.slice();
    c[10] = c[10]! + k * eps;
    return [c, c.slice()];
  };
  const renders = (eps = 1e-7) => Array.from({ length: FLOOR_RUNS }, (_, k) => render(k, eps));
  const JOB = '{"job":1}\n';

  test('constants', () => {
    expect(FLOOR_RUNS).toBe(8);
    expect(FLOOR_MARGIN_DB).toBe(6);
    expect(FLOOR_MAX_DB).toBe(-100);
    expect(FLOOR_CASES).toEqual(['vowel-formants', 'mod-lfo-note']);
  });

  test('first generation: render 1 and the largest residual against it', () => {
    const r = renders();
    const s = settleFloorCase(JOB, r, null);
    if (!('status' in s) || s.status !== 'changed') throw new Error(JSON.stringify(s));
    expect(s.wav).toBe(r[0]!);
    expect(s.reason).toBe('first generation');
    expect(s.floorDb).toBe(roundTo(residualDb(r[0]!, r[7]!), 2));
  });

  test('unchanged when the job is equal and every render is within floor + 6 dB', () => {
    const r = renders();
    const first = settleFloorCase(JOB, r, null) as { floorDb: number };
    const committed = { jobBytes: utf8(JOB), wav: r[0]!, floorDb: first.floorDb };
    const s = settleFloorCase(JOB, renders(), committed);
    expect(s).toMatchObject({ status: 'unchanged', floorDb: first.floorDb });
  });

  test('changed when the job changed', () => {
    const r = renders();
    const committed = { jobBytes: utf8('{"job":0}\n'), wav: r[0]!, floorDb: -120 };
    expect(settleFloorCase(JOB, r, committed)).toMatchObject({
      status: 'changed',
      reason: 'the job changed',
    });
  });

  test('changed when a render is beyond the committed floor + 6 dB', () => {
    const r = renders();
    const committed = { jobBytes: utf8(JOB), wav: r[0]!, floorDb: -200 };
    const s = settleFloorCase(JOB, r, committed);
    expect(s).toMatchObject({ status: 'changed' });
    expect((s as { reason: string }).reason).toMatch(/over -194 dB/);
  });

  test('identical renders and a floor above -100 dB are errors; so is the wrong count', () => {
    const same = Array.from({ length: FLOOR_RUNS }, () => render(0));
    expect(settleFloorCase(JOB, same, null)).toEqual({
      error: 'case is declared floor but its 8 renders are identical; remove the floor',
    });
    expect(settleFloorCase(JOB, renders(0.5), null)).toMatchObject({
      error: expect.stringMatching(/above -100 dB is a defect/),
    });
    expect(settleFloorCase(JOB, renders().slice(1), null)).toEqual({
      error: 'a floor case needs 8 renders, got 7',
    });
  });
});

describe('measure-worklet-start', () => {
  test('the first quantum strictly after f32(t)', () => {
    expect(WORKLET_START_CASE).toBe('measure-worklet-start');
    expect(expectedWorkletStartQuantum(0)).toBe(1);
    expect(expectedWorkletStartQuantum(257 / 44100)).toBe(3);
    expect(expectedWorkletStartQuantum(383 / 44100)).toBe(3);
    expect(expectedWorkletStartQuantum(-1)).toBe(0);
  });

  test('on a boundary: one quantum later when f32(t) >= t, the boundary quantum when below', () => {
    // 53376 = 128 * 417 and f32 rounds up; 70912 = 128 * 554 and f32 rounds down
    const up = 53376 / 44100;
    const down = 70912 / 44100;
    expect(Math.fround(up) > up).toBe(true);
    expect(Math.fround(down) < down).toBe(true);
    expect(expectedWorkletStartQuantum(up)).toBe(418);
    expect(expectedWorkletStartQuantum(down)).toBe(554);
  });

  function startJob(ts: number[]): EngineJob {
    return job({
      frames: 4000,
      pauses: [
        {
          suspendSec: 0,
          haps: ts.map((t, i) => ({ t, duration: 0.01, cycle: 0, v: 0, event: i })),
        },
      ],
    });
  }
  const sound = (from: number[]): Float32Array[] => {
    const c = new Float32Array(4000);
    for (const f of from) for (let i = f; i < f + 64; i++) c[i] = 0.25;
    return [c, c];
  };

  test('agrees when each voice first sounds in its expected quantum', () => {
    const e = workletStartEvidence(startJob([257 / 44100, 2000.5 / 44100]), sound([384, 2048]));
    expect(e.haps.map((h) => [h.expectedQuantum, h.observedQuantum])).toEqual([
      [3, 3],
      [16, 16],
    ]);
    expect(e.haps[0]).toMatchObject({ tFrames: 257, offsetInQuantum: 1 });
    expect(e.haps[1]!.offsetInQuantum).toBe(80.5);
    expect(e.agrees).toBe(true);
  });

  test('disagrees on an early, late or missing voice; no haps never agrees', () => {
    const j = startJob([257 / 44100]);
    expect(workletStartEvidence(j, sound([256])).agrees).toBe(false);
    expect(workletStartEvidence(j, sound([512])).agrees).toBe(false);
    const missing = workletStartEvidence(j, sound([]));
    expect(missing.haps[0]!.observedQuantum).toBe(-1);
    expect(missing.agrees).toBe(false);
    expect(workletStartEvidence(startJob([]), sound([])).agrees).toBe(false);
  });
});

describe('measure-worklet-teardown', () => {
  test('the cut is ceil(stopFrame / 128) * 128', () => {
    expect(WORKLET_TEARDOWN_CASE).toBe('measure-worklet-teardown');
    expect(ladderTeardownCutFrame(0)).toBe(0);
    expect(ladderTeardownCutFrame(127)).toBe(128);
    expect(ladderTeardownCutFrame(128)).toBe(128);
    expect(ladderTeardownCutFrame(129)).toBe(256);
  });

  function tdJob(release: unknown): EngineJob {
    return job({
      frames: 20000,
      pauses: [{ suspendSec: 0, haps: [{ t: 0, duration: 0.25, cycle: 0, v: 0, event: 0 }] }],
      values: [{ s: 'sawtooth', release } as never],
    });
  }
  const ring = (until: number, amp = 0.2): Float32Array[] => {
    const c = new Float32Array(20000);
    for (let i = 0; i < until; i++) c[i] = amp * Math.sin(i / 5) + (i % 2 ? 1e-3 : -1e-3);
    return [c, c];
  };

  test('agrees when the ring stops exactly at the expected cut', () => {
    // stop frame TimeToSampleFrame(0.31 s) = 13671, cut 13696
    const e = workletTeardownEvidence(tdJob(0.05), ring(13696));
    expect(e).toMatchObject({
      stopFrame: 13671,
      stopFrameIsMultipleOf128: false,
      expectedCutFrame: 13696,
      observedCutFrame: 13696,
      agrees: true,
    });
    expect(e.ringPeakBeforeCut).toBeGreaterThan(0.1);
  });

  test('disagrees on a different cut, a decayed ring or no cut at all', () => {
    expect(workletTeardownEvidence(tdJob(0.05), ring(13824)).agrees).toBe(false);
    expect(workletTeardownEvidence(tdJob(0.05), ring(13568)).agrees).toBe(false);
    const decayed = ring(13696, 0).map((c) => c.map((v) => v * 1e-4));
    expect(workletTeardownEvidence(tdJob(0.05), decayed).agrees).toBe(false);
    const never = workletTeardownEvidence(tdJob(0.05), ring(20000));
    expect(never.observedCutFrame).toBe(20000);
    expect(never.agrees).toBe(false);
  });

  test('a stop frame on a quantum boundary never agrees (the capture must avoid it)', () => {
    // t + duration + release + 0.01 = 13696 / 44100 s
    const e = workletTeardownEvidence(tdJob(13696 / 44100 - 0.26), ring(13696));
    expect(e.stopFrame).toBe(13696);
    expect(e.stopFrameIsMultipleOf128).toBe(true);
    expect(e.agrees).toBe(false);
  });

  test('the hap must set a numeric release, and there must be one hap', () => {
    expect(() => workletTeardownEvidence(tdJob(undefined), ring(10))).toThrow(/numeric release/);
    expect(() => workletTeardownEvidence(tdJob('x'), ring(10))).toThrow(/numeric release/);
    expect(() => workletTeardownEvidence(job(), ring(10))).toThrow(/needs one hap, got 0/);
  });
});

describe('worklet determinism', () => {
  test('worklet prefixes', () => {
    expect(isWorkletCase('supersaw-detune')).toBe(true);
    expect(isWorkletCase('measure-worklet-start')).toBe(true);
    expect(isWorkletCase('measure-feedback-echo')).toBe(false);
    expect(isWorkletCase('osc-sine')).toBe(false);
  });

  test('identical renders', () => {
    const r = [Float32Array.of(0.1, 0.2)];
    const d = determinismOf('x', [r, [r[0]!.slice()], [r[0]!.slice()]]);
    expect(d.bitEqual).toBe(true);
    expect(d.hashes.length).toBe(3);
    expect(new Set(d.hashes).size).toBe(1);
    expect(d.worstResidualDb).toBeNull();
    expect(d.firstDiffFrame).toBeNull();
  });

  test('a differing render: hashes, worst residual, first differing frame (-0 counts)', () => {
    const a = [Float32Array.of(1, 0, 1, 1)];
    const b = [Float32Array.of(1, -0, 1, 1)];
    const c = [Float32Array.of(1, 0, 1, 0.5)];
    const d = determinismOf('x', [a, b, c]);
    expect(d.bitEqual).toBe(false);
    expect(d.firstDiffFrame).toBe(1);
    expect(d.worstResidualDb).toBe(roundTo(10 * Math.log10(0.25 / 3), 2));
  });

  test('fewer than two renders throw', () => {
    expect(() => determinismOf('x', [[Float32Array.of(1)]])).toThrow(/at least two/);
  });
});

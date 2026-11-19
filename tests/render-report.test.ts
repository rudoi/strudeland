import { describe, expect, test } from 'bun:test';
import type { Features } from '../src/lib/render/features';
import type { GateName, GateResult } from '../src/lib/render/gates/types';
import type { ParityResult } from '../src/lib/render/parity';
import {
  combineResults,
  formatReport,
  goldenSummary,
  reportExitCode,
  reportJson,
  type FixtureReport,
  type MutationReport,
  type Report,
} from '../src/lib/render/report';

const gate = (g: GateName, verdict: GateResult['verdict'], o: Partial<GateResult> = {}) =>
  ({ gate: g, verdict, measurements: {}, reasons: [], warnings: [], ...o }) as GateResult;

const parity = (o: Partial<ParityResult> = {}): ParityResult => ({
  verdict: 'pass',
  profile: 'self',
  frameDelta: 0,
  offsetSec: 0,
  correlation: 1,
  aligned: true,
  peakDeltaDb: 0,
  lufsDeltaDb: 0,
  bandMeanDeltaDb: {},
  bandP95DeltaDb: {},
  hashEqual: true,
  warnings: [],
  reasons: [],
  ...o,
});

const fixture = (o: Partial<FixtureReport> = {}): FixtureReport => ({
  name: 'duck',
  status: 'pass',
  bars: 8,
  chunks: 1,
  seconds: 1.23,
  gates: [gate('twin', 'pass'), gate('mix', 'pass')],
  parity: parity(),
  parityNote: null,
  errors: [],
  unsupported: [],
  ...o,
});

const report = (o: Partial<Report> = {}): Report => ({
  schema: 1,
  startedAt: '2026-11-10T00:00:00.000Z',
  engine: 'superdough',
  engineVersion: null,
  playwright: '1.64.1',
  chromium: '156.0',
  fixtures: [fixture()],
  mutations: [],
  goldens: [],
  harnessError: null,
  ...o,
});

const mutation = (o: Partial<MutationReport> = {}): MutationReport => ({
  name: 'stems-orbit-dropped',
  fixture: 'stems-5',
  gate: 'stems',
  status: 'caught',
  detail: 'orbit 4 missing in bar 1',
  ...o,
});

describe('formatReport', () => {
  test('table layout for a passing fixture', () => {
    const lines = formatReport(report()).split('\n');
    expect(lines[0]).toBe(
      'render:check  engine superdough  playwright 1.64.1  chromium 156.0  2026-11-10T00:00:00.000Z',
    );
    expect(lines[1]).toBe('fixture  bars  chunks  seam  twin  stems  bass  mix   parity  time');
    expect(lines[2]).toBe('duck     8     1       -     pass  -      -     pass  pass    1.2s');
    expect(lines[3]).toBe('1 fixtures: 1 pass, 0 fail');
    expect(lines[4]).toBe('');
  });

  test('columns pad to the longest cell', () => {
    const text = formatReport(
      report({ fixtures: [fixture(), fixture({ name: 'multi-chunk-test', bars: 16, chunks: 2 })] }),
    );
    const lines = text.split('\n');
    expect(lines[1]!.startsWith('fixture           bars')).toBe(true);
    expect(lines[2]!.startsWith('duck              8   ')).toBe(true);
    expect(lines[3]!.startsWith('multi-chunk-test  16  ')).toBe(true);
  });

  test('failing fixture: FAIL cells and one line per reason', () => {
    const f = fixture({
      name: 'pad-seam',
      status: 'fail',
      gates: [
        gate('seam', 'fail', { reasons: ['#0: seam lost material', '#1: again'] }),
        gate('twin', 'pass'),
        gate('mix', 'pass'),
      ],
      parity: parity({ verdict: 'fail', reasons: ['peak delta 1 dB exceeds 0.1 dB'] }),
    });
    const text = formatReport(report({ fixtures: [f] }));
    const row = text.split('\n')[2]!;
    expect(row.split(/\s+/)).toEqual([
      'pad-seam',
      '8',
      '1',
      'FAIL',
      'pass',
      '-',
      '-',
      'pass',
      'FAIL',
      '1.2s',
    ]);
    expect(text).toContain('  pad-seam seam: #0: seam lost material\n  pad-seam seam: #1: again\n');
    expect(text).toContain('  pad-seam parity: peak delta 1 dB exceeds 0.1 dB\n');
    expect(text).toContain('1 fixtures: 0 pass, 1 fail');
  });

  test('unverifiable and not-listed cells', () => {
    const f = fixture({
      name: 'stems-shared',
      gates: [
        gate('twin', 'pass'),
        gate('stems', 'unverifiable', { reasons: ['aliased'] }),
        gate('mix', 'pass'),
      ],
      parity: null,
    });
    const row = formatReport(report({ fixtures: [f] })).split('\n')[2]!;
    expect(row.split(/\s+/)).toEqual([
      'stems-shared',
      '8',
      '1',
      '-',
      'pass',
      'unver',
      '-',
      'pass',
      '-',
      '1.2s',
    ]);
  });

  test('an expected unverifiable gate in a failing fixture does not print its reasons', () => {
    const f = fixture({
      status: 'fail',
      gates: [
        gate('stems', 'unverifiable', { reasons: ['aliased'] }),
        gate('mix', 'fail', { reasons: ['x'] }),
      ],
    });
    const text = formatReport(report({ fixtures: [f] }));
    expect(text).not.toContain('aliased');
    expect(text).toContain('  duck mix: x\n');
  });

  test('render errors print as gate render; parity notes as gate parity', () => {
    const f = fixture({
      status: 'error',
      gates: [],
      parity: null,
      errors: ['eval: boom (bar 3)'],
    });
    const g = fixture({
      name: 'bass-line',
      status: 'fail',
      parity: null,
      parityNote: 'no golden (run with --update-goldens)',
    });
    const text = formatReport(report({ fixtures: [f, g] }));
    expect(text).toContain('  duck render: eval: boom (bar 3)\n');
    expect(text).toContain('  bass-line parity: no golden (run with --update-goldens)\n');
    expect(text).toContain('2 fixtures: 0 pass, 2 fail');
  });

  test('warnings print for a passing fixture, including the advisory hash warning', () => {
    const f = fixture({
      gates: [
        gate('twin', 'pass'),
        gate('mix', 'pass', { warnings: ['peak 1.38 over full scale'] }),
      ],
      parity: parity({
        hashEqual: false,
        warnings: ['pcm hash differs (advisory)'],
        reasons: ['LUFS check skipped'],
      }),
    });
    const text = formatReport(report({ fixtures: [f] }));
    expect(text).toContain('  duck mix warning: peak 1.38 over full scale\n');
    expect(text).toContain('  duck parity warning: pcm hash differs (advisory)\n');
    // a passing fixture's parity notes are not failure reasons
    expect(text).not.toContain('LUFS check skipped');
  });

  test('mutation block highlights misses', () => {
    const text = formatReport(
      report({
        mutations: [
          mutation(),
          mutation({
            name: 'bass-dropped',
            fixture: 'bass-line',
            gate: 'bass',
            status: 'missed',
            detail: 'gate bass returned pass',
          }),
          mutation({
            name: 'seam-lookback-0',
            fixture: 'pad-seam',
            gate: 'seam',
            status: 'skipped',
            detail: 'fixture not selected',
          }),
          mutation({ name: 'x', status: 'error', detail: 'mutated render failed' }),
        ],
      }),
    );
    expect(text).toContain('mutations: 1 caught, 1 missed, 1 error, 1 skipped\n');
    expect(text).toContain(
      '  stems-orbit-dropped  stems-5 stems   caught  orbit 4 missing in bar 1\n',
    );
    expect(text).toContain(
      '  bass-dropped         bass-line bass  MISSED  gate bass returned pass\n',
    );
    expect(text).toContain('ERROR  mutated render failed');
    expect(text).toContain('1 fixtures: 1 pass, 0 fail, 1 mutation(s) missed, 1 mutation error(s)');
  });

  test('golden summary lines and aggregate', () => {
    const text = formatReport(
      report({
        goldens: [
          { name: 'pulse', state: 'new' },
          {
            name: 'multi-chunk-test',
            state: 'changed',
            frames: 0,
            peakDb: 0.02,
            lufs: -0.01,
            bandsMaxDb: 0.04,
            hashFrom: 'a1b2c3d4',
            hashTo: 'e5f6a7b8',
          },
          { name: 'soft-pad-melody', state: 'unchanged' },
          { name: 'bass-test', state: 'not-written' },
          { name: 'odd', state: 'changed', frames: -3, peakDb: 0, lufs: null, bandsMaxDb: 0 },
        ],
      }),
    );
    expect(text).toContain('golden  pulse            new\n');
    expect(text).toContain(
      'golden  multi-chunk-test changed  frames 0  peak +0.02 dB  lufs -0.01 LU  bands max 0.04 dB  hash a1b2c3d4 -> e5f6a7b8\n',
    );
    expect(text).toContain('golden  soft-pad-melody  unchanged\n');
    expect(text).toContain('golden  bass-test        not written (gates failed)\n');
    expect(text).toContain(
      'golden  odd              changed  frames -3  peak +0.00 dB  bands max 0.00 dB\n',
    );
    expect(text).toContain('goldens: 1 new, 2 changed, 1 unchanged, 1 not written\n');
  });

  test('the rust engine and its version in the header', () => {
    const r = report({ engine: 'rust', engineVersion: 'strudeland-engine 0.1.0 (abc1234)' });
    expect(formatReport(r).split('\n')[0]).toBe(
      'render:check  engine rust (strudeland-engine 0.1.0 (abc1234))  playwright 1.64.1  ' +
        'chromium 156.0  2026-11-10T00:00:00.000Z',
    );
  });

  test('an unsupported fixture: no gate cells, unsup parity, its signatures, its own count', () => {
    const u = fixture({
      name: 'worklets',
      status: 'unsupported',
      gates: [],
      parity: null,
      unsupported: ['s=supersaw keys= bar=0', 's=sine keys=shape bar=2'],
    });
    const text = formatReport(
      report({ engine: 'rust', engineVersion: 'v', fixtures: [fixture(), u] }),
    );
    const lines = text.split('\n');
    expect(lines[3]).toBe('worklets  8     1       -     -     -      -     -     unsup   1.2s');
    expect(lines[4]).toBe('  worklets unsupported: s=supersaw keys= bar=0');
    expect(lines[5]).toBe('  worklets unsupported: s=sine keys=shape bar=2');
    expect(text).toContain('2 fixtures: 1 pass, 0 fail, 1 unsupported\n');
    expect(text).not.toContain('disagree');
  });

  test('an unsupported fixture corpus.json marks supported: UNSUP and the disagreement', () => {
    const u = fixture({
      status: 'unsupported',
      gates: [],
      parity: null,
      errors: ['corpus.json marks it supported'],
      unsupported: ['s=sine keys=shape bar=2'],
    });
    const text = formatReport(report({ fixtures: [u] }));
    expect(text).toContain('UNSUP');
    expect(text).toContain('  duck rust: corpus.json marks it supported\n');
    expect(text).toContain(
      '1 fixtures: 0 pass, 0 fail, 1 unsupported (1 disagree with corpus.json)',
    );
  });

  test('a harness error is printed', () => {
    expect(formatReport(report({ harnessError: 'stalled' }))).toContain('harness error: stalled\n');
  });
});

describe('reportExitCode', () => {
  test('0 when everything passes and every mutation is caught or skipped', () => {
    expect(
      reportExitCode(report({ mutations: [mutation(), mutation({ status: 'skipped' })] })),
    ).toBe(0);
  });
  test('1 for a failing or errored fixture', () => {
    expect(reportExitCode(report({ fixtures: [fixture({ status: 'fail' })] }))).toBe(1);
    expect(reportExitCode(report({ fixtures: [fixture({ status: 'error' })] }))).toBe(1);
  });
  test('unsupported: 0 as corpus.json says, 1 when it disagrees', () => {
    const u = (errors: string[]) =>
      fixture({
        status: 'unsupported',
        gates: [],
        parity: null,
        unsupported: ['s=x keys= bar=0'],
        errors,
      });
    expect(reportExitCode(report({ fixtures: [fixture(), u([])] }))).toBe(0);
    expect(reportExitCode(report({ fixtures: [u(['update rust in corpus.json'])] }))).toBe(1);
  });
  test('1 for a missed or errored mutation', () => {
    expect(reportExitCode(report({ mutations: [mutation({ status: 'missed' })] }))).toBe(1);
    expect(reportExitCode(report({ mutations: [mutation({ status: 'error' })] }))).toBe(1);
  });
});

test('reportJson round trips with a trailing newline', () => {
  const r = report({ mutations: [mutation()], goldens: [{ name: 'a', state: 'new' }] });
  const text = reportJson(r);
  expect(text.endsWith('}\n')).toBe(true);
  expect(JSON.parse(text)).toEqual(r);
});

const features = (o: Partial<Features> = {}): Features => ({
  schema: 1,
  sampleRate: 44100,
  frames: 44100,
  windowSec: 1,
  peak: 0.5,
  peakDb: -6.02,
  lufs: -14,
  bandsDb: { sub: [-20, -21], low: [-30, -30], mid: [-40, -40], high: [-50, -50] },
  onset: [0, 1, 0],
  pcmSha256: 'a'.repeat(64),
  ...o,
});

describe('goldenSummary', () => {
  test('new when there is no old golden', () => {
    expect(goldenSummary('x', null, features())).toEqual({ name: 'x', state: 'new' });
  });
  test('unchanged when byte-identical', () => {
    expect(goldenSummary('x', features(), features())).toEqual({ name: 'x', state: 'unchanged' });
  });
  test('changed with deltas as next minus old', () => {
    const old = features();
    const next = features({
      frames: 44102,
      peakDb: -6.0,
      lufs: -14.25,
      bandsDb: { sub: [-20, -21.5, -9], low: [-30, -30], mid: [-40.2, -40], high: [-50, -50] },
      pcmSha256: 'b'.repeat(64),
    });
    expect(goldenSummary('x', old, next)).toEqual({
      name: 'x',
      state: 'changed',
      frames: 2,
      peakDb: 0.02,
      lufs: -0.25,
      bandsMaxDb: 0.5,
      hashFrom: 'aaaaaaaa',
      hashTo: 'bbbbbbbb',
    });
  });
  test('lufs is null when either side is null; equal hashes are omitted', () => {
    const g = goldenSummary('x', features({ lufs: null }), features({ onset: [0, 2, 0] }));
    expect(g.state).toBe('changed');
    expect(g.lufs).toBeNull();
    expect(g.hashFrom).toBeUndefined();
    expect(g.hashTo).toBeUndefined();
    expect(g.bandsMaxDb).toBe(0);
  });
});

describe('combineResults', () => {
  test('fail wins', () => {
    const r = combineResults('seam', [
      gate('seam', 'pass'),
      gate('seam', 'fail', { reasons: ['lost'] }),
      gate('seam', 'unverifiable'),
    ]);
    expect(r.verdict).toBe('fail');
    expect(r.gate).toBe('seam');
  });
  test('pass over unverifiable', () => {
    expect(
      combineResults('seam', [gate('seam', 'unverifiable'), gate('seam', 'pass')]).verdict,
    ).toBe('pass');
    expect(combineResults('seam', [gate('seam', 'unverifiable')]).verdict).toBe('unverifiable');
  });
  test('empty list is unverifiable with reason no results', () => {
    const r = combineResults('seam', []);
    expect(r.verdict).toBe('unverifiable');
    expect(r.reasons).toEqual(['no results']);
    expect(r.measurements).toEqual({ parts: [] });
  });
  test('reasons and warnings carry #i: prefixes; measurements are the parts', () => {
    const r = combineResults('seam', [
      gate('seam', 'pass', { warnings: ['w0'], measurements: { a: 1 } }),
      gate('seam', 'fail', { reasons: ['r1a', 'r1b'], warnings: ['w1'], measurements: { a: 2 } }),
    ]);
    expect(r.reasons).toEqual(['#1: r1a', '#1: r1b']);
    expect(r.warnings).toEqual(['#0: w0', '#1: w1']);
    expect(r.measurements).toEqual({ parts: [{ a: 1 }, { a: 2 }] });
  });
});

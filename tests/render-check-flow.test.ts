import { basename, join, normalize } from 'node:path';
import { describe, expect, test } from 'bun:test';
import type { EngineName } from '../src/lib/render/engines';
import { DEFAULT_OUT_DIR, type CheckOptions } from '../src/lib/render/check-options';
import {
  chunkRequest,
  planChunks,
  type AssembledWindow,
  renderChunkPlans,
  stochasticCrossings,
} from '../src/lib/render/chunks';
import {
  expectedFrames,
  windowFrames,
  type RenderError,
  type RenderRequest,
  type RenderResult,
  type Renderer,
  type ScheduleResult,
} from '../src/lib/render/contract';
import type { Corpus, FixtureEntry } from '../src/lib/render/corpus';
import type { UnsupportedSignature } from '../src/lib/render/engine-job';
import { lookbackBars, lookbackSeconds, scaleChunkBars } from '../src/lib/render/lookback';
import type { RenderEvent } from '../src/lib/render/schedule';
import { buildStructure } from '../src/lib/strudel/structure';
import {
  CHUNK_MIN_BARS,
  SEAM_OVERLAP_BARS,
  TWIN_BITEXACT_HOLDS,
} from '../src/lib/render/thresholds';
import {
  applyExpectations,
  concatChannels,
  Limiter,
  manifestDisagreement,
  prepareFixture,
  runCheck,
  twinMode,
  type CheckDeps,
  type CheckFs,
  type FixtureContext,
} from '../scripts/render/check';
import type { MutationSpec } from '../scripts/render/mutations';
import {
  formatRenderError,
  HarnessError,
  RenderFailedError,
  type HarnessErrorCode,
} from '../scripts/render/transfer';
import { decodeWav } from '../src/lib/render/wav';
import { ev } from './helpers/render-events';
import { noiseSeeded, SR } from './helpers/render-signals';

// ---- the fake song: cps 2 (0.5 s bars), a 55 Hz bass note on every bar, seeded noise bursts.
// The fake renders one global signal, so chunks join seamlessly and twins are bit-equal.

const CPS = 2;
const BAR_SEC = 1 / CPS;
const GLOBAL_SEC = 12;
const BURSTS = (() => {
  const n = GLOBAL_SEC * SR;
  const x = noiseSeeded(3, n, 0.2);
  const every = Math.round(0.25 * SR);
  for (let i = 0; i < n; i++) x[i] = x[i]! * Math.exp(-(i % every) / (0.03 * SR));
  return x;
})();
/** Head of a chunk silenced when it is rendered with less lookback than this (a lost tail). */
const NEEDS_LOOKBACK_SEC = 0.25;

const bassEvents = (bars: number): RenderEvent[] =>
  Array.from({ length: bars }, (_, b) =>
    ev({ tSec: b * BAR_SEC, durSec: BAR_SEC, bar: b, orbit: 2, freq: 55, note: 33, gain: 0.5 }),
  );

interface FakeOptions {
  events?: RenderEvent[];
  scheduleErrors?: RenderError[];
  renderErrors?: RenderError[];
  /** Throw a HarnessError (code `harnessCode`, default 'chromium-missing') when scheduling this
   *  song code. */
  harnessErrorOn?: string;
  harnessCode?: HarnessErrorCode;
  /** Throw a HarnessError with this code from every render of this song code. */
  renderHarnessErrorOn?: { code: string; error: HarnessErrorCode };
  gain?: number;
  pairs?: number;
  delayMs?: number;
  /** Signatures `schedule` reports (as the Rust renderer does). */
  unsupported?: UnsupportedSignature[];
  /** Which engine the fake claims to be (default superdough). */
  engine?: EngineName;
}

class FakeRenderer implements Renderer {
  get engine(): EngineName {
    return this.o.engine ?? 'superdough';
  }
  schedules: RenderRequest[] = [];
  renders: RenderRequest[] = [];
  inFlight = 0;
  maxInFlight = 0;
  constructor(readonly o: FakeOptions = {}) {}

  private async busy<T>(f: () => T): Promise<T> {
    this.inFlight++;
    this.maxInFlight = Math.max(this.maxInFlight, this.inFlight);
    try {
      await new Promise((r) => setTimeout(r, this.o.delayMs ?? 0));
      return f();
    } finally {
      this.inFlight--;
    }
  }

  schedule(req: RenderRequest): Promise<ScheduleResult> {
    this.schedules.push(req);
    return this.busy(() => {
      if (this.o.harnessErrorOn === req.code) {
        throw new HarnessError(this.o.harnessCode ?? 'chromium-missing', 'no browser');
      }
      return {
        events: this.o.events ?? bassEvents(14),
        cps: CPS,
        structure: buildStructure({}),
        errors: this.o.scheduleErrors ?? [],
        ...(this.o.unsupported ? { unsupported: this.o.unsupported } : {}),
      };
    });
  }

  render(req: RenderRequest): Promise<RenderResult> {
    this.renders.push(req);
    return this.busy(() => {
      const h = this.o.renderHarnessErrorOn;
      if (h && h.code === req.code) throw new HarnessError(h.error, `page ${h.error}`);
      return this.make(req);
    });
  }

  private make(req: RenderRequest): RenderResult {
    const lb = Math.min(
      typeof req.lookbackSec === 'number' ? req.lookbackSec : 0,
      req.fromBar / CPS,
    );
    const head = Math.round(lb * SR);
    const frames = expectedFrames(req, CPS, lb);
    const start = Math.round((req.fromBar * SR) / CPS) - head;
    const gain = this.o.gain ?? 1;
    const m = req.mutation;
    const mono = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      const abs = start + i;
      const t = abs / SR;
      const bar = Math.floor(t * CPS + 1e-9);
      const dropBass =
        m?.kind === 'dropNotesBelowHz' && (m.bars === undefined || m.bars.includes(bar));
      let v = (dropBass ? 0 : 0.3 * Math.sin(2 * Math.PI * 55 * t)) + BURSTS[abs]!;
      const lostTail =
        req.fromBar > 0 &&
        lb < NEEDS_LOOKBACK_SEC &&
        i >= head &&
        i - head < NEEDS_LOOKBACK_SEC * SR;
      if (lostTail) v = 0;
      mono[i] = v * gain;
    }
    const pairs = req.mode === 'stems' ? (this.o.pairs ?? 1) : 1;
    const channels = Array.from({ length: 2 * pairs }, () => mono.slice());
    const errors = this.o.renderErrors ?? [];
    return {
      channels,
      sampleRate: SR,
      frames,
      windowStartFrame: head,
      lookbackSecUsed: lb,
      bufferStartSec: req.fromBar / CPS - lb,
      cps: CPS,
      // deliberately empty: the gates must use the harness schedule
      events: [],
      stemPairs: pairs,
      errors,
      ok: errors.length === 0,
      timing: { evalMs: 0, renderMs: 0, totalMs: 0 },
      request: req,
    };
  }

  async dispose(): Promise<void> {}
}

const fixture = (o: Partial<FixtureEntry> = {}): FixtureEntry => ({
  name: 'a',
  file: 'a.js',
  kind: 'targeted',
  windowBars: [0, 12],
  layout: 'linear',
  gates: ['seam', 'twin', 'bass', 'mix'],
  expectChunks: 2,
  stochastic: false,
  usesSamples: false,
  rust: 'supported',
  tags: [],
  ...o,
});

const corpusOf = (...fixtures: FixtureEntry[]): Corpus => ({
  schema: 1,
  samples: ['kick.wav', 'hat.wav'],
  fixtures,
});

const opts = (o: Partial<CheckOptions> = {}): CheckOptions => ({
  engine: 'superdough',
  planner: 'page',
  filters: [],
  updateGoldens: false,
  keepWav: false,
  mutations: false,
  concurrency: 1,
  rebuild: false,
  outDir: '/out',
  help: false,
  ...o,
});

class MemFs implements CheckFs {
  files = new Map<string, string | Uint8Array>();
  readText(p: string): string | null {
    const v = this.files.get(normalize(p));
    return typeof v === 'string' ? v : null;
  }
  writeText(p: string, text: string): void {
    this.files.set(normalize(p), text);
  }
  writeBytes(p: string, bytes: Uint8Array): void {
    this.files.set(normalize(p), bytes);
  }
}

const deps = (
  renderer: Renderer,
  corpus: Corpus,
  o: Partial<CheckDeps> = {},
): CheckDeps & { fs: MemFs } => {
  let t = 0;
  return {
    renderer,
    engineVersion: '156.0-fake',
    rustVersion: null,
    playwright: '1.64.1',
    corpus,
    readSong: (file) => `// ${file}`,
    fs: new MemFs(),
    clock: { now: () => (t += 500), iso: () => '2026-11-10T00:00:00.000Z' },
    goldensDir: '/goldens',
    ...o,
  } as CheckDeps & { fs: MemFs };
};

/** A run with --update-goldens first, so the parity step has a golden. */
async function withGoldens(renderer: Renderer, corpus: Corpus) {
  const d = deps(renderer, corpus);
  const first = await runCheck(opts({ updateGoldens: true }), d);
  expect(first.exitCode).toBe(0);
  return d;
}

const gateOf = (r: Awaited<ReturnType<typeof runCheck>>, name: string, gate: string) =>
  r.report.fixtures.find((f) => f.name === name)!.gates.find((g) => g.gate === gate);

const BASE: RenderRequest = {
  code: '// a.js',
  layout: 'linear',
  fromBar: 0,
  toBar: 12,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: 44100,
};

describe('runCheck', () => {
  test('a fixture passes against its golden', async () => {
    const r = new FakeRenderer();
    const d = await withGoldens(r, corpusOf(fixture()));
    const run = await runCheck(opts(), d);
    expect(run.exitCode).toBe(0);
    const f = run.report.fixtures[0]!;
    expect(f).toMatchObject({
      name: 'a',
      status: 'pass',
      bars: 12,
      chunks: 2,
      parityNote: null,
      errors: [],
    });
    expect(f.gates.map((g) => [g.gate, g.verdict])).toEqual([
      ['seam', 'pass'],
      ['twin', 'pass'],
      ['bass', 'pass'],
      ['mix', 'pass'],
    ]);
    expect(f.parity?.verdict).toBe('pass');
    expect(f.parity?.hashEqual).toBe(true);
    expect(run.report).toMatchObject({
      playwright: '1.64.1',
      chromium: '156.0-fake',
      harnessError: null,
    });
    expect(JSON.parse(d.fs.readText('/out/report.json')!)).toEqual(run.report);
  });

  test('the plan comes from schedule and each chunk request is chunkRequest(base, plan)', async () => {
    const events = bassEvents(14);
    const r = new FakeRenderer({ events });
    await runCheck(
      opts({ updateGoldens: true }),
      deps(r, corpusOf(fixture({ gates: ['mix'], expectChunks: undefined }))),
    );
    expect(r.schedules).toEqual([BASE]);
    const lb = lookbackBars(lookbackSeconds(events), BAR_SEC);
    const plans = planChunks({
      fromBar: 0,
      toBar: 12,
      chunkBars: scaleChunkBars(CHUNK_MIN_BARS, lb),
      lookbackBars: lb,
      barSec: BAR_SEC,
      avoidSeamBars: stochasticCrossings(events, BAR_SEC),
    });
    expect(plans.length).toBe(2);
    expect(r.renders).toEqual(plans.map((p) => chunkRequest(BASE, p)));
    expect(r.renders[1]).toMatchObject({
      fromBar: 8,
      toBar: 12,
      overlapBars: 0,
      lookbackSec: lb * BAR_SEC,
    });
    expect(r.renders[0]).toMatchObject({ fromBar: 0, toBar: 8, overlapBars: 1, lookbackSec: 0 });
  });

  test('a fixture with usesSamples passes the corpus samples map', async () => {
    const r = new FakeRenderer();
    await runCheck(
      opts({ updateGoldens: true }),
      deps(r, corpusOf(fixture({ usesSamples: true, gates: ['mix'], expectChunks: undefined }))),
    );
    expect(r.schedules[0]!.samples).toEqual({ kick: 'kick.wav', hat: 'hat.wav' });
    expect(r.renders.every((q) => q.samples?.kick === 'kick.wav')).toBe(true);
  });

  test('schedule errors fail the fixture without rendering or gates', async () => {
    const r = new FakeRenderer({ scheduleErrors: [{ kind: 'eval', message: 'boom', bar: 3 }] });
    const run = await runCheck(opts(), deps(r, corpusOf(fixture())));
    expect(run.exitCode).toBe(1);
    expect(r.renders).toEqual([]);
    expect(run.report.fixtures[0]).toMatchObject({
      status: 'error',
      gates: [],
      parity: null,
      errors: ['eval: boom (bar 3)'],
    });
  });

  test('an ok: false render fails the fixture without gates', async () => {
    const r = new FakeRenderer({ renderErrors: [{ kind: 'worklet', message: 'processor died' }] });
    const run = await runCheck(opts(), deps(r, corpusOf(fixture())));
    expect(run.exitCode).toBe(1);
    expect(r.renders.length).toBe(1);
    expect(run.report.fixtures[0]).toMatchObject({
      status: 'error',
      gates: [],
      errors: ['worklet: processor died'],
    });
  });

  test('a missing golden fails; --update-goldens writes it and judges no parity', async () => {
    const r = new FakeRenderer();
    const d = deps(r, corpusOf(fixture()));
    const missing = await runCheck(opts(), d);
    expect(missing.exitCode).toBe(1);
    expect(missing.report.fixtures[0]).toMatchObject({
      status: 'fail',
      parity: null,
      parityNote: 'no golden (run with --update-goldens)',
    });
    const update = await runCheck(opts({ updateGoldens: true }), d);
    expect(update.exitCode).toBe(0);
    expect(update.report.fixtures[0]).toMatchObject({ status: 'pass', parity: null });
    expect(update.report.goldens).toEqual([{ name: 'a', state: 'new' }]);
    expect(d.fs.readText('/goldens/a.json')).toContain('"pcmSha256"');
    const again = await runCheck(opts({ updateGoldens: true }), d);
    expect(again.report.goldens).toEqual([{ name: 'a', state: 'unchanged' }]);
  });

  test('a gate-failing fixture is not written under --update-goldens', async () => {
    const r = new FakeRenderer();
    const d = deps(r, corpusOf(fixture({ expectChunks: 3 })));
    const run = await runCheck(opts({ updateGoldens: true }), d);
    expect(run.exitCode).toBe(1);
    expect(run.report.goldens).toEqual([{ name: 'a', state: 'not-written' }]);
    expect(d.fs.readText('/goldens/a.json')).toBeNull();
  });

  test('an unreadable golden fails the fixture', async () => {
    const r = new FakeRenderer();
    const d = deps(r, corpusOf(fixture()));
    d.fs.writeText('/goldens/a.json', '{"schema": 9}');
    const run = await runCheck(opts(), d);
    expect(run.exitCode).toBe(1);
    expect(run.report.fixtures[0]!.parityNote).toStartWith('golden a.json: unsupported schema');
  });

  test('an expectChunks shortfall fails seam without running it', async () => {
    const r = new FakeRenderer();
    const run = await runCheck(
      opts({ updateGoldens: true }),
      deps(r, corpusOf(fixture({ expectChunks: 3 }))),
    );
    const seam = gateOf(run, 'a', 'seam')!;
    expect(seam.verdict).toBe('fail');
    expect(seam.reasons).toEqual(['planned 2 chunk(s), expected >= 3']);
    // chunks plus the twin re-render only: no seam head re-render
    expect(r.renders.length).toBe(3);
  });

  test('the twin gate renders chunk 0 twice and picks its mode', async () => {
    for (const stochastic of [false, true]) {
      const r = new FakeRenderer();
      const run = await runCheck(
        opts({ updateGoldens: true }),
        deps(r, corpusOf(fixture({ gates: ['twin'], expectChunks: undefined, stochastic }))),
      );
      const chunk0 = JSON.stringify(r.renders[0]);
      expect(r.renders.filter((q) => JSON.stringify(q) === chunk0).length).toBe(2);
      expect(r.renders[0]!.fromBar).toBe(0);
      const twin = gateOf(run, 'a', 'twin')!;
      expect(twin.verdict).toBe('pass');
      expect(twin.measurements.mode).toBe(twinMode(TWIN_BITEXACT_HOLDS, stochastic, 'superdough'));
    }
  });

  test('twinMode', () => {
    expect(twinMode(true, false, 'superdough')).toBe('exact');
    expect(twinMode(true, true, 'superdough')).toBe('tolerance');
    expect(twinMode(false, false, 'superdough')).toBe('tolerance');
    expect(twinMode(false, true, 'superdough')).toBe('tolerance');
    for (const holds of [true, false]) {
      for (const stochastic of [true, false]) {
        expect(twinMode(holds, stochastic, 'rust')).toBe('exact');
      }
    }
  });

  test('stems renders each chunk in stems mode without overlap', async () => {
    const r = new FakeRenderer({ pairs: 2, events: [ev({ tSec: 0, attack: 0.5, orbit: 1 })] });
    const fx = fixture({
      gates: ['stems'],
      expectChunks: undefined,
      expect: { unverifiable: ['stems'] },
    });
    const run = await runCheck(opts({ updateGoldens: true }), deps(r, corpusOf(fx)));
    const stems = r.renders.filter((q) => q.mode === 'stems');
    expect(stems.map((q) => [q.fromBar, q.toBar, q.overlapBars])).toEqual([
      [0, 8, 0],
      [8, 12, 0],
    ]);
    // no fast-attack note: unverifiable, which this fixture expects
    expect(gateOf(run, 'a', 'stems')!.verdict).toBe('unverifiable');
    expect(run.report.fixtures[0]!.status).toBe('pass');
  });

  test('an unverifiable gate the manifest did not expect fails', async () => {
    const r = new FakeRenderer({ events: [ev({ tSec: 0, attack: 0.5, orbit: 1 })] });
    const run = await runCheck(
      opts({ updateGoldens: true }),
      deps(r, corpusOf(fixture({ gates: ['stems'], expectChunks: undefined }))),
    );
    const stems = gateOf(run, 'a', 'stems')!;
    expect(stems.verdict).toBe('fail');
    expect(stems.reasons[0]).toStartWith('unverifiable: ');
    expect(run.exitCode).toBe(1);
  });

  test('an expected-unverifiable gate that verifies fails', async () => {
    const r = new FakeRenderer();
    const run = await runCheck(
      opts({ updateGoldens: true }),
      deps(
        r,
        corpusOf(
          fixture({ gates: ['bass'], expectChunks: undefined, expect: { unverifiable: ['bass'] } }),
        ),
      ),
    );
    const bass = gateOf(run, 'a', 'bass')!;
    expect(bass.verdict).toBe('fail');
    expect(bass.reasons).toContain('expected unverifiable');
  });

  test('expect.clip: fails without clipping, passes (with a warning) with it', async () => {
    const fx = fixture({ gates: ['mix'], expectChunks: undefined, expect: { clip: true } });
    const clean = await runCheck(
      opts({ updateGoldens: true }),
      deps(new FakeRenderer(), corpusOf(fx)),
    );
    expect(gateOf(clean, 'a', 'mix')).toMatchObject({ verdict: 'fail' });
    expect(gateOf(clean, 'a', 'mix')!.reasons).toContain('expected clipping');
    const hot = await runCheck(
      opts({ updateGoldens: true }),
      deps(new FakeRenderer({ gain: 4 }), corpusOf(fx)),
    );
    const mix = gateOf(hot, 'a', 'mix')!;
    expect(mix.verdict).toBe('pass');
    expect(mix.warnings.some((w) => w.includes('over full scale'))).toBe(true);
  });

  test('a mutation reaches every chunk request and the gates keep the unmutated events', async () => {
    const spec: MutationSpec = {
      name: 'bass-gone',
      fixture: 'a',
      gate: 'bass',
      // four bars: the first dropped note still rings from the bar before it
      apply: (q) => ({ ...q, mutation: { kind: 'dropNotesBelowHz', hz: 130, bars: [3, 4, 5, 6] } }),
    };
    const r = new FakeRenderer();
    const d = await withGoldens(r, corpusOf(fixture()));
    r.renders = [];
    const run = await runCheck(opts({ mutations: true }), { ...d, mutations: [spec] });
    const mutated = r.renders.filter((q) => q.mutation !== undefined);
    expect(mutated.map((q) => [q.fromBar, q.toBar])).toEqual([
      [0, 8],
      [8, 12],
    ]);
    expect(mutated.every((q) => q.mutation?.kind === 'dropNotesBelowHz')).toBe(true);
    // the fake's results carry no events: a caught bass mutation proves the schedule was used
    expect(run.report.mutations).toEqual([
      expect.objectContaining({ name: 'bass-gone', status: 'caught' }),
    ]);
    expect(run.exitCode).toBe(0);
  });

  test('seam lookback mutation is caught through the twin-floor escalation', async () => {
    const spec: MutationSpec = {
      name: 'seam-lookback-0',
      fixture: 'a',
      gate: 'seam',
      apply: (q) => ({ ...q, lookbackSec: 0 }),
    };
    const r = new FakeRenderer();
    const d = await withGoldens(r, corpusOf(fixture()));
    r.renders = [];
    const run = await runCheck(opts({ mutations: true }), { ...d, mutations: [spec] });
    expect(run.report.mutations[0]).toMatchObject({ status: 'caught' });
    // chunk 1 rendered twice with lookback 0 (the chunk, then the escalation re-render)
    const zeroed = r.renders.filter((q) => q.fromBar === 8 && q.lookbackSec === 0);
    expect(zeroed.length).toBe(2);
  });

  test('a mutation the gate does not see is missed; unselected and failing controls are skipped', async () => {
    const noop: MutationSpec = {
      name: 'noop',
      fixture: 'a',
      gate: 'bass',
      apply: (q) => ({ ...q, mutation: { kind: 'dropOrbit', orbit: 9, bars: [1] } }),
    };
    const other: MutationSpec = { ...noop, name: 'other', fixture: 'b' };
    const failing: MutationSpec = { ...noop, name: 'failing', gate: 'seam' };
    const corpus = corpusOf(fixture({ expectChunks: 3 }), fixture({ name: 'b', file: 'b.js' }));
    const run = await runCheck(opts({ mutations: true, filters: ['a'], updateGoldens: true }), {
      ...deps(new FakeRenderer(), corpus),
      mutations: [noop, other, failing],
    });
    expect(run.report.mutations.map((m) => [m.name, m.status, m.detail])).toEqual([
      ['noop', 'missed', 'gate bass returned pass; the known defect was not detected'],
      ['other', 'skipped', 'fixture not selected'],
      ['failing', 'skipped', 'control did not pass'],
    ]);
    expect(run.exitCode).toBe(1);
  });

  test('--no-mutations runs none', async () => {
    const r = new FakeRenderer();
    const run = await runCheck(opts({ updateGoldens: true }), deps(r, corpusOf(fixture())));
    expect(run.report.mutations).toEqual([]);
  });

  test('an environment HarnessError exits 2 with a partial report', async () => {
    const r = new FakeRenderer({ harnessErrorOn: '// b.js' });
    const d = deps(
      r,
      corpusOf(
        fixture(),
        fixture({ name: 'b', file: 'b.js' }),
        fixture({ name: 'c', file: 'c.js' }),
      ),
    );
    const run = await runCheck(opts({ updateGoldens: true }), d);
    expect(run.exitCode).toBe(2);
    expect(run.report.harnessError).toBe('no browser');
    expect(run.report.fixtures.map((f) => f.name)).toEqual(['a']);
    expect(r.schedules.map((q) => q.code)).toEqual(['// a.js', '// b.js']);
    expect(JSON.parse(d.fs.readText('/out/report.json')!).harnessError).toBe('no browser');
  });

  for (const code of ['eval-failed', 'stalled'] as const) {
    test(`a page ${code} while scheduling fails that fixture only; the run continues`, async () => {
      const r = new FakeRenderer({ harnessErrorOn: '// b.js', harnessCode: code });
      const d = deps(
        r,
        corpusOf(
          fixture(),
          fixture({ name: 'b', file: 'b.js' }),
          fixture({ name: 'c', file: 'c.js' }),
        ),
      );
      const run = await runCheck(opts({ updateGoldens: true }), d);
      expect(run.exitCode).toBe(1);
      expect(run.report.harnessError).toBeNull();
      expect(run.report.fixtures.map((f) => [f.name, f.status])).toEqual([
        ['a', 'pass'],
        ['b', 'error'],
        ['c', 'pass'],
      ]);
      expect(run.report.fixtures[1]!.errors).toEqual([`${code}: no browser`]);
    });
  }

  test('a page render-failed fails that fixture only; its mutation reports the error', async () => {
    const r = new FakeRenderer({
      renderHarnessErrorOn: { code: '// b.js', error: 'render-failed' },
    });
    const d = deps(r, corpusOf(fixture(), fixture({ name: 'b', file: 'b.js' })));
    const run = await runCheck(opts({ updateGoldens: true }), d);
    expect(run.exitCode).toBe(1);
    expect(run.report.harnessError).toBeNull();
    expect(run.report.fixtures.map((f) => f.status)).toEqual(['pass', 'error']);
    expect(run.report.fixtures[1]!.errors).toEqual(['render-failed: page render-failed']);
  });

  test("a stalled mutated render is that mutation's error, not a harness error", async () => {
    const r = new FakeRenderer();
    const spec: MutationSpec = {
      name: 'stall',
      fixture: 'a',
      gate: 'bass',
      apply: (q) => ({ ...q, code: '// stall' }),
    };
    r.o.renderHarnessErrorOn = { code: '// stall', error: 'stalled' };
    const d = deps(r, corpusOf(fixture()), { mutations: [spec] });
    const run = await runCheck(opts({ updateGoldens: true, mutations: true }), d);
    expect(run.report.harnessError).toBeNull();
    expect(run.exitCode).not.toBe(2);
    expect(run.report.mutations).toHaveLength(1);
    expect(run.report.mutations[0]!.detail).toContain('stalled: page stalled');
  });

  test('fixtures run concurrently and report in manifest order', async () => {
    const r = new FakeRenderer({ delayMs: 5 });
    const names = ['a', 'b', 'c', 'd'];
    const corpus = corpusOf(
      ...names.map((n) =>
        fixture({ name: n, file: `${n}.js`, gates: ['mix'], expectChunks: undefined }),
      ),
    );
    const lines: string[] = [];
    const run = await runCheck(opts({ updateGoldens: true, concurrency: 3 }), {
      ...deps(r, corpus),
      log: (l) => lines.push(l),
    });
    expect(r.maxInFlight).toBe(3);
    expect(run.report.fixtures.map((f) => f.name)).toEqual(names);
    expect(lines.filter((l) => l.startsWith('start ')).length).toBe(4);
    expect(lines.filter((l) => /^\[\d\/4\] /.test(l)).length).toBe(4);
  });

  test("a fixture's chunks and twin render at once, each chunk once, within --concurrency", async () => {
    const r = new FakeRenderer({ delayMs: 5 });
    const run = await runCheck(
      opts({ updateGoldens: true, concurrency: 4 }),
      deps(r, corpusOf(fixture())),
    );
    expect(run.exitCode).toBe(0);
    // two chunks and the twin re-render together; seam, bass and mix reuse the chunks
    expect(r.renders.length).toBe(3);
    expect(r.maxInFlight).toBe(3);
  });

  test('mutations overlap the remaining fixtures and stay within --concurrency', async () => {
    const r = new FakeRenderer({ delayMs: 5 });
    const spec: MutationSpec = {
      name: 'seam-lookback-0',
      fixture: 'a',
      gate: 'seam',
      apply: (q) => ({ ...q, lookbackSec: 0 }),
    };
    const corpus = corpusOf(fixture(), fixture({ name: 'b', file: 'b.js' }));
    const d = await withGoldens(r, corpus);
    r.maxInFlight = 0;
    const lines: string[] = [];
    const run = await runCheck(opts({ mutations: true, concurrency: 2 }), {
      ...d,
      mutations: [spec],
      log: (l) => lines.push(l),
    });
    expect(run.report.mutations).toEqual([
      expect.objectContaining({ name: 'seam-lookback-0', status: 'caught' }),
    ]);
    expect(r.maxInFlight).toBe(2);
    // the mutation starts before fixture b is done
    expect(lines.indexOf('mutation seam-lookback-0 on a')).toBeLessThan(
      lines.findIndex((l) => l.startsWith('[2/2] ')),
    );
  });

  test('a filter that matches nothing is a harness error', async () => {
    const run = await runCheck(
      opts({ filters: ['zzz'] }),
      deps(new FakeRenderer(), corpusOf(fixture())),
    );
    expect(run.exitCode).toBe(2);
    expect(run.report.harnessError).toBe(`no fixture matches 'zzz'`);
  });

  test('--keep-wav writes the mix and the stems WAVs', async () => {
    const r = new FakeRenderer({ events: [ev({ tSec: 0, attack: 0.5, orbit: 1 })] });
    const fx = fixture({
      gates: ['stems', 'mix'],
      expectChunks: undefined,
      expect: { unverifiable: ['stems'] },
    });
    const d = deps(r, corpusOf(fx));
    await runCheck(opts({ updateGoldens: true, keepWav: true, outDir: DEFAULT_OUT_DIR }), d);
    const mix = d.fs.files.get(join(DEFAULT_OUT_DIR, 'wav', 'a.wav')) as Uint8Array;
    const stems = d.fs.files.get(join(DEFAULT_OUT_DIR, 'wav', 'a.stems.wav')) as Uint8Array;
    // 12 bars at 0.5 s, float32 stereo, 44-byte header
    expect(mix.length).toBe(44 + 12 * BAR_SEC * SR * 2 * 4);
    expect(stems.length).toBe(44 + 12 * BAR_SEC * SR * 2 * 4);
  });

  test("a --keep-wav write error is that fixture's error; the run continues", async () => {
    const r = new FakeRenderer();
    const d = deps(r, corpusOf(fixture(), fixture({ name: 'b', file: 'b.js' })));
    const write = d.fs.writeBytes.bind(d.fs);
    d.fs.writeBytes = (p, bytes) => {
      if (basename(p) === 'a.wav') throw new Error('disk full');
      write(p, bytes);
    };
    const run = await runCheck(opts({ updateGoldens: true, keepWav: true }), d);
    expect(run.exitCode).toBe(1);
    expect(run.report.harnessError).toBeNull();
    expect(run.report.fixtures.map((f) => f.status)).toEqual(['error', 'pass']);
    expect(run.report.fixtures[0]!.errors).toEqual(['keep-wav: disk full']);
    expect(d.fs.files.has(join('/out', 'wav', 'b.wav'))).toBe(true);
  });
});

describe('concatChannels', () => {
  const ch = (...v: number[]) => Float32Array.from(v);
  test('joins chunks end to end', () => {
    expect(
      concatChannels([
        [ch(1, 2), ch(3)],
        [ch(4), ch(5, 6)],
      ]),
    ).toEqual([ch(1, 2, 4, 0), ch(3, 0, 5, 6)]);
  });
  test('chunks with fewer channels contribute silence of their length', () => {
    const out = concatChannels([
      [ch(1, 1), ch(2, 2)],
      [ch(3, 3, 3), ch(4, 4, 4), ch(5, 5, 5), ch(6, 6, 6)],
      [ch(7), ch(8)],
    ]);
    expect(out).toEqual([
      ch(1, 1, 3, 3, 3, 7),
      ch(2, 2, 4, 4, 4, 8),
      ch(0, 0, 5, 5, 5, 0),
      ch(0, 0, 6, 6, 6, 0),
    ]);
  });
  test('no chunks, no channels', () => {
    expect(concatChannels([])).toEqual([]);
  });
});

describe('Limiter', () => {
  test('runs at most n tasks at once, in call order', async () => {
    const limit = new Limiter(2);
    let active = 0;
    let max = 0;
    const started: number[] = [];
    const task = (i: number) =>
      limit.run(async () => {
        started.push(i);
        max = Math.max(max, ++active);
        await new Promise((r) => setTimeout(r, 2));
        active--;
        return i;
      });
    expect(await Promise.all([0, 1, 2, 3, 4].map(task))).toEqual([0, 1, 2, 3, 4]);
    expect(started).toEqual([0, 1, 2, 3, 4]);
    expect(max).toBe(2);
  });

  test('a failing task frees its slot', async () => {
    const limit = new Limiter(1);
    await expect(limit.run(async () => Promise.reject(new Error('x')))).rejects.toThrow('x');
    expect(await limit.run(async () => 1)).toBe(1);
  });
});

describe('applyExpectations', () => {
  const g = (verdict: 'pass' | 'fail' | 'unverifiable', clipFraction: number | null = 0) => ({
    gate: 'mix' as const,
    verdict,
    measurements: { clipFraction },
    reasons: verdict === 'pass' ? [] : ['why'],
    warnings: [],
  });
  test('passes through a pass and a fail', () => {
    expect(applyExpectations(fixture(), g('pass'))).toEqual(g('pass'));
    expect(applyExpectations(fixture(), g('fail'))).toEqual(g('fail'));
  });
  test('an unexpected unverifiable fails with prefixed reasons', () => {
    expect(applyExpectations(fixture(), g('unverifiable'))).toMatchObject({
      verdict: 'fail',
      reasons: ['unverifiable: why'],
    });
  });
  test('expect.clip needs a clip fraction above 0', () => {
    const fx = fixture({ expect: { clip: true } });
    expect(applyExpectations(fx, g('pass', 0.28)).verdict).toBe('pass');
    expect(applyExpectations(fx, g('pass', null))).toMatchObject({
      verdict: 'fail',
      reasons: ['expected clipping'],
    });
  });
});

test('formatRenderError', () => {
  expect(formatRenderError({ kind: 'hap', message: 'bad' })).toBe('hap: bad');
  expect(formatRenderError({ kind: 'sample', message: 'missing', bar: 0 })).toBe(
    'sample: missing (bar 0)',
  );
});

// ---- --engine rust

const RUST_VERSION = 'strudeland-engine 0.1.0 (abc1234)';
const SIG: UnsupportedSignature = { s: 'supersaw', keys: ['shape'], firstBar: 2 };

/** A fake that reports itself as the Rust engine. */
const rustFake = (o: FakeOptions = {}) => new FakeRenderer({ ...o, engine: 'rust' });

/** A rust run over `corpus`, after superdough goldens were written for it. */
async function rustRun(
  renderer: FakeRenderer,
  corpus: Corpus,
  o: Partial<CheckOptions> = {},
  mutations?: MutationSpec[],
) {
  const d = await withGoldens(new FakeRenderer(), corpus);
  return runCheck(opts({ engine: 'rust', ...o }), {
    ...d,
    renderer,
    rustVersion: RUST_VERSION,
    ...(mutations ? { mutations } : {}),
  });
}

describe('runCheck --engine rust', () => {
  test('a supported fixture passes with the engine profile; the report names the build', async () => {
    const run = await rustRun(rustFake(), corpusOf(fixture()));
    expect(run.exitCode).toBe(0);
    const f = run.report.fixtures[0]!;
    expect(f.status).toBe('pass');
    expect(f.parity?.profile).toBe('engine');
    expect(f.unsupported).toEqual([]);
    expect(run.report).toMatchObject({ engine: 'rust', engineVersion: RUST_VERSION });
  });

  test('superdough reports engine superdough, no engine version, and the self profile', async () => {
    const r = new FakeRenderer();
    const d = await withGoldens(r, corpusOf(fixture()));
    const run = await runCheck(opts(), d);
    expect(run.report).toMatchObject({ engine: 'superdough', engineVersion: null });
    expect(run.report.fixtures[0]!.parity?.profile).toBe('self');
  });

  test('twin is exact on rust even for a stochastic fixture', async () => {
    const corpus = corpusOf(
      fixture({ gates: ['twin'], expectChunks: undefined, stochastic: true }),
    );
    const run = await rustRun(rustFake(), corpus);
    expect(gateOf(run, 'a', 'twin')!.measurements.mode).toBe('exact');
  });

  test('unsupported as corpus.json says: status unsupported, nothing rendered, exit 0', async () => {
    const r = rustFake({ unsupported: [SIG, { s: 'sine', keys: [], firstBar: 0 }] });
    const run = await rustRun(r, corpusOf(fixture({ rust: 'unsupported' })), { mutations: true }, [
      { name: 'm', fixture: 'a', gate: 'bass', apply: (q) => q },
    ]);
    expect(run.exitCode).toBe(0);
    expect(r.renders).toEqual([]);
    expect(run.report.fixtures[0]).toMatchObject({
      status: 'unsupported',
      gates: [],
      parity: null,
      parityNote: null,
      errors: [],
      unsupported: ['s=supersaw keys=shape bar=2', 's=sine keys= bar=0'],
    });
    expect(run.report.mutations).toEqual([
      expect.objectContaining({ name: 'm', status: 'skipped', detail: 'control did not run' }),
    ]);
  });

  test('a supported fixture detected unsupported fails the run (exit 1)', async () => {
    const r = rustFake({ unsupported: [SIG] });
    const run = await rustRun(r, corpusOf(fixture()));
    expect(run.exitCode).toBe(1);
    expect(r.renders).toEqual([]);
    const f = run.report.fixtures[0]!;
    expect(f.status).toBe('unsupported');
    expect(f.unsupported).toEqual(['s=supersaw keys=shape bar=2']);
    expect(f.errors).toHaveLength(1);
    expect(f.errors[0]).toContain('update `rust` in corpus.json');
  });

  test('an unsupported fixture detected supported fails the run (exit 1) without rendering', async () => {
    const r = rustFake();
    const run = await rustRun(r, corpusOf(fixture({ rust: 'unsupported' })));
    expect(run.exitCode).toBe(1);
    expect(r.renders).toEqual([]);
    const f = run.report.fixtures[0]!;
    expect(f.status).toBe('error');
    expect(f.errors[0]).toContain('the engine plays every hap; update `rust` in corpus.json');
  });

  test('superdough ignores the rust field', async () => {
    const corpus = corpusOf(fixture({ rust: 'unsupported' }));
    const d = await withGoldens(new FakeRenderer(), corpus);
    const run = await runCheck(opts(), d);
    expect(run.exitCode).toBe(0);
    expect(run.report.fixtures[0]!.status).toBe('pass');
  });

  test('schedule errors win over unsupported signatures', async () => {
    const r = rustFake({
      unsupported: [SIG],
      scheduleErrors: [{ kind: 'eval', message: 'boom' }],
    });
    const run = await rustRun(r, corpusOf(fixture({ rust: 'unsupported' })));
    expect(run.exitCode).toBe(1);
    expect(run.report.fixtures[0]).toMatchObject({
      status: 'error',
      errors: ['eval: boom'],
      unsupported: [],
    });
  });

  test('an unsupported error at render time is a render error (status error)', async () => {
    const r = rustFake({
      renderErrors: [{ kind: 'unsupported', message: 's=sine keys=vowel', bar: 1 }],
    });
    const run = await rustRun(r, corpusOf(fixture()));
    expect(run.exitCode).toBe(1);
    expect(run.report.fixtures[0]).toMatchObject({
      status: 'error',
      errors: ['unsupported: s=sine keys=vowel (bar 1)'],
      unsupported: [],
    });
  });

  test('mutations run on rust renders and are caught', async () => {
    const spec: MutationSpec = {
      name: 'bass-gone',
      fixture: 'a',
      gate: 'bass',
      apply: (q) => ({ ...q, mutation: { kind: 'dropNotesBelowHz', hz: 130, bars: [3, 4, 5, 6] } }),
    };
    const r = rustFake();
    const run = await rustRun(r, corpusOf(fixture()), { mutations: true }, [spec]);
    expect(r.renders.some((q) => q.mutation?.kind === 'dropNotesBelowHz')).toBe(true);
    expect(run.report.mutations).toEqual([
      expect.objectContaining({ name: 'bass-gone', status: 'caught' }),
    ]);
    expect(run.exitCode).toBe(0);
  });
});

describe('prepareFixture', () => {
  const corpus = corpusOf(fixture());
  const base = { corpus, readSong: (f: string) => `// ${f}` };

  test("returns the plan when nothing is unsupported; the engine is the renderer's", async () => {
    const p = await prepareFixture(fixture(), { ...base, renderer: rustFake() });
    if (!('fx' in p)) throw new Error('expected a plan');
    expect(p.fx.plans.length).toBe(2);
    expect(p.fx.renderer.engine).toBe('rust');
    expect('engine' in p.fx).toBe(false);
    const q = await prepareFixture(fixture(), { ...base, renderer: new FakeRenderer() });
    expect('fx' in q && q.fx.renderer.engine).toBe('superdough');
  });

  test('the plan is the shared renderChunkPlans with the default seam overlap', async () => {
    const events = bassEvents(14);
    const p = await prepareFixture(fixture(), { ...base, renderer: new FakeRenderer({ events }) });
    if (!('fx' in p)) throw new Error('expected a plan');
    const [fromBar, toBar] = fixture().windowBars;
    expect(p.fx.plans).toEqual(renderChunkPlans({ fromBar, toBar, events, cps: p.fx.cps }));
    expect(p.fx.plans[0]!.overlapBars).toBe(SEAM_OVERLAP_BARS);
  });

  test('an empty unsupported list is a plan', async () => {
    const p = await prepareFixture(fixture(), {
      ...base,
      renderer: new FakeRenderer({ unsupported: [] }),
    });
    expect('fx' in p).toBe(true);
  });

  test('returns the signatures when the schedule has any', async () => {
    const p = await prepareFixture(fixture(), {
      ...base,
      renderer: new FakeRenderer({ unsupported: [SIG] }),
    });
    expect(p).toEqual({ unsupported: [SIG] });
  });

  test('schedule errors come first', async () => {
    const p = await prepareFixture(fixture(), {
      ...base,
      renderer: new FakeRenderer({
        unsupported: [SIG],
        scheduleErrors: [{ kind: 'eval', message: 'x' }],
      }),
    });
    expect(p).toEqual({ errors: ['eval: x'] });
  });
});

describe('manifestDisagreement', () => {
  const fx = { fx: {} as never };
  test('null when the field matches the detection', () => {
    expect(manifestDisagreement(fixture(), fx)).toBeNull();
    expect(
      manifestDisagreement(fixture({ rust: 'unsupported' }), { unsupported: [SIG] }),
    ).toBeNull();
  });
  test('a message naming the fix otherwise', () => {
    expect(manifestDisagreement(fixture(), { unsupported: [SIG] })).toContain(
      'marks this fixture rust: supported',
    );
    expect(manifestDisagreement(fixture({ rust: 'unsupported' }), fx)).toContain(
      'marks this fixture rust: unsupported',
    );
  });
});

describe('runCheck --planner batch', () => {
  /** The batch window: the whole window rendered at once by a fake at `gain`. */
  const batchOf = async (gain: number, calls: FixtureContext[]) => {
    const whole = new FakeRenderer({ gain, engine: 'rust' });
    return async (fx: FixtureContext): Promise<AssembledWindow> => {
      calls.push(fx);
      const r = await whole.render(fx.base);
      return { ...r, events: fx.sched.events, seamSecs: [] };
    };
  };

  test('mix, bass and parity take the batch window; twin and seam still render chunks', async () => {
    const corpus = corpusOf(fixture());
    const plain = await rustRun(rustFake(), corpus, { planner: 'host' });
    const calls: FixtureContext[] = [];
    const r = rustFake();
    const d = await withGoldens(new FakeRenderer(), corpus);
    const run = await runCheck(opts({ engine: 'rust', planner: 'batch', keepWav: true }), {
      ...d,
      renderer: r,
      rustVersion: RUST_VERSION,
      batchWindow: await batchOf(0.5, calls),
    });
    expect(calls.map((fx) => [fx.fixture.name, fx.base])).toEqual([['a', BASE]]);
    // The batch audio is half the chunks' level: the mix gate measured it.
    const peak = (x: typeof run) => gateOf(x, 'a', 'mix')!.measurements.peak as number;
    expect(peak(run)).toBeCloseTo(peak(plain) / 2, 3);
    expect(gateOf(run, 'a', 'bass')!.verdict).toBe('pass');
    // Chunk renders (seam, twin) still go through the renderer, the twin twice.
    expect(r.renders.length).toBe(3);
    expect(r.renders.filter((q) => q.fromBar === 0).length).toBe(2);
    // --keep-wav writes the batch audio.
    const kept = d.fs.files.get(join('/out', 'wav', 'a.wav')) as Uint8Array;
    expect(decodeWav(kept).channels[0]!.length).toBe(windowFrames(0, 12, CPS, SR));
    expect(Math.max(...decodeWav(kept).channels[0]!)).toBeCloseTo(peak(plain) / 2, 3);
  });

  test('a mix-only fixture renders nothing through the renderer', async () => {
    const corpus = corpusOf(fixture({ gates: ['mix', 'bass'], expectChunks: undefined }));
    const r = rustFake();
    const d = await withGoldens(new FakeRenderer(), corpus);
    const calls: FixtureContext[] = [];
    const run = await runCheck(opts({ engine: 'rust', planner: 'batch' }), {
      ...d,
      renderer: r,
      rustVersion: RUST_VERSION,
      batchWindow: await batchOf(1, calls),
    });
    expect(calls.length).toBe(1);
    expect(r.renders).toEqual([]);
    expect(run.report.fixtures[0]!.gates.map((g) => g.verdict)).toEqual(['pass', 'pass']);
  });

  test('a failed batch render is the fixture error; the run goes on', async () => {
    const corpus = corpusOf(fixture({ gates: ['mix'], expectChunks: undefined }), {
      ...fixture({ gates: ['mix'], expectChunks: undefined }),
      name: 'b',
      file: 'b.js',
    });
    const d = await withGoldens(new FakeRenderer(), corpus);
    const run = await runCheck(opts({ engine: 'rust', planner: 'batch' }), {
      ...d,
      renderer: rustFake(),
      rustVersion: RUST_VERSION,
      batchWindow: async (fx) => {
        if (fx.fixture.name === 'a') {
          throw new RenderFailedError(["batch: the batch schedule differs from the planner's"]);
        }
        return { ...(await new FakeRenderer().render(fx.base)), seamSecs: [] };
      },
    });
    expect(run.exitCode).toBe(1);
    expect(run.report.fixtures.map((f) => [f.name, f.status, f.errors])).toEqual([
      ['a', 'error', ["batch: the batch schedule differs from the planner's"]],
      ['b', 'pass', []],
    ]);
  });

  test('an unsupported fixture never reaches the batch', async () => {
    const calls: FixtureContext[] = [];
    const corpus = corpusOf(fixture({ rust: 'unsupported' }));
    const d = await withGoldens(new FakeRenderer(), corpus);
    const run = await runCheck(opts({ engine: 'rust', planner: 'batch' }), {
      ...d,
      renderer: rustFake({ unsupported: [SIG] }),
      rustVersion: RUST_VERSION,
      batchWindow: await batchOf(1, calls),
    });
    expect(run.exitCode).toBe(0);
    expect(calls).toEqual([]);
    expect(run.report.fixtures[0]!.status).toBe('unsupported');
  });

  test('mutations re-render the chunks; the batch window is not mutated', async () => {
    const spec: MutationSpec = {
      name: 'bass-gone',
      fixture: 'a',
      gate: 'bass',
      apply: (q) => ({ ...q, mutation: { kind: 'dropNotesBelowHz', hz: 130, bars: [3, 4, 5, 6] } }),
    };
    const calls: FixtureContext[] = [];
    const r = rustFake();
    const d = await withGoldens(new FakeRenderer(), corpusOf(fixture()));
    const run = await runCheck(opts({ engine: 'rust', planner: 'batch', mutations: true }), {
      ...d,
      renderer: r,
      rustVersion: RUST_VERSION,
      batchWindow: await batchOf(1, calls),
      mutations: [spec],
    });
    expect(calls.length).toBe(1);
    expect(r.renders.some((q) => q.mutation?.kind === 'dropNotesBelowHz')).toBe(true);
    expect(run.report.mutations).toEqual([
      expect.objectContaining({ name: 'bass-gone', status: 'caught' }),
    ]);
  });
});

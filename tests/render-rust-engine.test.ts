import { describe, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import {
  assembleResult,
  bunRunner,
  buildEngine,
  cachedJobFile,
  crateVersion,
  ENGINE_TIMEOUT_MS,
  binaryArtifact,
  buildVersion,
  exitCodeToOutcome,
  findEngineBinary,
  parseSummary,
  runEngine,
  safeName,
  signatures,
  SIGNATURES_TIMEOUT_MS,
  writeJobFile,
  type CachedJobText,
  type EngineProcessRunner,
  type EngineSummary,
  type JobFile,
  type ProcessResult,
} from '../scripts/render/rust-engine';
import { HarnessError } from '../scripts/render/transfer';
import { expectedFrames, type RenderError, type RenderRequest } from '../src/lib/render/contract';
import { planEngineJob, type EngineJob } from '../src/lib/render/engine-job';
import { selectHaps, windowPlan } from '../src/lib/render/plan';
import { decodeWav, encodeWav } from '../src/lib/render/wav';
import { buildStructure } from '../src/lib/strudel/structure';
import { pageJob } from '../src/render/pcm';
import { hap, req } from './helpers/fake-offline';

// ---- fakes

interface Call {
  cmd: string;
  args: string[];
  opts?: { stdin?: string; cwd?: string; timeoutMs?: number };
}

const result = (o: Partial<ProcessResult> = {}): ProcessResult => ({
  code: 0,
  signal: null,
  stdout: '',
  stderr: '',
  ...o,
});

/** A runner that answers each call with `answer(call)`; `throws` rejects like a spawn ENOENT. */
function fakeRunner(answer: (c: Call) => ProcessResult | 'enoent' | Promise<ProcessResult>) {
  const calls: Call[] = [];
  const runner: EngineProcessRunner = {
    async run(cmd, args, opts) {
      const c = { cmd, args, opts };
      calls.push(c);
      const a = await answer(c);
      if (a === 'enoent') throw Object.assign(new Error(`spawn ${cmd} ENOENT`), { code: 'ENOENT' });
      return a;
    },
  };
  return { runner, calls };
}

const summaryJson = (o: Partial<EngineSummary> = {}): string =>
  JSON.stringify({ ok: true, frames: 64, channels: 2, errors: [], renderMs: 3, ...o });

/** The binary: writes a WAV of `channels` x `frames` to the `--out` path, prints a summary. */
function fakeEngine(
  o: {
    frames?: number;
    channels?: number;
    code?: number | null;
    signal?: string | null;
    errors?: RenderError[];
    stdout?: string;
    stderr?: string;
    writeWav?: boolean;
    delayMs?: number;
  } = {},
) {
  return fakeRunner(async (c) => {
    if (o.delayMs) await new Promise((r) => setTimeout(r, o.delayMs));
    const out = c.args[c.args.indexOf('--out') + 1]!;
    const frames = o.frames ?? 64;
    const channels = o.channels ?? 2;
    if (o.writeWav !== false) {
      const data = Array.from({ length: channels }, (_, k) =>
        Float32Array.from({ length: frames }, (_, i) => (k + 1) * 0.001 * i),
      );
      writeFileSync(out, encodeWav(data, 44100, 'f32'));
    }
    const errors = o.errors ?? [];
    return result({
      code: o.code === undefined ? (errors.length > 0 ? 1 : 0) : o.code,
      signal: o.signal ?? null,
      stdout: o.stdout ?? summaryJson({ ok: errors.length === 0, frames, channels, errors }) + '\n',
      stderr: o.stderr ?? '',
    });
  });
}

const tmp = () => mkdtempSync(join(tmpdir(), 'strudeland-rust-engine-'));

const tinyJob = (frames = 64): EngineJob => ({
  schema: 2,
  sampleRate: 44100,
  frames,
  seed: 0,
  cps: 0.5,
  channels: { count: 2, multiChannelOrbits: false },
  orbits: [],
  pauses: [{ suspendSec: 0, haps: [] }],
  values: [],
});

/** The page's job info for a chunk request, planned as the page plans it. */
function pageInfo(r: RenderRequest, cps: number, lookbackSec: number) {
  const plan = windowPlan(r, cps, lookbackSec);
  const haps = [
    hap(plan.c0, plan.c0 + 0.5, { s: 'sine', note: 40, orbit: 1 }),
    hap(plan.c0 + 1, plan.c0 + 1.5, { s: 'sine', note: 52, orbit: 2 }),
  ];
  const { selected } = selectHaps(haps);
  const planned = planEngineJob(selected, selected, plan, r, cps, 0);
  const structure = buildStructure({ bpm: 120, meter: '4/4', sections: [{ name: 'a', bars: 8 }] });
  const errors: RenderError[] = [];
  return { ...pageJob(planned, { errors, plan, cps, structure }), plan, planned };
}

const silentWav = (channels: number, frames: number) => ({
  channels: Array.from({ length: channels }, () => new Float32Array(frames)),
  sampleRate: 44100,
  format: 'f32' as const,
});

const okSummary = (frames: number, channels = 2): EngineSummary => ({
  ok: true,
  frames,
  channels,
  errors: [],
  renderMs: 12,
});

// ---- exit codes

describe('exitCodeToOutcome', () => {
  test('0 with a clean summary is ok; 1 with errors is render errors', () => {
    expect(exitCodeToOutcome(result({ stdout: summaryJson() }))).toEqual({ kind: 'ok' });
    const errors: RenderError[] = [{ kind: 'unsupported', message: 's=sine keys=shape', bar: 0 }];
    expect(
      exitCodeToOutcome(result({ code: 1, stdout: summaryJson({ ok: false, errors }) })),
    ).toEqual({ kind: 'render-errors' });
  });

  test('exit 2, other codes and signals crash with stderr', () => {
    const two = exitCodeToOutcome(result({ code: 2, stderr: 'invalid job:\nschema: 2' }));
    expect(two).toEqual({ kind: 'crashed', message: 'engine exited 2: invalid job:\nschema: 2' });
    expect(exitCodeToOutcome(result({ code: 101, stdout: summaryJson() }))).toMatchObject({
      kind: 'crashed',
      message: 'engine exited 101: ',
    });
    const abort = exitCodeToOutcome(result({ code: null, signal: 'SIGABRT', stderr: 'panicked' }));
    expect(abort).toEqual({ kind: 'crashed', message: 'engine killed by SIGABRT: panicked' });
    expect(exitCodeToOutcome(result({ code: null }))).toMatchObject({
      message: 'engine killed by a signal: ',
    });
  });

  test('a stdout that is not the summary crashes', () => {
    for (const stdout of ['', 'hello', '{}', '[]', '{"ok":true}', summaryJson() + 'x']) {
      expect(exitCodeToOutcome(result({ stdout })).kind).toBe('crashed');
    }
    expect(exitCodeToOutcome(result({ stdout: 'hi' }))).toMatchObject({
      message: 'engine exited 0 without a summary: ',
    });
  });

  test('an exit code that contradicts the summary crashes', () => {
    const errors: RenderError[] = [{ kind: 'hap', message: 'x' }];
    expect(exitCodeToOutcome(result({ code: 0, stdout: summaryJson({ errors }) }))).toMatchObject({
      message: 'engine exited 0 with render errors: ',
    });
    expect(exitCodeToOutcome(result({ code: 1, stdout: summaryJson() }))).toMatchObject({
      message: 'engine exited 1 without render errors: ',
    });
  });

  test('stderr is cut to 300 characters', () => {
    const o = exitCodeToOutcome(result({ code: 2, stderr: 'e'.repeat(1000) }));
    expect(o.kind === 'crashed' && o.message).toBe(`engine exited 2: ${'e'.repeat(300)}`);
  });
});

test('parseSummary checks every field', () => {
  expect(parseSummary(summaryJson() + '\n')).toEqual({
    ok: true,
    frames: 64,
    channels: 2,
    errors: [],
    renderMs: 3,
  });
  expect(parseSummary(summaryJson({ errors: [{ kind: 'hap' }] as never }))).toBeNull();
  expect(parseSummary(summaryJson({ frames: '64' as never }))).toBeNull();
  expect(parseSummary('null')).toBeNull();
});

// ---- build

const artifact = (name: string, kind: string[], executable: string | null) =>
  JSON.stringify({ reason: 'compiler-artifact', target: { name, kind }, executable });

const METADATA = JSON.stringify({
  packages: [
    { name: 'strudeland-project', version: '9.9.9' },
    { name: 'strudeland-engine', version: '0.1.0' },
  ],
});

/** cargo build, cargo metadata and git, answered from `o`. */
function buildRunner(
  o: {
    build?: ProcessResult | 'enoent';
    metadata?: ProcessResult | 'enoent';
    head?: ProcessResult | 'enoent';
    status?: ProcessResult | 'enoent';
  } = {},
) {
  return fakeRunner((c) => {
    if (c.cmd === 'cargo' && c.args[0] === 'build') {
      return (
        o.build ??
        result({
          stdout: [
            artifact('strudeland_engine', ['lib'], null),
            '{"reason":"build-script-executed"}',
            artifact('strudeland-engine', ['bin'], '/t/release/strudeland-engine'),
            '{"reason":"build-finished","success":true}',
          ].join('\n'),
        })
      );
    }
    if (c.cmd === 'cargo' && c.args[0] === 'metadata') {
      return o.metadata ?? result({ stdout: METADATA });
    }
    if (c.cmd === 'git' && c.args[0] === 'rev-parse') {
      return o.head ?? result({ stdout: 'abc1234\n' });
    }
    if (c.cmd === 'git' && c.args[0] === 'status') return o.status ?? result({ stdout: '' });
    throw new Error(`unexpected ${c.cmd} ${c.args.join(' ')}`);
  });
}

describe('buildEngine', () => {
  test('runs the release build in cwd and returns the bin artifact and version', async () => {
    const { runner, calls } = buildRunner();
    expect(await buildEngine(runner, '/repo')).toEqual({
      path: '/t/release/strudeland-engine',
      version: 'strudeland-engine 0.1.0 (abc1234)',
    });
    expect(calls[0]).toEqual({
      cmd: 'cargo',
      args: ['build', '--release', '-p', 'strudeland-engine', '--message-format=json'],
      opts: { cwd: '/repo' },
    });
    expect(calls.every((c) => c.opts?.cwd === '/repo')).toBe(true);
  });

  test('a dirty tree marks the version', async () => {
    const { runner } = buildRunner({ status: result({ stdout: ' M src/a.ts\n' }) });
    expect((await buildEngine(runner, '/repo')).version).toBe(
      'strudeland-engine 0.1.0 (abc1234-dirty)',
    );
  });

  test('a missing cargo is a build-missing harness error', async () => {
    const { runner } = buildRunner({ build: 'enoent' });
    const e = await buildEngine(runner, '/repo').catch((x) => x);
    expect(e).toBeInstanceOf(HarnessError);
    expect(e.code).toBe('build-missing');
    expect(e.message).toContain('cannot run cargo');
  });

  test('a failed build is a build-missing harness error with cargo stderr', async () => {
    const { runner } = buildRunner({
      build: result({ code: 101, stderr: 'error[E0425]: cannot find value `x`' }),
    });
    const e = await buildEngine(runner, '/repo').catch((x) => x);
    expect(e).toBeInstanceOf(HarnessError);
    expect(e.code).toBe('build-missing');
    expect(e.message).toContain('exited 101');
    expect(e.message).toContain('error[E0425]');
    const killed = buildRunner({ build: result({ code: null, signal: 'SIGKILL' }) });
    expect((await buildEngine(killed.runner, '/r').catch((x) => x)).message).toContain(
      'killed by SIGKILL',
    );
  });

  test('a build without the bin artifact is a harness error', async () => {
    const { runner } = buildRunner({
      build: result({ stdout: artifact('strudeland_engine', ['lib'], null) }),
    });
    const e = await buildEngine(runner, '/repo').catch((x) => x);
    expect(e).toBeInstanceOf(HarnessError);
    expect(e.message).toContain('no strudeland-engine binary artifact');
  });

  test('the version falls back to unknown parts', async () => {
    const { runner } = buildRunner({
      metadata: result({ code: 1 }),
      head: 'enoent',
      status: 'enoent',
    });
    expect(await buildVersion(runner, '/r', 'strudeland-engine')).toBe(
      'strudeland-engine unknown (unknown)',
    );
    const noPkg = buildRunner({ metadata: result({ stdout: '{"packages":[]}' }) });
    expect(await buildVersion(noPkg.runner, '/r', 'strudeland-engine')).toBe(
      'strudeland-engine unknown (abc1234)',
    );
  });
});

test('binaryArtifact takes the bin artifact (the last one), skipping noise', () => {
  const out = [
    'not json',
    '{broken',
    artifact('strudeland-engine', ['lib'], '/lib'),
    artifact('other', ['bin'], '/other'),
    artifact('strudeland-engine', ['bin'], '/a'),
    artifact('strudeland-engine', ['bin'], '/b'),
  ].join('\n');
  expect(binaryArtifact(out, 'strudeland-engine')).toBe('/b');
  expect(binaryArtifact('', 'strudeland-engine')).toBeNull();
  expect(
    binaryArtifact(artifact('strudeland-engine', ['bin'], null), 'strudeland-engine'),
  ).toBeNull();
});

test('crateVersion', () => {
  expect(crateVersion(METADATA, 'strudeland-engine')).toBe('0.1.0');
  expect(crateVersion('{}', 'strudeland-engine')).toBeNull();
  expect(crateVersion('nope', 'strudeland-engine')).toBeNull();
});

describe('findEngineBinary', () => {
  test('STRUDELAND_ENGINE_BIN overrides the build and is reported as external', async () => {
    const { runner, calls } = buildRunner();
    const exe = resolve('/my/engine');
    expect(await findEngineBinary({ STRUDELAND_ENGINE_BIN: exe }, runner, '/r')).toEqual({
      path: exe,
      version: `strudeland-engine (external: ${exe})`,
    });
    // Neither built nor credited to the repo HEAD.
    expect(calls).toHaveLength(0);
  });

  test('a relative STRUDELAND_ENGINE_BIN resolves against the repo root, not outDir', async () => {
    const { runner } = buildRunner();
    const root = resolve('/repo');
    const env = { STRUDELAND_ENGINE_BIN: 'target/release/strudeland-engine' };
    const exe = join(root, 'target', 'release', 'strudeland-engine');
    expect(await findEngineBinary(env, runner, root)).toEqual({
      path: exe,
      version: `strudeland-engine (external: ${exe})`,
    });
    const up = await findEngineBinary({ STRUDELAND_ENGINE_BIN: '../bin/engine' }, runner, root);
    expect(up.path).toBe(resolve('/bin/engine'));
  });

  test('unset or empty builds', async () => {
    for (const env of [{}, { STRUDELAND_ENGINE_BIN: '' }]) {
      const { runner, calls } = buildRunner();
      expect(await findEngineBinary(env, runner, '/r')).toEqual({
        path: '/t/release/strudeland-engine',
        version: 'strudeland-engine 0.1.0 (abc1234)',
      });
      expect(calls[0]!.args[0]).toBe('build');
      expect(calls.filter((c) => c.args[0] === 'metadata')).toHaveLength(1);
    }
  });
});

// ---- run

describe('runEngine', () => {
  test('writes the job, runs render in outDir, decodes the WAV, deletes the temp files', async () => {
    const dir = tmp();
    try {
      const { runner, calls } = fakeEngine({ frames: 64 });
      let seenJob: unknown = null;
      const wrapped: EngineProcessRunner = {
        run: async (cmd, args, opts) => {
          seenJob = JSON.parse(readFileSync(args[args.indexOf('--job') + 1]!, 'utf8'));
          return runner.run(cmd, args, opts);
        },
      };
      const job = tinyJob();
      const { summary, wav } = await runEngine('/bin/engine', job, dir, 'bass line', wrapped);
      expect(seenJob).toEqual(job);
      expect(summary).toEqual({ ok: true, frames: 64, channels: 2, errors: [], renderMs: 3 });
      expect(wav.channels).toHaveLength(2);
      expect(wav.channels[1]![10]).toBeCloseTo(0.02, 6);
      const c = calls[0]!;
      expect(c.cmd).toBe('/bin/engine');
      expect(c.args[0]).toBe('render');
      expect(c.args[2]).toMatch(/[/\\]engine[/\\]bass_line-\d+\.job\.json$/);
      expect(c.args[4]).toMatch(/[/\\]engine[/\\]bass_line-\d+\.wav$/);
      expect(c.opts).toEqual({ cwd: dir, timeoutMs: ENGINE_TIMEOUT_MS });
      expect(readdirSync(join(dir, 'engine'))).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('a job file renders more than once and is neither rewritten nor deleted', async () => {
    const dir = tmp();
    try {
      const job = tinyJob();
      const file = writeJobFile(dir, 'cached key', JSON.stringify(job), job.sampleRate);
      expect(file.path).toMatch(/[/\\]engine[/\\]cached_key-\d+\.job\.json$/);
      const { runner, calls } = fakeEngine();
      for (let i = 0; i < 2; i++) {
        const { summary, wav } = await runEngine('/e', file, dir, 'r', runner);
        expect(summary.ok).toBe(true);
        expect(wav.channels).toHaveLength(2);
      }
      expect(calls.map((c) => c.args[2])).toEqual([file.path, file.path]);
      expect(new Set(calls.map((c) => c.args[4])).size).toBe(2);
      expect(JSON.parse(readFileSync(file.path, 'utf8'))).toEqual(job);
      expect(readdirSync(join(dir, 'engine'))).toEqual([basename(file.path)]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('an unmutated cached job is written from the page text verbatim, once', () => {
    const dir = tmp();
    try {
      const job = tinyJob();
      // The page's JSON (key order, number spelling) is what the engine reads, not a
      // re-serialization of the parsed job.
      const text = JSON.stringify(job, null, 1).replace('"cps": 0.5', '"cps": 5e-1');
      expect(text).not.toBe(JSON.stringify(job));
      const entry: CachedJobText = { job, text };
      const written: JobFile[] = [];
      const file = cachedJobFile(entry, dir, 'k', 44100, (f) => written.push(f));
      expect(readFileSync(file.path, 'utf8')).toBe(text);
      expect(file.sampleRate).toBe(44100);
      expect(cachedJobFile(entry, dir, 'k', 44100, (f) => written.push(f))).toBe(file);
      expect(written).toEqual([file]);
      expect(readdirSync(join(dir, 'engine'))).toEqual([basename(file.path)]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('keep leaves the job and the WAV', async () => {
    const dir = tmp();
    try {
      await runEngine('/e', tinyJob(), dir, 'k', fakeEngine().runner, true);
      const files = readdirSync(join(dir, 'engine')).sort();
      expect(files).toHaveLength(2);
      expect(files[0]).toMatch(/^k-\d+\.job\.json$/);
      expect(files[1]).toMatch(/^k-\d+\.wav$/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('concurrent runs with one name use distinct temp files', async () => {
    const dir = tmp();
    try {
      const { runner, calls } = fakeEngine({ delayMs: 5 });
      const runs = await Promise.all(
        Array.from({ length: 4 }, () => runEngine('/e', tinyJob(), dir, 'same', runner, true)),
      );
      expect(runs.every((r) => r.summary.ok)).toBe(true);
      const jobs = calls.map((c) => c.args[2]);
      const wavs = calls.map((c) => c.args[4]);
      expect(new Set(jobs).size).toBe(4);
      expect(new Set(wavs).size).toBe(4);
      expect(readdirSync(join(dir, 'engine'))).toHaveLength(8);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('exit 1 keeps the summary errors and the audio', async () => {
    const dir = tmp();
    try {
      const errors: RenderError[] = [{ kind: 'unsupported', message: 's=sine keys=vowel', bar: 2 }];
      const { summary, wav } = await runEngine(
        '/e',
        tinyJob(),
        dir,
        'x',
        fakeEngine({ errors }).runner,
      );
      expect(summary.errors).toEqual(errors);
      expect(summary.ok).toBe(false);
      expect(wav.channels).toHaveLength(2);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('a crash is a failed summary with an other error, no audio, files removed', async () => {
    const dir = tmp();
    try {
      for (const [o, message] of [
        [{ code: 2, stderr: 'invalid job', writeWav: false }, 'engine exited 2: invalid job'],
        [
          { code: null, signal: 'SIGABRT', stderr: 'panicked at x' },
          'engine killed by SIGABRT: panicked at x',
        ],
        [{ stdout: 'garbage' }, 'engine exited 0 without a summary: '],
      ] as const) {
        const { summary, wav } = await runEngine('/e', tinyJob(), dir, 'c', fakeEngine(o).runner);
        expect(summary).toEqual({
          ok: false,
          frames: 0,
          channels: 0,
          errors: [{ kind: 'other', message }],
          renderMs: 0,
        });
        expect(wav.channels).toEqual([]);
      }
      expect(readdirSync(join(dir, 'engine'))).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('a missing or unreadable WAV is an other error', async () => {
    const dir = tmp();
    try {
      const missing = await runEngine(
        '/e',
        tinyJob(),
        dir,
        'w',
        fakeEngine({ writeWav: false }).runner,
      );
      expect(missing.summary.errors[0]!.kind).toBe('other');
      expect(missing.summary.errors[0]!.message).toMatch(/^engine wav: /);
      const garbage: EngineProcessRunner = {
        run: async (_cmd, args) => {
          writeFileSync(args[4]!, 'not a wav');
          return result({ stdout: summaryJson() });
        },
      };
      const bad = await runEngine('/e', tinyJob(), dir, 'w', garbage);
      expect(bad.summary.errors[0]!.message).toBe('engine wav: not a RIFF/WAVE file');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('a binary that cannot be spawned is a build-missing harness error', async () => {
    const dir = tmp();
    try {
      const { runner } = fakeRunner(() => 'enoent');
      const e = await runEngine('/nope', tinyJob(), dir, 'n', runner).catch((x) => x);
      expect(e).toBeInstanceOf(HarnessError);
      expect(e.code).toBe('build-missing');
      expect(e.message).toContain('cannot run /nope');
      expect(readdirSync(join(dir, 'engine'))).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe('signatures', () => {
  const JOB = '{"schema":2}';
  const ok = (stdout: string) => fakeRunner(() => result({ stdout }));
  const failure = async (runner: EngineProcessRunner): Promise<HarnessError> => {
    const e = await signatures('/e', JOB, runner).catch((x) => x);
    expect(e).toBeInstanceOf(HarnessError);
    return e as HarnessError;
  };

  test('returns the parsed list; the job text goes to stdin, no cwd', async () => {
    const line = '[{"s":"z_noise","keys":[],"firstBar":2},{"s":"sine","keys":["ir"],"firstBar":0}]';
    const { runner, calls } = ok(line + '\n');
    expect(await signatures('/bin/e', JOB, runner)).toEqual([
      { s: 'z_noise', keys: [], firstBar: 2 },
      { s: 'sine', keys: ['ir'], firstBar: 0 },
    ]);
    expect(calls).toEqual([
      {
        cmd: '/bin/e',
        args: ['signatures', '--job', '-'],
        opts: { stdin: JOB, timeoutMs: SIGNATURES_TIMEOUT_MS },
      },
    ]);
    expect(SIGNATURES_TIMEOUT_MS).toBe(60_000);
  });

  test('an empty list, and extra fields are allowed', async () => {
    expect(await signatures('/e', JOB, ok('[]\n').runner)).toEqual([]);
    expect(
      await signatures('/e', JOB, ok('[{"s":"a","keys":["b"],"firstBar":-1,"x":1}]').runner),
    ).toEqual([{ s: 'a', keys: ['b'], firstBar: -1, x: 1 } as any]);
  });

  test('a non-zero exit is render-failed with stderr', async () => {
    const { runner } = fakeRunner(() =>
      result({ code: 2, stderr: 'strudeland-engine: invalid job:\nschema 1 job: regenerate it\n' }),
    );
    const e = await failure(runner);
    expect(e.code).toBe('render-failed');
    expect(e.message).toBe(
      'engine signatures exited 2: strudeland-engine: invalid job:\nschema 1 job: regenerate it',
    );
    const one = await failure(fakeRunner(() => result({ code: 1, stdout: '[]' })).runner);
    expect(one.message).toBe('engine signatures exited 1: ');
  });

  test('a signal is render-failed naming it', async () => {
    const e = await failure(
      fakeRunner(() => result({ code: null, signal: 'SIGKILL', stderr: 'x' })).runner,
    );
    expect(e.code).toBe('render-failed');
    expect(e.message).toBe('engine signatures killed by SIGKILL: x');
  });

  test('stderr is cut to 300 characters', async () => {
    const e = await failure(
      fakeRunner(() => result({ code: 2, stderr: '  ' + 'y'.repeat(500) + '\n' })).runner,
    );
    expect(e.message).toBe('engine signatures exited 2: ' + 'y'.repeat(300));
  });

  test('stdout that is not a signature list is render-failed', async () => {
    for (const stdout of [
      'not json',
      '',
      '{}',
      'null',
      '[1]',
      '[{"s":1,"keys":[],"firstBar":0}]',
      '[{"s":"a","keys":[1],"firstBar":0}]',
      '[{"s":"a","keys":"b","firstBar":0}]',
      '[{"s":"a","keys":[],"firstBar":0.5}]',
      '[{"s":"a","keys":[]}]',
    ]) {
      const e = await failure(ok(stdout).runner);
      expect(e.code).toBe('render-failed');
      expect(e.message).toBe(`engine signatures printed an invalid list: ${stdout.trim()}`);
    }
    const long = await failure(ok('x'.repeat(400)).runner);
    expect(long.message).toBe('engine signatures printed an invalid list: ' + 'x'.repeat(300));
  });

  test('a binary that cannot be spawned is a build-missing harness error', async () => {
    const e = await failure(fakeRunner(() => 'enoent').runner);
    expect(e.code).toBe('build-missing');
    expect(e.message).toBe('cannot run /e: spawn /e ENOENT');
  });
});

test('safeName', () => {
  expect(safeName('bass-line_1.2')).toBe('bass-line_1.2');
  expect(safeName('a b/c:dé')).toBe('a_b_c_d_');
});

// ---- assembly

describe('assembleResult', () => {
  const timing = { evalMs: 5, renderMs: 12, totalMs: 40 };

  test('frames, window start, lookback and buffer start equal the superdough plan', () => {
    for (const [r, cps, lookback] of [
      [req({ fromBar: 0, toBar: 2 }), 0.5, 0],
      [req({ fromBar: 3, toBar: 6, overlapBars: 1, lookbackSec: 'auto' }), 0.5, 4.3],
      [req({ fromBar: 1, toBar: 3, lookbackSec: 9 }), 0.4, 9],
    ] as const) {
      const { info, plan } = pageInfo(r, cps, lookback);
      const res = assembleResult({
        req: r,
        info,
        summary: okSummary(plan.frames),
        wav: silentWav(2, plan.frames),
        timing,
      });
      // what the page's render path reports for the same request (main.ts renderResult)
      expect({
        frames: res.frames,
        windowStartFrame: res.windowStartFrame,
        lookbackSecUsed: res.lookbackSecUsed,
        bufferStartSec: res.bufferStartSec,
      }).toEqual({
        frames: plan.frames,
        windowStartFrame: plan.windowStartFrame,
        lookbackSecUsed: plan.lookbackSecUsed,
        bufferStartSec: plan.bufferStartSec,
      });
      expect(res.frames).toBe(expectedFrames(r, cps, plan.lookbackSecUsed));
      expect(res.ok).toBe(true);
      expect(res.errors).toEqual([]);
      expect(res.cps).toBe(cps);
      expect(res.events).toBe(info.events);
      expect(res.stemPairs).toBe(1);
      expect(res.request).toBe(r);
      expect(res.timing).toEqual(timing);
      expect(res.sampleRate).toBe(44100);
    }
  });

  test('stems: 2 * stemPairs channels', () => {
    const r = req({ toBar: 2, mode: 'stems' });
    const { info, plan } = pageInfo(r, 0.5, 0);
    expect(info.stemPairs).toBe(2);
    const wav = silentWav(4, plan.frames);
    const res = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames, 4),
      wav,
      timing,
    });
    expect(res.ok).toBe(true);
    expect(res.channels).toBe(wav.channels);
    const wrong = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames, 2),
      wav: silentWav(2, plan.frames),
      timing,
    });
    expect(wrong.ok).toBe(false);
    expect(wrong.errors).toEqual([
      { kind: 'other', message: 'engine wrote 2 channels, expected 4' },
    ]);
  });

  test('a frame mismatch (WAV or summary) is an other error', () => {
    const r = req({ toBar: 2 });
    const { info, plan } = pageInfo(r, 0.5, 0);
    const short = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames),
      wav: silentWav(2, plan.frames - 1),
      timing,
    });
    expect(short.ok).toBe(false);
    expect(short.errors[0]!.message).toBe(
      `engine frames ${plan.frames - 1} (summary ${plan.frames}), expected ${plan.frames}`,
    );
    const summary = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames + 1),
      wav: silentWav(2, plan.frames),
      timing,
    });
    expect(summary.errors[0]!.kind).toBe('other');
    const rate = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames),
      wav: { ...silentWav(2, plan.frames), sampleRate: 48000 },
      timing,
    });
    expect(rate.errors).toEqual([
      { kind: 'other', message: 'engine sample rate 48000, expected 44100' },
    ]);
  });

  test('errors: page and engine, deduplicated in order; unsupported kept', () => {
    const r = req({ toBar: 2 });
    const { info, plan } = pageInfo(r, 0.5, 0);
    const page: RenderError = { kind: 'eval', message: 'warn' };
    const unsupported: RenderError = { kind: 'unsupported', message: 's=sine keys=shape', bar: 1 };
    const res = assembleResult({
      req: r,
      info: { ...info, errors: [page] },
      summary: {
        ...okSummary(plan.frames),
        ok: false,
        errors: [unsupported, page, unsupported],
      },
      wav: silentWav(2, plan.frames),
      timing,
    });
    expect(res.errors).toEqual([page, unsupported]);
    expect(res.ok).toBe(false);
  });

  test('a failed run without audio gets silent channels of the planned shape', () => {
    const r = req({ toBar: 2 });
    const { info, plan } = pageInfo(r, 0.5, 0);
    const crashed: RenderError = { kind: 'other', message: 'engine killed by SIGABRT: x' };
    const res = assembleResult({
      req: r,
      info,
      summary: { ok: false, frames: 0, channels: 0, errors: [crashed], renderMs: 0 },
      wav: { channels: [], sampleRate: 44100, format: 'f32' },
      timing,
    });
    expect(res.errors).toEqual([crashed]);
    expect(res.ok).toBe(false);
    expect(res.channels).toHaveLength(2);
    expect(res.channels.every((c) => c.length === plan.frames && c.every((x) => x === 0))).toBe(
      true,
    );
    const silent = assembleResult({
      req: r,
      info,
      summary: okSummary(plan.frames),
      wav: { channels: [], sampleRate: 44100, format: 'f32' },
      timing,
    });
    expect(silent.errors).toEqual([{ kind: 'other', message: 'engine produced no audio' }]);
  });
});

// ---- the binary's WAV

test('decodes a 6-channel WAV the engine binary wrote (WAVE_FORMAT_EXTENSIBLE)', () => {
  const bytes = new Uint8Array(
    readFileSync(new URL('./helpers/engine-sample.wav', import.meta.url)),
  );
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  expect(view.getUint16(20, true)).toBe(0xfffe);
  const wav = decodeWav(bytes);
  expect(wav.sampleRate).toBe(44100);
  expect(wav.format).toBe('f32');
  expect(wav.channels).toHaveLength(6);
  expect(wav.channels.every((c) => c.length === 1024)).toBe(true);
  const peak = Math.max(...wav.channels[0]!.map(Math.abs));
  expect(peak).toBeGreaterThan(0.01);
  expect(peak).toBeLessThanOrEqual(1);
  expect(wav.channels[1]).toEqual(wav.channels[0]!);
  for (const c of wav.channels.slice(2)) expect(c.every((x) => x === 0)).toBe(true);
});

// ---- the real runner

describe('bunRunner', () => {
  test('exit code, stdout, stderr, stdin and cwd', async () => {
    const dir = tmp();
    try {
      const r = await bunRunner.run(
        process.execPath,
        [
          '-e',
          'process.stdout.write(await Bun.stdin.text()); console.log(process.cwd()); console.error("err"); process.exit(3)',
        ],
        { stdin: 'in\n', cwd: dir },
      );
      expect(r.code).toBe(3);
      expect(r.signal).toBeNull();
      const [first, cwd] = r.stdout.trim().split('\n');
      expect(first).toBe('in');
      expect(existsSync(cwd!)).toBe(true);
      expect(r.stderr).toBe('err\n');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test.skipIf(process.platform === 'win32')('a signal leaves code null', async () => {
    const r = await bunRunner.run('sh', ['-c', 'kill -ABRT $$']);
    expect(r.code).toBeNull();
    expect(r.signal).toBe('SIGABRT');
  });

  test('a timeout kills the process', async () => {
    const r = await bunRunner.run(process.execPath, ['-e', 'await Bun.sleep(5000)'], {
      timeoutMs: 50,
    });
    if (process.platform === 'win32') {
      expect(r.code).not.toBe(0);
    } else {
      expect(r.code).toBeNull();
      expect(r.signal).toBe('SIGKILL');
    }
  });

  test('a missing executable rejects', async () => {
    await expect(bunRunner.run('/no/such/strudeland-engine', [])).rejects.toThrow();
  });
});

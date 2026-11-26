import { describe, expect, test } from 'bun:test';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import {
  buildHost,
  findHostBinary,
  HOST_TIMEOUT_MS,
  hostPlanRenderArgs,
  HostPlanner,
  parsePlanRenderOutput,
  SongFiles,
} from '../scripts/render/host-planner';
import type { EngineProcessRunner, ProcessResult } from '../scripts/render/rust-engine';
import { HarnessError } from '../scripts/render/transfer';
import type { RenderRequest } from '../src/lib/render/contract';
import { singleChunkRequest } from '../src/lib/render/job-plan';
import { sha256 } from '../scripts/sha256';

interface Call {
  cmd: string;
  args: string[];
  opts?: { stdin?: string; cwd?: string; timeoutMs?: number };
}

function fakeRunner(answer: (c: Call) => ProcessResult | 'enoent') {
  const calls: Call[] = [];
  const runner: EngineProcessRunner = {
    async run(cmd, args, opts) {
      const c = { cmd, args, opts };
      calls.push(c);
      const a = answer(c);
      if (a === 'enoent') throw Object.assign(new Error(`spawn ${cmd} ENOENT`), { code: 'ENOENT' });
      return a;
    },
  };
  return { runner, calls };
}

const res = (o: Partial<ProcessResult>): ProcessResult => ({
  code: 0,
  signal: null,
  stdout: '',
  stderr: '',
  ...o,
});

const JOB_TEXT = '{"schema":2,"haps":[]}';
const CHUNK_PLAN = { fromBar: 2, toBar: 6, c0: 1, c1: 7, frames: 100 };
const ONE_CHUNK = {
  json: JOB_TEXT,
  plan: CHUNK_PLAN,
  info: { events: [{ note: 'c3', tSec: 0 }], stemPairs: 2 },
};
/** A single-chunk `plan-render` output; `o` overrides the plan's keys. */
const jobPlan = (o: Record<string, unknown> = {}): string =>
  JSON.stringify({
    window: { fromBar: 2, toBar: 6, layout: 'linear' },
    musicFrames: 100,
    cps: 0.5,
    structure: { bpm: 120 },
    lookbackSec: 0,
    errors: [{ kind: 'other', message: 'careful' }],
    chunks: [ONE_CHUNK],
    ...o,
  });

const REQ: RenderRequest = {
  code: 'note("c3").s("sine")',
  layout: 'linear',
  fromBar: 2,
  toBar: 6,
  overlapBars: 1,
  lookbackSec: 'auto',
  mode: 'stems',
  sampleRate: 44100,
};

const failureOf = (f: () => unknown): HarnessError => {
  try {
    f();
  } catch (e) {
    if (e instanceof HarnessError) return e;
    throw e;
  }
  throw new Error('expected a HarnessError');
};

const PLAN_TEXT = JSON.stringify({
  window: { fromBar: 0, toBar: 4, layout: 'linear' },
  cps: 0.5,
  errors: [],
  chunks: [{ json: JOB_TEXT, plan: { fromBar: 0, toBar: 4, c0: 0, c1: 4 } }],
});

describe('parsePlanRenderOutput', () => {
  test('plan-render --song <file> --request <json>', () => {
    expect(hostPlanRenderArgs('/s.js', '{"a":1}')).toEqual([
      'plan-render',
      '--song',
      '/s.js',
      '--request',
      '{"a":1}',
    ]);
  });

  test('exit 0: the plan, with each chunk job text exact', () => {
    const planned = parsePlanRenderOutput(res({ stdout: PLAN_TEXT }));
    expect(planned.chunks).toHaveLength(1);
    expect(planned.chunks[0]!.json).toBe(JOB_TEXT);
    expect(planned.chunks[0]!.plan.toBar).toBe(4);
    expect(parsePlanRenderOutput(res({ stdout: '{"chunks":[]}' })).chunks).toEqual([]);
  });

  test('exit 1 and 2 and a signal: render-failed with the stderr', () => {
    for (const r of [
      res({ code: 1, stderr: 'strudeland-host: meta: pass --length N' }),
      res({ code: 2, stderr: 'strudeland-host: invalid request: x' }),
      res({ code: null, signal: 'SIGTERM', stderr: 'killed' }),
    ]) {
      const e = failureOf(() => parsePlanRenderOutput(r));
      expect(e.code).toBe('render-failed');
      expect(e.message).toContain(r.stderr);
    }
  });

  test('exit 0 with a stdout that is not a plan', () => {
    const cases: [string, string][] = [
      ['', 'not JSON'],
      ['{"chunks":', 'not JSON'],
      ['[]', 'without chunks'],
      ['{"window":{}}', 'without chunks'],
      ['{"chunks":{}}', 'without chunks'],
      ['{"chunks":[3]}', 'malformed chunk 0'],
      ['{"chunks":[{"json":1,"plan":{"fromBar":0,"toBar":4}}]}', 'malformed chunk 0'],
      ['{"chunks":[{"json":"{}"}]}', 'malformed chunk 0'],
      [
        '{"chunks":[{"json":"{}","plan":{"fromBar":0,"toBar":4}},{"json":"{}","plan":{"fromBar":"4","toBar":8}}]}',
        'malformed chunk 1',
      ],
    ];
    for (const [stdout, why] of cases) {
      const e = failureOf(() => parsePlanRenderOutput(res({ stdout, stderr: 'err' })));
      expect(e.code).toBe('render-failed');
      expect(e.message).toContain(why);
    }
  });
});

const withDir = (f: (dir: string) => Promise<void>) => async () => {
  const dir = mkdtempSync(join(tmpdir(), 'host-planner-'));
  try {
    await f(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
};

describe('HostPlanner.job', () => {
  /** `hp.job(req)` over a runner that answers `answer`: the result or its HarnessError, and the
   *  calls. */
  async function jobOf(answer: ProcessResult, req: RenderRequest = REQ) {
    const dir = mkdtempSync(join(tmpdir(), 'host-planner-'));
    try {
      const { runner, calls } = fakeRunner(() => answer);
      const hp = new HostPlanner({ bin: '/bin/host', version: 'v', outDir: dir, runner });
      const out = await hp.job(req).catch((e) => e);
      return { out, calls, song: join(dir, 'host', `${sha256(req.code)}.js`) };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }

  test('plan-render --song <file> --request <the single-chunk request>', async () => {
    const req: RenderRequest = {
      ...REQ,
      samples: { bd: 'x' },
      mutation: { kind: 'drop', fraction: 0.5 } as unknown as RenderRequest['mutation'],
    };
    const { calls, song } = await jobOf(res({ stdout: jobPlan() }), req);
    expect(calls[0]!.args).toEqual([
      'plan-render',
      '--song',
      song,
      '--request',
      '{"length":{"kind":"window","fromBar":2,"toBar":6,"layout":"linear"},' +
        '"chunk":{"overlapBars":1,"lookbackSec":"auto","mode":"stems"}}',
    ]);
    expect(calls[0]!.opts?.timeoutMs).toBe(HOST_TIMEOUT_MS);
    for (const lookbackSec of [1.25, 0]) {
      const { calls } = await jobOf(res({ stdout: jobPlan() }), { ...REQ, lookbackSec });
      expect(JSON.parse(calls[0]!.args[4]!).chunk.lookbackSec).toBe(lookbackSec);
    }
  });

  test("exit 0: the one chunk's exact text, the lazily parsed job and pageJobInfo", async () => {
    const { out } = await jobOf(res({ stdout: jobPlan(), stderr: 'console.warn: x\n' }));
    expect(out.text).toBe(JOB_TEXT);
    expect(out.job).toEqual({ schema: 2, haps: [] });
    expect(out.job).toBe(out.job); // parsed once
    expect(out.info).toEqual({
      jsonLength: JOB_TEXT.length,
      events: ONE_CHUNK.info.events,
      errors: [{ kind: 'other', message: 'careful' }],
      plan: CHUNK_PLAN,
      cps: 0.5,
      structure: { bpm: 120 },
      stemPairs: 2,
    });
  });

  test('exit 1 and 2 and a signal: render-failed with the stderr', async () => {
    const cases: [ProcessResult, string][] = [
      [
        res({ code: 1, stderr: 'strudeland-host: eval: boom\n' }),
        'strudeland-host exited 1: strudeland-host: eval: boom',
      ],
      [
        res({ code: 2, stdout: jobPlan(), stderr: 'strudeland-host: bad\nusage' }),
        'strudeland-host exited 2: strudeland-host: bad\nusage',
      ],
      [res({ code: null, signal: 'SIGKILL' }), 'strudeland-host killed by SIGKILL: '],
      [res({ code: 1, stderr: 'x'.repeat(5000) }), `strudeland-host exited 1: ${'x'.repeat(2000)}`],
    ];
    for (const [answer, message] of cases) {
      const { out } = await jobOf(answer);
      expect(out).toBeInstanceOf(HarnessError);
      expect(out.code).toBe('render-failed');
      expect(out.message).toBe(message);
    }
  });

  test('a plan without exactly one chunk with info is render-failed', async () => {
    const { info: _, ...bare } = ONE_CHUNK;
    const cases: [string, string][] = [
      [jobPlan({ chunks: [ONE_CHUNK, ONE_CHUNK] }), 'not a single-chunk plan (2 chunk(s))'],
      [jobPlan({ chunks: [] }), 'not a single-chunk plan (0 chunk(s))'],
      [jobPlan({ chunks: [bare] }), 'not a single-chunk plan (1 chunk(s))'],
    ];
    for (const [stdout, why] of cases) {
      const { out } = await jobOf(res({ stdout, stderr: 'err' }));
      expect(out).toBeInstanceOf(HarnessError);
      expect(out.code).toBe('render-failed');
      expect(out.message).toBe(`strudeland-host exited 0: ${why}: err`);
    }
    const { out } = await jobOf(res({ stdout: '{"chunks":', stderr: 'err' }));
    expect(out.message).toStartWith('strudeland-host exited 0 with a plan that is not JSON (');
  });

  test('exit 0 with a job that is not a JSON object', async () => {
    // Not JSON: found when the job is first read, not when the output is parsed.
    const { out } = await jobOf(
      res({ stdout: jobPlan({ chunks: [{ ...ONE_CHUNK, json: '{' }] }) }),
    );
    expect(out.text).toBe('{');
    const e1 = failureOf(() => out.job);
    expect(e1.message).toStartWith('strudeland-host exited 0 with a job that is not JSON (');
    const array = await jobOf(res({ stdout: jobPlan({ chunks: [{ ...ONE_CHUNK, json: '[]' }] }) }));
    expect(array.out).toBeInstanceOf(HarnessError);
    expect(array.out.message).toStartWith(
      'strudeland-host exited 0 with a job that is not an object',
    );
  });
});

describe('HostPlanner', () => {
  test(
    'writes the song by hash and runs a single-chunk plan-render with the timeout',
    withDir(async (dir) => {
      const { runner, calls } = fakeRunner(() => res({ stdout: jobPlan() }));
      const hp = new HostPlanner({ bin: '/bin/host', version: 'v', outDir: dir, runner });
      const a = await hp.job(REQ);
      const b = await hp.job({ ...REQ, fromBar: 0 });
      expect(a.text).toBe(JOB_TEXT);
      expect(b.text).toBe(JOB_TEXT);
      const song = join(dir, 'host', `${sha256(REQ.code)}.js`);
      expect(readFileSync(song, 'utf8')).toBe(REQ.code);
      expect(readdirSync(join(dir, 'host'))).toEqual([`${sha256(REQ.code)}.js`]);
      expect(calls.map((c) => c.cmd)).toEqual(['/bin/host', '/bin/host']);
      const args = (req: RenderRequest) =>
        hostPlanRenderArgs(song, JSON.stringify(singleChunkRequest(req)));
      expect(calls[0]!.args).toEqual(args(REQ));
      expect(calls[1]!.args).toEqual(args({ ...REQ, fromBar: 0 }));
      expect(calls[0]!.opts?.timeoutMs).toBe(HOST_TIMEOUT_MS);
      expect(HOST_TIMEOUT_MS).toBe(600_000);
    }),
  );

  test(
    'planRender runs plan-render with the timeout; songFile writes the song by hash',
    withDir(async (dir) => {
      const { runner, calls } = fakeRunner(() => res({ stdout: PLAN_TEXT }));
      const hp = new HostPlanner({ bin: '/bin/host', version: 'v', outDir: dir, runner });
      const song = hp.songFile(REQ.code);
      expect(song).toBe(join(dir, 'host', `${sha256(REQ.code)}.js`));
      expect(readFileSync(song, 'utf8')).toBe(REQ.code);
      const planned = await hp.planRender(song, '{"r":1}');
      expect(planned.chunks[0]!.json).toBe(JOB_TEXT);
      expect(calls).toHaveLength(1);
      expect(calls[0]!.cmd).toBe('/bin/host');
      expect(calls[0]!.args).toEqual(hostPlanRenderArgs(song, '{"r":1}'));
      expect(calls[0]!.opts?.timeoutMs).toBe(HOST_TIMEOUT_MS);
      const failing = new HostPlanner({
        bin: '/bin/host',
        version: 'v',
        outDir: dir,
        runner: fakeRunner(() => res({ code: 1, stderr: 'strudeland-host: eval: x' })).runner,
      });
      const e = await failing.planRender(song, '{}').catch((x) => x);
      expect(e).toBeInstanceOf(HarnessError);
      expect(e.code).toBe('render-failed');
      const missing = new HostPlanner({
        bin: '/no/host',
        version: 'v',
        outDir: dir,
        runner: fakeRunner(() => 'enoent').runner,
      });
      expect((await missing.planRender(song, '{}').catch((x) => x)).code).toBe('build-missing');
    }),
  );

  test(
    'a song file left truncated by an earlier run is rewritten, with no temporary file left',
    withDir(async (dir) => {
      const song = join(dir, 'host', `${sha256(REQ.code)}.js`);
      mkdirSync(join(dir, 'host'), { recursive: true });
      writeFileSync(song, REQ.code.slice(0, 5));
      const { runner } = fakeRunner(() => {
        // The host reads the whole song by the time it runs.
        expect(readFileSync(song, 'utf8')).toBe(REQ.code);
        return res({ stdout: jobPlan() });
      });
      const hp = new HostPlanner({ bin: '/bin/host', version: 'v', outDir: dir, runner });
      await hp.job(REQ);
      expect(readFileSync(song, 'utf8')).toBe(REQ.code);
      expect(readdirSync(join(dir, 'host'))).toEqual([`${sha256(REQ.code)}.js`]);
    }),
  );

  test(
    'a host error rejects with render-failed',
    withDir(async (dir) => {
      const { runner } = fakeRunner(() => res({ code: 1, stderr: 'strudeland-host: eval: x' }));
      const hp = new HostPlanner({ bin: '/bin/host', version: 'v', outDir: dir, runner });
      const e = await hp.job(REQ).catch((x) => x);
      expect(e).toBeInstanceOf(HarnessError);
      expect(e.code).toBe('render-failed');
    }),
  );

  test(
    'a binary that cannot be spawned is build-missing',
    withDir(async (dir) => {
      const { runner } = fakeRunner(() => 'enoent');
      const hp = new HostPlanner({ bin: '/no/host', version: 'v', outDir: dir, runner });
      const e = await hp.job(REQ).catch((x) => x);
      expect(e).toBeInstanceOf(HarnessError);
      expect(e.code).toBe('build-missing');
      expect(e.message).toStartWith('cannot run /no/host: ');
    }),
  );

  test(
    'version is the given string; dispose leaves the song files',
    withDir(async (dir) => {
      const { runner } = fakeRunner(() => res({ stdout: jobPlan() }));
      const hp = new HostPlanner({
        bin: '/b',
        version: 'strudeland-host 0.1.0 (abc)',
        outDir: dir,
        runner,
      });
      expect(await hp.version()).toBe('strudeland-host 0.1.0 (abc)');
      await hp.job(REQ);
      await hp.dispose();
      expect(existsSync(join(dir, 'host', `${sha256(REQ.code)}.js`))).toBe(true);
    }),
  );
});

describe('findHostBinary and buildHost', () => {
  test('STRUDELAND_HOST_BIN: no build, resolved against cwd, the external version', async () => {
    const { runner, calls } = fakeRunner(() => res({}));
    const rel = await findHostBinary({ STRUDELAND_HOST_BIN: 'target/h' }, runner, '/repo');
    expect(rel).toEqual({
      path: resolve('/repo', 'target/h'),
      version: `strudeland-host (external: ${resolve('/repo', 'target/h')})`,
    });
    const exe = resolve('/x/h');
    const abs = await findHostBinary({ STRUDELAND_HOST_BIN: exe }, runner, '/repo');
    expect(abs).toEqual({ path: exe, version: `strudeland-host (external: ${exe})` });
    expect(calls).toEqual([]);
  });

  const artifact = (name: string, exe: string) =>
    JSON.stringify({
      reason: 'compiler-artifact',
      target: { name, kind: ['bin'] },
      executable: exe,
    });
  const METADATA = JSON.stringify({
    packages: [
      { name: 'strudeland-engine', version: '0.1.0' },
      { name: 'strudeland-host', version: '0.2.0' },
    ],
  });
  const cargoAndGit = (build: ProcessResult) =>
    fakeRunner((c) => {
      if (c.cmd === 'cargo' && c.args[0] === 'build') return build;
      if (c.cmd === 'cargo') return res({ stdout: METADATA });
      if (c.args[0] === 'rev-parse') return res({ stdout: 'abc1234\n' });
      return res({ stdout: '' });
    });

  test('an empty override builds; the host artifact and its crate version', async () => {
    const { runner, calls } = cargoAndGit(
      res({
        stdout: [
          artifact('strudeland-engine', '/t/engine'),
          artifact('strudeland-host', '/t/host'),
        ].join('\n'),
      }),
    );
    const found = await findHostBinary({ STRUDELAND_HOST_BIN: '' }, runner, '/repo');
    expect(found).toEqual({ path: '/t/host', version: 'strudeland-host 0.2.0 (abc1234)' });
    expect(calls[0]!.args).toEqual([
      'build',
      '--release',
      '-p',
      'strudeland-host',
      '--message-format=json',
    ]);
    expect(calls[0]!.opts?.cwd).toBe('/repo');
  });

  test('a failed build and a build without the host artifact are build-missing', async () => {
    const failed = cargoAndGit(res({ code: 101, stderr: 'error[E0425]' }));
    const e1 = await buildHost(failed.runner, '/repo').catch((x) => x);
    expect(e1).toBeInstanceOf(HarnessError);
    expect(e1.code).toBe('build-missing');
    expect(e1.message).toBe('cargo build of strudeland-host exited 101: error[E0425]');
    const none = cargoAndGit(res({ stdout: artifact('strudeland-engine', '/t/engine') }));
    const e2 = await buildHost(none.runner, '/repo').catch((x) => x);
    expect(e2.message).toBe('cargo build printed no strudeland-host binary artifact');
    const missing = fakeRunner(() => 'enoent');
    const e3 = await buildHost(missing.runner, '/repo').catch((x) => x);
    expect(e3.code).toBe('build-missing');
    expect(e3.message).toStartWith('cannot run cargo: ');
  });
});

describe('SongFiles', () => {
  test(
    'names a song by its hash and writes it once per writer, replacing a stale file',
    withDir(async (dir) => {
      const songs = new SongFiles(join(dir, 'songs'));
      const path = join(dir, 'songs', `${sha256('a')}.js`);
      mkdirSync(join(dir, 'songs'));
      writeFileSync(path, 'truncat');
      expect(songs.file('a')).toEqual({ path, hash: sha256('a') });
      expect(readFileSync(path, 'utf8')).toBe('a');
      // Written once: a second ask leaves the file alone.
      writeFileSync(path, 'edited');
      expect(songs.file('a').path).toBe(path);
      expect(readFileSync(path, 'utf8')).toBe('edited');
      // A new writer (a new run) writes it again.
      expect(new SongFiles(join(dir, 'songs')).file('a').path).toBe(path);
      expect(readFileSync(path, 'utf8')).toBe('a');
      expect(readdirSync(join(dir, 'songs'))).toEqual([`${sha256('a')}.js`]);
    }),
  );
});

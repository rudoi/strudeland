import { afterAll, describe, expect, test } from 'bun:test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import {
  BATCH_TIMEOUT_MS,
  batchRenderArgs,
  BatchRenderer,
  findRenderBinary,
  type BatchTarget,
} from '../scripts/render/batch-render';
import type { EngineProcessRunner, ProcessResult } from '../scripts/render/rust-engine';
import { HarnessError, RenderFailedError } from '../scripts/render/transfer';
import { windowFrames, type RenderRequest } from '../src/lib/render/contract';
import { encodeWav } from '../src/lib/render/wav';
import { ev } from './helpers/render-events';

interface Call {
  cmd: string;
  args: string[];
  opts?: { timeoutMs?: number };
}

function fakeRunner(answer: (c: Call) => ProcessResult | 'enoent') {
  const calls: Call[] = [];
  const runner: EngineProcessRunner = {
    async run(cmd, args, opts) {
      const c = { cmd, args, opts };
      calls.push(c);
      const a = answer(c);
      if (a === 'enoent') throw new Error(`spawn ${cmd} ENOENT`);
      return a;
    },
  };
  return { runner, calls };
}

const res = (o: Partial<ProcessResult> = {}): ProcessResult => ({
  code: 0,
  signal: null,
  stdout: '',
  stderr: '',
  ...o,
});

const OUT = mkdtempSync(join(tmpdir(), 'batch-render-'));
afterAll(() => rmSync(OUT, { recursive: true, force: true }));

const BASE: RenderRequest = {
  code: '// a.js',
  layout: 'clip',
  fromBar: 2,
  toBar: 6,
  overlapBars: 0,
  lookbackSec: 0,
  mode: 'mix',
  sampleRate: 44100,
};
const EVENTS = [ev({ tSec: 4, bar: 2 })];
const TARGET: BatchTarget = {
  name: 'a',
  songPath: '/corpus/a.js',
  base: BASE,
  cps: 0.5,
  events: EVENTS,
};
const FRAMES = windowFrames(2, 6, 0.5, 44100);
const INFO = {
  window: { fromBar: 2, toBar: 6, layout: 'clip' },
  cps: 0.5,
  structure: {},
  lookbackSec: 0,
  errors: [],
  events: EVENTS,
  chunks: [
    { fromBar: 2, toBar: 4 },
    { fromBar: 4, toBar: 6 },
  ],
  summary: {},
};

const files = (info: unknown, frames = FRAMES) => {
  const wav = encodeWav([new Float32Array(frames), new Float32Array(frames)], 44100, 'f32');
  const text = new TextEncoder().encode(typeof info === 'string' ? info : JSON.stringify(info));
  return (p: string): Uint8Array => {
    if (p.endsWith('.info.json')) return text;
    if (p.endsWith('.wav')) return wav;
    throw new Error(`ENOENT: ${p}`);
  };
};

const renderer = (answer: ProcessResult | 'enoent', readFile = files(INFO)) => {
  const { runner, calls } = fakeRunner(() => answer);
  let t = 0;
  const r = new BatchRenderer({
    bin: '/bin/sr',
    outDir: OUT,
    runner,
    readFile,
    now: () => (t += 250),
  });
  return { r, calls };
};

const failure = async (p: Promise<unknown>): Promise<string[]> => {
  try {
    await p;
  } catch (e) {
    if (e instanceof RenderFailedError) return e.lines;
    throw e;
  }
  throw new Error('expected a RenderFailedError');
};

describe('batchRenderArgs', () => {
  test('the explicit window, loudness off, a float WAV, one worker and the info file', () => {
    expect(batchRenderArgs('/s/a.js', BASE, '/o/a.wav', '/o/a.info.json')).toEqual([
      'render',
      '--song',
      '/s/a.js',
      '--from',
      '2',
      '--to',
      '6',
      '--layout',
      'clip',
      '--loudness',
      'off',
      '--format',
      'wav',
      '--bit-depth',
      '32f',
      '--jobs',
      '1',
      '--out',
      '/o/a.wav',
      '--info',
      '/o/a.info.json',
    ]);
  });
});

describe('findRenderBinary', () => {
  test('STRUDELAND_RENDER_BIN skips the build', async () => {
    const { runner, calls } = fakeRunner(() => res());
    const found = await findRenderBinary({ STRUDELAND_RENDER_BIN: 'x/sr' }, runner, '/repo');
    expect(found).toEqual({
      path: resolve('/repo', 'x/sr'),
      version: `strudeland-render (external: ${resolve('/repo', 'x/sr')})`,
    });
    expect(calls).toEqual([]);
  });

  test('otherwise cargo builds strudeland-render', async () => {
    const { runner, calls } = fakeRunner((c) =>
      c.cmd === 'cargo' && c.args[0] === 'build'
        ? res({
            stdout: JSON.stringify({
              reason: 'compiler-artifact',
              target: { name: 'strudeland-render', kind: ['bin'] },
              executable: '/t/release/strudeland-render',
            }),
          })
        : res({ code: 1 }),
    );
    const found = await findRenderBinary({}, runner, '/repo');
    expect(found.path).toBe('/t/release/strudeland-render');
    expect(calls[0]!.args).toEqual([
      'build',
      '--release',
      '-p',
      'strudeland-render',
      '--message-format=json',
    ]);
  });
});

describe('BatchRenderer.window', () => {
  test('runs the binary and returns the window with the batch seams', async () => {
    const { r, calls } = renderer(res());
    const a = await r.window(TARGET);
    expect(calls).toEqual([
      {
        cmd: '/bin/sr',
        args: batchRenderArgs(
          '/corpus/a.js',
          BASE,
          join(OUT, 'batch', 'a.wav'),
          join(OUT, 'batch', 'a.info.json'),
        ),
        opts: { timeoutMs: BATCH_TIMEOUT_MS },
      },
    ]);
    expect(a.frames).toBe(FRAMES);
    expect(a.channels.map((c) => c.length)).toEqual([FRAMES, FRAMES]);
    expect(a.seamSecs).toEqual([4]);
    expect(a.bufferStartSec).toBe(4);
    expect(a.events).toBe(EVENTS);
    expect(a.timing).toEqual({ evalMs: 0, renderMs: 250, totalMs: 250 });
  });

  test('a binary that cannot run stops the run', async () => {
    const { r } = renderer('enoent');
    const e = await r.window(TARGET).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(HarnessError);
    expect((e as HarnessError).code).toBe('build-missing');
  });

  test('a failed render is the fixture error with stderr', async () => {
    expect(
      await failure(
        renderer(res({ code: 1, stderr: 'strudeland-render: boom\n' })).r.window(TARGET),
      ),
    ).toEqual(['batch: strudeland-render exited 1: strudeland-render: boom']);
    expect(
      await failure(renderer(res({ code: null, signal: 'SIGKILL' })).r.window(TARGET)),
    ).toEqual(['batch: strudeland-render killed by SIGKILL: ']);
  });

  test('a bad info file, a bad WAV, a failed cross-check and a short WAV', async () => {
    expect(await failure(renderer(res(), files('{')).r.window(TARGET))).toEqual([
      expect.stringMatching(/^batch info: not JSON/),
    ]);
    const noWav = (p: string): Uint8Array => {
      if (p.endsWith('.wav')) return new Uint8Array(3);
      return files(INFO)(p);
    };
    expect(await failure(renderer(res(), noWav).r.window(TARGET))).toEqual([
      expect.stringMatching(/^batch: the WAV: /),
    ]);
    expect(await failure(renderer(res(), files({ ...INFO, cps: 0.25 })).r.window(TARGET))).toEqual([
      "batch: cps 0.25 differs from the planner's 0.5",
    ]);
    expect(
      await failure(
        renderer(res(), files({ ...INFO, errors: [{ kind: 'other', message: 'x' }] })).r.window(
          TARGET,
        ),
      ),
    ).toEqual(['batch: other: x']);
    expect(await failure(renderer(res(), files(INFO, FRAMES - 2)).r.window(TARGET))).toEqual([
      `batch: the WAV has ${FRAMES - 2}/${FRAMES - 2} frames, the window ${FRAMES}`,
    ]);
  });
});

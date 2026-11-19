import { describe, expect, test } from 'bun:test';
import {
  DEFAULT_CONCURRENCY,
  DEFAULT_OUT_DIR,
  isBuildStale,
  MAX_CONCURRENCY,
  parseCheckArgs,
  plansWithHost,
  selectFixtures,
  USAGE,
  type CheckOptions,
} from '../src/lib/render/check-options';
import type { Corpus, FixtureEntry } from '../src/lib/render/corpus';

const ok = (argv: string[]): CheckOptions => {
  const r = parseCheckArgs(argv);
  if ('error' in r) throw new Error(r.error);
  return r;
};
const err = (argv: string[]): string => {
  const r = parseCheckArgs(argv);
  if (!('error' in r)) throw new Error('expected an error');
  return r.error;
};

describe('parseCheckArgs', () => {
  test('defaults', () => {
    expect(ok([])).toEqual({
      engine: 'superdough',
      planner: 'page',
      filters: [],
      updateGoldens: false,
      keepWav: false,
      mutations: true,
      concurrency: DEFAULT_CONCURRENCY,
      rebuild: false,
      outDir: DEFAULT_OUT_DIR,
      help: false,
    });
    expect(DEFAULT_OUT_DIR).toBe('.render-out');
    expect(DEFAULT_CONCURRENCY).toBeGreaterThanOrEqual(1);
    expect(DEFAULT_CONCURRENCY).toBeLessThanOrEqual(MAX_CONCURRENCY);
  });

  test('each flag', () => {
    expect(ok(['--update-goldens']).updateGoldens).toBe(true);
    expect(ok(['--keep-wav']).keepWav).toBe(true);
    expect(ok(['--no-mutations']).mutations).toBe(false);
    expect(ok(['--rebuild']).rebuild).toBe(true);
    expect(ok(['--out', 'x/y']).outDir).toBe('x/y');
    expect(ok(['--out=z']).outDir).toBe('z');
    expect(ok(['-h']).help).toBe(true);
    expect(ok(['--help']).help).toBe(true);
    expect(ok(['--concurrency', '2']).concurrency).toBe(2);
    expect(ok(['--concurrency=3']).concurrency).toBe(3);
  });

  test('positionals, mixed with flags, in order', () => {
    const o = ok(['pulse', '--keep-wav', 'tag:seam', 'stems-*']);
    expect(o.filters).toEqual(['pulse', 'tag:seam', 'stems-*']);
    expect(o.keepWav).toBe(true);
  });

  test('--concurrency bounds and non-integers', () => {
    expect(ok(['--concurrency', '1']).concurrency).toBe(1);
    expect(ok(['--concurrency', String(MAX_CONCURRENCY)]).concurrency).toBe(MAX_CONCURRENCY);
    const msg = `--concurrency must be an integer from 1 to ${MAX_CONCURRENCY}`;
    expect(err(['--concurrency', '0'])).toBe(msg);
    expect(err(['--concurrency', String(MAX_CONCURRENCY + 1)])).toBe(msg);
    expect(err(['--concurrency', '1.5'])).toBe(msg);
    expect(err(['--concurrency', 'two'])).toBe(msg);
    expect(err(['--concurrency=-1'])).toBe(msg);
  });

  test('--engine: both values, inline or separate', () => {
    expect(ok(['--engine', 'rust']).engine).toBe('rust');
    expect(ok(['--engine=rust']).engine).toBe('rust');
    expect(ok(['--engine', 'superdough']).engine).toBe('superdough');
    expect(ok(['--engine', 'rust', '--engine', 'superdough']).engine).toBe('superdough');
    const o = ok(['bass-line', '--engine', 'rust', 'duck']);
    expect(o.filters).toEqual(['bass-line', 'duck']);
  });

  test('--engine: missing and bad values', () => {
    expect(err(['--engine'])).toBe('--engine needs a value');
    expect(err(['--engine='])).toBe('--engine needs a value');
    expect(err(['--engine', '--keep-wav'])).toBe('--engine needs a value');
    const bad = (v: string) => `--engine must be superdough or rust, got '${v}'`;
    expect(err(['--engine', 'chromium'])).toBe(bad('chromium'));
    expect(err(['--engine=Rust'])).toBe(bad('Rust'));
  });

  test('--engine rust refuses --update-goldens, in either order', () => {
    const msg = '--update-goldens with --engine rust: engine renders never write goldens';
    expect(err(['--engine', 'rust', '--update-goldens'])).toBe(msg);
    expect(err(['--update-goldens', 'x', '--engine=rust'])).toBe(msg);
    expect(ok(['--engine', 'superdough', '--update-goldens']).updateGoldens).toBe(true);
  });

  test('--planner: page by default, host with --engine rust in either order', () => {
    expect(ok([]).planner).toBe('page');
    expect(ok(['--engine', 'rust']).planner).toBe('page');
    expect(ok(['--planner', 'page']).planner).toBe('page');
    expect(ok(['--planner', 'page', '--engine', 'superdough']).planner).toBe('page');
    const host = ok(['--planner', 'host', '--engine', 'rust']);
    expect(host.planner).toBe('host');
    expect(host.engine).toBe('rust');
    expect(ok(['--engine=rust', '--planner=host', 'duck']).planner).toBe('host');
    expect(ok(['--engine=rust', '--planner=host', 'duck']).filters).toEqual(['duck']);
  });

  test('--planner host requires --engine rust', () => {
    const msg = '--planner host requires --engine rust';
    expect(err(['--planner', 'host'])).toBe(msg);
    expect(err(['--planner', 'host', '--engine', 'superdough'])).toBe(msg);
    expect(err(['--engine', 'rust', '--planner', 'host', '--engine', 'superdough'])).toBe(msg);
  });

  test('--planner: missing and bad values', () => {
    expect(err(['--planner'])).toBe('--planner needs a value');
    expect(err(['--planner='])).toBe('--planner needs a value');
    expect(err(['--planner', '--engine'])).toBe('--planner needs a value');
    expect(err(['--planner', 'x'])).toBe(`--planner must be page, host or batch, got 'x'`);
    expect(err(['--planner', 'nope', '--engine', 'rust'])).toBe(
      `--planner must be page, host or batch, got 'nope'`,
    );
    expect(err(['--engine', 'rust', '--planner=Host'])).toBe(
      `--planner must be page, host or batch, got 'Host'`,
    );
    expect(err(['--engine', 'rust', '--planner=Batch'])).toBe(
      `--planner must be page, host or batch, got 'Batch'`,
    );
  });

  test('--planner batch with --engine rust in either order', () => {
    const batch = ok(['--planner', 'batch', '--engine', 'rust']);
    expect(batch.planner).toBe('batch');
    expect(batch.engine).toBe('rust');
    expect(ok(['--engine=rust', '--planner=batch', 'duck', '--keep-wav'])).toMatchObject({
      planner: 'batch',
      filters: ['duck'],
      keepWav: true,
    });
  });

  test('--planner batch requires --engine rust', () => {
    const msg = '--planner batch requires --engine rust';
    expect(err(['--planner', 'batch'])).toBe(msg);
    expect(err(['--planner', 'batch', '--engine', 'superdough'])).toBe(msg);
    expect(err(['--engine', 'rust', '--planner', 'batch', '--engine', 'superdough'])).toBe(msg);
  });

  test('plansWithHost: host and batch plan with the headless host, page does not', () => {
    expect(plansWithHost('page')).toBe(false);
    expect(plansWithHost('host')).toBe(true);
    expect(plansWithHost('batch')).toBe(true);
  });

  test('unknown flag', () => {
    expect(err(['--bogus'])).toBe(`unknown option '--bogus'`);
    expect(err(['-x'])).toBe(`unknown option '-x'`);
  });

  test('missing value', () => {
    expect(err(['--concurrency'])).toBe('--concurrency needs a value');
    expect(err(['--out'])).toBe('--out needs a value');
    expect(err(['--out', '--keep-wav'])).toBe('--out needs a value');
    expect(err(['--out='])).toBe('--out needs a value');
  });

  test('a boolean flag with a value is an error', () => {
    expect(err(['--keep-wav=1'])).toBe('--keep-wav takes no value');
  });

  test('usage names every flag', () => {
    for (const f of [
      '--engine',
      '--planner',
      '--update-goldens',
      '--keep-wav',
      '--no-mutations',
      '--concurrency',
      '--rebuild',
      '--out',
      '--help',
    ]) {
      expect(USAGE).toContain(f);
    }
    for (const word of [
      'superdough',
      'rust',
      'STRUDELAND_ENGINE_BIN',
      'unsupported',
      'page',
      'host',
      'STRUDELAND_HOST_BIN',
      'batch',
      'strudeland-render',
      'STRUDELAND_RENDER_BIN',
    ]) {
      expect(USAGE).toContain(word);
    }
  });
});

const fx = (name: string, tags: string[] = []): FixtureEntry => ({
  name,
  file: `${name}.js`,
  kind: 'targeted',
  windowBars: [0, 4],
  layout: 'linear',
  gates: ['mix'],
  stochastic: false,
  usesSamples: false,
  rust: 'supported',
  tags,
});
const corpus: Corpus = {
  schema: 1,
  samples: [],
  fixtures: [
    { ...fx('pulse', ['9/8']), kind: 'external' },
    fx('stems-5', ['stems']),
    fx('stems-shared', ['stems']),
    fx('duck'),
  ],
};
const names = (r: FixtureEntry[] | { error: string }) => ('error' in r ? r : r.map((f) => f.name));

describe('selectFixtures', () => {
  test('no filters selects every fixture', () => {
    expect(names(selectFixtures(corpus, []))).toEqual(['pulse', 'stems-5', 'stems-shared', 'duck']);
  });
  test('exact name', () => {
    expect(names(selectFixtures(corpus, ['duck']))).toEqual(['duck']);
    expect(names(selectFixtures(corpus, ['stems']))).toEqual({
      error: `no fixture matches 'stems'`,
    });
  });
  test('glob matches whole names', () => {
    expect(names(selectFixtures(corpus, ['stems-*']))).toEqual(['stems-5', 'stems-shared']);
    expect(names(selectFixtures(corpus, ['*u*']))).toEqual(['pulse', 'duck']);
    expect(names(selectFixtures(corpus, ['s*5']))).toEqual(['stems-5']);
  });
  test('tag filter', () => {
    expect(names(selectFixtures(corpus, ['tag:stems']))).toEqual(['stems-5', 'stems-shared']);
    expect(names(selectFixtures(corpus, ['tag:9/8']))).toEqual(['pulse']);
  });
  test('several filters keep manifest order without duplicates', () => {
    expect(names(selectFixtures(corpus, ['duck', 'tag:stems', 'stems-5']))).toEqual([
      'stems-5',
      'stems-shared',
      'duck',
    ]);
  });
  test('a filter that matches nothing is an error', () => {
    expect(selectFixtures(corpus, ['duck', 'nope'])).toEqual({
      error: `no fixture matches 'nope'`,
    });
    expect(selectFixtures(corpus, ['tag:none'])).toEqual({
      error: `no fixture matches 'tag:none'`,
    });
  });
  test('regex characters in a filter are literal', () => {
    expect(selectFixtures(corpus, ['stems.5'])).toEqual({ error: `no fixture matches 'stems.5'` });
  });
});

describe('isBuildStale', () => {
  test('missing build', () => expect(isBuildStale(100, null)).toBe(true));
  test('older build', () => expect(isBuildStale(100, 99)).toBe(true));
  test('newer build', () => expect(isBuildStale(100, 101)).toBe(false));
  test('equal mtimes are not stale', () => expect(isBuildStale(100, 100)).toBe(false));
});

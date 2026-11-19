/**
 * `render:check` command line: argument parsing, fixture selection and the render-page build
 * staleness test. Pure: no fs, Strudel, DOM or Playwright imports.
 */

import type { Corpus, FixtureEntry } from './corpus';
import { ENGINE_TRAITS, type EngineName } from './engines';

const ENGINES = Object.keys(ENGINE_TRAITS) as EngineName[];

/** Where `--engine rust` plans its jobs: the render page in Chromium or the headless host;
 *  `batch` is `host`, and the main window of each fixture is rendered by the whole Rust
 *  pipeline (`strudeland-render`). */
export type PlannerName = 'page' | 'host' | 'batch';
const PLANNERS: PlannerName[] = ['page', 'host', 'batch'];

/** The headless host plans every request (no render page, no browser). */
export const plansWithHost = (planner: PlannerName): boolean => planner !== 'page';

export interface CheckOptions {
  /** `--engine`; default superdough. */
  engine: EngineName;
  /** `--planner`; default page. `host` and `batch` require `--engine rust`. */
  planner: PlannerName;
  /** Fixture names, globs (`*` = any run of characters) or `tag:<tag>`; empty = all. */
  filters: string[];
  updateGoldens: boolean;
  keepWav: boolean;
  mutations: boolean;
  /** Renders (pages) at once, 1..MAX_CONCURRENCY. */
  concurrency: number;
  rebuild: boolean;
  outDir: string;
  help: boolean;
}

/** Renders (pages) at once: long fixtures take minutes each in one page. */
export const MAX_CONCURRENCY = 4;
export const DEFAULT_CONCURRENCY = MAX_CONCURRENCY;
export const DEFAULT_OUT_DIR = '.render-out';

export const USAGE = `usage: bun run render:check [options] [fixture | glob | tag:<tag>]...

Renders the corpus in headless Chromium, runs each fixture's gates and compares its features
with tests/render/goldens/<fixture>.json. No fixture arguments = every fixture.

options:
  --engine E         superdough (default) or rust: render with the Rust engine (built with
                     cargo, or STRUDELAND_ENGINE_BIN) and judge it against the superdough
                     goldens; fixtures it cannot play are reported unsupported
  --planner P        page (default), host or batch: with --engine rust, plan the engine jobs on
                     the render page in Chromium or with the headless host (strudeland-host,
                     built with cargo, or STRUDELAND_HOST_BIN; no Chromium). The host reports
                     the errors it captures (console.warn/error, thrown errors), not the
                     page-only ones (unhandled rejections, page errors). batch: like host, and
                     the main window of each fixture is rendered by the whole Rust pipeline
                     (strudeland-render, built with cargo, or STRUDELAND_RENDER_BIN)
  --update-goldens   write goldens for fixtures whose gates pass (parity is not judged;
                     superdough only)
  --keep-wav         write the float32 mix to <out>/wav/<fixture>.wav (and .stems.wav)
  --no-mutations     skip the mutation set (the gates must still catch known defects)
  --concurrency N    renders at once, 1-${MAX_CONCURRENCY} (default ${DEFAULT_CONCURRENCY})
  --rebuild          build the render page even if it is not stale
  --out DIR          report and WAV folder (default ${DEFAULT_OUT_DIR})
  -h, --help         this text

exit codes: 0 all pass (with --engine rust, fixtures it cannot play pass as unsupported when
corpus.json marks them so), 1 a fixture failed, a mutation was missed, or a fixture's rust
support disagrees with corpus.json, 2 harness error (usage, invalid corpus, Chromium or render
page missing, engine or host build failed, render page failure, stall)`;

const defaults = (): CheckOptions => ({
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

/** The flags that take no value and the option each one sets. */
const SWITCHES: Record<string, (o: CheckOptions) => void> = {
  '-h': (o) => (o.help = true),
  '--help': (o) => (o.help = true),
  '--update-goldens': (o) => (o.updateGoldens = true),
  '--keep-wav': (o) => (o.keepWav = true),
  '--no-mutations': (o) => (o.mutations = false),
  '--rebuild': (o) => (o.rebuild = true),
};

export function parseCheckArgs(argv: string[]): CheckOptions | { error: string } {
  const o = defaults();
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (!arg.startsWith('-') || arg === '-') {
      o.filters.push(arg);
      continue;
    }
    const eq = arg.startsWith('--') ? arg.indexOf('=') : -1;
    const flag = eq >= 0 ? arg.slice(0, eq) : arg;
    const inline = eq >= 0 ? arg.slice(eq + 1) : undefined;
    const value = (): string | { error: string } => {
      if (inline !== undefined) return inline === '' ? { error: `${flag} needs a value` } : inline;
      const next = argv[i + 1];
      if (next === undefined || next.startsWith('-')) return { error: `${flag} needs a value` };
      i++;
      return next;
    };
    const setSwitch = Object.hasOwn(SWITCHES, flag) ? SWITCHES[flag] : undefined;
    if (setSwitch) {
      if (inline !== undefined) return { error: `${flag} takes no value` };
      setSwitch(o);
      continue;
    }
    switch (flag) {
      case '--concurrency': {
        const v = value();
        if (typeof v !== 'string') return v;
        const n = /^\d+$/.test(v) ? Number(v) : NaN;
        if (!Number.isInteger(n) || n < 1 || n > MAX_CONCURRENCY) {
          return { error: `--concurrency must be an integer from 1 to ${MAX_CONCURRENCY}` };
        }
        o.concurrency = n;
        break;
      }
      case '--engine': {
        const v = value();
        if (typeof v !== 'string') return v;
        if (!(ENGINES as string[]).includes(v)) {
          return { error: `--engine must be ${ENGINES.join(' or ')}, got '${v}'` };
        }
        o.engine = v as EngineName;
        break;
      }
      case '--planner': {
        const v = value();
        if (typeof v !== 'string') return v;
        if (!(PLANNERS as string[]).includes(v)) {
          return { error: `--planner must be page, host or batch, got '${v}'` };
        }
        o.planner = v as PlannerName;
        break;
      }
      case '--out': {
        const v = value();
        if (typeof v !== 'string') return v;
        o.outDir = v;
        break;
      }
      default:
        return { error: `unknown option '${arg}'` };
    }
  }
  if (o.planner !== 'page' && o.engine !== 'rust') {
    return { error: `--planner ${o.planner} requires --engine rust` };
  }
  if (o.updateGoldens && !ENGINE_TRAITS[o.engine].writesGoldens) {
    return {
      error: `--update-goldens with --engine ${o.engine}: engine renders never write goldens`,
    };
  }
  return o;
}

function globRegExp(glob: string): RegExp {
  const body = glob
    .split('*')
    .map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
    .join('.*');
  return new RegExp(`^${body}$`);
}

function matches(f: FixtureEntry, filter: string): boolean {
  if (filter.startsWith('tag:')) return f.tags.includes(filter.slice(4));
  return globRegExp(filter).test(f.name);
}

/** The fixtures matched by any filter, in manifest order; all of them for no filters. */
export function selectFixtures(
  corpus: Corpus,
  filters: string[],
): FixtureEntry[] | { error: string } {
  if (filters.length === 0) return [...corpus.fixtures];
  for (const filter of filters) {
    if (!corpus.fixtures.some((f) => matches(f, filter))) {
      return { error: `no fixture matches '${filter}'` };
    }
  }
  return corpus.fixtures.filter((f) => filters.some((filter) => matches(f, filter)));
}

/** The render page needs a build when it is missing or older than its newest source. */
export function isBuildStale(newestSourceMs: number, builtMs: number | null): boolean {
  return builtMs === null || builtMs < newestSourceMs;
}

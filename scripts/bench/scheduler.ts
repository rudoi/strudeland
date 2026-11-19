/**
 * Scheduler bench: times the webview scheduler (the real Cyclist, under bun/JSC) on a song, with
 * a backend that only counts haps. Prints JSON on stdout and a table on stderr.
 * Exit 0 ok, 2 bad arguments or an evaluate failure.
 */
import { createRuntime } from '../../src/lib/strudel/runtime';
import type { AudioBackend } from '../../src/lib/audio/backend';
import { hostPrebake } from '../../src/host/prebake';
import { parseMeta } from '../../src/lib/project/meta';
import { evaluateOptions, staticStructure } from '../../src/lib/project/evaluate-options';
import { median, nearestRank, sortedAsc } from '../../src/lib/render/dsp';

export interface Options {
  song: string;
  seconds: number;
  section: string;
  out: string | null;
}

export const DEFAULTS: Options = {
  song: 'tests/bench/live-pressure.js',
  seconds: 30,
  section: 'orchestra',
  out: null,
};

export const RUNS = 3;
const WARMUP_SECONDS = 5;
const FIRE_STEP = 0.1;

export class UsageError extends Error {}

/** Parses the command line (without `bun` and the script path); throws `UsageError`. */
export function parseArgs(argv: string[]): Options {
  const o = { ...DEFAULTS };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    const value = () => {
      const v = argv[++i];
      if (v === undefined) throw new UsageError(`${flag} needs a value`);
      return v;
    };
    if (flag === '--song') o.song = value();
    else if (flag === '--section') o.section = value();
    else if (flag === '--out') o.out = value();
    else if (flag === '--seconds') {
      const raw = value();
      const n = Number(raw);
      if (!Number.isFinite(n) || n <= 0) throw new UsageError(`--seconds must be positive: ${raw}`);
      o.seconds = n;
    } else throw new UsageError(`unknown argument: ${flag}`);
  }
  return o;
}

export interface RunResult {
  msPerFire: { p50: number; max: number };
  msPerSecond: number;
  haps: number;
}

export function summarize(fireMs: number[], seconds: number, haps: number): RunResult {
  const total = fireMs.reduce((a, b) => a + b, 0);
  return {
    msPerFire: { p50: nearestRank(sortedAsc(fireMs), 0.5), max: Math.max(...fireMs) },
    msPerSecond: total / seconds,
    haps,
  };
}

export interface Report {
  song: string;
  section: string;
  seconds: number;
  runs: RunResult[];
  msPerSecond: number;
  msPerFire: { p50: number; max: number };
  haps: number;
  bun: string;
  strudel: string;
}

export function buildReport(
  o: Options,
  runs: RunResult[],
  env: { bun: string; strudel: string },
): Report {
  return {
    song: o.song,
    section: o.section,
    seconds: o.seconds,
    runs,
    msPerSecond: median(runs.map((r) => r.msPerSecond)),
    msPerFire: {
      p50: median(runs.map((r) => r.msPerFire.p50)),
      max: Math.max(...runs.map((r) => r.msPerFire.max)),
    },
    haps: Math.min(...runs.map((r) => r.haps)),
    bun: env.bun,
    strudel: env.strudel,
  };
}

export function formatTable(r: Report): string {
  const f = (n: number) => n.toFixed(2).padStart(9);
  const lines = [
    `scheduler bench: ${r.song} [${r.section}], ${r.seconds}s simulated, bun ${r.bun}, strudel ${r.strudel}`,
    'run   msPerSecond  fire p50  fire max      haps',
    ...r.runs.map(
      (x, i) =>
        `${String(i + 1).padStart(3)} ${f(x.msPerSecond)}  ${f(x.msPerFire.p50)} ${f(x.msPerFire.max)} ${String(x.haps).padStart(9)}`,
    ),
    `median msPerSecond: ${r.msPerSecond.toFixed(2)}`,
  ];
  return lines.join('\n');
}

async function oneRun(code: string, o: Options): Promise<RunResult> {
  const { meta } = parseMeta(code);
  const structure = staticStructure(meta);
  const index = structure.sections.findIndex((s) => s.name === o.section);
  if (index < 0) throw new UsageError(`the song has no section named '${o.section}'`);
  let now = 0;
  let haps = 0;
  const timers = new Map<number, () => void>();
  let next = 0;
  const backend: AudioBackend = {
    id: 'superdough',
    init: async () => {},
    resume: async () => true,
    isRunning: () => true,
    getTime: () => now,
    setSeed: () => {},
    output: () => {
      haps++;
    },
  };
  const rt = createRuntime({
    backends: { superdough: backend },
    prebake: hostPrebake,
    setInterval: (fn) => (timers.set(++next, fn), next),
    clearInterval: (id) => void timers.delete(id as number),
  });
  const fire = () => [...timers.values()].forEach((f) => f());
  const ok = await rt.evaluate(code, { ...evaluateOptions(meta), autostart: false });
  if (!ok) throw new Error('the song failed to evaluate');
  rt.transport.startFrom(index, structure);
  if (!(await rt.start())) throw new Error('the runtime did not start');
  for (let t = 0; t < WARMUP_SECONDS; t += FIRE_STEP) {
    now += FIRE_STEP;
    fire();
  }
  haps = 0;
  const fireMs: number[] = [];
  const steps = Math.round(o.seconds / FIRE_STEP);
  for (let i = 0; i < steps; i++) {
    now += FIRE_STEP;
    const t0 = performance.now();
    fire();
    fireMs.push(performance.now() - t0);
  }
  rt.stop();
  return summarize(fireMs, steps * FIRE_STEP, haps);
}

async function strudelHead(): Promise<string> {
  try {
    const j = JSON.parse(await Bun.file('vendor/strudel/UPSTREAM.json').text());
    return String(j.commit);
  } catch {
    return 'unknown';
  }
}

async function main(argv: string[]): Promise<number> {
  let o: Options;
  try {
    o = parseArgs(argv);
    const code = await Bun.file(o.song).text();
    const runs: RunResult[] = [];
    for (let i = 0; i < RUNS; i++) runs.push(await oneRun(code, o));
    const report = buildReport(o, runs, { bun: Bun.version, strudel: await strudelHead() });
    console.error(formatTable(report));
    const json = JSON.stringify(report, null, 2);
    console.log(json);
    if (o.out) await Bun.write(o.out, json + '\n');
    return 0;
  } catch (e) {
    console.error(`bench:scheduler: ${(e as Error).message}`);
    return 2;
  }
}

if (import.meta.main) process.exit(await main(process.argv.slice(2)));

/**
 * The `render:check` report: per-fixture gate and parity verdicts, the mutation set and the
 * golden summary, as a console table and as JSON. Pure.
 */

import { serializeFeatures, type Features } from './features';
import { round } from './dsp';
import type { EngineName } from './engines';
import {
  GATE_NAMES,
  type GateName,
  type GateResult,
  type GateVerdict,
  type Json,
} from './gates/types';
import type { ParityResult } from './parity';

export interface FixtureReport {
  name: string;
  /** `unsupported`: the Rust engine cannot play some haps (no gate, parity or mutation ran);
   *  it fails the run only with `errors` (corpus.json disagrees). */
  status: 'pass' | 'fail' | 'error' | 'unsupported';
  bars: number;
  chunks: number;
  seconds: number;
  gates: GateResult[];
  /** Null when parity was not judged (render error, missing golden, --update-goldens). */
  parity: ParityResult | null;
  /** Why the fixture failed outside a gate or `compareFeatures` (missing or unreadable golden);
   *  printed as gate `parity`. */
  parityNote: string | null;
  /** Formatted RenderError lines (`kind: message`, `(bar N)`): status `error`; for status
   *  `unsupported`, the corpus.json disagreement. */
  errors: string[];
  /** The unsupported signatures (`s=<s> keys=<k1,k2> bar=<n>`), status `unsupported` only. */
  unsupported: string[];
}

export type MutationStatus = 'caught' | 'missed' | 'skipped' | 'error';

export interface MutationReport {
  name: string;
  fixture: string;
  gate: GateName;
  status: MutationStatus;
  detail: string;
}

export interface GoldenChange {
  name: string;
  state: 'new' | 'unchanged' | 'changed' | 'not-written';
  frames?: number;
  peakDb?: number;
  lufs?: number | null;
  bandsMaxDb?: number;
  hashFrom?: string;
  hashTo?: string;
}

export interface Report {
  schema: 1;
  startedAt: string;
  /** Which engine rendered. */
  engine: EngineName;
  /** The Rust build (`strudeland-engine <version> (<sha>)`); null for superdough. */
  engineVersion: string | null;
  playwright: string;
  chromium: string;
  fixtures: FixtureReport[];
  mutations: MutationReport[];
  goldens: GoldenChange[];
  /** Set when a fatal harness error stopped the run (exit 2); the fixtures are those finished. */
  harnessError: string | null;
}

const r2 = (x: number) => round(x, 2);

/** One gate's result over several parts (seams, chunks): fail wins, then pass, else
 *  unverifiable (also for no parts). */
export function combineResults(gate: GateName, results: GateResult[]): GateResult {
  const prefix = (lists: string[][]) =>
    lists.flatMap((list, i) => list.map((line) => `#${i}: ${line}`));
  const verdict: GateVerdict = results.some((r) => r.verdict === 'fail')
    ? 'fail'
    : results.some((r) => r.verdict === 'pass')
      ? 'pass'
      : 'unverifiable';
  const reasons = prefix(results.map((r) => r.reasons));
  if (results.length === 0) reasons.push('no results');
  return {
    gate,
    verdict,
    measurements: { parts: results.map((r) => r.measurements as Json) },
    reasons,
    warnings: prefix(results.map((r) => r.warnings)),
  };
}

/** How a freshly written golden differs from the one it replaces (`old` null = none). */
export function goldenSummary(name: string, old: Features | null, next: Features): GoldenChange {
  if (old === null) return { name, state: 'new' };
  if (serializeFeatures(old) === serializeFeatures(next)) return { name, state: 'unchanged' };
  const change: GoldenChange = {
    name,
    state: 'changed',
    frames: next.frames - old.frames,
    peakDb: r2(next.peakDb - old.peakDb),
    lufs: next.lufs === null || old.lufs === null ? null : r2(next.lufs - old.lufs),
  };
  let bandsMax = 0;
  for (const band of Object.keys(next.bandsDb) as (keyof Features['bandsDb'])[]) {
    const a = old.bandsDb[band];
    const b = next.bandsDb[band];
    for (let i = 0; i < Math.min(a.length, b.length); i++) {
      bandsMax = Math.max(bandsMax, Math.abs(b[i]! - a[i]!));
    }
  }
  change.bandsMaxDb = r2(bandsMax);
  if (old.pcmSha256 !== next.pcmSha256) {
    change.hashFrom = old.pcmSha256.slice(0, 8);
    change.hashTo = next.pcmSha256.slice(0, 8);
  }
  return change;
}

const signed = (x: number, digits: number) =>
  (x > 0 ? '+' : x < 0 ? '-' : digits > 0 ? '+' : '') + Math.abs(x).toFixed(digits);

function goldenLine(g: GoldenChange, nameWidth: number): string {
  const head = `golden  ${g.name.padEnd(nameWidth)}`;
  if (g.state === 'not-written') return `${head} not written (gates failed)`;
  if (g.state !== 'changed') return `${head} ${g.state}`;
  const parts = [`${head} changed`];
  if (g.frames !== undefined) parts.push(`frames ${signed(g.frames, 0)}`);
  if (g.peakDb !== undefined) parts.push(`peak ${signed(g.peakDb, 2)} dB`);
  if (g.lufs !== undefined && g.lufs !== null) parts.push(`lufs ${signed(g.lufs, 2)} LU`);
  if (g.bandsMaxDb !== undefined) parts.push(`bands max ${g.bandsMaxDb.toFixed(2)} dB`);
  if (g.hashFrom !== undefined && g.hashTo !== undefined) {
    parts.push(`hash ${g.hashFrom} -> ${g.hashTo}`);
  }
  return parts.join('  ');
}

function goldenAggregate(goldens: GoldenChange[]): string {
  const n = (s: GoldenChange['state']) => goldens.filter((g) => g.state === s).length;
  return (
    `goldens: ${n('new')} new, ${n('changed')} changed, ${n('unchanged')} unchanged, ` +
    `${n('not-written')} not written`
  );
}

/** Whether the fixture fails the run: it did not pass, unless it is unsupported as corpus.json
 *  says (an unsupported fixture carries errors only when the manifest disagrees). */
const fixtureFails = (f: FixtureReport): boolean =>
  f.status === 'unsupported' ? f.errors.length > 0 : f.status !== 'pass';

const verdictCell = (v: GateVerdict | undefined): string =>
  v === undefined ? '-' : v === 'pass' ? 'pass' : v === 'fail' ? 'FAIL' : 'unver';

function table(rows: string[][]): string[] {
  const widths = rows[0]!.map((_, c) => Math.max(...rows.map((r) => r[c]!.length)));
  return rows.map((r) =>
    r
      .map((cell, c) => (c === r.length - 1 ? cell : cell.padEnd(widths[c]!)))
      .join('  ')
      .trimEnd(),
  );
}

/** Reasons (failing fixtures only) and warnings (every fixture) under the table. */
function detailLines(f: FixtureReport): string[] {
  const out: string[] = [];
  if (f.status === 'unsupported') {
    for (const e of f.errors) out.push(`  ${f.name} rust: ${e}`);
    for (const u of f.unsupported) out.push(`  ${f.name} unsupported: ${u}`);
    return out;
  }
  if (f.status !== 'pass') {
    for (const e of f.errors) out.push(`  ${f.name} render: ${e}`);
    for (const g of f.gates) {
      if (g.verdict === 'pass') continue;
      // an unverifiable gate still standing was expected; its reasons are not failures
      if (g.verdict === 'unverifiable') continue;
      for (const r of g.reasons) out.push(`  ${f.name} ${g.gate}: ${r}`);
    }
    if (f.parityNote !== null) out.push(`  ${f.name} parity: ${f.parityNote}`);
    if (f.parity && f.parity.verdict !== 'pass') {
      for (const r of f.parity.reasons) out.push(`  ${f.name} parity: ${r}`);
    }
  }
  for (const g of f.gates)
    for (const w of g.warnings) out.push(`  ${f.name} ${g.gate} warning: ${w}`);
  if (f.parity) for (const w of f.parity.warnings) out.push(`  ${f.name} parity warning: ${w}`);
  return out;
}

export function formatReport(r: Report): string {
  const lines: string[] = [];
  const engine = r.engineVersion === null ? r.engine : `${r.engine} (${r.engineVersion})`;
  lines.push(
    `render:check  engine ${engine}  playwright ${r.playwright}  chromium ${r.chromium}  ` +
      r.startedAt,
  );
  const rows: string[][] = [
    ['fixture', 'bars', 'chunks', ...GATE_NAMES, 'parity', 'time'],
    ...r.fixtures.map((f) => [
      f.name,
      String(f.bars),
      String(f.chunks),
      ...GATE_NAMES.map((g) => verdictCell(f.gates.find((x) => x.gate === g)?.verdict)),
      f.status === 'unsupported'
        ? f.errors.length > 0
          ? 'UNSUP'
          : 'unsup'
        : verdictCell(f.parity?.verdict),
      `${f.seconds.toFixed(1)}s`,
    ]),
  ];
  lines.push(...table(rows));
  for (const f of r.fixtures) lines.push(...detailLines(f));

  if (r.mutations.length > 0) {
    const count = (s: MutationStatus) => r.mutations.filter((m) => m.status === s).length;
    const parts = [`${count('caught')} caught`];
    for (const s of ['missed', 'error', 'skipped'] as const) {
      if (count(s) > 0) parts.push(`${count(s)} ${s}`);
    }
    lines.push(`mutations: ${parts.join(', ')}`);
    const w = Math.max(...r.mutations.map((m) => m.name.length));
    const fw = Math.max(...r.mutations.map((m) => `${m.fixture} ${m.gate}`.length));
    for (const m of r.mutations) {
      const status = m.status === 'missed' ? 'MISSED' : m.status === 'error' ? 'ERROR' : m.status;
      const detail = m.detail ? `  ${m.detail}` : '';
      lines.push(
        `  ${m.name.padEnd(w)}  ${`${m.fixture} ${m.gate}`.padEnd(fw)}  ${status}${detail}`,
      );
    }
  }

  if (r.goldens.length > 0) {
    const w = Math.max(...r.goldens.map((g) => g.name.length));
    for (const g of r.goldens) lines.push(goldenLine(g, w));
    lines.push(goldenAggregate(r.goldens));
  }

  if (r.harnessError !== null) lines.push(`harness error: ${r.harnessError}`);
  const pass = r.fixtures.filter((f) => f.status === 'pass').length;
  const unsupported = r.fixtures.filter((f) => f.status === 'unsupported').length;
  const failed = r.fixtures.length - pass - unsupported;
  let last = `${r.fixtures.length} fixtures: ${pass} pass, ${failed} fail`;
  if (unsupported > 0) last += `, ${unsupported} unsupported`;
  const disagree = r.fixtures.filter((f) => f.status === 'unsupported' && f.errors.length > 0);
  if (disagree.length > 0) last += ` (${disagree.length} disagree with corpus.json)`;
  const missed = r.mutations.filter((m) => m.status === 'missed').length;
  const errored = r.mutations.filter((m) => m.status === 'error').length;
  if (missed > 0) last += `, ${missed} mutation(s) missed`;
  if (errored > 0) last += `, ${errored} mutation error(s)`;
  lines.push(last);
  return lines.join('\n') + '\n';
}

export const reportJson = (r: Report): string => JSON.stringify(r, null, 2) + '\n';

/** 1 if any fixture failed (`fixtureFails`: an `unsupported` one only when corpus.json
 *  disagrees) or any mutation was missed or errored; harness errors (exit 2) are the caller's. */
export function reportExitCode(r: Report): 0 | 1 {
  const bad =
    r.fixtures.some(fixtureFails) ||
    r.mutations.some((m) => m.status === 'missed' || m.status === 'error');
  return bad ? 1 : 0;
}

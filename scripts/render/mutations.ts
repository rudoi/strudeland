/**
 * The fixed mutation set: known defects injected into a fixture's render requests, each of which
 * its gate must catch. Run by `render:check` after the fixtures. No Playwright import.
 */

import type { RenderRequest } from '../../src/lib/render/contract';
import type { Corpus } from '../../src/lib/render/corpus';
import type { GateName, GateResult } from '../../src/lib/render/gates/types';
import type { MutationReport } from '../../src/lib/render/report';

export interface MutationSpec {
  name: string;
  fixture: string;
  /** The stems gate renders in stems mode, every other gate in mix mode. */
  gate: GateName;
  /** Applied to every request of the gate's renders (every chunk, re-renders included). */
  apply(req: RenderRequest): RenderRequest;
}

export const MUTATIONS: MutationSpec[] = [
  {
    name: 'seam-lookback-0',
    fixture: 'pad-seam',
    gate: 'seam',
    apply: (r) => ({ ...r, lookbackSec: 0 }),
  },
  {
    name: 'stems-orbit-dropped',
    fixture: 'stems-5',
    gate: 'stems',
    apply: (r) => ({ ...r, mutation: { kind: 'dropOrbit', orbit: 4, bars: [1, 4, 6] } }),
  },
  {
    name: 'bass-dropped',
    fixture: 'bass-line',
    gate: 'bass',
    // Three adjacent bars, not every bar: the bass gate is song-relative (misses are measured
    // against the median note), so dropping every bass note leaves nothing to miss against. Not
    // bar 2: the bar-1 g1 release leaks into its a1 band and reads only ~13 dB down.
    apply: (r) => ({ ...r, mutation: { kind: 'dropNotesBelowHz', hz: 130, bars: [3, 4, 5] } }),
  },
];

/** `caught` iff the mutated gate fails; asserted on the verdict only, never on measurements.
 *  `error` (a failed mutated render, or no result) carries `error` as its detail. */
export function judgeMutation(
  spec: MutationSpec,
  control: GateResult | undefined,
  mutated: GateResult | undefined,
  error?: string,
): MutationReport {
  const base = { name: spec.name, fixture: spec.fixture, gate: spec.gate };
  if (control === undefined) {
    return { ...base, status: 'skipped', detail: 'control did not run' };
  }
  if (control.verdict !== 'pass') {
    return { ...base, status: 'skipped', detail: 'control did not pass' };
  }
  if (error !== undefined || mutated === undefined) {
    return {
      ...base,
      status: 'error',
      detail: error ?? 'the mutated render produced no gate result',
    };
  }
  if (mutated.verdict === 'fail') {
    return { ...base, status: 'caught', detail: mutated.reasons[0] ?? '' };
  }
  return {
    ...base,
    status: 'missed',
    detail: `gate ${spec.gate} returned ${mutated.verdict}; the known defect was not detected`,
  };
}

/** Problems with `specs` against the corpus: unknown fixture, or a gate the fixture does not
 *  list. */
export function validateMutations(corpus: Corpus, specs: MutationSpec[] = MUTATIONS): string[] {
  const problems: string[] = [];
  const names = new Set<string>();
  for (const m of specs) {
    if (names.has(m.name)) problems.push(`mutation '${m.name}': duplicate name`);
    names.add(m.name);
    const fx = corpus.fixtures.find((f) => f.name === m.fixture);
    if (!fx) {
      problems.push(`mutation '${m.name}': fixture '${m.fixture}' is not in the corpus`);
      continue;
    }
    if (!fx.gates.includes(m.gate)) {
      problems.push(`mutation '${m.name}': fixture '${m.fixture}' does not list gate '${m.gate}'`);
    }
  }
  return problems;
}

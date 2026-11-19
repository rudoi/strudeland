/**
 * Mutation sweep: for each MUTATIONS entry, the control and the mutated render of its fixture,
 * plus the clean render of every other fixture carrying that gate (false alarms). Sweeps
 * SEAM_FAIL_DB (T over 0.25..6 against worstDeficitDb and the twin floor), STEMS_MISS_DB and
 * BASS_MISS_DB (6, 8, …, 24) and the bass verdict rule, and writes evidence/mutation-sweep.json
 * with the choices of 04 section 6 probe 3. A unit is a seam, an orbit-bar or a bass note.
 *
 *   bun scripts/render/probes/mutation-sweep.probe.ts [fixture…]
 */

import { chunkRequest, contentOf, headOf, overlapOf } from '../../../src/lib/render/chunks';
import {
  SAMPLE_RATE as SR,
  type HapMutation,
  type RenderRequest,
} from '../../../src/lib/render/contract';
import {
  bassEvents,
  bassGate,
  bassPresence,
  type BassPresence,
} from '../../../src/lib/render/gates/bass';
import { seamConsistency, twinFloorDb } from '../../../src/lib/render/gates/seam';
import {
  judgeStemScores,
  stemsOnsetScores,
  type StemNoteScore,
} from '../../../src/lib/render/gates/stems';
import type { GateName } from '../../../src/lib/render/gates/types';
import {
  BASS_CLUSTER_DEEP_DB,
  BASS_CLUSTER_MIN,
  BASS_CLUSTER_SPAN_BARS,
  BASS_FRACTION_MAX,
  BASS_MISS_DB,
  BASS_WINDOW_SEC,
  SEAM_FAIL_DB,
  SEAM_TOLERANCE_DB,
  STEMS_MISS_DB,
} from '../../../src/lib/render/thresholds';
import {
  assembledOf,
  chunkResults,
  pool,
  renderIn,
  type FixtureContext,
  type RequestTx,
} from '../check';
import { MUTATIONS, type MutationSpec } from '../mutations';
import {
  chooseThreshold,
  closestRule,
  range,
  ruleGrid,
  seamFlagged,
  sweepRow,
  type RuleKey,
  type SweepRow,
} from './calibrate';
import { CONCURRENCY, openProbe, writeEvidence } from './probe-env';

const SWEEP_DB = range(6, 24, 2);
const SEAM_T = [0.25, 0.5, 1, 2, 3, 4, 6];

const env = await openProbe(Bun.argv.slice(2));

interface SeamUnit {
  fixture: string;
  seam: number;
  worstDeficitDb: number;
  /** Only measured when the worst deficit is over the smallest swept T (as on escalation). */
  twinFloorDb: number | null;
}

async function seamUnits(fx: FixtureContext, tx?: RequestTx): Promise<SeamUnit[]> {
  const results = await chunkResults(fx, tx);
  const out: SeamUnit[] = [];
  const opts = { toleranceDb: SEAM_TOLERANCE_DB };
  for (let k = 0; k + 1 < fx.plans.length; k++) {
    const bars = fx.plans[k]!.overlapBars;
    const seamSec = fx.plans[k]!.contentEndBar * fx.barSec;
    const head = headOf(results[k + 1]!, bars);
    const sc = seamConsistency(overlapOf(results[k]!), head, SR, seamSec, opts);
    let floorDb: number | null = null;
    if (sc.worstDeficitDb > Math.min(...SEAM_T)) {
      const req = chunkRequest(fx.base, fx.plans[k + 1]!);
      const twin = headOf(await renderIn(fx, tx ? tx(req) : req), bars);
      floorDb = twinFloorDb(head, twin, SR, seamSec);
    }
    out.push({
      fixture: fx.fixture.name,
      seam: k,
      worstDeficitDb: sc.worstDeficitDb,
      twinFloorDb: floorDb,
    });
  }
  return out;
}

async function stemNotes(fx: FixtureContext, tx?: RequestTx): Promise<StemNoteScore[]> {
  const notes: StemNoteScore[] = [];
  for (const plan of fx.plans) {
    const req: RenderRequest = { ...chunkRequest(fx.base, plan), mode: 'stems', overlapBars: 0 };
    const r = await renderIn(fx, tx ? tx(req) : req);
    notes.push(
      ...stemsOnsetScores(contentOf(r), SR, fx.sched.events, fx.barSec, {
        offsetSec: r.request.fromBar / fx.cps,
      }).notes,
    );
  }
  return notes;
}

interface BassRun {
  fixture: string;
  barSec: number;
  /** BassPresence per swept missDb (and the current BASS_MISS_DB). */
  byDb: Map<number, BassPresence>;
  /** Eligible, in-buffer bass notes the mutation removes (mutated runs only). */
  targets: { bar: number; tSec: number }[];
}

async function bassRun(
  fx: FixtureContext,
  tx?: RequestTx,
  mutation?: HapMutation,
): Promise<BassRun> {
  const a = await assembledOf(fx, tx);
  const byDb = new Map<number, BassPresence>();
  for (const db of [...new Set([...SWEEP_DB, BASS_MISS_DB])]) {
    byDb.set(
      db,
      bassPresence(a.channels, SR, fx.sched.events, {
        missDb: db,
        offsetSec: a.bufferStartSec,
        barSec: fx.barSec,
      }),
    );
  }
  const targets: BassRun['targets'] = [];
  if (mutation?.kind === 'dropNotesBelowHz') {
    const win = Math.round(BASS_WINDOW_SEC * SR);
    const len = a.channels[0]!.length;
    for (const e of bassEvents(fx.sched.events)) {
      const bar = Math.floor(e.tSec / fx.barSec + 1e-6);
      const i0 = Math.round((e.tSec - a.bufferStartSec) * SR);
      if (i0 < 0 || i0 + win > len) continue;
      if (e.freq >= mutation.hz) continue;
      if (mutation.bars && !mutation.bars.includes(bar)) continue;
      targets.push({ bar, tSec: e.tSec });
    }
  }
  return { fixture: fx.fixture.name, barSec: fx.barSec, byDb, targets };
}

const specFor = (gate: GateName): MutationSpec => MUTATIONS.find((m) => m.gate === gate)!;

try {
  const seamSpec = specFor('seam');
  const stemsSpec = specFor('stems');
  const bassSpec = specFor('bass');
  const all = env.fixtures();
  const ctx = new Map<string, FixtureContext>();
  for (const f of all.filter((f) =>
    (['seam', 'stems', 'bass'] as const).some((g) => f.gates.includes(g)),
  )) {
    ctx.set(f.name, await env.prepare(f));
  }

  // One job per clean fixture (its gates in turn, so they share its memoized mix renders) and
  // one per mutation.
  type Job = { name: string; gates: GateName[]; spec?: MutationSpec };
  const jobs: Job[] = [];
  for (const f of all) {
    const gates = (['bass', 'seam', 'stems'] as GateName[]).filter((g) => f.gates.includes(g));
    if (gates.length > 0) jobs.push({ name: f.name, gates });
  }
  for (const spec of [seamSpec, stemsSpec, bassSpec]) {
    if (ctx.has(spec.fixture)) jobs.push({ name: spec.fixture, gates: [spec.gate], spec });
  }

  const seamClean: SeamUnit[] = [];
  const seamMut: SeamUnit[] = [];
  const stemsClean: Record<string, StemNoteScore[]> = {};
  let stemsMut: StemNoteScore[] | null = null;
  const bassClean: BassRun[] = [];
  let bassMut: BassRun | null = null;

  const failure = await pool(jobs, CONCURRENCY, async (job) => {
    const fx = ctx.get(job.name)!;
    const tx = job.spec?.apply;
    for (const gate of job.gates) {
      const t0 = performance.now();
      const tag = `${job.name} ${gate}${job.spec ? ` (${job.spec.name})` : ''}`;
      env.log(`start ${tag}`);
      if (gate === 'seam') {
        (job.spec ? seamMut : seamClean).push(...(await seamUnits(fx, tx)));
      } else if (gate === 'stems') {
        const notes = await stemNotes(fx, tx);
        if (job.spec) stemsMut = notes;
        else stemsClean[job.name] = notes;
      } else {
        const run = await bassRun(fx, tx, job.spec ? job.spec.apply(fx.base).mutation : undefined);
        if (job.spec) bassMut = run;
        else bassClean.push(run);
      }
      env.log(`done ${tag} ${((performance.now() - t0) / 1000).toFixed(1)}s`);
    }
  });
  if (failure !== null) throw failure;

  const evidence: Record<string, unknown> = { ...env.header, sweepDb: SWEEP_DB, seamT: SEAM_T };

  // ---- seam
  if (seamMut.length > 0) {
    const rows: SweepRow[] = SEAM_T.map((t) =>
      sweepRow(
        t,
        seamMut.filter((u) => seamFlagged(u.worstDeficitDb, u.twinFloorDb, t)).length,
        seamMut.length,
        seamClean.filter((u) => seamFlagged(u.worstDeficitDb, u.twinFloorDb, t)).length,
        seamClean.length,
      ),
    );
    evidence.seam = {
      mutation: seamSpec.name,
      fixture: seamSpec.fixture,
      clean: seamClean.sort((a, b) => a.fixture.localeCompare(b.fixture) || a.seam - b.seam),
      mutated: seamMut,
      rows,
      choice: chooseThreshold(rows),
      previous: SEAM_FAIL_DB,
    };
  }

  // ---- stems
  const mutStems = stemsMut as StemNoteScore[] | null;
  if (mutStems) {
    const m = stemsSpec.apply(ctx.get(stemsSpec.fixture)!.base).mutation;
    if (m?.kind !== 'dropOrbit') throw new Error('stems mutation is not dropOrbit');
    const control = stemsClean[stemsSpec.fixture] ?? [];
    const targetKeys = new Set(
      control
        .filter((n) => n.orbit === m.orbit && m.bars.includes(n.bar))
        .map((n) => `${n.orbit}:${n.bar}`),
    );
    const rows: SweepRow[] = SWEEP_DB.map((db) => {
      const mut = judgeStemScores(mutStems, db).misses;
      const caught = mut.filter((x) => targetKeys.has(`${x.orbit}:${x.bar}`)).length;
      const mutFalse = mut.length - caught;
      let fa = mutFalse;
      let units = 0;
      for (const notes of Object.values(stemsClean)) {
        const j = judgeStemScores(notes, db);
        fa += j.misses.length;
        units += j.tested;
      }
      return sweepRow(db, caught, targetKeys.size, fa, units);
    });
    evidence.stems = {
      mutation: stemsSpec.name,
      fixture: stemsSpec.fixture,
      cleanFixtures: Object.keys(stemsClean).sort(),
      targets: [...targetKeys],
      note: 'falseAlarms counts misses in clean renders plus misses outside the dropped orbit-bars in the mutated render',
      rows,
      perFixtureMisses: Object.fromEntries(
        SWEEP_DB.map((db) => [
          db,
          Object.fromEntries(
            Object.entries(stemsClean).map(([k, v]) => [k, judgeStemScores(v, db).misses]),
          ),
        ]),
      ),
      choice: chooseThreshold(rows),
      previous: STEMS_MISS_DB,
    };
  }

  // ---- bass
  const mutBass = bassMut as BassRun | null;
  if (mutBass) {
    const targetBars = new Set(mutBass.targets.map((t) => t.bar));
    const isTarget = (bar: number, tSec: number) =>
      mutBass.targets.some((t) => t.bar === bar && Math.abs(t.tSec - tSec) < 1e-6);
    const rows: SweepRow[] = SWEEP_DB.map((db) => {
      const mp = mutBass.byDb.get(db)!;
      const caught = mp.misses.filter((x) => isTarget(x.bar, x.tSec)).length;
      let fa = mp.misses.length - caught;
      let units = 0;
      for (const run of bassClean) {
        const p = run.byDb.get(db)!;
        fa += p.misses.length;
        units += p.checked;
      }
      return sweepRow(db, caught, mutBass.targets.length, fa, units);
    });
    const choice = chooseThreshold(rows);
    const missDb = choice?.db ?? BASS_MISS_DB;

    const axes: Record<RuleKey, number[]> = {
      clusterMin: [2, 3, 4],
      clusterSpanBars: range(2, 8, 1),
      clusterDeepDb: range(missDb, 18, 1),
      fractionMax: range(0.1, 0.3, 0.05),
    };
    const steps: Record<RuleKey, number> = {
      clusterMin: 1,
      clusterSpanBars: 1,
      clusterDeepDb: 1,
      fractionMax: 0.05,
    };
    const start = {
      clusterMin: BASS_CLUSTER_MIN,
      clusterSpanBars: BASS_CLUSTER_SPAN_BARS,
      clusterDeepDb: BASS_CLUSTER_DEEP_DB,
      fractionMax: BASS_FRACTION_MAX,
    };
    const grid = ruleGrid(axes);
    const judged = grid.map((rule) => {
      const clean = Object.fromEntries(
        bassClean.map((run) => [
          run.fixture,
          bassGate(run.byDb.get(missDb)!, run.barSec, rule).verdict,
        ]),
      );
      const mutated = bassGate(mutBass.byDb.get(missDb)!, mutBass.barSec, rule).verdict;
      const cleanFails = Object.entries(clean)
        .filter(([, v]) => v !== 'pass')
        .map(([k]) => k);
      return { rule, cleanFails, mutated, valid: cleanFails.length === 0 && mutated === 'fail' };
    });
    const valid = judged.filter((j) => j.valid).map((j) => j.rule);
    const chosen = closestRule(valid, start, steps);
    // When no rule is valid, the rule that catches the mutation with the fewest clean failures
    // (closest to start on ties) names the fixtures whose bass gate cannot hold.
    let fallback: { rule: (typeof grid)[number]; cleanFails: string[] } | null = null;
    if (!chosen) {
      const catching = judged.filter((j) => j.mutated === 'fail');
      const fewest = Math.min(...catching.map((j) => j.cleanFails.length));
      const best = closestRule(
        catching.filter((j) => j.cleanFails.length === fewest).map((j) => j.rule),
        start,
        steps,
      );
      if (best) {
        fallback = {
          rule: best,
          cleanFails: judged.find((j) => j.rule === best)!.cleanFails,
        };
      }
    }
    const rule = chosen ?? fallback?.rule ?? start;
    const detail = (run: BassRun, db: number) => {
      const p = run.byDb.get(db)!;
      const g = bassGate(p, run.barSec, rule);
      return {
        checked: p.checked,
        misses: p.misses,
        verdict: g.verdict,
        reasons: g.reasons,
        measurements: g.measurements,
      };
    };
    evidence.bass = {
      mutation: bassSpec.name,
      fixture: bassSpec.fixture,
      cleanFixtures: bassClean.map((r) => r.fixture).sort(),
      targetBars: [...targetBars].sort((a, b) => a - b),
      targets: mutBass.targets.length,
      note: 'falseAlarms counts misses in clean renders plus misses outside the dropped notes in the mutated render',
      rows,
      choice,
      previous: BASS_MISS_DB,
      rule: {
        axes,
        start,
        combinations: grid.length,
        validCount: valid.length,
        chosen,
        fallback,
        startJudged:
          judged.find(
            (j) =>
              j.rule.clusterMin === start.clusterMin &&
              j.rule.clusterSpanBars === start.clusterSpanBars &&
              j.rule.clusterDeepDb === start.clusterDeepDb &&
              Math.abs(j.rule.fractionMax - start.fractionMax) < 1e-9,
          ) ?? null,
        valid,
      },
      atChosen: {
        missDb,
        rule,
        clean: Object.fromEntries(bassClean.map((r) => [r.fixture, detail(r, missDb)])),
        mutated: detail(mutBass, missDb),
      },
    };
  }

  const path = writeEvidence('mutation-sweep', evidence);
  for (const k of ['seam', 'stems', 'bass']) {
    const e = evidence[k] as { rows: SweepRow[]; choice: unknown } | undefined;
    if (!e) continue;
    console.log(`${k}:`);
    for (const r of e.rows) console.log(`  ${JSON.stringify(r)}`);
    console.log(`  choice ${JSON.stringify(e.choice)}`);
  }
  const bass = evidence.bass as
    { rule: { chosen: unknown; fallback: unknown; validCount: number } } | undefined;
  if (bass)
    console.log(
      `bass rule: ${JSON.stringify(bass.rule.chosen)} fallback ${JSON.stringify(bass.rule.fallback)} valid ${bass.rule.validCount}`,
    );
  console.log(`wrote ${path}`);
} finally {
  await env.close();
}

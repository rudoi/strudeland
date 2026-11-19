import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  extractFeatures,
  parseFeatures,
  serializeFeatures,
  type Features,
} from '../src/lib/render/features';
import {
  compareFeatures,
  PROFILES,
  profileFor,
  type ParityProfile,
} from '../src/lib/render/parity';
import { bursts, mixResult, stereo } from './helpers/render-results';
import { concat, scale, silence, SR, sine } from './helpers/render-signals';

const SEC = 8;
const feat = (x: Float32Array) => extractFeatures(mixResult(stereo(x), { cps: 1 / SEC }));
const base = bursts(SEC);
const golden = feat(base);
const clone = (f: Features): Features => structuredClone(f);
/** A loose profile (the engine's former start values) for the tolerance checks below. */
const LOOSE: ParityProfile = {
  name: 'engine',
  frameTolerance: 441,
  maxOffsetSec: 0.05,
  peakDb: 3,
  lufsDb: 2,
  bandMeanDb: 3,
  bandP95Db: 6,
  hashMustMatch: false,
};

describe('compareFeatures', () => {
  test('identical features pass under both profiles', () => {
    for (const p of [PROFILES.self, LOOSE]) {
      const r = compareFeatures(golden, clone(golden), p);
      expect(r.verdict).toBe('pass');
      expect(r.hashEqual).toBe(true);
      expect(r.warnings).toEqual([]);
      expect(r.frameDelta).toBe(0);
      expect(r.offsetSec).toBe(0);
      expect(r.aligned).toBe(true);
      expect(r.peakDeltaDb).toBe(0);
      expect(r.lufsDeltaDb).toBe(0);
      for (const v of Object.values(r.bandMeanDeltaDb)) expect(v).toBe(0);
      for (const v of Object.values(r.bandP95DeltaDb)) expect(v).toBe(0);
    }
  });

  test('hashMustMatch short-circuits on equal hash', () => {
    const p: ParityProfile = { ...PROFILES.self, hashMustMatch: true };
    const r = compareFeatures(golden, clone(golden), p);
    expect(r.verdict).toBe('pass');
    expect(r.peakDeltaDb).toBe(0);
  });

  test('a one-sample change: advisory warning, or a hard fail with hashMustMatch', () => {
    const ch = stereo(base.slice());
    ch[0]![50] += 1e-7;
    const cand = extractFeatures(mixResult(ch, { cps: 1 / SEC }));
    const e = compareFeatures(golden, cand, LOOSE);
    expect(e.verdict).toBe('pass');
    expect(e.hashEqual).toBe(false);
    expect(e.warnings).toEqual(['pcm hash differs (advisory)']);
    expect(compareFeatures(golden, cand, PROFILES.self).verdict).toBe('pass');
    const strict = compareFeatures(golden, cand, { ...PROFILES.self, hashMustMatch: true });
    expect(strict.verdict).toBe('fail');
    expect(strict.reasons).toContain('pcm hash differs');
    expect(strict.warnings).toEqual([]);
  });

  test('+1 dB gain moves LUFS and peak, not the level-matched bands', () => {
    const cand = feat(scale(base, 10 ** (1 / 20)));
    const self = compareFeatures(golden, cand, PROFILES.self);
    expect(self.lufsDeltaDb!).toBeCloseTo(1, 1);
    expect(self.peakDeltaDb).toBeCloseTo(1, 1);
    for (const v of Object.values(self.bandMeanDeltaDb)) expect(Math.abs(v)).toBeLessThan(0.15);
    expect(self.verdict).toBe('fail');
    expect(self.reasons.some((r) => r.startsWith('LUFS delta'))).toBe(true);
    expect(compareFeatures(golden, cand, LOOSE).verdict).toBe('pass');
  });

  test('a delayed candidate fails the offset check under both profiles', () => {
    const shifted = concat(silence(0.3), base).subarray(0, base.length);
    const cand = feat(shifted);
    for (const p of [PROFILES.self, LOOSE]) {
      const r = compareFeatures(golden, cand, p);
      expect(r.offsetSec).toBeCloseTo(0.3, 2);
      expect(r.verdict).toBe('fail');
      expect(r.reasons.some((x) => x.startsWith('offset'))).toBe(true);
    }
  });

  test('a band 6 dB low fails only that band', () => {
    const cand = clone(golden);
    cand.bandsDb.mid = cand.bandsDb.mid.map((v) => v - 6);
    const r = compareFeatures(golden, cand, LOOSE);
    expect(r.verdict).toBe('fail');
    expect(r.bandMeanDeltaDb.mid!).toBeCloseTo(-6, 1);
    expect(r.reasons.filter((x) => x.startsWith('band mid mean'))).toHaveLength(1);
    expect(r.reasons.some((x) => x.startsWith('band low'))).toBe(false);
    expect(r.reasons.some((x) => x.startsWith('band high'))).toBe(false);
  });

  test('band p95 breach is reported', () => {
    const cand = clone(golden);
    // 10 percent of windows 12 dB low: mean delta small, p95 large.
    cand.bandsDb.high = cand.bandsDb.high.map((v, i) => (i % 10 === 0 ? v - 12 : v));
    const p: ParityProfile = { ...LOOSE, bandMeanDb: 100 };
    const r = compareFeatures(golden, cand, p);
    expect(r.verdict).toBe('fail');
    expect(r.reasons.some((x) => x.startsWith('band high p95'))).toBe(true);
  });

  test('frame drift of 441 passes a loose profile and fails self', () => {
    const cand = { ...clone(golden), frames: golden.frames + 441 };
    expect(compareFeatures(golden, cand, LOOSE).verdict).toBe('pass');
    const r = compareFeatures(golden, cand, PROFILES.self);
    expect(r.verdict).toBe('fail');
    expect(r.frameDelta).toBe(441);
    expect(r.reasons[0]).toContain('frames differ by 441');
  });

  test('negative frame drift beyond tolerance fails', () => {
    const cand = { ...clone(golden), frames: golden.frames - 442 };
    expect(compareFeatures(golden, cand, LOOSE).verdict).toBe('fail');
  });

  test('peak breach is reported', () => {
    const cand = { ...clone(golden), peakDb: golden.peakDb + 5 };
    const r = compareFeatures(golden, cand, LOOSE);
    expect(r.verdict).toBe('fail');
    expect(r.reasons.some((x) => x.startsWith('peak delta'))).toBe(true);
  });

  test('different sample rate or schema is unverifiable', () => {
    const sr = compareFeatures(golden, { ...clone(golden), sampleRate: 48000 }, LOOSE);
    expect(sr.verdict).toBe('unverifiable');
    expect(sr.reasons[0]).toContain('sample rate');
    const schema = compareFeatures(golden, { ...clone(golden), schema: 2 as 1 }, LOOSE);
    expect(schema.verdict).toBe('unverifiable');
  });

  test('0.5 s windows are unverifiable', () => {
    const short = extractFeatures(mixResult(stereo(bursts(0.5)), { cps: 2 }));
    const r = compareFeatures(short, clone(short), LOOSE);
    expect(r.verdict).toBe('unverifiable');
    expect(r.reasons.some((x) => x.includes('common band windows'))).toBe(true);
  });

  test('a constant tone has no onsets: not aligned, bands decide', () => {
    const tone = feat(sine(1000, 0.2, SEC));
    const r = compareFeatures(tone, clone(tone), LOOSE);
    expect(r.aligned).toBe(false);
    expect(r.verdict).toBe('pass');
    expect(r.reasons.some((x) => x.startsWith('not aligned'))).toBe(true);
    const louder = feat(sine(1000, 0.4, SEC));
    expect(compareFeatures(tone, louder, PROFILES.self).verdict).toBe('fail');
  });

  test('silence against silence passes with null LUFS', () => {
    const s = feat(silence(SEC));
    const r = compareFeatures(s, clone(s), PROFILES.self);
    expect(r.verdict).toBe('pass');
    expect(r.lufsDeltaDb).toBeNull();
    expect(r.reasons.some((x) => x.includes('LUFS check skipped'))).toBe(true);
    expect(r.reasons.some((x) => x.includes('skipped: golden mean'))).toBe(true);
  });

  test('golden round trip through serialize and parse still passes', () => {
    const g = parseFeatures(serializeFeatures(golden));
    for (const p of [PROFILES.self, LOOSE]) {
      expect(compareFeatures(g, golden, p).verdict).toBe('pass');
    }
  });
});

describe('profiles', () => {
  test('profileFor picks self for superdough and engine for rust', () => {
    expect(profileFor('superdough')).toBe(PROFILES.self);
    expect(profileFor('rust')).toBe(PROFILES.engine);
  });

  const ROOT = resolve(import.meta.dir, '..');
  const ENGINE_EVIDENCE = 'plans/0010-rust-engine-core/evidence/engine-null-final.json';

  test('the engine profile equals the calibrated decision', () => {
    const evidence = JSON.parse(readFileSync(join(ROOT, ENGINE_EVIDENCE), 'utf8'));
    expect(PROFILES.engine).toEqual(evidence.decision.engine);
    expect(PROFILES.engine).not.toBe(PROFILES.self);
  });

  test('every calibrated engine field cites an existing evidence file', () => {
    const source = readFileSync(join(ROOT, 'src/lib/render/parity.ts'), 'utf8');
    const block = source.slice(source.indexOf('  engine: {'));
    const fields = Object.keys(PROFILES.engine).filter(
      (k) => k !== 'name' && k !== 'hashMustMatch',
    );
    expect(fields.length).toBe(6);
    for (const field of fields) {
      const cite = new RegExp(
        `// calibrated: see (plans/\\S+\\.json) decision\\.engine\\.${field}\\b`,
      );
      const m = block.match(cite);
      expect(m, field).not.toBeNull();
      expect(existsSync(join(ROOT, m![1]!)), m![1]).toBe(true);
    }
  });
});

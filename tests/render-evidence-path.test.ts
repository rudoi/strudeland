import { describe, expect, test } from 'bun:test';
import { DEFAULT_EVIDENCE_DIR, evidencePath } from '../scripts/render/probes/evidence-path';

const ROOT = '/repo';

describe('evidencePath', () => {
  test('defaults to the render harness evidence directory', () => {
    expect(evidencePath(ROOT, 'twin-null', {})).toEqual({
      dir: `/repo/${DEFAULT_EVIDENCE_DIR}`,
      path: `/repo/${DEFAULT_EVIDENCE_DIR}/twin-null.json`,
    });
  });
  test('an empty override keeps the default', () => {
    expect(evidencePath(ROOT, 'x', { PROBE_EVIDENCE_DIR: '' }).dir).toBe(
      `/repo/${DEFAULT_EVIDENCE_DIR}`,
    );
  });
  test('a relative directory resolves against the root', () => {
    expect(evidencePath(ROOT, 'x', { PROBE_EVIDENCE_DIR: 'plans/p/evidence' }).path).toBe(
      '/repo/plans/p/evidence/x.json',
    );
  });
  test('an absolute directory is used as is', () => {
    expect(evidencePath(ROOT, 'x', { PROBE_EVIDENCE_DIR: '/tmp/ev' }).path).toBe('/tmp/ev/x.json');
  });
  test('a label suffixes the file name', () => {
    expect(evidencePath(ROOT, 'twin-null', { PROBE_LABEL: 'pool-off' }).path).toBe(
      `/repo/${DEFAULT_EVIDENCE_DIR}/twin-null-pool-off.json`,
    );
  });
  test("a probe's own default applies unless PROBE_EVIDENCE_DIR is set", () => {
    const own = 'plans/0010-rust-engine-core/evidence';
    expect(evidencePath(ROOT, 'engine-null', { PROBE_LABEL: 'final' }, own).path).toBe(
      `/repo/${own}/engine-null-final.json`,
    );
    expect(evidencePath(ROOT, 'x', { PROBE_EVIDENCE_DIR: '/tmp/ev' }, own).path).toBe(
      '/tmp/ev/x.json',
    );
  });
  test('a label with path characters is refused', () => {
    expect(() => evidencePath(ROOT, 'x', { PROBE_LABEL: '../up' })).toThrow('PROBE_LABEL');
    expect(() => evidencePath(ROOT, 'x', { PROBE_LABEL: 'a b' })).toThrow('PROBE_LABEL');
  });
});

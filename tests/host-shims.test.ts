import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { test } from 'bun:test';

const childFlag = 'STRUDELAND_TEST_HOST_SHIMS_CHILD';

if (process.env[childFlag] === '1') {
  await import('./helpers/host-shim-cases');
} else {
  test('host shims match the real Strudel methods in a fresh process', () => {
    // Other suites import the shims and the real methods in the same Bun process.
    // Import both from scratch to get the real prototype before any shim installs.
    const result = spawnSync(process.execPath, ['test', import.meta.path], {
      cwd: resolve(import.meta.dir, '..'),
      env: { ...process.env, [childFlag]: '1' },
      encoding: 'utf8',
      timeout: 10_000,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(result.stdout + result.stderr);
  }, 12_000);
}

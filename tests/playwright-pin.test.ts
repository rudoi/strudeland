import { describe, expect, test } from 'bun:test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { readPlaywrightPin } from '../scripts/playwright-pin';

const withPackage = (pkg: unknown, f: (root: string) => void) => {
  const root = mkdtempSync(join(tmpdir(), 'pin-'));
  try {
    writeFileSync(join(root, 'package.json'), JSON.stringify(pkg));
    f(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
};

describe('readPlaywrightPin', () => {
  test('the exact devDependency, or null without one', () => {
    withPackage({ devDependencies: { playwright: '1.2.3' } }, (root) =>
      expect(readPlaywrightPin(root)).toBe('1.2.3'),
    );
    withPackage({ devDependencies: {} }, (root) => expect(readPlaywrightPin(root)).toBeNull());
    withPackage({}, (root) => expect(readPlaywrightPin(root)).toBeNull());
  });

  test('the repo has a pin', () => {
    expect(readPlaywrightPin(join(import.meta.dir, '..'))).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

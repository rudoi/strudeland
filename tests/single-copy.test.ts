import { fileURLToPath } from 'node:url';
import { readFileSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from 'bun:test';

const root = fileURLToPath(new URL('../', import.meta.url));

/** Every package.json under node_modules (the glob does not follow symlinks, so a workspace link
 * itself never shows up), grouped by package name; scanned once. */
const installed = new Map<string, string[]>();
for (const p of new Bun.Glob('node_modules/**/package.json').scanSync({ cwd: root })) {
  try {
    const { name } = JSON.parse(readFileSync(join(root, p), 'utf8'));
    const packagePath = p.replaceAll('\\', '/');
    if (typeof name === 'string' && packagePath.endsWith(`/${name}/package.json`))
      installed.set(name, [...(installed.get(name) ?? []), packagePath]);
  } catch {
    // Not every package.json in node_modules is valid JSON (test fixtures); skip those.
  }
}
const installedCopies = (name: string) => installed.get(name) ?? [];

// A second copy of any of these breaks instanceof checks, StateField identity, Pattern.prototype or
// the Vim instance vim-mode.ts shares with Strudel's keybindings.
for (const name of ['@codemirror/state', '@codemirror/view', '@replit/codemirror-vim']) {
  test(`exactly one installed copy of ${name}`, () => {
    expect(installedCopies(name)).toEqual([`node_modules/${name}/package.json`]);
  });
}

const vendored = { '@strudel/core': 'core', superdough: 'superdough' };

for (const [name, dir] of Object.entries(vendored)) {
  test(`${name} resolves to the vendored package and has no installed copy`, () => {
    expect(realpathSync(join(root, 'node_modules', name))).toBe(
      realpathSync(join(root, 'vendor/strudel/packages', dir)),
    );
    expect(installedCopies(name)).toEqual([]);
  });
}

test('the copy check sees a real nested copy but not a symlink', () => {
  // The vendored links are found by neither glob, while a real installed package is found once.
  expect(installedCopies('@strudel/codemirror')).toEqual([]);
  expect(installedCopies('vite')).toEqual(['node_modules/vite/package.json']);
});

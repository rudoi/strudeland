import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/** The exact `playwright` devDependency in `<root>/package.json`, or null when there is none.
 *  Callers decide what a missing pin means. */
export function readPlaywrightPin(root: string): string | null {
  const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const pin = pkg.devDependencies?.playwright;
  return typeof pin === 'string' ? pin : null;
}

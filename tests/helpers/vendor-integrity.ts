// Integrity check of vendor/strudel/: every vendored file matches upstream-manifest.json unless a
// PATCHES.md entry lists it, and nothing else lives under packages/.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parsePatches } from '../../scripts/strudel-patches';
import { sha256 } from '../../scripts/sha256';

/** Files under `dir`, as POSIX paths relative to `dir`, sorted. Directories named in `skip` are not
 * entered. */
export function walkFiles(dir: string, { skip = [] as string[] } = {}): string[] {
  const out: string[] = [];
  const walk = (rel: string) => {
    for (const e of readdirSync(rel === '' ? dir : join(dir, rel), { withFileTypes: true })) {
      const child = rel === '' ? e.name : `${rel}/${e.name}`;
      if (!e.isDirectory()) out.push(child);
      else if (!skip.includes(e.name)) walk(child);
    }
  };
  walk('');
  return out.sort();
}

/** Files under `<vendorDir>/packages/`, as POSIX paths relative to `vendorDir`, sorted. Directories
 * named `node_modules` are skipped; `dist/`, `test/` and `bench/` are not. */
export function walkVendorPackages(vendorDir: string): string[] {
  const packages = join(vendorDir, 'packages');
  if (!existsSync(packages)) return [];
  return walkFiles(packages, { skip: ['node_modules'] }).map((f) => `packages/${f}`);
}

export type VendorReport = {
  /** In the manifest but not on disk. */
  missing: string[];
  /** Differs from the manifest and no PATCHES.md entry lists it. */
  modifiedUnlisted: string[];
  /** Listed in PATCHES.md and in the manifest, but identical to upstream (`<id>: <path>`). */
  staleListed: string[];
  /** Listed in PATCHES.md, not in the manifest, and not on disk (`<id>: <path>`). */
  listedMissing: string[];
  /** Under packages/, in neither the manifest nor PATCHES.md. */
  unlisted: string[];
};

const fileHash = (p: string) => sha256(readFileSync(p));

export function checkVendor(vendorDir: string): VendorReport {
  const manifest = JSON.parse(readFileSync(join(vendorDir, 'upstream-manifest.json'), 'utf8')) as {
    files: Record<string, string>;
  };
  const patchesPath = join(vendorDir, 'PATCHES.md');
  const patches = existsSync(patchesPath) ? parsePatches(readFileSync(patchesPath, 'utf8')) : [];
  const listed = new Set(patches.flatMap((p) => p.files));
  const report: VendorReport = {
    missing: [],
    modifiedUnlisted: [],
    staleListed: [],
    listedMissing: [],
    unlisted: [],
  };
  for (const [path, hash] of Object.entries(manifest.files)) {
    const abs = join(vendorDir, path);
    if (!existsSync(abs)) report.missing.push(path);
    else if (fileHash(abs) !== hash && !listed.has(path)) report.modifiedUnlisted.push(path);
  }
  for (const { id, files } of patches)
    for (const path of files) {
      const abs = join(vendorDir, path);
      const hash = manifest.files[path];
      if (hash !== undefined) {
        if (existsSync(abs) && fileHash(abs) === hash) report.staleListed.push(`${id}: ${path}`);
      } else if (!existsSync(abs)) report.listedMissing.push(`${id}: ${path}`);
    }
  for (const path of walkVendorPackages(vendorDir))
    if (!(path in manifest.files) && !listed.has(path)) report.unlisted.push(path);
  return report;
}

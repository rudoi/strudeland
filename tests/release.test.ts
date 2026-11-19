import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  bumpReleaseFiles,
  nextReleaseVersion,
  readReleaseVersion,
  releaseOptions,
  versionFiles,
  type VersionFile,
} from '../scripts/release-lib';

const versionSources = Object.fromEntries(
  versionFiles.map((file) => [file, readFileSync(new URL(`../${file}`, import.meta.url), 'utf8')]),
) as Record<VersionFile, string>;

describe('release version handling', () => {
  test('selects one operation and supports a dry run', () => {
    expect(releaseOptions(['--patch', '--dry-run'])).toEqual({ change: 'patch', dryRun: true });
    expect(releaseOptions(['--current'])).toEqual({ change: 'current', dryRun: false });
    expect(releaseOptions(['--set', '1.2.3'])).toEqual({ change: { set: '1.2.3' }, dryRun: false });
    expect(() => releaseOptions(['--minor', '--major'])).toThrow('exactly one');
    expect(() => releaseOptions(['--set', '1.0.0-beta'])).toThrow('Invalid release version');
  });

  test('rejects unchanged, older, and MSI-incompatible versions', () => {
    expect(nextReleaseVersion('1.2.3', 'minor')).toBe('1.3.0');
    expect(nextReleaseVersion('1.2.3', 'major')).toBe('2.0.0');
    expect(() => nextReleaseVersion('1.2.3', { set: '1.2.3' })).toThrow('greater');
    expect(() => nextReleaseVersion('1.2.3', { set: '1.1.9' })).toThrow('greater');
    expect(() => nextReleaseVersion('1.2.65535', 'patch')).toThrow('MSI version limits');
  });

  test('bumps the app, CLI and lockfile while preserving other crates', () => {
    const current = readReleaseVersion(versionSources);
    const next = nextReleaseVersion(current, 'patch');
    const result = bumpReleaseFiles(versionSources, next);
    expect(readReleaseVersion(result)).toBe(next);
    for (const file of versionFiles) expect(result[file]).not.toBe(versionSources[file]);
    const crateVersions = (source: string) =>
      Array.from(
        source.matchAll(/^name = "strudeland-(?!app"|cli").*"\nversion = "[^"]+"$/gm),
        ([row]) => row,
      );
    expect(crateVersions(result['Cargo.lock'])).toEqual(
      crateVersions(versionSources['Cargo.lock']),
    );
  });

  test('rejects inconsistent versions before planning a bump', () => {
    const source = { ...versionSources, 'src-tauri/Cargo.toml': '[package]\nversion = "0.0.0"\n' };
    expect(() => readReleaseVersion(source)).toThrow('versions differ');
  });
});

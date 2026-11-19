const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export const versionFiles = [
  'package.json',
  'src-tauri/tauri.conf.json',
  'src-tauri/Cargo.toml',
  'crates/strudeland-cli/Cargo.toml',
  'Cargo.lock',
] as const;

export type VersionFile = (typeof versionFiles)[number];
type VersionFiles = Record<VersionFile, string>;
type Change = 'current' | 'patch' | 'minor' | 'major' | { set: string };

function parts(version: string): number[] {
  if (!versionPattern.test(version)) throw new Error(`Invalid release version ${version}.`);
  const values = version.split('.').map(Number);
  if (values.some((part, i) => part > [255, 255, 65535][i])) {
    throw new Error(`Release version ${version} exceeds the MSI version limits.`);
  }
  return values;
}

export function releaseOptions(args: string[]): { change: Change; dryRun: boolean } {
  const dryRun = args.includes('--dry-run');
  const options = args.filter((arg) => arg !== '--dry-run');
  if (options.length === 2 && options[0] === '--set') {
    parts(options[1]);
    return { change: { set: options[1] }, dryRun };
  }
  if (options.length === 1) {
    switch (options[0]) {
      case '--current':
        return { change: 'current', dryRun };
      case '--patch':
        return { change: 'patch', dryRun };
      case '--minor':
        return { change: 'minor', dryRun };
      case '--major':
        return { change: 'major', dryRun };
    }
  }
  throw new Error('Choose exactly one of --current, --patch, --minor, --major or --set VERSION.');
}

export function nextReleaseVersion(current: string, change: Change): string {
  const [major, minor, patch] = parts(current);
  if (change === 'current') return current;
  const next =
    typeof change === 'object'
      ? change.set
      : {
          major: `${major + 1}.0.0`,
          minor: `${major}.${minor + 1}.0`,
          patch: `${major}.${minor}.${patch + 1}`,
        }[change];
  const nextParts = parts(next);
  const index = nextParts.findIndex((part, i) => part !== [major, minor, patch][i]);
  if (index < 0 || nextParts[index] < [major, minor, patch][index]) {
    throw new Error(`Release version must be greater than ${current}.`);
  }
  return next;
}

function lockVersionPattern(name: string): RegExp {
  return new RegExp(`(\\[\\[package\\]\\]\\nname = "${name}"\\nversion = ")([^"]+)(")`);
}

export function readReleaseVersion(files: VersionFiles): string {
  const version: unknown = JSON.parse(files['package.json']).version;
  if (typeof version !== 'string') throw new Error('package.json has no version.');
  parts(version);
  const tauriVersion: unknown = JSON.parse(files['src-tauri/tauri.conf.json']).version;
  if (tauriVersion !== version) throw new Error('The Tauri and frontend versions differ.');
  for (const [file, name] of [
    ['src-tauri/Cargo.toml', 'strudeland-app'],
    ['crates/strudeland-cli/Cargo.toml', 'strudeland-cli'],
  ] as const) {
    const manifestVersion = files[file].match(/^version = "([^"]+)"$/m)?.[1];
    const lockVersion = files['Cargo.lock'].match(lockVersionPattern(name))?.[2];
    if (manifestVersion !== version || lockVersion !== version) {
      throw new Error(`The frontend, ${file}, and Cargo.lock versions differ.`);
    }
  }
  return version;
}

export function bumpReleaseFiles(files: VersionFiles, next: string): VersionFiles {
  const current = readReleaseVersion(files);
  nextReleaseVersion(current, { set: next });
  const changed = { ...files };
  for (const name of ['package.json', 'src-tauri/tauri.conf.json'] as const) {
    changed[name] = files[name].replace(
      /^([ \t]*"version"[ \t]*:[ \t]*")[^"]+(")/m,
      (_, prefix, suffix) => `${prefix}${next}${suffix}`,
    );
  }
  for (const name of ['src-tauri/Cargo.toml', 'crates/strudeland-cli/Cargo.toml'] as const) {
    changed[name] = files[name].replace(/^version = "[^"]+"$/m, `version = "${next}"`);
  }
  for (const name of ['strudeland-app', 'strudeland-cli']) {
    changed['Cargo.lock'] = changed['Cargo.lock'].replace(
      lockVersionPattern(name),
      (_, prefix, _version, suffix) => `${prefix}${next}${suffix}`,
    );
  }
  if (readReleaseVersion(changed) !== next) throw new Error('The version bump was incomplete.');
  return changed;
}

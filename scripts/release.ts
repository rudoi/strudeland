import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  bumpReleaseFiles,
  nextReleaseVersion,
  readReleaseVersion,
  releaseOptions,
  versionFiles,
  type VersionFile,
} from './release-lib';

const repository = 'rudoi/strudeland';
const root = fileURLToPath(new URL('..', import.meta.url));
const usage = `Usage: bun run release (--current | --patch | --minor | --major | --set VERSION) [--dry-run]

Version changes open an auto-merge PR against public main. After it merges and passes CI, --current tags
the checked main revision and dispatches the installer build. CI prepares a GitHub release draft.`;

function run(command: string, args: string[], input?: string): string {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', input });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `${command} ${args[0]} failed.`);
  return result.stdout.trim();
}

function isPublicRemote(url: string): boolean {
  return /^(?:https:\/\/github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)rudoi\/strudeland(?:\.git)?$/.test(
    url,
  );
}

function remoteName(): string {
  const remote = run('git', ['remote'])
    .split(/\r?\n/)
    .find(
      (name) =>
        name &&
        isPublicRemote(run('git', ['remote', 'get-url', name])) &&
        isPublicRemote(run('git', ['remote', 'get-url', '--push', name])),
    );
  if (!remote) throw new Error('Configure a Git remote for github.com/rudoi/strudeland first.');
  return remote;
}

function publicRef(remote: string, ref: string): string | undefined {
  const refs = run('git', ['ls-remote', remote, ref, `${ref}^{}`])
    .split(/\r?\n/)
    .map((line) => line.split('\t'));
  return (
    refs.find(([, name]) => name === `${ref}^{}`)?.[0] ?? refs.find(([, name]) => name === ref)?.[0]
  );
}

function readVersionFiles(commit: string): Record<VersionFile, string> {
  return Object.fromEntries(
    versionFiles.map((file) => [file, run('git', ['show', `${commit}:${file}`]) + '\n']),
  ) as Record<VersionFile, string>;
}

function verifyMainChecks(commit: string): void {
  for (const workflow of ['macos.yml', 'native.yml']) {
    const result = run('gh', [
      'run',
      'list',
      '--repo',
      repository,
      '--workflow',
      workflow,
      '--branch',
      'main',
      '--event',
      'push',
      '--commit',
      commit,
      '--limit',
      '1',
      '--json',
      'conclusion',
      '--jq',
      '.[0].conclusion',
    ]);
    if (result !== 'success')
      throw new Error(`Waiting for a successful ${workflow} run for ${commit} on main.`);
  }
}

function openBump(remote: string, commit: string, version: string, dryRun: boolean): void {
  const branch = `release/v${version}`;
  const files = bumpReleaseFiles(readVersionFiles(commit), version);
  if (
    run('git', ['rev-parse', 'HEAD']) !== commit ||
    run('git', ['status', '--porcelain', '-uno'])
  ) {
    throw new Error('For a version bump, start at public main with clean tracked files.');
  }
  if (publicRef(remote, `refs/heads/${branch}`) || publicRef(remote, `refs/tags/v${version}`)) {
    throw new Error(`A branch or tag already exists for ${version}.`);
  }
  console.log(
    `${dryRun ? 'Would open' : 'Opening'} a version bump PR for ${version} from ${commit}.`,
  );
  if (dryRun) return;
  run('git', ['switch', '-c', branch, commit]);
  for (const file of versionFiles)
    writeFileSync(new URL(`../${file}`, import.meta.url), files[file]);
  run('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1']);
  run('git', ['add', '--', ...versionFiles]);
  const title = `chore(release): v${version}`;
  run('git', ['commit', '-m', title]);
  run('git', ['push', '--set-upstream', remote, branch]);
  const pullRequest = run(
    'gh',
    [
      'pr',
      'create',
      '--repo',
      repository,
      '--head',
      branch,
      '--base',
      'main',
      '--title',
      title,
      '--body-file',
      '-',
    ],
    `Bump Strudeland to ${version}. After merging and checking main, run \`bun run release --current\` to prepare its release draft.\n`,
  );
  console.log(pullRequest);
  run('gh', [
    'pr',
    'merge',
    pullRequest,
    '--repo',
    repository,
    '--squash',
    '--auto',
    '--match-head-commit',
    run('git', ['rev-parse', 'HEAD']),
  ]);
  console.log('Enabled auto-merge for the version bump.');
}

function buildRelease(remote: string, commit: string, version: string, dryRun: boolean): void {
  const workflow = spawnSync('git', ['cat-file', '-e', `${commit}:.github/workflows/release.yml`], {
    cwd: root,
  });
  if (workflow.status !== 0)
    throw new Error('The release workflow must land on public main first.');
  verifyMainChecks(commit);
  const tag = `v${version}`;
  const ref = `refs/tags/${tag}`;
  const remoteTag = publicRef(remote, ref);
  if (remoteTag && remoteTag !== commit)
    throw new Error(`${tag} already points to a different commit.`);
  console.log(
    `${dryRun ? 'Would prepare' : 'Preparing'} a draft for ${tag} from public main at ${commit}.`,
  );
  if (dryRun) return;
  if (!remoteTag) {
    const localTag = spawnSync('git', ['rev-parse', '--verify', '--quiet', `${ref}^{commit}`], {
      cwd: root,
      encoding: 'utf8',
    });
    if (localTag.status === 1)
      run('git', ['tag', '-a', tag, '-m', `Strudeland ${version}`, commit]);
    else if (localTag.status !== 0 || localTag.stdout.trim() !== commit) {
      throw new Error(`The local tag ${tag} is unavailable or points to a different commit.`);
    }
    run('git', ['push', remote, `${ref}:${ref}`]);
  }
  console.log(
    run('gh', [
      'workflow',
      'run',
      'release.yml',
      '--repo',
      repository,
      '--ref',
      'main',
      '-f',
      `tag=${tag}`,
    ]),
  );
}

try {
  const args = process.argv.slice(2);
  if (args.length === 0 || args.includes('--help')) {
    console.log(usage);
  } else {
    const { change, dryRun } = releaseOptions(args);
    const remote = remoteName();
    const commit = publicRef(remote, 'refs/heads/main');
    if (!commit) throw new Error('Could not resolve public main.');
    const available = spawnSync('git', ['cat-file', '-e', `${commit}^{commit}`], { cwd: root });
    if (available.status !== 0) {
      if (dryRun) throw new Error(`Fetch main from ${remote} before checking this release.`);
      run('git', ['fetch', '--no-tags', remote, 'main']);
    }
    const version = nextReleaseVersion(readReleaseVersion(readVersionFiles(commit)), change);
    if (change === 'current') buildRelease(remote, commit, version, dryRun);
    else openBump(remote, commit, version, dryRun);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}

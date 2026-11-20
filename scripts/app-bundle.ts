import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

const root = resolve(import.meta.dir, '..');
const macTarget = 'aarch64-apple-darwin';
const windowsTarget = 'x86_64-pc-windows-msvc';

export function bundleOptions(args: string[]) {
  const [command, ...flags] = args;
  const known = ['--debug', '--adhoc', '--bundle-only'];
  if (
    !['app', 'dmg', 'msi', 'prepare'].includes(command) ||
    flags.some((flag) => !known.includes(flag)) ||
    new Set(flags).size !== flags.length ||
    (command === 'msi' && flags.length > 0) ||
    (command === 'prepare' && flags.length > 0) ||
    (flags.includes('--bundle-only') && (command !== 'dmg' || flags.length !== 1))
  ) {
    throw new Error(
      'usage: app:bundle app|dmg [--debug] [--adhoc], msi, prepare, or dmg --bundle-only',
    );
  }
  return {
    command,
    target: command === 'msi' ? windowsTarget : macTarget,
    debug: flags.includes('--debug'),
    adhoc: flags.includes('--adhoc'),
    bundleOnly: flags.includes('--bundle-only'),
  };
}

export function cliExecutable(messages: string): string {
  const artifacts = messages
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .filter(
      (msg) =>
        msg.reason === 'compiler-artifact' &&
        msg.target.name === 'strudeland' &&
        msg.target.kind.includes('bin') &&
        typeof msg.executable === 'string',
    );
  if (artifacts.length !== 1) throw new Error('Cargo did not report one strudeland CLI executable');
  return artifacts[0].executable;
}

async function cargoCli(debug: boolean, isWindows: boolean): Promise<string> {
  const proc = Bun.spawn(
    [
      'cargo',
      'build',
      '--locked',
      '-p',
      'strudeland-cli',
      '--bin',
      'strudeland',
      '--message-format=json-render-diagnostics',
      ...(isWindows ? [] : ['--features', 'strudeland-render/host-jitless']),
      ...(debug ? [] : ['--release']),
    ],
    { cwd: root, stdout: 'pipe', stderr: 'inherit' },
  );
  const [messages, status] = await Promise.all([new Response(proc.stdout).text(), proc.exited]);
  if (status !== 0) throw new Error(`CLI build failed with exit code ${status}`);
  return cliExecutable(messages);
}

async function main(args: string[]): Promise<number> {
  const options = bundleOptions(args);
  const host = Bun.spawnSync(['rustc', '--print', 'host-tuple'], { stderr: 'inherit' });
  if (host.exitCode !== 0 || host.stdout.toString().trim() !== options.target) {
    throw new Error(`${options.command} requires a native ${options.target} Rust toolchain`);
  }

  const isWindows = options.command === 'msi';
  const profile = options.debug ? 'debug' : 'release';
  const sidecar = `binaries/${profile}/strudeland-cli`;
  const env = { ...process.env };
  if (!options.bundleOnly) {
    const executable = await cargoCli(options.debug, isWindows);
    if (isWindows) {
      env.STRUDELAND_CLI_MSI_SOURCE = executable;
    } else {
      const dest = join(root, 'src-tauri', `${sidecar}-${options.target}`);
      await mkdir(dirname(dest), { recursive: true });
      await copyFile(executable, dest);
    }
  }

  if (options.command === 'prepare') return 0;
  const config = isWindows
    ? {
        bundle: {
          windows: {
            wix: {
              fragmentPaths: ['windows/cli.wxs'],
              componentRefs: ['StrudelandCli'],
            },
          },
        },
      }
    : {
        bundle: {
          externalBin: [sidecar],
          ...(options.adhoc ? { macOS: { signingIdentity: '-' } } : {}),
        },
      };

  return Bun.spawn(
    [
      'bun',
      'run',
      'tauri',
      options.bundleOnly ? 'bundle' : 'build',
      '--ci',
      '--bundles',
      options.command,
      '--config',
      JSON.stringify(config),
      ...(options.debug ? ['--debug'] : []),
      ...(isWindows ? ['--no-sign'] : []),
      ...(options.bundleOnly ? [] : ['--', '--locked']),
    ],
    { cwd: root, env, stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' },
  ).exited;
}

if (import.meta.main) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

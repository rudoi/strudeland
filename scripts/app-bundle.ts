import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

const root = resolve(import.meta.dir, '..');
const macTarget = 'aarch64-apple-darwin';
const windowsTarget = 'x86_64-pc-windows-msvc';
// Tauri's WiX runner only forwards TAURI-prefixed application environment variables.
export const MSI_SOURCE_ENV = 'TAURI_STRUDELAND_CLI_MSI_SOURCE';

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

export async function matchesTarget(executable: Blob, target: string): Promise<boolean> {
  const header = new DataView(await executable.slice(0, 64).arrayBuffer());
  if (target === macTarget) {
    // Thin little-endian 64-bit Mach-O, CPU_TYPE_ARM64.
    return (
      header.byteLength >= 8 &&
      header.getUint32(0, true) === 0xfeedfacf &&
      header.getUint32(4, true) === 0x0100000c
    );
  }
  if (target !== windowsTarget || header.byteLength < 64 || header.getUint16(0, true) !== 0x5a4d) {
    return false;
  }
  // The DOS header points to the PE signature and machine field.
  const offset = header.getUint32(60, true);
  const pe = new DataView(await executable.slice(offset, offset + 6).arrayBuffer());
  return (
    pe.byteLength === 6 && pe.getUint32(0, true) === 0x4550 && pe.getUint16(4, true) === 0x8664
  );
}

async function verifyTarget(executable: string, target: string) {
  if (!(await matchesTarget(Bun.file(executable), target))) {
    throw new Error(`${executable} is not an executable for ${target}`);
  }
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
  const macSidecar = join(root, 'src-tauri', `${sidecar}-${options.target}`);
  const env = { ...process.env };
  if (options.bundleOnly) {
    await verifyTarget(macSidecar, options.target);
  } else {
    const executable = await cargoCli(options.debug, isWindows);
    await verifyTarget(executable, options.target);
    if (isWindows) {
      env[MSI_SOURCE_ENV] = executable;
    } else {
      await mkdir(dirname(macSidecar), { recursive: true });
      await copyFile(executable, macSidecar);
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

// `bun run app:signed`: build the debug app bundle signed with a stable code-signing identity and
// open it. macOS keys privacy grants (the microphone) to the app's signature; the ad-hoc signature
// of a plain debug build changes on every build, so the grant would be asked for again each time.
//
// The identity is `APPLE_SIGNING_IDENTITY` (Tauri's own variable) when set, else the first
// "Apple Development" identity in the login keychain. Nothing personal is committed.

export const BUNDLE = 'target/debug/bundle/macos/strudeland.app';

export interface Options {
  open: boolean;
}

/** Parses the script's arguments: `--no-open` builds without launching. */
export function parseArgs(argv: string[]): Options | { error: string } {
  let open = true;
  for (const arg of argv) {
    if (arg === '--no-open') open = false;
    else return { error: `unknown argument ${arg}; usage: bun run app:signed [--no-open]` };
  }
  return { open };
}

/**
 * The identity to sign with: `fromEnv` when it is set, else the first "Apple Development"
 * identity in `security find-identity -v -p codesigning` output.
 */
export function pickIdentity(
  findIdentityOutput: string,
  fromEnv: string | undefined,
): { identity: string } | { error: string } {
  const env = fromEnv?.trim();
  if (env) return { identity: env };
  for (const line of findIdentityOutput.split('\n')) {
    const m = /^\s*\d+\)\s+[0-9A-F]{40}\s+"(Apple Development: [^"]+)"/.exec(line);
    if (m) return { identity: m[1] };
  }
  return {
    error:
      'no "Apple Development" signing identity in the keychain; create one in Xcode (Settings > Accounts > Manage Certificates) or set APPLE_SIGNING_IDENTITY to another code-signing identity',
  };
}

async function run(cmd: string[], env?: Record<string, string>): Promise<number> {
  const proc = Bun.spawn(cmd, {
    stdout: 'inherit',
    stderr: 'inherit',
    env: { ...process.env, ...env },
  });
  return proc.exited;
}

async function main(): Promise<number> {
  const opts = parseArgs(process.argv.slice(2));
  if ('error' in opts) {
    console.error(`error: ${opts.error}`);
    return 2;
  }
  const list = Bun.spawnSync(['security', 'find-identity', '-v', '-p', 'codesigning']);
  const picked = pickIdentity(list.stdout.toString(), process.env.APPLE_SIGNING_IDENTITY);
  if ('error' in picked) {
    console.error(`error: ${picked.error}`);
    return 1;
  }
  console.error(`signing with ${picked.identity}`);
  const built = await run(['bun', 'run', 'tauri', 'build', '--debug', '--bundles', 'app'], {
    APPLE_SIGNING_IDENTITY: picked.identity,
  });
  if (built !== 0) return 1;
  if (opts.open) return (await run(['open', BUNDLE])) === 0 ? 0 : 1;
  console.error(`built ${BUNDLE}`);
  return 0;
}

if (import.meta.main) process.exit(await main());

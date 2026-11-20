import { describe, expect, test } from 'bun:test';
import { bundleOptions, cliExecutable, MSI_SOURCE_ENV } from '../scripts/app-bundle';

describe('app bundle options', () => {
  test('selects an Apple Silicon target, debug profile and ad hoc signing', () => {
    expect(bundleOptions(['app', '--debug', '--adhoc'])).toEqual({
      command: 'app',
      target: 'aarch64-apple-darwin',
      debug: true,
      adhoc: true,
      bundleOnly: false,
    });
  });

  test('selects native Windows x64 for the MSI', () => {
    expect(bundleOptions(['msi']).target).toBe('x86_64-pc-windows-msvc');
  });

  test('the split signed build prepares and bundles the release DMG', () => {
    expect(bundleOptions(['prepare']).debug).toBe(false);
    expect(bundleOptions(['dmg', '--bundle-only']).bundleOnly).toBe(true);
  });

  test('rejects unsupported targets and flags', () => {
    for (const args of [
      [],
      ['deb'],
      ['msi', '--debug'],
      ['prepare', '--adhoc'],
      ['app', '--bundle-only'],
      ['dmg', '--bundle-only', '--adhoc'],
      ['dmg', '--adhoc', '--adhoc'],
      ['dmg', '--target'],
    ]) {
      expect(() => bundleOptions(args)).toThrow('usage:');
    }
  });
});

describe('Cargo CLI executable', () => {
  const artifact = {
    reason: 'compiler-artifact',
    target: { name: 'strudeland', kind: ['bin'] },
    executable: 'C:\\shared target\\release\\strudeland.exe',
  };

  test('uses the exact artifact path reported by Cargo', () => {
    const messages = [
      { reason: 'compiler-artifact', target: { name: 'strudeland', kind: ['lib'] } },
      { reason: 'build-finished', success: true },
      artifact,
    ];
    expect(cliExecutable(messages.map((message) => JSON.stringify(message)).join('\n'))).toBe(
      artifact.executable,
    );
  });

  test('rejects missing and ambiguous executables', () => {
    const message = JSON.stringify(artifact);
    expect(() => cliExecutable('')).toThrow('one strudeland CLI executable');
    expect(() => cliExecutable(`${message}\n${message}`)).toThrow('one strudeland CLI executable');
  });
});

test('the WiX source variable survives Tauri’s restricted installer environment', async () => {
  expect(MSI_SOURCE_ENV.startsWith('TAURI')).toBe(true);
  const fragment = await Bun.file(new URL('../src-tauri/windows/cli.wxs', import.meta.url)).text();
  expect(fragment).toContain(`$(env.${MSI_SOURCE_ENV})`);
});

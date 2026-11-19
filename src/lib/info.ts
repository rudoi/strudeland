import { invoke, isTauri } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';

export interface AppInfo {
  version: string;
  /** The vendored Strudel (from strudel-pins.json via Rust). */
  strudel: {
    /** Upstream commit, 40 hex. */
    commit: string;
    /** Its commit date, ISO 8601. */
    date: string;
    /** npm package name -> pinned version. */
    kabelsalat: Record<string, string>;
  };
}

/** The version tooltip: `Strudel <short commit> (<day>)`, then one `<name> <version>` line each. */
export function formatStrudel(s: AppInfo['strudel']): string {
  const lines = [`Strudel ${s.commit.slice(0, 7)} (${s.date.slice(0, 10)})`];
  for (const [name, version] of Object.entries(s.kabelsalat)) lines.push(`${name} ${version}`);
  return lines.join('\n');
}

/** Rust round trip. Resolves null when not running inside Tauri (plain `bun run dev` in a browser). */
export async function getAppInfo(): Promise<AppInfo | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AppInfo>('app_info');
  } catch (e) {
    console.error('app_info failed', e); // a real Rust/IPC error, not "no Tauri"
    return null;
  }
}

/** Open a link in the system browser (WKWebView does not navigate away for target=_blank). */
export async function openExternal(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch {
    window.open(url, '_blank', 'noreferrer');
  }
}

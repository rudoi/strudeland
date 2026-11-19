/**
 * The status bar copy for sounds the Rust engine skipped: one notice per signature (the runtime
 * already dedupes them per play). The keys go in the notice's tooltip. Pure, so it is tested
 * without the UI.
 */
import type { EngineWarning } from './backend';

export interface EngineNotice {
  /** Stable per signature (the `{#each}` key). */
  key: string;
  text: string;
  /** The skipped keys; absent when there are none. */
  detail?: string;
}

export function engineNotices(ws: readonly EngineWarning[]): EngineNotice[] {
  return ws.map((w) => {
    const n: EngineNotice = { key: w.signature, text: `rust engine skipped ${w.signature}` };
    if (w.keys.length > 0) n.detail = `keys: ${w.keys.join(', ')}`;
    return n;
  });
}

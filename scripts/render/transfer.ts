/**
 * Pure driver helpers (no Playwright import, so `bun test` covers them): PCM decoding, the
 * harness and render error types, timeouts and static path resolution for the page server.
 */

import { isAbsolute, relative, resolve, sep } from 'node:path';
import type { RenderError } from '../../src/lib/render/contract';

export type HarnessErrorCode =
  'chromium-missing' | 'build-missing' | 'eval-failed' | 'render-failed' | 'stalled';

/** A failure in the harness. The environment codes stop the whole check (exit 2); the page
 *  codes (`eval-failed`, `render-failed`, `stalled`) fail only the fixture being rendered. */
export class HarnessError extends Error {
  constructor(
    readonly code: HarnessErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'HarnessError';
  }
}

/** Codes that mean the environment is unusable, so no fixture can render. */
const FATAL_CODES: ReadonlySet<HarnessErrorCode> = new Set(['chromium-missing', 'build-missing']);

export const isFatalHarnessError = (e: HarnessError): boolean => FATAL_CODES.has(e.code);

/** `fn` over `items`, at most `n` at a time, in order; results keep the items' order. */
export async function mapBatched<T, R>(
  items: readonly T[],
  n: number,
  fn: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  if (!(n >= 1)) throw new RangeError(`batch size must be >= 1, got ${n}`);
  const out: R[] = [];
  for (let i = 0; i < items.length; i += n) {
    const batch = items.slice(i, i + n);
    out.push(...(await Promise.all(batch.map((item, k) => fn(item, i + k)))));
  }
  return out;
}

/** The browser stopped answering within a deadline; the caller disposes the renderer. */
export class RendererStalledError extends HarnessError {
  constructor(message: string) {
    super('stalled', message);
    this.name = 'RendererStalledError';
  }
}

/** A render returned `ok: false` (or a gate re-render did): the fixture is a render error. */
export class RenderFailedError extends Error {
  constructor(readonly lines: string[]) {
    super(lines.join('; '));
    this.name = 'RenderFailedError';
  }
}

/** `kind: message`, with ` (bar N)` when the error names a bar. */
export const formatRenderError = (e: RenderError): string =>
  `${e.kind}: ${e.message}${e.bar !== undefined ? ` (bar ${e.bar})` : ''}`;

/** A Float32 channel from the page's base64 (little endian), decoded straight into a fresh
 *  (so aligned) buffer: a pooled `Buffer`'s `byteOffset` need not be a multiple of 4. */
export function decodeChannel(b64: string): Float32Array {
  const len = Buffer.byteLength(b64, 'base64');
  const bytes = Buffer.from(new ArrayBuffer(len));
  const n = bytes.write(b64, 'base64');
  if (n % 4 !== 0) throw new RangeError(`channel data is ${n} bytes, not a multiple of 4`);
  return new Float32Array(bytes.buffer, 0, n / 4);
}

/** The page's job JSON from its slices (`jobSlice(0..n)`), in order. */
export function assembleSlices(slices: readonly string[]): string {
  return slices.join('');
}

/** `p`, or a `RendererStalledError` naming `what` after `ms`. The timer is always cleared. */
export async function withTimeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new RendererStalledError(`${what} timed out after ${ms} ms`)),
      ms,
    );
  });
  try {
    return await Promise.race([p, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/** The file a URL path names under `root`, or null when it does not decode or escapes `root`.
 *  `/` and directory paths are not mapped to an index here; the caller does that. */
export function resolveStatic(root: string, urlPath: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  if (decoded.includes('\0')) return null;
  const base = resolve(root);
  const file = resolve(base, '.' + sep + decoded.replace(/^[/\\]+/, ''));
  const rel = relative(base, file);
  if (rel === '' || rel === '..' || rel.startsWith('..' + sep) || isAbsolute(rel)) return null;
  return file;
}

/** A page-listener message (`[page error] ...`, `[console] ...`) as a `RenderError`. */
export function pageError(text: string): RenderError {
  return { kind: /worklet|processor/i.test(text) ? 'worklet' : 'other', message: text };
}

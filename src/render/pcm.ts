/**
 * Pure helpers of the render page: PCM to base64 (the driver decodes it with
 * `scripts/render/transfer.ts` `decodeChannel`), the job JSON and its slices (the driver joins
 * them with `assembleSlices`), and the page's error bookkeeping.
 */

export { asciiJson, dedupeErrors, pageJob } from '../lib/render/page-job';
export { isErrorLog } from '../lib/render/logged-errors';

/** `btoa` input slice: small enough for `String.fromCharCode(...)`, and a multiple of 3 so the
 *  slices' base64 concatenates without padding in the middle. */
export const SLICE_BYTES = 3 * 10_922;

/** Base64 of a Float32 channel's bytes (platform order; WebAudio platforms are little endian). */
export function encodeChannel(f: Float32Array): string {
  const bytes = new Uint8Array(f.buffer, f.byteOffset, f.byteLength);
  let out = '';
  for (let i = 0; i < bytes.length; i += SLICE_BYTES) {
    out += btoa(String.fromCharCode(...bytes.subarray(i, i + SLICE_BYTES)));
  }
  return out;
}

/** Default characters per job slice: one bridge message stays well under its limits. */
export const JOB_SLICE_CHARS = 4 * 1024 * 1024;

/** Number of slices of a `length`-character string. */
export function sliceCount(length: number, sliceChars: number = JOB_SLICE_CHARS): number {
  if (!Number.isInteger(sliceChars) || sliceChars < 1) {
    throw new RangeError(`slice size must be a positive integer, got ${sliceChars}`);
  }
  return Math.ceil(length / sliceChars);
}

/** Slice `index` of `text`, `sliceChars` characters each; throws on an index out of range. */
export function sliceText(
  text: string,
  index: number,
  sliceChars: number = JOB_SLICE_CHARS,
): string {
  const n = sliceCount(text.length, sliceChars);
  if (!Number.isInteger(index) || index < 0 || index >= n) {
    throw new RangeError(`slice ${index} out of range (${n} slices)`);
  }
  return text.slice(index * sliceChars, (index + 1) * sliceChars);
}

/** The part of an `AudioWorkletNode` `processorMessage` uses. */
export interface MessagingNode {
  port: { onmessage: ((e: MessageEvent) => void) | null };
  onprocessorerror: ((e: ErrorEvent) => void) | null;
}

/** The first message the node's processor posts; rejects with the processor's error when it
 *  throws (in its constructor, or in `process()`), which otherwise never posts. */
export function processorMessage<T>(node: MessagingNode, name: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    node.port.onmessage = (e) => resolve(e.data as T);
    node.onprocessorerror = (e) => {
      reject(new Error(`${name}: processor error${e.message ? `: ${e.message}` : ''}`));
    };
  });
}

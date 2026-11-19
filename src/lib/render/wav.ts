/**
 * WAV encode and decode for render output. `f32` writes samples verbatim (NaN, Inf and
 * over-unity are preserved: the harness must see them); `i16` clamps. Pure.
 */

export type WavFormat = 'f32' | 'i16';

export interface DecodedWav {
  channels: Float32Array[];
  sampleRate: number;
  format: WavFormat;
}

export class WavError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'WavError';
  }
}

const TAG_PCM = 1;
const TAG_FLOAT = 3;
const TAG_EXTENSIBLE = 0xfffe;

function writeAscii(view: DataView, offset: number, s: string): void {
  for (let i = 0; i < s.length; i++) view.setUint8(offset + i, s.charCodeAt(i));
}

function readAscii(bytes: Uint8Array, offset: number, len: number): string {
  let s = '';
  for (let i = 0; i < len && offset + i < bytes.length; i++) {
    s += String.fromCharCode(bytes[offset + i]!);
  }
  return s;
}

/** Interleaved little-endian WAV with the canonical 44-byte header. */
export function encodeWav(
  channels: Float32Array[],
  sampleRate: number,
  format: WavFormat,
): Uint8Array {
  if (channels.length === 0) throw new WavError('no channels');
  if (channels.length > 65535)
    throw new WavError(`unsupported format: ${channels.length} channels`);
  if (!Number.isInteger(sampleRate) || sampleRate <= 0) {
    throw new WavError(`invalid sample rate ${sampleRate}`);
  }
  const frames = channels[0]!.length;
  if (channels.some((c) => c.length !== frames)) throw new WavError('channel length mismatch');

  const nch = channels.length;
  const bytesPerSample = format === 'f32' ? 4 : 2;
  const blockAlign = nch * bytesPerSample;
  const dataSize = frames * blockAlign;
  if (dataSize + 36 > 0xffffffff) throw new WavError('data larger than 4 GiB');

  const buf = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buf);
  writeAscii(view, 0, 'RIFF');
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(view, 8, 'WAVE');
  writeAscii(view, 12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, format === 'f32' ? TAG_FLOAT : TAG_PCM, true);
  view.setUint16(22, nch, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * blockAlign, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bytesPerSample * 8, true);
  writeAscii(view, 36, 'data');
  view.setUint32(40, dataSize, true);

  let off = 44;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < nch; c++) {
      const v = channels[c]![i]!;
      if (format === 'f32') {
        view.setFloat32(off, v, true);
        off += 4;
      } else {
        const clamped = Number.isNaN(v) ? 0 : Math.max(-1, Math.min(1, v));
        view.setInt16(off, Math.round(clamped * 32767), true);
        off += 2;
      }
    }
  }
  return new Uint8Array(buf);
}

interface Fmt {
  tag: number;
  channels: number;
  sampleRate: number;
  bits: number;
}

/** Decode a PCM16, float32 or extensible WAV into one Float32Array per channel. */
export function decodeWav(bytes: Uint8Array): DecodedWav {
  if (bytes.length < 12 || readAscii(bytes, 0, 4) !== 'RIFF' || readAscii(bytes, 8, 4) !== 'WAVE') {
    throw new WavError('not a RIFF/WAVE file');
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let fmt: Fmt | null = null;
  let off = 12;
  while (off + 8 <= bytes.length) {
    const id = readAscii(bytes, off, 4);
    const size = view.getUint32(off + 4, true);
    const body = off + 8;
    if (id === 'fmt ') {
      if (size < 16 || body + 16 > bytes.length) throw new WavError('missing fmt chunk');
      let tag = view.getUint16(body, true);
      if (tag === TAG_EXTENSIBLE) {
        // cbSize (2) at +16, validBits (2), channelMask (4), then the sub-format GUID at +24.
        if (size < 40 || body + 26 > bytes.length) {
          throw new WavError(`unsupported format ${tag}/${view.getUint16(body + 14, true)}`);
        }
        tag = view.getUint16(body + 24, true);
      }
      fmt = {
        tag,
        channels: view.getUint16(body + 2, true),
        sampleRate: view.getUint32(body + 4, true),
        bits: view.getUint16(body + 14, true),
      };
    } else if (id === 'data') {
      if (!fmt) throw new WavError('missing fmt chunk');
      return decodeData(view, body, Math.min(size, bytes.length - body), fmt);
    }
    off = body + size + (size % 2);
  }
  throw new WavError(fmt ? 'missing data chunk' : 'missing fmt chunk');
}

/** Whether this host stores floats little-endian (as WAV does), so f32 data can be viewed. */
const LITTLE_ENDIAN = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;

function decodeData(view: DataView, start: number, size: number, fmt: Fmt): DecodedWav {
  let format: WavFormat;
  if (fmt.tag === TAG_PCM && fmt.bits === 16) format = 'i16';
  else if (fmt.tag === TAG_FLOAT && fmt.bits === 32) format = 'f32';
  else throw new WavError(`unsupported format ${fmt.tag}/${fmt.bits}`);
  if (fmt.channels === 0) throw new WavError(`unsupported format ${fmt.tag}/${fmt.bits}`);

  const bytesPerSample = fmt.bits / 8;
  const blockAlign = fmt.channels * bytesPerSample;
  const frames = Math.floor(size / blockAlign);
  const n = fmt.channels;
  const channels = Array.from({ length: n }, () => new Float32Array(frames));
  if (format === 'f32' && LITTLE_ENDIAN) {
    // One copy of the data (aligned, whatever the chunk's offset), then a strided de-interleave.
    const bytes = new Uint8Array(view.buffer, view.byteOffset + start, frames * blockAlign);
    const samples = new Float32Array(bytes.slice().buffer);
    for (let c = 0; c < n; c++) {
      const ch = channels[c]!;
      for (let i = 0, j = c; i < frames; i++, j += n) ch[i] = samples[j]!;
    }
  } else if (format === 'f32') {
    let off = start;
    for (let i = 0; i < frames; i++) {
      for (let c = 0; c < n; c++, off += 4) channels[c]![i] = view.getFloat32(off, true);
    }
  } else {
    let off = start;
    for (let i = 0; i < frames; i++) {
      for (let c = 0; c < n; c++, off += 2) channels[c]![i] = view.getInt16(off, true) / 32768;
    }
  }
  return { channels, sampleRate: fmt.sampleRate, format };
}

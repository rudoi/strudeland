import { readSongHeader, type MetaError, type SongHeader } from '../../src/lib/project/song-header';

/** `readSongHeader` of a header that must read cleanly; throws its error otherwise. */
export function okHeader(code: string): Extract<SongHeader, { ok: true }> {
  const header = readSongHeader(code);
  if (!header.ok) throw header.error;
  return header;
}

/** The error of a header that must break a rule. */
export function headerError(code: string): MetaError {
  const header = readSongHeader(code);
  if (header.ok) throw new Error('expected a meta error');
  return header.error;
}

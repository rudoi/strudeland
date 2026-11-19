/** Small pure helpers behind the project UI, kept out of the components so they can be tested. */

/** The default project name for a folder path: its last segment (trailing slashes ignored). */
export function defaultProjectName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  return trimmed.slice(Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\')) + 1);
}

export type AppShortcut = 'openProject' | 'newSong';
export type PrimaryModifier = 'metaKey' | 'ctrlKey';

/** Match CodeMirror's Command on Mac and Control elsewhere. */
export function primaryModifier(platform: string): PrimaryModifier {
  return platform.includes('Mac') ? 'metaKey' : 'ctrlKey';
}

/** Window-level open and new shortcuts with the platform's primary modifier alone. */
export function matchShortcut(
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'> & {
    repeat?: boolean;
  },
  modifier: PrimaryModifier,
): AppShortcut | null {
  if (!e[modifier] || (e.metaKey && e.ctrlKey) || e.altKey || e.shiftKey || e.repeat) return null;
  const key = e.key.toLowerCase();
  if (key === 'o') return 'openProject';
  if (key === 'n') return 'newSong';
  return null;
}

/** The extension of song files. */
export const SONG_EXT = '.js';

/** `name` without a trailing song extension. */
export function stripSongExt(name: string): string {
  return name.endsWith(SONG_EXT) ? name.slice(0, -SONG_EXT.length) : name;
}

/** The project path of the song named `name`: "main" to "songs/main.js". */
export function songPath(name: string): string {
  return `songs/${name}${SONG_EXT}`;
}

/** A song name typed into the "+ New song" field: trimmed, and a trailing ".js" is dropped. */
export function songStem(input: string): string {
  return stripSongExt(input.trim());
}

/** Transport views and segments shared by the song and transport tests. */
import type { Segment, View } from '../../src/lib/strudel/transport';

export const SONG: View = { kind: 'song' };
export const RELEASE: View = { kind: 'release' };
export const loopView = (section: number): View => ({ kind: 'loop', section });
export const seg = (at: number, songBar: number, view: View): Segment => ({ at, songBar, view });

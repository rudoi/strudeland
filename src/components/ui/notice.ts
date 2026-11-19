import type { Tone } from '../../lib/theme/tones';

/** The tones a `Notice` takes: every tone but `accent`. */
export type NoticeTone = Exclude<Tone, 'accent'>;

/** A danger notice interrupts (`alert`); the rest, inherited included, are polite (`status`). */
export function noticeRole(tone: NoticeTone | undefined): 'alert' | 'status' {
  return tone === 'danger' ? 'alert' : 'status';
}

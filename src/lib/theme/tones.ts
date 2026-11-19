/** The meanings a primitive's `tone` prop can take; each sets `data-tone` on its root. */
export type Tone = 'neutral' | 'accent' | 'warning' | 'danger';

/** A categorical colour index (`data-cat`), from the `--cat-1` … `--cat-4` ramp. */
export type Cat = 1 | 2 | 3 | 4;

/** The category for the i-th item (section 0 is cat 1), cycling through the four. */
export function catOf(i: number): Cat {
  return ((((i % 4) + 4) % 4) + 1) as Cat;
}

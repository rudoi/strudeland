import type { Tone } from '../../lib/theme/tones';

export interface PressedAttrs {
  /** `aria-pressed`; undefined leaves the attribute off. */
  ariaPressed: 'true' | 'false' | undefined;
  /** The tone to set; undefined keeps the variant's default. */
  dataTone: Tone | undefined;
  /** Whether the `pressed` class applies. */
  pressedClass: boolean;
}

/**
 * A toggle button's attributes. `pressed` unset changes nothing; set, it gives `aria-pressed`, and
 * when true the `pressed` class and the `accent` tone (an explicit `tone` wins).
 */
export function pressedAttrs(pressed: boolean | undefined, tone: Tone | undefined): PressedAttrs {
  if (pressed === undefined) return { ariaPressed: undefined, dataTone: tone, pressedClass: false };
  return {
    ariaPressed: pressed ? 'true' : 'false',
    dataTone: tone ?? (pressed ? 'accent' : undefined),
    pressedClass: pressed,
  };
}

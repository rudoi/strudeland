<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import type { Tone } from '../../lib/theme/tones';
  import { pressedAttrs } from './button';

  type Props = HTMLButtonAttributes & {
    variant?: 'default' | 'primary' | 'quiet';
    /**
     * Tints text and outline, or the fill of a `primary`. Defaults to `accent` for `primary` and
     * `neutral` otherwise.
     */
    tone?: Tone;
    size?: 'sm' | 'md';
    /**
     * A toggle's state: unset is a plain button; set renders `aria-pressed`, and when true the
     * pressed look in the `accent` tone (an explicit `tone` wins). The caller toggles it.
     */
    pressed?: boolean;
    children: Snippet;
  };

  let {
    variant = 'default',
    tone,
    size = 'md',
    pressed,
    children,
    class: className,
    type = 'button',
    ...rest
  }: Props = $props();

  const attrs = $derived(pressedAttrs(pressed, tone));
  const dataTone = $derived(attrs.dataTone ?? (variant === 'primary' ? 'accent' : 'neutral'));
</script>

<button
  {...rest}
  {type}
  class={['btn', 'control', 'pressable', variant, size, { pressed: attrs.pressedClass }, className]}
  data-tone={dataTone}
  aria-pressed={attrs.ariaPressed}
>
  {@render children()}
</button>

<style>
  /* Height, padding, outline and the hover, press and disabled states: .control and .pressable. */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1-5);
    background-color: var(--surface-2);
    color: var(--text);
    font-weight: 500;
    white-space: nowrap;
    transition:
      background-color var(--dur-fast) var(--ease),
      transform var(--dur-fast) var(--ease);
  }
  .quiet {
    background-color: transparent;
    border-color: transparent;
  }
  .default:enabled:not([data-tone='neutral']) {
    color: var(--tone);
    border-color: var(--tone);
  }
  .quiet:enabled:not([data-tone='neutral']) {
    color: var(--tone);
  }
  /* A pressed toggle (a primary keeps its fill); disabled falls through to .btn:disabled. */
  .default.pressed:enabled,
  .quiet.pressed:enabled {
    background-color: var(--tone-wash);
    color: var(--tone);
    border-color: var(--tone);
  }
  .primary {
    font-weight: 600;
  }
  .primary:enabled {
    background-color: var(--tone);
    border-color: var(--tone);
    color: var(--tone-ink);
  }
  /* A primary lightens on hover instead of taking the hover overlay; it still takes the press one. */
  .primary:hover:enabled {
    background-color: color-mix(in srgb, var(--tone) 80%, var(--text));
  }
  .primary:hover:not(:active) {
    background-image: none;
  }
  .btn:active:enabled {
    transform: translateY(1px);
  }
  .btn:disabled {
    background-color: transparent;
  }
  .quiet:disabled {
    border-color: transparent;
  }
</style>

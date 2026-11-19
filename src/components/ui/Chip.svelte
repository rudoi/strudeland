<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import type { Tone } from '../../lib/theme/tones';

  type Props = HTMLAttributes<HTMLSpanElement> & {
    /** Unset: inherits the nearest `data-tone` or `data-cat` (neutral at the root). */
    tone?: Tone;
    /** Filled with the tone instead of outlined. */
    lit?: boolean;
    children: Snippet;
  };

  let { tone, lit = false, children, class: className, ...rest }: Props = $props();
</script>

<span {...rest} class={['chip', { lit }, className]} data-tone={tone}>{@render children()}</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    flex: none;
    padding: var(--space-0) var(--space-1-5);
    border-radius: var(--radius-pill);
    border: 1px solid var(--tone-edge);
    color: var(--tone);
    font-size: var(--fs-2xs);
    font-weight: 600;
    line-height: 1;
    letter-spacing: var(--tracking-label);
    text-transform: lowercase;
    white-space: nowrap;
  }
  .lit {
    background-color: var(--tone);
    border-color: var(--tone);
    color: var(--tone-ink);
  }
</style>

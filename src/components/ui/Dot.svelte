<script lang="ts">
  import type { HTMLAttributes } from 'svelte/elements';
  import type { Tone } from '../../lib/theme/tones';

  type Props = HTMLAttributes<HTMLSpanElement> & {
    /** Unset: inherits the nearest `data-tone` or `data-cat` (neutral at the root). */
    tone?: Tone;
    size?: 'sm' | 'md';
    shape?: 'round' | 'square';
  };

  let { tone, size = 'md', shape = 'round', class: className, ...rest }: Props = $props();
</script>

<!-- Decorative by default; pass aria-hidden={false} with a label or title to announce it. -->
<span aria-hidden="true" {...rest} class={['dot', size, shape, className]} data-tone={tone}></span>

<style>
  .dot {
    display: inline-block;
    flex: none;
    width: 8px;
    height: 8px;
    background-color: var(--tone);
  }
  .sm {
    width: 6px;
    height: 6px;
  }
  .round {
    border-radius: 50%;
  }
  .square {
    border-radius: var(--radius-xs);
  }
</style>

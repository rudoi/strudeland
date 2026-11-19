<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Props = HTMLButtonAttributes & { children: Snippet };

  let { children, class: className, ...rest }: Props = $props();
</script>

<button type="button" role="menuitem" {...rest} class={['item', 'pressable', className]}>
  {@render children()}
</button>

<style>
  /* Hover, press and disabled: .pressable. */
  .item {
    display: block;
    width: 100%;
    padding: var(--space-1-5) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--text);
    text-align: left;
    white-space: nowrap;
  }
</style>

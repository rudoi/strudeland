<script lang="ts">
  import { helpTarget } from '../../lib/help/targets';
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Props = HTMLButtonAttributes & { help?: string; children: Snippet };

  let { help, children, class: className, ...rest }: Props = $props();
</script>

<button
  use:helpTarget={help}
  type="button"
  role="menuitem"
  {...rest}
  class={['item', 'pressable', className]}
>
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

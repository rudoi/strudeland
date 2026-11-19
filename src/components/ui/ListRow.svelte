<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Props = HTMLButtonAttributes & {
    active?: boolean;
    children: Snippet;
    /** Sits at the right, inside the button. */
    trailing?: Snippet;
  };

  let { active = false, children, trailing, class: className, ...rest }: Props = $props();
</script>

<button
  type="button"
  aria-current={active ? 'true' : undefined}
  {...rest}
  class={['row', 'pressable', { active }, className]}
>
  <span class="main">{@render children()}</span>
  {#if trailing}<span class="trailing">{@render trailing()}</span>{/if}
</button>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--control-md);
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--text-dim);
    text-align: left;
  }
  /* The hover overlay and the disabled look: .pressable (the overlay sits on the active fill). */
  .row:hover:not(:disabled) {
    color: var(--text);
  }
  .active {
    background-color: var(--surface-3);
    color: var(--text);
  }
  .active::before {
    content: '';
    position: absolute;
    left: 0;
    top: var(--space-1);
    bottom: var(--space-1);
    width: 2px;
    border-radius: var(--radius-pill);
    background-color: var(--accent);
  }
  .main {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trailing {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1-5);
    flex: none;
  }
</style>

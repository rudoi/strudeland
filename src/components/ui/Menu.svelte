<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  type Props = HTMLAttributes<HTMLDivElement> & {
    open: boolean;
    /** Called on a window click outside the menu or on Escape. */
    onclose: () => void;
    children: Snippet;
  };

  let { open, onclose, children, class: className, ...rest }: Props = $props();

  // Window listeners only while open. The opener stops its click's propagation, so the click that
  // opens the menu does not close it.
  $effect(() => {
    if (!open) return;
    const onClick = () => onclose();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('click', onClick);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('click', onClick);
      window.removeEventListener('keydown', onKey);
    };
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    role="menu"
    tabindex="-1"
    {...rest}
    class={['menu', className]}
    onclick={(e) => e.stopPropagation()}
  >
    {@render children()}
  </div>
{/if}

<style>
  .menu {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: var(--z-pop);
    display: flex;
    flex-direction: column;
    min-width: 140px;
    margin-top: var(--space-1);
    padding: var(--space-1);
    border-radius: var(--radius-md);
    background-color: var(--surface-2);
    box-shadow: var(--shadow-pop);
  }
</style>

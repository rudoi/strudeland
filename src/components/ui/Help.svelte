<script lang="ts">
  import { onMount } from 'svelte';
  import { helpMessage } from '../../lib/help/targets';

  let { compact = false }: { compact?: boolean } = $props();
  let element: HTMLDivElement;
  let scope = $state<HTMLDialogElement | null>(null);

  onMount(() => {
    scope = element.closest('dialog');
  });

  const idle = 'Hover over a control or use Tab to see what it does.';
  const activeText = $derived($helpMessage?.scope === scope ? $helpMessage.text : '');
  const inactive = $derived(compact && $helpMessage?.scope != null);
</script>

<div class="help" class:compact bind:this={element} data-help-gutter aria-hidden="true">
  <span class="label">{inactive ? '' : 'help'}</span>
  <span class="message">{inactive ? '' : activeText || idle}</span>
</div>

<style>
  .help {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    align-items: start;
    gap: var(--space-1-5);
    min-inline-size: 0;
    min-block-size: 2lh;
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: var(--fs-sm) / 1.3 var(--font-ui);
  }

  .label {
    color: var(--text-dim);
    font: inherit;
    font-weight: 500;
  }

  .message {
    max-inline-size: 78ch;
    overflow-wrap: anywhere;
  }

  .compact {
    padding: 0;
    border: 0;
  }

  @media (max-width: 800px) {
    .compact {
      min-block-size: 3lh;
    }
  }
</style>

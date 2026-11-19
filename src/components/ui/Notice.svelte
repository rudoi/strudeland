<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import Dot from './Dot.svelte';
  import { noticeRole, type NoticeTone } from './notice';

  type Props = HTMLAttributes<HTMLDivElement> & {
    /** Unset: inherits the nearest `data-tone` or `data-cat` (neutral at the root). */
    tone?: NoticeTone;
    layout?: 'inline' | 'bar';
    children: Snippet;
    /** Buttons at the right. */
    actions?: Snippet;
  };

  let { tone, layout = 'inline', children, actions, class: className, ...rest }: Props = $props();
</script>

<div role={noticeRole(tone)} {...rest} class={['notice', layout, className]} data-tone={tone}>
  {#if layout === 'inline'}<Dot {tone} size="sm" />{/if}
  <span class="text">{@render children()}</span>
  {#if actions}<span class="actions">{@render actions()}</span>{/if}
</div>

<style>
  .notice {
    color: var(--tone);
    min-width: 0;
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1-5);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-1-5) var(--space-3);
    background-color: var(--tone-wash);
    border-bottom: 1px solid var(--tone-edge);
  }
  .text {
    min-width: 0;
  }
  .actions {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: auto;
    flex: none;
  }
</style>

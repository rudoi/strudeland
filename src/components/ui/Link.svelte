<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAnchorAttributes } from 'svelte/elements';
  import { openExternal } from '../../lib/info';
  import { followExternal } from './link';

  type Props = HTMLAnchorAttributes & { href: string; children: Snippet };

  let { href, children, onclick, class: className, ...rest }: Props = $props();

  function click(e: MouseEvent & { currentTarget: EventTarget & HTMLAnchorElement }) {
    onclick?.(e);
    followExternal(e, href, openExternal);
  }
</script>

<a {...rest} {href} class={['link', className]} onclick={click}>{@render children()}</a>

<style>
  .link {
    color: var(--link);
    text-decoration: underline;
    text-decoration-color: var(--border-strong);
    text-underline-offset: 2px;
    cursor: pointer;
    transition: color var(--dur-fast) var(--ease);
  }
  .link:hover {
    color: var(--link-hover);
  }
</style>

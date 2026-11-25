<script lang="ts">
  import { helpTarget } from '../../lib/help/targets';
  import type { Snippet } from 'svelte';
  import type { HTMLAnchorAttributes } from 'svelte/elements';
  import { openExternal } from '../../lib/info';
  import { followExternal } from './link';

  type Props = HTMLAnchorAttributes & { help?: string; href: string; children: Snippet };

  let { help, href, children, onclick, class: className, ...rest }: Props = $props();

  function click(e: MouseEvent & { currentTarget: EventTarget & HTMLAnchorElement }) {
    onclick?.(e);
    followExternal(e, href, openExternal);
  }
</script>

<a use:helpTarget={help} {...rest} {href} class={['link', className]} onclick={click}
  >{@render children()}</a
>

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

<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { helpTarget } from '../lib/help/targets';
  import type { Cat } from '../lib/theme/tones';
  import Chip from './ui/Chip.svelte';
  import Dot from './ui/Dot.svelte';

  /** Above this many bars the pads become one continuous meter. */
  const MAX_PADS = 16;

  type Props = Omit<HTMLButtonAttributes, 'onclick'> & {
    cat: Cat;
    name: string;
    bars: number;
    /** The section has `loop: true`. */
    loop: boolean;
    /** The transport is looping this section. */
    looping: boolean;
    /** This section is playing. */
    current: boolean;
    /** Pads lit (from `litBars`); 0 for a section that is not the current one. */
    lit: number;
    onclick: (e: MouseEvent) => void;
    /** Rendered after the block (the section `Menu`). */
    menu?: Snippet;
  };

  let {
    cat,
    name,
    bars,
    loop,
    looping,
    current,
    lit,
    onclick,
    menu,
    class: className,
    ...rest
  }: Props = $props();

  const pads = $derived(bars <= MAX_PADS ? Math.max(0, bars) : 0);
</script>

<button
  type="button"
  {...rest}
  class={['block', 'pressable', { loop, current }, className]}
  data-cat={cat}
  {onclick}
  use:helpTarget={'Choose where playback starts or which sections repeat.'}
>
  <span class="head">
    <Dot shape="square" />
    <span class="name">{name}</span>
    <span class="bars mono">{bars}</span>
    <!-- The loop chip takes the section colour (data-cat); looping is always the accent. -->
    {#if loop}<Chip title="part of the song’s defined loop">loop</Chip>{/if}
    {#if looping}<Chip tone="accent" lit title="part of the temporary playback loop">looping</Chip
      >{/if}
  </span>
  {#if pads > 0}
    <span class="pads" aria-hidden="true">
      {#each { length: pads }, k (k)}
        <span class="pad" class:past={k < lit - 1} class:now={k === lit - 1}></span>
      {/each}
    </span>
  {:else}
    <span class="meter" aria-hidden="true">
      <span class="fill" class:on={lit > 0} style:width="{bars > 0 ? (lit / bars) * 100 : 0}%"
      ></span>
    </span>
  {/if}
</button>
{@render menu?.()}

<style>
  .block {
    box-sizing: border-box;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-1-5);
    padding: var(--space-1-5);
    border: 1px solid var(--tone-edge);
    border-radius: var(--radius-md);
    background-color: var(--surface-2);
    color: var(--text);
    text-align: left;
    overflow: hidden;
    transition:
      background-color var(--dur-fast) var(--ease),
      border-color var(--dur-fast) var(--ease);
  }
  .current {
    background-color: var(--tone-wash);
  }
  .loop {
    border-color: var(--tone);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-1-5);
    min-width: 0;
    font-size: var(--fs-sm);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bars {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .pads {
    display: flex;
    gap: var(--space-1);
  }
  .pad,
  .meter {
    flex: 1;
    height: 6px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-3);
    transition:
      background-color var(--dur-fast) var(--ease),
      box-shadow var(--dur-fast) var(--ease);
  }
  .pad.past {
    background-color: var(--tone-wash);
  }
  .pad.now {
    background-color: var(--tone-edge);
    box-shadow: 0 0 0 1px var(--tone);
  }
  .meter {
    display: block;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background-color: var(--tone-wash);
    transition: width var(--dur-fast) var(--ease);
  }
  .fill.on {
    box-shadow: inset -1px 0 0 var(--tone);
  }
</style>

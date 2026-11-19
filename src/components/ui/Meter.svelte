<script lang="ts">
  import type { HTMLAttributes } from 'svelte/elements';

  type Props = HTMLAttributes<HTMLDivElement> & {
    /** Display fractions 0..1 (from `meter.ts`). */
    rms: number;
    peak: number;
    /** The peak tick takes the clip colour. */
    clip: boolean;
    /** False draws the empty track only. */
    active: boolean;
    /** A slim bar for stacking (the master meter): no inset well. */
    thin?: boolean;
  };

  let { rms, peak, clip, active, thin = false, class: className, ...rest }: Props = $props();

  const clamp = (x: number) => (Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0);
</script>

<!-- Not announced: it changes 30 times a second; the opener carries the accessible name. -->
<div aria-hidden="true" {...rest} class={['meter', { thin }, className]}>
  {#if active}
    <span class="fill" style:transform="scaleX({clamp(rms)})"></span>
    <span class="tick" class:clip style:left="{clamp(peak) * 100}%"></span>
  {/if}
</div>

<style>
  .meter {
    position: relative;
    flex: none;
    width: var(--meter-w);
    height: var(--space-2);
    border-radius: var(--radius-pill);
    background-color: var(--meter-well);
    box-shadow: inset 0 0 0 1px var(--border);
    overflow: hidden;
  }
  .meter.thin {
    height: var(--space-1);
    box-shadow: none;
  }
  .fill {
    position: absolute;
    inset: 0;
    background-color: var(--meter-rms);
    transform-origin: left;
  }
  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background-color: var(--meter-peak);
  }
  .tick.clip {
    background-color: var(--meter-clip);
  }
</style>

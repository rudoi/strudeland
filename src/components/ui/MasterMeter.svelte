<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import {
    createMasterMeter,
    type LevelFrame,
    type MasterMeterView,
  } from '../../lib/levels/levels';
  import { SCOPE_CLOCK_KEY, type ScopeClock } from '../../lib/scope/clock';
  import Button from './Button.svelte';
  import Dot from './Dot.svelte';
  import Meter from './Meter.svelte';
  import { createMeterModel } from './meter';

  type Props = {
    /** The runtime's merged level frame since the last call, or null. */
    drain: () => LevelFrame | null;
    limiter: boolean;
    /** False when the master worklet did not load: the toggle is disabled. */
    available: boolean;
    onlimiter: (on: boolean) => void;
  };

  let { drain, limiter, available, onlimiter }: Props = $props();

  // Outside App there is no clock: the meter stays empty and never subscribes.
  const clock = getContext<ScopeClock | undefined>(SCOPE_CLOCK_KEY);

  const meter = createMasterMeter(
    () => performance.now(),
    () => createMeterModel(() => performance.now()),
  );
  const REST: MasterMeterView = {
    active: false,
    channels: [
      { rms: 0, peak: 0, clip: false },
      { rms: 0, peak: 0, clip: false },
    ],
    readout: '-',
    clip: false,
    limiting: false,
  };
  let view = $state.raw<MasterMeterView>(REST);

  onMount(() => {
    if (!clock) return;
    // Levels do not depend on scope samples: rest only when the clock's loop stops (stopped, or
    // the window hidden).
    const unsubscribe = clock.subscribe((_buf, running) => {
      view = running ? meter.step(drain()) : meter.step(null, false);
    });
    return () => {
      unsubscribe();
      view = meter.step(null, false);
    };
  });
</script>

<div class="master">
  <Button
    size="sm"
    variant="quiet"
    pressed={limiter}
    disabled={!available && !limiter}
    title={available
      ? 'safety limiter on the master output'
      : 'the limiter is unavailable: the master worklet did not load'}
    onclick={() => onlimiter(!limiter)}
  >
    {#if view.limiting}<Dot size="sm" tone="warning" />{/if}
    limit
  </Button>
  <button type="button" class="bars" aria-label="clear clip" onclick={() => meter.clearClip()}>
    {#each ['l', 'r'] as label, ch (label)}
      {@const c = view.channels[ch]!}
      <span class="label">{label}</span>
      <Meter
        thin
        active={view.active}
        rms={c.rms}
        peak={c.peak}
        clip={c.clip}
        style="width: 100%"
      />
    {/each}
  </button>
  <span class="readout" class:clip={view.clip}>{view.readout}</span>
</div>

<style>
  .master {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    flex: none;
  }
  .bars {
    display: grid;
    grid-template-columns: auto var(--master-meter-w);
    gap: var(--space-1) var(--space-1-5);
    align-items: center;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .label {
    font-size: var(--fs-2xs);
    line-height: 1;
  }
  .readout {
    min-width: 6ch;
    text-align: right;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .readout.clip {
    color: var(--meter-clip);
  }
</style>

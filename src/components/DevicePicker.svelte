<script lang="ts">
  import type { MeterView } from './ui/meter';
  import type { RecordingController } from '../lib/recording/controller';
  import { channelChoices, inputLabel, pickerDisabled } from '../lib/recording/memo-ui';
  import type { RecordingUiState } from '../lib/recording/types';
  import Button from './ui/Button.svelte';
  import Chip from './ui/Chip.svelte';
  import Dot from './ui/Dot.svelte';
  import Menu from './ui/Menu.svelte';
  import MenuItem from './ui/MenuItem.svelte';
  import Meter from './ui/Meter.svelte';

  let {
    recording,
    rec,
    hasProject,
    level,
  }: {
    recording: RecordingController | undefined;
    rec: RecordingUiState | null;
    hasProject: boolean;
    /** The meter's display values; null draws the empty track. */
    level: MeterView | null;
  } = $props();

  let open = $state(false);
  const label = $derived(inputLabel(rec));
  const disabled = $derived(pickerDisabled(rec, hasProject));
  const channels = $derived(channelChoices(rec));
  const devices = $derived(rec?.devices ?? []);

  // A take starting or the project closing shuts the menu.
  $effect(() => {
    if (disabled) open = false;
  });

  function opener(e: MouseEvent) {
    e.stopPropagation();
    open = !open;
    if (open) void recording?.refreshDevices();
  }

  function choose(deviceId: string, channel: number) {
    open = false;
    void recording?.selectInput(deviceId, channel);
  }
</script>

{#snippet mark(on: boolean)}
  {#if on}<Dot size="sm" tone="accent" />{:else}<span class="blank"></span>{/if}
{/snippet}

<span class="picker">
  <Button
    variant="quiet"
    size="sm"
    aria-label={label}
    title={label}
    aria-haspopup="menu"
    aria-expanded={open}
    {disabled}
    onclick={opener}
  >
    <Meter
      rms={level?.rms ?? 0}
      peak={level?.peak ?? 0}
      clip={level?.clip ?? false}
      active={level !== null}
    />
  </Button>
  <Menu {open} onclose={() => (open = false)} class="devices">
    <div class="label heading">input</div>
    {#each devices as d (d.id)}
      {@const on = rec?.input?.deviceId === d.id}
      <MenuItem
        role="menuitemradio"
        aria-checked={on}
        title={d.isDefault ? 'system default' : undefined}
        onclick={() => choose(d.id, 0)}
      >
        <span class="row">
          {@render mark(on)}
          <span class="name">{d.name}</span>
          {#if d.bluetooth}<Chip tone="warning">bluetooth</Chip>{/if}
        </span>
      </MenuItem>
    {:else}
      <MenuItem disabled>no input devices</MenuItem>
    {/each}
    {#if channels.length > 0 && rec?.input}
      {@const input = rec.input}
      <div class="label heading">channel</div>
      {#each channels as ch (ch)}
        <MenuItem
          role="menuitemradio"
          aria-checked={input.channel === ch}
          onclick={() => choose(input.deviceId, ch)}
        >
          <span class="row">
            {@render mark(input.channel === ch)}
            <span class="name">channel {ch + 1}</span>
          </span>
        </MenuItem>
      {/each}
    {/if}
  </Menu>
</span>

<style>
  .picker {
    position: relative;
    display: inline-flex;
  }
  .picker :global(.devices) {
    max-height: 60vh;
    overflow-y: auto;
  }
  .heading {
    padding: var(--space-1) var(--space-3) var(--space-0);
  }
  .row {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .blank {
    display: inline-block;
    flex: none;
    width: 6px;
  }
</style>

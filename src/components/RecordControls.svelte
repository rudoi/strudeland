<script lang="ts">
  import { onDestroy } from 'svelte';
  import { readable } from 'svelte/store';
  import { createMeterModel, type MeterView } from './ui/meter';
  import type { RecordingController } from '../lib/recording/controller';
  import {
    backingChoice,
    effectiveBacking,
    recordButton,
    toggle,
    toggleViews,
    type ToggleKey,
  } from '../lib/recording/memo-ui';
  import type { RecordingUiState } from '../lib/recording/types';
  import type { Session } from '../lib/project/session';
  import type { Memos } from '../lib/recording/memos';
  import Button from './ui/Button.svelte';
  import Dot from './ui/Dot.svelte';
  import DevicePicker from './DevicePicker.svelte';

  let {
    recording,
    session,
    memos,
  }: {
    recording: RecordingController | undefined;
    session: Session;
    /** A playing memo stops before a take starts. */
    memos?: Memos;
  } = $props();

  // Both are constant for the window's lifetime, so reading them once is correct.
  // svelte-ignore state_referenced_locally
  const ss = session.store;
  // svelte-ignore state_referenced_locally
  const recStore = recording?.store ?? readable<RecordingUiState | null>(null);
  const choice = backingChoice;

  const rec = $derived($recStore);
  const hasProject = $derived($ss.project !== null);
  const hasSong = $derived($ss.active !== null);

  // The elapsed time ticks only while recording.
  let now = $state(performance.now());
  $effect(() => {
    if (rec?.phase !== 'recording') return;
    now = performance.now();
    const id = setInterval(() => (now = performance.now()), 250);
    return () => clearInterval(id);
  });

  const btn = $derived(
    recordButton(rec, hasProject, rec?.takeStartedAtMs != null ? now - rec.takeStartedAtMs : 0),
  );
  const toggles = $derived(toggleViews($choice, rec?.phase ?? null, hasProject, hasSong));

  // The meter follows `recording://level`; null (not armed) resets it.
  let level = $state<MeterView | null>(null);
  const model = createMeterModel(() => performance.now());
  // svelte-ignore state_referenced_locally
  const unlevel = recording?.level.subscribe((l) => {
    if (l === null) {
      model.reset();
      level = null;
    } else level = model.push(l);
  });
  onDestroy(() => unlevel?.());

  function click() {
    if (!recording) return;
    if (btn.action === 'record') {
      memos?.stopPlayback();
      void recording.record(effectiveBacking($choice, hasSong));
    } else if (btn.action === 'stop') void recording.stop();
  }

  function flip(key: ToggleKey) {
    choice.update((c) => toggle(c, key, hasSong));
  }
</script>

<span class="group">
  <Button
    variant={btn.variant}
    tone={btn.tone}
    disabled={btn.disabled}
    title={btn.title}
    aria-label={btn.ariaLabel}
    onclick={click}
  >
    {#if btn.glyph === 'dot'}
      <Dot size="sm" tone="danger" />
    {:else if btn.glyph === 'square'}
      <Dot size="sm" shape="square" class="stop-glyph" />
    {/if}
    {#if btn.text}<span>{btn.text}</span>{/if}
    {#if btn.mono}<span class="mono">{btn.mono}</span>{/if}
  </Button>
  {#each toggles as t (t.key)}
    <Button
      size="sm"
      pressed={t.pressed}
      disabled={t.disabled}
      title={t.title}
      onclick={() => flip(t.key)}
    >
      {t.label}
    </Button>
  {/each}
  <DevicePicker {recording} {rec} {hasProject} {level} />
</span>

<style>
  .group {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  /* The stop square sits on the primary's danger fill: draw it in the fill's ink. */
  .group :global(.stop-glyph) {
    background-color: var(--tone-ink);
  }
</style>

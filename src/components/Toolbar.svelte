<script lang="ts">
  import { get, readable } from 'svelte/store';
  import type { App } from '../lib/app';
  import { takeRunning } from '../lib/recording/memo-ui';
  import type { RecordingUiState } from '../lib/recording/types';
  import { bpmFromCps, meterString } from '../lib/strudel/structure';
  import Button from './ui/Button.svelte';
  import Scope from './ui/Scope.svelte';
  import RecordControls from './RecordControls.svelte';

  let { app }: { app: App } = $props();
  // `app` is constant for the window's lifetime, so reading it once is correct.
  // svelte-ignore state_referenced_locally
  const { runtime, recording, session, memos, prefs } = app;
  const rt = runtime.store;
  const prefsStore = prefs.store;
  const recStore = recording?.store ?? readable<RecordingUiState | null>(null);
  // While a take runs, play and stop belong to it.
  const taking = $derived(takeRunning($recStore?.phase ?? null));
  const playerStore = memos?.player ?? readable(null);
  const memoAudible = $derived(
    $playerStore?.status === 'loading' || $playerStore?.status === 'playing',
  );

  function stop() {
    if (taking && recording) void recording.stop();
    else {
      // Stopping a memo also stops the song it started.
      memos?.stopPlayback();
      if (get(rt).started) runtime.stop();
    }
  }
</script>

{#snippet readout(value: string, label: string, title?: string)}
  <span class="readout" {title}>
    <span class="value mono">{value}</span>
    <span class="label">{label}</span>
  </span>
{/snippet}

<header>
  <Button
    variant="primary"
    onclick={() => app.evaluateDoc()}
    disabled={taking}
    title={taking ? 'recording a memo' : undefined}
  >
    {$rt.started ? 're-evaluate' : 'play'}
  </Button>
  <Button onclick={stop} disabled={!$rt.started && !taking && !memoAudible}>stop</Button>
  <label class="audition">
    <input
      type="checkbox"
      checked={$prefsStore.auditionNotes}
      onchange={(event) => void prefs.setAuditionNotes(event.currentTarget.checked)}
    />
    Audition notes
  </label>
  <RecordControls {recording} {session} {memos} />
  {@render readout($rt.cps.toFixed(3), 'cps')}
  {@render readout(
    String(Math.round(bpmFromCps($rt.cps, $rt.meter))),
    'bpm',
    'one cycle is one bar; bpm counts quarter notes, so 6/8 at 120 has 3 per bar',
  )}
  {@render readout(meterString($rt.meter), 'meter', 'meter')}
  <span class="label out" class:live={$rt.started}>out</span>
  <Scope size="md" running={$rt.started} />
  <span class="hint backend">backend: {$rt.backend}</span>
</header>

<style>
  header {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    height: var(--toolbar-h);
    box-sizing: border-box;
    padding: 0 var(--space-3);
    background-color: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .readout {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1-5);
  }
  .audition {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1-5);
    font-size: var(--fs-ui);
  }
  .audition input {
    margin: 0;
    accent-color: var(--accent);
  }
  .value {
    display: inline-flex;
    align-items: center;
    height: var(--control-md);
    padding: 0 var(--space-3);
    border-radius: var(--radius-pill);
    background-color: var(--bg);
    font-size: var(--fs-ui);
  }
  .out.live {
    color: var(--accent);
  }
  /* The controls keep their size; on a narrow window the backend hint truncates instead. */
  header > :global(*) {
    flex-shrink: 0;
  }
  .backend {
    flex-shrink: 1;
    min-width: 0;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

<script lang="ts">
  import type { App } from '../lib/app';
  import {
    bpmFromCps,
    cpsAtMeter,
    meterAtBar,
    meterString,
    type Meter,
  } from '../lib/strudel/structure';
  import Button from './ui/Button.svelte';
  import Scope from './ui/Scope.svelte';
  import RecordControls from './RecordControls.svelte';

  let { app }: { app: App } = $props();
  // `app` is constant for the window's lifetime, so reading it once is correct.
  // svelte-ignore state_referenced_locally
  const { runtime, recording, session, scopeClock, prefs: preferences, arbiter } = app;
  const rt = runtime.store;
  const prefs = preferences.store;
  const sessionState = session.store;
  const play = arbiter.store;
  // While a take runs, play and stop belong to it.
  const taking = $derived($play.taking);
  let playingMeter = $state.raw<Meter | null>(null);
  $effect(() => {
    const structure = $rt.structure;
    if (!$rt.started || !structure?.meterChanges) {
      playingMeter = null;
      return;
    }
    return scopeClock.subscribe(() => {
      const { located } = runtime.position();
      playingMeter = located ? meterAtBar(structure, located.songBar) : null;
    });
  });
  const meter = $derived(playingMeter ?? $rt.meter);
  const bpm = $derived(bpmFromCps($rt.cps, $rt.meter));
  const stopHelp = $derived.by(() => {
    if ($play.stopTarget === 'take') {
      if ($play.takePhase === 'starting') return 'Wait for the recording to start.';
      if ($play.takePhase === 'stopping') return 'The memo is already being saved.';
      return 'End the recording and save it in this project.';
    }
    if ($play.stopTarget === 'memo') return 'Stop the memo and any song or metronome backing.';
    if ($rt.started) return 'Stop song playback.';
    return 'Playback is already stopped.';
  });

  function stop() {
    void arbiter.stop();
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
    onclick={() => arbiter.play()}
    disabled={taking}
    help={taking
      ? 'Finish the recording before evaluating the song.'
      : $sessionState.active === null
        ? 'Open a song to start playback.'
        : $rt.started
          ? 'Save edits and apply the open song’s code to playback.'
          : 'Save edits and start playback from the open song’s code.'}
  >
    {$rt.started ? 're-evaluate' : 'play'}
  </Button>
  <Button onclick={stop} disabled={$play.stopTarget === null} help={stopHelp}>stop</Button>
  <Button
    data-piano-roll
    aria-controls="piano-roll-dock"
    aria-expanded={$prefs.pianoRollOpen && $sessionState.active !== null}
    disabled={$sessionState.active === null}
    help={$sessionState.active === null
      ? 'Open a song to inspect its notes.'
      : $prefs.pianoRollOpen
        ? 'Hide the note grid and give the code editor more room.'
        : 'Show notes for the pattern you’re working on.'}
    onclick={(event) => {
      if ($prefs.pianoRollOpen) event.currentTarget.focus();
      void app.prefs.setPianoRollOpen(!$prefs.pianoRollOpen);
    }}>Piano Roll</Button
  >
  <RecordControls {recording} {session} {arbiter} />
  {@render readout(cpsAtMeter($rt.cps, $rt.meter, meter).toFixed(3), 'cps')}
  {@render readout(
    String(Math.round(bpm)),
    'bpm',
    'one cycle is one bar; bpm counts quarter notes, so 6/8 at 120 has 3 per bar',
  )}
  {@render readout(meterString(meter), 'meter', 'current time signature')}
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

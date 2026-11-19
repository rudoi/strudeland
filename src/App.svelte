<script lang="ts">
  import { createApp } from './lib/app';
  import { matchShortcut } from './lib/project/ui';
  import ConflictBar from './components/ConflictBar.svelte';
  import Editor from './components/Editor.svelte';
  import NewSong from './components/NewSong.svelte';
  import SongList from './components/SongList.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Timeline from './components/Timeline.svelte';
  import Toolbar from './components/Toolbar.svelte';
  import Welcome from './components/Welcome.svelte';
  import { SCOPE_CLOCK_KEY } from './lib/scope/clock';
  import { onDestroy, onMount, setContext } from 'svelte';

  const app = createApp();
  // Every Scope reads the one shared clock; set before any child renders.
  setContext(SCOPE_CLOCK_KEY, app.scopeClock);
  const session = app.session;
  const store = session.store;
  const rt = app.runtime.store;
  // The song whose audio is playing: only while the runtime runs.
  const playingPath = $derived($rt.started ? ($store.evaluated?.path ?? null) : null);

  let newSongOpen = $state(false);
  const conflict = $derived($store.songs.find((s) => s.path === $store.active)?.conflict ?? null);

  // Reopen the last project.
  onMount(() => {
    void session.init();
  });

  // Drop the control listener when this instance goes away (HMR remounts the component), so the
  // old app does not answer requests too.
  onDestroy(() => app.destroy());
  import.meta.hot?.dispose(() => app.destroy());

  // Gesture rule: every pointerdown/keydown makes sure audio runs (a no-op when it does).
  function onGesture() {
    void app.runtime.resumeAudio();
  }

  // Window-level so they work with the editor hidden.
  function onKeydown(e: KeyboardEvent) {
    const shortcut = matchShortcut(e);
    if (!shortcut) return;
    e.preventDefault();
    if (shortcut === 'openProject') void session.openProject();
    else if ($store.project) newSongOpen = true;
  }
</script>

<svelte:window
  onpointerdowncapture={onGesture}
  onkeydowncapture={onGesture}
  onkeydown={onKeydown}
/>

<main>
  {#if $store.project}
    <SongList
      {session}
      state={$store}
      {playingPath}
      bind:newSongOpen
      memos={app.memos}
      recording={app.recording}
    />
  {/if}
  <div class="col">
    <Toolbar {app} />
    {#if conflict}<ConflictBar {session} {conflict} />{/if}
    {#if !$store.project}
      <Welcome {session} />
    {:else if $store.active === null}
      <section class="empty">
        <p>no songs yet</p>
        <NewSong {session} bind:open={newSongOpen} />
      </section>
    {/if}
    <Editor {app} hidden={$store.active === null} />
    {#if $store.active !== null}<Timeline {app} />{/if}
    <StatusBar
      runtime={app.runtime}
      {session}
      control={app.control}
      prefs={app.prefs}
      toggleVim={app.toggleVim}
      vimMode={app.vimMode}
      recording={app.recording}
      memos={app.memos}
    />
  </div>
</main>

<style>
  main {
    display: flex;
    flex-direction: row;
    height: 100%;
  }
  .col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
  }
  .empty p {
    margin: 0;
    color: var(--text-dim);
  }
</style>

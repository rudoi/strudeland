<script lang="ts">
  import { createApp } from './lib/app';
  import { createProjectActions } from './lib/project/actions';
  import { matchShortcut, primaryModifier } from './lib/project/ui';
  import { resetHelp, startHelp } from './lib/help/targets';
  import { exportApi } from './lib/export/api';
  import { createShell } from './lib/shell';
  import { menuPort, quitPort } from './lib/shell-ports';
  import ConflictBar from './components/ConflictBar.svelte';
  import Editor from './components/Editor.svelte';
  import ExportAudio from './components/ExportAudio.svelte';
  import InstrumentBrowser from './components/InstrumentBrowser.svelte';
  import NewProject from './components/NewProject.svelte';
  import NewSong from './components/NewSong.svelte';
  import PianoRoll from './components/PianoRoll.svelte';
  import Settings from './components/Settings.svelte';
  import SongList from './components/SongList.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Timeline from './components/Timeline.svelte';
  import Toolbar from './components/Toolbar.svelte';
  import Welcome from './components/Welcome.svelte';
  import { SCOPE_CLOCK_KEY } from './lib/scope/clock';
  import { onDestroy, onMount, setContext, tick } from 'svelte';

  const app = createApp();
  // Every Scope reads the one shared clock; set before any child renders.
  setContext(SCOPE_CLOCK_KEY, app.scopeClock);
  const session = app.session;
  const store = session.store;
  const projectActions = createProjectActions(session);
  const newProjectFolder = projectActions.folder;
  const shell = createShell({
    session,
    projectActions,
    exportApi,
    quit: quitPort,
    menu: menuPort,
    activeElement: () =>
      document.activeElement instanceof HTMLElement ? document.activeElement : null,
  });
  const shellStore = shell.store;
  const modal = $derived($shellStore.modal);
  const prefs = app.prefs.store;
  const rollOpen = $derived($prefs.pianoRollOpen && $store.active !== null);
  const projectRoot = $derived($store.project?.root);
  const activeSong = $derived($store.active);
  $effect(() => {
    // Retained controls can have a different meaning after switching the workspace view.
    void [projectRoot, activeSong, rollOpen];
    resetHelp();
  });
  const shortcutModifier = primaryModifier(navigator.platform);
  // The song whose audio is playing: only while the runtime runs.
  const pb = session.playback.store;
  const playingPath = $derived($pb.playing?.path ?? null);

  let newSongOpen = $state(false);
  const conflict = $derived($store.songs.find((s) => s.path === $store.active)?.conflict ?? null);

  function closeInstruments(applied = false) {
    const target = shell.close('instruments', applied);
    void tick().then(() => (target === 'editor' ? app.focusEditor() : target?.focus()));
  }

  async function closeSettings() {
    const opener = shell.close('settings');
    await tick();
    if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
  }

  onMount(() => {
    const stopHelp = startHelp(document);
    const stopShell = shell.start();
    return () => {
      stopShell();
      stopHelp();
      projectActions.destroy();
    };
  });

  // Drop the control listener when this instance goes away (HMR remounts the component), so the
  // old app does not answer requests too.
  const teardown = () => {
    shell.destroy();
    app.destroy();
  };
  onDestroy(teardown);
  import.meta.hot?.dispose(teardown);

  // Unlock the editor's preview graph on its own gestures. Other gestures resume song audio.
  function onGesture(event: Event) {
    if (
      event.target instanceof Element &&
      event.target.closest(
        '[data-settings], [data-note-audition-toggle], [data-instrument-browser], [data-piano-roll]',
      )
    )
      return;
    if (!app.prepareNoteAudition(event.target)) void app.runtime.resumeAudio();
  }

  // Capture app shortcuts before the editor can consume Ctrl-O or Ctrl-N in Vim mode.
  function onKeydown(e: KeyboardEvent) {
    onGesture(e);
    if (e.defaultPrevented) return;
    const shortcut = matchShortcut(e, shortcutModifier);
    if (!shortcut) return;
    e.preventDefault();
    e.stopPropagation();
    if (shell.shortcut(shortcut)) newSongOpen = true;
  }
</script>

<svelte:window onpointerdowncapture={onGesture} onkeydowncapture={onKeydown} />

<div class="shell">
  <main>
    {#if $store.project}
      <SongList
        oninstruments={shell.openInstruments}
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
        <Welcome
          {session}
          ready={$shellStore.ready}
          onnew={() => shell.projectAction('newProject')}
          onopen={() => shell.projectAction('openProject')}
          onrecent={shell.openRecent}
        />
      {:else if $store.active === null}
        <section class="empty">
          <p>no songs yet</p>
          <NewSong {session} bind:open={newSongOpen} />
        </section>
      {/if}
      <div class="editing" hidden={$store.active === null} class:roll-open={rollOpen}>
        <Editor {app} hidden={$store.active === null} />
        <div id="piano-roll-dock" hidden={!rollOpen}>
          {#if rollOpen}
            {#key $store.active}<PianoRoll {app} />{/key}
          {/if}
        </div>
      </div>
      {#if $store.active !== null}<Timeline {app} />{/if}
    </div>
  </main>
  <StatusBar
    runtime={app.runtime}
    {session}
    control={app.control}
    prefs={app.prefs}
    toggleVim={app.toggleVim}
    vimMode={app.vimMode}
    recording={app.recording}
    memos={app.memos}
    onsettings={shell.openSettings}
  />
</div>

{#if modal?.kind === 'settings'}
  <Settings prefs={app.prefs} onclose={closeSettings} />
{/if}

{#if $newProjectFolder !== null}
  <NewProject
    folder={$newProjectFolder}
    oncreate={projectActions.create}
    oncancel={projectActions.cancel}
  />
{/if}

{#if modal?.kind === 'instruments' && modal.root === $store.project?.root}
  <InstrumentBrowser
    {app}
    root={modal.root}
    selection={modal.selection}
    onclose={closeInstruments}
  />
{/if}

{#if modal?.kind === 'export'}
  <ExportAudio
    song={modal.song}
    controller={modal.controller}
    onclose={() => shell.close('export')}
  />
{/if}

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  main {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .editing:not([hidden]) {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .roll-open :global(.editor) {
    min-height: 4rem;
  }
  #piano-roll-dock:not([hidden]) {
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-height: 75%;
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

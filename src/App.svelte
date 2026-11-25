<script lang="ts">
  import { createApp } from './lib/app';
  import { listenForAppQuit, replyToAppQuit } from './lib/app-quit';
  import { createProjectActions } from './lib/project/actions';
  import { onProjectMenu, type ProjectAction, type ProjectMenuAction } from './lib/project/api';
  import { matchShortcut, primaryModifier } from './lib/project/ui';
  import { exportApi, setExportMenuAvailable } from './lib/export/api';
  import { captureExportSong, createExportDialog, type ExportSong } from './lib/export/controller';
  import ConflictBar from './components/ConflictBar.svelte';
  import Editor from './components/Editor.svelte';
  import ExportAudio from './components/ExportAudio.svelte';
  import InstrumentBrowser from './components/InstrumentBrowser.svelte';
  import NewProject from './components/NewProject.svelte';
  import NewSong from './components/NewSong.svelte';
  import PianoRoll from './components/PianoRoll.svelte';
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
  const rt = app.runtime.store;
  const prefs = app.prefs.store;
  const rollOpen = $derived($prefs.pianoRollOpen && $store.active !== null);
  const shortcutModifier = primaryModifier(navigator.platform);
  // The song whose audio is playing: only while the runtime runs.
  const pb = session.playback.store;
  const playingPath = $derived($rt.started ? ($pb.evaluated?.path ?? null) : null);

  let newSongOpen = $state(false);
  let instruments = $state.raw<{
    root: string;
    selection: ReturnType<typeof session.instrumentSelection>;
    opener: HTMLButtonElement;
  } | null>(null);
  function openInstruments(opener: HTMLButtonElement) {
    if ($store.project)
      instruments = { root: $store.project.root, selection: session.instrumentSelection(), opener };
  }
  function closeInstruments(applied = false) {
    const opener = instruments?.opener;
    instruments = null;
    void tick().then(() => (applied ? app.focusEditor() : opener?.focus()));
  }
  $effect(() => {
    if (instruments && instruments.root !== $store.project?.root) instruments = null;
  });
  let projectsReady = $state(false);
  let nativeMenuReady = $state(false);
  let quitting = false;
  let exportDialog = $state.raw<{
    song: ExportSong;
    controller: ReturnType<typeof createExportDialog>;
  } | null>(null);
  const conflict = $derived($store.songs.find((s) => s.path === $store.active)?.conflict ?? null);
  const exportAvailable = $derived(
    projectsReady && $store.project !== null && $store.active !== null,
  );

  $effect(() => {
    if (nativeMenuReady) void setExportMenuAvailable(exportAvailable).catch(console.error);
  });

  function runProjectAction(action: ProjectAction) {
    if (projectsReady && !quitting && !exportDialog && !instruments)
      void projectActions.run(action);
  }

  function onMenuAction(action: ProjectMenuAction) {
    if (action !== 'exportAudio') {
      runProjectAction(action);
    } else if (
      exportAvailable &&
      !quitting &&
      !instruments &&
      !exportDialog &&
      !projectActions.isBusy()
    ) {
      const song = captureExportSong($store, session.activeText());
      if (song) exportDialog = { song, controller: createExportDialog(song, exportApi) };
    }
  }

  async function onAppQuit(id: number) {
    if (quitting) {
      await replyToAppQuit(id, false);
      return;
    }
    quitting = true;
    const controller = exportDialog?.controller;
    let approved = false;
    try {
      if (!controller || (await controller.requestQuit())) {
        approved = await session.guardShutdown();
      }
    } finally {
      try {
        await replyToAppQuit(id, approved);
      } finally {
        controller?.finishQuit();
        quitting = false;
      }
    }
  }

  // Reopen the last project before allowing the menu or shortcut to switch it.
  onMount(() => {
    let destroyed = false;
    let unlisten: (() => void) | undefined;
    let unlistenQuit: (() => void) | undefined;
    void session
      .init(() => !quitting && (exportDialog?.controller.canClose() ?? true))
      .then(async () => {
        if (destroyed || !$store.available) return;
        projectsReady = true;
        const stop = await onProjectMenu(onMenuAction);
        if (destroyed) {
          stop();
          return;
        }
        unlisten = stop;
        nativeMenuReady = true;
        const stopQuit = await listenForAppQuit((id) => {
          if (!destroyed) void onAppQuit(id).catch(console.error);
        });
        if (destroyed) stopQuit();
        else unlistenQuit = stopQuit;
      })
      .catch(console.error);
    return () => {
      destroyed = true;
      projectActions.destroy();
      unlisten?.();
      unlistenQuit?.();
    };
  });

  // Drop the control listener when this instance goes away (HMR remounts the component), so the
  // old app does not answer requests too.
  onDestroy(() => app.destroy());
  import.meta.hot?.dispose(() => app.destroy());

  // Unlock the editor's preview graph on its own gestures. Other gestures resume song audio.
  function onGesture(event: Event) {
    if (
      event.target instanceof Element &&
      event.target.closest(
        '[data-note-audition-toggle], [data-instrument-browser], [data-piano-roll]',
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
    if (quitting || exportDialog || instruments) return;
    if (shortcut === 'openProject') runProjectAction(shortcut);
    else if ($store.project && $newProjectFolder === null) newSongOpen = true;
  }
</script>

<svelte:window onpointerdowncapture={onGesture} onkeydowncapture={onKeydown} />

<main>
  {#if $store.project}
    <SongList
      oninstruments={openInstruments}
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
        ready={projectsReady}
        onnew={() => runProjectAction('newProject')}
        onopen={() => runProjectAction('openProject')}
        onrecent={(path) => void projectActions.openRecent(path)}
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

{#if $newProjectFolder !== null}
  <NewProject
    folder={$newProjectFolder}
    oncreate={projectActions.create}
    oncancel={projectActions.cancel}
  />
{/if}

{#if instruments && instruments.root === $store.project?.root}
  <InstrumentBrowser
    {app}
    root={instruments.root}
    selection={instruments.selection}
    onclose={closeInstruments}
  />
{/if}

{#if exportDialog}
  <ExportAudio
    song={exportDialog.song}
    controller={exportDialog.controller}
    onclose={() => (exportDialog = null)}
  />
{/if}

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

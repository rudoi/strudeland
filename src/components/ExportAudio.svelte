<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import {
    loopMode,
    loopUnavailable,
    MODES,
    roleLabel,
    showMp3Warning,
    type createExportDialog,
    type ExportSong,
  } from '../lib/export/controller';
  import { helpTarget } from '../lib/help/targets';
  import { authoredLoop } from '../lib/strudel/structure';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Help from './ui/Help.svelte';
  import Notice from './ui/Notice.svelte';

  let {
    song,
    controller,
    onclose,
  }: {
    song: ExportSong;
    controller: ReturnType<typeof createExportDialog>;
    onclose: () => void;
  } = $props();

  const source = untrack(() => song);
  const store = untrack(() => controller.store);
  const options = $derived($store.options);
  const unavailable = $derived(loopUnavailable(source, options));
  const activity = $derived($store.activity);
  const quit = $derived($store.quit);
  const locked = $derived(activity.kind !== 'idle' || quit !== null);
  const lockedHelp = $derived(
    quit
      ? 'Finish the quit decision first.'
      : activity.kind === 'picking'
        ? 'Finish choosing the destination folder first.'
        : activity.kind === 'confirming'
          ? 'Answer the file replacement prompt first.'
          : 'Wait for the current export to finish.',
  );
  const modeHelp = {
    standard: 'Choose the file format for a single stereo mixdown.',
    stems: 'Choose how the audio is split into separate WAV files.',
    'intro-loop': 'Effects from the opening carry into the first pass of the marked loop.',
    'tagged-loop': 'Uses the marked loop in the song. Your player needs to read the loop marker.',
    'folded-loop': 'Starts at the first bar of the marked loop with fresh effects.',
    'one-shot': 'Choose a clip, a section, or a length for the main body of the sound.',
  } satisfies Record<(typeof MODES)[number]['id'], string>;
  const sections = source.structure?.sections ?? [];
  const loop = source.structure && authoredLoop(source.structure);
  const markedLoop =
    loop &&
    (loop.first === loop.last
      ? sections[loop.first]?.name
      : `${sections[loop.first]?.name} through ${sections[loop.last]?.name}`);
  const target = $derived($store.preview.kind === 'ready' ? $store.preview.target : null);
  const result = $derived($store.result);
  const files = $derived(
    result && 'files' in result ? result.files : Array.isArray(target) ? target : null,
  );
  const canExport = $derived(
    !locked && !unavailable && $store.preview.kind === 'ready' && !$store.result,
  );
  const rangeRequired = $derived(
    options.range === 'section'
      ? 'Choose a section to preview the destination.'
      : 'Enter a positive whole number of bars to preview the destination.',
  );
  const exportHelp = $derived(
    locked
      ? lockedHelp
      : unavailable
        ? 'This song marks no loop; mark one to export a loop preset.'
        : $store.preview.kind === 'loading'
          ? 'Wait for the destination check to finish.'
          : $store.preview.kind === 'error'
            ? 'Resolve the output preview error before exporting.'
            : $store.preview.kind === 'idle'
              ? rangeRequired
              : 'Render the captured editor source to the listed destination.',
  );
  let dialog: HTMLDialogElement;

  onMount(() => {
    const previous = document.activeElement;
    const element = dialog;
    element.showModal();
    void controller.refresh();
    return () => {
      controller.dispose();
      element.close();
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  });

  function dismiss() {
    if (controller.canClose()) onclose();
  }
</script>

{#snippet pathList(paths: string[])}
  <ul class="file-list">
    {#each paths as path (path)}
      <li class="path mono">{path}</li>
    {/each}
  </ul>
{/snippet}

<dialog
  bind:this={dialog}
  aria-labelledby="export-audio-title"
  oncancel={(event) => {
    event.preventDefault();
    if (quit) controller.answerQuit('cancel');
    else dismiss();
  }}
>
  <form onsubmit={(event) => (event.preventDefault(), void controller.start())}>
    <header class="title">
      <h2 id="export-audio-title">export audio</h2>
      <div class="source">
        <strong>{source.title}</strong>
        <span class="hint mono">{source.songPath}</span>
        {#if source.dirty}<span class="hint dirty">includes unsaved edits</span>{/if}
      </div>
    </header>

    <div class="body">
      <fieldset class="modes" disabled={locked}>
        <legend class="label">export mode</legend>
        {#each MODES as mode (mode.id)}
          <label class="mode pressable" class:selected={options.mode === mode.id}>
            <!-- svelte-ignore a11y_autofocus -->
            <input
              type="radio"
              name="export-mode"
              checked={options.mode === mode.id}
              aria-labelledby={'mode-' + mode.id}
              aria-describedby={'detail-' + mode.id}
              use:helpTarget={locked ? lockedHelp : modeHelp[mode.id]}
              autofocus={mode.id === 'standard'}
              onchange={() => controller.select({ mode: mode.id })}
            />
            <span id={'mode-' + mode.id} class="mode-name">{mode.label}</span>
            <span id={'detail-' + mode.id} class="hint mode-detail">{mode.detail}</span>
          </label>
        {/each}
      </fieldset>

      <div class="details">
        <div class="settings">
          {#if options.mode === 'stems'}
            <label class="select-label">
              <span class="label">group stems by</span>
              <select
                class="control"
                disabled={locked}
                use:helpTarget={locked
                  ? lockedHelp
                  : 'Split stems by mixer orbit, pattern voice label, or instrument sound.'}
                value={options.grouping}
                onchange={(event) =>
                  controller.select({
                    grouping: event.currentTarget.value as typeof options.grouping,
                  })}
              >
                <option value="orbit">orbit</option>
                <option value="voice">voice label</option>
                <option value="instrument">instrument</option>
              </select>
            </label>
          {:else}
            <label class="select-label">
              <span class="label">format</span>
              <select
                class="control"
                disabled={locked}
                use:helpTarget={locked
                  ? lockedHelp
                  : 'Use WAV for lossless audio and sample-exact loops, or choose a compressed format.'}
                value={options.format}
                onchange={(event) =>
                  controller.select({ format: event.currentTarget.value as typeof options.format })}
              >
                <option value="wav">WAV</option>
                <option value="mp3">MP3</option>
                <option value="ogg">Ogg Vorbis</option>
                <option value="opus">Opus</option>
              </select>
            </label>
          {/if}
          {#if !loopMode(options.mode)}
            <label class="select-label">
              <span class="label">range</span>
              <select
                class="control"
                disabled={locked}
                use:helpTarget={locked
                  ? lockedHelp
                  : 'Render the arranged clip, one named section, or a number of bars from the start.'}
                value={options.range}
                onchange={(event) =>
                  controller.select({ range: event.currentTarget.value as typeof options.range })}
              >
                <option value="clip">clip</option>
                {#if sections.length > 0}<option value="section">named section</option>{/if}
                <option value="length">length in bars</option>
              </select>
            </label>
          {/if}
          {#if loopMode(options.mode)}
            <div class="range-hint">
              <span class="label">range</span>
              <div>{markedLoop || 'marked loop'}</div>
            </div>
          {:else if options.range === 'section'}
            <label class="select-label span-settings">
              <span class="label">section</span>
              <select
                class="control"
                disabled={locked}
                use:helpTarget={locked ? lockedHelp : 'Use this section’s position and length.'}
                value={options.section}
                onchange={(event) => controller.select({ section: event.currentTarget.value })}
              >
                {#each sections as section (section.name)}
                  <option value={section.name}>{section.name}</option>
                {/each}
              </select>
            </label>
          {:else if options.range === 'length'}
            <div class="span-settings">
              <Field
                label="bars"
                layout="stacked"
                width="fill"
                disabled={locked}
                help={locked
                  ? lockedHelp
                  : $store.preview.kind === 'idle'
                    ? rangeRequired
                    : 'Set how many bars to render from the start of the song.'}
                value={options.bars}
                inputmode="numeric"
                oninput={(event) => controller.select({ bars: event.currentTarget.value })}
              />
            </div>
          {/if}
        </div>

        {#if options.mode === 'stems'}
          <p class="hint">
            stereo 32-bit float WAV at 44.1 kHz. import the stems at the same position and leave
            their gains at unity to recreate the pre-master mix. peaks can exceed full scale.
          </p>
          {#if options.grouping === 'voice'}
            <p class="hint">
              each <code>.label(…)</code> gets a stem. parts without a label share one stem.
            </p>
          {/if}
          {#if options.grouping !== 'orbit'}
            <p class="hint">
              each orbit has a shared effects stem for delay, reverb and filter changes. a filter
              can subtract part of the direct signal on this stem.
            </p>
          {/if}
        {/if}
        {#if unavailable}
          <Notice tone="warning"
            >loop export needs a <code>meta.loop</code> range or a section with
            <code>loop: true</code></Notice
          >
        {/if}
        {#if showMp3Warning(options) && !$store.result}
          <Notice tone="warning"
            >MP3 encoder delay and padding can cause gaps. use WAV for sample-exact loops.</Notice
          >
        {/if}

        <section class="destination" aria-labelledby="export-destination">
          <h3 class="label" id="export-destination">destination folder</h3>
          <div class="destination-row">
            <span class="path mono">{options.folder ?? 'project renders/'}</span>
            <Button
              size="sm"
              disabled={locked}
              onclick={controller.pickFolder}
              help={locked ? lockedHelp : 'Choose another folder for the rendered files.'}
              >choose…</Button
            >
          </div>
          {#if options.folder !== null}
            <Button
              size="sm"
              variant="quiet"
              disabled={locked}
              onclick={() => controller.select({ folder: null })}
              help={locked ? lockedHelp : 'Write the output under this project’s renders/ folder.'}
            >
              use project folder
            </Button>
          {/if}
        </section>

        <section class="files" aria-labelledby="export-files">
          <h3 class="label" id="export-files">
            {result
              ? 'written files'
              : options.mode === 'stems'
                ? 'stem set folder'
                : 'output files'}
          </h3>
          {#if result && 'paths' in result}
            {@render pathList(result.paths)}
          {:else if target && 'directory' in target}
            <span class="path mono">{target.directory}</span>
          {:else if files}
            <ul class="file-list">
              {#each files as file (file.path)}
                <li>
                  <strong class="file-role">{roleLabel(file.role)}</strong>
                  <span class="path mono">{file.path}</span>
                  {#if 'godotLoopOffset' in file && file.godotLoopOffset !== null}
                    <span class="hint">
                      Godot loop offset <span class="mono">{file.godotLoopOffset} s</span>
                    </span>
                  {:else if 'exists' in file && file.exists}
                    <span class="hint replaces">file exists</span>
                  {/if}
                </li>
              {/each}
            </ul>
          {:else if $store.preview.kind === 'error'}
            <Notice tone="danger">{$store.preview.message}</Notice>
            <Button
              size="sm"
              disabled={locked}
              onclick={controller.refresh}
              help={locked ? lockedHelp : 'Check the output paths again.'}>refresh preview</Button
            >
          {:else if $store.preview.kind === 'loading'}
            <p class="hint" role="status">checking destination…</p>
          {:else}
            <p class="hint">{rangeRequired}</p>
          {/if}
        </section>

        {#if $store.failure}
          <Notice tone="danger">{$store.failure.message}</Notice>
          {#if $store.failure.written.length}
            <div>
              <p class="label">files already written</p>
              {@render pathList($store.failure.written)}
            </div>
          {/if}
        {/if}
        {#if $store.result}
          {#each $store.result.warnings as warning (warning)}
            <Notice tone="warning">{warning}</Notice>
          {/each}
        {/if}
      </div>
    </div>

    <footer>
      <div role="status">
        {#if activity.kind === 'rendering'}
          <label class="progress">
            <span>rendering {activity.completed} of {activity.total} chunks</span>
            <progress value={activity.completed} max={activity.total}></progress>
          </label>
        {:else if activity.kind === 'confirming'}
          <span>confirm file replacement…</span>
        {:else if activity.kind === 'planning' || activity.kind === 'encoding' || activity.kind === 'writing'}
          <label class="progress">
            <span>{activity.kind}…</span>
            <progress></progress>
          </label>
        {:else if $store.result}
          <span class="complete">export complete</span>
        {:else if $store.cancelled}
          <span>export cancelled</span>
        {/if}
      </div>
      <div class="actions">
        {#if quit === 'ask'}
          <Button
            onclick={() => controller.answerQuit('cancel')}
            help="Keep the app open and let the export finish.">cancel quit</Button
          >
          <Button
            variant="primary"
            onclick={() => controller.answerQuit('wait')}
            help="See the export result before deciding whether to quit."
          >
            wait for export
          </Button>
        {:else if quit === 'waiting'}
          <Button
            onclick={() => controller.answerQuit('cancel')}
            help="Keep the app open and let the export finish.">cancel quit</Button
          >
        {:else if quit === 'result'}
          <Button
            onclick={() => controller.answerQuit('cancel')}
            help="Acknowledge the export result and cancel quitting.">keep app open</Button
          >
          <Button
            variant="primary"
            onclick={() => controller.answerQuit('continue')}
            help="Acknowledge the export result and continue to the unsaved changes check."
          >
            continue quitting
          </Button>
        {:else if quit === 'settling'}
          <Button disabled help="The app is finishing your quit decision.">please wait</Button>
        {:else if $store.result}
          <Button variant="primary" onclick={dismiss} help="Close the export results.">done</Button>
        {:else}
          <Button
            disabled={locked}
            onclick={dismiss}
            help={locked ? lockedHelp : 'Return to the editor.'}>close</Button
          >
          <Button variant="primary" type="submit" disabled={!canExport} help={exportHelp}
            >export audio</Button
          >
        {/if}
      </div>
    </footer>
    <Help />
  </form>
</dialog>

<style>
  dialog {
    box-sizing: border-box;
    width: min(760px, calc(100% - 2 * var(--space-4)));
    max-height: calc(100% - 2 * var(--space-4));
    padding: 0;
    border: 0;
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  dialog::backdrop {
    background: var(--modal-backdrop);
  }
  form {
    display: flex;
    flex-direction: column;
    max-height: calc(100dvh - 2 * var(--space-4));
  }
  h2,
  h3,
  p {
    margin: 0;
  }
  h2 {
    font-size: var(--fs-md);
  }
  h3.label,
  legend {
    color: var(--text-dim);
    font-weight: 500;
  }
  .title,
  footer {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3) var(--space-4);
  }
  .title {
    border-bottom: 1px solid var(--border);
  }
  .source {
    min-width: 0;
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-2);
    overflow-wrap: anywhere;
  }
  .dirty,
  .replaces {
    color: var(--warning);
  }
  .body {
    display: grid;
    grid-template-columns: minmax(13rem, 0.85fr) minmax(0, 1.15fr);
    gap: var(--space-4);
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-4);
  }
  fieldset {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0 0 var(--space-2);
  }
  .modes {
    align-self: start;
    display: grid;
    gap: var(--space-1);
  }
  .mode {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-0) var(--space-2);
    padding: var(--space-1-5) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
  }
  .mode.selected {
    background: var(--accent-wash);
    border-color: var(--accent-edge);
  }
  .mode input {
    align-self: center;
    margin: 0;
    accent-color: var(--accent);
  }
  .mode-name,
  .file-role {
    font-weight: 600;
  }
  .mode-detail {
    grid-column: 2;
    color: var(--text-dim);
  }
  .details {
    display: grid;
    align-content: start;
    gap: var(--space-3);
    min-width: 0;
  }
  .settings {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }
  .select-label,
  .range-hint {
    display: grid;
    gap: var(--space-1);
  }
  select {
    min-width: 0;
    background: var(--bg);
    color: var(--text);
  }
  .range-hint div {
    overflow-wrap: anywhere;
    padding: var(--space-1) 0;
  }
  .span-settings {
    grid-column: 1 / -1;
  }
  .destination,
  .files {
    display: grid;
    gap: var(--space-1-5);
    border-top: 1px solid var(--border);
    padding-top: var(--space-2);
    min-width: 0;
  }
  .destination-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .destination > :global(button),
  .files > :global(button) {
    justify-self: start;
  }
  .path {
    overflow-wrap: anywhere;
    font-size: var(--fs-sm);
  }
  .file-list {
    list-style: none;
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
  }
  .file-list li {
    display: grid;
    min-width: 0;
    gap: var(--space-0);
    padding: var(--space-1-5) var(--space-2);
    background: var(--well);
    border-radius: var(--radius-sm);
  }
  footer {
    border-top: 1px solid var(--border);
  }
  footer [role='status'] {
    flex: 1;
    color: var(--text-dim);
  }
  .progress {
    display: grid;
    justify-items: start;
    gap: var(--space-1);
  }
  progress {
    max-width: 100%;
    accent-color: var(--accent);
  }
  .complete {
    color: var(--accent);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    margin-left: auto;
  }
  @media (max-width: 540px) {
    .body {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>

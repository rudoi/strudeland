<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { exportApi } from '../lib/export/api';
  import {
    createExportDialog,
    loopMode,
    MODES,
    roleLabel,
    showMp3Warning,
    type ExportSong,
  } from '../lib/export/controller';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Notice from './ui/Notice.svelte';

  let { song, onclose }: { song: ExportSong; onclose: () => void } = $props();

  const source = untrack(() => song);
  const controller = createExportDialog(source, exportApi);
  const store = controller.store;
  const options = $derived($store.options);
  const activity = $derived($store.activity);
  const locked = $derived(activity.kind !== 'idle');
  const sections = $derived(source.structure?.sections ?? []);
  const markedLoop = $derived(sections.find((section) => section.loop));
  const canExport = $derived(!locked && $store.preview.kind === 'ready' && !$store.result);
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

<dialog
  bind:this={dialog}
  aria-labelledby="export-audio-title"
  oncancel={(event) => {
    event.preventDefault();
    dismiss();
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
          <label class="select-label">
            <span class="label">format</span>
            <select
              class="control"
              disabled={locked}
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
          {#if !loopMode(options.mode)}
            <label class="select-label">
              <span class="label">range</span>
              <select
                class="control"
                disabled={locked}
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
              <div>{markedLoop ? markedLoop.name : 'marked loop'}</div>
            </div>
          {:else if options.range === 'section'}
            <label class="select-label span-settings">
              <span class="label">section</span>
              <select
                class="control"
                disabled={locked}
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
                value={options.bars}
                inputmode="numeric"
                oninput={(event) => controller.select({ bars: event.currentTarget.value })}
              />
            </div>
          {/if}
        </div>

        {#if loopMode(options.mode) && !markedLoop}
          <Notice tone="warning"
            >loop export needs a section with <code>loop: true</code> in
            <code>meta.sections</code></Notice
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
            <Button size="sm" disabled={locked} onclick={controller.pickFolder}>choose…</Button>
          </div>
          {#if options.folder !== null}
            <Button
              size="sm"
              variant="quiet"
              disabled={locked}
              onclick={() => controller.select({ folder: null })}
            >
              use project folder
            </Button>
          {/if}
        </section>

        <section class="files" aria-labelledby="export-files">
          <h3 class="label" id="export-files">
            {$store.result ? 'written files' : 'output files'}
          </h3>
          {#if $store.result}
            <ul class="file-list">
              {#each $store.result.files as file (file.path)}
                <li>
                  <strong class="file-role">{roleLabel(file.role)}</strong>
                  <span class="path mono">{file.path}</span>
                  {#if file.godotLoopOffset !== null}
                    <span class="hint"
                      >Godot loop offset <span class="mono">{file.godotLoopOffset} s</span></span
                    >
                  {/if}
                </li>
              {/each}
            </ul>
          {:else if $store.preview.kind === 'ready'}
            <ul class="file-list">
              {#each $store.preview.files as file (file.path)}
                <li>
                  <strong class="file-role">{roleLabel(file.role)}</strong>
                  <span class="path mono">{file.path}</span>
                  {#if file.exists}<span class="hint replaces">file exists</span>{/if}
                </li>
              {/each}
            </ul>
          {:else if $store.preview.kind === 'error'}
            <Notice tone="danger">{$store.preview.message}</Notice>
            <Button size="sm" disabled={locked} onclick={controller.refresh}>refresh preview</Button
            >
          {:else if $store.preview.kind === 'loading'}
            <p class="hint" role="status">checking filenames…</p>
          {:else}
            <p class="hint">enter a positive whole number of bars to preview filenames</p>
          {/if}
        </section>

        {#if $store.failure}
          <Notice tone="danger">{$store.failure.message}</Notice>
          {#if $store.failure.written.length}
            <div>
              <p class="label">files already written</p>
              <ul class="file-list">
                {#each $store.failure.written as path (path)}
                  <li class="path mono">{path}</li>
                {/each}
              </ul>
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
        {:else if activity.kind === 'planning' || activity.kind === 'encoding' || activity.kind === 'writing'}
          <label class="progress">
            <span>{activity.kind}…</span>
            <progress></progress>
          </label>
        {:else if $store.result}
          <span class="complete">export complete</span>
        {/if}
      </div>
      <div class="actions">
        {#if $store.result}
          <Button variant="primary" onclick={dismiss}>done</Button>
        {:else}
          <Button disabled={locked} onclick={dismiss}>close</Button>
          <Button variant="primary" type="submit" disabled={!canExport}>export audio</Button>
        {/if}
      </div>
    </footer>
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

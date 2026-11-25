<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { App } from '../lib/app';
  import { errorMessage } from '../lib/errors';
  import { createInstrumentBrowser } from '../lib/project/instrument-browser';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Help from './ui/Help.svelte';
  import ListRow from './ui/ListRow.svelte';
  import Notice from './ui/Notice.svelte';

  let {
    app,
    root,
    selection,
    onclose,
  }: {
    app: App;
    root: string;
    selection: ReturnType<App['session']['instrumentSelection']>;
    onclose: (applied?: boolean) => void;
  } = $props();
  const browser = untrack(() =>
    createInstrumentBrowser(app.instruments, root, app.instrumentPreview.stop),
  );
  const catalog = browser.store;
  const { instrumentPreview, arbiter } = untrack(() => app);
  const audio = instrumentPreview.store;
  const play = arbiter.store;
  const playing = $derived($play.started || $play.memoAudible);
  const taking = $derived($play.taking);
  const busy = $derived(playing || taking);
  const busyHelp = $derived(
    playing && taking
      ? 'Stop playback and recording to hear a preview.'
      : taking
        ? 'Stop recording to hear a preview.'
        : 'Stop playback to hear a preview.',
  );
  const sounding = $derived($audio.status === 'playing' || $audio.status === 'loading');
  const list = $derived($catalog.lists[$catalog.source]);
  const version = $derived($catalog.version);
  const definition = $derived($catalog.definition);
  const global = $derived($catalog.lists.library?.entries.find((row) => row.id === version?.id));
  const pin = $derived($catalog.lists.project?.entries.find((row) => row.id === version?.id));
  const differs = $derived(global && pin && global.fingerprint !== pin.fingerprint);
  const inProject = $derived($catalog.source === 'project');
  const projectPins = $derived($catalog.lists.project);
  const definitionHelp = $derived(
    $catalog.readError
      ? 'Refresh the list to retry reading this instrument.'
      : 'Reading the instrument definition.',
  );
  const selectionHelp =
    'Select a complete pattern expression in the editor, then reopen Instruments.';
  const writingHelp = 'Wait for the project pin to finish saving.';
  let dialog: HTMLDialogElement;
  let rows: HTMLUListElement;
  let applying = $state(false);
  let applyError = $state('');
  let mounted = false;
  const short = (hash: string) => hash.slice(0, 8);

  onMount(() => {
    mounted = true;
    dialog.showModal();
    void browser.refresh();
    return () => {
      mounted = false;
      browser.close();
      dialog.close();
    };
  });

  function navigate(event: KeyboardEvent, index: number) {
    const count = $catalog.rows.length;
    if (!count) return;
    const next =
      event.key === 'ArrowDown'
        ? Math.min(index + 1, count - 1)
        : event.key === 'ArrowUp' && index >= 0
          ? Math.max(index - 1, 0)
          : event.key === 'Home' && index >= 0
            ? 0
            : event.key === 'End' && index >= 0
              ? count - 1
              : -1;
    if (next < 0) return;
    event.preventDefault();
    browser.show($catalog.source, $catalog.rows[next].id);
    rows.querySelectorAll('button')[next]?.focus();
  }

  async function apply() {
    if (!selection || !version || !inProject || applying) return;
    applying = true;
    applyError = '';
    try {
      await app.session.applyInstrument({
        ...selection,
        id: version.id,
        pinFingerprint: version.fingerprint,
      });
      if (mounted) onclose(true);
    } catch (e) {
      if (mounted) applyError = errorMessage(e);
    } finally {
      applying = false;
    }
  }

  function refresh() {
    applyError = '';
    void browser.refresh();
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="instruments-title"
  data-instrument-browser
  oncancel={(e) => {
    e.preventDefault();
    if (!applying) onclose();
  }}
>
  <header>
    <h2 id="instruments-title">instruments</h2>
    <Button
      variant="quiet"
      size="sm"
      disabled={applying}
      onclick={() => onclose()}
      help={applying
        ? 'Wait for the editor change to finish.'
        : 'Stop the preview and return to the workspace.'}>close</Button
    >
  </header>
  <div class="panes" inert={applying}>
    <div class="catalog">
      <div class="sources" role="group" aria-label="instrument source">
        <Button
          size="sm"
          pressed={!inProject}
          help="Browse versions available to pin to this project."
          onclick={() => {
            applyError = '';
            browser.show('library');
          }}>global library</Button
        >
        <Button
          size="sm"
          pressed={inProject}
          help="Browse the instrument versions this project uses."
          onclick={() => {
            applyError = '';
            browser.show('project');
          }}>project pins</Button
        >
      </div>
      <Field
        type="search"
        value={$catalog.search}
        oninput={(e) => {
          applyError = '';
          browser.filter(e.currentTarget.value);
        }}
        onkeydown={(e) => navigate(e, -1)}
        aria-label="search instruments"
        help={list
          ? 'Filter by instrument ID. Press Down Arrow to move to the first result.'
          : 'Instruments are loading. Enter part of an instrument ID to filter the list.'}
        placeholder="search instruments"
        width="fill"
        spellcheck="false"
        autofocus
      />
      <ul bind:this={rows} aria-label={inProject ? 'project pins' : 'global library'}>
        {#each $catalog.rows as row, index (row.id)}
          <li>
            <ListRow
              active={row.id === version?.id}
              tabindex={row.id === version?.id ? 0 : -1}
              help="Show this instrument’s definition. Use Up and Down Arrow to browse the list."
              onkeydown={(e) => {
                applyError = '';
                navigate(e, index);
              }}
              onclick={() => {
                applyError = '';
                browser.show($catalog.source, row.id);
              }}
              title={row.id}><span class="mono">{row.id}</span></ListRow
            >
          </li>
        {/each}
      </ul>
      {#if !list}<p class="subtle" role="status">loading instruments…</p>
      {:else if list.error}<Notice tone="danger">{list.error}</Notice>
      {:else if !$catalog.rows.length}
        <p class="subtle">
          {list.entries.length
            ? 'no matching instruments'
            : inProject
              ? 'no project pins yet'
              : 'the global library is empty'}
        </p>
      {/if}
      <div class="list-footer">
        <span class="hint"
          >{#if list && !list.error}{$catalog.rows.length}
            {$catalog.rows.length === 1 ? 'instrument' : 'instruments'}{/if}</span
        >
        <Button
          variant="quiet"
          size="sm"
          onclick={refresh}
          help="Reload the global library and project pins.">refresh</Button
        >
      </div>
    </div>
    <section class="inspector" aria-label="instrument details">
      {#if version}
        <div class="sheet">
          <h3 class="mono" title={version.id}>{version.id}</h3>
          <div class="audition">
            <Button
              variant="primary"
              disabled={!sounding && (busy || !definition)}
              aria-label={sounding ? 'stop preview' : 'preview'}
              help={sounding
                ? 'End this instrument’s preview.'
                : busy
                  ? busyHelp
                  : !definition
                    ? definitionHelp
                    : 'Hear a short example of the selected instrument version.'}
              onclick={() => {
                if (sounding) instrumentPreview.stop();
                else if (version) void instrumentPreview.play(version);
              }}
            >
              <span class="glyph" aria-hidden="true">{sounding ? '■' : '▶'}</span>{sounding
                ? 'stop'
                : 'preview'}
            </Button>
            <span class="version"
              >{inProject ? 'project pin' : 'global library'}
              <code title={version.fingerprint}>{short(version.fingerprint)}</code></span
            >
          </div>
          {#if $audio.status === 'loading'}<p class="subtle" role="status">preparing preview…</p>
          {:else if $audio.status === 'error'}<Notice tone="danger">{$audio.message}</Notice>
          {:else if busy}<p class="subtle">{busyHelp}</p>{/if}
          {#if $catalog.readError}
            <Notice tone="danger">{$catalog.readError}</Notice>
          {:else if definition}
            <h4>{definition.voices.length === 1 ? 'voice' : 'kit voices'}</h4>
            <dl class="voices">
              {#each definition.voices as voice (voice.name)}
                <div>
                  <dt>{voice.name}</dt>
                  <dd>
                    {#each voice.chain as step}<span class="step"
                        >{step.control}<span class="punct">(</span><span class="literal"
                          >{typeof step.value === 'object'
                            ? step.value.param
                            : JSON.stringify(step.value)}</span
                        ><span class="punct">)</span></span
                      >{/each}
                  </dd>
                </div>
              {/each}
            </dl>
            <h4>parameter defaults</h4>
            {#if Object.keys(definition.parameters).length}
              <dl class="defaults">
                {#each Object.entries(definition.parameters) as [name, value] (name)}
                  <div>
                    <dt>{name}</dt>
                    <dd>{JSON.stringify(value)}</dd>
                  </div>
                {/each}
              </dl>
            {:else}<p class="subtle">no parameters</p>{/if}
          {:else}<p class="subtle" role="status">reading instrument…</p>{/if}
        </div>
        <footer>
          {#if differs && global && pin}
            <div class="difference">
              project pin <code title={pin.fingerprint}>{short(pin.fingerprint)}</code> differs from
              library <code title={global.fingerprint}>{short(global.fingerprint)}</code>
            </div>
          {:else if inProject && $catalog.lists.library && !$catalog.lists.library.error && !global}
            <p class="subtle">this pin is only available in this project</p>
          {:else if !inProject && pin}<p class="subtle">
              this version is pinned to the project
            </p>{/if}
          <div class="actions">
            {#if !inProject && !pin}
              <Button
                disabled={!definition || $catalog.writing || !projectPins || !!projectPins.error}
                help={$catalog.writing
                  ? writingHelp
                  : !projectPins
                    ? 'Wait for the project pins to load.'
                    : projectPins.error
                      ? 'Refresh the list to retry reading project pins.'
                      : !definition
                        ? definitionHelp
                        : 'Copy this version into the project so songs can use it.'}
                onclick={browser.writePin}>pin to project</Button
              >
            {/if}
            {#if differs}
              <Button
                disabled={!definition || $catalog.writing}
                help={$catalog.writing
                  ? writingHelp
                  : !definition
                    ? definitionHelp
                    : 'Replace the project’s pinned version with the global library version.'}
                onclick={browser.writePin}>update project pin</Button
              >
            {/if}
            {#if !inProject && pin}
              <Button
                help="Inspect the version saved in this project."
                onclick={() => {
                  applyError = '';
                  browser.show('project', pin!.id);
                }}>view project pin</Button
              >
            {:else if inProject}
              <Button
                disabled={!selection || !definition || $catalog.writing}
                help={$catalog.writing
                  ? writingHelp
                  : !selection
                    ? selectionHelp
                    : !definition
                      ? definitionHelp
                      : 'Apply this pin to the selected pattern. The edit stays unsaved.'}
                onclick={apply}>apply to selection</Button
              >
            {/if}
          </div>
          {#if !inProject && $catalog.lists.project?.error}<Notice tone="danger"
              >could not read project pins: {$catalog.lists.project.error}</Notice
            >
          {:else if inProject && $catalog.lists.library?.error}<p class="subtle">
              library comparison unavailable
            </p>{/if}
          {#if inProject && !selection}<p class="subtle">{selectionHelp}</p>{/if}
          {#if $catalog.writing || applying}<p class="subtle" role="status">
              {applying ? 'applying to selection…' : 'saving project pin…'}
            </p>{/if}
          {#if $catalog.writeError || applyError}<Notice tone="danger"
              >{$catalog.writeError || applyError}</Notice
            >{/if}
        </footer>
      {:else}
        <div class="empty-detail">
          <p>
            {inProject && !list?.entries.length && !list?.error
              ? 'pin an instrument from the global library to use it in this project.'
              : 'choose an instrument to see its voices and hear a preview.'}
          </p>
        </div>
      {/if}
    </section>
  </div>
  <Help />
</dialog>

<style>
  dialog {
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
  dialog[open] {
    display: flex;
    flex-direction: column;
  }
  dialog::backdrop {
    background: var(--modal-backdrop);
  }
  header,
  .list-footer,
  .audition,
  .sources,
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  header,
  .list-footer {
    justify-content: space-between;
  }
  header {
    flex: none;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }
  h2,
  h3,
  h4,
  p,
  dl,
  dd,
  ul {
    margin: 0;
  }
  h2 {
    font-size: var(--fs-md);
  }
  .panes {
    display: grid;
    grid-template-columns: minmax(210px, 31%) minmax(0, 1fr);
    height: min(438px, 75vh);
    min-height: 0;
  }
  .catalog,
  .inspector {
    min-height: 0;
    min-width: 0;
  }
  .catalog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-3) var(--space-2);
    border-right: 1px solid var(--border);
    background: var(--bg);
    overflow: auto;
  }
  .sources {
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  ul {
    padding: var(--space-1);
    margin: 0 calc(-1 * var(--space-1));
    list-style: none;
    overflow: auto;
  }
  .list-footer {
    margin-top: auto;
    padding-top: var(--space-1);
  }
  .inspector {
    display: flex;
    flex-direction: column;
    overflow: auto;
  }
  .sheet {
    display: grid;
    align-content: start;
    gap: var(--space-2);
    padding: var(--space-5);
  }
  h3 {
    font-size: var(--fs-xl);
    font-weight: 500;
    line-height: 1.2;
    overflow-wrap: anywhere;
  }
  h4 {
    font-size: var(--fs-ui);
    font-weight: 500;
    margin-top: var(--space-3);
  }
  .audition {
    flex-wrap: wrap;
    gap: var(--space-3);
  }
  .audition :global(.btn) {
    min-width: 84px;
  }
  .glyph {
    font-size: var(--fs-xs);
  }
  .version,
  .subtle,
  .difference {
    font-size: var(--fs-sm);
    color: var(--text-dim);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .version code {
    margin-left: var(--space-1);
  }
  .voices,
  .defaults {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  .voices > div,
  .defaults > div {
    display: grid;
    grid-template-columns: minmax(60px, 30%) minmax(0, 1fr);
    gap: var(--space-2);
    padding: var(--space-1-5) var(--space-2);
    border-bottom: 1px solid var(--border);
  }
  .voices {
    background: var(--bg);
    border-radius: var(--radius-sm);
  }
  .voices > div:last-child,
  .defaults > div:last-child {
    border-bottom: 0;
  }
  .step {
    display: inline-block;
    max-width: 100%;
    margin-right: var(--space-1-5);
    color: var(--syntax-fn);
  }
  .punct {
    color: var(--syntax-punct);
  }
  .literal,
  .defaults dd {
    color: var(--syntax-string);
  }
  .defaults dd {
    text-align: right;
  }
  footer {
    margin-top: auto;
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--border);
  }
  .actions {
    flex-wrap: wrap;
  }
  .difference {
    color: var(--warning);
  }
  .empty-detail {
    padding: var(--space-5);
    margin: auto;
    color: var(--text-dim);
    max-width: 34ch;
    text-align: center;
  }
  @media (max-width: 600px) {
    .panes {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: max-content max-content;
      height: auto;
      overflow-y: auto;
    }
    .catalog {
      border-right: 0;
      border-bottom: 1px solid var(--border);
      gap: var(--space-2);
    }
    .catalog,
    .inspector {
      overflow: visible;
    }
    ul {
      max-height: 100px;
    }
    .sheet,
    footer {
      padding: var(--space-4);
    }
  }
</style>

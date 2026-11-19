<script lang="ts">
  import { tick } from 'svelte';
  import { readable } from 'svelte/store';
  import type { PlayHow } from '../lib/audio/memo-player';
  import { errorMessage } from '../lib/errors';
  import { songStem } from '../lib/project/ui';
  import type { RecordingController } from '../lib/recording/controller';
  import {
    formatDuration,
    memoChips,
    memoPlayItems,
    memoRow,
    type SongTempo,
  } from '../lib/recording/memo-ui';
  import type { Memos } from '../lib/recording/memos';
  import type { RecordingEntry, RecordingUiState } from '../lib/recording/types';
  import Button from './ui/Button.svelte';
  import Chip from './ui/Chip.svelte';
  import Dot from './ui/Dot.svelte';
  import Field from './ui/Field.svelte';
  import ListRow from './ui/ListRow.svelte';
  import Menu from './ui/Menu.svelte';
  import MenuItem from './ui/MenuItem.svelte';
  import Notice from './ui/Notice.svelte';

  let {
    memos,
    recording,
    song,
  }: {
    memos: Memos;
    recording: RecordingController | undefined;
    /** The open song's tempo; null without a song. */
    song: SongTempo | null;
  } = $props();

  // Both are constant for the window's lifetime, so reading them once is correct.
  // svelte-ignore state_referenced_locally
  const ms = memos.store;
  // svelte-ignore state_referenced_locally
  const player = memos.player;
  // svelte-ignore state_referenced_locally
  const recStore = recording?.store ?? readable<RecordingUiState | null>(null);
  const phase = $derived($recStore?.phase ?? null);

  /** The row whose menu is open. */
  let menuFor = $state<string | null>(null);
  /** The row being renamed, its field text and error. */
  let renaming = $state<string | null>(null);
  let renameText = $state('');
  let renameError = $state<string | null>(null);
  let renameInput = $state<HTMLInputElement>();
  /** The row asking to confirm a delete. */
  let deleting = $state<string | null>(null);
  let confirmBox = $state<HTMLElement>();
  /** A failed delete's message, shown under its row until the next interaction. */
  let deleteError = $state<{ stem: string; message: string } | null>(null);

  function openMenu(e: MouseEvent, stem: string) {
    e.stopPropagation();
    deleteError = null;
    menuFor = menuFor === stem ? null : stem;
    if (menuFor === null) return;
    const anchor = e.currentTarget as HTMLElement;
    void tick().then(() =>
      anchor.closest('li')?.querySelector('[role="menu"]')?.scrollIntoView({ block: 'nearest' }),
    );
  }

  function clickRow(m: RecordingEntry) {
    deleteError = null;
    if (memoRow(m.stem, $player, phase).action === 'stop') memos.stopPlayback();
    else void memos.play(m.stem, 'alone');
  }

  function playWith(stem: string, how: PlayHow) {
    menuFor = null;
    void memos.play(stem, how);
  }

  async function startRename(stem: string) {
    menuFor = null;
    deleting = null;
    renaming = stem;
    renameText = stem;
    renameError = null;
    await tick();
    renameInput?.focus();
    renameInput?.select();
  }

  function endRename() {
    renaming = null;
    renameError = null;
  }

  async function commitRename(stem: string) {
    const next = songStem(renameText);
    if (!next || next === stem) return endRename();
    try {
      await memos.rename(stem, next);
      endRename();
    } catch (e) {
      renameError = errorMessage(e);
    }
  }

  function onRenameKey(e: KeyboardEvent, stem: string) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void commitRename(stem);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      endRename();
    }
  }

  async function startDelete(stem: string) {
    menuFor = null;
    renaming = null;
    deleting = stem;
    await tick();
    // The first button is cancel.
    confirmBox?.querySelector('button')?.focus();
  }

  async function confirmDelete(stem: string) {
    deleting = null;
    try {
      await memos.remove(stem);
    } catch (e) {
      deleteError = { stem, message: errorMessage(e) };
    }
  }

  function onRowKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && deleting !== null) {
      e.preventDefault();
      deleting = null;
    }
  }
</script>

<svelte:window onclick={() => (deleteError = null)} />

<section class="memos">
  <div class="label head">memos</div>
  {#if $ms.loaded && $ms.items.length === 0}
    <p class="hint empty">no memos yet</p>
  {:else}
    <ul>
      {#each $ms.items as m (m.stem)}
        {@const row = memoRow(m.stem, $player, phase)}
        {@const chips = memoChips(m)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li onkeydown={onRowKey}>
          {#if renaming === m.stem}
            <div class="edit">
              <Field
                value={renameText}
                size="sm"
                width="fill"
                bind:el={renameInput}
                aria-label="rename memo"
                spellcheck="false"
                oninput={(e) => {
                  renameText = e.currentTarget.value;
                  renameError = null;
                }}
                onkeydown={(e) => onRenameKey(e, m.stem)}
                onblur={endRename}
              />
              {#if renameError}<Notice tone="danger">{renameError}</Notice>{/if}
            </div>
          {:else if deleting === m.stem}
            <div class="line confirm" bind:this={confirmBox}>
              <span class="ask" title={m.stem}>delete {m.stem}?</span>
              <Button size="sm" onclick={() => (deleting = null)}>cancel</Button>
              <Button size="sm" tone="danger" onclick={() => void confirmDelete(m.stem)}>
                delete
              </Button>
            </div>
          {:else}
            <div class="line">
              <ListRow
                active={row.active}
                disabled={row.disabled}
                title={row.title}
                onclick={() => clickRow(m)}
              >
                {m.stem}
                {#snippet trailing()}
                  <span class="hint mono">{formatDuration(m.duration)}</span>
                  {#if row.active}<Dot tone="accent" size="sm" />{/if}
                {/snippet}
              </ListRow>
              <Button
                variant="quiet"
                size="sm"
                aria-label="memo actions"
                aria-haspopup="menu"
                aria-expanded={menuFor === m.stem}
                onclick={(e) => openMenu(e, m.stem)}
              >
                …
              </Button>
            </div>
            {#if chips.length > 0}
              <div class="chips">
                {#each chips as c (c.id)}
                  <Chip tone={c.tone} title={c.title}>{c.text}</Chip>
                {/each}
              </div>
            {/if}
            <Menu open={menuFor === m.stem} onclose={() => (menuFor = null)}>
              {#each memoPlayItems(m, song, phase) as item (item.how)}
                <MenuItem
                  disabled={item.disabled}
                  title={item.title}
                  onclick={() => playWith(m.stem, item.how)}
                >
                  {item.label}
                </MenuItem>
              {/each}
              <MenuItem onclick={() => void startRename(m.stem)}>rename</MenuItem>
              <MenuItem onclick={() => void startDelete(m.stem)}>delete</MenuItem>
            </Menu>
          {/if}
          {#if deleteError?.stem === m.stem}
            <Notice tone="danger">{deleteError.message}</Notice>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .memos {
    flex: none;
    max-height: 40%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-top: 1px solid var(--border);
  }
  .head {
    padding: var(--space-2) var(--space-3);
  }
  .empty {
    margin: 0;
    padding: 0 var(--space-3) var(--space-2);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: var(--space-1);
    min-height: 0;
    overflow-y: auto;
  }
  li {
    position: relative;
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
  }
  .line :global(.row) {
    flex: 1;
    min-width: 0;
  }
  .confirm {
    padding: var(--space-1) var(--space-2);
  }
  .ask {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: 0 var(--space-2) var(--space-1);
  }
</style>

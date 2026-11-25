<script lang="ts">
  import type { Session, SessionState } from '../lib/project/session';
  import type { RecordingController } from '../lib/recording/controller';
  import { songTempoOf } from '../lib/recording/memo-ui';
  import type { Memos } from '../lib/recording/memos';
  import MemoList from './MemoList.svelte';
  import NewSong from './NewSong.svelte';
  import Button from './ui/Button.svelte';
  import Chip from './ui/Chip.svelte';
  import Dot from './ui/Dot.svelte';
  import ListRow from './ui/ListRow.svelte';
  import Scope from './ui/Scope.svelte';

  let {
    session,
    oninstruments,
    state,
    playingPath,
    newSongOpen = $bindable(false),
    memos,
    recording,
  }: {
    session: Session;
    oninstruments: (opener: HTMLButtonElement) => void;
    state: SessionState;
    playingPath: string | null;
    newSongOpen?: boolean;
    /** Unset outside Tauri: no memo section. */
    memos?: Memos;
    recording?: RecordingController;
  } = $props();
  const song = $derived(
    state.active !== null && state.structure !== null ? songTempoOf(state.structure) : null,
  );
</script>

<aside class="grid-bg">
  <div class="project label" title={state.project?.root}>{state.project?.name}</div>
  <ul>
    {#each state.songs as song (song.path)}
      <li>
        <ListRow
          active={song.path === state.active}
          onclick={() => void session.switchSong(song.path)}
          help={song.path === state.active
            ? 'This song is open in the editor.'
            : 'Open this song in the editor.'}
          title={song.path}
        >
          {song.title}
          {#snippet trailing()}
            {#if song.path === playingPath}<Scope size="sm" running />{/if}
            {#if song.dirty}<Dot tone="warning" aria-hidden={false} title="unsaved changes" />{/if}
            {#if song.conflict}
              <Chip
                tone={song.conflict === 'deleted' ? 'danger' : 'warning'}
                title={song.conflict === 'deleted' ? 'deleted on disk' : 'changed on disk'}
              >
                {song.conflict === 'deleted' ? 'deleted' : 'changed'}
              </Chip>
            {/if}
          {/snippet}
        </ListRow>
      </li>
    {/each}
  </ul>
  <NewSong {session} bind:open={newSongOpen} showField={state.songs.length > 0} />
  <div class="instruments">
    <Button
      onclick={(e) => oninstruments(e.currentTarget)}
      aria-haspopup="dialog"
      help="Browse instruments, audition sounds, and manage project pins.">instruments</Button
    >
  </div>
  {#if memos}<MemoList {memos} {recording} {song} />{/if}
</aside>

<style>
  aside {
    width: 200px;
    flex: none;
    display: flex;
    flex-direction: column;
    background-color: var(--surface);
    border-right: 1px solid var(--border);
    overflow: hidden;
  }
  .instruments {
    padding: var(--space-2);
    border-top: 1px solid var(--border);
  }
  .project {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: var(--space-1);
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>

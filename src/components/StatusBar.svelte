<script lang="ts">
  import { onMount } from 'svelte';
  import { readable, type Readable } from 'svelte/store';
  import { engineNotices } from '../lib/audio/engine-notices';
  import type { ControlInfo } from '../lib/control/protocol';
  import { vimChip, type VimMode } from '../lib/editor/vim-mode';
  import { formatStrudel, getAppInfo, type AppInfo } from '../lib/info';
  import type { PrefsStore } from '../lib/prefs/store';
  import type { Session } from '../lib/project/session';
  import type { RecordingController } from '../lib/recording/controller';
  import type { Memos } from '../lib/recording/memos';
  import { recordingNotices } from '../lib/recording/memo-ui';
  import type { RecordingUiState } from '../lib/recording/types';
  import type { Runtime } from '../lib/strudel/runtime';
  import Button from './ui/Button.svelte';
  import Chip from './ui/Chip.svelte';
  import Link from './ui/Link.svelte';
  import MasterMeter from './ui/MasterMeter.svelte';
  import Notice from './ui/Notice.svelte';

  let {
    runtime,
    session,
    control,
    recording,
    memos,
    prefs,
    toggleVim,
    vimMode,
  }: {
    runtime: Runtime;
    session: Session;
    control: Readable<ControlInfo>;
    /** Unset until the recording controller is wired: no recording notices. */
    recording?: RecordingController;
    /** Unset with `recording`: no playback notices. */
    memos?: Memos;
    prefs: PrefsStore;
    /** Flips the vim pref and focuses the editor. */
    toggleVim: () => Promise<void>;
    vimMode: VimMode;
  } = $props();
  // `runtime` is constant for the window's lifetime, so reading it once is correct.
  // svelte-ignore state_referenced_locally
  const rt = runtime.store;
  // svelte-ignore state_referenced_locally
  const ss = session.store;
  // svelte-ignore state_referenced_locally
  const cs = control;
  // svelte-ignore state_referenced_locally
  const recStore = recording?.store ?? readable<RecordingUiState | null>(null);
  // svelte-ignore state_referenced_locally
  const playerStore = memos?.player ?? readable(null);
  // svelte-ignore state_referenced_locally
  const ps = prefs.store;
  // svelte-ignore state_referenced_locally
  const vimStore = vimMode.store;
  const chip = $derived(vimChip($ps.vim ? ($vimStore ?? { mode: 'normal' }) : null));
  const notices = $derived(recordingNotices($recStore, $playerStore));

  let info = $state<AppInfo | null>(null);
  onMount(async () => {
    info = await getAppInfo();
  });
</script>

<footer>
  <label class="audition" data-note-audition-toggle>
    <input
      type="checkbox"
      checked={$ps.auditionNotes}
      onchange={(event) => void prefs.setAuditionNotes(event.currentTarget.checked)}
    />
    Audition notes
  </label>
  {#if $rt.evalError}
    <Notice tone="danger">error: {$rt.evalError}</Notice>
  {:else if $rt.outputError}
    <Notice tone="danger">audio error: {$rt.outputError}</Notice>
  {/if}
  {#if $ss.error}
    <Notice tone="danger">{$ss.error}</Notice>
  {/if}
  {#if $ss.metaError}
    <Notice tone="danger">meta: {$ss.metaError}</Notice>
  {/if}
  {#if $ps.error}
    <Notice tone="danger">prefs: {$ps.error}</Notice>
  {/if}
  {#if $rt.tempoOverride}
    <Notice tone="warning">setcps in the code overrides meta.bpm</Notice>
  {/if}
  {#each engineNotices($rt.engineWarnings) as n (n.key)}
    <Notice tone="warning" title={n.detail}>{n.text}</Notice>
  {/each}
  {#if $ss.notice}
    <Notice tone="warning">Claude edited {$ss.notice.title}</Notice>
  {/if}
  {#if $cs.status === 'in-use'}
    <Notice tone="warning" title={$cs.path ?? ''}>control socket in use</Notice>
  {:else if $cs.status === 'disabled'}
    <Notice tone="warning">control disabled: {$cs.detail}</Notice>
  {/if}
  {#each notices as n (n.id)}
    {#if n.action}
      {@const action = n.action}
      <Notice tone={n.tone}>
        {n.text}
        {#snippet actions()}
          {#if action.kind === 'link'}
            <Link href={action.href}>{action.label}</Link>
          {:else if action.kind === 'dismiss-playback'}
            <Button size="sm" variant="quiet" onclick={() => memos?.stopPlayback()}>dismiss</Button>
          {:else}
            <Button size="sm" variant="quiet" onclick={() => recording?.clearError()}>
              dismiss
            </Button>
          {/if}
        {/snippet}
      </Notice>
    {:else}
      <Notice tone={n.tone}>{n.text}</Notice>
    {/if}
  {/each}
  {#if $rt.audio === 'suspended'}
    <Button size="sm" onclick={() => void runtime.resumeAudio()}>
      audio suspended: click to enable
    </Button>
  {/if}
  <div class="editor-prefs">
    {#if $ss.available}
      <button
        class="pressable"
        type="button"
        aria-pressed={$ps.formatOnSave}
        title="Lay out Strudel chains, song sections, and dense mini-notation when saving"
        onclick={() => void prefs.setFormatOnSave(!$ps.formatOnSave)}
      >
        <Chip lit={$ps.formatOnSave}>Format on Save</Chip>
      </button>
    {/if}
    <button
      class="pressable"
      type="button"
      aria-pressed={$ps.vim}
      title="vim keybindings"
      onclick={() => void toggleVim()}
    >
      <Chip tone={chip.tone} lit={chip.lit}>{chip.label}</Chip>
    </button>
  </div>
  <span class="hint">
    {#if info}
      <span title={formatStrudel(info.strudel)}>strudeland {info.version}</span>
      ·
    {/if}
    powered by
    <Link href="https://strudel.cc">Strudel</Link>
    (AGPL-3.0-or-later,
    <Link href="https://codeberg.org/uzu/strudel">source</Link>)
  </span>
  <MasterMeter
    drain={runtime.drainLevels}
    limiter={$rt.limiter}
    available={$rt.limiterAvailable}
    onlimiter={runtime.setLimiter}
  />
</footer>

<style>
  footer {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    min-height: var(--statusbar-h);
    padding: var(--space-2) var(--space-3);
    background-color: var(--surface);
    border-top: 1px solid var(--border);
  }
  .audition {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    gap: var(--space-1-5);
    font-size: var(--fs-ui);
  }
  .audition input {
    margin: 0;
    accent-color: var(--accent);
  }
  .editor-prefs {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .editor-prefs button {
    padding: 0;
    border: none;
    border-radius: var(--radius-pill);
    background: none;
    color: inherit;
    font: inherit;
  }
</style>

<script lang="ts">
  import { readable, type Readable } from 'svelte/store';
  import { engineNotices } from '../lib/audio/engine-notices';
  import type { ControlInfo } from '../lib/control/protocol';
  import { vimChip, type VimMode } from '../lib/editor/vim-mode';
  import { helpTarget } from '../lib/help/targets';
  import type { PrefsStore } from '../lib/prefs/store';
  import type { Session } from '../lib/project/session';
  import type { RecordingController } from '../lib/recording/controller';
  import type { Memos } from '../lib/recording/memos';
  import { recordingNotices } from '../lib/recording/memo-ui';
  import type { RecordingUiState } from '../lib/recording/types';
  import type { Runtime } from '../lib/strudel/runtime';
  import Button from './ui/Button.svelte';
  import Chip from './ui/Chip.svelte';
  import Help from './ui/Help.svelte';
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
    onsettings,
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
    onsettings: (opener: HTMLButtonElement) => void;
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
</script>

<footer>
  <div class="notices">
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
              <Link href={action.href} help="Open the microphone permissions for this computer."
                >{action.label}</Link
              >
            {:else if action.kind === 'dismiss-playback'}
              <Button
                size="sm"
                variant="quiet"
                help="Dismiss this memo playback message."
                onclick={() => memos?.stopPlayback()}>dismiss</Button
              >
            {:else}
              <Button
                size="sm"
                variant="quiet"
                help="Dismiss this recording message."
                onclick={() => recording?.clearError()}
              >
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
      <Button
        size="sm"
        help="Resume the audio output so you can hear playback."
        onclick={() => void runtime.resumeAudio()}
      >
        audio suspended: click to enable
      </Button>
    {/if}
  </div>
  <div class="status-line">
    <div class="help-slot"><Help compact /></div>
    <div class="tools">
      <div class="editor-prefs">
        {#if $ss.available}
          <button
            class="pressable"
            type="button"
            aria-pressed={$ps.formatOnSave}
            use:helpTarget={$ps.formatOnSave
              ? 'Keep your existing layout when saving songs.'
              : 'Format chains, song sections and mini-notation when saving songs.'}
            onclick={() => void prefs.setFormatOnSave(!$ps.formatOnSave)}
          >
            <Chip lit={$ps.formatOnSave}>Format on Save</Chip>
          </button>
        {/if}
        <button
          class="pressable"
          type="button"
          aria-pressed={$ps.vim}
          use:helpTarget={$ps.vim
            ? 'Turn off Vim keybindings in the editor.'
            : 'Use Vim keybindings in the editor.'}
          onclick={() => void toggleVim()}
        >
          <Chip tone={chip.tone} lit={chip.lit}>{chip.label}</Chip>
        </button>
      </div>
      <MasterMeter
        drain={runtime.drainLevels}
        limiter={$rt.limiter}
        available={$rt.limiterAvailable}
        onlimiter={runtime.setLimiter}
      />
      <button
        type="button"
        class="settings pressable"
        data-settings
        aria-label="Settings"
        aria-haspopup="dialog"
        use:helpTarget={'Change typed-note previews and view credits and licenses.'}
        onclick={(event) => onsettings(event.currentTarget)}
      >
        <svg aria-hidden="true" width="18" height="18" viewBox="-12 -12 24 24" fill="currentColor">
          <path
            fill-rule="evenodd"
            d="M7.36,-2.25 9.85,-1.74 9.85,1.74 7.36,2.25 6.80,3.61 8.19,5.74 5.74,8.19 3.61,6.80 2.25,7.36 1.74,9.85 -1.74,9.85 -2.25,7.36 -3.61,6.80 -5.74,8.19 -8.19,5.74 -6.80,3.61 -7.36,2.25 -9.85,1.74 -9.85,-1.74 -7.36,-2.25 -6.80,-3.61 -8.19,-5.74 -5.74,-8.19 -3.61,-6.80 -2.25,-7.36 -1.74,-9.85 1.74,-9.85 2.25,-7.36 3.61,-6.80 5.74,-8.19 8.19,-5.74 6.80,-3.61ZM0,-3.3a3.3,3.3 0 1,0 0,6.6a3.3,3.3 0 1,0 0,-6.6Z"
          />
        </svg>
      </button>
    </div>
  </div>
</footer>

<style>
  footer {
    flex: none;
    padding: var(--space-1-5) var(--space-3);
    background-color: var(--surface);
    border-top: 1px solid var(--border);
  }
  .status-line,
  .tools,
  .notices,
  .editor-prefs {
    display: flex;
    align-items: center;
  }
  .status-line,
  .tools,
  .notices {
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
  }
  .help-slot {
    min-width: 0;
    /* Let Help take a whole row before the controls squeeze the message too tightly. */
    flex: 1 1 17rem;
  }
  .tools {
    justify-content: flex-end;
    margin-left: auto;
  }
  .notices {
    max-height: 30vh;
    overflow: auto;
    overflow-wrap: anywhere;
    padding-block: var(--space-1);
  }
  .notices:not(:has(> *)) {
    display: none;
  }
  .editor-prefs {
    gap: var(--space-2);
  }
  .settings {
    display: grid;
    place-items: center;
    flex: none;
    box-sizing: border-box;
    inline-size: var(--control-md);
    block-size: var(--control-md);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    color: var(--text);
    background: var(--surface-2);
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

<script lang="ts">
  import { onMount } from 'svelte';
  import license from '../../LICENSE?raw';
  import notices from '../../THIRD-PARTY-NOTICES.md?raw';
  import { version } from '../../package.json';
  import pins from '../../strudel-pins.json';
  import { helpTarget } from '../lib/help/targets';
  import { formatStrudel, getAppInfo, type AppInfo } from '../lib/info';
  import type { PrefsStore } from '../lib/prefs/store';
  import Button from './ui/Button.svelte';
  import Help from './ui/Help.svelte';
  import Link from './ui/Link.svelte';
  import Notice from './ui/Notice.svelte';

  let { prefs, onclose }: { prefs: PrefsStore; onclose: () => void } = $props();
  // svelte-ignore state_referenced_locally
  const ps = prefs.store;
  let dialog: HTMLDialogElement;
  let info = $state.raw<AppInfo | null>(null);
  const upstream = $derived(info?.strudel ?? pins.strudel);
  let licenseOpen = $state(false);
  let noticesOpen = $state(false);

  onMount(() => {
    const element = dialog;
    element.showModal();
    void getAppInfo().then((loaded) => (info = loaded));
    return () => element.close();
  });
</script>

<dialog
  data-settings
  bind:this={dialog}
  aria-labelledby="settings-title"
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
>
  <div class="body">
    <h2 id="settings-title">settings</h2>
    <label class="audition" data-note-audition-toggle>
      <input
        type="checkbox"
        checked={$ps.auditionNotes}
        use:helpTarget={$ps.auditionNotes
          ? 'Turn off previews for pitches you type.'
          : 'Preview pitches you type while playback is stopped.'}
        onchange={(event) => void prefs.setAuditionNotes(event.currentTarget.checked)}
      />
      Audition notes
    </label>
    {#if $ps.error}
      <Notice tone="danger">prefs: {$ps.error}</Notice>
    {/if}

    <section aria-labelledby="settings-about">
      <h3 id="settings-about">about &amp; licenses</h3>
      <p>
        Strudeland {info?.version ?? version} · powered by
        <Link href="https://strudel.cc" help="Visit the Strudel live coding project.">Strudel</Link>
      </p>
      <p>
        upstream
        <Link
          href={`${pins.strudel.repo}/src/commit/${upstream.commit}`}
          title={info ? formatStrudel(info.strudel) : upstream.commit}
          help="Browse the Strudel source used by this version.">{upstream.commit.slice(0, 7)}</Link
        >
        ({upstream.date.slice(0, 10)})
      </p>
      <p>© Strudeland contributors. Strudel © Strudel contributors.</p>
      <p>
        You can redistribute and modify Strudeland under the GNU Affero General Public License,
        version 3 or later (AGPL-3.0-or-later). It comes with no warranty.
      </p>
      <Link
        href="https://github.com/rudoi/strudeland"
        help="Browse the Strudeland source code on GitHub.">Strudeland source</Link
      >
      <details bind:open={licenseOpen}>
        <summary
          use:helpTarget={licenseOpen
            ? 'Hide the full license text.'
            : 'Read the full AGPL license offline.'}>license</summary
        >
        <pre>{license}</pre>
      </details>
      <details bind:open={noticesOpen}>
        <summary
          use:helpTarget={noticesOpen
            ? 'Hide the third-party notices.'
            : 'Read the credits and license notices for bundled code and assets offline.'}
          >third-party notices</summary
        >
        <pre>{notices}</pre>
      </details>
    </section>
  </div>
  <div class="bottom">
    <Help />
    <Button onclick={onclose} help="Return to the workspace. Your preferences apply immediately."
      >close</Button
    >
  </div>
</dialog>

<style>
  dialog {
    box-sizing: border-box;
    width: min(30rem, calc(100% - 2 * var(--space-4)));
    max-height: calc(100% - 2 * var(--space-4));
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-pop);
  }
  dialog[open] {
    display: flex;
    flex-direction: column;
  }
  dialog::backdrop {
    background: var(--modal-backdrop);
  }
  .body {
    overflow: auto;
    padding: var(--space-4);
  }
  h2,
  h3,
  p {
    margin: 0;
  }
  h2 {
    font-size: var(--fs-md);
  }
  h3 {
    font-size: var(--fs-ui);
  }
  .audition {
    display: inline-flex;
    gap: var(--space-1-5);
    align-items: center;
    margin-block: var(--space-3);
    font-size: var(--fs-ui);
  }
  .audition input {
    margin: 0;
    accent-color: var(--accent);
  }
  section {
    display: grid;
    justify-items: start;
    gap: var(--space-1-5);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }
  details {
    width: 100%;
    min-width: 0;
  }
  summary {
    width: fit-content;
    cursor: pointer;
  }
  pre {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  .bottom {
    flex: none;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--border);
  }
  .bottom :global(.help) {
    padding: 0;
    border: 0;
  }
</style>

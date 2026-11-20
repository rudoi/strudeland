<script lang="ts">
  import type { Session } from '../lib/project/session';
  import Button from './ui/Button.svelte';
  import ListRow from './ui/ListRow.svelte';
  import Notice from './ui/Notice.svelte';
  import Scope from './ui/Scope.svelte';

  let {
    session,
    ready,
    onnew,
    onopen,
    onrecent,
  }: {
    session: Session;
    ready: boolean;
    onnew: () => void;
    onopen: () => void;
    onrecent: (path: string) => void;
  } = $props();
  // `session` is constant for the window's lifetime.
  // svelte-ignore state_referenced_locally
  const store = session.store;
</script>

<section class="grid-bg">
  <h1 class="wordmark">
    strudeland
    <Scope size="sm" running={false} />
  </h1>
  {#if !$store.available}
    <p class="dim">projects need the desktop app.</p>
  {:else}
    {#if $store.error}<Notice tone="danger">{$store.error}</Notice>{/if}
    <div class="row">
      <Button onclick={onnew} disabled={!ready}>new project</Button>
      <Button onclick={onopen} disabled={!ready}>open project</Button>
    </div>
    {#if ready && $store.recents.length}
      <h2 class="label">recent</h2>
      <ul>
        {#each $store.recents as r (r.path)}
          <li>
            <ListRow onclick={() => onrecent(r.path)}>
              <span class="recent">
                <span class="name">{r.name}</span>
                <span class="hint path">{r.path}</span>
              </span>
            </ListRow>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

<style>
  section {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-5);
  }
  .wordmark {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  .dim {
    margin: 0;
    color: var(--text-dim);
  }
  h2 {
    margin: var(--space-3) 0 0;
  }
  .row {
    display: flex;
    gap: var(--space-3);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    width: min(480px, 100%);
  }
  .path {
    word-break: break-all;
  }
  /* Name over path, inside the row's main content (which ellipsizes), so a long path wraps. */
  .recent {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .recent .path {
    white-space: normal;
  }
</style>

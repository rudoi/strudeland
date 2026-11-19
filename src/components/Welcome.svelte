<script lang="ts">
  import type { Session } from '../lib/project/session';
  import { defaultProjectName } from '../lib/project/ui';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import ListRow from './ui/ListRow.svelte';
  import Notice from './ui/Notice.svelte';
  import Scope from './ui/Scope.svelte';

  let { session }: { session: Session } = $props();
  // `session` is constant for the window's lifetime.
  // svelte-ignore state_referenced_locally
  const store = session.store;

  let pickedDir = $state<string | null>(null);
  let name = $state('');

  async function newProject() {
    const dir = await session.pickNewProjectFolder();
    if (dir) {
      pickedDir = dir;
      name = defaultProjectName(dir);
    }
  }

  async function create() {
    if (!pickedDir || !name.trim()) return;
    const dir = pickedDir;
    pickedDir = null;
    await session.createProject(dir, name.trim());
  }
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
    {#if pickedDir}
      <form onsubmit={(e) => (e.preventDefault(), void create())}>
        <Field
          layout="stacked"
          label="project name"
          width="fill"
          value={name}
          oninput={(e) => (name = e.currentTarget.value)}
          autofocus
          spellcheck="false"
        />
        <div class="hint path">{pickedDir}</div>
        <div class="row">
          <Button variant="primary" type="submit" disabled={!name.trim()}>create</Button>
          <Button type="button" onclick={() => (pickedDir = null)}>cancel</Button>
        </div>
      </form>
    {:else}
      <div class="row">
        <Button onclick={() => void newProject()}>new project</Button>
        <Button onclick={() => void session.openProject()}>open project</Button>
      </div>
      {#if $store.recents.length}
        <h2 class="label">recent</h2>
        <ul>
          {#each $store.recents as r (r.path)}
            <li>
              <ListRow onclick={() => void session.openRecent(r.path)}>
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
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: min(420px, 100%);
  }
</style>

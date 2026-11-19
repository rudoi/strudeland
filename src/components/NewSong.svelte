<script lang="ts">
  import { tick } from 'svelte';
  import type { Session } from '../lib/project/session';
  import { songStem } from '../lib/project/ui';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Notice from './ui/Notice.svelte';

  let {
    session,
    open = $bindable(false),
    showField = true,
  }: { session: Session; open?: boolean; showField?: boolean } = $props();

  let name = $state('');
  let error = $state<string | null>(null);
  let input = $state<HTMLInputElement>();

  $effect(() => {
    if (open && showField) void tick().then(() => input?.focus());
  });

  function cancel() {
    open = false;
    name = '';
    error = null;
  }

  async function confirm() {
    const stem = songStem(name);
    if (!stem) return;
    try {
      await session.createSong(stem);
      cancel();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      void confirm();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
    }
  }
</script>

{#if open && showField}
  <div class="new">
    <Field
      value={name}
      width="fill"
      bind:el={input}
      oninput={(e) => {
        name = e.currentTarget.value;
        error = null;
      }}
      onkeydown={onKey}
      placeholder="song name"
      spellcheck="false"
      aria-label="New song name"
    />
    {#if error}<Notice tone="danger">{error}</Notice>{/if}
  </div>
{:else}
  <div class="add">
    <Button variant="quiet" onclick={() => (open = true)}>+ new song</Button>
  </div>
{/if}

<style>
  .new {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
  }
  .add {
    padding: var(--space-1) var(--space-2);
  }
</style>

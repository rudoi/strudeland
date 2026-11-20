<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { defaultProjectName } from '../lib/project/ui';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';

  let {
    folder,
    oncreate,
    oncancel,
  }: {
    folder: string;
    oncreate: (name: string) => void;
    oncancel: () => void;
  } = $props();

  let name = $state(untrack(() => defaultProjectName(folder)));
  let dialog: HTMLDialogElement;

  onMount(() => {
    const element = dialog;
    element.showModal();
    return () => element.close();
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="new-project-title"
  oncancel={(event) => {
    event.preventDefault();
    oncancel();
  }}
>
  <form onsubmit={(event) => (event.preventDefault(), oncreate(name))}>
    <h2 id="new-project-title">new project</h2>
    <Field
      layout="stacked"
      label="project name"
      width="fill"
      value={name}
      oninput={(event) => (name = event.currentTarget.value)}
      autofocus
      spellcheck="false"
    />
    <div class="hint path">{folder}</div>
    <div class="actions">
      <Button type="button" onclick={oncancel}>cancel</Button>
      <Button variant="primary" type="submit" disabled={!name.trim()}>create</Button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(420px, calc(100% - 2 * var(--space-5)));
    padding: var(--space-5);
    border: 0;
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-pop);
  }
  dialog::backdrop {
    background: var(--modal-backdrop);
  }
  form {
    display: grid;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-md);
  }
  .path {
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>

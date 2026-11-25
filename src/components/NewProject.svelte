<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { defaultProjectName } from '../lib/project/ui';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Help from './ui/Help.svelte';

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
  const missingName = $derived(!name.trim());
  let dialog: HTMLDialogElement;

  onMount(() => {
    const previous = document.activeElement;
    const element = dialog;
    element.showModal();
    return () => {
      element.close();
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
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
    <div class="body">
      <h2 id="new-project-title">new project</h2>
      <Field
        layout="stacked"
        label="project name"
        help={missingName
          ? 'Enter a name to create the project.'
          : 'Set the display name for the project in the chosen folder.'}
        width="fill"
        value={name}
        oninput={(event) => (name = event.currentTarget.value)}
        autofocus
        spellcheck="false"
      />
      <div class="hint path">{folder}</div>
    </div>
    <div class="actions">
      <Button type="button" onclick={oncancel} help="Close without creating a project."
        >cancel</Button
      >
      <Button
        variant="primary"
        type="submit"
        disabled={missingName}
        help={missingName
          ? 'Enter a name to create the project.'
          : 'Set up and open the project in the chosen folder.'}>create</Button
      >
    </div>
    <Help />
  </form>
</dialog>

<style>
  dialog {
    width: min(420px, calc(100% - 2 * var(--space-5)));
    max-height: calc(100% - 2 * var(--space-5));
    padding: 0;
    border: 0;
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  dialog::backdrop {
    background: var(--modal-backdrop);
  }
  form {
    display: flex;
    flex-direction: column;
    max-height: calc(100dvh - 2 * var(--space-5));
  }
  .body {
    display: grid;
    gap: var(--space-3);
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-5) var(--space-5) var(--space-3);
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
    flex: none;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: 0 var(--space-5) var(--space-3);
  }
</style>

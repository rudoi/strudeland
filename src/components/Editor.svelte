<script lang="ts">
  import { onMount } from 'svelte';
  import type { App } from '../lib/app';
  import { createEditor } from '../lib/editor/setup';

  let { app, hidden = false }: { app: App; hidden?: boolean } = $props();
  let host: HTMLDivElement;

  onMount(() => {
    // The view is created once per window (initTheme rule); the session fills it with songs.
    const view = createEditor(host, '', app.runtime, app.hooks);
    app.attachView(view);
    return () => {
      app.attachView(undefined);
      view.destroy();
    };
  });
</script>

<div class="editor" class:hidden bind:this={host}></div>

<style>
  .editor {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .editor.hidden {
    display: none;
  }
  .editor :global(.cm-editor) {
    height: 100%;
  }
</style>

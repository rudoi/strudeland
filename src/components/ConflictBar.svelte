<script lang="ts">
  import type { Session } from '../lib/project/session';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';

  let { session, conflict }: { session: Session; conflict: 'changed' | 'deleted' } = $props();
</script>

<Notice layout="bar" tone="warning" role="alert">
  {conflict === 'changed'
    ? 'this song changed on disk while you have unsaved edits.'
    : 'this song was deleted on disk.'}
  {#snippet actions()}
    {#if conflict === 'changed'}
      <Button
        size="sm"
        onclick={() => session.keepMine()}
        help="Keep your edits in the editor. Saving afterward replaces the version on disk."
        >keep mine</Button
      >
      <Button
        size="sm"
        onclick={() => session.takeDisk()}
        help="Replace your unsaved edits with the version on disk.">take disk</Button
      >
    {:else}
      <Button
        size="sm"
        onclick={() => void session.recreateDeleted()}
        help="Write the editor copy back to the deleted song’s original path."
        >save to recreate</Button
      >
      <Button
        size="sm"
        onclick={() => void session.closeDeleted()}
        help="Discard the editor copy of the deleted song.">close</Button
      >
    {/if}
  {/snippet}
</Notice>

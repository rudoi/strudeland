<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  type Props = Omit<HTMLInputAttributes, 'value' | 'size' | 'width'> & {
    label?: string;
    /** One-way: the input shows it, and typing does not write it back (use `oninput`). */
    value: string | number;
    size?: 'sm' | 'md';
    layout?: 'inline' | 'stacked';
    /** `narrow` is 64px; `fill` stretches the field and its input to the container's width. */
    width?: 'narrow' | 'auto' | 'fill';
    el?: HTMLInputElement;
  };

  let {
    label,
    value,
    size = 'md',
    layout = 'inline',
    width = 'auto',
    el = $bindable(),
    class: className,
    ...rest
  }: Props = $props();
</script>

<label class={['field', layout, { fill: width === 'fill' }]}>
  {#if label}<span class="label">{label}</span>{/if}
  <!-- svelte-ignore a11y_autofocus -->
  <input bind:this={el} {value} {...rest} class={['input', 'control', size, width, className]} />
</label>

<style>
  .field {
    min-width: 0;
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1-5);
  }
  .stacked {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  /* Height, padding, outline and the disabled outline: .control. */
  .input {
    min-width: 0;
    background-color: var(--bg);
    color: var(--text);
  }
  .input::placeholder {
    color: var(--placeholder);
  }
  .input:disabled {
    color: var(--text-faint);
  }
  .narrow {
    width: 64px;
  }
  .field.fill {
    width: 100%;
  }
  .inline .input.fill {
    flex: 1;
  }
</style>

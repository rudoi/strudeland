<script lang="ts">
  import { tick } from 'svelte';
  import type { App } from '../lib/app';
  import {
    blockPlayhead,
    commitInput,
    editableStructure,
    fieldValue,
    hasEnding,
    litBars,
    minorTicks,
    playheadPct,
    rulerBars,
    rulerMarks,
    type InputField,
  } from '../lib/project/timeline';
  import { authoredLoop, meterString } from '../lib/strudel/structure';
  import { canPlayEnding, type Located } from '../lib/strudel/transport';
  import { catOf } from '../lib/theme/tones';
  import SectionBlock from './SectionBlock.svelte';
  import Button from './ui/Button.svelte';
  import Field from './ui/Field.svelte';
  import Menu from './ui/Menu.svelte';
  import MenuItem from './ui/MenuItem.svelte';
  import Notice from './ui/Notice.svelte';

  let { app }: { app: App } = $props();
  // `app` is constant for the window's lifetime, so reading it once is correct.
  // svelte-ignore state_referenced_locally
  const { runtime, session } = app;
  const rt = runtime.store;
  const tp = runtime.transport.store;
  const ss = session.store;

  // None while the header has a meta error: no blocks, menu or inputs (see the note below).
  const structure = $derived(editableStructure($ss));
  const sections = $derived(structure?.sections ?? []);
  const bars = $derived(rulerBars(structure));
  const marks = $derived(rulerMarks(bars));
  // Beat ticks between the bar marks, for the meter of what the blocks show.
  const ticks = $derived(
    structure?.meterChanges ? 0 : minorTicks(bars, structure?.meter ?? $rt.meter),
  );
  const savedLoop = $derived(structure && authoredLoop(structure));
  const hasLoop = $derived(savedLoop !== null);
  const ending = $derived(hasEnding(structure));

  // The evaluated song is the active one; otherwise there is no playhead or menu.
  const playingActive = $derived($rt.started && $ss.evaluated?.path === $ss.active);
  const playingOther = $derived($rt.started && $ss.evaluated !== null && !playingActive);

  // Position, refreshed each animation frame while the scheduler runs.
  let position = $state.raw<{ cycle: number; located: Located | null }>({
    cycle: 0,
    located: null,
  });
  $effect(() => {
    if (!$rt.started) return;
    let frame = 0;
    const tick = () => {
      position = runtime.position();
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  });

  // Without sections the playhead runs over the 16-bar ruler; with them it sits in the block
  // that plays and spans the blocks only: it follows the blocks' layout, whose widths are not
  // proportional to bars (min width, gaps), so it would not line up with the ruler's marks.
  const pct = $derived(
    playingActive && sections.length === 0 ? playheadPct($rt.structure, position) : null,
  );
  const head = $derived(playingActive ? blockPlayhead(structure, position.located) : null);
  const activeSection = $derived(playingActive ? (position.located?.section ?? null) : null);
  const audition = $derived($tp.view.kind === 'loop' ? $tp.view : null);
  const endingEnabled = $derived(
    canPlayEnding($rt.started, $rt.structure, $tp.view, position.located) && playingActive,
  );

  // ---- section menu ----
  let menuFor = $state<number | null>(null);
  let choosingEnd = $state(false);
  let track = $state<HTMLDivElement>();
  const armedName = $derived(
    $tp.armedSection === null ? null : (structure?.sections[$tp.armedSection]?.name ?? null),
  );
  const resolveStructure = () => ($rt.started ? $rt.structure : structure);

  function openMenu(e: MouseEvent, index: number) {
    e.stopPropagation();
    menuFor = menuFor === index ? null : index;
    choosingEnd = false;
  }
  /** Start here or loop this section; while stopped, also evaluate so it starts. */
  function arm(i: number, act: 'startFrom' | 'loopSection') {
    const s = resolveStructure();
    menuFor = null;
    if (!s) return;
    runtime.transport[act](i, s);
    if (!$rt.started) app.evaluateDoc();
  }
  function stopLooping() {
    const s = resolveStructure();
    menuFor = null;
    if (s) runtime.transport.loopSection(null, s);
  }
  function armRange(first: number, last: number) {
    const s = resolveStructure();
    menuFor = null;
    if (!s) return;
    runtime.transport.loopRange(first, last, s);
    if (!$rt.started) app.evaluateDoc();
  }
  async function showRangeOptions(show: boolean) {
    choosingEnd = show;
    await tick();
    track?.querySelector<HTMLButtonElement>('[data-range-focus]')?.focus();
  }

  // ---- meta inputs ----
  let seenDoc = '';
  let problem = $state<{ key: string; message: string } | null>(null);

  function onFocus() {
    seenDoc = session.activeText() ?? '';
  }
  function commit(e: Event, key: string, field: InputField, section?: number) {
    const el = e.currentTarget as HTMLInputElement;
    const r = commitInput(session, field, el.value, { section, hasLoopSection: hasLoop }, seenDoc);
    seenDoc = r.seen;
    const out = r.outcome;
    if (out.kind === 'applied') {
      problem = null;
      return;
    }
    if (out.kind === 'invalid') {
      problem = { key, message: out.error };
      return;
    }
    // Nothing was applied: show what the header says now.
    if (structure) el.value = fieldValue(structure, field, section);
    problem = {
      key,
      message:
        out.kind === 'stale'
          ? 'the header changed, try again'
          : 'this header cannot be edited here; edit it in the code',
    };
  }
</script>

<section class="timeline">
  <div class="status">
    {#if playingOther}
      <span>playing: {$ss.evaluated?.title}</span>
    {:else if $tp.pendingStart && armedName}
      <span>will start at {armedName}</span>
    {/if}
    {#if ending}
      <Button
        size="sm"
        class="ending"
        onclick={() => runtime.transport.playEnding()}
        disabled={!endingEnabled}
      >
        play ending
      </Button>
    {/if}
  </div>

  {#if !playingOther}
    <div class="track" bind:this={track}>
      {#if sections.length > 0}
        <div class="blocks">
          {#each sections as sec, i (i)}
            <div class="cell" style:flex-grow={sec.bars}>
              <SectionBlock
                cat={catOf(i)}
                name={sec.name}
                bars={sec.bars}
                loop={savedLoop !== null && i >= savedLoop.first && i <= savedLoop.last}
                looping={audition !== null &&
                  i >= audition.section &&
                  i <= (audition.through ?? audition.section)}
                current={activeSection === i}
                lit={head?.section === i ? litBars(sec.bars, head.localCycle) : 0}
                onclick={(e) => openMenu(e, i)}
                aria-haspopup="menu"
                aria-expanded={menuFor === i}
              >
                {#snippet menu()}
                  <Menu open={menuFor === i} onclose={() => (menuFor = null)} class="section-menu">
                    {#if choosingEnd}
                      <MenuItem onclick={() => showRangeOptions(false)}>back</MenuItem>
                      {#each sections.slice(i + 1) as end, offset (end.name)}
                        <MenuItem
                          data-range-focus={offset === 0 ? '' : undefined}
                          onclick={() => armRange(i, i + offset + 1)}
                        >
                          through {end.name}
                        </MenuItem>
                      {/each}
                    {:else}
                      <MenuItem onclick={() => arm(i, 'startFrom')}>start here</MenuItem>
                      <MenuItem onclick={() => arm(i, 'loopSection')}>loop this section</MenuItem>
                      {#if i < sections.length - 1}
                        <MenuItem data-range-focus="" onclick={() => showRangeOptions(true)}
                          >loop through…</MenuItem
                        >
                      {/if}
                      <MenuItem onclick={stopLooping} disabled={$tp.view.kind !== 'loop'}>
                        stop looping
                      </MenuItem>
                    {/if}
                  </Menu>
                {/snippet}
              </SectionBlock>
              {#if head?.section === i}
                <div class="playhead" style:left="{head.fraction * 100}%"></div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      <div
        class="ruler"
        class:minor={ticks > 0}
        style:background-size={ticks > 0 ? `${100 / ticks}% 4px` : undefined}
      >
        {#each marks as bar (bar)}
          <span class="mark mono" style:left="{(bar / bars) * 100}%">{bar + 1}</span>
        {/each}
      </div>
      {#if structure?.meterChanges}
        <ul class="meter-changes mono" aria-label="time signature changes">
          {#each structure.meterChanges.slice(0, 32) as change (change.bar)}
            <li>bar {change.bar + 1} · {meterString(change.meter)}</li>
          {/each}
          {#if structure.meterChanges.length > 32}
            <li>{structure.meterChanges.length - 32} more in the header</li>
          {/if}
        </ul>
      {/if}
      {#if pct !== null}
        <div class="playhead" style:left="{pct}%"></div>
      {/if}
    </div>
  {/if}

  {#if structure}
    <div class="inputs">
      <Field
        label="quarter-note bpm"
        size="sm"
        width="narrow"
        class="mono"
        type="number"
        value={structure.bpm}
        onfocus={onFocus}
        onchange={(e) => commit(e, 'bpm', 'bpm')}
      />
      <Field
        label={structure.meterChanges ? 'starting meter' : 'meter'}
        size="sm"
        width="narrow"
        class="mono"
        type="text"
        value={meterString(structure.meter)}
        onfocus={onFocus}
        onchange={(e) => commit(e, 'meter', 'meter')}
      />
      {#if hasLoop}
        <Field
          label="loops"
          size="sm"
          width="narrow"
          class="mono"
          type="number"
          value={structure.loops}
          onfocus={onFocus}
          onchange={(e) => commit(e, 'loops', 'loops')}
        />
      {/if}
      {#each structure.sections as sec, i (i)}
        <Field
          label="{sec.name} bars"
          size="sm"
          width="narrow"
          class="mono"
          type="number"
          value={sec.bars}
          onfocus={onFocus}
          onchange={(e) => commit(e, `bars${i}`, 'bars', i)}
        />
      {/each}
      <span class="hint applies">applies on the next evaluate</span>
    </div>
    {#if problem}
      <Notice tone="danger">{problem.message}</Notice>
    {/if}
  {:else if $ss.metaError}
    <div class="hint">fix the meta header to edit</div>
  {/if}
</section>

<style>
  .timeline {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background-color: var(--surface);
    border-top: 1px solid var(--border);
    font-size: var(--fs-sm);
  }
  .status {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    color: var(--text-dim);
  }
  .status:empty {
    display: none;
  }
  .status :global(.ending) {
    margin-left: auto;
  }
  .track {
    position: relative;
  }
  .blocks {
    display: flex;
    gap: var(--space-1-5);
  }
  .cell {
    position: relative;
    flex-basis: 0;
    min-width: 44px;
  }
  .cell :global(.section-menu) {
    max-height: 40vh;
    overflow-y: auto;
  }
  .ruler {
    position: relative;
    height: var(--ruler-h);
    margin-top: var(--space-1);
    border-top: 1px solid var(--scope-grat-major);
  }
  .ruler.minor {
    background-image: linear-gradient(to right, var(--scope-grat) 1px, transparent 1px);
    background-repeat: repeat-x;
    background-position: left top;
  }
  .mark {
    position: absolute;
    top: 0;
    bottom: 0;
    padding-left: var(--space-1);
    border-left: 1px solid var(--scope-grat-major);
    color: var(--text-faint);
    font-size: var(--fs-2xs);
    line-height: var(--ruler-h);
  }
  .meter-changes {
    display: flex;
    flex-wrap: wrap;
    column-gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--text-dim);
    font-size: var(--fs-2xs);
  }
  /* The scan line: a 1px phosphor line with a nub on top and an afterglow tail behind it. */
  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: var(--z-playhead);
    width: 1px;
    background-color: var(--scan-line);
    box-shadow:
      0 0 6px var(--scan-line),
      0 0 14px var(--scope-glow);
    pointer-events: none;
  }
  .playhead::before {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    right: 1px;
    width: 44px;
    background-image: linear-gradient(to right, transparent, var(--scan-tail));
  }
  .playhead::after {
    content: '';
    position: absolute;
    top: -3px;
    left: -3px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: var(--scan-line);
  }
  .inputs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1-5) var(--space-3);
    align-items: center;
  }
  .applies {
    margin-left: auto;
    white-space: nowrap;
  }
</style>

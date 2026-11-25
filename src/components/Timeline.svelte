<script lang="ts">
  import { tick } from 'svelte';
  import type { App } from '../lib/app';
  import type { ArmChoice } from '../lib/project/song-playback';
  import {
    blockPlayhead,
    commitInput,
    editableStructure,
    fieldValue,
    hasEnding,
    litBars,
    minorTicks,
    mixedMeterRuler,
    playheadPct,
    rulerBars,
    rulerMarks,
    sectionItemHelp,
    type InputField,
    type SectionItemTexts,
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
  const { runtime, session, arbiter } = app;
  const rt = runtime.store;
  const ss = session.store;
  const playback = session.playback;
  const pb = playback.store;
  const canArm = playback.canArm;
  const play = arbiter.store;
  /** A section item's help: re-evaluate, else wait for the backing, else what a click does now. */
  function itemHelp(resolves: boolean, text: SectionItemTexts): string {
    return sectionItemHelp(
      resolves,
      {
        canPlayFrom: $play.canPlayFrom,
        stopTarget: $play.stopTarget,
        started: $rt.started,
        taking: $play.taking,
      },
      text,
    );
  }

  // None while the header has a meta error: no blocks, menu or inputs (see the note below).
  const structure = $derived(editableStructure($ss));
  const sections = $derived(structure?.sections ?? []);
  const bars = $derived(rulerBars(structure));
  const marks = $derived(rulerMarks(bars));
  // Beat ticks between the bar marks, for the meter of what the blocks show.
  const ticks = $derived(
    structure?.meterChanges ? 0 : minorTicks(bars, structure?.meter ?? $rt.meter),
  );
  const mixedRuler = $derived(structure?.meterChanges ? mixedMeterRuler(structure) : null);
  const savedLoop = $derived(structure && authoredLoop(structure));
  const hasLoop = $derived(savedLoop !== null);

  // The playing song is the active one; otherwise there is no playhead or menu.
  const playingActive = $derived($pb.playing !== null && $pb.playing.path === $ss.active);
  const playingOther = $derived($pb.playing !== null && !playingActive);
  const playable = $derived($rt.started ? $rt.structure : structure);
  const ending = $derived(hasEnding(playable));

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
  const head = $derived.by(() => {
    const located = position.located;
    if (!playingActive || !located) return null;
    const name = playable?.sections[located.section]?.name;
    const section = sections.findIndex((item) => item.name === name);
    return blockPlayhead(structure, { ...located, section });
  });
  const auditionNames = $derived($pb.transport.loop?.names ?? []);
  const endingEnabled = $derived(
    canPlayEnding($rt.started, $rt.structure, $pb.transport.view, position.located) &&
      playingActive,
  );

  // ---- section menu ----
  let menuFor = $state<number | null>(null);
  let choosingEnd = $state(false);
  let track = $state<HTMLDivElement>();
  // Shown with the blocks only (none while the header has a meta error). The module disarms on a
  // meta error, except for the arm of a `start` still waiting on the runtime, so the gate stays.
  const armedName = $derived(structure ? $pb.transport.armed : null);

  function openMenu(e: MouseEvent, index: number) {
    e.stopPropagation();
    menuFor = menuFor === index ? null : index;
    choosingEnd = false;
  }
  /** Start here or loop (through a later section); while stopped, also evaluate so it starts. */
  function playFrom(choice: ArmChoice) {
    menuFor = null;
    arbiter.playFrom(choice);
  }
  function stopLooping() {
    menuFor = null;
    playback.stopLooping();
  }
  async function showRangeOptions(show: boolean) {
    choosingEnd = show;
    await tick();
    const focus = track?.querySelector<HTMLButtonElement>('[data-range-focus]:not(:disabled)');
    (focus ?? track?.querySelector<HTMLButtonElement>('[role="menu"] button'))?.focus();
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
      <span>playing: {$pb.evaluated?.title}</span>
    {:else if armedName}
      <span>will start at {armedName}</span>
    {/if}
    {#if ending}
      <Button
        size="sm"
        class="ending"
        onclick={() => playback.playEnding()}
        disabled={!endingEnabled}
        help={endingEnabled
          ? 'Finish the loop pass, then play the ending once.'
          : playingOther
            ? 'The ending belongs to the song currently playing.'
            : !$rt.started
              ? 'Start song playback to reach its defined loop.'
              : $pb.transport.view.kind === 'release'
                ? 'The ending is already queued or playing.'
                : 'Available while the song plays through its defined loop.'}
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
                looping={auditionNames.includes(sec.name)}
                current={head?.section === i}
                lit={head?.section === i ? litBars(sec.bars, head.localCycle) : 0}
                onclick={(e) => openMenu(e, i)}
                aria-haspopup="menu"
                aria-expanded={menuFor === i}
              >
                {#snippet menu()}
                  <Menu open={menuFor === i} onclose={() => (menuFor = null)} class="section-menu">
                    {#if choosingEnd}
                      <MenuItem
                        help="Return to the section’s playback options."
                        onclick={() => showRangeOptions(false)}>back</MenuItem
                      >
                      {#each sections.slice(i + 1) as end (end.name)}
                        {@const range = $canArm({ loop: sec.name, through: end.name })}
                        <MenuItem
                          data-range-focus=""
                          disabled={!range || !$play.canPlayFrom}
                          help={itemHelp(range, {
                            stale: 'Re-evaluate the song to use this section range.',
                            started:
                              'Repeat through the selected end, starting at the next available bar line.',
                            taking:
                              'Set this section range to repeat the next time the song plays.',
                            stopped: 'Start playback and repeat through the selected end.',
                          })}
                          onclick={() => playFrom({ loop: sec.name, through: end.name })}
                        >
                          through {end.name}
                        </MenuItem>
                      {/each}
                    {:else}
                      {@const target = $canArm({ start: sec.name })}
                      <MenuItem
                        disabled={!target || !$play.canPlayFrom}
                        help={itemHelp(target, {
                          stale: 'Re-evaluate the song to play this section.',
                          started: 'Jump to this section at the next available bar line.',
                          taking: 'Set this section as the starting point for the next play.',
                          stopped: 'Start song playback at the first bar of this section.',
                        })}
                        onclick={() => playFrom({ start: sec.name })}
                      >
                        start here
                      </MenuItem>
                      <MenuItem
                        disabled={!$canArm({ loop: sec.name }) || !$play.canPlayFrom}
                        help={itemHelp($canArm({ loop: sec.name }), {
                          stale: 'Re-evaluate the song to loop this section.',
                          started: 'Repeat this section starting at the next available bar line.',
                          taking: 'Set this section to repeat the next time the song plays.',
                          stopped: 'Start playback and repeat this section.',
                        })}
                        onclick={() => playFrom({ loop: sec.name })}
                      >
                        loop this section
                      </MenuItem>
                      {#if i < sections.length - 1}
                        <MenuItem
                          data-range-focus=""
                          help="Choose the last section to include in the loop."
                          onclick={() => showRangeOptions(true)}>loop through…</MenuItem
                        >
                      {/if}
                      <MenuItem
                        onclick={stopLooping}
                        disabled={$pb.transport.view.kind !== 'loop'}
                        help={$pb.transport.view.kind !== 'loop'
                          ? 'There’s no temporary section loop to clear.'
                          : $rt.started
                            ? 'Return to the song’s normal playback at the next available bar line.'
                            : 'Clear the temporary section loop for the next play.'}
                      >
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
        {#each mixedRuler?.ticks ?? [] as run (run.from)}
          <span
            class="meter-ticks"
            aria-hidden="true"
            style:left="{(run.from / bars) * 100}%"
            style:width="{((run.to - run.from) / bars) * 100}%"
            style:background-size="{100 / run.count}% 4px"
          ></span>
        {/each}
        {#each marks as bar (bar)}
          <span class="mark mono" style:left="{(bar / bars) * 100}%">{bar + 1}</span>
        {/each}
      </div>
      {#if mixedRuler?.labels.length}
        <ul class="meter-changes mono" aria-label="time signature changes">
          {#each mixedRuler.labels as change (change.bar)}
            <li
              title="bar {change.bar + 1} · {meterString(change.meter)}"
              style:left="{(change.bar / bars) * 100}%"
            >
              {meterString(change.meter)}
            </li>
          {/each}
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
        help="Set the tempo in quarter notes per minute. Applies on the next evaluation."
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
        help={structure.meterChanges
          ? 'Set the time signature before the first meter change, for example 4/4. Applies on the next evaluation.'
          : 'Set the song’s time signature, for example 4/4. Applies on the next evaluation.'}
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
          help="Set how many times the defined loop plays in the song’s clip layout."
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
          help="Set this section’s length in whole bars. Applies on the next evaluation."
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
  .ruler.minor,
  .meter-ticks {
    background-image: linear-gradient(to right, var(--scope-grat) 1px, transparent 1px);
    background-repeat: repeat-x;
    background-position: left top;
  }
  .meter-ticks {
    position: absolute;
    top: 0;
    height: 4px;
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
    position: relative;
    height: var(--ruler-h);
    margin: 0;
    padding: 0;
    list-style: none;
    color: var(--text-dim);
    font-size: var(--fs-2xs);
  }
  .meter-changes li {
    position: absolute;
    border-left: 1px solid var(--scope-grat-major);
    padding-left: var(--space-1);
    line-height: var(--ruler-h);
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

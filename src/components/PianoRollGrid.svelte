<script lang="ts">
  import { tick, untrack } from 'svelte';
  import {
    adjacentStep,
    barAtOffset,
    cycleNumber,
    fitPitchRange,
    gridPoint,
    isBlackKey,
    keyPitch,
    layoutNoteTracks,
    pitchLabel,
    rollGeometry,
    rollOffset,
    snapGrid,
    type GridBar,
    type PitchRange,
  } from '../lib/piano-roll/grid';
  import type { RollBar, RollEdit, RollNote } from '../lib/piano-roll/types';
  import Button from './ui/Button.svelte';

  let {
    notes,
    bars,
    editable,
    insertable,
    playhead,
    onedit,
    onaudition,
    onsource,
  }: {
    notes: readonly RollNote[];
    bars: readonly RollBar[];
    editable: boolean;
    insertable: boolean;
    playhead?: number;
    onedit: (edit: RollEdit) => boolean;
    onaudition: (pitch: number, note?: RollNote) => void;
    onsource?: (note: RollNote) => void;
  } = $props();

  const ROW = 17;
  const FREE_STEPS = 24;
  const KEY_WIDTH = 54;
  const RULER_HEIGHT = 25;
  let viewport = $state<HTMLDivElement>();
  let surface = $state<HTMLDivElement>();
  let placeButton = $state<HTMLButtonElement>();
  let subdivision = $state(2);
  let keyboardPitch = $state(60);
  let cursor = $state<{ step: number; bar: number; pitch: number }>();
  let selected = $state<string>();
  let ignoreClick: string | undefined;
  let scrollFrame = 0;

  interface Gesture {
    pointer: number;
    note: RollNote;
    notes: readonly RollNote[];
    bars: readonly RollBar[];
    range: PitchRange;
    x: number;
    y: number;
    scrollY: number;
    grabOffset: number;
    resize: boolean;
    axis?: 'pitch' | 'time';
    lastX: number;
    lastY: number;
    free: boolean;
    edit?: RollEdit;
  }

  let gesture = $state.raw<Gesture>();
  const geometry = $derived(rollGeometry(bars));
  const windowStart = $derived(geometry.bars[0]?.cycle ?? 0);
  const windowEnd = $derived(geometry.bars.length ? geometry.bars.at(-1)!.cycle + 1 : 0);
  const visible = $derived(
    notes.filter((note) => {
      const start = cycleNumber(note.visibleStart ?? note.start);
      const end = cycleNumber(note.visibleEnd ?? note.end);
      return start < windowEnd && (end > windowStart || (start === end && start === windowStart));
    }),
  );
  const fitRange = $derived(fitPitchRange(visible));
  const pitchRange = $derived(gesture?.range ?? fitRange);
  const pitches = $derived(
    Array.from({ length: pitchRange.high - pitchRange.low + 1 }, (_, i) => pitchRange.high - i),
  );
  const selectedNote = $derived(visible.find((note) => note.id === selected));
  const sourceKey = (note: RollNote | undefined) =>
    note?.origin ? `${note.origin.from}:${note.origin.to}` : undefined;
  const selectedSource = $derived(sourceKey(selectedNote));
  const sourceCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const note of notes) {
      const key = sourceKey(note);
      if (key !== undefined && note.pitchEditable) counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return counts;
  });
  const canShowSource = (note: RollNote) =>
    Boolean(onsource && (note.origin || note.contributors?.length));
  const markers = $derived(layoutNoteTracks(visible, windowStart, windowEnd));

  const percent = (offset: number) => (offset / geometry.width) * 100;
  const timePercent = (cycle: number) => percent(rollOffset(geometry.bars, cycle));
  const laneY = (pitch: number) => (pitchRange.high - keyPitch(pitch)) * ROW;

  $effect(() => {
    const { low, high } = fitRange;
    const view = viewport;
    if (!view) return;
    untrack(() => {
      const centerPitch = Math.round((low + high) / 2);
      if (keyboardPitch < low || keyboardPitch > high) keyboardPitch = centerPitch;
      const height = view.clientHeight - RULER_HEIGHT;
      if ((high - low + 1) * ROW > height)
        view.scrollTop = Math.max(0, laneY(centerPitch) + ROW / 2 - height / 2);
    });
  });

  $effect(() => {
    if (
      cursor &&
      (!insertable ||
        !geometry.bars[cursor.bar] ||
        cursor.pitch < fitRange.low ||
        cursor.pitch > fitRange.high)
    )
      cursor = undefined;
  });

  $effect(() => () => cancelAnimationFrame(scrollFrame));

  function pointerOffset(clientX: number): number {
    const rect = surface!.getBoundingClientRect();
    return ((clientX - rect.left) / rect.width) * geometry.width;
  }

  const pitchAt = (clientY: number) =>
    keyPitch(pitchRange.high - Math.floor((clientY - surface!.getBoundingClientRect().top) / ROW));

  function beginPointer(event: PointerEvent, note: RollNote) {
    if (event.button !== 0 || !event.isPrimary) return;
    ignoreClick = undefined;
    if (!editable || (!note.pitchEditable && !note.timeEditable)) return;
    const button = event.currentTarget as HTMLButtonElement;
    const rect = button.getBoundingClientRect();
    gesture = {
      pointer: event.pointerId,
      note,
      notes,
      bars,
      range: pitchRange,
      x: event.clientX,
      y: event.clientY,
      scrollY: viewport!.scrollTop,
      grabOffset: pointerOffset(event.clientX) - rollOffset(geometry.bars, cycleNumber(note.start)),
      resize: Boolean(note.timeEditable && rect.width > 16 && rect.right - event.clientX <= 6),
      lastX: event.clientX,
      lastY: event.clientY,
      free: event.altKey,
    };
    button.setPointerCapture(event.pointerId);
  }

  function timeEdit(current: Gesture): RollEdit | undefined {
    const { note, resize, lastX, grabOffset, free } = current;
    const start = cycleNumber(note.start);
    const end = cycleNumber(note.end);
    const bar = geometry.bars.find((bar) => start >= bar.cycle && start < bar.cycle + 1);
    if (!bar) return;
    const division = free ? FREE_STEPS : subdivision;
    const offset = pointerOffset(lastX) - (resize ? 0 : grabOffset);
    const point = snapGrid(bar, offset, division, resize ? 1 : 1 - (end - start));
    const value = cycleNumber(point);
    if (resize)
      return value > start && value !== end
        ? { kind: 'resize', id: note.id, end: point }
        : undefined;
    return value !== start ? { kind: 'move', id: note.id, start: point } : undefined;
  }

  function updateGesture(current: Gesture): Gesture {
    const dx = current.lastX - current.x;
    const dy = current.lastY - current.y;
    let { axis } = current;
    if (!axis && Math.max(Math.abs(dx), Math.abs(dy)) < 4) return current;
    if (!axis)
      axis =
        !current.note.timeEditable ||
        (!current.resize && current.note.pitchEditable && Math.abs(dy) > Math.abs(dx))
          ? 'pitch'
          : 'time';
    if (axis === 'time') return { ...current, axis, edit: timeEdit(current) };
    const distance = dy + viewport!.scrollTop - current.scrollY;
    const pitch = keyPitch(current.note.pitch - Math.round(distance / ROW));
    return {
      ...current,
      axis,
      edit:
        pitch === current.note.pitch ? undefined : { kind: 'pitch', id: current.note.id, pitch },
    };
  }

  const stillCurrent = (current: Gesture) =>
    editable && current.notes === notes && current.bars === bars;

  function edgeScroll() {
    scrollFrame = 0;
    const current = gesture;
    const view = viewport;
    if (!view || !current?.axis || !stillCurrent(current)) return;
    const rect = view.getBoundingClientRect();
    const previousY = view.scrollTop;
    const previousX = view.scrollLeft;
    if (current.axis === 'pitch') {
      if (current.lastY < rect.top + RULER_HEIGHT + 8) view.scrollTop -= 6;
      if (current.lastY > rect.bottom - 8) view.scrollTop += 6;
    } else {
      if (current.lastX < rect.left + KEY_WIDTH + 8) view.scrollLeft -= 6;
      if (current.lastX > rect.right - 8) view.scrollLeft += 6;
    }
    if (previousX !== view.scrollLeft || previousY !== view.scrollTop) {
      gesture = updateGesture(current);
      scrollFrame = requestAnimationFrame(edgeScroll);
    }
  }

  function movePointer(event: PointerEvent) {
    if (gesture?.pointer !== event.pointerId) return;
    if (!stillCurrent(gesture)) return;
    gesture = updateGesture({
      ...gesture,
      lastX: event.clientX,
      lastY: event.clientY,
      free: event.altKey,
    });
    if (gesture.axis && !scrollFrame) scrollFrame = requestAnimationFrame(edgeScroll);
  }

  function endPointer(event: PointerEvent, cancelled = false) {
    const current = gesture;
    if (current?.pointer !== event.pointerId) return;
    gesture = undefined;
    cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
    if (!cancelled && (current.axis || !stillCurrent(current))) {
      ignoreClick = current.note.id;
      if (current.axis && stillCurrent(current)) {
        const { edit } = updateGesture({
          ...current,
          lastX: event.clientX,
          lastY: event.clientY,
          free: event.altKey,
        });
        if (edit) onedit(edit);
      }
    }
  }

  function activateNote(event: MouseEvent, note: RollNote) {
    if (ignoreClick === note.id && event.detail !== 0) {
      ignoreClick = undefined;
      return;
    }
    selected = note.id;
    if (event.altKey && canShowSource(note)) onsource?.(note);
    else onaudition(note.pitch, note);
  }

  function noteKeys(event: KeyboardEvent, note: RollNote) {
    if (event.shiftKey && event.key === 'Enter' && canShowSource(note)) {
      event.preventDefault();
      onsource?.(note);
      return;
    }
    if (!editable || event.metaKey || event.ctrlKey || event.altKey) return;
    let edit: Exclude<RollEdit, { kind: 'insert' }> | undefined;
    if (note.pitchEditable && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
      const pitch = keyPitch(note.pitch + (event.key === 'ArrowUp' ? 1 : -1));
      if (pitch !== note.pitch) edit = { kind: 'pitch', id: note.id, pitch };
    }
    if (note.timeEditable) {
      if (event.key === 'Delete' || event.key === 'Backspace')
        edit = { kind: 'delete', id: note.id };
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        const start = cycleNumber(note.start);
        const end = cycleNumber(note.end);
        const bar = geometry.bars.find((bar) => start >= bar.cycle && start < bar.cycle + 1);
        if (bar) {
          const resize = event.shiftKey;
          const step = adjacentStep(
            bar,
            resize ? end : start,
            subdivision,
            event.key === 'ArrowRight' ? 1 : -1,
          );
          const point = gridPoint(bar, step, subdivision);
          const value = cycleNumber(point);
          if (resize && value > start && value <= bar.cycle + 1)
            edit = { kind: 'resize', id: note.id, end: point };
          if (!resize && step >= 0 && value + (end - start) <= bar.cycle + 1 + Number.EPSILON)
            edit = { kind: 'move', id: note.id, start: point };
        }
      }
    }
    if (edit) {
      event.preventDefault();
      if (onedit(edit)) void restoreNoteFocus(event.currentTarget as HTMLButtonElement, note, edit);
    }
  }

  async function restoreNoteFocus(
    button: HTMLButtonElement,
    previous: RollNote,
    edit: Exclude<RollEdit, { kind: 'insert' }>,
  ) {
    await tick();
    if (document.activeElement !== document.body || !surface) return;
    const pitch = edit.kind === 'pitch' ? edit.pitch : previous.pitch;
    const start = edit.kind === 'move' ? edit.start : previous.start;
    const next =
      edit.kind === 'delete'
        ? (visible.find((note) => cycleNumber(note.start) >= cycleNumber(start)) ?? visible.at(-1))
        : visible.find((note) => note.pitch === pitch && note.start === start);
    if (button.isConnected) {
      button.focus();
    } else if (next) {
      Array.from(surface.querySelectorAll<HTMLButtonElement>('[data-note-id]'))
        .find((candidate) => candidate.dataset.noteId === next.id)
        ?.focus();
    } else {
      placeButton?.focus();
    }
  }

  function showCursor() {
    if (cursor || !geometry.bars.length) return;
    cursor = { bar: 0, step: 0, pitch: keyboardPitch };
  }

  function pointCursor(event: MouseEvent) {
    const offset = pointerOffset(event.clientX);
    const bar = barAtOffset(geometry.bars, offset);
    if (!bar) return;
    const steps = bar.numerator * subdivision;
    const step = Math.max(
      0,
      Math.min(steps - 1, Math.floor(((offset - bar.offset) / bar.width) * steps)),
    );
    cursor = { bar: geometry.bars.indexOf(bar), step, pitch: pitchAt(event.clientY) };
  }

  function insertNote() {
    if (!cursor || !editable || !insertable) return;
    const bar = geometry.bars[cursor.bar];
    if (!bar || cursor.step >= bar.numerator * subdivision) return;
    onedit({
      kind: 'insert',
      start: gridPoint(bar, cursor.step, subdivision),
      end: gridPoint(bar, cursor.step + 1, subdivision),
      pitch: cursor.pitch,
    });
  }

  function cursorKeys(event: KeyboardEvent) {
    const view = viewport;
    if (event.metaKey || event.ctrlKey || event.altKey || !cursor || !view || !surface) return;
    let { bar, step, pitch } = cursor;
    switch (event.key) {
      case 'ArrowLeft':
        if (step > 0) step--;
        else if (bar > 0) step = geometry.bars[--bar]!.numerator * subdivision - 1;
        break;
      case 'ArrowRight':
        if (step + 1 < geometry.bars[bar]!.numerator * subdivision) step++;
        else if (bar < geometry.bars.length - 1) {
          bar++;
          step = 0;
        }
        break;
      case 'ArrowUp':
        pitch = Math.min(pitchRange.high, pitch + 1);
        break;
      case 'ArrowDown':
        pitch = Math.max(pitchRange.low, pitch - 1);
        break;
      case 'Enter':
      case ' ':
        insertNote();
        event.preventDefault();
        return;
      default:
        return;
    }
    event.preventDefault();
    cursor = { bar, step, pitch };
    const targetBar: GridBar = geometry.bars[bar]!;
    const position = cycleNumber(gridPoint(targetBar, step, subdivision));
    const x = (timePercent(position) / 100) * surface.clientWidth;
    const y = laneY(pitch);
    if (x < view.scrollLeft) view.scrollLeft = x;
    if (x > view.scrollLeft + view.clientWidth - KEY_WIDTH - 28)
      view.scrollLeft = x - view.clientWidth + KEY_WIDTH + 28;
    if (y < view.scrollTop) view.scrollTop = y;
    if (y + ROW > view.scrollTop + view.clientHeight - RULER_HEIGHT)
      view.scrollTop = y + ROW + RULER_HEIGHT - view.clientHeight;
  }

  function markerPosition(note: RollNote) {
    const start = cycleNumber(note.visibleStart ?? note.start);
    const end = cycleNumber(note.visibleEnd ?? note.end);
    return { left: timePercent(start), width: timePercent(end) - timePercent(start) };
  }

  const ghost = $derived.by(() => {
    const note = gesture?.note;
    const edit = gesture?.edit;
    if (!note || !edit) return;
    const start = edit.kind === 'move' ? cycleNumber(edit.start) : cycleNumber(note.start);
    const end =
      edit.kind === 'resize'
        ? cycleNumber(edit.end)
        : start + (cycleNumber(note.end) - cycleNumber(note.start));
    return {
      pitch: edit.kind === 'pitch' ? edit.pitch : note.pitch,
      left: timePercent(start),
      width: timePercent(end) - timePercent(start),
    };
  });

  const cursorPosition = $derived.by(() => {
    if (!cursor) return;
    const bar = geometry.bars[cursor.bar];
    if (!bar || cursor.step >= bar.numerator * subdivision) return;
    return {
      left: timePercent(cycleNumber(gridPoint(bar, cursor.step, subdivision))),
      width: percent(bar.width / (bar.numerator * subdivision)),
      top: laneY(cursor.pitch),
    };
  });

  const playheadVisible = $derived(
    playhead !== undefined &&
      geometry.bars.length > 0 &&
      playhead >= geometry.bars[0]!.cycle &&
      playhead < geometry.bars.at(-1)!.cycle + 1,
  );
</script>

{#if geometry.bars.length}
  <div class="roll">
    <div class="commands">
      {#if editable}
        <label class="snap">
          Snap
          <select bind:value={subdivision} onchange={() => (cursor = undefined)} class="control">
            <option value={1}>Beat</option>
            <option value={2}>½ beat</option>
            <option value={3}>Beat triplet</option>
            <option value={4}>¼ beat</option>
          </select>
        </label>
      {/if}
      {#if editable && insertable}
        <Button
          size="sm"
          variant="quiet"
          title="Double-click an empty cell or press Enter at the placement cursor."
          onclick={() => placeButton?.focus()}>Add note</Button
        >
      {/if}
      {#if editable && visible.some((note) => note.timeEditable)}
        <span
          class="keys-help"
          title="Arrow keys move or retune. Shift and left or right resizes. Delete removes a note. Hold Alt to bypass the active snap while dragging."
        >
          ↑↓ Pitch · ←→ Timing · ⇧←→ Length
        </span>
      {/if}
      {#if selectedNote && canShowSource(selectedNote)}
        <Button
          size="sm"
          variant="quiet"
          class="source-link"
          onclick={() => onsource?.(selectedNote)}
        >
          Go to source
        </Button>
      {/if}
    </div>

    <div class="viewport" bind:this={viewport}>
      <div
        class="sheet"
        style:--roll-row={ROW + 'px'}
        style:--roll-key-width={KEY_WIDTH + 'px'}
        style:--roll-ruler-height={RULER_HEIGHT + 'px'}
        style:--roll-min-width={Math.max(240, Math.ceil(geometry.width * 25)) + 'px'}
      >
        <div class="corner" aria-hidden="true">Pitch</div>
        <div class="ruler">
          {#each geometry.bars as bar (bar.cycle)}
            <div
              class="measure"
              style:left={percent(bar.offset) + '%'}
              style:width={percent(bar.width) + '%'}
            >
              <span class="bar-label">{bar.label}</span>
              <span class="meter">{bar.numerator}/{bar.denominator}</span>
            </div>
          {/each}
        </div>

        <div class="piano">
          {#each pitches as pitch (pitch)}
            <button
              class="piano-key"
              class:black={isBlackKey(pitch)}
              class:octave={pitch % 12 === 0}
              type="button"
              aria-label={'Audition ' + pitchLabel(pitch)}
              tabindex={pitch === keyboardPitch ? 0 : -1}
              onclick={() => onaudition(pitch)}
              onfocus={() => (keyboardPitch = pitch)}
              onkeydown={(event) => {
                if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
                const neighbor =
                  event.key === 'ArrowUp' ? 'previousElementSibling' : 'nextElementSibling';
                (event.currentTarget[neighbor] as HTMLButtonElement | null)?.focus();
                event.preventDefault();
              }}>{pitchLabel(pitch)}</button
            >
          {/each}
        </div>

        <div class="surface" bind:this={surface} style:height={pitches.length * ROW + 'px'}>
          {#each pitches as pitch (pitch)}
            <div
              class="lane"
              class:black={isBlackKey(pitch)}
              class:octave={pitch % 12 === 0}
              style:top={laneY(pitch) + 'px'}
              aria-hidden="true"
            ></div>
          {/each}
          {#each geometry.bars as bar (bar.cycle)}
            <div
              class="beat-grid"
              style:left={percent(bar.offset) + '%'}
              style:width={percent(bar.width) + '%'}
              style:--roll-beat={100 / bar.numerator + '%'}
              style:--roll-subbeat={100 / (bar.numerator * (editable ? subdivision : 1)) + '%'}
              class:compound={bar.numerator > 3 && bar.numerator % 3 === 0 && bar.denominator >= 8}
              aria-hidden="true"
            ></div>
          {/each}
          {#if editable && insertable}
            <button
              class="placement"
              type="button"
              bind:this={placeButton}
              aria-label="Place a note. Arrows move the placement cursor. Enter adds."
              onfocus={showCursor}
              onclick={pointCursor}
              ondblclick={() => insertNote()}
              onkeydown={cursorKeys}
            ></button>
            {#if cursorPosition}
              <div
                class="placement-cursor"
                style:left={cursorPosition.left + '%'}
                style:width={cursorPosition.width + '%'}
                style:top={cursorPosition.top + 'px'}
                aria-hidden="true"
              ></div>
            {/if}
          {/if}
          {#each markers as { note, track, tracks } (note.id)}
            {@const mark = markerPosition(note)}
            {@const height = (ROW - 4) / tracks}
            {@const origin = sourceKey(note)}
            {@const linked = origin === undefined ? 0 : (sourceCounts.get(origin) ?? 0)}
            {@const details = linked > 1 ? ` · pitch shared by ${linked} notes` : ''}
            <button
              type="button"
              class="note"
              data-note-id={note.id}
              class:editable={editable && (note.pitchEditable || note.timeEditable)}
              class:continued={cycleNumber(note.start) < windowStart ||
                (note.visibleStart != null && note.visibleStart !== note.start)}
              class:cropped={cycleNumber(note.end) > windowEnd ||
                (note.visibleEnd != null && note.visibleEnd !== note.end)}
              class:linked={selected === note.id ||
                (origin !== undefined && origin === selectedSource)}
              class:source-match={note.sourceMatch}
              class:pending={gesture?.axis !== undefined && gesture?.note === note}
              style:left={mark.left + '%'}
              style:width={mark.width + '%'}
              style:top={laneY(note.pitch) + 2 + track * height + 'px'}
              style:height={height + 'px'}
              title={pitchLabel(note.pitch) + ' · ' + note.start + ' to ' + note.end + details}
              aria-label={pitchLabel(note.pitch) +
                ', cycle ' +
                note.start +
                ' to ' +
                note.end +
                details +
                (note.sourceMatch ? ' · matches selection source' : '')}
              onclick={(event) => activateNote(event, note)}
              onfocus={() => (selected = note.id)}
              onkeydown={(event) => noteKeys(event, note)}
              onpointerdown={(event) => beginPointer(event, note)}
              onpointermove={movePointer}
              onpointerup={endPointer}
              onpointercancel={(event) => endPointer(event, true)}
              onlostpointercapture={(event) => endPointer(event, true)}
            >
              <span>{pitchLabel(note.pitch)}</span>
              {#if editable && note.timeEditable}
                <span class="resize" aria-hidden="true"></span>
              {/if}
            </button>
          {/each}
          {#if ghost}
            <div
              class="ghost"
              style:left={ghost.left + '%'}
              style:width={ghost.width + '%'}
              style:top={laneY(Math.max(pitchRange.low, Math.min(pitchRange.high, ghost.pitch))) +
                2 +
                'px'}
              style:height={ROW - 4 + 'px'}
              aria-hidden="true"
            >
              {pitchLabel(ghost.pitch)}
            </div>
          {/if}
          {#if playheadVisible && playhead !== undefined}
            <div class="head" style:left={timePercent(playhead) + '%'} aria-hidden="true"></div>
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .roll {
    min-width: 0;
    padding-inline: var(--space-3);
    padding-block-end: var(--space-2);
  }

  .commands {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    margin-block-end: var(--space-1);
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }

  .commands:empty {
    display: none;
  }

  .snap {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    white-space: nowrap;
  }

  .snap select {
    padding: 0 var(--space-1);
    height: var(--control-sm);
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .keys-help {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }

  .commands :global(.source-link) {
    margin-inline-start: auto;
  }

  .viewport {
    max-block-size: min(32dvh, 240px);
    overflow: auto;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xs);
    overscroll-behavior: contain;
    background: var(--well);
  }

  .sheet {
    display: grid;
    grid-template-columns: var(--roll-key-width) minmax(var(--roll-min-width), 1fr);
    grid-template-rows: var(--roll-ruler-height) auto;
    min-inline-size: max-content;
  }

  .corner,
  .ruler {
    position: sticky;
    top: 0;
    z-index: var(--z-playhead);
    border-block-end: 1px solid var(--border-strong);
    background: var(--surface);
  }

  .corner {
    left: 0;
    z-index: var(--z-pop);
    display: grid;
    place-items: center;
    border-inline-end: 1px solid var(--border-strong);
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }

  .ruler {
    min-width: 0;
  }

  .measure {
    position: absolute;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    box-sizing: border-box;
    height: 100%;
    overflow: hidden;
    border-inline-start: 1px solid var(--border-strong);
    padding-inline: var(--space-1-5);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }

  .bar-label {
    color: var(--text);
  }

  .meter {
    color: var(--text-dim);
  }

  .piano {
    position: sticky;
    left: 0;
    z-index: var(--z-app);
    border-inline-end: 1px solid var(--border-strong);
    background: var(--surface-2);
  }

  .piano-key {
    display: block;
    box-sizing: border-box;
    inline-size: 100%;
    block-size: var(--roll-row);
    border: 0;
    border-block-end: 1px solid var(--border);
    padding: 0 var(--space-2) 0 0;
    background: var(--surface-3);
    color: var(--text-dim);
    font: var(--fs-xs) var(--font-mono);
    text-align: end;
  }

  .piano-key.black {
    border-inline-start: 8px solid var(--surface-3);
    background: var(--bg);
    color: var(--text-faint);
  }

  .piano-key.octave {
    color: var(--text);
  }

  .piano-key:hover,
  .piano-key:focus-visible {
    background: var(--focus-wash);
    color: var(--text);
  }

  .surface {
    position: relative;
    isolation: isolate;
    overflow: hidden;
  }

  .lane,
  .beat-grid,
  .placement,
  .placement-cursor,
  .note,
  .ghost,
  .head {
    position: absolute;
  }

  .lane {
    inline-size: 100%;
    block-size: var(--roll-row);
    box-sizing: border-box;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
  }

  .lane.black {
    background: var(--well);
  }

  .lane.octave {
    box-shadow: inset 0 -1px var(--border-strong);
  }

  .beat-grid {
    inset-block: 0;
    background-image:
      linear-gradient(to right, var(--border-strong), var(--border-strong)),
      repeating-linear-gradient(to right, var(--border) 0 1px, transparent 1px var(--roll-beat)),
      repeating-linear-gradient(
        to right,
        var(--grid-dot) 0 1px,
        transparent 1px var(--roll-subbeat)
      );
    background-size:
      1px 100%,
      auto,
      auto;
    background-repeat: no-repeat;
    pointer-events: none;
  }

  .beat-grid.compound {
    background-image:
      linear-gradient(to right, var(--border-strong), var(--border-strong)),
      repeating-linear-gradient(
        to right,
        var(--border-strong) 0 1px,
        transparent 1px calc(3 * var(--roll-beat))
      ),
      repeating-linear-gradient(to right, var(--border) 0 1px, transparent 1px var(--roll-beat)),
      repeating-linear-gradient(
        to right,
        var(--grid-dot) 0 1px,
        transparent 1px var(--roll-subbeat)
      );
    background-size:
      1px 100%,
      auto,
      auto,
      auto;
  }

  .placement {
    inset: 0;
    border: 0;
    background: transparent;
    cursor: crosshair;
  }

  .placement:focus-visible {
    outline-offset: -2px;
  }

  .placement-cursor {
    display: none;
    z-index: var(--z-app);
    box-sizing: border-box;
    height: var(--roll-row);
    border: 1px dashed var(--focus);
    pointer-events: none;
  }

  .placement:focus + .placement-cursor {
    display: block;
  }

  .note,
  .ghost {
    z-index: var(--z-app);
    display: flex;
    align-items: center;
    box-sizing: border-box;
    min-width: 3px;
    border: 1px solid var(--focus-edge);
    border-radius: var(--radius-xs);
    background: var(--surface-3);
    color: var(--text);
  }

  .note {
    justify-content: space-between;
    overflow: hidden;
    padding: 0 var(--space-1);
    cursor: pointer;
    font: var(--fs-2xs) var(--font-mono);
    text-align: start;
    white-space: nowrap;
  }

  .note.editable {
    border-color: var(--accent-edge);
    background: var(--accent-wash);
    touch-action: none;
  }

  .note.editable:hover,
  .note.editable.linked {
    border-color: var(--accent);
    color: var(--accent);
  }

  .note.linked,
  .note.source-match,
  .note:focus-visible {
    z-index: var(--z-playhead);
    outline-offset: 0;
    border-color: var(--focus);
  }

  .note.continued {
    border-inline-start-style: dashed;
    border-start-start-radius: 0;
    border-end-start-radius: 0;
  }

  .note.cropped {
    border-inline-end-style: dashed;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }

  .note.pending {
    opacity: 0.45;
  }

  .resize {
    position: absolute;
    inset-block: 2px;
    inset-inline-end: 2px;
    width: 3px;
    border-inline-end: 1px solid var(--accent-edge);
    cursor: ew-resize;
  }

  .ghost {
    border-style: dashed;
    border-color: var(--accent);
    padding-inline: var(--space-1);
    background: var(--accent-wash);
    font: var(--fs-2xs) var(--font-mono);
    pointer-events: none;
  }

  .head {
    inset-block: 0;
    z-index: var(--z-playhead);
    width: 1px;
    background: var(--accent);
    pointer-events: none;
  }
</style>

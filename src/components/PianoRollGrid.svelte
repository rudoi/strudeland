<script lang="ts">
  import { helpTarget } from '../lib/help/targets';
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
    noteMarkerKey,
    pitchLabel,
    rollGeometry,
    rollOffset,
    shiftRollEdge,
    snapGrid,
    type GridBar,
    type PitchRange,
  } from '../lib/piano-roll/grid';
  import { projectRollEdit, sameRollPosition } from '../lib/piano-roll/feedback';
  import type { RollBar, RollEdit, RollNote } from '../lib/piano-roll/types';
  import Button from './ui/Button.svelte';

  let {
    notes,
    bars,
    editable,
    insertable,
    gestureKey,
    focusedBar,
    gridSteps,
    isFlashing,
    oncancel,
    playhead,
    onedit,
    onaudition,
    onsource,
  }: {
    notes: readonly RollNote[];
    bars: readonly RollBar[];
    editable: boolean;
    insertable: boolean;
    gestureKey: string;
    focusedBar?: number;
    gridSteps?: number;
    isFlashing: (note: RollNote) => boolean;
    oncancel: () => void;
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
  let ignoreClick = false;
  let scrollFrame = 0;

  interface Gesture {
    pointer: number;
    note: RollNote;
    key: string;
    range: PitchRange;
    x: number;
    y: number;
    scrollY: number;
    grabOffset: number;
    anchor: string;
    resize: boolean;
    axis?: 'pitch' | 'time';
    lastX: number;
    lastY: number;
    free: boolean;
    edit?: Exclude<RollEdit, { kind: 'insert' }>;
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
  const stepsInBar = (bar: GridBar) => gridSteps ?? bar.numerator * subdivision;

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
        cursor.step >= stepsInBar(geometry.bars[cursor.bar]!) ||
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

  function shownEdge(note: RollNote, right: boolean): string {
    if (right) {
      const end = note.visibleEnd ?? note.end;
      return cycleNumber(end) > windowEnd ? String(windowEnd) : end;
    }
    const start = note.visibleStart ?? note.start;
    return cycleNumber(start) < windowStart ? String(windowStart) : start;
  }

  function beginPointer(event: PointerEvent, note: RollNote) {
    if (event.button !== 0 || !event.isPrimary) return;
    ignoreClick = false;
    if (!editable || (!note.pitchEditable && !note.timeEditable)) return;
    const button = event.currentTarget as HTMLButtonElement;
    const rect = button.getBoundingClientRect();
    const resize = Boolean(note.timeEditable && rect.width > 16 && rect.right - event.clientX <= 6);
    const anchor = shownEdge(note, resize);
    gesture = {
      pointer: event.pointerId,
      note,
      key: gestureKey,
      range: pitchRange,
      x: event.clientX,
      y: event.clientY,
      scrollY: viewport!.scrollTop,
      grabOffset: pointerOffset(event.clientX) - rollOffset(geometry.bars, cycleNumber(anchor)),
      anchor,
      resize,
      lastX: event.clientX,
      lastY: event.clientY,
      free: event.altKey,
    };
    button.setPointerCapture(event.pointerId);
  }

  function timeEdit(current: Gesture): Gesture['edit'] {
    const { note, resize, lastX, grabOffset, anchor, free } = current;
    const start = cycleNumber(note.start);
    const end = cycleNumber(note.end);
    const offset = pointerOffset(lastX) - grabOffset;
    const bar = barAtOffset(geometry.bars, offset);
    if (!bar) return;
    const steps = free ? bar.numerator * FREE_STEPS : stepsInBar(bar);
    const lastBar = bar === geometry.bars.at(-1);
    const upper = !resize && lastBar ? 1 - 1 / steps : 1;
    const snapped = snapGrid(bar, offset, steps, upper);
    const point = shiftRollEdge(resize ? note.end : note.start, anchor, snapped);
    const value = cycleNumber(point);
    if (resize)
      return value > start && value !== end
        ? { kind: 'resize', id: note.id, end: point }
        : undefined;
    return value >= 0 && value !== start ? { kind: 'move', id: note.id, start: point } : undefined;
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
    editable &&
    current.key === gestureKey &&
    notes.some((note) => note.id === current.note.id && sameRollPosition(note, current.note));

  $effect(() => {
    const current = gesture;
    if (!current || stillCurrent(current)) return;
    gesture = undefined;
    ignoreClick = Boolean(current.axis);
    cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
    if (current.axis) oncancel();
  });

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
      ignoreClick = true;
      if (current.axis && stillCurrent(current)) {
        const { edit } = updateGesture({
          ...current,
          lastX: event.clientX,
          lastY: event.clientY,
          free: event.altKey,
        });
        if (edit && onedit(edit)) {
          void restoreNoteFocus(event.currentTarget as HTMLButtonElement, current.note, edit);
        }
      }
    }
  }

  function noteHelp(note: RollNote): string {
    if (editable && note.timeEditable) {
      return note.pitchEditable
        ? 'Drag to move or retune. Shift+Left or Right resizes. Delete removes.'
        : 'Drag to move in time. Shift+Left or Right resizes. Delete removes.';
    }
    if (editable && note.pitchEditable) return 'Drag up or down to change pitch.';
    if (canShowSource(note)) return 'Press Shift+Enter to find this note’s source in the editor.';
    return 'Preview this note while playback and recording are idle. This note is read-only.';
  }

  function activateNote(event: MouseEvent, note: RollNote) {
    if (ignoreClick && event.detail !== 0) {
      ignoreClick = false;
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
        const resize = event.shiftKey;
        const delta = event.key === 'ArrowRight' ? 1 : -1;
        const original = resize ? note.end : note.start;
        const anchor = shownEdge(note, resize);
        const edge = cycleNumber(anchor);
        const bar =
          geometry.bars.find((bar) => edge >= bar.cycle && edge < bar.cycle + 1) ??
          (edge === windowEnd ? geometry.bars.at(-1) : undefined);
        if (bar) {
          const step = adjacentStep(bar, edge, stepsInBar(bar), delta);
          const previous = geometry.bars[geometry.bars.indexOf(bar) - 1];
          const snapped =
            step < 0 && previous
              ? gridPoint(previous, stepsInBar(previous) - 1, stepsInBar(previous))
              : gridPoint(bar, step, stepsInBar(bar));
          const point = shiftRollEdge(original, anchor, snapped);
          const value = cycleNumber(point);
          if (resize && value > cycleNumber(note.start) && cycleNumber(snapped) <= windowEnd)
            edit = { kind: 'resize', id: note.id, end: point };
          if (!resize && value >= 0 && cycleNumber(snapped) < windowEnd)
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
    const focused = document.activeElement;
    if ((focused !== document.body && focused !== button) || !surface) return;
    const destination = projectRollEdit([previous], edit, false).edited[0];
    const next = destination
      ? (visible.find((note) => note.id === previous.id && sameRollPosition(note, destination)) ??
        visible.find((note) => isFlashing(note) && sameRollPosition(note, destination)) ??
        visible.find((note) => sameRollPosition(note, destination)))
      : (visible.find((note) => cycleNumber(note.start) >= cycleNumber(previous.start)) ??
        visible.at(-1));
    if (next) {
      Array.from(surface.querySelectorAll<HTMLButtonElement>('[data-note-id]'))
        .find((candidate) => candidate.dataset.noteId === next.id)
        ?.focus();
    } else if (button.isConnected && button.dataset.noteId === previous.id) {
      button.focus();
    } else {
      (placeButton ?? surface.querySelector<HTMLButtonElement>('[data-note-id]'))?.focus();
    }
  }

  function showCursor() {
    if (cursor || !geometry.bars.length) return;
    const bar = geometry.bars.findIndex((bar) => bar.cycle === focusedBar);
    cursor = { bar: Math.max(0, bar), step: 0, pitch: keyboardPitch };
  }

  function pointCursor(event: MouseEvent) {
    const offset = pointerOffset(event.clientX);
    const bar = barAtOffset(geometry.bars, offset);
    if (!bar) return;
    const steps = stepsInBar(bar);
    const step = Math.max(
      0,
      Math.min(steps - 1, Math.floor(((offset - bar.offset) / bar.width) * steps)),
    );
    cursor = { bar: geometry.bars.indexOf(bar), step, pitch: pitchAt(event.clientY) };
  }

  function insertNote() {
    if (!cursor || !editable || !insertable) return;
    const bar = geometry.bars[cursor.bar];
    if (!bar || cursor.step >= stepsInBar(bar)) return;
    onedit({
      kind: 'insert',
      start: gridPoint(bar, cursor.step, stepsInBar(bar)),
      end: gridPoint(bar, cursor.step + 1, stepsInBar(bar)),
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
        else if (bar > 0) step = stepsInBar(geometry.bars[--bar]!) - 1;
        break;
      case 'ArrowRight':
        if (step + 1 < stepsInBar(geometry.bars[bar]!)) step++;
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
    const position = cycleNumber(gridPoint(targetBar, step, stepsInBar(targetBar)));
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
    if (!bar || cursor.step >= stepsInBar(bar)) return;
    return {
      left: timePercent(cycleNumber(gridPoint(bar, cursor.step, stepsInBar(bar)))),
      width: percent(bar.width / stepsInBar(bar)),
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
      {#if editable && gridSteps !== undefined}
        <span class="snap">Snap · {gridSteps} slots per bar</span>
      {:else if editable}
        <label class="snap">
          Snap
          <select
            use:helpTarget={'Set the beat division used to place and move notes.'}
            bind:value={subdivision}
            onchange={() => (cursor = undefined)}
            class="control"
          >
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
          help="Focus the placement cursor. Move it with the arrow keys and press Enter to add a note."
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
          help="Jump to the code for the selected note."
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
              class:focused={bar.cycle === focusedBar}
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
              use:helpTarget={'Preview this pitch while playback and recording are idle. Use Up or Down to choose a key.'}
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
              style:--roll-subbeat={100 / (editable ? stepsInBar(bar) : bar.numerator) + '%'}
              class:compound={bar.numerator > 3 && bar.numerator % 3 === 0 && bar.denominator >= 8}
              aria-hidden="true"
            ></div>
          {/each}
          {#if editable && insertable}
            <button
              class="placement"
              type="button"
              bind:this={placeButton}
              use:helpTarget={'Double-click an empty cell to add a note at that pitch and beat.'}
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
          {#each markers as { note, track, tracks } (noteMarkerKey(note, track))}
            {@const mark = markerPosition(note)}
            {@const flashing = isFlashing(note)}
            {@const height = (ROW - 4) / tracks}
            {@const origin = sourceKey(note)}
            {@const linked = origin === undefined ? 0 : (sourceCounts.get(origin) ?? 0)}
            {@const details = linked > 1 ? ` · pitch shared by ${linked} notes` : ''}
            <button
              type="button"
              class="note"
              data-note-id={note.id}
              use:helpTarget={noteHelp(note)}
              data-roll-edited={flashing ? '' : undefined}
              class:flashing
              class:editable={editable && (note.pitchEditable || note.timeEditable)}
              class:continued={cycleNumber(note.start) < windowStart ||
                (note.visibleStart != null && note.visibleStart !== note.start)}
              class:cropped={cycleNumber(note.end) > windowEnd ||
                (note.visibleEnd != null && note.visibleEnd !== note.end)}
              class:linked={selected === note.id ||
                (origin !== undefined && origin === selectedSource)}
              class:source-match={note.sourceMatch}
              class:pending={gesture?.axis !== undefined && gesture?.note.id === note.id}
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

  .measure.focused .bar-label {
    color: var(--accent);
  }

  .note.flashing {
    z-index: var(--z-playhead);
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  @media (prefers-reduced-motion: no-preference) {
    .note.flashing {
      animation: roll-edit-pulse var(--dur-edit-feedback) var(--ease);
    }
  }

  @keyframes roll-edit-pulse {
    from {
      outline-offset: 4px;
    }
    to {
      outline-offset: 1px;
    }
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

<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { readable } from 'svelte/store';
  import type { App } from '../lib/app';
  import type { PianoRollSnapshot } from '../lib/project/session';
  import { takeRunning } from '../lib/recording/memo-ui';
  import { MAX_BARS, meterAtBar, type Section } from '../lib/strudel/structure';
  import { planRollEdit, resolveRollTarget } from '../lib/piano-roll/source';
  import { planEventEdit, preparePhraseEvents } from '../lib/piano-roll/event-source';
  import { cycleNumber } from '../lib/piano-roll/grid';
  import { notePosition, projectRollEdit, sameRollPosition } from '../lib/piano-roll/feedback';
  import type {
    InspectionRequest,
    RollBar,
    RollEdit,
    RollNote,
    RollTarget,
    SourceSpan,
  } from '../lib/piano-roll/types';
  import Button from './ui/Button.svelte';
  import PianoRollGrid from './PianoRollGrid.svelte';

  let { app }: { app: App } = $props();
  // svelte-ignore state_referenced_locally
  const { session, runtime, rollInspection, scopeClock, recording } = app;
  const song = session.store;
  const transport = runtime.store;
  const revisions = session.pianoRollRevision;
  const inspected = rollInspection.store;
  const recorder = recording?.store ?? readable(null);
  const windowBars = 4;
  const emptyNotes: readonly RollNote[] = [];
  const activePath = $derived($song.active);

  let mode = $state<'expression' | 'song'>('expression');
  let page = $state.raw<{
    context: string;
    gridRow?: number;
    index: number;
    mode: typeof mode;
  } | null>(null);
  let anchorChoice = $state.raw<{ context: string; name: string } | null>(null);
  let shared = $state.raw<{ context: string; name: string } | null>(null);
  let approvedDraft = $state.raw<{ song: string; code: string }>();
  let retained = $state.raw<{ snapshot: PianoRollSnapshot; target: RollTarget }>();
  let optimistic = $state.raw<{ key: string; notes: readonly RollNote[] }>();
  let feedback = $state.raw<{ key: string; notes: readonly ReturnType<typeof notePosition>[] }>();
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;
  let notice = $state('');
  let contributors = $state.raw<{ spans: readonly SourceSpan[]; section?: Section } | null>(null);
  let playhead = $state<number>();

  const snapshot = $derived.by(() => {
    const revision = $revisions;
    const path = activePath;
    if (!path) return null;
    const current = session.pianoRollSnapshot();
    return current?.revision === revision && current.path === path ? current : null;
  });
  const songKey = $derived(snapshot ? JSON.stringify([snapshot.root, snapshot.path]) : '');
  const target = $derived.by(() => {
    if (!snapshot?.selection) return null;
    if (
      retained?.snapshot.sourceRevision === snapshot.sourceRevision &&
      retained.snapshot.selectionRevision === snapshot.selectionRevision
    )
      return retained.target;
    return resolveRollTarget(snapshot.code, snapshot.selection);
  });
  const structure = $derived($song.structure);
  const definition = $derived(target?.definition);
  const contextKey = $derived(
    target ? JSON.stringify([songKey, target.expression.from, target.expression.to]) : '',
  );
  const definitionMode = $derived(
    definition && shared?.context === contextKey && shared.name === definition.name,
  );
  const expressionFrom = $derived(
    (definitionMode ? definition?.expression : target?.expression)?.from,
  );
  const expressionTo = $derived((definitionMode ? definition?.expression : target?.expression)?.to);
  const relevantSections = $derived.by(() => {
    const names = new Set(target?.sections);
    for (const use of definition?.uses ?? []) {
      for (const name of use.sections) names.add(name);
    }
    if (anchorChoice?.context === contextKey && anchorChoice) names.add(anchorChoice.name);
    return structure?.sections.filter((section) => names.has(section.name)) ?? [];
  });
  const anchor = $derived.by(() => {
    if (anchorChoice?.context === contextKey) {
      const chosen = relevantSections.find((section) => section.name === anchorChoice?.name);
      if (chosen) return chosen;
    }
    return (
      relevantSections.find((section) => target?.sections.includes(section.name)) ??
      relevantSections[0]
    );
  });

  const phrase = $derived(
    mode === 'expression' ? (definitionMode ? definition?.phrase : target?.phrase) : undefined,
  );
  const editTarget = $derived(
    mode !== 'expression' ||
      !phrase ||
      expressionFrom === undefined ||
      expressionTo === undefined ||
      (!definitionMode &&
        definition &&
        (definition.uses.length || definition.incomplete) &&
        (definition.expression.from !== expressionFrom ||
          definition.expression.to !== expressionTo))
      ? undefined
      : { from: expressionFrom, to: expressionTo },
  );
  const gridBars = $derived(mode === 'expression' ? target?.gridBars : undefined);
  const defaultPage = $derived(
    gridBars === undefined
      ? 0
      : Math.min(Math.max(0, (target?.gridRow ?? 0) - 1), Math.max(0, gridBars - windowBars)),
  );
  const focusedPage = $derived(
    page?.context === contextKey && page.mode === mode && page.gridRow === target?.gridRow
      ? page.index
      : defaultPage,
  );
  const anchorBar = $derived(anchor?.start);
  const origin = $derived(mode === 'song' ? (anchorBar ?? 0) : 0);
  const from = $derived(origin + focusedPage * (gridBars === undefined ? windowBars : 1));
  const maxCycle = $derived(
    phrase?.bars ??
      (mode === 'song' ? structure?.totalBars || MAX_BARS : (gridBars ?? anchor?.bars ?? MAX_BARS)),
  );
  const to = $derived(Math.min(from + windowBars, maxCycle));
  const bars = $derived.by<RollBar[]>(() => {
    if (!structure) return [];
    return Array.from({ length: Math.max(0, to - from) }, (_, i) => {
      const cycle = from + i;
      const written = mode === 'song' ? cycle : anchorBar === undefined ? null : anchorBar + cycle;
      const meter = written === null ? structure.meter : meterAtBar(structure, written);
      return {
        cycle,
        label:
          mode === 'song' || gridBars !== undefined ? `Bar ${cycle + 1}` : `Cycle ${cycle + 1}`,
        numerator: meter.num,
        denominator: meter.den,
      };
    });
  });
  const matchesLive = $derived(
    snapshot &&
      $song.evaluated?.path === snapshot.path &&
      $transport.activeCode === snapshot.code &&
      session.pianoRollMatchesLivePins(),
  );
  const needsInspection = $derived(Boolean(target && (mode === 'song' || !phrase)));
  const draftApproved = $derived(
    approvedDraft?.song === songKey && approvedDraft?.code === snapshot?.code,
  );
  const viewKey = $derived(
    JSON.stringify([contextKey, mode, anchorBar, from, to, expressionFrom, expressionTo]),
  );
  const sourceRevision = $derived(snapshot?.sourceRevision);
  const gestureKey = $derived(`${sourceRevision}:${viewKey}`);
  const inspectionKey = $derived(
    needsInspection && rollInspection.available && (matchesLive || draftApproved) && to > from
      ? gestureKey
      : '',
  );
  const request = $derived.by<InspectionRequest | null>(() => {
    if (!inspectionKey) return null;
    // The key excludes caret motion inside the working expression.
    return untrack(() =>
      snapshot && expressionFrom !== undefined && expressionTo !== undefined
        ? {
            root: snapshot.root,
            code: snapshot.code,
            view: mode,
            expression: { from: expressionFrom, to: expressionTo },
            from,
            to,
            anchorBar,
          }
        : null,
    );
  });
  const response = $derived($inspected?.request === request ? $inspected : null);
  const inspectionNotes = $derived(
    response?.phase === 'ready' ? response.result.notes : emptyNotes,
  );
  const baseNotes = $derived.by(() => {
    if (phrase) return preparePhraseEvents(phrase.notes);
    if (mode !== 'song') return inspectionNotes;
    return inspectionNotes.map((note) => ({
      ...note,
      sourceMatch: [note.origin, ...(note.contributors ?? [])].some(
        (span) =>
          span &&
          expressionFrom !== undefined &&
          expressionTo !== undefined &&
          span.from >= expressionFrom &&
          span.to <= expressionTo,
      ),
    }));
  });
  const preparedNotes = $derived(
    baseNotes
      .filter((note) => note.event || (editTarget && note.pitchEditable))
      .map((note) => ({
        ...note,
        pitchEditable: Boolean(note.event || (editTarget && note.pitchEditable)),
        timeEditable: Boolean(note.event || (editTarget && note.timeEditable)),
      })),
  );
  const showingOptimistic = $derived(
    !phrase && response?.phase !== 'ready' && optimistic?.key === gestureKey,
  );
  const notes = $derived(showingOptimistic ? (optimistic?.notes ?? emptyNotes) : preparedNotes);
  const omitted = $derived(baseNotes.length - preparedNotes.length);
  const sourceMatches = $derived(mode === 'song' && notes.some((note) => note.sourceMatch));
  const auditionSound = $derived.by(() => {
    if (phrase?.sound) return phrase.sound;
    const sounds = new Set(notes.map((note) => note.sound));
    return sounds.size === 1 ? sounds.values().next().value : undefined;
  });

  $effect(() => rollInspection.request(request));
  $effect(() => {
    playhead = undefined;
    if (mode !== 'song' || !matchesLive || !$transport.started) return;
    const arranged = Boolean($transport.structure?.sections.length);
    // The recorder's unsectioned backing can start one cycle after the transport.
    if (!arranged && takeRunning($recorder?.phase ?? null)) return;
    return scopeClock.subscribe((_, running) => {
      if (!running) {
        playhead = undefined;
        return;
      }
      const position = runtime.position();
      playhead = arranged ? position.located?.songBar : position.cycle;
    });
  });
  $effect(() => {
    const key = gestureKey;
    untrack(() => {
      notice = '';
      contributors = null;
      if (optimistic?.key !== key) optimistic = undefined;
      if (feedback && feedback.key !== key) clearFeedback();
    });
  });

  onDestroy(() => {
    rollInspection.request(null);
    app.stopPianoRollAudition();
    clearFeedback();
  });

  function clearFeedback() {
    clearTimeout(feedbackTimer);
    feedback = undefined;
    session.clearPianoRollFeedback();
  }

  function approveDraft() {
    if (snapshot) approvedDraft = { song: songKey, code: snapshot.code };
  }

  function changeAnchor(section: Section | undefined) {
    if (snapshot && section) {
      anchorChoice = { context: contextKey, name: section.name };
      page = null;
    }
  }

  function goToPage(index: number) {
    if (snapshot) page = { context: contextKey, gridRow: target?.gridRow, index, mode };
  }

  function openDefinition() {
    if (!snapshot || !definition) return;
    mode = 'expression';
    shared = { context: contextKey, name: definition.name };
    page = null;
  }

  function reveal(context: PianoRollSnapshot, span: SourceSpan, section?: Section) {
    mode = 'expression';
    session.revealPianoRoll(context, span);
    if (section) anchorChoice = { context: contextKey, name: section.name };
  }

  function goToNote(note: RollNote) {
    if (!snapshot) return;
    const cycle = cycleNumber(note.visibleStart ?? note.start);
    const section = structure?.sections.find(
      ({ start, bars }) => cycle >= start && cycle < start + bars,
    );
    if (note.origin) reveal(snapshot, note.origin, section);
    else contributors = { spans: note.contributors ?? [], section };
  }

  async function audition(pitch: number, sound?: string, automatic = false) {
    const revision = snapshot?.revision;
    const voice = await app.auditionPianoRoll(pitch, sound, automatic);
    if (voice && (automatic || revision === snapshot?.revision)) {
      notice = voice === 'triangle' ? 'Triangle preview' : '';
    }
  }

  function edit(change: RollEdit) {
    if (!snapshot || !target || expressionFrom === undefined || expressionTo === undefined)
      return false;
    const current = session.pianoRollSnapshot();
    if (current?.revision !== snapshot.revision) {
      notice = 'The editor changed during this edit.';
      return false;
    }
    const note = 'id' in change ? notes.find((item) => item.id === change.id) : undefined;
    if ('id' in change && !note) return false;
    const expression = { from: expressionFrom, to: expressionTo };
    const direct = editTarget && planRollEdit(snapshot.code, editTarget, change);
    const patches =
      direct ||
      (note &&
        planEventEdit(
          snapshot.code,
          mode === 'song' ? { kind: 'song' } : { kind: 'expression', expression },
          note,
          change,
        ));
    if (!patches) {
      notice = 'This edit does not fit the source pattern.';
      return false;
    }
    const preview = projectRollEdit(notes, change, Boolean(direct));
    const previousPage = focusedPage;
    const previousGridRow = target.gridRow;
    const wasApproved = matchesLive || draftApproved;
    const previousContext = contextKey;
    const result = session.editPianoRoll(snapshot, patches, target.expression);
    if (!result) {
      notice = 'The editor changed during this edit.';
      return false;
    }
    const mapped = result.target;
    const nextTarget = mapped && resolveRollTarget(result.snapshot.code, mapped);
    if (nextTarget) {
      retained = {
        snapshot: result.snapshot,
        target: { ...nextTarget, gridRow: nextTarget.gridRow ?? previousGridRow },
      };
    }
    if (anchorChoice?.context === previousContext)
      anchorChoice = { ...anchorChoice, context: contextKey };
    if (shared?.context === previousContext) shared = { ...shared, context: contextKey };
    goToPage(previousPage);
    if (wasApproved) approvedDraft = { song: songKey, code: result.snapshot.code };
    optimistic = { key: gestureKey, notes: preview.notes };
    feedback = { key: gestureKey, notes: preview.edited.map(notePosition) };
    clearTimeout(feedbackTimer);
    feedbackTimer = setTimeout(clearFeedback, 850);
    notice = '';
    if (change.kind === 'pitch' || change.kind === 'insert') {
      void audition(change.pitch, note?.sound ?? auditionSound, true);
    }
    return true;
  }
</script>

<section class="dock" aria-label="Piano roll" data-piano-roll>
  <div class="roll-toolbar">
    <div class="views" role="group" aria-label="Piano roll view">
      <Button size="sm" pressed={mode === 'expression'} onclick={() => (mode = 'expression')}
        >Expression</Button
      >
      <Button size="sm" pressed={mode === 'song'} onclick={() => (mode = 'song')}>In song</Button>
    </div>
    {#if target}
      <span class="target" title={target.label}
        >{definitionMode ? definition?.name : target.label}</span
      >
      {#if relevantSections.length > 1}
        <label class="anchor">
          <span>Meter at</span>
          <select
            class="control"
            value={anchor?.name}
            onchange={(event) =>
              changeAnchor(
                relevantSections.find((section) => section.name === event.currentTarget.value),
              )}
          >
            {#each relevantSections as section (section.name)}
              <option value={section.name}>{section.name} · bar {section.start + 1}</option>
            {/each}
          </select>
        </label>
      {:else if anchor}
        <span class="anchor">{anchor.name} · bar {anchor.start + 1}</span>
      {:else if mode === 'expression'}
        <span class="anchor">Base meter</span>
      {/if}
      <div class="pages" role="group" aria-label="Visible bars">
        <Button
          size="sm"
          title="Previous bars"
          aria-label="Previous bars"
          disabled={focusedPage === 0}
          onclick={() => goToPage(focusedPage - 1)}>‹</Button
        >
        <Button
          size="sm"
          title="Next bars"
          aria-label="Next bars"
          disabled={to >= maxCycle}
          onclick={() => goToPage(focusedPage + 1)}>›</Button
        >
      </div>
    {/if}
  </div>
  {#if mode === 'expression' && definition?.phrase && (definition.uses.length || definition.incomplete) && snapshot}
    <div class="shared">
      {#if !definitionMode}
        <Button size="sm" onclick={openDefinition}>Edit shared definition</Button>
      {:else}
        <Button size="sm" onclick={() => (shared = null)}>Back to selection</Button>
      {/if}
      <span
        >{definition.uses.length} known {definition.uses.length === 1
          ? 'use'
          : 'uses'}{definition.incomplete ? ' · other uses may be affected' : ''}</span
      >
      {#each definition.uses as use (use.span.from)}
        {@const context = snapshot}
        <Button size="sm" variant="quiet" onclick={() => reveal(context, use.span)}>
          {use.sections.join(', ') ||
            `Line ${context.code.slice(0, use.span.from).split('\n').length}`}
        </Button>
      {/each}
    </div>
  {/if}
  {#if !target}
    <p class="roll-status" role="status">
      Place the caret inside a pitched pattern or select an expression.
    </p>
  {:else if needsInspection && !rollInspection.available && !showingOptimistic}
    <p class="roll-status" role="status">
      Generated and in-song patterns can be inspected in the desktop app.
    </p>
  {:else if needsInspection && !request && !showingOptimistic}
    <div class="preview">
      <Button size="sm" onclick={approveDraft}>Preview expression</Button>
      <span>Inspect the current draft without starting playback.</span>
    </div>
  {:else if response?.phase === 'error' && !showingOptimistic}
    <p class="roll-status" role="status">{response.message}</p>
  {:else if needsInspection && !showingOptimistic && (!response || response.phase === 'loading')}
    <p class="roll-status" role="status">Inspecting pattern…</p>
  {:else}
    {#if showingOptimistic && !request && rollInspection.available}
      <div class="preview">
        <Button size="sm" onclick={approveDraft}>Preview expression</Button>
        <span>Inspect the current draft without starting playback.</span>
      </div>
    {/if}
    {#if response?.phase === 'error'}<p class="roll-status" role="status">
        {response.message}
      </p>{/if}
    {#if sourceMatches || omitted || (response?.phase === 'ready' && response.result.truncated) || !notes.length}
      <p class="roll-status compact" role="status">
        {sourceMatches ? 'Source matches outlined' : ''}{omitted
          ? `${sourceMatches ? ' · ' : ''}${omitted} ${omitted === 1 ? 'event lacks' : 'events lack'} a stable edit identity`
          : ''}{response?.phase === 'ready' && response.result.truncated
          ? `${sourceMatches || omitted ? ' · ' : ''}Results limited`
          : ''}{!notes.length && !omitted ? 'No pitched notes in this window' : ''}
      </p>
    {/if}
    {#if snapshot && contributors?.spans.length}
      <div class="shared">
        <span>Source contributors</span>
        {#each contributors.spans as span (`${span.from}:${span.to}`)}
          {@const context = snapshot}
          <Button
            size="sm"
            variant="quiet"
            onclick={() => reveal(context, span, contributors?.section)}
            >{context.code.slice(span.from, span.to)}</Button
          >
        {/each}
      </div>
    {/if}
    {#if notice}<p class="roll-status compact" role="status">{notice}</p>{/if}
    <PianoRollGrid
      {notes}
      {bars}
      editable={editTarget !== undefined || notes.length > 0}
      {gestureKey}
      focusedBar={mode === 'expression' ? target.gridRow : undefined}
      isFlashing={(note) =>
        Boolean(
          feedback?.key === gestureKey &&
          feedback.notes.some((position) => sameRollPosition(note, position)),
        )}
      oncancel={() => (notice = 'The editor changed during this edit.')}
      insertable={editTarget !== undefined && phrase?.insertable === true}
      {playhead}
      onedit={edit}
      onaudition={(pitch, note) => void audition(pitch, note ? note.sound : auditionSound)}
      onsource={mode === 'song' ? goToNote : undefined}
    />
  {/if}
</section>

<style>
  .dock {
    display: flex;
    flex-direction: column;
    min-height: 0;
    max-height: min(41vh, 19rem);
    overflow: auto;
    background: var(--surface);
    border-top: 1px solid var(--border);
  }
  .roll-toolbar,
  .shared,
  .preview {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    padding: var(--space-1-5) var(--space-3);
    font-size: var(--fs-sm);
  }
  .views,
  .pages,
  .anchor {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .pages {
    margin-inline-start: auto;
  }
  .target {
    min-width: 3rem;
    max-width: 15rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .anchor,
  .shared span,
  .preview span,
  .roll-status {
    color: var(--text-dim);
  }
  .anchor select {
    max-width: 12rem;
  }
  .shared,
  .preview {
    border-top: 1px solid var(--border);
  }
  .roll-status {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    font-size: var(--fs-sm);
  }
  .compact {
    padding-block: var(--space-1);
  }
</style>

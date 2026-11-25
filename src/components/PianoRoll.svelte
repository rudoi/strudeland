<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { App } from '../lib/app';
  import type { PianoRollSnapshot } from '../lib/project/session';
  import { MAX_BARS, meterAtBar, type Section } from '../lib/strudel/structure';
  import { planRollEdit, resolveRollTarget } from '../lib/piano-roll/source';
  import { cycleNumber } from '../lib/piano-roll/grid';
  import type {
    InspectionRequest,
    RollBar,
    RollEdit,
    RollNote,
    SourceSpan,
  } from '../lib/piano-roll/types';
  import Button from './ui/Button.svelte';
  import PianoRollGrid from './PianoRollGrid.svelte';

  let { app }: { app: App } = $props();
  // svelte-ignore state_referenced_locally
  const { session, runtime, rollInspection, scopeClock } = app;
  const song = session.store;
  const transport = runtime.store;
  const revisions = session.pianoRollRevision;
  const inspected = rollInspection.store;
  const windowBars = 4;
  const emptyNotes: readonly RollNote[] = [];
  const activePath = $derived($song.active);

  let mode = $state<'expression' | 'song'>('expression');
  let page = $state.raw<{ selection: number; index: number; mode: typeof mode } | null>(null);
  let anchorChoice = $state.raw<{ selection: number; name: string } | null>(null);
  let shared = $state.raw<{ selection: number; path: string; name: string } | null>(null);
  let approvedRevision = $state<number>();
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
  const target = $derived(
    snapshot?.selection ? resolveRollTarget(snapshot.code, snapshot.selection) : null,
  );
  const structure = $derived($song.structure);
  const definition = $derived(target?.definition);
  const selectionRevision = $derived(snapshot?.selectionRevision);
  const definitionMode = $derived(
    definition &&
      shared?.selection === snapshot?.selectionRevision &&
      shared?.path === snapshot?.path &&
      shared?.name === definition.name,
  );
  const relevantSections = $derived.by(() => {
    const names = new Set(target?.sections);
    for (const use of definition?.uses ?? []) {
      for (const name of use.sections) names.add(name);
    }
    if (anchorChoice?.selection === snapshot?.selectionRevision && anchorChoice) {
      names.add(anchorChoice.name);
    }
    return structure?.sections.filter((section) => names.has(section.name)) ?? [];
  });
  const anchor = $derived.by(() => {
    if (anchorChoice && anchorChoice.selection === snapshot?.selectionRevision) {
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
    mode !== 'expression' || !phrase
      ? undefined
      : definition?.uses.length || definition?.incomplete
        ? definitionMode
          ? definition.expression
          : undefined
        : (definition?.expression ?? target?.expression),
  );
  const focusedPage = $derived(
    page && page.selection === snapshot?.selectionRevision && page.mode === mode ? page.index : 0,
  );
  const origin = $derived(mode === 'song' ? (anchor?.start ?? 0) : 0);
  const from = $derived(origin + focusedPage * windowBars);
  const maxCycle = $derived(
    phrase?.bars ??
      (mode === 'song' ? structure?.totalBars || MAX_BARS : (anchor?.bars ?? MAX_BARS)),
  );
  const to = $derived(Math.min(from + windowBars, maxCycle));
  const bars = $derived.by<RollBar[]>(() => {
    if (!structure) return [];
    return Array.from({ length: Math.max(0, to - from) }, (_, i) => {
      const cycle = from + i;
      const written = mode === 'song' ? cycle : anchor ? anchor.start + cycle : null;
      const meter = written === null ? structure.meter : meterAtBar(structure, written);
      return {
        cycle,
        label: mode === 'song' ? `Bar ${cycle + 1}` : `Cycle ${cycle + 1}`,
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
  const needsInspection = $derived(target && (mode === 'song' || !phrase));
  const request = $derived.by<InspectionRequest | null>(() => {
    if (!snapshot || !target || !needsInspection || !rollInspection.available || to <= from)
      return null;
    if (!matchesLive && approvedRevision !== snapshot.revision) return null;
    return {
      root: snapshot.root,
      code: snapshot.code,
      view: mode,
      expression: target.expression,
      from,
      to,
      anchorBar: anchor?.start,
    };
  });
  const response = $derived($inspected?.request === request ? $inspected : null);
  const notes = $derived.by(() => {
    if (phrase) return phrase.notes;
    if (response?.phase !== 'ready' || !target) return emptyNotes;
    const { from, to } = target.expression;
    return mode === 'song'
      ? response.result.notes.map((note) => ({
          ...note,
          sourceMatch: [note.origin, ...(note.contributors ?? [])].some(
            (span) => span && span.from >= from && span.to <= to,
          ),
        }))
      : response.result.notes;
  });
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
    // Clear feedback when the source selection changes, but retain successful edit feedback.
    if (selectionRevision !== undefined) {
      notice = '';
      contributors = null;
    }
  });

  onDestroy(() => {
    rollInspection.request(null);
    app.stopPianoRollAudition();
  });

  function changeAnchor(section: Section | undefined) {
    if (snapshot && section) {
      anchorChoice = { selection: snapshot.selectionRevision, name: section.name };
      page = null;
    }
  }

  function goToPage(index: number) {
    if (snapshot) page = { selection: snapshot.selectionRevision, index, mode };
  }

  function openDefinition() {
    if (!snapshot || !definition) return;
    mode = 'expression';
    shared = { selection: snapshot.selectionRevision, path: snapshot.path, name: definition.name };
    page = null;
  }

  function reveal(context: PianoRollSnapshot, span: SourceSpan, section?: Section) {
    mode = 'expression';
    session.revealPianoRoll(context, span);
    const current = session.pianoRollSnapshot();
    if (current && section) {
      anchorChoice = { selection: current.selectionRevision, name: section.name };
    }
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
    if (!snapshot || !editTarget) return false;
    const note = 'id' in change ? notes.find((item) => item.id === change.id) : undefined;
    const patches = planRollEdit(snapshot.code, editTarget, change);
    if (!patches || !session.editPianoRoll(snapshot, patches, target?.expression)) {
      notice = 'This edit does not fit the source phrase.';
      return false;
    }
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
      {snapshot?.selection
        ? 'Select a supported pattern expression or a complete note token.'
        : 'Select a pattern in the editor.'}
    </p>
  {:else if needsInspection && !rollInspection.available}
    <p class="roll-status" role="status">
      Generated and in-song patterns can be inspected in the desktop app.
    </p>
  {:else if needsInspection && !request}
    <div class="preview">
      <Button size="sm" onclick={() => (approvedRevision = snapshot?.revision)}
        >Preview expression</Button
      >
      <span>Inspect the current draft without starting playback.</span>
    </div>
  {:else if response?.phase === 'error'}
    <p class="roll-status" role="status">{response.message}</p>
  {:else if needsInspection && (!response || response.phase === 'loading')}
    <p class="roll-status" role="status">Inspecting pattern…</p>
  {:else}
    {#if !phrase}
      <p class="roll-status compact" role="status">
        {mode === 'song'
          ? sourceMatches
            ? 'Arranged notes · source matches outlined'
            : 'Arranged notes · selection source unavailable'
          : 'Evaluated expression · view only'}{response?.phase === 'ready' &&
        response.result.truncated
          ? ' · results limited'
          : ''}{notes.length === 0 ? ' · no pitched notes in this window' : ''}
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
      editable={editTarget !== undefined}
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

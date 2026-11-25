import { describe, expect, test } from 'bun:test';
import type { Readable } from 'svelte/store';
import { buildStructure, type SongStructure } from '../src/lib/strudel/structure';
import {
  createTransportModel,
  INITIAL_SEGMENTS,
  prune,
  type Segment,
  type TransportState,
} from '../src/lib/strudel/transport';

// intro 2 (bars 0-1), body 4 looping (2-5), tail 2 (6-7); total 8.
const S = buildStructure({
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
});
// a 1, b 2 with no loop section.
const OTHER = buildStructure({
  sections: [
    { name: 'a', bars: 1 },
    { name: 'b', bars: 2 },
  ],
});

/** A model over a plain segment list; `started` notifies on every set, as the scheduler does. */
function harness() {
  let started = false;
  const startedRuns = new Set<(v: boolean) => void>();
  const startedStore: Readable<boolean> = {
    subscribe(run) {
      startedRuns.add(run);
      run(started);
      return () => startedRuns.delete(run);
    },
  };
  const h = {
    segments: INITIAL_SEGMENTS as readonly Segment[],
    structure: null as SongStructure | null,
    horizon: 0,
    now: 0,
    installs: [] as { next: readonly Segment[]; started: boolean }[],
    prunes: [] as number[],
    setStarted(v: boolean) {
      started = v;
      startedRuns.forEach((fn) => fn(v));
    },
  };
  const transport = createTransportModel({
    started: startedStore,
    structure: () => h.structure,
    boundary: {
      horizon: () => h.horizon,
      segments: () => h.segments,
      install(next, started) {
        h.segments = next;
        h.installs.push({ next, started });
      },
      prune(cycle) {
        h.prunes.push(cycle);
        h.segments = prune(h.segments, cycle);
      },
    },
    now: () => h.now,
  });
  const states: TransportState[] = [];
  transport.store.subscribe((s) => states.push(s));
  const state = () => states.at(-1)!;
  return { h, transport, states, state };
}

describe('createTransportModel', () => {
  test('starts on the song view with nothing armed', () => {
    const { h, state } = harness();
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
    expect(h.installs).toEqual([]);
  });

  test('a start-from while stopped replaces the list at cycle 0 and arms the section', () => {
    const { h, transport, state } = harness();
    transport.startFrom(2, S);
    expect(h.segments).toEqual([{ at: 0, songBar: 6, view: { kind: 'song' } }]);
    expect(h.installs.at(-1)!.started).toBe(false);
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: true, armedSection: 2 });
    // Another start-from replaces it rather than appending.
    transport.startFrom(1, S);
    expect(h.segments).toEqual([{ at: 0, songBar: 2, view: { kind: 'song' } }]);
    expect(state().armedSection).toBe(1);
  });

  test('a loop while stopped arms the loop view; clearing it resets', () => {
    const { h, transport, state } = harness();
    transport.loopSection(1, S);
    expect(h.segments).toEqual([{ at: 0, songBar: 2, view: { kind: 'loop', section: 1 } }]);
    expect(state()).toEqual({
      view: { kind: 'loop', section: 1 },
      pendingStart: true,
      armedSection: 1,
    });
    transport.loopRange(0, 1, S);
    expect(state()).toEqual({
      view: { kind: 'loop', section: 0, through: 1 },
      pendingStart: true,
      armedSection: 0,
    });
    transport.loopSection(null, S);
    expect(h.segments).toBe(INITIAL_SEGMENTS);
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });
  });

  test('a start keeps what was armed; it is no longer pending', () => {
    const { h, transport, state } = harness();
    transport.loopSection(1, S);
    const armed = h.segments;
    h.setStarted(true);
    expect(h.segments).toBe(armed);
    expect(state()).toEqual({
      view: { kind: 'loop', section: 1 },
      pendingStart: false,
      armedSection: null,
    });
  });

  test('while playing a change appends at the next bar past the horizon, against the evaluated structure', () => {
    const { h, transport, state } = harness();
    h.structure = S;
    h.setStarted(true);
    h.installs.length = 0;
    h.now = 1.5;
    h.horizon = 2.25;
    // The click's structure only matters while stopped.
    transport.startFrom(2, OTHER);
    expect(h.prunes).toEqual([1.5]);
    expect(h.segments).toEqual([
      { at: 0, songBar: 0, view: { kind: 'song' } },
      { at: 3, songBar: 6, view: { kind: 'song' } },
    ]);
    expect(h.installs).toEqual([{ next: h.segments, started: true }]);
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });

    h.horizon = 2.5;
    transport.loopSection(1, OTHER);
    expect(h.segments.at(-1)).toEqual({ at: 3, songBar: 2, view: { kind: 'loop', section: 1 } });
    expect(state().view).toEqual({ kind: 'loop', section: 1 });
  });

  test('while playing without an evaluated structure a change does nothing', () => {
    const { h, transport, states } = harness();
    h.setStarted(true);
    const before = states.length;
    transport.startFrom(1, S);
    transport.loopSection(1, S);
    expect(h.segments).toBe(INITIAL_SEGMENTS);
    expect(states.length).toBe(before);
  });

  test('every stop rewinds the transport, also a repeated one while stopped', () => {
    const { h, transport, state } = harness();
    h.structure = S;
    h.setStarted(true);
    h.horizon = 1;
    transport.loopSection(1, S);
    expect(state().view).toEqual({ kind: 'loop', section: 1 });
    h.setStarted(false);
    expect(h.segments).toBe(INITIAL_SEGMENTS);
    expect(h.installs.at(-1)).toEqual({ next: INITIAL_SEGMENTS, started: false });
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });

    transport.startFrom(1, S);
    expect(state().pendingStart).toBe(true);
    h.setStarted(false);
    expect(h.segments).toBe(INITIAL_SEGMENTS);
    expect(state().pendingStart).toBe(false);
  });

  test('reset drops what was armed or appended', () => {
    const { h, transport, state } = harness();
    transport.loopSection(1, S);
    transport.reset();
    expect(h.segments).toBe(INITIAL_SEGMENTS);
    expect(state()).toEqual({ view: { kind: 'song' }, pendingStart: false, armedSection: null });

    h.structure = S;
    h.setStarted(true);
    h.horizon = 1;
    transport.loopSection(1, S);
    h.installs.length = 0;
    transport.reset();
    expect(h.installs).toEqual([{ next: INITIAL_SEGMENTS, started: true }]);
    expect(state().view).toEqual({ kind: 'song' });
  });

  test('play ending releases at the next loop pass and shows the release view', () => {
    const { h, transport, state, states } = harness();
    h.structure = S;
    // Stopped: nothing to end.
    transport.playEnding();
    expect(h.segments).toBe(INITIAL_SEGMENTS);

    h.setStarted(true);
    // Outside the loop section (the intro): not yet.
    h.now = 1;
    h.horizon = 1.5;
    transport.playEnding();
    expect(h.segments).toBe(INITIAL_SEGMENTS);

    // Inside it, at song bar 3; the horizon's bar (4) is mid-pass, so the release waits for bar 6.
    h.now = 3;
    h.horizon = 3.2;
    transport.playEnding();
    expect(h.segments).toEqual([
      { at: 0, songBar: 0, view: { kind: 'song' } },
      { at: 6, songBar: 6, view: { kind: 'release' } },
    ]);
    expect(state()).toEqual({ view: { kind: 'release' }, pendingStart: false, armedSection: null });

    // Once released it cannot be armed again.
    const before = states.length;
    const released = h.segments;
    transport.playEnding();
    expect(h.segments).toBe(released);
    expect(states.length).toBe(before);
  });
});

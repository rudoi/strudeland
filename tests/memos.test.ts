import { describe, expect, test } from 'bun:test';
import { get, writable } from 'svelte/store';
import {
  createMemoPlayer,
  type MemoPlayer,
  type MemoPlayerState,
  type PlayableMemo,
  type PlayHow,
} from '../src/lib/audio/memo-player';
import type { ProjectChange } from '../src/lib/project/api';
import { createMemos, type PlayerIo } from '../src/lib/recording/memos';
import { RecordingError, type RecordingUiState, type Sidecar } from '../src/lib/recording/types';
import { encodeWav } from '../src/lib/render/wav';
import { buildStructure, type SongStructure } from '../src/lib/strudel/structure';
import type { RuntimeState } from '../src/lib/strudel/runtime';
import { fakeAudioGraph } from './helpers/fake-audio-graph';
import { flush } from './helpers/fake-project-api';
import { entry, fakeRecordingApi } from './helpers/fake-recording-api';

/** A player stub that records its calls and lets a test set its state. */
function stubPlayer() {
  const store = writable<MemoPlayerState>({ status: 'idle' });
  const calls: string[] = [];
  let io: PlayerIo | null = null;
  const player: MemoPlayer = {
    store: { subscribe: store.subscribe },
    async play(item: PlayableMemo, how: PlayHow) {
      calls.push(`play:${item.stem}:${how}`);
      store.set({ status: 'playing', stem: item.stem, how });
    },
    stop() {
      calls.push('stop');
      store.set({ status: 'idle' });
    },
    dispose() {
      calls.push('dispose');
    },
  };
  return {
    player,
    calls,
    store,
    create: (i: PlayerIo) => ((io = i), player),
    io: () => io!,
  };
}

function setup(
  opts: { roots?: string[]; root?: string | null; recording?: boolean; listen?: boolean } = {},
) {
  const { api, fake } = fakeRecordingApi({ roots: opts.roots ?? ['/p', '/q'] });
  const session = writable<{
    project: { root: string } | null;
    active: string | null;
    structure: SongStructure | null;
  }>({
    project: opts.root === null ? null : { root: opts.root ?? '/p' },
    active: 'songs/a.js',
    structure: buildStructure({}),
  });
  const handlers = new Set<(c: ProjectChange) => void>();
  let unlistened = 0;
  const rec = writable<RecordingUiState>({
    phase: 'armed',
    countInBeat: null,
    takeStartedAtMs: null,
    devices: [],
    input: null,
    error: null,
    permission: 'authorized',
    warning: null,
    stem: null,
    lastStem: null,
  });
  const p = stubPlayer();
  const memos = createMemos({
    api,
    session: { store: { subscribe: session.subscribe } },
    onChange:
      opts.listen === false
        ? undefined
        : async (h) => {
            handlers.add(h);
            return () => {
              unlistened++;
              handlers.delete(h);
            };
          },
    recording: opts.recording === false ? undefined : { store: { subscribe: rec.subscribe } },
    createPlayer: p.create,
  });
  const emit = (c: Partial<ProjectChange>) => {
    const change = {
      root: '/p',
      songs: { changed: [], added: [], removed: [] },
      manifest: false,
      recordings: false,
      instruments: false,
      ...c,
    };
    handlers.forEach((h) => h(change));
  };
  const setPhase = (phase: RecordingUiState['phase']) => rec.update((s) => ({ ...s, phase }));
  return {
    api,
    fake,
    memos,
    session,
    p,
    emit,
    setPhase,
    handlers,
    unlistened: () => unlistened,
    state: () => get(memos.store),
    stems: () => get(memos.store).items.map((m) => m.stem),
    lists: () => fake.argsOf('list'),
  };
}

const at = (iso: string) => ({ modifiedMs: Date.parse(iso) });

describe('the memo list', () => {
  test('loads the open project newest first', async () => {
    const s = setup();
    s.fake.memos.set('/p', [
      entry('old', at('2025-01-01T00:00:00Z')),
      entry('new', at('2026-01-01T00:00:00Z')),
    ]);
    await s.memos.refresh();
    expect(s.stems()).toEqual(['new', 'old']);
    expect(s.state()).toMatchObject({ loaded: true, error: null });
    expect(s.lists()[0]).toEqual(['/p']);
  });

  test('no project: nothing is listed', async () => {
    const s = setup({ root: null });
    await s.memos.refresh();
    expect(s.lists()).toEqual([]);
    expect(s.state()).toMatchObject({ items: [], loaded: false });
  });

  test('the initial load happens on its own', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    await flush();
    expect(s.stems()).toEqual(['a']);
  });

  test('a list failure keeps the items and reports the error', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    await s.memos.refresh();
    s.fake.failNext('list', new RecordingError('disk on fire', 'other'));
    await s.memos.refresh();
    expect(s.state()).toMatchObject({ error: 'disk on fire', loaded: true });
    expect(s.stems()).toEqual(['a']);
    await s.memos.refresh();
    expect(s.state().error).toBeNull();
  });

  test('refreshes on a recordings change for the open root only', async () => {
    const s = setup();
    await flush();
    const n = s.lists().length;
    s.emit({ recordings: true });
    await flush();
    expect(s.lists()).toHaveLength(n + 1);
    s.emit({ recordings: true, root: '/q' });
    s.emit({ recordings: false });
    s.emit({ manifest: true });
    await flush();
    expect(s.lists()).toHaveLength(n + 1);
  });

  test('refreshes when a take finishes saving', async () => {
    const s = setup();
    await flush();
    const n = s.lists().length;
    s.setPhase('recording');
    s.setPhase('stopping');
    await flush();
    expect(s.lists()).toHaveLength(n);
    s.setPhase('armed');
    await flush();
    expect(s.lists()).toHaveLength(n + 1);
  });

  test('refreshes during a listing share one trailing listing', async () => {
    const s = setup();
    await flush();
    s.fake.memos.set('/p', [entry('a')]);
    const n = s.lists().length;
    const release = Promise.withResolvers<void>();
    s.fake.hold('list', release.promise);
    const first = s.memos.refresh();
    s.fake.hold('list', null);
    s.fake.memos.set('/p', [entry('a'), entry('b')]);
    const second = s.memos.refresh();
    const third = s.memos.refresh();
    expect(third).toBe(second);
    expect(s.lists()).toHaveLength(n + 1);
    release.resolve();
    await Promise.all([first, second]);
    expect(s.lists()).toHaveLength(n + 2);
    expect(s.stems().sort()).toEqual(['a', 'b']);
  });

  test('a project switch stops playback, clears the items and loads the new list', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    s.fake.memos.set('/q', [entry('q1')]);
    await s.memos.refresh();
    await s.memos.play('a', 'alone');
    const release = Promise.withResolvers<void>();
    s.fake.hold('list', release.promise);
    s.session.update((x) => ({ ...x, project: { root: '/q' } }));
    expect(s.p.calls.at(-1)).toBe('stop');
    expect(s.state()).toMatchObject({ items: [], loaded: false });
    expect(get(s.memos.player)).toEqual({ status: 'idle' });
    release.resolve();
    s.fake.hold('list', null);
    await flush();
    expect(s.stems()).toEqual(['q1']);
  });

  test('a result that arrives after a project switch is dropped', async () => {
    const s = setup();
    await flush();
    s.fake.memos.set('/p', [entry('stale')]);
    s.fake.memos.set('/q', [entry('fresh')]);
    const release = Promise.withResolvers<void>();
    s.fake.hold('list', release.promise);
    const p = s.memos.refresh();
    s.session.update((x) => ({ ...x, project: { root: '/q' } }));
    release.resolve();
    await p;
    await flush();
    expect(s.stems()).toEqual(['fresh']);
  });

  test('a stale failure after a switch is dropped too', async () => {
    const s = setup();
    await flush();
    const release = Promise.withResolvers<void>();
    s.fake.hold('list', release.promise);
    s.fake.failNext('list', new RecordingError('the project changed', 'projectChanged'));
    const p = s.memos.refresh();
    // The new project's list waits too, and answers after the stale failure.
    s.session.update((x) => ({ ...x, project: { root: '/q' } }));
    s.fake.hold('list', null);
    release.resolve();
    await p;
    await flush();
    expect(s.state()).toMatchObject({ error: null, loaded: true });
  });

  test('closing the project empties the list', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    await s.memos.refresh();
    s.session.update((x) => ({ ...x, project: null }));
    expect(s.state()).toMatchObject({ items: [], loaded: false });
  });

  test('destroy unlistens and disposes the player', async () => {
    const s = setup();
    await flush();
    s.memos.destroy();
    expect(s.unlistened()).toBe(1);
    expect(s.p.calls.at(-1)).toBe('dispose');
    const n = s.lists().length;
    s.setPhase('stopping');
    s.setPhase('armed');
    await s.memos.refresh();
    expect(s.lists()).toHaveLength(n);
  });
});

describe('playback through the list', () => {
  test('plays a listed memo; the state mirrors the player', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    await s.memos.refresh();
    await s.memos.play('a', 'metronome');
    expect(s.p.calls).toEqual(['play:a:metronome']);
    expect(get(s.memos.player)).toEqual({ status: 'playing', stem: 'a', how: 'metronome' });
    s.memos.stopPlayback();
    expect(get(s.memos.player)).toEqual({ status: 'idle' });
  });

  test('an unknown stem plays nothing', async () => {
    const s = setup();
    await s.memos.play('ghost', 'alone');
    expect(s.p.calls).toEqual([]);
  });

  test('nothing plays while a take runs', async () => {
    const s = setup();
    s.fake.memos.set('/p', [entry('a')]);
    await s.memos.refresh();
    for (const phase of ['starting', 'count-in', 'recording', 'stopping'] as const) {
      s.setPhase(phase);
      await s.memos.play('a', 'alone');
    }
    expect(s.p.calls).toEqual([]);
    s.setPhase('armed');
    await s.memos.play('a', 'alone');
    expect(s.p.calls).toEqual(['play:a:alone']);
  });

  test('the player reads bytes from the open project', async () => {
    const s = setup();
    s.fake.memos.set('/q', [entry('b')]);
    s.fake.bytes.set('/q:b', new Uint8Array([7]));
    s.session.update((x) => ({ ...x, project: { root: '/q' } }));
    expect(await s.p.io().readBytes('b')).toEqual(new Uint8Array([7]));
    expect(s.fake.argsOf('read')).toEqual([['/q', 'b']]);
    s.session.update((x) => ({ ...x, project: null }));
    await expect(s.p.io().readBytes('b')).rejects.toThrow('no project is open');
  });

  test("the song tempo is the open song's static header", () => {
    const s = setup();
    s.session.update((x) => ({ ...x, structure: buildStructure({ bpm: 90, meter: '3/4' }) }));
    expect(s.p.io().songTempo()).toEqual({ cps: 0.5, meter: { num: 3, den: 4 } });
    s.session.update((x) => ({ ...x, active: null }));
    expect(s.p.io().songTempo()).toBeNull();
  });
});

describe('rename and delete', () => {
  async function ready() {
    const s = setup();
    s.fake.memos.set('/p', [
      entry('a', at('2026-01-02T00:00:00Z')),
      entry('b', at('2026-01-01T00:00:00Z')),
    ]);
    await s.memos.refresh();
    return s;
  }

  test('rename resolves to the new stem and refreshes', async () => {
    const s = await ready();
    expect(await s.memos.rename('a', 'chorus idea')).toBe('chorus idea');
    expect(s.fake.argsOf('rename')).toEqual([['/p', 'a', 'chorus idea']]);
    expect(s.stems()).toEqual(['chorus idea', 'b']);
  });

  test('rename stops the playback of that memo only', async () => {
    const s = await ready();
    await s.memos.play('b', 'alone');
    await s.memos.rename('a', 'x');
    expect(s.p.calls).toEqual(['play:b:alone']);
    await s.memos.rename('b', 'y');
    expect(s.p.calls).toEqual(['play:b:alone', 'stop']);
  });

  test.each([
    ['invalidName', 'a/b'],
    ['exists', 'b'],
  ] as const)('rename propagates %p and leaves the list', async (kind, next) => {
    const s = await ready();
    const lists = s.lists().length;
    const e = await s.memos.rename('a', next).catch((x) => x);
    expect(e).toBeInstanceOf(RecordingError);
    expect(e.kind).toBe(kind);
    expect(s.stems()).toEqual(['a', 'b']);
    expect(s.lists()).toHaveLength(lists);
  });

  test('rename of a vanished memo is notFound', async () => {
    const s = await ready();
    const e = await s.memos.rename('ghost', 'x').catch((x) => x);
    expect(e.kind).toBe('notFound');
    expect(e.message).toBe('recording ghost not found');
  });

  test('delete removes, refreshes and stops that memo', async () => {
    const s = await ready();
    await s.memos.play('a', 'alone');
    await s.memos.remove('a');
    expect(s.p.calls).toEqual(['play:a:alone', 'stop']);
    expect(s.fake.argsOf('delete')).toEqual([['/p', 'a']]);
    expect(s.stems()).toEqual(['b']);
  });

  test('a failed delete refreshes the list and throws', async () => {
    const s = await ready();
    // Someone removed it on disk meanwhile.
    s.fake.memos.set(
      '/p',
      s.fake.memos.get('/p')!.filter((m) => m.stem !== 'a'),
    );
    const e = await s.memos.remove('a').catch((x) => x);
    expect(e).toBeInstanceOf(RecordingError);
    expect(e.kind).toBe('notFound');
    expect(s.stems()).toEqual(['b']);
  });

  test('a projectChanged error propagates', async () => {
    const s = await ready();
    s.fake.failNext('delete', new RecordingError('the project changed', 'projectChanged'));
    const e = await s.memos.remove('a').catch((x) => x);
    expect(e.kind).toBe('projectChanged');
  });

  test('without a project they throw', async () => {
    const s = setup({ root: null });
    await expect(s.memos.rename('a', 'b')).rejects.toThrow('no project is open');
    await expect(s.memos.remove('a')).rejects.toThrow('no project is open');
  });
});

describe('with the real player', () => {
  function sidecar(bar1Frame: number | null, cps = 0.5): Sidecar {
    return {
      schema: 1,
      createdAt: '2026-01-01T00:00:00Z',
      app: { version: '0.1.0' },
      input: { deviceId: 'm', deviceName: 'M', channel: 0, channels: 1, sampleRate: 48000 },
      output: { baseLatency: 0, outputLatency: null },
      tempo: { bpm: cps * 240, meter: '4/4', cps, beatsPerBar: 4, tempoOverride: false },
      backing: {
        song: true,
        metronome: false,
        countIn: false,
        songPath: 'songs/a.js',
        startSection: null,
        loopSection: null,
      },
      timing: {
        bar1Frame,
        bar1HostNs: null,
        clockOffsetNs: { start: null, end: null },
        anchors: [],
        outputStamps: [],
      },
      xruns: 0,
      recovered: false,
    };
  }

  test('reads the bytes of the open project and plays them; a tempo mismatch refuses', async () => {
    const { api, fake } = fakeRecordingApi();
    const g = fakeAudioGraph({ currentTime: 1 });
    const session = writable({
      project: { root: '/p' },
      active: 'songs/a.js' as string | null,
      structure: buildStructure({ bpm: 100 }) as SongStructure | null,
    });
    const rt = writable({ started: false, cps: 0.5, meter: { num: 4, den: 4 } } as RuntimeState);
    const memos = createMemos({
      api,
      session: { store: { subscribe: session.subscribe } },
      createPlayer: (io) =>
        createMemoPlayer({
          graph: () => g.graph,
          isRunning: () => true,
          resumeAudio: async () => true,
          backing: {
            startSong: async () => ({ T: 2, cps: 0.5 }),
            startMetronome: () => true,
            stopAll: () => {},
          },
          runtime: { store: { subscribe: rt.subscribe }, stop: () => {} },
          ...io,
        }),
    });
    fake.memos.set('/p', [entry('a', { sidecar: sidecar(0) })]);
    fake.bytes.set('/p:a', encodeWav([new Float32Array(480)], 48000, 'f32'));
    await memos.refresh();
    await memos.play('a', 'song');
    expect(get(memos.player)).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'tempo',
      message: 'memo was recorded at 120 · 4/4; the song is now 100 · 4/4',
    });
    expect(fake.argsOf('read')).toEqual([]);
    session.update((x) => ({ ...x, structure: buildStructure({ bpm: 120 }) }));
    await memos.play('a', 'song');
    expect(get(memos.player)).toEqual({ status: 'playing', stem: 'a', how: 'song' });
    expect(fake.argsOf('read')).toEqual([['/p', 'a']]);
    expect(g.sources()[0]!.starts).toEqual([[2, 0]]);
    memos.destroy();
  });
});

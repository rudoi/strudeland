import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { get } = await import('svelte/store');
const { buildStructure } = await import('../src/lib/strudel/structure');
const { createSongPlayback } = await import('../src/lib/project/song-playback');
const { createSession } = await import('../src/lib/project/session');
const { createEditorState } = await import('../src/lib/editor/setup');
const { EditorView } = await import('@codemirror/view');
import type { PlaybackState, PlaybackTransport } from '../src/lib/project/song-playback';
import type { Session } from '../src/lib/project/session';
import type { InstrumentSnapshot } from '../src/lib/strudel/instruments';
import type { SongStructure } from '../src/lib/strudel/structure';
import { deferred, fakeProjectApi, flush } from './helpers/fake-project-api';
import { stubRuntime } from './helpers/stub-runtime';

const PA = 'songs/a.js';
const PB = 'songs/b.js';

const layout = (...names: string[]): SongStructure =>
  buildStructure({ sections: names.map((name) => ({ name, bars: 2 })) });

const opts = (metaTitle?: unknown, instruments?: InstrumentSnapshot) => ({
  options: instruments === undefined ? {} : { instruments },
  metaTitle,
});

/** The module alone over the stub runtime, with a header structure the test sets (null: none). */
function setup(
  structure: SongStructure | null = layout('intro', 'verse', 'chorus'),
  runtimeOpts: Parameters<typeof stubRuntime>[0] = {},
) {
  const rt = stubRuntime(runtimeOpts);
  let header = structure;
  let gen = 1;
  const pb = createSongPlayback({
    runtime: rt.runtime,
    getHeader: () => header,
    currentGen: () => gen,
  });
  return {
    rt,
    pb,
    st: () => get(pb.store),
    tp: () => get(pb.store).transport,
    canArm: (choice: Parameters<typeof pb.arm>[0]) => get(pb.canArm)(choice),
    setHeader: (s: SongStructure | null) => void (header = s),
    setGen: (g: number) => void (gen = g),
    /** Evaluated and playing `structure`. */
    async play(path = PA, s: SongStructure | null = header) {
      expect(await pb.evaluate(path, path, opts(), gen)).toBe(true);
      rt.patch({ started: true, structure: s });
    },
  };
}

const NOT_ARMED: PlaybackTransport = { view: { kind: 'song' }, loop: null, armed: null };

describe('evaluate', () => {
  test('records the evaluated song with its title; playing only while started', async () => {
    const h = setup();
    expect(h.st().evaluated).toBeNull();
    expect(await h.pb.evaluate(PA, 'code', opts('Alpha', {}), 1)).toBe(true);
    expect(h.rt.evaluated).toEqual(['code']);
    expect(h.st()).toMatchObject({
      evaluated: { path: PA, title: 'Alpha' },
      playing: null,
      started: false,
    });
    expect(h.pb.instruments()).toEqual({});
    h.rt.setStarted(true);
    expect(h.st()).toMatchObject({ playing: { path: PA, title: 'Alpha' }, started: true });
    h.rt.setStarted(false);
    expect(h.st()).toMatchObject({ evaluated: { path: PA, title: 'Alpha' }, playing: null });
  });

  test('the file name without a title; a failed evaluate keeps the previous song', async () => {
    const h = setup();
    await h.pb.evaluate(PA, 'a', opts('Alpha'), 1);
    h.rt.setResult(false);
    expect(await h.pb.evaluate(PB, 'b', opts(), 1)).toBe(false);
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
    h.rt.setResult(true);
    await h.pb.evaluate(PB, 'b', opts('  '), 1);
    expect(h.st().evaluated).toEqual({ path: PB, title: 'b' });
  });

  test('an evaluate that resolves in another generation is not recorded', async () => {
    const h = setup();
    const gate = deferred();
    h.rt.hold(gate.promise);
    const pending = h.pb.evaluate(PA, 'a', opts('Alpha'), 1);
    h.setGen(2);
    gate.resolve();
    expect(await pending).toBe(true);
    expect(h.st().evaluated).toBeNull();
  });

  test('a song taking over playback finds the transport reset in the same tick', async () => {
    const h = setup();
    await h.play(PA);
    h.pb.arm({ loop: 'verse' });
    expect(h.tp().view).toEqual({ kind: 'loop', section: 1 });
    let seen: unknown = null;
    h.rt.runtime.evaluate = async () => {
      seen = get(h.rt.runtime.transport.store).view;
      return true;
    };
    const pending = h.pb.evaluate(PB, 'b', opts(), 1);
    expect(seen).toEqual({ kind: 'song' }); // called before any await
    await pending;
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('re-evaluating the playing song keeps its transport', async () => {
    const h = setup();
    await h.play(PA);
    h.pb.arm({ loop: 'verse' });
    await h.pb.evaluate(PA, 'a2', opts(), 1);
    expect(h.tp().loop).toEqual({ section: 'verse', names: ['verse'] });
  });

  test('evaluating another song while stopped keeps the armed start', async () => {
    const h = setup();
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.pb.arm({ start: 'chorus' });
    await h.pb.evaluate(PB, 'b', opts(), 1);
    expect(h.tp().armed).toBe('chorus');
  });
});

describe('disarming', () => {
  test('refused and activeChanged disarm while stopped', () => {
    for (const verb of ['refused', 'activeChanged'] as const) {
      const h = setup();
      h.pb.arm({ start: 'verse' });
      expect(h.tp()).toEqual({ view: { kind: 'song' }, loop: null, armed: 'verse' });
      h.pb[verb]();
      expect(h.tp()).toEqual(NOT_ARMED);
      expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
    }
  });

  test('refused and activeChanged leave a playing song its loop', async () => {
    for (const verb of ['refused', 'activeChanged'] as const) {
      const h = setup();
      await h.play();
      h.pb.arm({ loop: 'chorus' });
      h.pb[verb]();
      expect(h.tp().loop).toEqual({ section: 'chorus', names: ['chorus'] });
    }
  });

  test('every runtime stop and reset drops the armed names', async () => {
    const h = setup();
    h.pb.arm({ loop: 'verse', through: 'chorus' });
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 1, through: 2 },
      loop: { section: 'verse', through: 'chorus', names: ['verse', 'chorus'] },
      armed: 'verse',
    });
    // Starting consumes the armed start: the loop is now the view's, named from the evaluated song.
    h.rt.patch({ started: true, structure: layout('intro', 'verse', 'chorus') });
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 1, through: 2 },
      loop: { section: 'verse', through: 'chorus', names: ['verse', 'chorus'] },
      armed: null,
    });
    // The scheduler stopping by itself rewinds the transport.
    h.rt.setStarted(false);
    expect(h.tp()).toEqual(NOT_ARMED);
    // A reset from outside the module.
    h.pb.arm({ start: 'chorus' });
    h.rt.runtime.transport.reset();
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('no published state is armed while started, through a start and a stop', () => {
    const h = setup(layout('intro', 'verse', 'chorus'), { live: true, audio: 'running' });
    h.rt.patch({ structure: layout('intro', 'verse', 'chorus') });
    const seen: PlaybackState[] = [];
    const off = h.pb.store.subscribe((s) => seen.push(s));
    for (const choice of [{ start: 'chorus' }, { loop: 'verse' }]) {
      h.pb.arm(choice);
      // The runtime publishes `started` before the transport model re-applies.
      h.rt.setStarted(true);
      h.rt.runtime.stop();
    }
    off();
    expect(seen.some((s) => s.transport.armed !== null)).toBe(true);
    for (const s of seen) {
      if (s.started) expect(s.transport.armed).toBeNull();
      else if (s.transport.armed === null) expect(s.transport.loop).toBeNull();
    }
    expect(seen.at(-1)).toMatchObject({ started: false, transport: NOT_ARMED });
  });

  test('a runtime stop while already stopped rewinds an armed start, as the runtime does', () => {
    const h = setup();
    h.pb.arm({ start: 'chorus' });
    expect(h.tp().armed).toBe('chorus');
    h.rt.runtime.stop();
    expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
    expect(h.tp()).toEqual(NOT_ARMED);
    // A repeated toggle to stopped rewinds too.
    h.pb.arm({ start: 'chorus' });
    h.rt.setStarted(false);
    expect(h.tp()).toEqual(NOT_ARMED);
  });
});

describe('song lifecycle', () => {
  test('projectChanged stops, resets and forgets the evaluated song', async () => {
    const h = setup();
    await h.play();
    h.pb.arm({ loop: 'verse' });
    const stops = h.rt.stops();
    h.pb.projectChanged();
    expect(h.rt.stops()).toBe(stops + 1);
    expect(h.st().evaluated).toBeNull();
    expect(get(h.rt.runtime.transport.store).view).toEqual({ kind: 'song' });
  });

  test('renamed retitles from the kept meta.title, else the new file name', async () => {
    const h = setup();
    await h.pb.evaluate(PA, 'a', opts('Alpha'), 1);
    h.pb.renamed(PB, 'songs/x.js'); // not the evaluated song
    expect(h.st().evaluated).toEqual({ path: PA, title: 'Alpha' });
    h.pb.renamed(PA, 'songs/x.js');
    expect(h.st().evaluated).toEqual({ path: 'songs/x.js', title: 'Alpha' });
    await h.pb.evaluate(PB, 'b', opts(), 1);
    h.pb.renamed(PB, 'songs/n.js');
    expect(h.st().evaluated).toEqual({ path: 'songs/n.js', title: 'n' });
  });

  test('removed stops and forgets only the evaluated song; stopped says whether it played', async () => {
    const h = setup();
    await h.play(PA);
    expect(h.pb.removed(PB)).toEqual({ stopped: false });
    expect(h.rt.stops()).toBe(0);
    expect(h.pb.removed(PA)).toEqual({ stopped: true });
    expect(h.rt.stops()).toBe(1);
    expect(h.st().evaluated).toBeNull();

    await h.pb.evaluate(PB, 'b', opts(), 1);
    h.rt.setStarted(false);
    expect(h.pb.removed(PB)).toEqual({ stopped: false });
    expect(h.st().evaluated).toBeNull();
  });

  test('removing the stopped evaluated song leaves the runtime and the armed start alone', async () => {
    const h = setup();
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.pb.arm({ start: 'verse' });
    const stops = h.rt.stops();
    expect(h.pb.removed(PA)).toEqual({ stopped: false });
    expect(h.rt.stops()).toBe(stops);
    expect(h.st().evaluated).toBeNull();
    expect(h.tp().armed).toBe('verse');
  });

  test('removing the evaluated song stops its evaluate in flight', async () => {
    const h = setup();
    await h.pb.evaluate(PA, 'a', opts(), 1);
    const held = deferred();
    h.rt.hold(held.promise);
    const run = h.pb.evaluate(PA, 'a2', opts(), 1);
    const stops = h.rt.stops();
    expect(h.pb.removed(PA)).toEqual({ stopped: false });
    expect(h.rt.stops()).toBe(stops + 1);
    held.resolve();
    await run;
  });
});

describe('arm', () => {
  test('while stopped, names resolve against the active header', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    // Evaluated with other sections: they do not matter while stopped.
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.rt.patch({ structure: layout('verse') });
    h.pb.arm({ start: 'chorus' });
    expect(get(h.rt.runtime.transport.store)).toMatchObject({
      pendingStart: true,
      armedSection: 2,
    });
    expect(h.tp()).toEqual({ view: { kind: 'song' }, loop: null, armed: 'chorus' });
    expect(h.canArm({ start: 'nope' })).toBe(false);
    h.pb.arm({ start: 'nope' });
    expect(h.tp().armed).toBe('chorus');
  });

  test('starting from the first section arms nothing: there is nothing pending', () => {
    const h = setup();
    h.pb.arm({ start: 'intro' });
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('while playing, names resolve against the evaluated structure', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    await h.play(PA, layout('verse', 'chorus'));
    expect(h.canArm({ loop: 'intro' })).toBe(false); // in the header, not in the song playing
    h.pb.arm({ loop: 'chorus' });
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 1 },
      loop: { section: 'chorus', names: ['chorus'] },
      armed: null,
    });
  });

  test('a playing loop renames itself when a hot reload changes the sections', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    await h.play();
    h.pb.arm({ loop: 'verse' });
    h.rt.patch({ structure: layout('intro', 'bridge', 'chorus') });
    expect(h.tp().loop).toEqual({ section: 'bridge', names: ['bridge'] });
  });

  test('a range must appear contiguously and in order in the structure in force', async () => {
    const playing = layout('a', 'b', 'c');
    const h = setup(layout('b', 'c', 'a'));
    await h.play(PA, playing);
    expect(h.canArm({ loop: 'b', through: 'c' })).toBe(true);
    h.pb.arm({ loop: 'b', through: 'c' });
    expect(h.tp().view).toEqual({ kind: 'loop', section: 1, through: 2 });
    expect(h.tp().loop).toEqual({ section: 'b', through: 'c', names: ['b', 'c'] });
    h.setHeader(layout('b', 'a', 'c'));
    expect(h.canArm({ loop: 'b', through: 'c' })).toBe(false);
    h.setHeader(layout('b', 'new', 'c'));
    expect(h.canArm({ loop: 'b', through: 'c' })).toBe(false);
    expect(h.canArm({ loop: 'c', through: 'b' })).toBe(false); // through before loop
  });

  test('nothing is armable under a meta error; while playing stopLooping still leaves', async () => {
    const h = setup();
    h.setHeader(null);
    for (const choice of [{ start: 'verse' }, { loop: 'verse' }] as const) {
      expect(h.canArm(choice)).toBe(false);
      h.pb.arm(choice);
      expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
    }
    h.pb.stopLooping();
    expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
    h.setHeader(layout('intro', 'verse', 'chorus'));
    await h.play();
    h.pb.arm({ loop: 'verse' });
    h.setHeader(null);
    expect(h.canArm({ loop: 'verse' })).toBe(false);
    h.rt.setPosition({ cycle: 3, located: null });
    h.rt.setHorizon(3);
    h.pb.stopLooping();
    expect(h.tp().view).toEqual({ kind: 'song' });
  });

  test('canArm recomputes when the evaluated structure changes while playing', async () => {
    const h = setup(layout('intro', 'verse', 'bridge'));
    await h.play(PA, layout('intro', 'verse'));
    const seen: boolean[] = [];
    const off = h.pb.canArm.subscribe((can) => seen.push(can({ start: 'bridge' })));
    h.rt.patch({ structure: layout('intro', 'verse', 'bridge') });
    off();
    expect(seen).toEqual([false, true]);
  });

  test('nothing is armable while stopped without an active song', () => {
    const h = setup(null);
    expect(h.canArm({ start: 'verse' })).toBe(false);
    h.pb.stopLooping();
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('stopLooping while stopped drops anything armed, a start included', () => {
    const h = setup();
    h.pb.arm({ start: 'chorus' });
    h.pb.stopLooping();
    expect(h.tp()).toEqual(NOT_ARMED);
    h.pb.arm({ loop: 'verse' });
    h.pb.stopLooping();
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('stopLooping while playing leaves the loop at the next bar', async () => {
    const h = setup();
    await h.play();
    h.pb.arm({ loop: 'verse' });
    h.rt.setPosition({ cycle: 3, located: null });
    h.rt.setHorizon(3);
    h.pb.stopLooping();
    expect(h.tp()).toEqual({ view: { kind: 'song' }, loop: null, armed: null });
    expect(h.rt.segments().at(-1)).toEqual({ at: 3, songBar: 3, view: { kind: 'song' } });
  });

  test('playEnding releases the authored loop at the next loop pass', async () => {
    const s = buildStructure({
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4, loop: true },
        { name: 'tail', bars: 2 },
      ],
    });
    const h = setup(s);
    await h.play(PA, s);
    h.rt.setPosition({ cycle: 2, located: null });
    h.rt.setHorizon(2);
    h.pb.playEnding();
    expect(h.tp().view).toEqual({ kind: 'release' });
  });
});

describe('the armed section is the one the header shows', () => {
  test('with nothing evaluated yet', () => {
    const h = setup(layout('intro', 'verse'));
    h.pb.arm({ start: 'verse' });
    expect(h.tp().armed).toBe('verse');
  });

  test('with the header edited after the last evaluate', async () => {
    const h = setup(layout('intro', 'verse'));
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.rt.patch({ structure: layout('intro', 'verse') });
    h.setHeader(layout('intro', 'bridge', 'verse'));
    h.pb.arm({ start: 'bridge' });
    expect(get(h.rt.runtime.transport.store).armedSection).toBe(1);
    expect(h.tp().armed).toBe('bridge');
  });

  test('after a failed evaluate', async () => {
    const h = setup(layout('intro', 'verse'));
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.rt.patch({ structure: layout('x', 'y', 'z') });
    h.rt.setResult(false);
    h.pb.arm({ start: 'verse' });
    expect(await h.pb.evaluate(PA, 'broken', opts(), 1)).toBe(false);
    expect(h.tp().armed).toBe('verse');
  });
});

describe('a stopped arm follows its names when the header changes', () => {
  const rts = (h: ReturnType<typeof setup>) => get(h.rt.runtime.transport.store);

  test('a reorder re-arms the same name at its new index', () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    h.pb.arm({ start: 'chorus' });
    h.setHeader(layout('intro', 'chorus', 'verse'));
    h.pb.headerChanged();
    expect(rts(h)).toMatchObject({ pendingStart: true, armedSection: 1 });
    expect(h.rt.segments().at(-1)).toEqual({ at: 0, songBar: 2, view: { kind: 'song' } });
    expect(h.tp().armed).toBe('chorus');
  });

  test('the same index at another bar re-arms it there', () => {
    const h = setup(layout('intro', 'verse'));
    h.pb.arm({ start: 'verse' });
    h.setHeader(
      buildStructure({
        sections: [
          { name: 'intro', bars: 4 },
          { name: 'verse', bars: 2 },
        ],
      }),
    );
    h.pb.headerChanged();
    expect(h.rt.segments().at(-1)).toEqual({ at: 0, songBar: 4, view: { kind: 'song' } });
    expect(h.tp().armed).toBe('verse');
  });

  test('a rename removing the armed section disarms', () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    h.pb.arm({ loop: 'chorus' });
    h.setHeader(layout('intro', 'verse', 'hook'));
    h.pb.headerChanged();
    expect(rts(h).pendingStart).toBe(false);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('an edit that leaves the armed section in place leaves the arm untouched', () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    h.pb.arm({ loop: 'verse' });
    const before = h.rt.segments();
    h.setHeader(layout('intro', 'verse', 'bridge'));
    h.pb.headerChanged();
    expect(h.rt.segments()).toBe(before);
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 1 },
      loop: { section: 'verse', names: ['verse'] },
      armed: 'verse',
    });
  });

  test('a range re-arms where its run moved, and disarms once the run breaks', () => {
    const h = setup(layout('intro', 'verse', 'chorus', 'outro'));
    h.pb.arm({ loop: 'verse', through: 'chorus' });
    h.setHeader(layout('verse', 'chorus', 'intro', 'outro'));
    h.pb.headerChanged();
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 0, through: 1 },
      loop: { section: 'verse', through: 'chorus', names: ['verse', 'chorus'] },
      armed: 'verse',
    });
    h.setHeader(layout('verse', 'bridge', 'chorus', 'outro'));
    h.pb.headerChanged();
    expect(rts(h).pendingStart).toBe(false);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('a range whose ends swap order disarms', () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    h.pb.arm({ loop: 'verse', through: 'chorus' });
    h.setHeader(layout('intro', 'chorus', 'verse'));
    h.pb.headerChanged();
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('a header that gains a meta error disarms', () => {
    const h = setup(layout('intro', 'verse'));
    h.pb.arm({ start: 'verse' });
    h.setHeader(null);
    h.pb.headerChanged();
    expect(rts(h).pendingStart).toBe(false);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('an evaluate in flight keeps the arm it will start from', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'), { live: true, audio: 'running' });
    h.pb.arm({ start: 'chorus' });
    const held = deferred();
    h.rt.hold(held.promise);
    const run = h.pb.evaluate(PA, 'a', opts(), 1);
    h.setHeader(layout('chorus', 'intro', 'verse'));
    h.pb.headerChanged();
    expect(rts(h)).toMatchObject({ pendingStart: true, armedSection: 2 });
    held.resolve();
    expect(await run).toBe(true);
    expect(h.st().started).toBe(true);
    // Started from the chorus at bar 4, where the evaluated text has it.
    expect(h.rt.segments().at(-1)).toEqual({ at: 0, songBar: 4, view: { kind: 'song' } });
  });

  test('an evaluate that resolves stopped catches the arm up with the header', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'), { live: true, audio: 'running' });
    h.pb.arm({ start: 'chorus' });
    const held = deferred();
    h.rt.hold(held.promise);
    h.rt.setResult(false);
    const run = h.pb.evaluate(PA, 'a', opts(), 1);
    h.setHeader(layout('intro', 'chorus', 'verse'));
    h.pb.headerChanged();
    expect(rts(h).armedSection).toBe(2);
    held.resolve();
    expect(await run).toBe(false);
    expect(rts(h)).toMatchObject({ pendingStart: true, armedSection: 1 });
    expect(h.tp().armed).toBe('chorus');
  });

  test('while playing the header is not consulted', async () => {
    const h = setup(layout('intro', 'verse', 'chorus'));
    await h.play();
    h.pb.arm({ loop: 'verse' });
    const before = h.rt.segments();
    h.setHeader(layout('verse', 'intro', 'chorus'));
    h.pb.headerChanged();
    h.setHeader(null);
    h.pb.headerChanged();
    expect(h.rt.segments()).toBe(before);
    expect(h.tp().view).toEqual({ kind: 'loop', section: 1 });
  });
});

describe('start', () => {
  /** Stopped, with `a` evaluated as intro/verse/chorus and a header that differs from it. */
  async function evaluated() {
    const h = setup(layout('bridge'), { live: true, audio: 'running' });
    await h.pb.evaluate(PA, 'a', opts(), 1);
    h.rt.runtime.stop();
    h.rt.patch({ structure: layout('intro', 'verse', 'chorus') });
    h.rt.setCycleTime((cycle) => 10 + cycle);
    return h;
  }

  test('arms the section of the evaluated song, then starts it in the same tick', async () => {
    const h = await evaluated();
    let seen: unknown = null;
    h.rt.setStart(() => {
      seen = { ...h.tp() };
      return true;
    });
    const pending = h.pb.start({
      section: { start: 'verse' },
      countIn: true,
      backend: 'superdough',
    });
    // No await before `runtime.start`: the resume stays inside the gesture.
    expect(h.rt.starts()).toEqual([{ countIn: true, backend: 'superdough' }]);
    expect(seen).toEqual({ view: { kind: 'song' }, loop: null, armed: 'verse' });
    expect(await pending).toEqual({ T: 11, cps: 0.5 });
    expect(h.st()).toMatchObject({ started: true, playing: { path: PA } });
    expect(h.tp().armed).toBeNull();
  });

  test('a looped section plays as a loop, named from the evaluated song', async () => {
    const h = await evaluated();
    expect(await h.pb.start({ section: { loop: 'chorus' }, countIn: false })).toEqual({
      T: 10,
      cps: 0.5,
    });
    expect(h.tp()).toEqual({
      view: { kind: 'loop', section: 2 },
      loop: { section: 'chorus', names: ['chorus'] },
      armed: null,
    });
  });

  test('a playing song stops first; without a section it starts from the top', async () => {
    const h = await evaluated();
    h.rt.setStarted(true);
    expect(await h.pb.start({ countIn: false })).toEqual({ T: 10, cps: 0.5 });
    expect(h.rt.stops()).toBe(2);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('a section the evaluated song lacks starts nothing', async () => {
    const h = await evaluated();
    expect(await h.pb.start({ section: { start: 'bridge' }, countIn: false })).toEqual({
      missingSection: 'bridge',
    });
    h.rt.patch({ structure: null });
    expect(await h.pb.start({ section: { loop: 'verse' }, countIn: false })).toEqual({
      missingSection: 'verse',
    });
    expect(h.rt.starts()).toEqual([]);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('a failed start disarms, unless the runtime started meanwhile', async () => {
    const h = await evaluated();
    h.rt.setStart(() => false);
    expect(await h.pb.start({ section: { loop: 'verse' }, countIn: false })).toBeNull();
    expect(h.tp()).toEqual(NOT_ARMED);
    expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
    h.rt.setStart(() => {
      h.rt.setStarted(true); // another start won the race
      return false;
    });
    expect(await h.pb.start({ section: { loop: 'verse' }, countIn: false })).toBeNull();
    expect(h.tp().view).toEqual({ kind: 'loop', section: 1 });
  });

  test('a start without a bar 1 time stops again and gives null', async () => {
    const h = await evaluated();
    h.rt.setCycleTime(() => null);
    expect(await h.pb.start({ countIn: true })).toBeNull();
    expect(h.rt.cycleTimes()).toEqual([1]);
    expect(h.st().started).toBe(false);
  });
});

// ---------------------------------------------------------------- through the session

const sectioned = (title: string, ...names: string[]) =>
  `const meta = {
  title: '${title}',
  sections: [${names.map((n) => `{ name: '${n}', bars: 2 }`).join(', ')}],
};
$: s("x")
`;
const SA = sectioned('Alpha', 'intro', 'verse', 'chorus');
const SB = sectioned('Beta', 'a', 'b');

async function opened(files: Record<string, string> = { [PA]: SA, [PB]: SB }) {
  const { api, fake } = fakeProjectApi({
    '/p': { name: 'Proj', files },
    '/q': { name: 'Other', files: { 'songs/q.js': SB } },
  });
  fake.last = '/p';
  const rt = stubRuntime();
  const view = new EditorView({ parent: document.body });
  let session!: Session;
  session = createSession({
    api,
    runtime: rt.runtime,
    createState: (doc: string) =>
      createEditorState(doc, rt.runtime, { onDocChange: () => session.docChanged() }),
    isTauri: () => true,
  });
  session.attachView(view);
  await session.init();
  await flush();
  const pb = session.playback;
  return {
    rt,
    fake,
    session,
    view,
    pb,
    tp: () => get(pb.store).transport,
    /** Evaluate the active song and play it with its own sections. */
    async play() {
      await session.saveAndEvaluate();
      rt.patch({ started: true, structure: get(session.store).structure });
    },
  };
}

describe('the session', () => {
  test('switching songs while stopped disarms', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    expect(h.tp().armed).toBe('verse');
    await h.session.switchSong(PB);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('switching songs while playing leaves the playing song its transport', async () => {
    const h = await opened();
    await h.play();
    h.pb.arm({ loop: 'verse' });
    await h.session.switchSong(PB);
    expect(h.tp().loop).toEqual({ section: 'verse', names: ['verse'] });
  });

  test('a project change stops, disarms and forgets the evaluated song', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate();
    h.pb.arm({ start: 'verse' });
    const stops = h.rt.stops();
    await h.session.openRecent('/q');
    expect(h.rt.stops()).toBe(stops + 1);
    expect(get(h.pb.store)).toMatchObject({ evaluated: null, transport: NOT_ARMED });
  });

  test('a different song taking over playback evaluates on a reset transport', async () => {
    const h = await opened();
    await h.play();
    h.pb.arm({ loop: 'verse' });
    await h.session.switchSong(PB);
    let seen: unknown = null;
    const evaluate = h.rt.runtime.evaluate;
    h.rt.runtime.evaluate = (code, o) => {
      seen = get(h.rt.runtime.transport.store).view;
      return evaluate(code, o);
    };
    await h.session.saveAndEvaluate();
    expect(seen).toEqual({ kind: 'song' });
    expect(get(h.pb.store).playing).toEqual({ path: PB, title: 'Beta' });
  });

  test('re-evaluating the playing song keeps its loop', async () => {
    const h = await opened();
    await h.play();
    h.pb.arm({ loop: 'chorus' });
    await h.session.saveAndEvaluate();
    expect(h.tp().loop).toEqual({ section: 'chorus', names: ['chorus'] });
  });

  test('Play keeps a start armed while stopped, also after a switch', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate(); // A evaluated, stopped
    await h.session.switchSong(PB);
    h.pb.arm({ start: 'b' });
    await h.session.saveAndEvaluate(); // B is not taking over playback
    expect(h.tp().armed).toBe('b');
  });

  test('a meta error refusing the evaluate disarms', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    h.view.dispatch({
      changes: {
        from: 0,
        to: h.view.state.doc.length,
        insert: SA.replace("'Alpha'", "'A', bpm: 'x'"),
      },
    });
    await h.session.saveAndEvaluate();
    expect(h.rt.evaluated).toEqual([]);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('a meta error in the edited header disarms before the store publishes', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    const seen: (string | null)[] = [];
    const off = h.session.store.subscribe((st) => {
      if (st.metaError !== null) seen.push(h.tp().armed);
    });
    h.view.dispatch({
      changes: {
        from: 0,
        to: h.view.state.doc.length,
        insert: SA.replace("'Alpha'", "'A', bpm: 'x'"),
      },
    });
    off();
    expect(seen).toEqual([null]);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('editing the header while stopped keeps the armed start on its name', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    const edit = (from: string, to: string) => {
      const text = h.view.state.doc.toString();
      h.view.dispatch({ changes: { from: 0, to: text.length, insert: text.replace(from, to) } });
    };
    edit("name: 'verse'", "name: 'tmp'");
    expect(h.tp()).toEqual(NOT_ARMED); // renamed away: disarmed as the header changes
    edit("name: 'tmp'", "name: 'verse'");
    h.pb.arm({ start: 'verse' });
    edit(
      "{ name: 'verse', bars: 2 }, { name: 'chorus', bars: 2 }",
      "{ name: 'chorus', bars: 2 }, { name: 'verse', bars: 2 }",
    );
    expect(get(h.rt.runtime.transport.store)).toMatchObject({
      pendingStart: true,
      armedSection: 2,
    });
    expect(h.tp().armed).toBe('verse');
  });

  test('deleting the active song while stopped disarms for the next one', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    await h.session.deleteSong(PA, { discard: false });
    expect(get(h.session.store).active).toBe(PB);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('deleting the active song while another plays leaves its transport alone', async () => {
    const h = await opened();
    await h.session.switchSong(PB);
    await h.play();
    h.pb.arm({ loop: 'b' });
    await h.session.switchSong(PA);
    await h.session.deleteSong(PA, { discard: false });
    expect(h.tp().loop).toEqual({ section: 'b', names: ['b'] });
    expect(get(h.pb.store).playing).toEqual({ path: PB, title: 'Beta' });
  });

  test('deleting the stopped evaluated song leaves the next song its armed start', async () => {
    const h = await opened();
    await h.session.saveAndEvaluate(); // A evaluated
    h.rt.runtime.stop();
    await h.session.switchSong(PB);
    h.pb.arm({ start: 'b' });
    const stops = h.rt.stops();
    expect(await h.session.deleteSong(PA, { discard: false })).toEqual({
      ok: true,
      stopped: false,
    });
    expect(h.rt.stops()).toBe(stops);
    expect(get(h.pb.store).evaluated).toBeNull();
    expect(h.tp().armed).toBe('b');
  });

  /** A, active, deleted on disk; B also has a chorus. */
  async function deletedOnDisk() {
    const h = await opened({ [PA]: SA, [PB]: sectioned('Beta', 'intro', 'chorus') });
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(get(h.session.store).songs.find((s) => s.path === PA)?.conflict).toBe('deleted');
    return h;
  }

  test('closing a song deleted on disk disarms for the next one', async () => {
    const h = await deletedOnDisk();
    h.pb.arm({ start: 'chorus' });
    expect(h.tp().armed).toBe('chorus');
    await h.session.closeDeleted();
    expect(get(h.session.store).active).toBe(PB);
    expect(h.tp()).toEqual(NOT_ARMED);
  });

  test('closing the playing song deleted on disk stops it', async () => {
    const h = await deletedOnDisk();
    await h.play();
    expect(get(h.pb.store).playing).toEqual({ path: PA, title: 'Alpha' });
    const stops = h.rt.stops();
    await h.session.closeDeleted();
    expect(h.rt.stops()).toBe(stops + 1);
    expect(get(h.pb.store)).toMatchObject({ evaluated: null, playing: null, started: false });
  });

  test('deleting an inactive song leaves the armed start alone', async () => {
    const h = await opened();
    h.pb.arm({ start: 'verse' });
    await h.session.deleteSong(PB, { discard: false });
    expect(h.tp().armed).toBe('verse');
  });
});

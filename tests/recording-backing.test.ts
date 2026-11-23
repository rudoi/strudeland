import { describe, expect, test } from 'bun:test';
import { writable } from 'svelte/store';
import type { MetronomeGrid } from '../src/lib/audio/metronome';
import type { EvalOutcome } from '../src/lib/control/protocol';
import { createBackingStarter } from '../src/lib/recording/backing';
import { buildStructure } from '../src/lib/strudel/structure';
import { stubRuntime } from './helpers/stub-runtime';

/** A session with an open song (or none) whose evaluate answers `outcome`, and a metronome log. */
function setup(opts: { active?: string | null; outcome?: Partial<EvalOutcome> } = {}) {
  const rt = stubRuntime({ live: true, audio: 'running' });
  const store = writable({ active: opts.active === undefined ? 'songs/a.js' : opts.active });
  const evaluations: [string, { autostart?: boolean } | undefined][] = [];
  let outcome: Partial<EvalOutcome> = opts.outcome ?? { ok: true };
  const session = {
    store: { subscribe: store.subscribe },
    async evaluateBuffer(path: string, o?: { autostart?: boolean }): Promise<EvalOutcome> {
      evaluations.push([path, o]);
      // The real session evaluates through the runtime; the stub applies the tempo.
      if (outcome.ok) await rt.runtime.evaluate('s("x")', { cps: 0.75, ...o });
      return {
        ok: false,
        song: path,
        title: 'a',
        started: false,
        audio: 'running',
        error: null,
        ...outcome,
      };
    },
  };
  const metronome = {
    log: [] as string[],
    grids: [] as MetronomeGrid[],
    result: true,
    start(g: MetronomeGrid) {
      this.log.push('start');
      this.grids.push(g);
      return this.result;
    },
    stop() {
      this.log.push('stop');
    },
  };
  const backing = createBackingStarter({ runtime: rt.runtime, session, metronome });
  return {
    rt,
    backing,
    evaluations,
    metronome,
    setOutcome: (o: Partial<EvalOutcome>) => void (outcome = o),
    setActive: (a: string | null) => store.set({ active: a }),
  };
}

const started = (h: ReturnType<typeof setup>) => {
  let s = false;
  h.rt.runtime.store.subscribe((v) => (s = v.started))();
  return s;
};

describe('prepareSong', () => {
  test('evaluates the open song without autostart and reads the tempo from the runtime', async () => {
    const h = setup();
    h.rt.patch({ tempoOverride: true, meter: { num: 6, den: 8 } });
    expect(await h.backing.prepareSong()).toEqual({
      ok: true,
      cps: 0.75,
      meter: { num: 6, den: 8 },
      tempoOverride: true,
    });
    expect(h.evaluations).toEqual([['songs/a.js', { autostart: false }]]);
    expect(started(h)).toBe(false);
    expect(h.rt.starts()).toEqual([]);
  });

  test('an eval error', async () => {
    const h = setup({ outcome: { ok: false, error: 'boom is not defined' } });
    expect(await h.backing.prepareSong()).toEqual({
      ok: false,
      kind: 'eval',
      message: 'boom is not defined',
    });
  });

  test('a meta error', async () => {
    const h = setup({ outcome: { ok: false, error: 'meta: bpm must be a number (1:20)' } });
    expect(await h.backing.prepareSong()).toEqual({
      ok: false,
      kind: 'meta',
      message: 'meta: bpm must be a number (1:20)',
    });
  });

  test('a failure without a message', async () => {
    const h = setup({ outcome: { ok: false, error: null } });
    expect(await h.backing.prepareSong()).toEqual({
      ok: false,
      kind: 'eval',
      message: 'evaluate failed',
    });
  });

  test('no open song is an eval failure and evaluates nothing', async () => {
    const h = setup({ active: null });
    expect(await h.backing.prepareSong()).toEqual({
      ok: false,
      kind: 'eval',
      message: 'no song is open',
    });
    expect(h.evaluations).toEqual([]);
  });
});

describe('startSong', () => {
  test('prepared, with a count-in: starts with it and T is cycle 1', async () => {
    const h = setup();
    h.rt.setCycleTime((c) => 10 + c * 2);
    expect(await h.backing.startSong({ countIn: true, prepared: true })).toEqual({
      T: 12,
      cps: 0.5,
    });
    expect(h.evaluations).toEqual([]);
    expect(h.rt.starts()).toEqual([{ countIn: true, backend: 'superdough' }]);
    expect(h.rt.cycleTimes()).toEqual([1]);
  });

  test('not prepared, no count-in: evaluates first and T is cycle 0', async () => {
    const h = setup();
    h.rt.setCycleTime((c) => 10 + c * 2);
    expect(await h.backing.startSong()).toEqual({ T: 10, cps: 0.75 });
    expect(h.evaluations).toEqual([['songs/a.js', { autostart: false }]]);
    expect(h.rt.starts()).toEqual([{ countIn: false, backend: 'superdough' }]);
    expect(h.rt.cycleTimes()).toEqual([0]);
  });

  test('an evaluate failure starts nothing', async () => {
    const h = setup({ outcome: { ok: false, error: 'boom' } });
    h.rt.setCycleTime(() => 1);
    expect(await h.backing.startSong()).toBeNull();
    expect(h.rt.starts()).toEqual([]);
  });

  test('a playing runtime is stopped first', async () => {
    const h = setup();
    h.rt.setStarted(true);
    h.rt.setCycleTime(() => 3);
    const order: string[] = [];
    const stop = h.rt.runtime.stop;
    h.rt.runtime.stop = () => {
      order.push('stop');
      stop();
    };
    h.rt.setStart(() => {
      order.push('start');
      return true;
    });
    expect(await h.backing.startSong({ prepared: true })).toEqual({ T: 3, cps: 0.5 });
    expect(order).toEqual(['stop', 'start']);
  });

  test('a failed start gives null (the runtime restored itself)', async () => {
    const h = setup();
    h.rt.setStart(() => false);
    h.rt.setCycleTime(() => 3);
    expect(await h.backing.startSong({ prepared: true })).toBeNull();
    expect(h.rt.cycleTimes()).toEqual([]);
    expect(h.rt.stops()).toBe(0);
  });

  test('a null T stops the runtime again and gives null', async () => {
    const h = setup();
    expect(await h.backing.startSong({ countIn: true, prepared: true })).toBeNull();
    expect(h.rt.stops()).toBe(1);
    expect(started(h)).toBe(false);
    // It did not start the song, so stopAll leaves the runtime alone.
    h.rt.setStarted(true);
    h.backing.stopAll();
    expect(h.rt.stops()).toBe(1);
  });
});

describe('startSong with a section', () => {
  const structure = buildStructure({
    bpm: 120,
    sections: [
      { name: 'intro', bars: 2 },
      { name: 'verse', bars: 4, loop: true },
    ],
  });

  test('arms the section by name after the stop, then starts', async () => {
    const h = setup();
    h.rt.patch({ structure });
    h.rt.setStarted(true);
    h.rt.setCycleTime(() => 3);
    const order: string[] = [];
    const stop = h.rt.runtime.stop;
    h.rt.runtime.stop = () => {
      order.push('stop');
      stop();
    };
    h.rt.setStart(() => {
      order.push(...h.rt.transportCalls(), 'start');
      return true;
    });
    const r = await h.backing.startSong({
      prepared: true,
      section: { name: 'verse', loop: false },
    });
    expect(r).toEqual({ T: 3, cps: 0.5 });
    expect(order).toEqual(['stop', 'startFrom 1', 'start']);
    h.rt.setStarted(false);
    expect(
      await h.backing.startSong({ prepared: true, section: { name: 'verse', loop: true } }),
    ).toEqual({ T: 3, cps: 0.5 });
    expect(h.rt.transportCalls().at(-1)).toBe('loopSection 1');
  });

  test('a section the evaluated song lacks starts nothing', async () => {
    const h = setup();
    h.rt.patch({ structure });
    h.rt.setCycleTime(() => 3);
    expect(
      await h.backing.startSong({ prepared: true, section: { name: 'bridge', loop: false } }),
    ).toEqual({ missingSection: 'bridge' });
    h.rt.patch({ structure: null });
    expect(
      await h.backing.startSong({ prepared: true, section: { name: 'verse', loop: true } }),
    ).toEqual({ missingSection: 'verse' });
    expect(h.rt.starts()).toEqual([]);
    expect(h.rt.transportCalls()).toEqual([]);
  });

  test('a failed start disarms the section', async () => {
    const h = setup();
    h.rt.patch({ structure });
    h.rt.setStart(() => false);
    expect(
      await h.backing.startSong({ prepared: true, section: { name: 'intro', loop: false } }),
    ).toBeNull();
    expect(h.rt.transportCalls()).toEqual(['startFrom 0', 'reset']);
  });
});

describe('startMetronome and stopAll', () => {
  test('startMetronome passes the grid through', () => {
    const h = setup();
    const grid = { T: 4, cps: 0.5, num: 4, from: -4, until: 4 };
    expect(h.backing.startMetronome(grid)).toBe(true);
    expect(h.metronome.grids).toEqual([grid]);
    h.metronome.result = false;
    expect(h.backing.startMetronome(grid)).toBe(false);
  });

  test('stopAll stops the metronome and the song it started, once', async () => {
    const h = setup();
    h.rt.setCycleTime(() => 1);
    await h.backing.startSong({ prepared: true });
    expect(started(h)).toBe(true);
    h.backing.stopAll();
    expect(h.metronome.log).toEqual(['stop']);
    expect(h.rt.stops()).toBe(1);
    expect(started(h)).toBe(false);
    h.backing.stopAll();
    expect(h.metronome.log).toEqual(['stop', 'stop']);
    expect(h.rt.stops()).toBe(1);
  });

  test('stopAll leaves a song it did not start playing', () => {
    const h = setup();
    h.rt.setStarted(true);
    h.backing.stopAll();
    expect(h.metronome.log).toEqual(['stop']);
    expect(h.rt.stops()).toBe(0);
    expect(started(h)).toBe(true);
  });

  test('stopAll does not stop a song that already ended on its own', async () => {
    const h = setup();
    h.rt.setCycleTime(() => 1);
    await h.backing.startSong({ prepared: true });
    h.rt.setStarted(false); // auto-stop
    h.backing.stopAll();
    expect(h.rt.stops()).toBe(0);
  });
});

test('fixed-grid song backing refuses mixed meter before starting', async () => {
  const h = setup();
  h.rt.patch({
    structure: buildStructure({
      sections: [{ name: 'body', bars: 2 }],
      meterChanges: [{ bar: 2, meter: '3/4' }],
    }),
  });
  expect(await h.backing.prepareSong()).toEqual({
    ok: false,
    kind: 'meta',
    message: 'song backing requires a fixed time signature',
  });
  expect(await h.backing.startSong({ prepared: true })).toBeNull();
  expect(h.rt.starts()).toEqual([]);
});

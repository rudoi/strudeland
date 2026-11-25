import { describe, expect, test } from 'bun:test';
import { get, writable } from 'svelte/store';
import type { MetronomeGrid } from '../src/lib/audio/metronome';
import { createOutputClock } from '../src/lib/audio/output-clock';
import { createSongPlayback } from '../src/lib/project/song-playback';
import { LEAD_S, type BackingStarter, type PrepareOutcome } from '../src/lib/recording/backing';
import {
  COUNT_IN_TICK_MS,
  createRecordingController,
  MESSAGES,
  OUTPUT_RETRIES,
  OUTPUT_RETRY_MS,
  STAMP_INTERVAL_MS,
} from '../src/lib/recording/controller';
import { RecordingError, type BackingChoice, type StopFields } from '../src/lib/recording/types';
import { buildStructure, type SongStructure } from '../src/lib/strudel/structure';
import { fakeAudioGraph } from './helpers/fake-audio-graph';
import { deferred, flush } from './helpers/fake-project-api';
import { INTERFACE, MIC, fakeRecordingApi } from './helpers/fake-recording-api';
import { stubRuntime } from './helpers/stub-runtime';

// ---------------------------------------------------------------- harness

/** The host clock is this far ahead of `performance.now()`, so every offset is OFFSET − 0.5 ms. */
const OFFSET = 7_000_000_000_000n;
const START_OFFSET = { offsetNs: '6999999500000', rttNs: '1000000' };
/** The output stamp: context time 5 is heard at performance time 2000 ms. */
const STAMP = { contextTime: 5, performanceTime: 2000 };
const HOST = (ms: number) => (OFFSET - 500_000n + BigInt(Math.round(ms * 1e6))).toString();

const SONG_ON: BackingChoice = { song: true, metronome: true, countIn: true };
const STRUCTURE = buildStructure({
  bpm: 90,
  meter: '3/4',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'verse', bars: 4, loop: true },
  ],
});

function setup(
  opts: {
    project?: string | null;
    active?: string | null;
    structure?: SongStructure | null;
    prepare?: PrepareOutcome;
    resume?: boolean;
    outputDelaySec?: () => number;
  } = {},
) {
  const log: string[] = [];
  const { api: raw, fake } = fakeRecordingApi({ roots: ['/p', '/q'] });
  // Calls of the API and the backing in one log (pings excepted).
  const api = new Proxy(raw, {
    get(target, key: string) {
      const v = (target as any)[key];
      if (typeof v !== 'function' || key === 'hostNowNs' || key.startsWith('on')) return v;
      return (...args: unknown[]) => {
        log.push(`api.${key}`);
        return v.apply(target, args);
      };
    },
  });
  let ms = 1000;
  const nowMs = () => ms++;
  fake.hostNowNs = async () => OFFSET + BigInt(Math.round((ms - 1) * 1e6));

  const rt = stubRuntime({ live: true, audio: 'running' });
  const resume = opts.resume ?? true;
  rt.runtime.resumeAudio = async () => resume;
  const realResume = rt.runtime.resumeAudio;
  rt.runtime.resumeAudio = () => {
    log.push('resumeAudio');
    return realResume();
  };
  const realStop = rt.runtime.stop;
  rt.runtime.stop = () => {
    log.push('runtime.stop');
    realStop();
  };

  const sessionStore = writable({
    project: opts.project === null ? null : { root: opts.project ?? '/p' },
    active: opts.active === undefined ? 'songs/a.js' : opts.active,
    structure: opts.structure === undefined ? STRUCTURE : opts.structure,
  });
  // Arms against the session's header, as the timeline does.
  const playback = createSongPlayback({
    runtime: rt.runtime,
    getHeader: () => {
      const { active, structure } = get(sessionStore);
      return { active, structure: active === null ? null : structure, metaError: null };
    },
    currentGen: () => 0,
    titleFrom: (path) => path,
  });
  const g = fakeAudioGraph({
    currentTime: 5,
    outputTimestamp: STAMP,
    baseLatency: 0.005,
  });
  let graphOn = true;
  const outputClock = createOutputClock({ graph: () => (graphOn ? g.graph : null) });

  const backing = {
    prepare: opts.prepare ?? {
      ok: true,
      cps: 0.5,
      meter: { num: 4, den: 4 },
      tempoOverride: false,
    },
    song: { T: 7, cps: 0.5 } as { T: number; cps: number } | null,
    metronomeOk: true,
    grids: [] as MetronomeGrid[],
    holdPrepare: null as Promise<void> | null,
    holdSong: null as Promise<void> | null,
  };
  const starter: BackingStarter = {
    async prepareSong() {
      log.push('prepareSong');
      await backing.holdPrepare;
      return backing.prepare;
    },
    async startSong(o) {
      log.push(`startSong ${JSON.stringify(o)}`);
      await backing.holdSong;
      return backing.song;
    },
    startMetronome(grid) {
      log.push('startMetronome');
      backing.grids.push(grid);
      return backing.metronomeOk;
    },
    stopAll() {
      log.push('stopAll');
    },
  };

  const timers = new Map<number, { fn: () => void; ms: number }>();
  let nextTimer = 0;
  const sleeps: number[] = [];
  let onSleep: (() => void) | null = null;
  const c = createRecordingController({
    api,
    runtime: rt.runtime,
    playback,
    session: { store: { subscribe: sessionStore.subscribe } },
    backing: starter,
    outputClock,
    nowMs,
    ...(opts.outputDelaySec ? { outputDelaySec: opts.outputDelaySec } : {}),
    setInterval: (fn, every) => {
      timers.set(++nextTimer, { fn, ms: every });
      return nextTimer;
    },
    clearInterval: (id) => void timers.delete(id as number),
    sleep: async (t) => {
      sleeps.push(t);
      onSleep?.();
    },
  });
  const st = () => get(c.store);
  return {
    c,
    st,
    fake,
    log,
    rt,
    playback,
    g,
    backing,
    timers,
    sleeps,
    sessionStore,
    api: raw,
    setGraph: (on: boolean) => void (graphOn = on),
    onSleep: (f: (() => void) | null) => void (onSleep = f),
    /** Run every timer with period `every`. */
    fire: (every: number) => {
      for (const t of [...timers.values()]) if (t.ms === every) t.fn();
    },
    setRoot: (root: string | null) =>
      sessionStore.update((s) => ({ ...s, project: root === null ? null : { root } })),
    level: () => get(c.level),
    lastStop: (): StopFields | null => fake.stops[fake.stops.length - 1] ?? null,
  };
}

/** A controller that finished its setup and armed. */
async function armed(opts: Parameters<typeof setup>[0] = {}) {
  const h = setup(opts);
  await flush();
  expect(h.st().phase).toBe('armed');
  h.log.length = 0;
  return h;
}

const TEMPO_44 = { bpm: 120, meter: '4/4', cps: 0.5, beatsPerBar: 4, tempoOverride: false };
const TEMPO_34 = { bpm: 90, meter: '3/4', cps: 0.5, beatsPerBar: 3, tempoOverride: false };

// ---------------------------------------------------------------- setup and arming

describe('setup and arming', () => {
  test('status and devices fill the state, then it arms with a project open', async () => {
    const h = setup();
    await flush();
    expect(h.log).toEqual(['api.status', 'api.devices', 'api.arm']);
    expect(h.st()).toEqual({
      phase: 'armed',
      countInBeat: null,
      takeStartedAtMs: null,
      devices: [MIC],
      input: { deviceId: 'mic-1', channel: 0 },
      error: null,
      permission: 'authorized',
      warning: null,
      stem: null,
      lastStem: null,
    });
    expect(h.fake.listeners()).toEqual({ state: 1, level: 1 });
  });

  test('a recorder already armed is adopted without arming again', async () => {
    const h = setup();
    h.fake.status = {
      ...h.fake.status,
      state: 'armed',
      permission: 'authorized',
      device: INTERFACE,
      channel: 1,
    };
    await flush();
    expect(h.fake.names()).toEqual(['status', 'devices']);
    expect(h.st()).toMatchObject({
      phase: 'armed',
      permission: 'authorized',
      input: { deviceId: 'usb-2', channel: 1 },
    });
  });

  test('without a project it stays idle; the first project arms (arming, then armed)', async () => {
    const h = setup({ project: null });
    await flush();
    expect(h.st().phase).toBe('idle');
    expect(h.fake.names()).toEqual(['status', 'devices']);
    const gate = deferred();
    h.fake.hold('arm', gate.promise);
    h.setRoot('/p');
    expect(h.st().phase).toBe('arming');
    gate.resolve();
    await flush();
    expect(h.st().phase).toBe('armed');
  });

  test.each([
    ['permissionDenied', 'microphone access denied'],
    ['noDevice', 'no input device found'],
    ['other', 'stream failed'],
  ] as const)('a failed arm (%s) is idle with the error', async (kind, message) => {
    const h = setup();
    h.fake.failNext('arm', new RecordingError(message, kind));
    await flush();
    expect(h.st()).toMatchObject({ phase: 'idle', error: { kind, message } });
    // An error keeps it idle: no retry loop.
    h.setRoot('/q');
    await flush();
    expect(h.fake.names().filter((n) => n === 'arm')).toEqual(['arm']);
  });

  test('a permission refusal re-reads the permission', async () => {
    const h = setup();
    h.fake.failNext('arm', new RecordingError('microphone access denied', 'permissionDenied'));
    await flush();
    expect(h.st().permission).toBe('notDetermined');
    h.fake.status = { ...h.fake.status, permission: 'restricted' };
    h.fake.failNext('arm', new RecordingError('restricted', 'permissionDenied'));
    await h.c.refreshDevices();
    await flush();
    expect(h.st()).toMatchObject({
      phase: 'idle',
      permission: 'restricted',
      error: { kind: 'permissionDenied', message: 'restricted' },
    });
    expect(h.fake.names()).toEqual([
      'status',
      'devices',
      'arm',
      'status',
      'devices',
      'arm',
      'status',
    ]);
  });

  test('an error event while arming is idle with the error; the late arm result is dropped', async () => {
    const h = setup();
    const gate = deferred();
    h.fake.hold('arm', gate.promise);
    await flush();
    expect(h.st().phase).toBe('arming');
    h.fake.emitState({
      state: 'idle',
      error: { kind: 'permissionDenied', message: 'microphone access denied' },
    });
    expect(h.st()).toMatchObject({
      phase: 'idle',
      error: { kind: 'permissionDenied', message: 'microphone access denied' },
    });
    gate.resolve();
    await flush();
    expect(h.st().phase).toBe('idle');
  });

  test('an error event while armed (the stream is gone) is idle with the error', async () => {
    const h = await armed();
    h.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'device lost' } });
    expect(h.st()).toMatchObject({ phase: 'idle', error: { kind: 'deviceLost' } });
    expect(h.log).toEqual([]);
  });

  test('error-free state events and events in idle change nothing', async () => {
    const h = await armed();
    h.fake.emitState({ state: 'armed' });
    h.fake.emitState({ state: 'idle' });
    expect(h.st().phase).toBe('armed');
    const i = setup({ project: null });
    await flush();
    i.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'x' } });
    expect(i.st()).toMatchObject({ phase: 'idle', error: null });
  });

  test('a project switch while armed re-arms nothing (arming is app-level)', async () => {
    const h = await armed();
    h.setRoot('/q');
    await flush();
    expect(h.log).toEqual([]);
    expect(h.st().phase).toBe('armed');
  });
});

// ---------------------------------------------------------------- level

describe('level', () => {
  test('null until armed, the latest payload while armed, null again when idle', async () => {
    const h = setup();
    h.fake.emitLevel({ peak: 0.9, rms: 0.3 });
    expect(h.level()).toBeNull();
    await flush();
    h.fake.emitLevel({ peak: 0.5, rms: 0.25 });
    expect(h.level()).toEqual({ peak: 0.5, rms: 0.25 });
    h.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'gone' } });
    expect(h.level()).toBeNull();
    h.fake.emitLevel({ peak: 0.5, rms: 0.25 });
    expect(h.level()).toBeNull();
  });
});

// ---------------------------------------------------------------- record

describe('record', () => {
  test('the whole sequence with song, metronome and count-in, then stop', async () => {
    const h = await armed();
    const p = h.c.record(SONG_ON);
    // resumeAudio is the first thing, synchronously inside the gesture.
    expect(h.log).toEqual(['resumeAudio']);
    expect(h.st().phase).toBe('starting');
    await p;
    expect(h.log).toEqual([
      'resumeAudio',
      'prepareSong',
      'api.start',
      'startSong {"countIn":true,"prepared":true}',
      'startMetronome',
    ]);
    expect(h.fake.argsOf('start')).toEqual([
      [
        {
          root: '/p',
          songStem: 'a',
          fields: {
            tempo: TEMPO_44,
            backing: {
              song: true,
              metronome: true,
              countIn: true,
              songPath: 'songs/a.js',
              startSection: null,
              loopSection: null,
            },
          },
        },
      ],
    ]);
    expect(h.fake.pings).toBe(20);
    expect(h.backing.grids).toEqual([{ T: 7, cps: 0.5, num: 4, from: -4, until: undefined }]);
    expect(h.st()).toMatchObject({
      phase: 'count-in',
      countInBeat: 1,
      takeStartedAtMs: 1000,
      stem: 'a-01',
      error: null,
      warning: null,
    });

    // The count-in beat follows the context clock (4/4 at 0.5 cps: a click every 0.5 s from 5).
    for (const [t, beat] of [
      [5.49, 1],
      [5.5, 2],
      [6.2, 3],
      [6.99, 4],
    ] as const) {
      h.g.setTime(t);
      h.fire(COUNT_IN_TICK_MS);
      expect(h.st().countInBeat).toBe(beat);
    }
    h.g.setTime(7);
    h.fire(COUNT_IN_TICK_MS);
    expect(h.st()).toMatchObject({ phase: 'recording', countInBeat: null });

    // An output stamp each second.
    h.g.setOutputTimestamp({ contextTime: 7.5, performanceTime: 4500 });
    h.fire(STAMP_INTERVAL_MS);
    h.g.setTime(8);
    h.g.setOutputTimestamp({ contextTime: 8, performanceTime: 5000 });
    h.log.length = 0;
    await h.c.stop();
    expect(h.log).toEqual(['stopAll', 'api.stop']);
    expect(h.fake.pings).toBe(40);
    expect(h.lastStop()).toEqual({
      output: { baseLatency: 0.005, outputLatency: null },
      tempo: TEMPO_44,
      backing: {
        song: true,
        metronome: true,
        countIn: true,
        songPath: 'songs/a.js',
        startSection: null,
        loopSection: null,
      },
      timing: {
        // T = 7 is heard 2 s after the stamp's 2000 ms.
        bar1HostNs: HOST(4000),
        clockOffsetNs: { start: START_OFFSET, end: START_OFFSET },
        outputStamps: [
          [5, HOST(2000)],
          [7.5, HOST(4500)],
          [8, HOST(5000)],
        ],
      },
    });
    expect(h.st()).toMatchObject({
      phase: 'armed',
      stem: null,
      lastStem: 'a-01',
      takeStartedAtMs: null,
      error: null,
    });
    expect(h.timers.size).toBe(0);
  });

  test('song on with a count-in but no metronome: the count-in clicks stop at T', async () => {
    const h = await armed();
    await h.c.record({ song: true, metronome: false, countIn: true });
    expect(h.backing.grids).toEqual([{ T: 7, cps: 0.5, num: 4, from: -4, until: 7 }]);
    expect(h.st().phase).toBe('count-in');
  });

  test('song on alone: no metronome and straight to recording', async () => {
    const h = await armed();
    await h.c.record({ song: true, metronome: false, countIn: false });
    expect(h.log).toEqual([
      'resumeAudio',
      'prepareSong',
      'api.start',
      'startSong {"countIn":false,"prepared":true}',
    ]);
    expect(h.st()).toMatchObject({ phase: 'recording', countInBeat: null });
    await h.c.stop();
    expect(h.lastStop()?.timing.bar1HostNs).toBe(null); // stopped at 5, before T = 7
  });

  test('a stop after T keeps bar 1', async () => {
    const h = await armed();
    await h.c.record({ song: true, metronome: false, countIn: false });
    h.g.setTime(7);
    await h.c.stop();
    expect(h.lastStop()?.timing.bar1HostNs).toBe(HOST(4000));
  });

  test('bar 1 is later by the output delay, read when bar 1 is marked', async () => {
    let reads = 0;
    const h = await armed({ outputDelaySec: () => (reads++, 0.0015) });
    await h.c.record({ song: true, metronome: false, countIn: false });
    h.g.setTime(7);
    await h.c.stop();
    expect(h.lastStop()?.timing.bar1HostNs).toBe(HOST(4001.5));
    expect(reads).toBe(1);
    // the sidecar's stamps stay raw
    expect(h.lastStop()?.timing.outputStamps[0]).toEqual([5, HOST(2000)]);
  });

  test('metronome only: the tempo from the header, T a lead after now, songPath null', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect(h.log).toEqual(['resumeAudio', 'api.start', 'startMetronome']);
    expect(h.fake.argsOf('start')[0]).toEqual([
      {
        root: '/p',
        songStem: 'a',
        fields: {
          tempo: TEMPO_34,
          backing: {
            song: false,
            metronome: true,
            countIn: false,
            songPath: null,
            startSection: null,
            loopSection: null,
          },
        },
      },
    ]);
    expect(h.backing.grids).toEqual([
      { T: 5 + LEAD_S, cps: 0.5, num: 3, from: 0, until: undefined },
    ]);
    expect(h.st().phase).toBe('recording');
    h.g.setTime(5.25);
    await h.c.stop();
    expect(h.lastStop()?.timing.bar1HostNs).toBe(HOST(2250));
  });

  test('metronome with a count-in: T is one bar later and the beats count 1..3', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: true, countIn: true });
    // 3/4 at 0.5 cps: a bar is 2 s, a click 2/3 s.
    expect(h.backing.grids).toEqual([{ T: 7.25, cps: 0.5, num: 3, from: -3, until: undefined }]);
    expect(h.st()).toMatchObject({ phase: 'count-in', countInBeat: 1 });
    for (const [t, beat] of [
      [5.25, 1],
      [5.95, 2],
      [6.6, 3],
    ] as const) {
      h.g.setTime(t);
      h.fire(COUNT_IN_TICK_MS);
      expect(h.st().countInBeat).toBe(beat);
    }
    h.g.setTime(7.25);
    h.fire(COUNT_IN_TICK_MS);
    expect(h.st().phase).toBe('recording');
  });

  test('without a header structure the defaults apply', async () => {
    const h = await armed({ structure: null });
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect((h.fake.argsOf('start')[0]![0] as any).fields.tempo).toEqual(TEMPO_44);
  });

  test('all off: a plain memo, no backing, the clock offset still measured', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: false, countIn: true });
    expect(h.log).toEqual(['resumeAudio', 'api.start']);
    expect(h.fake.pings).toBe(20);
    expect(h.st()).toMatchObject({ phase: 'recording', countInBeat: null, stem: 'a-01' });
    await h.c.stop();
    expect(h.log).toEqual(['resumeAudio', 'api.start', 'stopAll', 'api.stop']);
    expect(h.lastStop()).toEqual({
      output: { baseLatency: 0.005, outputLatency: null },
      tempo: TEMPO_34,
      backing: {
        song: false,
        metronome: false,
        countIn: false,
        songPath: null,
        startSection: null,
        loopSection: null,
      },
      timing: {
        bar1HostNs: null,
        clockOffsetNs: { start: START_OFFSET, end: START_OFFSET },
        outputStamps: [[5, HOST(2000)]],
      },
    });
  });

  test('all off does not need audio', async () => {
    const h = await armed({ resume: false });
    await h.c.record({ song: false, metronome: false, countIn: false });
    expect(h.st().phase).toBe('recording');
  });

  test('without an open song the song is off; the memo is named memo-nn', async () => {
    const h = await armed({ active: null });
    await h.c.record(SONG_ON);
    expect(h.log).toEqual(['resumeAudio', 'api.start', 'startMetronome']);
    const args = h.fake.argsOf('start')[0]![0] as any;
    expect(args.songStem).toBeUndefined();
    expect('songStem' in args).toBe(false);
    expect(args.fields.backing).toMatchObject({ song: false, metronome: true, countIn: true });
    expect(h.st().stem).toBe('memo-01');
  });

  test('record while playing stops the song first and drops the armed section', async () => {
    const h = await armed();
    h.rt.patch({ started: true, structure: STRUCTURE });
    h.playback.arm({ loop: 'verse' });
    expect(get(h.playback.store).transport.loop?.section).toBe('verse');
    await h.c.record({ song: true, metronome: false, countIn: false });
    expect(h.log.slice(0, 3)).toEqual(['resumeAudio', 'runtime.stop', 'prepareSong']);
    expect((h.fake.argsOf('start')[0]![0] as any).fields.backing).toMatchObject({
      startSection: null,
      loopSection: null,
    });
  });

  test('a section armed while stopped is saved: start here, or loop this section', async () => {
    const h = await armed();
    h.playback.arm({ start: 'verse' });
    await h.c.record({ song: true, metronome: false, countIn: false });
    expect((h.fake.argsOf('start')[0]![0] as any).fields.backing).toMatchObject({
      startSection: 'verse',
      loopSection: null,
    });
    await h.c.stop();
    h.playback.arm({ loop: 'verse' });
    await h.c.record({ song: true, metronome: false, countIn: false });
    expect((h.fake.argsOf('start')[1]![0] as any).fields.backing).toMatchObject({
      startSection: null,
      loopSection: 'verse',
    });
    // With the song off nothing is armed.
    await h.c.stop();
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect((h.fake.argsOf('start')[2]![0] as any).fields.backing).toMatchObject({
      startSection: null,
      loopSection: null,
    });
  });

  test('an armed multi-section loop cannot be stored as one backing section', async () => {
    const h = await armed();
    h.playback.arm({ loop: 'intro', through: 'verse' });
    expect(get(h.playback.store).transport.armed).toBe('intro');
    await h.c.record(SONG_ON);
    expect(h.fake.argsOf('start')).toEqual([]);
    expect(h.st().error?.message).toContain('temporary section range');
  });

  test('the song tempo comes from prepareSong (bpm rounded to 6 places, the override)', async () => {
    const h = await armed({
      prepare: { ok: true, cps: 0.7, meter: { num: 7, den: 8 }, tempoOverride: true },
    });
    h.backing.song = { T: 7, cps: 0.7 };
    await h.c.record(SONG_ON);
    expect((h.fake.argsOf('start')[0]![0] as any).fields.tempo).toEqual({
      bpm: 147,
      meter: '7/8',
      cps: 0.7,
      beatsPerBar: 3.5,
      tempoOverride: true,
    });
    expect(h.backing.grids).toEqual([{ T: 7, cps: 0.7, num: 7, from: -7, until: undefined }]);
  });

  test('record is ignored unless armed with a project', async () => {
    const h = setup({ project: null });
    await flush();
    await h.c.record(SONG_ON);
    expect(h.fake.names()).not.toContain('start');
    const a = await armed();
    await a.c.record(SONG_ON);
    a.log.length = 0;
    await a.c.record(SONG_ON); // count-in
    expect(a.log).toEqual(['resumeAudio']);
  });
});

// ---------------------------------------------------------------- failures

describe('failures while starting', () => {
  test.each([
    ['eval', 'boom is not defined'],
    ['meta', 'meta: bpm must be a number'],
  ] as const)(
    'a %s error with the song on: armed with the error, the mic never opens',
    async (kind, message) => {
      const h = await armed({ prepare: { ok: false, kind, message } });
      await h.c.record(SONG_ON);
      expect(h.log).toEqual(['resumeAudio', 'prepareSong']);
      expect(h.fake.names()).not.toContain('start');
      expect(h.st()).toMatchObject({ phase: 'armed', error: { kind, message }, stem: null });
    },
  );

  test('audio not running with a backing: an audio error before the mic opens', async () => {
    const h = await armed({ resume: false });
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect(h.log).toEqual(['resumeAudio']);
    expect(h.st()).toMatchObject({
      phase: 'armed',
      error: { kind: 'audio', message: MESSAGES.audioOff },
    });
  });

  test('start refused for permission: idle with the error and the permission re-read', async () => {
    const h = await armed();
    h.fake.status = { ...h.fake.status, permission: 'denied' };
    h.fake.failNext('start', new RecordingError('microphone access denied', 'permissionDenied'));
    await h.c.record(SONG_ON);
    await flush();
    expect(h.log).toEqual(['resumeAudio', 'prepareSong', 'api.start', 'api.status']);
    expect(h.st()).toMatchObject({
      phase: 'idle',
      permission: 'denied',
      error: { kind: 'permissionDenied', message: 'microphone access denied' },
    });
  });

  test.each([
    ['busy', 'a take is already running'],
    ['other', 'no anchor after 2 s'],
    ['exists', 'recording a-01 already exists'],
    ['projectChanged', 'the project changed'],
    ['noDevice', 'no input device found'],
  ] as const)(
    'start rejected (%s): armed with the error, nothing to undo',
    async (kind, message) => {
      const h = await armed();
      h.fake.failNext('start', new RecordingError(message, kind));
      await h.c.record(SONG_ON);
      expect(h.log).toEqual(['resumeAudio', 'prepareSong', 'api.start']);
      expect(h.st()).toMatchObject({ phase: 'armed', error: { kind, message }, stem: null });
    },
  );

  test('the song failing to start discards the take', async () => {
    const h = await armed();
    h.backing.song = null;
    await h.c.record(SONG_ON);
    expect(h.log).toEqual([
      'resumeAudio',
      'prepareSong',
      'api.start',
      'startSong {"countIn":true,"prepared":true}',
      'stopAll',
      'api.stop',
      'api.delete',
    ]);
    expect(h.fake.stops).toEqual([null]);
    expect(h.fake.argsOf('delete')).toEqual([['/p', 'a-01']]);
    expect(h.st()).toMatchObject({
      phase: 'armed',
      stem: null,
      takeStartedAtMs: null,
      error: { kind: 'audio', message: MESSAGES.songFailed },
    });
  });

  test('the metronome failing to start discards the take', async () => {
    const h = await armed();
    h.backing.metronomeOk = false;
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect(h.log).toEqual([
      'resumeAudio',
      'api.start',
      'startMetronome',
      'stopAll',
      'api.stop',
      'api.delete',
    ]);
    expect(h.st().error).toEqual({ kind: 'audio', message: MESSAGES.metronomeFailed });
  });

  test('no context clock with the song off discards the take', async () => {
    const h = await armed();
    h.setGraph(false);
    await h.c.record({ song: false, metronome: true, countIn: false });
    expect(h.log).toEqual(['resumeAudio', 'api.start', 'stopAll', 'api.stop', 'api.delete']);
    expect(h.st().error).toEqual({ kind: 'audio', message: MESSAGES.audioOff });
  });

  test('discard errors are logged, not shown', async () => {
    const h = await armed();
    h.backing.song = null;
    h.fake.failNext('stop', new RecordingError('x', 'other'));
    h.fake.failNext('delete', new RecordingError('y', 'notFound'));
    const orig = console.error;
    const logged: unknown[] = [];
    console.error = (e: unknown) => void logged.push(e);
    try {
      await h.c.record(SONG_ON);
    } finally {
      console.error = orig;
    }
    expect(logged.map((e) => (e as Error).message)).toEqual(['x', 'y']);
    expect(h.st().error).toEqual({ kind: 'audio', message: MESSAGES.songFailed });
  });

  test('a clock that cannot be measured warns; the take goes on without bar 1', async () => {
    const h = await armed();
    h.fake.hostNowNs = async () => {
      throw new Error('ipc');
    };
    const orig = console.error;
    console.error = () => {};
    try {
      await h.c.record(SONG_ON);
    } finally {
      console.error = orig;
    }
    expect(h.st()).toMatchObject({
      phase: 'count-in',
      warning: MESSAGES.clockWarning,
      error: null,
    });
    h.g.setTime(8);
    await h.c.stop();
    expect(h.lastStop()?.timing).toEqual({
      bar1HostNs: null,
      clockOffsetNs: { start: null, end: null },
      outputStamps: [],
    });
  });

  test('a saved clocked take with no input timing warns and keeps the memo', async () => {
    const h = await armed();
    const original = h.api.stop.bind(h.api);
    h.api.stop = async (args) => {
      const saved = await original(args);
      const fields = args.sidecar!;
      return {
        ...saved,
        sidecar: {
          schema: 1,
          createdAt: '',
          app: { version: '1' },
          input: {
            deviceId: 'mic',
            deviceName: 'mic',
            sampleRate: 48000,
            channel: 0,
            channels: 1,
          },
          ...fields,
          timing: { ...fields.timing, bar1Frame: null, anchors: [] },
          xruns: 0,
          recovered: false,
        },
      };
    };
    await h.c.record(SONG_ON);
    h.g.setTime(8);
    await h.c.stop();
    expect(h.st().warning).toBe(MESSAGES.captureWarning);
    expect(h.st().lastStem).toBe('a-01');
  });

  test('the output clock is retried every 50 ms', async () => {
    const h = await armed();
    h.g.setOutputTimestamp({ contextTime: 0, performanceTime: 0 });
    h.onSleep(() => {
      if (h.sleeps.length === 3) h.g.setOutputTimestamp(STAMP);
    });
    await h.c.record(SONG_ON);
    expect(h.sleeps).toEqual([OUTPUT_RETRY_MS, OUTPUT_RETRY_MS, OUTPUT_RETRY_MS]);
    expect(h.st().warning).toBeNull();
    h.g.setTime(8);
    await h.c.stop();
    expect(h.lastStop()?.timing.bar1HostNs).toBe(HOST(4000));
  });

  test('an output clock that never answers warns after 2 s; the take goes on', async () => {
    const h = await armed();
    h.g.setOutputTimestamp('missing');
    await h.c.record(SONG_ON);
    expect(OUTPUT_RETRIES * OUTPUT_RETRY_MS).toBe(2000);
    expect(h.sleeps.length).toBe(OUTPUT_RETRIES);
    expect(h.st()).toMatchObject({ phase: 'count-in', warning: MESSAGES.outputWarning });
    h.g.setTime(8);
    await h.c.stop();
    expect(h.lastStop()?.timing).toMatchObject({ bar1HostNs: null, outputStamps: [] });
  });
});

// ---------------------------------------------------------------- stop

describe('stop', () => {
  test('during the count-in: bar 1 never sounded, so it is null', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    expect(h.st().phase).toBe('count-in');
    h.g.setTime(6);
    const p = h.c.stop();
    expect(h.st().phase).toBe('stopping');
    await p;
    expect(h.lastStop()?.timing.bar1HostNs).toBeNull();
    expect(h.lastStop()?.backing.countIn).toBe(true);
    expect(h.st()).toMatchObject({ phase: 'armed', lastStem: 'a-01' });
  });

  test('ignored in idle, armed, starting and stopping; a second stop shares the first', async () => {
    const i = setup({ project: null });
    await flush();
    await i.c.stop();
    const h = await armed();
    await h.c.stop();
    expect(h.log).toEqual([]);
    const gate = deferred();
    h.fake.hold('start', gate.promise);
    const rec = h.c.record(SONG_ON);
    await flush();
    expect(h.st().phase).toBe('starting');
    await h.c.stop();
    expect(h.st().phase).toBe('starting');
    h.fake.hold('start', null);
    gate.resolve();
    await rec;
    const hold = deferred();
    h.fake.hold('stop', hold.promise);
    const a = h.c.stop();
    const b = h.c.stop();
    expect(a).toBe(b);
    hold.resolve();
    await a;
    expect(h.fake.argsOf('stop').length).toBe(1);
  });

  test('a rejected stop is armed with the error', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.fake.failNext('stop', new RecordingError('write failed', 'other'));
    await h.c.stop();
    expect(h.st()).toMatchObject({
      phase: 'armed',
      error: { kind: 'other', message: 'write failed' },
      stem: null,
      lastStem: null,
    });
  });

  test('the output latency is recorded when the context has one', async () => {
    const h = await armed();
    h.g.setLatency({ baseLatency: 0.01, outputLatency: 0.02 });
    await h.c.record({ song: false, metronome: false, countIn: false });
    await h.c.stop();
    expect(h.lastStop()?.output).toEqual({ baseLatency: 0.01, outputLatency: 0.02 });
  });
});

// ---------------------------------------------------------------- mid-take events

describe('during a take', () => {
  test('device lost: the stop tail writes the sidecar and it ends idle with the error', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.g.setTime(8);
    h.log.length = 0;
    h.fake.emitState({
      state: 'idle',
      stem: 'a-01',
      error: { kind: 'deviceLost', message: 'input device lost' },
    });
    expect(h.st()).toMatchObject({ phase: 'stopping', error: { kind: 'deviceLost' } });
    await flush();
    expect(h.log).toEqual(['stopAll', 'api.stop']);
    expect(h.lastStop()?.timing.bar1HostNs).toBe(HOST(4000));
    expect(h.st()).toMatchObject({
      phase: 'idle',
      error: { kind: 'deviceLost', message: 'input device lost' },
      lastStem: 'a-01',
    });
    // No re-arm while the error stands; clearing it re-arms.
    expect(h.log).not.toContain('api.arm');
    h.c.clearError();
    await flush();
    expect(h.st()).toMatchObject({ phase: 'armed', error: null });
  });

  test('device lost, then the stop rejects: the device-lost error is kept', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.fake.failNext('stop', new RecordingError('x', 'other'));
    h.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'lost' } });
    await flush();
    expect(h.st()).toMatchObject({ phase: 'idle', error: { kind: 'deviceLost', message: 'lost' } });
  });

  test('device lost while the memo is saved: it ends idle with the error, not re-armed', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    const gate = deferred();
    h.fake.hold('stop', gate.promise);
    const stopped = h.c.stop();
    await flush();
    expect(h.st().phase).toBe('stopping');
    h.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'lost' } });
    h.fake.hold('stop', null);
    gate.resolve();
    await stopped;
    await flush();
    expect(h.st()).toMatchObject({
      phase: 'idle',
      error: { kind: 'deviceLost', message: 'lost' },
      lastStem: 'a-01',
    });
    expect(h.log).not.toContain('api.arm');
  });

  test('the length cap: the stop tail, armed with limitReached', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: false, countIn: false });
    h.fake.emitState({
      state: 'armed',
      stem: 'a-01',
      error: { kind: 'limitReached', message: 'length limit' },
    });
    await flush();
    expect(h.fake.stops.length).toBe(1);
    expect(h.st()).toMatchObject({
      phase: 'armed',
      error: { kind: 'limitReached', message: 'length limit' },
      lastStem: 'a-01',
    });
  });

  test('an error event while starting is a start failure: discarded, idle after a device loss', async () => {
    const h = await armed();
    const gate = deferred();
    h.fake.hold('start', gate.promise);
    const rec = h.c.record(SONG_ON);
    await flush();
    h.fake.emitState({ state: 'idle', error: { kind: 'deviceLost', message: 'lost' } });
    expect(h.st().phase).toBe('starting');
    h.fake.hold('start', null);
    gate.resolve();
    await rec;
    expect(h.log).toEqual([
      'resumeAudio',
      'prepareSong',
      'api.start',
      'stopAll',
      'api.stop',
      'api.delete',
    ]);
    expect(h.fake.stops).toEqual([null]);
    expect(h.st()).toMatchObject({ phase: 'idle', error: { kind: 'deviceLost' }, stem: null });
  });

  test('the song ending on its own changes nothing: capture and metronome go on', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.g.setTime(7);
    h.fire(COUNT_IN_TICK_MS);
    h.log.length = 0;
    h.rt.setStarted(false);
    await flush();
    expect(h.log).toEqual([]);
    expect(h.st().phase).toBe('recording');
    await h.c.stop();
    expect(h.log).toEqual(['stopAll', 'api.stop']);
  });

  test('a tempo change warns once and keeps the start tempo', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.rt.patch({ cps: 0.5 + 1e-10 });
    expect(h.st().warning).toBeNull();
    h.rt.patch({ cps: 0.6 });
    h.rt.patch({ cps: 0.7 });
    expect(h.st().warning).toBe(MESSAGES.tempoWarning);
    await h.c.stop();
    expect(h.lastStop()?.tempo).toEqual(TEMPO_44);
    // The next take starts without it.
    h.rt.patch({ cps: 0.5 });
    await h.c.record(SONG_ON);
    expect(h.st().warning).toBeNull();
  });

  test('no tempo watch with the song off', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: true, countIn: false });
    h.rt.patch({ cps: 0.9 });
    expect(h.st().warning).toBeNull();
  });

  test('a project switch mid-take runs the stop tail (Rust keeps the old project)', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.log.length = 0;
    h.setRoot('/q');
    expect(h.st().phase).toBe('stopping');
    await flush();
    expect(h.log).toEqual(['stopAll', 'api.stop']);
    expect(h.fake.argsOf('stop')).toEqual([[{ sidecar: h.lastStop() }]]);
    expect(h.st()).toMatchObject({ phase: 'armed', lastStem: 'a-01', error: null });
    expect((await h.fake.memos.get('/p'))!.map((m) => m.stem)).toEqual(['a-01']);
  });

  test('a project switch while starting aborts: discarded at the next step, no error', async () => {
    const h = await armed();
    const gate = deferred();
    h.fake.hold('start', gate.promise);
    const rec = h.c.record(SONG_ON);
    await flush();
    h.setRoot('/q');
    h.fake.hold('start', null);
    gate.resolve();
    await rec;
    expect(h.log).toEqual([
      'resumeAudio',
      'prepareSong',
      'api.start',
      'stopAll',
      'api.stop',
      'api.delete',
    ]);
    expect(h.fake.argsOf('delete')).toEqual([['/p', 'a-01']]);
    expect(h.st()).toMatchObject({ phase: 'armed', error: null, stem: null });
  });

  test('a project switch before the capture opened: nothing to undo', async () => {
    const h = await armed();
    const gate = deferred();
    h.backing.holdPrepare = gate.promise;
    const rec = h.c.record(SONG_ON);
    await flush();
    h.setRoot('/q');
    gate.resolve();
    await rec;
    expect(h.log).toEqual(['resumeAudio', 'prepareSong']);
    expect(h.st()).toMatchObject({ phase: 'armed', error: null });
  });

  test('a project switch while the song starts aborts after it', async () => {
    const h = await armed();
    const gate = deferred();
    h.backing.holdSong = gate.promise;
    const rec = h.c.record(SONG_ON);
    await flush();
    h.setRoot('/q');
    gate.resolve();
    await rec;
    expect(h.log.slice(-4)).toEqual([
      'startSong {"countIn":true,"prepared":true}',
      'stopAll',
      'api.stop',
      'api.delete',
    ]);
    expect(h.st().phase).toBe('armed');
  });
});

// ---------------------------------------------------------------- inputs and errors

describe('selectInput, refreshDevices, clearError', () => {
  test('selectInput re-arms with the choice', async () => {
    const h = await armed();
    h.fake.devices = { devices: [MIC, INTERFACE], selected: { deviceId: 'mic-1', channel: 0 } };
    await h.c.selectInput('usb-2', 1);
    expect(h.fake.argsOf('arm').at(-1)).toEqual([{ deviceId: 'usb-2', channel: 1 }]);
    expect(h.st()).toMatchObject({ phase: 'armed', input: { deviceId: 'usb-2', channel: 1 } });
  });

  test('selectInput from idle clears the error and arms', async () => {
    const h = setup();
    h.fake.failNext('arm', new RecordingError('no input device found', 'noDevice'));
    await flush();
    expect(h.st().phase).toBe('idle');
    await h.c.selectInput('mic-1', 0);
    expect(h.st()).toMatchObject({ phase: 'armed', error: null });
  });

  test('selectInput and refreshDevices are ignored during a take', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.log.length = 0;
    await h.c.selectInput('usb-2', 1);
    await h.c.refreshDevices();
    expect(h.log).toEqual([]);
  });

  test('refreshDevices while armed updates the list only', async () => {
    const h = await armed();
    h.fake.devices = { devices: [MIC, INTERFACE], selected: { deviceId: 'usb-2', channel: 1 } };
    await h.c.refreshDevices();
    expect(h.log).toEqual(['api.devices']);
    expect(h.st()).toMatchObject({
      phase: 'armed',
      devices: [MIC, INTERFACE],
      input: { deviceId: 'usb-2', channel: 1 },
    });
  });

  test('refreshDevices from idle re-arms (a re-plugged mic recovers)', async () => {
    const h = setup();
    h.fake.devices = { devices: [], selected: null };
    await flush();
    expect(h.st()).toMatchObject({ phase: 'idle', error: { kind: 'noDevice' }, devices: [] });
    h.fake.devices = { devices: [MIC], selected: { deviceId: 'mic-1', channel: 0 } };
    await h.c.refreshDevices();
    expect(h.st()).toMatchObject({
      phase: 'armed',
      error: null,
      devices: [MIC],
      input: { deviceId: 'mic-1', channel: 0 },
    });
  });

  test('refreshDevices from idle without a project does not arm', async () => {
    const h = setup({ project: null });
    await flush();
    await h.c.refreshDevices();
    expect(h.fake.names()).toEqual(['status', 'devices', 'devices']);
  });

  test('clearError clears it; from idle with a project it re-arms', async () => {
    const h = await armed();
    await h.c.record({ song: false, metronome: true, countIn: false });
    await h.c.stop();
    h.fake.failNext('start', new RecordingError('busy', 'busy'));
    await h.c.record(SONG_ON);
    expect(h.st().error?.kind).toBe('busy');
    h.c.clearError();
    expect(h.st()).toMatchObject({ phase: 'armed', error: null });
  });
});

// ---------------------------------------------------------------- destroy

describe('destroy', () => {
  test('unlistens, unsubscribes and disarms', async () => {
    const h = await armed();
    h.c.destroy();
    await flush();
    expect(h.fake.listeners()).toEqual({ state: 0, level: 0 });
    expect(h.log).toEqual(['api.disarm']);
    h.setRoot('/q');
    h.fake.emitLevel({ peak: 1, rms: 1 });
    await flush();
    expect(h.log).toEqual(['api.disarm']);
  });

  test('during a take: the take is saved, then disarmed', async () => {
    const h = await armed();
    await h.c.record(SONG_ON);
    h.log.length = 0;
    h.c.destroy();
    await flush();
    expect(h.log).toEqual(['stopAll', 'api.stop', 'api.disarm']);
  });

  test('before setup finished: nothing arms', async () => {
    const h = setup();
    h.c.destroy();
    await flush();
    expect(h.fake.names()).not.toContain('arm');
    expect(h.fake.listeners()).toEqual({ state: 0, level: 0 });
  });
});

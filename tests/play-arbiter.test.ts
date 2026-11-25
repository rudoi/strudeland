import { describe, expect, test } from 'bun:test';
import { derived, get, writable } from 'svelte/store';
import type { MemoPlayerState } from '../src/lib/audio/memo-player';
import { createOutputClock } from '../src/lib/audio/output-clock';
import {
  createPlayArbiter,
  type PlayArbiter,
  type PlayArbiterState,
} from '../src/lib/play-arbiter';
import type { SessionEvalOutcome } from '../src/lib/project/session';
import { createSongPlayback, type ArmChoice } from '../src/lib/project/song-playback';
import type { BackingStarter } from '../src/lib/recording/backing';
import { createRecordingController } from '../src/lib/recording/controller';
import type { BackingChoice, RecordingPhase } from '../src/lib/recording/types';
import { buildStructure } from '../src/lib/strudel/structure';
import { fakeAudioGraph } from './helpers/fake-audio-graph';
import { deferred, flush } from './helpers/fake-project-api';
import { fakeRecordingApi } from './helpers/fake-recording-api';
import { stubRuntime } from './helpers/stub-runtime';

const MAIN = 'songs/main.js';
const OTHER = 'songs/other.js';
const VERSE: ArmChoice = { start: 'verse' };
const SONG: BackingChoice = { song: true, metronome: false, countIn: false };
const NONE: BackingChoice = { song: false, metronome: false, countIn: false };

// ---------------------------------------------------------------- the world

/**
 * Every state the arbiter tells apart. A take's backing is told by the runtime: a take with song
 * backing has it started, a metronome-only take does not.
 */
const STATES = {
  idle: { started: false },
  song: { started: true },
  'memo alone': { memo: 'playing', started: false },
  'memo loading with song': { memo: 'loading', started: false },
  'memo with song': { memo: 'playing', started: true },
  'take starting': { phase: 'starting', started: false },
  'take count-in with song': { phase: 'count-in', started: true },
  'take with song': { phase: 'recording', started: true },
  'take with metronome': { phase: 'recording', started: false },
  'take stopping': { phase: 'stopping', started: true },
  audition: { started: false, audition: true },
  'in-flight evaluate': { started: false, inFlight: true },
} satisfies Record<
  string,
  {
    phase?: RecordingPhase;
    memo?: 'loading' | 'playing';
    started: boolean;
    audition?: boolean;
    inFlight?: boolean;
  }
>;
type StateName = keyof typeof STATES;

const OUTCOME: SessionEvalOutcome = {
  ok: true,
  song: MAIN,
  title: 'main',
  started: false,
  audio: 'running',
  error: null,
  errorKind: null,
};

/**
 * The arbiter over small fakes that log every call in one list. The runtime is the stub; its
 * `stop` fires `stopAuditions` as the runtime's `onPlaybackRequest` does, and bumps a stop count
 * that a pending `saveAndEvaluate` reads before it autostarts, as the runtime's own counter
 * cancels an in-flight autostart.
 */
function world(name: StateName = 'idle', opts: { armOk?: boolean; wired?: boolean } = {}) {
  const log: string[] = [];
  const rt = stubRuntime({ live: true, audio: 'running' });
  let stopGen = 0;
  let audition = false;
  const realStop = rt.runtime.stop;
  const stopAuditions = () => {
    log.push('stopAuditions');
    audition = false;
  };
  rt.runtime.stop = () => {
    log.push('runtime.stop');
    stopGen++;
    realStop();
    stopAuditions();
  };
  const recStore = writable<{ phase: RecordingPhase }>({ phase: 'armed' });
  const recording = {
    store: { subscribe: recStore.subscribe },
    record(choice: BackingChoice) {
      log.push(`recording.record ${JSON.stringify(choice)}`);
      return Promise.resolve();
    },
    stop() {
      log.push('recording.stop');
      return Promise.resolve();
    },
  };
  const player = writable<MemoPlayerState>({ status: 'idle' });
  const memos = {
    player: { subscribe: player.subscribe },
    stopPlayback() {
      log.push('memos.stopPlayback');
      player.set({ status: 'idle' });
    },
  };
  let hold: Promise<void> = Promise.resolve();
  const session = {
    playback: {
      store: derived(rt.runtime.store, (s) => ({ started: s.started })),
      arm(choice: ArmChoice) {
        log.push(`arm ${JSON.stringify(choice)}`);
        return opts.armOk ?? true;
      },
    },
    async saveAndEvaluate() {
      log.push('saveAndEvaluate');
      const gen = stopGen;
      await hold;
      if (gen === stopGen) rt.setStarted(true);
    },
    evaluateBuffer(path: string, o?: { autostart?: boolean }) {
      log.push(`evaluateBuffer ${path} ${JSON.stringify(o)}`);
      return Promise.resolve({ ...OUTCOME, song: path });
    },
  };
  const wired = opts.wired ?? true;
  const arbiter = createPlayArbiter({
    session,
    runtime: rt.runtime,
    stopAuditions,
    ...(wired ? { recording, memos } : {}),
  });
  const s = STATES[name] as (typeof STATES)[StateName] & {
    phase?: RecordingPhase;
    memo?: 'loading' | 'playing';
    audition?: boolean;
    inFlight?: boolean;
  };
  if (s.started) rt.setStarted(true);
  if (s.phase) recStore.set({ phase: s.phase });
  if (s.memo) player.set({ status: s.memo, stem: 'take-01', how: s.started ? 'song' : 'alone' });
  let release: (() => void) | null = null;
  if (s.inFlight) {
    const d = deferred();
    hold = d.promise;
    release = d.resolve;
    arbiter.play();
  }
  if (s.audition) audition = true;
  log.length = 0;
  return {
    arbiter,
    log,
    rt,
    recStore,
    player,
    session,
    memos,
    recording,
    started: () => get(rt.runtime.store).started,
    audition: () => audition,
    setAudition: (on: boolean) => void (audition = on),
    /** Let the in-flight evaluate resolve. */
    release: async () => {
      release?.();
      await flush();
    },
  };
}

const PLAY = ['stopAuditions', 'memos.stopPlayback', 'saveAndEvaluate'];
const STOP_SONG = ['memos.stopPlayback', 'runtime.stop', 'stopAuditions'];

// ---------------------------------------------------------------- the table

type Row = {
  state: StateName;
  why: string;
  /** What the call returns (`stop` without `done`). */
  returns?: unknown;
  log: string[];
  /** Checked after the call (and a flush). */
  after?: (w: ReturnType<typeof world>) => void;
};

const run: Record<string, (a: PlayArbiter) => unknown> = {
  play: (a) => a.play(),
  stop: (a) => {
    const { done: _done, ...r } = a.stop();
    return r;
  },
  evaluate: (a) => a.evaluate(MAIN),
  playFrom: (a) => a.playFrom(VERSE),
  record: (a) => a.record(SONG),
};

const TABLE: Record<keyof typeof run, Row[]> = {
  play: [
    { state: 'idle', why: 'save if dirty, evaluate, autostart', returns: true, log: PLAY },
    { state: 'song', why: 'save, re-evaluate', returns: true, log: PLAY },
    {
      state: 'memo alone',
      why: 'stop the memo, save, evaluate, start',
      returns: true,
      log: PLAY,
      after: (w) => expect(get(w.arbiter.store).memoAudible).toBe(false),
    },
    {
      state: 'memo with song',
      why: 'stop the memo (its song), save, evaluate, start',
      returns: true,
      log: PLAY,
    },
    { state: 'take with song', why: 'refused', returns: false, log: [] },
    { state: 'take with metronome', why: 'refused', returns: false, log: [] },
    { state: 'take starting', why: 'refused', returns: false, log: [] },
    { state: 'take count-in with song', why: 'refused', returns: false, log: [] },
    { state: 'take stopping', why: 'refused', returns: false, log: [] },
    {
      state: 'audition',
      why: 'silence the audition first',
      returns: true,
      log: PLAY,
      after: (w) => expect(w.audition()).toBe(false),
    },
    { state: 'in-flight evaluate', why: 'a second request', returns: true, log: PLAY },
  ],
  stop: [
    {
      state: 'idle',
      why: 'stop the runtime anyway',
      returns: { stopped: null, wasPlaying: false },
      log: STOP_SONG,
    },
    {
      state: 'song',
      why: 'stop the song',
      returns: { stopped: 'song', wasPlaying: true },
      log: STOP_SONG,
      after: (w) => expect(w.started()).toBe(false),
    },
    {
      // Kept: `strudeland stop` reports not playing after it stopped a memo.
      state: 'memo alone',
      why: 'stop the memo; wasPlaying stays false (kept quirk)',
      returns: { stopped: 'memo', wasPlaying: false },
      log: STOP_SONG,
      after: (w) => expect(get(w.player).status).toBe('idle'),
    },
    {
      state: 'memo with song',
      why: 'stop the memo, then the song',
      returns: { stopped: 'memo', wasPlaying: true },
      log: STOP_SONG,
      after: (w) => {
        expect(get(w.player).status).toBe('idle');
        expect(w.started()).toBe(false);
      },
    },
    {
      state: 'take with song',
      why: 'end and save the take; the song is the take’s to stop',
      returns: { stopped: 'take', wasPlaying: true },
      log: ['recording.stop'],
      after: (w) => expect(w.started()).toBe(true),
    },
    {
      state: 'take with metronome',
      why: 'end and save the take',
      returns: { stopped: 'take', wasPlaying: false },
      log: ['recording.stop'],
    },
    {
      state: 'take starting',
      why: 'the take’s stop (a no-op while starting)',
      returns: { stopped: 'take', wasPlaying: false },
      log: ['recording.stop'],
    },
    {
      state: 'take count-in with song',
      why: 'end the take',
      returns: { stopped: 'take', wasPlaying: true },
      log: ['recording.stop'],
    },
    {
      state: 'take stopping',
      why: 'the take’s stop (a no-op while stopping)',
      returns: { stopped: 'take', wasPlaying: true },
      log: ['recording.stop'],
    },
    {
      state: 'audition',
      why: 'stop while idle silences the audition',
      returns: { stopped: null, wasPlaying: false },
      log: STOP_SONG,
      after: (w) => expect(w.audition()).toBe(false),
    },
    {
      state: 'in-flight evaluate',
      why: 'stop cancels the evaluate’s autostart',
      returns: { stopped: null, wasPlaying: false },
      log: STOP_SONG,
    },
  ],
  evaluate: [
    {
      state: 'idle',
      why: 'no save, evaluate, autostart',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} undefined`],
    },
    {
      state: 'song',
      why: 'as idle',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} undefined`],
    },
    {
      state: 'memo alone',
      why: 'stop the memo, evaluate, start',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} undefined`],
    },
    {
      state: 'memo with song',
      why: 'stop the memo, evaluate, start',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} undefined`],
    },
    {
      state: 'take with song',
      why: 'evaluate without autostart',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} {"autostart":false}`],
    },
    {
      state: 'take with metronome',
      why: 'evaluate without autostart',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} {"autostart":false}`],
    },
    {
      state: 'take starting',
      why: 'evaluate without autostart',
      log: ['memos.stopPlayback', `evaluateBuffer ${MAIN} {"autostart":false}`],
    },
  ],
  playFrom: [
    {
      state: 'idle',
      why: 'arm, then evaluate',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`, ...PLAY],
    },
    {
      state: 'song',
      why: 'arm only: the jump is at the next bar',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`],
    },
    {
      state: 'memo alone',
      why: 'arm, then play stops the memo',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`, ...PLAY],
    },
    {
      state: 'memo loading with song',
      why: 'the song has not started yet: arm, then play stops the memo',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`, ...PLAY],
    },
    {
      state: 'memo with song',
      why: 'refused: the memo’s backing does not jump',
      returns: false,
      log: [],
    },
    {
      state: 'take with song',
      why: 'refused: the take’s backing does not jump',
      returns: false,
      log: [],
    },
    {
      state: 'take count-in with song',
      why: 'refused: the backing is playing',
      returns: false,
      log: [],
    },
    {
      state: 'take stopping',
      why: 'refused: the backing still plays',
      returns: false,
      log: [],
    },
    {
      state: 'take starting',
      why: 'arm; play is refused',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`],
    },
    {
      state: 'take with metronome',
      why: 'arm; play is refused',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`],
    },
    {
      state: 'audition',
      why: 'arm, then play silences the audition',
      returns: true,
      log: [`arm ${JSON.stringify(VERSE)}`, ...PLAY],
    },
  ],
  record: [
    {
      state: 'idle',
      why: 'start a take',
      log: ['memos.stopPlayback', `recording.record ${JSON.stringify(SONG)}`],
    },
    {
      state: 'song',
      why: 'start a take (the take stops the song)',
      log: ['memos.stopPlayback', `recording.record ${JSON.stringify(SONG)}`],
    },
    {
      state: 'memo alone',
      why: 'stop the memo first',
      log: ['memos.stopPlayback', `recording.record ${JSON.stringify(SONG)}`],
      after: (w) => expect(get(w.player).status).toBe('idle'),
    },
    {
      state: 'memo with song',
      why: 'stop the memo (and its song) first',
      log: ['memos.stopPlayback', `recording.record ${JSON.stringify(SONG)}`],
    },
  ],
};

describe('the play table', () => {
  for (const [action, rows] of Object.entries(TABLE)) {
    describe(action, () => {
      for (const row of rows) {
        test(`${row.state}: ${row.why}`, async () => {
          const w = world(row.state);
          const result = run[action](w.arbiter);
          // Synchronous: everything is called before the first await.
          expect(w.log).toEqual(row.log);
          if (row.returns !== undefined) expect(result).toEqual(row.returns);
          await w.release();
          row.after?.(w);
        });
      }
    });
  }
});

// ---------------------------------------------------------------- the store

describe('the store', () => {
  const view = (s: PlayArbiterState) => ({
    taking: s.taking,
    memoAudible: s.memoAudible,
    started: s.started,
    stopTarget: s.stopTarget,
    canPlayFrom: s.canPlayFrom,
  });
  const T = true;
  const F = false;
  const rows: [StateName, ReturnType<typeof view>][] = [
    [
      'idle',
      {
        taking: F,
        memoAudible: F,
        started: F,
        stopTarget: null,
        canPlayFrom: T,
      },
    ],
    [
      'song',
      {
        taking: F,
        memoAudible: F,
        started: T,
        stopTarget: 'song',
        canPlayFrom: T,
      },
    ],
    // The runtime is stopped: a click arms, and play stops the memo.
    [
      'memo alone',
      {
        taking: F,
        memoAudible: T,
        started: F,
        stopTarget: 'memo',
        canPlayFrom: T,
      },
    ],
    // Not started yet: enabled until the song starts.
    [
      'memo loading with song',
      {
        taking: F,
        memoAudible: T,
        started: F,
        stopTarget: 'memo',
        canPlayFrom: T,
      },
    ],
    [
      'memo with song',
      {
        taking: F,
        memoAudible: T,
        started: T,
        stopTarget: 'memo',
        canPlayFrom: F,
      },
    ],
    [
      'take starting',
      {
        taking: T,
        memoAudible: F,
        started: F,
        stopTarget: 'take',
        canPlayFrom: T,
      },
    ],
    [
      'take count-in with song',
      {
        taking: T,
        memoAudible: F,
        started: T,
        stopTarget: 'take',
        canPlayFrom: F,
      },
    ],
    [
      'take with song',
      {
        taking: T,
        memoAudible: F,
        started: T,
        stopTarget: 'take',
        canPlayFrom: F,
      },
    ],
    [
      'take with metronome',
      {
        taking: T,
        memoAudible: F,
        started: F,
        stopTarget: 'take',
        canPlayFrom: T,
      },
    ],
    [
      'take stopping',
      {
        taking: T,
        memoAudible: F,
        started: T,
        stopTarget: 'take',
        canPlayFrom: F,
      },
    ],
  ];
  for (const [state, expected] of rows) {
    test(state, () => {
      const w = world(state);
      expect(view(get(w.arbiter.store))).toEqual(expected);
    });
  }

  test('takePhase follows the controller, and is null without one', () => {
    expect(get(world('take with song').arbiter.store).takePhase).toBe('recording');
    expect(get(world('idle').arbiter.store).takePhase).toBe('armed');
    expect(get(world('idle', { wired: false }).arbiter.store).takePhase).toBeNull();
  });

  test('a phase outside a take (idle, arming, armed) is not taking', () => {
    const w = world();
    for (const phase of ['idle', 'arming', 'armed'] as const) {
      w.recStore.set({ phase });
      expect(get(w.arbiter.store).taking).toBe(false);
    }
  });

  test('a memo error is not audible', () => {
    const w = world();
    w.player.set({ status: 'error', stem: 'take-01', kind: 'read', message: 'gone' } as any);
    expect(get(w.arbiter.store).memoAudible).toBe(false);
  });
});

// ---------------------------------------------------------------- auditions

describe('auditionsAvailable', () => {
  const rows: [StateName, boolean][] = [
    ['idle', true],
    ['audition', true],
    ['song', false],
    ['memo alone', false],
    ['memo loading with song', false],
    ['take starting', false],
    ['take with metronome', false],
    ['in-flight evaluate', false],
  ];
  for (const [state, available] of rows) {
    test(`${state}: ${available}`, () => {
      expect(world(state).arbiter.auditionsAvailable()).toBe(available);
    });
  }

  test('a pending play keeps them off until it settles, even after a stop', async () => {
    const w = world('in-flight evaluate');
    w.arbiter.stop();
    expect(w.arbiter.auditionsAvailable()).toBe(false);
    await w.release();
    expect(w.started()).toBe(false);
    expect(w.arbiter.auditionsAvailable()).toBe(true);
  });

  test('entering a take or an audible memo silences them', () => {
    const w = world();
    w.recStore.set({ phase: 'starting' });
    expect(w.log).toEqual(['stopAuditions']);
    w.recStore.set({ phase: 'recording' });
    expect(w.log).toEqual(['stopAuditions']);
    w.recStore.set({ phase: 'armed' });
    w.log.length = 0;
    w.player.set({ status: 'loading', stem: 'take-01', how: 'alone' });
    expect(w.log).toEqual(['stopAuditions']);
    w.log.length = 0;
    w.player.set({ status: 'idle' });
    w.recStore.set({ phase: 'armed' });
    expect(w.log).toEqual([]);
  });
});

// ---------------------------------------------------------------- the verbs' edges

describe('the verbs', () => {
  test('stop cancels an in-flight autostart: the song never starts', async () => {
    const w = world('in-flight evaluate');
    expect(w.arbiter.stop().wasPlaying).toBe(false);
    expect(w.rt.stops()).toBe(1);
    await w.release();
    expect(w.started()).toBe(false);
  });

  test('without a stop, the in-flight evaluate autostarts', async () => {
    const w = world('in-flight evaluate');
    await w.release();
    expect(w.started()).toBe(true);
  });

  test('stop during a take: done is the take’s stop', async () => {
    const w = world('take with song');
    const saved = deferred();
    w.recording.stop = () => (w.log.push('recording.stop'), saved.promise);
    const { done } = w.arbiter.stop();
    let settled = false;
    void done.then(() => (settled = true));
    await flush();
    expect(settled).toBe(false);
    saved.resolve();
    await flush();
    expect(settled).toBe(true);
  });

  test('stop outside a take: done is resolved', async () => {
    const w = world('song');
    await w.arbiter.stop().done;
  });

  test('a throwing runtime stop propagates', () => {
    const w = world('song');
    w.rt.runtime.stop = () => {
      throw new Error('canvas gone');
    };
    expect(() => w.arbiter.stop()).toThrow('canvas gone');
  });

  test('play returns true with nothing to evaluate (saveAndEvaluate does nothing then)', () => {
    const w = world();
    w.session.saveAndEvaluate = async () => void w.log.push('saveAndEvaluate (no song)');
    expect(w.arbiter.play()).toBe(true);
    expect(w.log).toEqual(['stopAuditions', 'memos.stopPlayback', 'saveAndEvaluate (no song)']);
  });

  test('playFrom with a choice that does not resolve arms nothing and does not play', () => {
    const w = world('idle', { armOk: false });
    expect(w.arbiter.playFrom(VERSE)).toBe(false);
    expect(w.log).toEqual([`arm ${JSON.stringify(VERSE)}`]);
  });

  test('playFrom arms again once the backing stops', () => {
    const w = world('take with song');
    expect(w.arbiter.playFrom(VERSE)).toBe(false);
    w.rt.setStarted(false);
    expect(w.arbiter.playFrom(VERSE)).toBe(true);
    expect(w.log).toEqual([`arm ${JSON.stringify(VERSE)}`]);
  });

  test('playFrom reads started before it arms', () => {
    const w = world('song');
    // An arm that stops the runtime (as a section the song lacks would) does not then play.
    w.session.playback.arm = (choice) => {
      w.log.push(`arm ${JSON.stringify(choice)}`);
      w.rt.setStarted(false);
      return true;
    };
    w.arbiter.playFrom(VERSE);
    expect(w.log).toEqual([`arm ${JSON.stringify(VERSE)}`]);
  });

  // Kept: another song's evaluate during a take takes over the backing.
  test('evaluate of another song during a take takes over the backing (kept quirk)', async () => {
    const w = world('take with song');
    expect(await w.arbiter.evaluate(OTHER)).toMatchObject({ song: OTHER });
    expect(w.log).toEqual(['memos.stopPlayback', `evaluateBuffer ${OTHER} {"autostart":false}`]);
  });

  test('every dependency is called through its object at call time', () => {
    const w = world('memo alone');
    const calls: string[] = [];
    w.memos.stopPlayback = () => void calls.push('patched stopPlayback');
    w.session.saveAndEvaluate = async () => void calls.push('patched saveAndEvaluate');
    w.session.playback.arm = () => (calls.push('patched arm'), true);
    w.recording.record = async () => void calls.push('patched record');
    w.rt.runtime.stop = () => void calls.push('patched runtime.stop');
    w.arbiter.playFrom(VERSE);
    w.arbiter.stop();
    void w.arbiter.record(NONE);
    expect(calls).toEqual([
      'patched arm',
      'patched stopPlayback',
      'patched saveAndEvaluate',
      'patched stopPlayback',
      'patched runtime.stop',
      'patched stopPlayback',
      'patched record',
    ]);
  });

  test('without recording or memos: stop stops the runtime, evaluate autostarts, record does nothing', async () => {
    const w = world('song', { wired: false });
    expect(w.arbiter.stop()).toMatchObject({ stopped: 'song', wasPlaying: true });
    expect(w.log).toEqual(['runtime.stop', 'stopAuditions']);
    w.log.length = 0;
    await w.arbiter.evaluate(MAIN);
    expect(w.log).toEqual([`evaluateBuffer ${MAIN} undefined`]);
    w.log.length = 0;
    await w.arbiter.record(SONG);
    expect(w.log).toEqual([]);
  });

  test('destroy releases the subscriptions', () => {
    const w = world();
    w.arbiter.destroy();
    w.arbiter.destroy();
    w.recStore.set({ phase: 'recording' });
    w.player.set({ status: 'playing', stem: 'take-01', how: 'alone' });
    w.rt.setStarted(true);
    expect(w.log).toEqual([]);
    expect(get(w.arbiter.store)).toMatchObject({
      taking: false,
      memoAudible: false,
      started: false,
    });
  });
});

// ---------------------------------------------------------------- the real controller

describe('with the recording controller', () => {
  /** The arbiter over a real controller on the fake recording API, with no backing. */
  async function withController() {
    const { api, fake } = fakeRecordingApi({ roots: ['/p'] });
    let ms = 1000;
    fake.hostNowNs = async () => 7_000_000_000_000n + BigInt(ms * 1e6);
    const rt = stubRuntime({ live: true, audio: 'running' });
    const sessionStore = writable({
      project: { root: '/p' },
      active: MAIN as string | null,
      structure: buildStructure({ bpm: 120, sections: [{ name: 'verse', bars: 2 }] }),
    });
    const playback = createSongPlayback({
      runtime: rt.runtime,
      getHeader: () => get(sessionStore).structure,
      currentGen: () => 0,
    });
    const starter: BackingStarter = {
      prepareSong: async () => ({
        ok: true,
        cps: 0.5,
        meter: { num: 4, den: 4 },
        tempoOverride: false,
      }),
      startSong: async () => null,
      startMetronome: () => true,
      stopAll: () => {},
    };
    const g = fakeAudioGraph({
      currentTime: 5,
      outputTimestamp: { contextTime: 5, performanceTime: 2000 },
    });
    const recording = createRecordingController({
      api,
      runtime: rt.runtime,
      playback,
      session: { store: { subscribe: sessionStore.subscribe } },
      backing: starter,
      outputClock: createOutputClock({ graph: () => g.graph }),
      nowMs: () => ms++,
      setInterval: () => 0,
      clearInterval: () => {},
      sleep: async () => {},
    });
    const arbiter = createPlayArbiter({
      session: {
        playback,
        saveAndEvaluate: async () => {},
        evaluateBuffer: async () => OUTCOME,
      },
      runtime: rt.runtime,
      recording,
      stopAuditions: () => {},
    });
    await flush();
    expect(get(recording.store).phase).toBe('armed');
    return { arbiter, recording, fake };
  }

  test('a take runs, refuses play and a second record, and stop saves it', async () => {
    const { arbiter, recording, fake } = await withController();
    await arbiter.record(NONE);
    await flush();
    expect(get(recording.store).phase).toBe('recording');
    expect(get(arbiter.store).stopTarget).toBe('take');
    expect(arbiter.play()).toBe(false);
    await arbiter.record(NONE);
    expect(fake.names().filter((n) => n === 'start')).toHaveLength(1);
    const { stopped, wasPlaying, done } = arbiter.stop();
    expect({ stopped, wasPlaying }).toEqual({ stopped: 'take', wasPlaying: false });
    await done;
    expect(fake.stops).toHaveLength(1);
    await flush();
    expect(get(arbiter.store).taking).toBe(false);
  });

  test('stop while the take is starting does nothing', async () => {
    const { arbiter, recording, fake } = await withController();
    const held = deferred();
    fake.hold('start', held.promise);
    const take = arbiter.record(NONE);
    await flush();
    expect(get(recording.store).phase).toBe('starting');
    const r = arbiter.stop();
    expect(r.stopped).toBe('take');
    await r.done;
    expect(fake.stops).toHaveLength(0);
    expect(get(recording.store).phase).toBe('starting');
    fake.hold('start', null);
    held.resolve();
    await take;
    await flush();
    await arbiter.stop().done;
    expect(fake.stops).toHaveLength(1);
  });
});

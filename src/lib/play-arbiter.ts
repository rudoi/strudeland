/**
 * The play arbiter: what play, stop, evaluate, section arming and record mean while a song, an
 * audible memo, a take or an audition is sounding. Its store is derived from the recording store,
 * the memo player and `runtime.store`; its verbs are the user's and the control socket's.
 *
 * A take owns play and stop: play is refused, stop ends (and saves) the take, and a control
 * evaluate never starts the song. An audible memo stops before anything else plays or records.
 * Auditions give way to all of it.
 *
 * Every dependency is called through its object at call time, so a test can replace a method.
 * `play`, `playFrom`, `stop` and `record` call through synchronously: nothing awaits before
 * `runtime.evaluate`, `runtime.start` or `resumeAudio`, which stay inside the user gesture.
 */
import { writable, type Readable } from 'svelte/store';
import type { Session, SessionEvalOutcome } from './project/session';
import type { ArmChoice, SongPlayback } from './project/song-playback';
import type { RecordingController } from './recording/controller';
import { memoAudible, takeRunning } from './recording/memo-ui';
import type { Memos } from './recording/memos';
import type { BackingChoice, RecordingPhase, RecordingUiState } from './recording/types';
import type { Runtime } from './strudel/runtime';

/** What a stop stops: the take, else the audible memo, else the song. */
export type StopTarget = 'take' | 'memo' | 'song';

export interface PlayArbiterState {
  /** A take runs: starting, count-in, recording or stopping. */
  taking: boolean;
  /** The recording controller's phase; null without one. */
  takePhase: RecordingPhase | null;
  /** A memo is loading or playing. */
  memoAudible: boolean;
  /** The runtime is started. */
  started: boolean;
  /** What a stop would stop now; null when nothing sounds. */
  stopTarget: StopTarget | null;
  /**
   * Section items may arm: not while a take's or a memo's song backing plays, nor while a take is
   * starting (the controller has read what is armed for the sidecar).
   */
  canPlayFrom: boolean;
}

export interface StopResult {
  stopped: StopTarget | null;
  /** `runtime.store.started` on entry, before anything stopped. */
  wasPlaying: boolean;
  /** Settles once the stop is done: the take's save, else at once. */
  done: Promise<void>;
}

export interface PlayArbiter {
  readonly store: Readable<PlayArbiterState>;
  /** No play request is pending, the runtime can audition, and no take or audible memo. */
  auditionsAvailable(): boolean;
  /**
   * The user's play (toolbar, Cmd-Enter): stop auditions and a memo, then save and evaluate the
   * active song. False, doing nothing, during a take.
   */
  play(): boolean;
  /**
   * The Timeline: arm a section and, while the song is stopped, play. True iff the choice armed;
   * false, doing nothing, while a take's or a memo's song backing plays or a take is starting
   * (`!canPlayFrom`).
   */
  playFrom(choice: ArmChoice): boolean;
  /**
   * The toolbar's and the control socket's stop: during a take end it (nothing while it is
   * starting or stopping: `stopped` is then null, `done` the save in progress), otherwise stop the memo and then the runtime, even when stopped (that
   * cancels an evaluate's autostart and silences auditions). Throws propagate.
   */
  stop(): StopResult;
  /**
   * The control socket's evaluate: stop a memo, then evaluate `path`'s buffer without saving;
   * during a take the song is not started (a playing backing takes the new code). A pending play
   * until it settles, as `play`'s.
   */
  evaluate(path: string): Promise<SessionEvalOutcome>;
  /** Start a take: stop a memo first. Call from the click: `resumeAudio` runs inside it. */
  record(choice: BackingChoice): Promise<void>;
  /** Release the subscriptions. Idempotent. */
  destroy(): void;
}

export interface PlayArbiterDeps {
  session: Pick<Session, 'saveAndEvaluate' | 'evaluateBuffer'> & {
    playback: Pick<SongPlayback, 'arm'>;
  };
  runtime: Pick<Runtime, 'store' | 'stop' | 'canAudition'>;
  /** The recording controller; absent outside Tauri. */
  recording?: Pick<RecordingController, 'record' | 'stop'> & {
    store: Readable<Pick<RecordingUiState, 'phase'>>;
  };
  /** The memo library; absent outside Tauri. */
  memos?: Pick<Memos, 'player' | 'stopPlayback'>;
  /** Silence every audition (the runtime's `onPlaybackRequest`). */
  stopAuditions(): void;
}

function derive(
  takePhase: RecordingPhase | null,
  memoAudible: boolean,
  started: boolean,
): PlayArbiterState {
  const taking = takeRunning(takePhase);
  const busy = taking || memoAudible;
  return {
    taking,
    takePhase,
    memoAudible,
    started,
    stopTarget: taking ? 'take' : memoAudible ? 'memo' : started ? 'song' : null,
    canPlayFrom: !(busy && started) && takePhase !== 'starting',
  };
}

export function createPlayArbiter(deps: PlayArbiterDeps): PlayArbiter {
  const { session, runtime, recording, memos } = deps;
  let playRequests = 0;
  let state = derive(null, false, false);
  const store = writable(state);
  /** Publish only a real change (`runtime.store` fires on every stats tick). */
  const update = (takePhase: RecordingPhase | null, audible: boolean, started: boolean) => {
    if (
      takePhase === state.takePhase &&
      audible === state.memoAudible &&
      started === state.started
    ) {
      return;
    }
    const prev = state;
    state = derive(takePhase, audible, started);
    store.set(state);
    // Auditions cannot start during a take or an audible memo, so silencing them on entry is enough.
    if ((state.taking && !prev.taking) || (state.memoAudible && !prev.memoAudible)) {
      deps.stopAuditions();
    }
  };

  const offs: (() => void)[] = [];
  offs.push(runtime.store.subscribe((s) => update(state.takePhase, state.memoAudible, s.started)));
  if (recording) {
    offs.push(
      recording.store.subscribe(({ phase }) => update(phase, state.memoAudible, state.started)),
    );
  }
  if (memos) {
    offs.push(
      memos.player.subscribe((p) => update(state.takePhase, memoAudible(p), state.started)),
    );
  }

  function play(): boolean {
    if (state.taking) return false;
    deps.stopAuditions();
    playRequests++;
    memos?.stopPlayback();
    void session.saveAndEvaluate().finally(() => playRequests--);
    return true;
  }

  return {
    store: { subscribe: store.subscribe },
    auditionsAvailable: () =>
      playRequests === 0 && runtime.canAudition() && !state.taking && !state.memoAudible,
    play,
    playFrom(choice) {
      if (!state.canPlayFrom) return false;
      const wasStarted = state.started;
      const armed = session.playback.arm(choice);
      if (armed && !wasStarted) play();
      return armed;
    },
    stop() {
      const { stopTarget: stopped, started: wasPlaying, takePhase } = state;
      if (stopped === 'take' && recording) {
        // Starting or stopping, the take's stop ends nothing (stopping, it returns the save).
        const ends = takePhase !== 'starting' && takePhase !== 'stopping';
        return { stopped: ends ? stopped : null, wasPlaying, done: recording.stop() };
      }
      memos?.stopPlayback();
      runtime.stop();
      return { stopped, wasPlaying, done: Promise.resolve() };
    },
    evaluate(path) {
      memos?.stopPlayback();
      // A pending play, as `play`'s: auditions wait for it.
      playRequests++;
      let pending: Promise<SessionEvalOutcome>;
      try {
        pending = session.evaluateBuffer(path, state.taking ? { autostart: false } : undefined);
      } catch (e) {
        playRequests--;
        throw e;
      }
      return pending.finally(() => playRequests--);
    },
    record(choice) {
      memos?.stopPlayback();
      return recording?.record(choice) ?? Promise.resolve();
    },
    destroy() {
      offs.splice(0).forEach((off) => off());
    },
  };
}

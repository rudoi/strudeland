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
import { get, writable, type Readable } from 'svelte/store';
import type { MemoPlayerState } from './audio/memo-player';
import type { SessionEvalOutcome } from './project/session';
import type { ArmChoice } from './project/song-playback';
import { takeRunning } from './recording/memo-ui';
import type { BackingChoice, RecordingPhase } from './recording/types';
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
  /** Play is offered: not during a take. */
  canPlay: boolean;
  /** What a stop would stop now; null when nothing sounds. */
  stopTarget: StopTarget | null;
  /** The toolbar's stop is offered. */
  stopEnabled: boolean;
  /** A take runs or a memo is audible. */
  busy: boolean;
  /** Section items may arm: not while a take's or a memo's song backing plays. */
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
   * The Timeline: arm a section and, while the song is stopped, play. True iff the choice armed.
   */
  playFrom(choice: ArmChoice): boolean;
  /**
   * The toolbar's and the control socket's stop: during a take end it (nothing while it is
   * starting or stopping), otherwise stop the memo and then the runtime, even when stopped (that
   * cancels an evaluate's autostart and silences auditions). Throws propagate.
   */
  stop(): StopResult;
  /**
   * The control socket's evaluate: stop a memo, then evaluate `path`'s buffer without saving;
   * during a take the song is not started (a playing backing takes the new code).
   */
  evaluate(path: string): Promise<SessionEvalOutcome>;
  /** Start a take: stop a memo first. Call from the click: `resumeAudio` runs inside it. */
  record(choice: BackingChoice): Promise<void>;
  /** Release the subscriptions. Idempotent. */
  destroy(): void;
}

export interface PlayArbiterDeps {
  session: {
    readonly playback: {
      readonly store: Readable<{ started: boolean }>;
      arm(choice: ArmChoice): boolean;
    };
    saveAndEvaluate(): Promise<void>;
    evaluateBuffer(path: string, opts?: { autostart?: boolean }): Promise<SessionEvalOutcome>;
  };
  runtime: Pick<Runtime, 'store' | 'stop' | 'canAudition'>;
  /** The recording controller; absent outside Tauri. */
  recording?: {
    readonly store: Readable<{ phase: RecordingPhase }>;
    record(choice: BackingChoice): Promise<void>;
    stop(): Promise<void>;
  };
  /** The memo library; absent outside Tauri. */
  memos?: {
    readonly player: Readable<MemoPlayerState>;
    stopPlayback(): void;
  };
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
    canPlay: !taking,
    stopTarget: taking ? 'take' : memoAudible ? 'memo' : started ? 'song' : null,
    stopEnabled: started || taking || memoAudible,
    busy,
    canPlayFrom: !(busy && started),
  };
}

export function createPlayArbiter(deps: PlayArbiterDeps): PlayArbiter {
  const { session, runtime, recording, memos } = deps;
  let takePhase: RecordingPhase | null = null;
  let memoAudible = false;
  let started = false;
  let playRequests = 0;
  let state = derive(takePhase, memoAudible, started);
  const store = writable(state);
  let ready = false;
  const publish = () => {
    if (!ready) return;
    state = derive(takePhase, memoAudible, started);
    store.set(state);
  };

  const offs: (() => void)[] = [];
  offs.push(
    runtime.store.subscribe((s) => {
      started = s.started;
      publish();
    }),
  );
  if (recording) {
    offs.push(
      recording.store.subscribe(({ phase }) => {
        takePhase = phase;
        publish();
        if (takeRunning(phase)) deps.stopAuditions();
      }),
    );
  }
  if (memos) {
    offs.push(
      memos.player.subscribe(({ status }) => {
        memoAudible = status === 'loading' || status === 'playing';
        publish();
        if (memoAudible) deps.stopAuditions();
      }),
    );
  }
  ready = true;
  publish();

  const taking = () => takeRunning(takePhase);

  function play(): boolean {
    if (taking()) return false;
    deps.stopAuditions();
    playRequests++;
    memos?.stopPlayback();
    void session.saveAndEvaluate().finally(() => playRequests--);
    return true;
  }

  return {
    store: { subscribe: store.subscribe },
    auditionsAvailable: () =>
      playRequests === 0 && runtime.canAudition() && !taking() && !memoAudible,
    play,
    playFrom(choice) {
      const wasStarted = get(session.playback.store).started;
      const armed = session.playback.arm(choice);
      if (armed && !wasStarted) play();
      return armed;
    },
    stop() {
      const wasPlaying = get(runtime.store).started;
      if (taking() && recording) {
        return { stopped: 'take', wasPlaying, done: recording.stop() };
      }
      const stopped: StopTarget | null = memoAudible ? 'memo' : wasPlaying ? 'song' : null;
      memos?.stopPlayback();
      runtime.stop();
      return { stopped, wasPlaying, done: Promise.resolve() };
    },
    evaluate(path) {
      memos?.stopPlayback();
      return session.evaluateBuffer(path, taking() ? { autostart: false } : undefined);
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

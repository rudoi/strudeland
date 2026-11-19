/**
 * The recording controller: arms the microphone while a project is open, runs a take (evaluate,
 * open the capture, measure the clock, start the backing, find bar 1's host time), stops it and
 * hands Rust the sidecar fields. A state machine over `RecordingUiState.phase`; the UI reads
 * `store` and `level` and calls the methods. It talks to Tauri only through `RecordingApi`, to the
 * scheduler only through the runtime and the backing starter, and to audio only through the
 * output clock.
 */
import { get, writable, type Readable } from 'svelte/store';
import type { OutputClock } from '../audio/output-clock';
import { fileStem } from '../project/session';
import type { Runtime, RuntimeState } from '../strudel/runtime';
import {
  beatsPerBar,
  bpmFromCps,
  buildStructure,
  meterString,
  type Meter,
  type SongStructure,
} from '../strudel/structure';
import { toRecordingError, type RecordingApi } from './api';
import { BACKING_MESSAGES, LEAD_S, type BackingStarter } from './backing';
import {
  clickInterval,
  downbeatHostNs,
  measureClockOffset,
  offsetToSidecar,
  stampToSidecar,
  type ClockOffset,
} from './sync';
import type {
  BackingChoice,
  InputSelection,
  LevelEvent,
  RecordingPhase,
  RecordingUiError,
  RecordingUiState,
  SidecarBacking,
  SidecarTempo,
  StateEvent,
  StopFields,
} from './types';

export interface RecordingController {
  readonly store: Readable<RecordingUiState>;
  /** The latest input level; null unless armed (or later). */
  readonly level: Readable<LevelEvent | null>;
  /** Call from the click: its first statement resumes audio inside the gesture. */
  record(choice: BackingChoice): Promise<void>;
  /** Ends the take (in `count-in` or `recording`; otherwise nothing). Resolves once it is saved. */
  stop(): Promise<void>;
  /** Saves the choice and re-arms with it (in `idle` or `armed`). */
  selectInput(deviceId: string, channel: number): Promise<void>;
  /** Re-reads the devices (in `idle` or `armed`); from `idle` it also re-arms. */
  refreshDevices(): Promise<void>;
  clearError(): void;
  destroy(): void;
}

export interface RecordingControllerDeps {
  api: RecordingApi;
  runtime: Pick<Runtime, 'store' | 'stop' | 'resumeAudio' | 'transport'>;
  session: {
    readonly store: Readable<{
      project: { root: string } | null;
      active: string | null;
      structure: SongStructure | null;
    }>;
  };
  backing: BackingStarter;
  outputClock: Pick<OutputClock, 'read' | 'latency' | 'now'>;
  /** `performance.now()`. */
  nowMs: () => number;
  /**
   * The delay the backing's master stage adds to what is heard (s), added to bar 1's host time;
   * default 0.
   */
  outputDelaySec?: () => number;
  setInterval?: (fn: () => void, ms: number) => unknown;
  clearInterval?: (id: unknown) => void;
  sleep?: (ms: number) => Promise<void>;
}

/** How often the count-in beat and the switch to `recording` are checked. */
export const COUNT_IN_TICK_MS = 50;
/** How often an output stamp is added to the sidecar during a take. */
export const STAMP_INTERVAL_MS = 1000;
/** The first output stamp is retried this often, this many times (2 s), before giving up. */
export const OUTPUT_RETRY_MS = 50;
export const OUTPUT_RETRIES = 40;
/** A cps change during the take larger than this warns (the runtime's `setcps` epsilon). */
const TEMPO_EPSILON = 1e-9;
export const MESSAGES = {
  ...BACKING_MESSAGES,
  clockWarning: 'the clock could not be measured: bar 1 is not marked in this memo',
  captureWarning: 'the memo was saved, but the input clock could not place bar 1',
  outputWarning: 'the output clock could not be read: bar 1 is not marked in this memo',
  tempoWarning: 'the tempo changed during the take: the memo keeps the tempo it started with',
} as const;

const round6 = (x: number): number => Math.round(x * 1e6) / 1e6;

/** A take from `record()` until its tempo is known. */
interface TakeStart {
  root: string;
  stem: string | null;
  /** The project changed while starting: discard at the next step. */
  abort: boolean;
  /** A Rust error event (device lost, length cap) during the take. */
  failure: RecordingUiError | null;
}

/** A take from the moment its tempo is known until it is saved or discarded (the same object). */
interface Take extends TakeStart {
  tempo: SidecarTempo;
  backing: SidecarBacking;
  meter: Meter;
  /** The grid's cps (the metronome's and the count-in beat's). */
  cps: number;
  /** Set when the song plays: a different runtime cps warns. */
  songCps: number | null;
  /** Context time of bar 1's downbeat; null with everything off. */
  T: number | null;
  startOffset: ClockOffset | null;
  bar1HostNs: bigint | null;
  outputStamps: [number, string][];
}

const hasTempo = (t: TakeStart | Take): t is Take => 'tempo' in t;

const INITIAL: RecordingUiState = {
  phase: 'idle',
  countInBeat: null,
  takeStartedAtMs: null,
  devices: [],
  input: null,
  error: null,
  permission: 'notDetermined',
  warning: null,
  stem: null,
  lastStem: null,
};

const uiError = (e: unknown): RecordingUiError => {
  const r = toRecordingError(e);
  return { kind: r.kind, message: r.message };
};

/** The phase after a take ends with `error`: the stream is gone after a loss or a refusal. */
const restingPhase = (error: RecordingUiError | null): RecordingPhase =>
  error?.kind === 'deviceLost' || error?.kind === 'permissionDenied' ? 'idle' : 'armed';

export function createRecordingController(deps: RecordingControllerDeps): RecordingController {
  const { api, runtime, session, backing, outputClock, nowMs } = deps;
  const setTimer = deps.setInterval ?? ((fn, ms) => globalThis.setInterval(fn, ms));
  const clearTimer =
    deps.clearInterval ??
    ((id) => globalThis.clearInterval(id as ReturnType<typeof globalThis.setInterval>));
  const sleep = deps.sleep ?? ((ms) => new Promise<void>((r) => setTimeout(r, ms)));
  const outputDelaySec = deps.outputDelaySec ?? (() => 0);

  let state: RecordingUiState = { ...INITIAL };
  const store = writable(state);
  const level = writable<LevelEvent | null>(null);

  let take: TakeStart | Take | null = null;
  let stopping: Promise<void> | null = null;
  let ready = false;
  let destroyed = false;
  /** Bumped by every arm and by an error event while arming; a stale arm result is dropped. */
  let armToken = 0;
  let lastRoot: string | null = null;
  const timers: unknown[] = [];
  let unwatchTempo: (() => void) | null = null;
  const offs: (() => void)[] = [];

  function patch(p: Partial<RecordingUiState>): void {
    state = { ...state, ...p };
    if (state.phase === 'idle' || state.phase === 'arming') level.set(null);
    store.set(state);
  }

  const warn = (w: string): void =>
    patch({ warning: state.warning && state.warning !== w ? `${state.warning}; ${w}` : w });

  function clearTimers(): void {
    for (const t of timers.splice(0)) clearTimer(t);
    unwatchTempo?.();
    unwatchTempo = null;
  }

  // ---- arming ----

  const project = () => get(session.store).project;

  function maybeArm(): void {
    if (!ready || destroyed) return;
    if (state.phase === 'idle' && state.error === null && project() !== null) void arm();
  }

  async function arm(opts?: { deviceId: string; channel: number }): Promise<void> {
    const token = ++armToken;
    patch({ phase: 'arming' });
    try {
      const st = await api.arm(opts);
      if (token !== armToken || destroyed) return;
      const input: InputSelection | null = st.device
        ? { deviceId: st.device.id, channel: st.channel }
        : state.input;
      patch({ phase: 'armed', permission: st.permission, input });
    } catch (e) {
      if (token !== armToken || destroyed) return;
      const error = uiError(e);
      patch({ phase: 'idle', error });
      if (error.kind === 'permissionDenied') await refreshPermission();
    }
  }

  async function refreshPermission(): Promise<void> {
    try {
      const st = await api.status();
      if (!destroyed) patch({ permission: st.permission });
    } catch (e) {
      console.error(e);
    }
  }

  // ---- events ----

  function onState(e: StateEvent): void {
    if (destroyed || !e.error) return;
    const error: RecordingUiError = { kind: e.error.kind, message: e.error.message };
    switch (state.phase) {
      case 'arming':
        armToken++;
        patch({ phase: 'idle', error });
        return;
      case 'armed':
        // The stream is gone without a take (a device unplugged while armed).
        if (e.state === 'idle') patch({ phase: 'idle', error });
        return;
      case 'starting':
        if (take) take.failure = error;
        return;
      case 'count-in':
      case 'recording':
        if (take) take.failure = error;
        patch({ error });
        void stop();
        return;
      case 'stopping':
        // Rust failed while the memo was being saved (a device lost): `finish` settles by it.
        if (take && take.failure === null) take.failure = error;
        return;
      default:
        return;
    }
  }

  function onLevel(e: LevelEvent): void {
    if (destroyed || state.phase === 'idle' || state.phase === 'arming') return;
    level.set({ peak: e.peak, rms: e.rms });
  }

  function onSession(s: { project: { root: string } | null }): void {
    const root = s.project?.root ?? null;
    const prev = lastRoot;
    lastRoot = root;
    if (prev !== null && root !== prev && take) {
      // Normally the session's beforeProjectChange hook stopped the take already.
      if (state.phase === 'starting') take.abort = true;
      else if (state.phase === 'count-in' || state.phase === 'recording') void stop();
    }
    maybeArm();
  }

  // ---- a take ----

  const interrupted = (t: TakeStart): boolean => t.abort || t.failure !== null || destroyed;

  /** End a take before anything was captured (or after a discard): back to resting. */
  function settle(t: TakeStart, error: RecordingUiError | null): void {
    clearTimers();
    if (take === t) take = null;
    const err = t.failure ?? error;
    patch({
      phase: restingPhase(err),
      error: err,
      stem: null,
      takeStartedAtMs: null,
      countInBeat: null,
    });
    maybeArm();
  }

  /** A failure after the capture started: silence, finish the memo with Rust's sidecar, delete it. */
  async function discard(t: TakeStart, error: RecordingUiError | null): Promise<void> {
    clearTimers();
    backing.stopAll();
    if (t.stem !== null) {
      try {
        await api.stop({ sidecar: null });
      } catch (e) {
        console.error(e);
      }
      try {
        await api.delete(t.root, t.stem);
      } catch (e) {
        console.error(e);
      }
    }
    settle(t, error);
  }

  const failed = (t: Take): Promise<void> => discard(t, null);

  function pushStamp(t: Take): void {
    if (!t.startOffset) return;
    const stamp = outputClock.read();
    if (stamp) t.outputStamps.push(stampToSidecar(t.startOffset.offsetNs, stamp));
  }

  function tickCountIn(t: Take): void {
    if (take !== t || state.phase !== 'count-in' || t.T === null) return;
    const now = outputClock.now();
    if (now === null) return;
    if (now >= t.T) {
      patch({ phase: 'recording', countInBeat: null });
      return;
    }
    const num = t.meter.num;
    const beat = Math.floor((now - (t.T - 1 / t.cps)) / clickInterval(t.cps, num)) + 1;
    const countInBeat = Math.min(num, Math.max(1, beat));
    if (countInBeat !== state.countInBeat) patch({ countInBeat });
  }

  async function run(choice: BackingChoice, resumed: Promise<boolean>): Promise<void> {
    // 1. Guard.
    const ss = get(session.store);
    if (destroyed || state.phase !== 'armed' || !ss.project) return;
    const song = choice.song && ss.active !== null;
    const metronome = choice.metronome;
    const countIn = choice.countIn && (song || metronome);
    const allOff = !song && !metronome;
    // 3. Read before the rewind of 2.
    const ts = get(runtime.transport.store);
    const wasPlaying = get(runtime.store).started;
    const start: TakeStart = { root: ss.project.root, stem: null, abort: false, failure: null };
    take = start;
    patch({ phase: 'starting', error: null, warning: null, countInBeat: null });
    // 2. A take always begins stopped.
    if (wasPlaying) runtime.stop();

    let audio = false;
    try {
      audio = await resumed;
    } catch (e) {
      console.error(e);
    }
    if (interrupted(start)) return settle(start, null);
    if (!allOff && !audio) return settle(start, { kind: 'audio', message: MESSAGES.audioOff });

    let startSection: string | null = null;
    let loopSection: string | null = null;
    if (song && !wasPlaying && ts.pendingStart && ts.armedSection !== null) {
      const name = ss.structure?.sections[ts.armedSection]?.name ?? null;
      if (ts.view.kind === 'loop') loopSection = name;
      else startSection = name;
    }
    const backingFields: SidecarBacking = {
      song,
      metronome,
      countIn,
      songPath: song ? ss.active : null,
      startSection,
      loopSection,
    };

    // 4. Tempo.
    let cps: number;
    let meter: Meter;
    let tempoOverride = false;
    if (song) {
      const prep = await backing.prepareSong();
      if (interrupted(start)) return settle(start, null);
      if (!prep.ok) return settle(start, { kind: prep.kind, message: prep.message });
      ({ cps, meter, tempoOverride } = prep);
    } else {
      // The open song's static header; the defaults without one (or with a meta error).
      const s = get(session.store).structure ?? buildStructure({});
      ({ cps, meter } = s);
    }
    // The same object, so an event or a project switch still finds it as `take`.
    const t: Take = Object.assign(start, {
      tempo: {
        bpm: round6(bpmFromCps(cps, meter)),
        meter: meterString(meter),
        cps,
        beatsPerBar: beatsPerBar(meter),
        tempoOverride,
      },
      backing: backingFields,
      meter,
      cps,
      songCps: null,
      T: null,
      startOffset: null,
      bar1HostNs: null,
      outputStamps: [],
    });

    // 5. Open the capture (resolves once it runs).
    try {
      const songStem = ss.active !== null ? fileStem(ss.active) : undefined;
      const { stem } = await api.start({
        root: t.root,
        ...(songStem !== undefined ? { songStem } : {}),
        fields: { tempo: t.tempo, backing: t.backing },
      });
      t.stem = stem;
      patch({ stem, takeStartedAtMs: nowMs() });
    } catch (e) {
      const error = uiError(e);
      settle(t, error);
      if (error.kind === 'permissionDenied') await refreshPermission();
      return;
    }
    if (interrupted(t)) return failed(t);

    // 6. The start offset.
    try {
      t.startOffset = await measureClockOffset({ hostNowNs: () => api.hostNowNs(), nowMs });
    } catch (e) {
      console.error(e);
      warn(MESSAGES.clockWarning);
    }
    if (interrupted(t)) return failed(t);

    if (!allOff) {
      // 7. The backing, and the downbeat T.
      let T: number;
      if (song) {
        const r = await backing.startSong({ countIn, prepared: true });
        if (interrupted(t)) return failed(t);
        if (!r || !('T' in r)) return discard(t, { kind: 'audio', message: MESSAGES.songFailed });
        T = r.T;
        t.cps = r.cps;
        t.songCps = r.cps;
      } else {
        const now = outputClock.now();
        if (now === null) return discard(t, { kind: 'audio', message: MESSAGES.audioOff });
        T = now + LEAD_S + (countIn ? 1 / cps : 0);
      }
      t.T = T;
      if (metronome || countIn) {
        const ok = backing.startMetronome({
          T,
          cps: t.cps,
          num: meter.num,
          from: countIn ? -meter.num : 0,
          until: metronome ? undefined : T,
        });
        if (!ok) return discard(t, { kind: 'audio', message: MESSAGES.metronomeFailed });
      }

      // 8. Bar 1's host time from one output stamp.
      if (t.startOffset) {
        let stamp = outputClock.read();
        for (let i = 0; stamp === null && i < OUTPUT_RETRIES; i++) {
          await sleep(OUTPUT_RETRY_MS);
          if (interrupted(t)) return failed(t);
          stamp = outputClock.read();
        }
        if (stamp) {
          t.bar1HostNs = downbeatHostNs(t.startOffset.offsetNs, stamp, T, outputDelaySec());
          t.outputStamps.push(stampToSidecar(t.startOffset.offsetNs, stamp));
        } else {
          warn(MESSAGES.outputWarning);
        }
      }
    }
    if (interrupted(t)) return failed(t);

    // 9. Count-in or recording.
    const now = outputClock.now();
    const counting = countIn && t.T !== null && now !== null && now < t.T;
    patch({ phase: counting ? 'count-in' : 'recording', countInBeat: null });
    if (counting) {
      tickCountIn(t);
      timers.push(setTimer(() => tickCountIn(t), COUNT_IN_TICK_MS));
    }
    if (t.startOffset) timers.push(setTimer(() => pushStamp(t), STAMP_INTERVAL_MS));
    if (t.songCps !== null) {
      const gridCps = t.songCps;
      let warned = false;
      unwatchTempo = runtime.store.subscribe((rs: RuntimeState) => {
        if (warned || Math.abs(rs.cps - gridCps) <= TEMPO_EPSILON) return;
        warned = true;
        warn(MESSAGES.tempoWarning);
      });
    }
  }

  async function finish(t: Take): Promise<void> {
    const now = outputClock.now();
    // A stop before the downbeat (during the count-in): bar 1 never sounded.
    const beforeDownbeat = t.T === null || (now !== null ? now < t.T : state.phase === 'count-in');
    patch({ phase: 'stopping', countInBeat: null });
    clearTimers();
    backing.stopAll();
    pushStamp(t);
    let end: ClockOffset | null = null;
    try {
      end = await measureClockOffset({ hostNowNs: () => api.hostNowNs(), nowMs });
    } catch (e) {
      console.error(e);
    }
    const sidecar: StopFields = {
      output: outputClock.latency(),
      tempo: t.tempo,
      backing: t.backing,
      timing: {
        bar1HostNs: beforeDownbeat || t.bar1HostNs === null ? null : t.bar1HostNs.toString(),
        clockOffsetNs: {
          start: t.startOffset ? offsetToSidecar(t.startOffset) : null,
          end: end ? offsetToSidecar(end) : null,
        },
        outputStamps: t.outputStamps,
      },
    };
    let stopError: RecordingUiError | null = null;
    let lastStem = state.lastStem;
    try {
      const saved = await api.stop({ sidecar });
      lastStem = saved.stem;
      if (sidecar.timing.bar1HostNs !== null && saved.sidecar?.timing.bar1Frame === null) {
        warn(MESSAGES.captureWarning);
      }
    } catch (e) {
      stopError = uiError(e);
    }
    if (take === t) take = null;
    // Read after the stop: an error event can arrive while it runs.
    patch({
      phase: restingPhase(t.failure),
      error: t.failure ?? stopError,
      lastStem,
      stem: null,
      takeStartedAtMs: null,
    });
    maybeArm();
  }

  function stop(): Promise<void> {
    if (stopping) return stopping;
    // A take in `count-in` or `recording` always has its tempo.
    if (!take || !hasTempo(take) || (state.phase !== 'count-in' && state.phase !== 'recording')) {
      return Promise.resolve();
    }
    stopping = finish(take).finally(() => {
      stopping = null;
    });
    return stopping;
  }

  // ---- setup ----

  void (async () => {
    try {
      const offState = await api.onState(onState);
      const offLevel = await api.onLevel(onLevel);
      offs.push(offState, offLevel);
      if (destroyed) offs.splice(0).forEach((f) => f());
    } catch (e) {
      console.error(e);
    }
    let armedInput: InputSelection | null = null;
    try {
      const st = await api.status();
      if (!destroyed) {
        patch({ permission: st.permission });
        if (st.state === 'armed') {
          armedInput = st.device ? { deviceId: st.device.id, channel: st.channel } : null;
          patch({ phase: 'armed', input: armedInput ?? state.input });
        }
      }
    } catch (e) {
      console.error(e);
    }
    try {
      const d = await api.devices();
      if (!destroyed) patch({ devices: d.devices, input: armedInput ?? d.selected });
    } catch (e) {
      console.error(e);
    }
    if (destroyed) return;
    ready = true;
    offs.push(session.store.subscribe(onSession));
  })();

  return {
    store: { subscribe: store.subscribe },
    level: { subscribe: level.subscribe },

    record(choice) {
      // Synchronously, inside the click: WebKit only honours resume() in the gesture.
      const resumed = runtime.resumeAudio();
      return run(choice, resumed);
    },

    stop,

    async selectInput(deviceId, channel) {
      if (destroyed || (state.phase !== 'idle' && state.phase !== 'armed')) return;
      patch({ error: null });
      await arm({ deviceId, channel });
    },

    async refreshDevices() {
      if (destroyed || (state.phase !== 'idle' && state.phase !== 'armed')) return;
      try {
        const d = await api.devices();
        if (destroyed) return;
        patch({ devices: d.devices, input: d.selected });
      } catch (e) {
        console.error(e);
        return;
      }
      // Unplugging and re-plugging, or granting access in System Settings, recovers here.
      if (state.phase === 'idle' && project() !== null) {
        patch({ error: null });
        await arm();
      }
    },

    clearError() {
      if (state.error === null) return;
      patch({ error: null });
      maybeArm();
    },

    destroy() {
      if (destroyed) return;
      const running = stop();
      destroyed = true;
      clearTimers();
      offs.splice(0).forEach((f) => f());
      void running.then(() => api.disarm()).catch((e) => console.error(e));
    },
  };
}

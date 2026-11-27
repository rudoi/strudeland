import { isTauri } from '@tauri-apps/api/core';
import type { EditorView } from '@codemirror/view';
import { get, writable, type Readable, type Writable } from 'svelte/store';
import { createInstrumentPreview } from './audio/instrument-preview';
import { createMemoPlayer } from './audio/memo-player';
import { createMetronome } from './audio/metronome';
import { createOutputClock } from './audio/output-clock';
import { tauriPlaybackApi } from './audio/playback-api';
import { createRustBackend, type RustBackend } from './audio/rust';
import { createSuperdoughBackend, type SuperdoughBackend } from './audio/superdough';
import { createTauriControlApi, type ControlApi } from './control/api';
import { createControlHandler, replyTo, type ControlHandler } from './control/handler';
import type { ControlInfo, ControlRequest } from './control/protocol';
import { createEditorState, setKeymap, withKeymap, type EditorHooks } from './editor/setup';
import { createVimMode, defineVimWrite, type VimMode } from './editor/vim-mode';
import { createVisuals, highlightAudition } from './editor/visuals';
import { inspectPianoRoll } from './piano-roll/api';
import { createInspectionQueue } from './piano-roll/inspection';
import { createTauriPrefsApi, type PrefsApi } from './prefs/api';
import { migrateLimiter, webStorage } from './prefs/migrate';
import { createPrefs, type PrefsStore } from './prefs/store';
import { createTauriProjectApi, type InstrumentBrowserApi, type ProjectApi } from './project/api';
import { createSession, errorMessage, windowTitle, type Session } from './project/session';
import { createTauriRecordingApi, type RecordingApi } from './recording/api';
import { createBackingStarter } from './recording/backing';
import { createRecordingController, type RecordingController } from './recording/controller';
import { captureAuditionSynths, createNoteAuditionPlayer } from './audio/note-audition';
import { createAuditionQueue } from './editor/note-audition-queue';
import { createMemos, type Memos } from './recording/memos';
import { createPlayArbiter, type PlayArbiter } from './play-arbiter';
import { createScopeClock, type ScopeClock } from './scope/clock';
import { prebake } from './strudel/prebake';
import { createRuntime, type Runtime } from './strudel/runtime';

export interface App {
  runtime: Runtime;
  session: Session;
  instruments: InstrumentBrowserApi;
  instrumentPreview: ReturnType<typeof createInstrumentPreview>;
  rollInspection: ReturnType<typeof createInspectionQueue>;
  auditionPianoRoll(
    pitch: number,
    sound?: string,
    automatic?: boolean,
  ): Promise<'triangle' | 'selected' | null>;
  stopPianoRollAudition(): void;
  focusEditor(): void;
  /** The control socket's state; `off` outside Tauri (no `ControlApi`). */
  control: Readable<ControlInfo>;
  /** Editor callbacks wired to the session; the editor view is created with these. */
  hooks: EditorHooks;
  /** The master-output scope clock; it runs while the runtime is started. */
  scopeClock: ScopeClock;
  /**
   * The recording controller; unset without a `RecordingApi` (outside Tauri), and the record
   * controls stay disabled.
   */
  recording?: RecordingController;
  /** The open project's memos and their playback; unset like `recording`. */
  memos?: Memos;
  /** What play, stop, section arming and record mean while a song, memo, take or audition sounds. */
  arbiter: PlayArbiter;
  /** App preferences; the one frontend copy of each pref. */
  prefs: PrefsStore;
  /** The shown view's vim mode; `null` while vim is off. */
  vimMode: VimMode;
  /** Resolves once the prefs load has settled and the shown view has been reconfigured; never rejects. */
  ready: Promise<void>;
  /** Flip the vim pref (the user's toggle) and focus the editor; never rejects. */
  toggleVim(): Promise<void>;
  /** The editor view (or undefined when it is destroyed); the session shows the active song in it. */
  attachView(view: EditorView | undefined): void;
  /** Unlock preview audio on a gesture in the editor. */
  prepareNoteAudition(target: EventTarget | null): boolean;
  /**
   * Restore vim's `:w`, stop listening for control requests and prefs, dispose of the scope clock, stop memo playback and the
   * recording controller (the component is destroyed, or HMR replaced it). Idempotent.
   */
  destroy(): void;
}

/** `createApp`'s test seams. Every field is optional; `createApp()` builds the real app. */
export interface AppOptions {
  /** Default: Tauri's. */
  api?: ProjectApi;
  /**
   * Whether the page runs in Tauri; default `isTauri`. It picks the defaults below and turns on
   * the Rust backend, the piano-roll inspection and the window title. It is also the session's
   * `isTauri` (project features).
   */
  inTauri?: () => boolean;
  /** Default: Tauri's in Tauri. With `null` the control socket is not attached. */
  controlApi?: ControlApi | null;
  /**
   * Default: Tauri's when the page really runs in Tauri (`isTauri`, not `inTauri`). With `null`
   * there is no recording controller and no memo list.
   */
  recordingApi?: RecordingApi | null;
  /** Default: Tauri's in Tauri. With `null` the prefs live in memory only. */
  prefsApi?: PrefsApi | null;
  /**
   * The live backends. Default: a superdough backend, plus in Tauri a Rust backend over the
   * playback API. Given without `rust`, every song plays on superdough.
   */
  backends?: { superdough: SuperdoughBackend; rust?: RustBackend };
}

/** Composition root: the live backends, one runtime, one session per window. */
export function createApp(options: AppOptions = {}): App {
  const {
    api = createTauriProjectApi(),
    inTauri = isTauri,
    controlApi = inTauri() ? createTauriControlApi() : null,
    recordingApi = isTauri() ? createTauriRecordingApi() : null,
    prefsApi = inTauri() ? createTauriPrefsApi() : null,
  } = options;
  const prefs = createPrefs(prefsApi);
  const rollInspection = createInspectionQueue(inTauri() ? inspectPianoRoll : null);
  const vimMode = createVimMode();
  let view: EditorView | undefined;
  let runtime!: Runtime;
  const visuals = createVisuals({
    getView: () => view,
    getScheduler: () => runtime.drawingScheduler,
  });
  const { superdough, rust } = options.backends ?? {
    superdough: createSuperdoughBackend(),
    // The Rust engine plays through the app; in a plain browser every song plays on superdough.
    rust: inTauri() ? createRustBackend(tauriPlaybackApi) : undefined,
  };
  runtime = createRuntime({
    backends: { superdough, rust },
    async prebake() {
      await prebake();
      captureAuditionSynths();
    },
    hooks: { ...visuals, onPlaybackRequest: stopAuditions },
  });
  const scopeClock = createScopeClock((buf) => runtime.readScope(buf));
  let unfollow: (() => void) | null = runtime.store.subscribe((s) => {
    scopeClock.setRunning(s.started);
  });
  let recording: RecordingController | undefined;
  let memos: Memos | undefined;
  // Built once the session and the recording exist; nothing reads it before.
  let arbiter!: PlayArbiter;
  const auditionsAvailable = () => arbiter.auditionsAvailable();
  const canAudition = () => Boolean(view) && prefs.auditionNotes() && auditionsAvailable();
  const auditionGraph = () => superdough.graph();
  const player = createNoteAuditionPlayer({ graph: auditionGraph });
  let rollAudition: { automatic: boolean } | null = null;
  function stopPianoRollAudition(): void {
    if (!rollAudition) return;
    rollAudition = null;
    player.stop();
  }
  const audition = createAuditionQueue({
    eligible: canAudition,
    // Read at call time, so a replaced `prepareAudition` is the one called.
    prepare: () => runtime.prepareAudition(),
    play(note) {
      rollAudition = null;
      const editor = view;
      const doc = editor?.state.doc;
      void player.play(note, () => {
        if (editor && view === editor && editor.state.doc === doc) {
          return highlightAudition(editor, note);
        }
      });
    },
    stop: player.stop,
  });
  const instrumentPreview = createInstrumentPreview({
    eligible: auditionsAvailable,
    prepare: () => runtime.prepareAudition(),
    graph: auditionGraph,
    render: (version) => api.instrumentPreview(version),
  });
  function stopAuditions() {
    rollAudition = null;
    audition.stop();
    instrumentPreview.stop();
  }
  const hooks: EditorHooks = {
    onSave: () => void session.save(),
    onEvaluate: () => arbiter.play(),
    onStop: () => void arbiter.stop(),
    onDocChange: () => session.docChanged(),
    onSelectionChange(selected) {
      stopPianoRollAudition();
      session.pianoRollChanged(selected);
    },
    auditionEnabled: canAudition,
    onAuditionEdit: audition.edit,
    vim: prefs.vim,
  };
  const session = createSession({
    api,
    runtime,
    createState: (doc) => createEditorState(doc, runtime, hooks),
    isTauri: inTauri,
    formatOnSave: prefs.formatOnSave,
    prepareState: (s) => withKeymap(s, prefs.vim()),
    onShown: (v) => {
      rollAudition = null;
      audition.stop();
      vimMode.attach(v);
    },
    // A project switch stops the memo and the take first.
    beforeProjectChange: async () => {
      stopAuditions();
      memos?.stopPlayback();
      await recording?.stop();
    },
  });
  const restoreVimWrite = defineVimWrite(() => void session.save());
  let unprefs: (() => void) | null = prefs.store.subscribe(({ vim, auditionNotes, limiter }) => {
    if (limiter !== get(runtime.store).limiter) runtime.setLimiter(limiter);
    if (!auditionNotes && (!rollAudition || rollAudition.automatic)) {
      rollAudition = null;
      audition.stop();
    }
    if (view && setKeymap(view, vim)) vimMode.attach(view);
  });
  const ready = prefs.load();
  if (prefsApi) void ready.then(() => migrateLimiter(prefs, webStorage));
  if (recordingApi) {
    ({ recording, memos } = createRecording(recordingApi, superdough, runtime, session, api));
  }
  arbiter = createPlayArbiter({ session, runtime, recording, memos, stopAuditions });
  if (inTauri()) syncWindowTitle(session, api);
  const control = writable<ControlInfo>({ status: 'off', path: null, detail: null });
  const detach = controlApi
    ? attachControl(
        controlApi,
        createControlHandler({
          session,
          runtime,
          vim: prefs.vim,
          arbiter,
        }),
        control,
      )
    : () => {};
  return {
    runtime,
    session,
    instruments: api,
    instrumentPreview,
    rollInspection,
    async auditionPianoRoll(pitch, sound, automatic = false) {
      const eligible = () => auditionsAvailable() && (!automatic || prefs.auditionNotes());
      if (!eligible() || !Number.isInteger(pitch) || pitch < 0 || pitch > 127) return null;
      stopAuditions();
      const request = { automatic };
      rollAudition = request;
      const ready = await runtime.prepareAudition();
      if (rollAudition !== request || !ready || !eligible()) return null;
      let voice: 'triangle' | 'selected' | null = null;
      await player.play(
        { pitch, sound: sound ?? '', controls: {}, fallbackToTriangle: true },
        (fallback) => {
          voice = fallback ? 'triangle' : 'selected';
          return undefined;
        },
      );
      return rollAudition === request ? voice : null;
    },
    stopPianoRollAudition,
    focusEditor: () => view?.focus(),
    control: { subscribe: control.subscribe },
    hooks,
    prefs,
    vimMode,
    ready,
    async toggleVim() {
      const saved = prefs.setVim(!prefs.vim());
      view?.focus();
      await saved;
    },
    attachView(v) {
      rollAudition = null;
      audition.stop();
      view = v;
      session.attachView(v);
      vimMode.attach(v);
    },
    prepareNoteAudition(target) {
      if (!(target instanceof Node) || !view?.contentDOM.contains(target) || !canAudition()) {
        return false;
      }
      void runtime.prepareAudition();
      return true;
    },
    scopeClock,
    recording,
    memos,
    arbiter,
    destroy() {
      rollInspection.request(null);
      stopAuditions();
      arbiter.destroy();
      if (unprefs) {
        restoreVimWrite();
        unprefs();
        unprefs = null;
        vimMode.destroy();
      }
      detach();
      if (!unfollow) return;
      memos?.destroy();
      recording?.destroy();
      unfollow();
      unfollow = null;
      scopeClock.setRunning(false);
      scopeClock.dispose();
    },
  };
}

/**
 * The recording controller and the memo library over one backing starter (song and metronome),
 * one metronome and one output clock, all on the one superdough backend's graph.
 */
function createRecording(
  recordingApi: RecordingApi,
  backend: SuperdoughBackend,
  runtime: Runtime,
  session: Session,
  api: ProjectApi,
): { recording: RecordingController; memos: Memos } {
  const graph = () => backend.graph();
  const metronome = createMetronome({ graph });
  const { playback } = session;
  const backing = createBackingStarter({ runtime, playback, session, metronome });
  const recording = createRecordingController({
    api: recordingApi,
    runtime,
    playback,
    session,
    backing,
    outputClock: createOutputClock({ graph }),
    nowMs: () => performance.now(),
    // The master worklet delays everything superdough plays, the backing included.
    outputDelaySec: () => backend.masterLatencySec(),
  });
  const memos = createMemos({
    api: recordingApi,
    session,
    onChange: (handler) => api.onChange(handler),
    recording,
    createPlayer: (io) =>
      createMemoPlayer({
        graph,
        isRunning: () => backend.isRunning(),
        resumeAudio: () => runtime.resumeAudio(),
        backing,
        runtime,
        ...io,
      }),
  });
  return { recording, memos };
}

/** Keep the window title in step with the session; only set when it changes. */
function syncWindowTitle(session: Session, api: ProjectApi): void {
  let last: string | null = null;
  session.store.subscribe((state) => {
    const title = windowTitle(state);
    if (title === last) return;
    last = title;
    api.setWindowTitle(title).catch((e) => console.error(e));
  });
}

/**
 * Listen for requests, then attach (so none is missed); runs once, not awaited by `createApp`.
 * Returns an idempotent function that stops listening, also when the listener is not set up yet.
 */
function attachControl(
  controlApi: ControlApi,
  handler: ControlHandler,
  control: Writable<ControlInfo>,
): () => void {
  let destroyed = false;
  let unlisten: (() => void) | null = null;
  const stop = () => {
    const u = unlisten;
    unlisten = null;
    u?.();
  };
  const handle = async (req: ControlRequest) => {
    const reply = await replyTo(handler, req);
    try {
      await controlApi.reply(req.id, reply);
    } catch (e) {
      console.error(e);
    }
  };
  void (async () => {
    try {
      unlisten = await controlApi.onRequest((req) => void handle(req));
      if (destroyed) return stop();
      control.set(await controlApi.attach());
    } catch (e) {
      console.error(e);
      control.set({
        status: 'disabled',
        path: null,
        detail: errorMessage(e),
      });
    }
  })();
  return () => {
    destroyed = true;
    stop();
  };
}

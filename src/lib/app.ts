import { isTauri } from '@tauri-apps/api/core';
import type { EditorView } from '@codemirror/view';
import { writable, type Readable, type Writable } from 'svelte/store';
import type { AudioGraph } from './audio/backend';
import { createInstrumentPreview } from './audio/instrument-preview';
import { createMemoPlayer } from './audio/memo-player';
import { createMetronome } from './audio/metronome';
import { createOutputClock } from './audio/output-clock';
import { tauriPlaybackApi } from './audio/playback-api';
import { createRustBackend } from './audio/rust';
import { createSuperdoughBackend } from './audio/superdough';
import { createTauriControlApi, type ControlApi } from './control/api';
import { createControlHandler, replyTo, type ControlHandler } from './control/handler';
import type { ControlInfo, ControlRequest } from './control/protocol';
import { createEditorState, setKeymap, withKeymap, type EditorHooks } from './editor/setup';
import { createVimMode, defineVimWrite, type VimMode } from './editor/vim-mode';
import { createVisuals, highlightAudition } from './editor/visuals';
import { inspectPianoRoll } from './piano-roll/api';
import { createInspectionQueue } from './piano-roll/inspection';
import { createTauriPrefsApi, type PrefsApi } from './prefs/api';
import { createPrefs, type PrefsStore } from './prefs/store';
import { createTauriProjectApi, type InstrumentBrowserApi, type ProjectApi } from './project/api';
import { createSession, errorMessage, windowTitle, type Session } from './project/session';
import { createTauriRecordingApi, type RecordingApi } from './recording/api';
import { createBackingStarter } from './recording/backing';
import { createRecordingController, type RecordingController } from './recording/controller';
import { takeRunning } from './recording/memo-ui';
import { captureAuditionSynths, createNoteAuditionPlayer } from './audio/note-audition';
import { createAuditionQueue } from './editor/note-audition-queue';
import { createMemos, type Memos } from './recording/memos';
import { loadLimiter, saveLimiter } from './audio/limiter-pref';
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
   * Save and evaluate the active song (the toolbar's play and Cmd-Enter); stops a playing memo
   * first. Does nothing while a take runs: play and stop belong to it.
   */
  evaluateDoc(): void;
  /**
   * Restore vim's `:w`, stop listening for control requests and prefs, dispose of the scope clock, stop memo playback and the
   * recording controller (the component is destroyed, or HMR replaced it). Idempotent.
   */
  destroy(): void;
}

/**
 * Composition root: the live backends, one runtime, one session per window. `api`, `controlApi` and
 * `recordingApi` are test seams; `controlApi` defaults to Tauri's in Tauri, and with `null` the
 * control socket is not attached. `recordingApi` defaults to Tauri's when the page really runs in
 * Tauri; with `null` there is no recording controller and no memo list. `prefsApi` defaults to
 * Tauri's in Tauri; with `null` the prefs live in memory only.
 */
export function createApp(
  api: ProjectApi = createTauriProjectApi(),
  inTauri: () => boolean = isTauri,
  controlApi: ControlApi | null = inTauri() ? createTauriControlApi() : null,
  recordingApi: RecordingApi | null = isTauri() ? createTauriRecordingApi() : null,
  prefsApi: PrefsApi | null = inTauri() ? createTauriPrefsApi() : null,
): App {
  const prefs = createPrefs(prefsApi);
  const rollInspection = createInspectionQueue(inTauri() ? inspectPianoRoll : null);
  const vimMode = createVimMode();
  let view: EditorView | undefined;
  let runtime!: Runtime;
  const visuals = createVisuals({
    getView: () => view,
    getScheduler: () => runtime.drawingScheduler,
  });
  const superdough = createSuperdoughBackend();
  // The Rust engine plays through the app; in a plain browser every song plays on superdough.
  const rust = inTauri() ? createRustBackend(tauriPlaybackApi) : undefined;
  runtime = createRuntime({
    backends: { superdough, rust },
    async prebake() {
      await prebake();
      captureAuditionSynths();
    },
    hooks: { ...visuals, onPlaybackRequest: stopAuditions },
    limiter: loadLimiter(),
  });
  const scopeClock = createScopeClock((buf) => runtime.readScope(buf));
  let savedLimiter: boolean | null = null;
  let unfollow: (() => void) | null = runtime.store.subscribe((s) => {
    scopeClock.setRunning(s.started);
    // The first call is the loaded value itself.
    if (savedLimiter !== null && s.limiter !== savedLimiter) saveLimiter(s.limiter);
    savedLimiter = s.limiter;
  });
  let recording: RecordingController | undefined;
  let memos: Memos | undefined;
  let playRequests = 0;
  let takeActive = false;
  let memoActive = false;
  const auditionsAvailable = () =>
    playRequests === 0 && runtime.canAudition() && !takeActive && !memoActive;
  const canAudition = () => Boolean(view) && prefs.auditionNotes() && auditionsAvailable();
  const auditionGraph = () => superdough.graph?.() ?? null;
  const player = createNoteAuditionPlayer({ graph: auditionGraph });
  let rollAudition: { automatic: boolean } | null = null;
  function stopPianoRollAudition(): void {
    if (!rollAudition) return;
    rollAudition = null;
    player.stop();
  }
  const audition = createAuditionQueue({
    eligible: canAudition,
    prepare: runtime.prepareAudition,
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
    prepare: runtime.prepareAudition,
    graph: auditionGraph,
    render: (version) => api.instrumentPreview(version),
  });
  function stopAuditions() {
    rollAudition = null;
    audition.stop();
    instrumentPreview.stop();
  }
  function evaluateDoc(): void {
    if (takeActive) return;
    stopAuditions();
    playRequests++;
    memos?.stopPlayback();
    void session.saveAndEvaluate().finally(() => playRequests--);
  }
  const hooks: EditorHooks = {
    onSave: () => void session.save(),
    onEvaluate: evaluateDoc,
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
  let unprefs: (() => void) | null = prefs.store.subscribe(({ vim, auditionNotes }) => {
    if (!auditionNotes && (!rollAudition || rollAudition.automatic)) {
      rollAudition = null;
      audition.stop();
    }
    if (view && setKeymap(view, vim)) vimMode.attach(view);
  });
  const ready = prefs.load();
  if (recordingApi) {
    ({ recording, memos } = createRecording(recordingApi, superdough, runtime, session, api));
  }
  const unrecording = recording?.store.subscribe(({ phase }) => {
    takeActive = takeRunning(phase);
    if (takeActive) stopAuditions();
  });
  const unmemo = memos?.player.subscribe(({ status }) => {
    memoActive = status === 'loading' || status === 'playing';
    if (memoActive) stopAuditions();
  });
  if (inTauri()) syncWindowTitle(session, api);
  const control = writable<ControlInfo>({ status: 'off', path: null, detail: null });
  const detach = controlApi
    ? attachControl(
        controlApi,
        createControlHandler({
          session,
          runtime,
          vim: prefs.vim,
          playback: {
            taking: () => takeActive,
            stopMemo: () => memos?.stopPlayback(),
            stopTake: () => recording?.stop() ?? Promise.resolve(),
          },
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
    toggleVim() {
      const saved = prefs.setVim(!prefs.vim());
      view?.focus();
      return saved;
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
    evaluateDoc,
    scopeClock,
    recording,
    memos,
    destroy() {
      rollInspection.request(null);
      stopAuditions();
      unrecording?.();
      unmemo?.();
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
  backend: { isRunning(): boolean; graph?(): AudioGraph | null; masterLatencySec?(): number },
  runtime: Runtime,
  session: Session,
  api: ProjectApi,
): { recording: RecordingController; memos: Memos } {
  const graph = () => backend.graph?.() ?? null;
  const metronome = createMetronome({ graph });
  const backing = createBackingStarter({ runtime, session, metronome });
  const recording = createRecordingController({
    api: recordingApi,
    runtime,
    session,
    backing,
    outputClock: createOutputClock({ graph }),
    nowMs: () => performance.now(),
    // The master worklet delays everything superdough plays, the backing included.
    outputDelaySec: () => backend.masterLatencySec?.() ?? 0,
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

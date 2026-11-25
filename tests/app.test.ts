import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createApp } = await import('../src/lib/app');
const { createEditor } = await import('../src/lib/editor/setup');
const { redo, undo } = await import('@codemirror/commands');
const { EditorSelection, EditorState, StateEffect } = await import('@codemirror/state');
const { EditorView: EditorViewExtension, keymap } = await import('@codemirror/view');
const { pianoRollEditHighlight, pianoRollEditLocation } = await import('../src/lib/editor/visuals');
const { planEventEdit, preparePhraseEvents } = await import('../src/lib/piano-roll/event-source');
const { resolvePostEditTarget } = await import('../src/lib/piano-roll/feedback');
const { resolveRollTarget } = await import('../src/lib/piano-roll/source');
const { get, writable } = await import('svelte/store');
const { ErrorCode } = await import('../src/lib/control/protocol');
const { getCM, Vim } = await import('@replit/codemirror-vim');
const { vimAdapter } = await import('../src/lib/editor/vim-mode');

import type { EditorView } from '@codemirror/view';
import type { Prefs } from '../src/lib/prefs/api';
import type { Sidecar } from '../src/lib/recording/types';
import { fakePrefsApi } from './helpers/fake-prefs-api';
import { fakeControlApi } from './helpers/fake-control-api';
import { fakeProjectApi, flush, tick } from './helpers/fake-project-api';
import { entry, fakeRecordingApi } from './helpers/fake-recording-api';

const MAIN = 'songs/main.js';
const A = `const meta = { title: 'Alpha' };\n$: s("a")\n`;
const SECTIONED = `const meta = { sections: [{ name: 'intro', bars: 2 }, { name: 'verse', bars: 2 }] };\n$: s("a")\n`;

/** A sidecar a memo plays with its song from; `tempoOverride` skips the static tempo check. */
function songSidecar(): Sidecar {
  return {
    schema: 1,
    createdAt: '2026-01-01T00:00:00Z',
    app: { version: '0.1.0' },
    input: { deviceId: 'mic', deviceName: 'Mic', channel: 0, channels: 1, sampleRate: 48000 },
    output: { baseLatency: 0.01, outputLatency: null },
    tempo: { bpm: 120, meter: '4/4', cps: 0.5, beatsPerBar: 4, tempoOverride: true },
    backing: {
      song: true,
      metronome: false,
      countIn: false,
      songPath: MAIN,
      startSection: null,
      loopSection: null,
    },
    timing: {
      bar1Frame: 0,
      bar1HostNs: '1',
      clockOffsetNs: { start: null, end: null },
      anchors: [],
      outputStamps: [],
    },
    xruns: 0,
    recovered: false,
  };
}

async function setup(opts: { last?: string | null; tauri?: boolean } = {}) {
  const { api, fake } = fakeProjectApi({
    '/p': { name: 'p', files: { [MAIN]: '$: s("a")\n', 'songs/x.js': A } },
    '/empty': { name: 'Empty', files: {} },
  });
  fake.last = opts.last === undefined ? '/p' : opts.last;
  const app = createApp(api, () => opts.tauri ?? true, null, null, null);
  const evaluated: string[] = [];
  app.runtime.stop = () => {}; // the real stop draws on a canvas happy-dom lacks
  app.runtime.evaluate = async (code: string) => (evaluated.push(code), true);
  const view = createEditor(document.body, '', app.runtime, app.hooks);
  app.attachView(view);
  await app.session.init();
  return { api, fake, app, view, evaluated };
}

describe('createApp', () => {
  test('a roll edit changes only the buffer and is one isolated undo step', async () => {
    const { app, view, fake, evaluated } = await setup();
    view.dispatch({ selection: { anchor: 3, head: 9 } });
    const snapshot = app.session.pianoRollSnapshot()!;
    expect(snapshot.selection).toEqual({ from: 3, to: 9 });
    const commit = app.session.editPianoRoll(snapshot, [{ from: 6, to: 7, insert: 'b' }]);
    expect(commit?.spans).toEqual([{ from: 6, to: 7 }]);
    expect(view.state.doc.toString()).toBe('$: s("b")\n');
    expect(get(app.session.store).songs[0].dirty).toBe(true);
    expect(fake.files().get(MAIN)).toBe('$: s("a")\n');
    expect(evaluated).toEqual([]);
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(snapshot.code);
    expect(app.session.editPianoRoll(snapshot, [{ from: 6, to: 7, insert: 'c' }])).toBeNull();
    app.destroy();
  });

  test('a roll snapshot is invalid after switching away and back', async () => {
    const { app, view } = await setup();
    view.dispatch({ selection: { anchor: 3, head: 9 } });
    const snapshot = app.session.pianoRollSnapshot()!;
    await app.session.switchSong('songs/x.js');
    await app.session.switchSong(MAIN);
    expect(app.session.pianoRollSnapshot()?.selection).toEqual(snapshot.selection);
    expect(app.session.editPianoRoll(snapshot, [{ from: 6, to: 7, insert: 'c' }])).toBeNull();
    app.destroy();
  });

  test('one caret is exposed and multiple ranges leave the roll without a target', async () => {
    const { app, view } = await setup();
    view.dispatch({ selection: { anchor: 6 } });
    expect(app.session.pianoRollSnapshot()?.selection).toEqual({ from: 6, to: 6 });
    const before = app.session.pianoRollSnapshot();
    view.dispatch({
      effects: StateEffect.appendConfig.of(EditorState.allowMultipleSelections.of(true)),
    });
    view.dispatch({
      selection: EditorSelection.create([EditorSelection.cursor(4), EditorSelection.cursor(6)]),
    });
    const after = app.session.pianoRollSnapshot();
    expect(after?.selection).toBeNull();
    expect(after?.revision).not.toBe(before?.revision);
    expect(after?.sourceRevision).toBe(before?.sourceRevision);
    app.destroy();
  });

  test('a roll edit reveals all changed spans and preserves the mapped selection and roll focus', async () => {
    const { app, view } = await setup();
    const captured: { otherEffects: number; spans: number | undefined }[] = [];
    view.dispatch({
      selection: { anchor: 6 },
      effects: StateEffect.appendConfig.of(
        EditorViewExtension.updateListener.of((update) => {
          for (const tr of update.transactions) {
            if (!tr.isUserEvent('input.pianoRoll')) continue;
            captured.push({
              otherEffects: tr.effects.filter((effect) => !effect.is(pianoRollEditLocation)).length,
              spans: tr.effects.find((effect) => effect.is(pianoRollEditLocation))?.value.length,
            });
          }
        }),
      ),
    });
    const focused = document.createElement('button');
    document.body.append(focused);
    focused.focus();
    const snapshot = app.session.pianoRollSnapshot()!;
    const commit = app.session.editPianoRoll(
      snapshot,
      [
        { from: 3, to: 3, insert: '(' },
        { from: 9, to: 9, insert: ').fast(2)' },
      ],
      { from: 3, to: 9 },
    );
    expect(commit?.spans).toEqual([
      { from: 3, to: 4 },
      { from: 10, to: 19 },
    ]);
    expect(commit?.target).toEqual({ from: 3, to: 19 });
    expect(commit?.snapshot.selection).toEqual({ from: 7, to: 7 });
    expect(captured).toEqual([{ otherEffects: 1, spans: 2 }]);
    expect(view.state.field(pianoRollEditHighlight).size).toBe(2);
    expect(document.activeElement).toBe(focused);
    app.session.clearPianoRollFeedback();
    expect(view.state.field(pianoRollEditHighlight).size).toBe(0);
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(snapshot.code);
    expect(view.state.selection.main.head).toBe(6);
    expect(redo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(commit!.snapshot.code);
    expect(view.state.field(pianoRollEditHighlight).size).toBe(0);
    focused.remove();
    app.destroy();
  });

  test.each([
    ['ASI semicolon', 'const lead = note(60)\nlead', ';('],
    ['shorthand property', 'const lead = note(60)\nsong(meta, { lead })', 'lead: ('],
  ])(
    'retains the saved event scope after rewriting %s syntax',
    async (_name, code, addedPrefix) => {
      const { app, view } = await setup();
      const from = code.lastIndexOf('lead');
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: code },
        selection: { anchor: from, head: from + 4 },
      });
      const snapshot = app.session.pianoRollSnapshot()!;
      const authored = resolveRollTarget(snapshot.code, snapshot.selection!)!;
      const [note] = preparePhraseEvents(authored.phrase!.notes, authored.phrase!.bars);
      const scope = { kind: 'expression', expression: authored.expression } as const;
      const patches = planEventEdit(code, scope, note!, {
        kind: 'pitch',
        id: note!.id,
        pitch: 64,
      })!;
      expect(patches[0]?.insert).toBe(addedPrefix);

      const result = app.session.editPianoRoll(snapshot, patches, authored.expression)!;
      const inside = from + addedPrefix.length;
      expect(result.targetCaret).toBe(inside);
      expect(result.snapshot.selection).toEqual({
        from: inside,
        to: inside + 4,
      });
      expect(resolveRollTarget(result.snapshot.code, result.target!)).toBeNull();
      const wrapped = resolvePostEditTarget(result, true);
      expect(wrapped?.phrase).toBeUndefined();
      expect(result.snapshot.code.slice(wrapped!.expression.from, wrapped!.expression.to)).toMatch(
        /^\(lead\)\._pianoEdits\(/,
      );
      expect(resolvePostEditTarget(result, false)).toBeNull();

      const repeated = planEventEdit(
        result.snapshot.code,
        { ...scope, expression: wrapped!.expression },
        note!,
        {
          kind: 'pitch',
          id: note!.id,
          pitch: 65,
        },
      );
      expect(repeated).toHaveLength(1);
      expect(repeated?.[0]?.insert).toContain('key: 65');
      expect(repeated?.[0]?.insert).not.toContain('._pianoEdits(');
      app.destroy();
    },
  );

  test('a changed instrument snapshot invalidates live piano roll parity', async () => {
    const { app, fake } = await setup();
    await app.session.evaluateBuffer(MAIN, { autostart: false });
    expect(app.session.pianoRollMatchesLivePins()).toBe(true);
    fake.instrumentSnapshots.set('/p', {
      lead: {
        schema: 1,
        parameters: {},
        voices: [{ name: 'voice', chain: [{ control: 's', value: 'triangle' }] }],
      },
    });
    fake.emit({ instruments: true });
    await flush();
    expect(app.session.pianoRollMatchesLivePins()).toBe(false);
    await app.session.evaluateBuffer(MAIN, { autostart: false });
    expect(app.session.pianoRollMatchesLivePins()).toBe(true);
    app.destroy();
  });

  test('init reopens the last project into the attached view and registers the close guard', async () => {
    const { app, view, fake } = await setup();
    expect(get(app.session.store).project?.name).toBe('p');
    expect(view.state.doc.toString()).toBe('$: s("a")\n');
    expect(fake.closeHandlers).toHaveLength(1);
  });

  test('a view attached after init shows the active song', async () => {
    const { api, fake } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    fake.last = '/p';
    const app = createApp(api, () => true, null, null, null);
    app.runtime.stop = () => {};
    await app.session.init();
    const view = createEditor(document.body, '', app.runtime, app.hooks);
    app.attachView(view);
    expect(view.state.doc.toString()).toBe(A);
  });

  test('onDocChange marks the song dirty', async () => {
    const { app, view } = await setup();
    view.dispatch({ changes: { from: 0, insert: '// x\n' } });
    expect(get(app.session.store).songs[0].dirty).toBe(true);
  });

  test('onSave writes the active song', async () => {
    const { app, view, fake } = await setup();
    view.dispatch({ changes: { from: 0, insert: '// x\n' } });
    app.hooks.onSave!();
    await tick();
    expect(fake.files().get(MAIN)).toBe('// x\n$: s("a")\n');
    expect(get(app.session.store).songs[0].dirty).toBe(false);
  });

  test('onEvaluate and evaluateDoc go through saveAndEvaluate', async () => {
    const { app, view, fake, evaluated } = await setup();
    const saves = () => fake.calls.filter((c) => c.startsWith('save:'));
    view.dispatch({ changes: { from: 0, insert: '// x\n' } });
    app.hooks.onEvaluate!();
    await tick();
    expect(saves()).toHaveLength(1);
    expect(evaluated).toEqual(['// x\n$: s("a")\n']);
    view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    app.evaluateDoc();
    await tick();
    expect(saves()).toHaveLength(2);
    expect(evaluated[1]).toBe('// y\n// x\n$: s("a")\n');
  });
});

/** The app with the song playback swapped for one that logs, playing or not. */
function logged(app: Awaited<ReturnType<typeof setup>>['app'], started: boolean, arms = true) {
  const calls: string[] = [];
  (app.session as any).playback = {
    store: writable({ started }),
    arm: (choice: unknown) => (calls.push(`arm ${JSON.stringify(choice)}`), arms),
  };
  app.session.saveAndEvaluate = async () => void calls.push('evaluate');
  return calls;
}

describe('playFrom', () => {
  test('while stopped it arms, then evaluates in the same call', async () => {
    const { app } = await setup();
    const calls = logged(app, false);
    app.playFrom({ start: 'verse' });
    expect(calls).toEqual(['arm {"start":"verse"}', 'evaluate']);
  });

  test('while stopped a choice that does not resolve evaluates nothing', async () => {
    const { app } = await setup();
    const calls = logged(app, false, false);
    app.playFrom({ loop: 'nope' });
    expect(calls).toEqual(['arm {"loop":"nope"}']);
  });

  test('while playing it arms only', async () => {
    const { app } = await setup();
    const calls = logged(app, true);
    app.playFrom({ loop: 'verse', through: 'chorus' });
    expect(calls).toEqual(['arm {"loop":"verse","through":"chorus"}']);
  });

  test('the real song playback arms the header section, then the evaluate runs', async () => {
    const { app, evaluated } = await setup();
    app.session.externalEdit(MAIN, SECTIONED);
    app.playFrom({ start: 'verse' });
    expect(get(app.session.playback.store).transport.armed).toBe('verse');
    await tick();
    expect(evaluated).toEqual([SECTIONED]);
  });
});

describe('window title', () => {
  test('no project is strudeland', async () => {
    const { fake } = await setup({ last: null });
    expect(fake.titles).toEqual(['strudeland']);
  });

  test('project and song, with a dot while dirty; only set on change', async () => {
    const { fake, view, app } = await setup();
    expect(fake.titles).toEqual(['strudeland', 'p — main']);
    view.dispatch({ changes: { from: 0, insert: 'x' } });
    expect(fake.titles.at(-1)).toBe('p — main ●');
    view.dispatch({ changes: { from: 0, insert: 'y' } });
    expect(fake.titles).toHaveLength(3);
    await app.session.save();
    expect(fake.titles.at(-1)).toBe('p — main');
  });

  test('follows a switch and a meta title edit', async () => {
    const { fake, view, app } = await setup();
    view.dispatch({ changes: { from: 0, insert: `const meta = { title: 'Beta' };\n` } });
    expect(fake.titles.at(-1)).toBe('p — Beta ●');
    await app.session.switchSong('songs/x.js');
    expect(fake.titles.at(-1)).toBe('p — Alpha');
  });

  test('zero songs is the project name', async () => {
    const { fake } = await setup({ last: '/empty' });
    expect(fake.titles.at(-1)).toBe('Empty');
  });

  test('a clean reload does not flash the dirty dot', async () => {
    const { fake } = await setup();
    await flush();
    const before = fake.titles.length;
    fake.files().set(MAIN, '$: s("b")\n');
    fake.emit({ changed: [MAIN] });
    await flush();
    expect(fake.titles.slice(before)).toEqual([]);
  });

  test('a manifest rename updates it', async () => {
    const { fake } = await setup();
    await flush();
    fake.disk.get('/p')!.name = 'Renamed';
    fake.emit({ manifest: true });
    await flush();
    expect(fake.titles.at(-1)).toBe('Renamed — main');
  });

  test('a failing setWindowTitle is logged, not thrown', async () => {
    const { api, fake } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    fake.last = '/p';
    api.setWindowTitle = async () => {
      throw 'no window';
    };
    const errors: unknown[] = [];
    const log = console.error;
    console.error = (e: unknown) => void errors.push(e);
    try {
      const app = createApp(api, () => true, null, null, null);
      app.runtime.stop = () => {};
      await app.session.init();
      await tick();
    } finally {
      console.error = log;
    }
    expect(errors).toContain('no window');
  });

  test('outside Tauri the title is never set', async () => {
    const { fake, view } = await setup({ tauri: false });
    view.dispatch({ changes: { from: 0, insert: 'x' } });
    expect(fake.titles).toEqual([]);
    expect(fake.calls).toEqual([]);
  });
});

describe('control socket', () => {
  async function withControl(info?: Parameters<typeof fakeControlApi>[0]) {
    const { api, fake } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    fake.last = '/p';
    const control = fakeControlApi(info);
    const app = createApp(api, () => true, control.api, null, null);
    app.runtime.stop = () => {};
    await app.session.init();
    await flush();
    return { app, control: control.fake, fake };
  }

  test('without a ControlApi it stays off', async () => {
    const { app } = await setup();
    await flush();
    expect(get(app.control)).toEqual({ status: 'off', path: null, detail: null });
    app.destroy(); // a no-op
  });

  test('outside Tauri the default is no ControlApi', async () => {
    const { api } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    const app = createApp(api, () => false);
    await flush();
    expect(get(app.control)).toEqual({ status: 'off', path: null, detail: null });
  });

  test('destroy unlistens once, and is idempotent', async () => {
    const { app, control } = await withControl();
    expect(control.listening).toBe(true);
    app.destroy();
    app.destroy();
    expect(control.unlistens).toBe(1);
    expect(control.listening).toBe(false);
    expect(control.log).toEqual(['onRequest', 'attach', 'unlisten']);
  });

  test('destroy before the listener is set up unlistens when it is, without attaching', async () => {
    const { api } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    const control = fakeControlApi();
    const app = createApp(api, () => true, control.api, null, null);
    app.destroy();
    await flush();
    expect(control.fake.log).toEqual(['onRequest', 'unlisten']);
    expect(control.fake.listening).toBe(false);
    app.destroy();
    expect(control.fake.unlistens).toBe(1);
  });

  test('subscribes, then attaches, and publishes the attach result', async () => {
    const info = { status: 'in-use' as const, path: '/tmp/x.sock', detail: null };
    const { app, control } = await withControl(info);
    expect(control.log).toEqual(['onRequest', 'attach']);
    expect(control.attaches).toBe(1);
    expect(get(app.control)).toEqual(info);
  });

  test('answers a request through reply', async () => {
    const { control } = await withControl();
    expect(await control.send({ id: 7, method: 'code.get', params: {} })).toEqual({
      result: { song: MAIN, title: 'Alpha', code: A, dirty: false },
    });
    expect(await control.send({ id: 8, method: 'eval', params: { song: '' } })).toEqual({
      error: {
        code: ErrorCode.INVALID_PARAMS,
        message: `invalid params: 'song' must be a non-empty string`,
      },
    });
    expect(control.log.slice(2)).toEqual(['reply:7', 'reply:8']);
  });

  test('a rejecting attach sets disabled and logs', async () => {
    const { api, fake } = fakeProjectApi({ '/p': { name: 'p', files: { [MAIN]: A } } });
    const control = fakeControlApi();
    control.fake.failAttach = new Error('no bridge');
    const errors: unknown[] = [];
    const log = console.error;
    console.error = (e: unknown) => void errors.push(e);
    try {
      const app = createApp(api, () => true, control.api, null, null);
      await flush();
      expect(get(app.control)).toEqual({ status: 'disabled', path: null, detail: 'no bridge' });
    } finally {
      console.error = log;
    }
    expect(errors).toHaveLength(1);
    expect(fake.calls).toEqual([]);
  });

  test('a rejecting reply is only logged; later requests are still answered', async () => {
    const { control } = await withControl();
    control.failReply = 'webview gone';
    const errors: unknown[] = [];
    const log = console.error;
    console.error = (e: unknown) => void errors.push(e);
    try {
      await control.send({ id: 1, method: 'stop', params: {} });
      await flush();
      control.failReply = null;
      expect(await control.send({ id: 2, method: 'stop', params: {} })).toEqual({
        result: { wasPlaying: false },
      });
    } finally {
      console.error = log;
    }
    expect(errors).toEqual(['webview gone']);
  });
});

describe('scope clock', () => {
  test('follows runtime started: a stop rests subscribers once', async () => {
    const { app } = await setup();
    const reads: Float32Array[] = [];
    app.runtime.readScope = (buf) => (reads.push(buf), false);
    const calls: (Float32Array | null)[] = [];
    app.scopeClock.subscribe((b) => void calls.push(b));
    // Not started yet: the clock does not run, so nothing rests.
    expect(get(app.runtime.store).started).toBe(false);
    app.destroy();
    expect(calls).toEqual([]);

    const { app: app2 } = await setup();
    const calls2: (Float32Array | null)[] = [];
    app2.scopeClock.subscribe((b) => void calls2.push(b));
    // The scheduler's own toggle, as a start and stop drive it. The drawer it also toggles
    // queries the pattern and clears a canvas, which happy-dom has no 2D context for.
    const proto = HTMLCanvasElement.prototype as any;
    const getContext = proto.getContext;
    proto.getContext = () => ({ clearRect: () => {}, canvas: { width: 0, height: 0 } });
    try {
      app2.runtime.scheduler.pattern = { queryArc: () => [] };
      app2.runtime.scheduler.setStarted(true);
      expect(get(app2.runtime.store).started).toBe(true);
      expect(calls2).toEqual([]);
      app2.runtime.scheduler.setStarted(false);
      expect(calls2).toEqual([null]);
    } finally {
      proto.getContext = getContext;
    }
    app2.destroy();
    expect(calls2).toEqual([null]);
  });

  test('destroy stops the clock once', async () => {
    const { app } = await setup();
    const calls: (Float32Array | null)[] = [];
    app.scopeClock.setRunning(true);
    app.scopeClock.subscribe((b) => void calls.push(b));
    app.destroy();
    expect(calls).toEqual([null]);
    app.destroy();
    expect(calls).toEqual([null]);
  });

  test('destroy disposes the clock once', async () => {
    const { app } = await setup();
    const dispose = app.scopeClock.dispose;
    let disposals = 0;
    app.scopeClock.dispose = () => (disposals++, dispose());
    app.destroy();
    app.destroy();
    expect(disposals).toBe(1);
    // A disposed clock takes no subscribers, so it never runs again.
    const calls: (Float32Array | null)[] = [];
    app.scopeClock.subscribe((b) => void calls.push(b));
    app.scopeClock.setRunning(true);
    app.scopeClock.setRunning(false);
    expect(calls).toEqual([]);
  });
});

describe('recording and memos', () => {
  async function withRecording() {
    const { api, fake } = fakeProjectApi({
      '/p': { name: 'p', files: { [MAIN]: '$: s("a")\n' } },
      '/q': { name: 'q', files: { [MAIN]: '$: s("b")\n' } },
    });
    fake.last = '/p';
    const rec = fakeRecordingApi({ roots: ['/p', '/q'] });
    rec.fake.memos.set('/p', [entry('take-01')]);
    rec.fake.memos.set('/q', [entry('other-01')]);
    const app = createApp(api, () => true, null, rec.api, null);
    app.runtime.stop = () => {};
    app.runtime.evaluate = async () => true;
    await app.session.init();
    await flush();
    return { app, fake, rec };
  }

  test('without a RecordingApi neither is wired', async () => {
    const { app } = await setup();
    expect(app.recording).toBeUndefined();
    expect(app.memos).toBeUndefined();
  });

  test('wires the controller and lists the open project', async () => {
    const { app, rec } = await withRecording();
    expect(app.recording).toBeDefined();
    expect(get(app.memos!.store).items.map((m) => m.stem)).toEqual(['take-01']);
    expect(rec.fake.listeners()).toEqual({ state: 1, level: 1 });
    // A project open arms the microphone.
    expect(rec.fake.names()).toContain('arm');
    expect(get(app.recording!.store).phase).toBe('armed');
  });

  test('a project switch stops the memo and the take first, then lists the new project', async () => {
    const { app, fake, rec } = await withRecording();
    const order: string[] = [];
    const stopPlayback = app.memos!.stopPlayback;
    app.memos!.stopPlayback = () => (order.push('stopPlayback'), stopPlayback());
    const stop = app.recording!.stop;
    app.recording!.stop = () => (order.push('recording.stop'), stop());
    fake.picks.push('/q');
    await app.session.openProject();
    await flush();
    expect(order).toEqual(['stopPlayback', 'recording.stop']);
    expect(get(app.session.store).project?.root).toBe('/q');
    expect(get(app.memos!.store).items.map((m) => m.stem)).toEqual(['other-01']);
    expect(rec.fake.argsOf('list').at(-1)).toEqual(['/q']);
  });

  test('project changes, recording and play requests retire an instrument preview immediately', async () => {
    const { app, fake } = await withRecording();
    const stop = app.instrumentPreview.stop;
    let stops = 0;
    app.instrumentPreview.stop = () => {
      stops++;
      stop();
    };
    app.evaluateDoc();
    expect(stops).toBeGreaterThan(0);
    stops = 0;
    const recording = app.recording!.record({ song: false, metronome: false, countIn: false });
    expect(stops).toBeGreaterThan(0);
    await recording;
    await app.recording!.stop();
    stops = 0;
    fake.picks.push('/q');
    await app.session.openProject();
    expect(stops).toBeGreaterThan(0);
    app.destroy();
  });

  test('during a take Cmd-Enter and evaluateDoc do nothing, as the disabled play button', async () => {
    const { app } = await withRecording();
    const calls: string[] = [];
    app.session.saveAndEvaluate = async () => void calls.push('saveAndEvaluate');
    const stopPlayback = app.memos!.stopPlayback;
    app.memos!.stopPlayback = () => (calls.push('stopPlayback'), stopPlayback());
    await app.recording!.record({ song: false, metronome: false, countIn: false });
    await flush();
    expect(get(app.recording!.store).phase).toBe('recording');
    app.hooks.onEvaluate!();
    app.evaluateDoc();
    expect(calls).toEqual([]);
    await app.recording!.stop();
    await flush();
    app.hooks.onEvaluate!();
    expect(calls).toEqual(['stopPlayback', 'saveAndEvaluate']);
  });

  /** Run the editor's binding for `key`; returns how many dispatches it made (a flash is one). */
  function press(view: EditorView, key: string): number {
    const run = view.state
      .facet(keymap)
      .flat()
      .find((b) => b.key === key)!.run!;
    const dispatch = view.dispatch;
    let count = 0;
    view.dispatch = ((...a: unknown[]) => {
      count++;
      (dispatch as (...a: unknown[]) => void).apply(view, a);
    }) as EditorView['dispatch'];
    try {
      expect(run(view)).toBe(true);
    } finally {
      view.dispatch = dispatch;
    }
    return count;
  }

  /**
   * Start and stop the scheduler as a start and stop drive it. The drawer it also toggles
   * queries the pattern and clears a canvas, which happy-dom has no 2D context for.
   */
  function schedulerStarted(app: Awaited<ReturnType<typeof withRecording>>['app'], on: boolean) {
    const proto = HTMLCanvasElement.prototype as any;
    const getContext = proto.getContext;
    proto.getContext = () => ({ clearRect: () => {}, canvas: { width: 0, height: 0 } });
    try {
      app.runtime.scheduler.pattern = { queryArc: () => [] };
      app.runtime.scheduler.setStarted(on);
    } finally {
      proto.getContext = getContext;
    }
  }

  test('during a take Cmd-Enter neither flashes nor evaluates', async () => {
    const { app } = await withRecording();
    const view = createEditor(document.body, '', app.runtime, app.hooks);
    const calls: string[] = [];
    app.session.saveAndEvaluate = async () => void calls.push('saveAndEvaluate');
    app.runtime.resumeAudio = async () => true;
    await app.recording!.record({ song: false, metronome: false, countIn: false });
    await flush();
    expect(get(app.recording!.store).phase).toBe('recording');
    expect(press(view, 'Mod-Enter')).toBe(0);
    expect(calls).toEqual([]);
    await app.recording!.stop();
    await flush();
    expect(press(view, 'Mod-Enter')).toBe(1);
    expect(calls).toEqual(['saveAndEvaluate']);
    view.destroy();
  });

  test('during a take with song backing Cmd-. ends the take, which then stops the song', async () => {
    const { app } = await withRecording();
    const view = createEditor(document.body, '', app.runtime, app.hooks);
    const calls: string[] = [];
    app.runtime.resumeAudio = async () => true;
    app.session.playback.start = async () => {
      schedulerStarted(app, true);
      return { T: 0, cps: 0.5 };
    };
    app.runtime.stop = () => {
      calls.push('runtime.stop');
      schedulerStarted(app, false);
    };
    const stop = app.recording!.stop;
    await app.recording!.record({ song: true, metronome: false, countIn: false });
    await flush();
    expect(get(app.recording!.store).phase).toBe('recording');
    expect(get(app.runtime.store).started).toBe(true);
    // The take owns the stop: the arbiter stops neither the memo nor the song; the take's own
    // stop silences its backing.
    app.recording!.stop = () => {
      calls.push('recording.stop');
      expect(get(app.runtime.store).started).toBe(true);
      return stop();
    };
    press(view, 'Mod-.');
    expect(calls).toEqual(['recording.stop', 'runtime.stop']);
    expect(get(app.runtime.store).started).toBe(false);
    await flush();
    expect(get(app.recording!.store).phase).not.toBe('recording');
    expect(calls).toEqual(['recording.stop', 'runtime.stop']);
    view.destroy();
  });

  test('during a memo with song backing Cmd-. stops the memo and the song', async () => {
    const { app, rec } = await withRecording();
    rec.fake.memos.set('/p', [
      entry('take-02', { hasSidecar: true, sidecar: songSidecar() }),
      entry('take-01'),
    ]);
    await app.memos!.refresh();
    const view = createEditor(document.body, '', app.runtime, app.hooks);
    const calls: string[] = [];
    // The memo loads until the audio resumes; the song backing is already playing.
    app.runtime.resumeAudio = () => new Promise<boolean>(() => {});
    app.runtime.stop = () => {
      calls.push('runtime.stop');
      schedulerStarted(app, false);
    };
    void app.memos!.play('take-02', 'song');
    schedulerStarted(app, true);
    expect(get(app.memos!.player).status).toBe('loading');
    expect(get(app.arbiter.store)).toMatchObject({ stopTarget: 'memo', canPlayFrom: false });
    press(view, 'Mod-.');
    expect(get(app.memos!.player).status).toBe('idle');
    expect(calls).toEqual(['runtime.stop']);
    expect(get(app.runtime.store).started).toBe(false);
    view.destroy();
  });

  test('a take with song backing saves the section armed through the song playback', async () => {
    const { app, rec } = await withRecording();
    app.runtime.resumeAudio = async () => true;
    app.session.externalEdit(MAIN, SECTIONED);
    app.session.playback.arm({ loop: 'verse' });
    await app.recording!.record({ song: true, metronome: false, countIn: false });
    expect((rec.fake.argsOf('start')[0]![0] as any).fields.backing).toMatchObject({
      startSection: null,
      loopSection: 'verse',
    });
    await app.recording!.stop();
  });

  test('destroy unlistens the recording events and disarms', async () => {
    const { app, rec } = await withRecording();
    app.destroy();
    await flush();
    expect(rec.fake.listeners()).toEqual({ state: 0, level: 0 });
    expect(rec.fake.names().at(-1)).toBe('disarm');
    app.destroy();
    await flush();
    expect(rec.fake.names().filter((n) => n === 'disarm')).toHaveLength(1);
  });
});

describe('vim keybindings', () => {
  const cmOf = (view: EditorView) => vimAdapter(view)!;

  /** An app whose prefs load waits until `resolve` (vim on in the prefs file). */
  async function withPrefs(initial: Partial<Prefs> = { vim: true }) {
    const { api, fake } = fakeProjectApi({
      '/p': { name: 'p', files: { [MAIN]: '$: s("a")\n', 'songs/x.js': A } },
    });
    fake.last = '/p';
    const prefs = fakePrefsApi(initial);
    const gate = prefs.deferGet();
    const app = createApp(api, () => true, null, null, prefs);
    app.runtime.stop = () => {};
    const view = createEditor(document.body, '', app.runtime, app.hooks);
    const focused: string[] = [];
    view.focus = () => void focused.push('focus');
    app.attachView(view);
    await app.session.init();
    return { app, view, fake, prefs, gate, focused };
  }

  test('ready resolves after the prefs load; the shown view then has vim, without focus', async () => {
    const { app, view, gate, focused } = await withPrefs();
    let ready = false;
    void app.ready.then(() => (ready = true));
    await flush();
    expect(ready).toBe(false);
    expect(getCM(view)).toBeNull();
    expect(get(app.vimMode.store)).toBeNull();
    gate.resolve();
    await app.ready;
    expect(ready).toBe(true);
    expect(getCM(view)).not.toBeNull();
    expect(view.state.doc.toString()).toBe('$: s("a")\n');
    expect(get(app.vimMode.store)).toEqual({ mode: 'normal' });
    expect(focused).toEqual([]);
    app.destroy();
  });

  test('a later toggle focuses the editor and the mode store follows', async () => {
    const { app, view, gate, focused } = await withPrefs();
    gate.resolve();
    await app.ready;
    Vim.handleKey(cmOf(view), 'i', 'user');
    expect(get(app.vimMode.store)).toEqual({ mode: 'insert' });
    await app.toggleVim();
    expect(getCM(view)).toBeNull();
    expect(get(app.vimMode.store)).toBeNull();
    expect(focused).toEqual(['focus']);
    await app.toggleVim();
    expect(get(app.vimMode.store)).toEqual({ mode: 'normal' });
    expect(focused).toEqual(['focus', 'focus']);
    app.destroy();
  });

  test('a song shown after the load gets vim, and the mode store re-attaches to it', async () => {
    const { app, view, gate } = await withPrefs();
    gate.resolve();
    await app.ready;
    await app.session.switchSong('songs/x.js');
    expect(view.state.doc.toString()).toBe(A);
    expect(getCM(view)).not.toBeNull();
    Vim.handleKey(cmOf(view), 'v', 'user');
    expect(get(app.vimMode.store)).toEqual({ mode: 'visual' });
    app.destroy();
  });

  test('with vim off in the prefs nothing changes on load', async () => {
    const { app, view, gate, focused } = await withPrefs({ vim: false });
    const state = view.state;
    gate.resolve();
    await app.ready;
    expect(view.state).toBe(state);
    expect(getCM(view)).toBeNull();
    expect(focused).toEqual([]);
    app.destroy();
  });

  test('a failed load keeps vim off and ready still resolves', async () => {
    const { app, view, prefs, gate } = await withPrefs();
    prefs.failGet = true;
    const log = console.error;
    console.error = () => {};
    try {
      gate.resolve();
      await app.ready;
    } finally {
      console.error = log;
    }
    expect(getCM(view)).toBeNull();
    expect(app.prefs.vim()).toBe(false);
    app.destroy();
  });

  test(':w saves through this app; destroy restores the no-op and releases the subscription', async () => {
    const { app, view, fake, gate } = await withPrefs();
    gate.resolve();
    await app.ready;
    const saves = () => fake.calls.filter((c) => c.startsWith('save:')).length;
    view.dispatch({ changes: { from: 0, insert: '// x\n' } });
    Vim.handleEx(cmOf(view), 'w');
    await tick();
    expect(saves()).toBe(1);
    expect(fake.files().get(MAIN)).toBe('// x\n$: s("a")\n');

    app.destroy();
    view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    Vim.handleEx(cmOf(view), 'w');
    await tick();
    expect(saves()).toBe(1);
    expect(get(app.vimMode.store)).toBeNull();
    // The prefs subscription is gone: a toggle no longer touches the view.
    await app.prefs.setVim(false);
    expect(getCM(view)).not.toBeNull();
    app.destroy(); // idempotent
  });

  test('a failed save surfaces as the prefs error, cleared by the next good save', async () => {
    const { app, prefs, gate, focused } = await withPrefs();
    gate.resolve();
    await app.ready;
    prefs.failSet = true;
    await app.prefs.setVim(false);
    expect(get(app.prefs.store).error).toBe('write failed');
    expect(app.prefs.vim()).toBe(false);
    prefs.failSet = false;
    await app.prefs.setVim(true);
    expect(get(app.prefs.store).error).toBeNull();
    expect(prefs.saved).toEqual([
      { vim: true, auditionNotes: false, formatOnSave: false, pianoRollOpen: false },
    ]);
    // Only the user's toggle focuses; a pref change by itself does not.
    expect(focused).toEqual([]);
    app.destroy();
  });

  test('Vim write uses the loaded formatting preference', async () => {
    const { app, view, gate, fake } = await withPrefs({ vim: true, formatOnSave: true });
    gate.resolve();
    await app.ready;
    const steps = Array(32).fill('bd').join(' ');
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: `$: s("${steps}").gain(.8)` },
    });
    Vim.handleEx(cmOf(view), 'w');
    await tick();
    const formatted = view.state.doc.toString();
    expect(formatted).toContain('s(`\n');
    expect(formatted).toEndWith('\n`)\n  .gain(.8)');
    expect(fake.files().get(MAIN)).toBe(formatted);
    app.destroy();
  });

  test('attachView(undefined) sets the mode store to null', async () => {
    const { app, gate } = await withPrefs();
    gate.resolve();
    await app.ready;
    expect(get(app.vimMode.store)).toEqual({ mode: 'normal' });
    app.attachView(undefined);
    expect(get(app.vimMode.store)).toBeNull();
    app.destroy();
  });
});

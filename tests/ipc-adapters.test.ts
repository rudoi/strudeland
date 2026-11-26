import { describe, expect, test } from 'bun:test';
import { errorMessage } from '../src/lib/errors';
import { mockTauriCore, mockTauriModule } from './helpers/tauri-core-mock';

// One guard per adapter: a rejection is an `Error` with the command's message, so a raw object
// never reaches `errorMessage`.

const core = mockTauriCore();
let unlistened = 0;
mockTauriModule('@tauri-apps/api/event', {
  listen: async () => () => void unlistened++,
});
mockTauriModule('@tauri-apps/plugin-opener', { openUrl: async () => {} });

const { createTauriPrefsApi } = await import('../src/lib/prefs/api');
const { inspectPianoRoll } = await import('../src/lib/piano-roll/api');
const { exportApi, setExportMenuAvailable } = await import('../src/lib/export/api');
const { ExportError } = await import('../src/lib/export/error');
const { createTauriControlApi } = await import('../src/lib/control/api');
const { getAppInfo } = await import('../src/lib/info');
const { listenForAppQuit, replyToAppQuit } = await import('../src/lib/app-quit');
const { CommandError } = await import('../src/lib/ipc/command-error');

async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  throw new Error('expected a rejection');
}

/** `call` rejects with an Error whose message is `message`, given the raw `value`. */
async function expectMessage(call: () => Promise<unknown>, value: unknown, message: string) {
  core.reject(value);
  const e = await rejection(call());
  expect(e).toBeInstanceOf(Error);
  expect(e).toBeInstanceOf(CommandError);
  expect((e as Error).message).toBe(message);
  expect(errorMessage(e)).toBe(message);
  return e;
}

describe('prefs', () => {
  test('get and set reject with the command message', async () => {
    const api = createTauriPrefsApi();
    await expectMessage(() => api.get(), { kind: 'other', message: 'no data dir' }, 'no data dir');
    const prefs = { vim: true, auditionNotes: false, formatOnSave: false, pianoRollOpen: false };
    await expectMessage(() => api.set(prefs), { kind: 'other', message: 'disk' }, 'disk');
    expect(core.calls).toEqual([{ cmd: 'prefs_set', args: { prefs } }]);
  });
});

describe('piano roll', () => {
  test('a busy inspection keeps its message', async () => {
    const message = 'A piano roll preview is already running.';
    const e = await expectMessage(
      () => inspectPianoRoll({ root: '/r' } as never),
      { kind: 'busy', message },
      message,
    );
    expect(e).toMatchObject({ kind: 'busy' });
    expect(core.calls).toEqual([{ cmd: 'piano_roll_inspect', args: { request: { root: '/r' } } }]);
  });
});

describe('export', () => {
  test('the written paths survive as ExportError.written', async () => {
    const raw = { kind: 'other', message: 'destination full', written: ['/out/a.wav'] };
    const e = await expectMessage(
      () => exportApi.start({} as never, 'src', [], () => {}),
      raw,
      'destination full',
    );
    expect(e).toBeInstanceOf(ExportError);
    expect((e as InstanceType<typeof ExportError>).written).toEqual(['/out/a.wav']);
    expect(core.calls.map((c) => c.cmd)).toEqual(['export_audio_start']);
  });

  test('pick, preview and the menu reject with the command message', async () => {
    const stale = { kind: 'projectChanged', message: 'the project changed', written: [] };
    const e = await expectMessage(
      () => exportApi.preview({} as never),
      stale,
      'the project changed',
    );
    expect(e).toMatchObject({ kind: 'projectChanged', written: [] });
    await expectMessage(
      () => exportApi.pickFolder(),
      { kind: 'busy', message: 'b', written: [] },
      'b',
    );
    await expectMessage(
      () => setExportMenuAvailable(true),
      { kind: 'other', message: 'no application menu' },
      'no application menu',
    );
    expect(core.calls).toEqual([{ cmd: 'export_audio_set_available', args: { available: true } }]);
  });
});

describe('control', () => {
  test('attach and reply reject with the IPC message', async () => {
    const api = createTauriControlApi();
    await expectMessage(() => api.attach(), 'ipc down', 'ipc down');
    await expectMessage(() => api.reply(1, { result: null } as never), new Error('gone'), 'gone');
    expect(core.calls).toEqual([
      { cmd: 'control_reply', args: { id: 1, reply: { result: null } } },
    ]);
  });
});

describe('info', () => {
  test('getAppInfo still resolves null on a rejection and outside Tauri', async () => {
    const error = console.error;
    console.error = () => {};
    try {
      core.reject({ kind: 'other', message: 'bad pins' });
      expect(await getAppInfo()).toBeNull();
    } finally {
      console.error = error;
    }
    const info = { version: '1', strudel: { commit: 'c', date: 'd', kabelsalat: {} } };
    core.answer(() => info);
    expect(await getAppInfo()).toEqual(info);
    core.setTauri(false);
    try {
      core.answer(() => info);
      expect(await getAppInfo()).toBeNull();
      expect(core.calls).toEqual([]);
    } finally {
      core.setTauri(true);
    }
  });
});

describe('app quit', () => {
  test('listen and reply reject with the command message', async () => {
    const before = unlistened;
    await expectMessage(() => listenForAppQuit(() => {}), 'not attached', 'not attached');
    expect(unlistened).toBe(before + 1);
    await expectMessage(() => replyToAppQuit(3, true), 'no request', 'no request');
    expect(core.calls).toEqual([{ cmd: 'app_quit_reply', args: { id: 3, approved: true } }]);
  });
});

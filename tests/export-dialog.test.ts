import { describe, expect, mock, test } from 'bun:test';
import { get } from 'svelte/store';
import type { ExportApi, ExportProgress, ExportResult, ExportTarget } from '../src/lib/export/api';
import {
  captureExportSong,
  createExportDialog,
  showMp3Warning,
} from '../src/lib/export/controller';
import type { SessionState } from '../src/lib/project/session';
import { buildStructure } from '../src/lib/strudel/structure';

const structure = buildStructure({
  sections: [
    { name: 'lead', bars: 2 },
    { name: 'body', bars: 4, loop: true },
  ],
});
const editor = {
  root: '/works',
  songPath: 'songs/main.js',
  title: 'main',
  source: 'unsaved editor text',
  dirty: true,
  structure,
};
const audio = { role: 'single', path: '/works/renders/main.wav', exists: false } as const;
const opening = { role: 'intro', path: '/works/renders/main-intro.wav', exists: true } as const;
const loop = { role: 'loop', path: '/works/renders/main-loop.wav', exists: false } as const;
const result = {
  files: [{ role: 'loop', path: loop.path, godotLoopOffset: 0 }],
  warnings: [],
} satisfies ExportResult;

function setup(source = editor) {
  const api = {
    preview: mock<ExportApi['preview']>(async () => [audio]),
    pickFolder: mock<ExportApi['pickFolder']>(async () => null),
    start: mock<ExportApi['start']>(async () => result),
  };
  const dialog = createExportDialog(source, api);
  return { api, dialog, state: () => get(dialog.store) };
}

const next = async () => {
  await Promise.resolve();
};

describe('export dialog', () => {
  test('captures active editor text and metadata even when another song is playing', () => {
    const project = { root: editor.root, name: 'works', songs: [{ path: editor.songPath }] };
    const state: SessionState = {
      available: true,
      project,
      songs: [{ path: editor.songPath, title: editor.title, dirty: true, conflict: 'changed' }],
      active: editor.songPath,
      recents: [],
      error: null,
      metaError: null,
      structure,
      notice: null,
      evaluated: { path: 'songs/other.js', title: 'other' },
    };
    expect(captureExportSong(state, editor.source)).toEqual(editor);
    expect(captureExportSong(state, '')?.source).toBe('');
    expect(captureExportSong(state, null)).toBeNull();
    expect(captureExportSong({ ...state, active: null }, editor.source)).toBeNull();
    expect(captureExportSong({ ...state, available: false }, editor.source)).toBeNull();
  });

  test('selects defaults, preserves regular ranges, and sends all loop presets without an override', async () => {
    const { api, dialog } = setup();
    const base = { root: editor.root, songPath: editor.songPath, format: 'wav', folder: null };
    await dialog.refresh();
    expect(api.preview).toHaveBeenLastCalledWith({
      ...base,
      preset: null,
      length: { kind: 'clip' },
    });
    dialog.select({ range: 'section', section: 'body' });
    expect(api.preview).toHaveBeenLastCalledWith({
      ...base,
      preset: null,
      length: { kind: 'section', name: 'body' },
    });
    dialog.select({ mode: 'one-shot', range: 'length', bars: '3' });
    expect(api.preview).toHaveBeenLastCalledWith({
      ...base,
      preset: 'one-shot',
      length: { kind: 'length', bars: 3 },
    });
    for (const preset of ['intro-loop', 'tagged-loop', 'folded-loop'] as const) {
      dialog.select({ mode: preset });
      expect(api.preview).toHaveBeenLastCalledWith({ ...base, preset, length: { kind: 'clip' } });
    }
  });

  test('accepts manual clip or loop requests when the static structure is unavailable', async () => {
    const { api, dialog, state } = setup({ ...editor, structure: buildStructure({}) });
    await dialog.refresh();
    expect(state().options.range).toBe('length');
    expect(api.preview).not.toHaveBeenCalled();
    for (const bars of ['', '0', '-3', '0.5', '3e1']) dialog.select({ bars });
    expect(api.preview).not.toHaveBeenCalled();
    dialog.select({ range: 'clip' });
    expect(api.preview).toHaveBeenLastCalledWith(
      expect.objectContaining({ length: { kind: 'clip' } }),
    );
    dialog.select({ mode: 'tagged-loop', format: 'mp3' });
    expect(api.preview).toHaveBeenLastCalledWith(
      expect.objectContaining({ preset: 'tagged-loop', format: 'mp3' }),
    );
    expect(showMp3Warning(state().options)).toBe(true);
    dialog.select({ mode: 'one-shot' });
    expect(showMp3Warning(state().options)).toBe(false);
  });

  test('preserves the selected folder when the picker is cancelled', async () => {
    const { api, dialog, state } = setup();
    api.pickFolder.mockResolvedValueOnce('/my renders');
    await dialog.pickFolder();
    expect(state().options.folder).toBe('/my renders');
    expect(api.preview).toHaveBeenLastCalledWith(
      expect.objectContaining({ folder: '/my renders' }),
    );
    await dialog.pickFolder();
    expect(state().options.folder).toBe('/my renders');
    expect(api.preview).toHaveBeenCalledTimes(1);
    dialog.select({ folder: null });
    expect(api.preview).toHaveBeenLastCalledWith(expect.objectContaining({ folder: null }));
  });

  test('ignores a late preview success and failure from superseded options', async () => {
    const { api, dialog, state } = setup();
    const initial = Promise.withResolvers<ExportTarget[]>();
    const mp3 = Promise.withResolvers<ExportTarget[]>();
    api.preview.mockReturnValueOnce(initial.promise).mockReturnValueOnce(mp3.promise);
    const pending = dialog.refresh();
    dialog.select({ format: 'mp3' });
    dialog.select({ format: 'opus' });
    await dialog.start();
    expect(api.start).not.toHaveBeenCalled();
    await next();
    initial.resolve([opening]);
    mp3.reject(Error('old request'));
    await pending;
    expect(state().preview).toMatchObject({
      kind: 'ready',
      files: [audio],
      request: { format: 'opus' },
    });
  });

  test('retries a failed preview', async () => {
    const { api, dialog, state } = setup();
    api.preview.mockRejectedValueOnce({ message: 'folder missing', written: [] });
    await dialog.refresh();
    expect(state().preview).toEqual({ kind: 'error', message: 'folder missing' });
    await dialog.start();
    expect(api.start).not.toHaveBeenCalled();
    await dialog.refresh();
    expect(state().preview.kind).toBe('ready');
  });

  test('locks the displayed request through native confirmation and restores it when cancelled', async () => {
    const { api, dialog, state } = setup();
    const answer = Promise.withResolvers<ExportResult | null>();
    api.preview.mockResolvedValue([opening, loop]);
    dialog.select({ mode: 'intro-loop' });
    await next();
    api.start.mockImplementationOnce((_request, _source, _preview, onProgress) => {
      onProgress({ kind: 'confirming' });
      return answer.promise;
    });
    const attempt = dialog.start();
    expect(state().activity.kind).toBe('confirming');
    expect(api.start).toHaveBeenCalledWith(
      expect.objectContaining({ preset: 'intro-loop' }),
      editor.source,
      [opening, loop],
      expect.any(Function),
    );
    expect(dialog.canClose()).toBe(false);
    dialog.select({ format: 'opus' });
    await dialog.start();
    expect(api.start).toHaveBeenCalledTimes(1);
    answer.resolve(null);
    await attempt;
    expect(state().options.format).toBe('wav');
    expect(state().result).toBeNull();
    expect(dialog.canClose()).toBe(true);

    await dialog.start();
    expect(api.start).toHaveBeenCalledTimes(2);
    expect(state().result).toEqual(result);
  });

  test('counts out-of-order chunks and refreshes collision status after a partial write', async () => {
    const { api, dialog, state } = setup();
    await dialog.refresh();
    const render = Promise.withResolvers<ExportResult>();
    let update!: (progress: ExportProgress) => void;
    api.start.mockImplementationOnce((_request, _source, _preview, onProgress) => {
      update = onProgress;
      return render.promise;
    });
    const running = dialog.start();
    expect(state().activity.kind).toBe('planning');
    update({ kind: 'planned', chunks: 2 });
    update({ kind: 'chunkRendered', index: 1, of: 2 });
    expect(state().activity).toEqual({ kind: 'rendering', completed: 1, total: 2 });
    update({ kind: 'chunkRendered', index: 0, of: 2 });
    expect(state().activity.kind).toBe('encoding');
    update({ kind: 'encoded', pass: 1 });
    expect(state().activity.kind).toBe('encoding');
    update({ kind: 'writing' });
    update({ kind: 'written', path: opening.path });
    expect(state().activity.kind).toBe('writing');

    api.preview.mockResolvedValueOnce([opening, loop]);
    render.reject({ message: 'destination full', written: [opening.path] });
    await running;
    expect(state().failure).toEqual({ message: 'destination full', written: [opening.path] });
    expect(state().preview).toMatchObject({ kind: 'ready', files: [opening, loop] });
    update({ kind: 'writing' });
    expect(state().activity.kind).toBe('idle');
    api.start.mockResolvedValueOnce({ ...result, warnings: ['repeat', 'repeat', 'other'] });
    await dialog.start();
    expect(state().failure).toBeNull();
    expect(state().result).toEqual({ ...result, warnings: ['repeat', 'other'] });
    expect(dialog.canClose()).toBe(true);
  });

  test('waits for the export and requires a separate decision after the result is visible', async () => {
    const { api, dialog, state } = setup();
    await dialog.refresh();
    const exportDone = Promise.withResolvers<ExportResult>();
    api.start.mockReturnValueOnce(exportDone.promise);
    const running = dialog.start();
    const quit = dialog.requestQuit();
    expect(state().quit).toBe('ask');
    dialog.answerQuit('continue');
    expect(state().quit).toBe('ask');
    expect(await dialog.requestQuit()).toBe(false);
    dialog.answerQuit('wait');
    expect(state().quit).toBe('waiting');

    exportDone.resolve(result);
    await running;
    expect(state().quit).toBe('result');
    expect(state().result).toEqual(result);
    expect(dialog.canClose()).toBe(false);
    dialog.select({ format: 'opus' });
    expect(state().options.format).toBe('wav');
    dialog.answerQuit('continue');
    expect(await quit).toBe(true);
    expect(state().quit).toBe('settling');
    dialog.finishQuit();
    expect(dialog.canClose()).toBe(true);
  });

  test('reports a partial write even when the export finishes before the wait decision', async () => {
    const { api, dialog, state } = setup();
    await dialog.refresh();
    const exportDone = Promise.withResolvers<ExportResult>();
    api.start.mockReturnValueOnce(exportDone.promise);
    const running = dialog.start();
    const quit = dialog.requestQuit();
    exportDone.reject({ message: 'destination full', written: [opening.path] });
    await running;
    expect(state().quit).toBe('result');
    expect(state().failure).toEqual({ message: 'destination full', written: [opening.path] });
    await dialog.start();
    expect(api.start).toHaveBeenCalledTimes(1);
    dialog.answerQuit('cancel');
    expect(await quit).toBe(false);
    dialog.finishQuit();
    expect(state().failure?.written).toEqual([opening.path]);
    expect(dialog.canClose()).toBe(true);
  });

  test('cancels Quit while waiting without interrupting replacement or export', async () => {
    const { api, dialog, state } = setup();
    await dialog.refresh();
    const exportDone = Promise.withResolvers<ExportResult | null>();
    api.start.mockImplementationOnce((_request, _source, _preview, progress) => {
      progress({ kind: 'confirming' });
      return exportDone.promise;
    });
    const running = dialog.start();
    const quit = dialog.requestQuit();
    dialog.answerQuit('wait');
    dialog.answerQuit('cancel');
    expect(await quit).toBe(false);
    dialog.finishQuit();
    expect(state().activity.kind).toBe('confirming');
    expect(dialog.canClose()).toBe(false);

    const retry = dialog.requestQuit();
    dialog.answerQuit('wait');
    exportDone.resolve(null);
    await running;
    expect(state()).toMatchObject({
      activity: { kind: 'idle' },
      quit: 'result',
      failure: null,
      result: null,
    });
    dialog.answerQuit('continue');
    expect(await retry).toBe(true);
  });

  test('an idle quit prevents a late start and a native folder picker refuses quit', async () => {
    const { api, dialog } = setup();
    await dialog.refresh();
    expect(await dialog.requestQuit()).toBe(true);
    await dialog.start();
    expect(api.start).not.toHaveBeenCalled();
    dialog.finishQuit();

    const folder = Promise.withResolvers<string | null>();
    api.pickFolder.mockReturnValueOnce(folder.promise);
    const picking = dialog.pickFolder();
    expect(await dialog.requestQuit()).toBe(false);
    folder.resolve(null);
    await picking;
    expect(dialog.canClose()).toBe(true);
  });
});

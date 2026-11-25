import { describe, expect, mock, test } from 'bun:test';
import {
  isProjectMenuAction,
  pickedFolder,
  UNSAVED_BUTTONS,
  unsavedChoice,
  unsavedMessage,
} from '../src/lib/project/api';

test('the native menu routes Settings alongside the project and export actions', () => {
  for (const action of ['settings', 'openProject', 'newProject', 'exportAudio']) {
    expect(isProjectMenuAction(action)).toBe(true);
  }
  for (const unrelated of ['quit', 'settings…', { action: 'settings' }, undefined]) {
    expect(isProjectMenuAction(unrelated)).toBe(false);
  }
});

test('the pressed button label maps to a choice', () => {
  expect(unsavedChoice(UNSAVED_BUTTONS.yes)).toBe('save');
  expect(unsavedChoice(UNSAVED_BUTTONS.no)).toBe('discard');
  expect(unsavedChoice(UNSAVED_BUTTONS.cancel)).toBe('cancel');
});

test('any other dialog result is cancel', () => {
  for (const r of ['Yes', 'No', 'Ok', '', 'save', null, undefined, true, 1]) {
    expect(unsavedChoice(r)).toBe('cancel');
  }
});

test('the unsaved prompt names the count', () => {
  expect(unsavedMessage(1)).toBe('1 song has unsaved changes.');
  expect(unsavedMessage(3)).toBe('3 songs have unsaved changes.');
});

test('the folder picker result is one path or null', () => {
  expect(pickedFolder('/a/b')).toBe('/a/b');
  expect(pickedFolder(null)).toBeNull();
  expect(pickedFolder(undefined)).toBeNull();
  expect(pickedFolder(['/x', '/y'])).toBe('/x');
  expect(pickedFolder([])).toBeNull();
  expect(pickedFolder(42)).toBeNull();
});

// ---------------------------------------------------------------- song command errors

const calls: { cmd: string; args: unknown }[] = [];
let invokeImpl: (cmd: string, args: unknown) => unknown = () => undefined;
mock.module('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    return invokeImpl(cmd, args);
  },
}));

const { createTauriProjectApi, ProjectError, PROJECT_CHANGED, toProjectError } =
  await import('../src/lib/project/api');

describe('ProjectError', () => {
  test('is an Error carrying its kind', () => {
    const e = new ProjectError('boom', 'songExists');
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe('ProjectError');
    expect(e.message).toBe('boom');
    expect(e.kind).toBe('songExists');
  });

  test('an object with kind and message keeps both', () => {
    const e = toProjectError({ kind: 'songNotFound', message: 'song songs/a.js not found' });
    expect(e).toBeInstanceOf(ProjectError);
    expect(e.kind).toBe('songNotFound');
    expect(e.message).toBe('song songs/a.js not found');
  });

  test('the stale-project message keeps projectChanged', () => {
    const e = toProjectError({ kind: 'projectChanged', message: PROJECT_CHANGED });
    expect(e.kind).toBe('projectChanged');
    expect(e.message).toBe('the project changed');
  });

  test('a bare string becomes other', () => {
    const e = toProjectError('disk full');
    expect(e.kind).toBe('other');
    expect(e.message).toBe('disk full');
  });

  test('other shapes become other', () => {
    expect(toProjectError({ kind: 1, message: 'x' }).kind).toBe('other');
    expect(toProjectError({ message: 'x' }).kind).toBe('other');
    expect(toProjectError(new Error('plain')).message).toBe('plain');
    expect(toProjectError(null).message).toBe('null');
    const same = new ProjectError('x', 'invalidName');
    expect(toProjectError(same)).toBe(same);
  });
});

describe('createTauriProjectApi song commands', () => {
  test('invoke names and argument keys', async () => {
    calls.length = 0;
    invokeImpl = (cmd) => (cmd === 'song_delete' ? undefined : 'songs/x.js');
    const api = createTauriProjectApi();
    expect(await api.createSong('/r', 'x')).toBe('songs/x.js');
    expect(await api.renameSong('/r', 'songs/a.js', 'x')).toBe('songs/x.js');
    await api.deleteSong('/r', 'songs/a.js');
    await api.projectRootOf('/some/dir');
    expect(calls).toEqual([
      { cmd: 'song_create', args: { root: '/r', stem: 'x' } },
      { cmd: 'song_rename', args: { root: '/r', path: 'songs/a.js', stem: 'x' } },
      { cmd: 'song_delete', args: { root: '/r', path: 'songs/a.js' } },
      { cmd: 'project_root_of', args: { dir: '/some/dir' } },
    ]);
  });

  test('create, rename and delete reject with a ProjectError', async () => {
    const api = createTauriProjectApi();
    invokeImpl = () => {
      throw { kind: 'songExists', message: 'song songs/x.js already exists' };
    };
    for (const call of [
      () => api.createSong('/r', 'x'),
      () => api.renameSong('/r', 'songs/a.js', 'x'),
      () => api.deleteSong('/r', 'songs/a.js'),
    ]) {
      const e = await call().catch((err: unknown) => err);
      expect(e).toBeInstanceOf(ProjectError);
      expect((e as InstanceType<typeof ProjectError>).kind).toBe('songExists');
    }
    invokeImpl = () => {
      throw 'plain string';
    };
    const e = (await api.deleteSong('/r', 'songs/a.js').catch((err: unknown) => err)) as Error;
    expect(e).toBeInstanceOf(ProjectError);
    expect(e.message).toBe('plain string');
  });

  test('saveSong and projectRootOf keep rejecting with the raw value', async () => {
    const api = createTauriProjectApi();
    invokeImpl = () => {
      throw PROJECT_CHANGED;
    };
    expect(await api.saveSong('/r', 'songs/a.js', 'x').catch((e: unknown) => e)).toBe(
      PROJECT_CHANGED,
    );
    expect(await api.projectRootOf('/d').catch((e: unknown) => e)).toBe(PROJECT_CHANGED);
  });
});

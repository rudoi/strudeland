import { describe, expect, test } from 'bun:test';
import { errorMessage } from '../src/lib/errors';
import { ExportError, toExportError } from '../src/lib/export/error';
import { COMMAND_ERROR_KINDS } from '../src/lib/ipc/bindings';
import { CommandError, narrowKind, toCommandError } from '../src/lib/ipc/command-error';
import { mockTauriCore, mockTauriModule } from './helpers/tauri-core-mock';

const core = mockTauriCore();
mockTauriModule('@tauri-apps/api/event', { listen: async () => () => {} });
const { invokeCommand } = await import('../src/lib/ipc/invoke');

describe('CommandError', () => {
  test('is an Error carrying its kind', () => {
    const e = new CommandError('boom', 'busy');
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe('CommandError');
    expect(e.message).toBe('boom');
    expect(e.kind).toBe('busy');
  });

  test('the kinds mirror the Rust CommandErrorKind, in order', () => {
    expect([...COMMAND_ERROR_KINDS]).toEqual([
      'projectChanged',
      'noProject',
      'notFound',
      'exists',
      'invalidName',
      'busy',
      'permissionDenied',
      'noDevice',
      'deviceLost',
      'limitReached',
      'stale',
      'output',
      'badBatch',
      'engine',
      'other',
    ]);
  });
});

describe('toCommandError', () => {
  test.each([...COMMAND_ERROR_KINDS])('the Rust object with kind %s keeps both', (kind) => {
    const raw = { kind, message: `m-${kind}` };
    const e = toCommandError(raw);
    expect(e).toBeInstanceOf(CommandError);
    expect([e.kind, e.message]).toEqual([kind, `m-${kind}`]);
    expect(e.cause).toBe(raw);
  });

  test('an unknown or missing kind becomes other, keeping the message', () => {
    expect(toCommandError({ kind: 'songExists', message: 'x' })).toMatchObject({
      kind: 'other',
      message: 'x',
    });
    expect(toCommandError({ kind: 1, message: 'y' })).toMatchObject({
      kind: 'other',
      message: 'y',
    });
    expect(toCommandError({ message: 'z' })).toMatchObject({ kind: 'other', message: 'z' });
  });

  test('a string, an Error, null and a malformed object are their text with kind other', () => {
    const cases: [unknown, string][] = [
      ['disk full', 'disk full'],
      [new Error('plain'), 'plain'],
      [null, 'null'],
      [{ kind: 'busy' }, '[object Object]'],
    ];
    for (const [raw, message] of cases) {
      const e = toCommandError(raw);
      expect(e).toBeInstanceOf(CommandError);
      expect([e.kind, e.message]).toEqual(['other', message]);
      expect(e.cause).toBe(raw);
    }
  });

  test('a CommandError is returned as is, with its own cause', () => {
    const e = new CommandError('x', 'stale', { cause: 'inner' });
    expect(toCommandError(e)).toBe(e);
    expect(toCommandError(e).cause).toBe('inner');
  });

  test('the raw object is the cause, so its written paths survive', () => {
    const e = toCommandError({ kind: 'other', message: 'x', written: ['/a'] });
    expect((e.cause as { written: unknown }).written).toEqual(['/a']);
  });
});

describe('narrowKind', () => {
  test('a kind in the list is kept; any other is the fallback', () => {
    const allowed = ['stale', 'engine'] as const;
    expect(narrowKind('stale', allowed, 'engine')).toBe('stale');
    expect(narrowKind('projectChanged', allowed, 'engine')).toBe('engine');
    expect(narrowKind('', allowed, 'engine')).toBe('engine');
  });
});

describe('toExportError', () => {
  test('written comes off the CommandError cause', () => {
    const c = toCommandError({ kind: 'other', message: 'x', written: ['/a'] });
    const e = toExportError(c);
    expect(e).toBeInstanceOf(ExportError);
    expect(e).toBeInstanceOf(CommandError);
    expect([e.name, e.kind, e.message, e.written]).toEqual(['ExportError', 'other', 'x', ['/a']]);
  });

  test('the non-string entries of written are dropped, and the paths kept', () => {
    const written = (w: unknown) =>
      toExportError(toCommandError({ kind: 'other', message: 'x', written: w })).written;
    expect(written(['/a.wav', null])).toEqual(['/a.wav']);
    expect(written(['/a', 1, '/b'])).toEqual(['/a', '/b']);
    expect(written([null])).toEqual([]);
    expect(written('/a')).toEqual([]);
  });

  test('no written is empty', () => {
    expect(toExportError(toCommandError({ kind: 'busy', message: 'b' }))).toMatchObject({
      kind: 'busy',
      written: [],
    });
    expect(toExportError('text')).toMatchObject({ kind: 'other', message: 'text', written: [] });
  });

  test('an ExportError is returned as is', () => {
    const e = new ExportError('x', 'other', ['/a']);
    expect(toExportError(e)).toBe(e);
  });
});

describe('invokeCommand', () => {
  test('resolves to the command result with the same name and args', async () => {
    core.answer(() => 42);
    expect(await invokeCommand('song_read', { root: '/r', path: 'songs/a.js' })).toBe(42);
    expect(core.calls).toEqual([{ cmd: 'song_read', args: { root: '/r', path: 'songs/a.js' } }]);
  });

  test('a { kind, message } rejection is an Error with that message', async () => {
    core.reject({ kind: 'projectChanged', message: 'the project changed' });
    const e = await invokeCommand('song_save', {
      root: '/r',
      path: 'songs/a.js',
      contents: '',
    }).catch((err: unknown) => err);
    expect(e).toBeInstanceOf(Error);
    expect(e).toBeInstanceOf(CommandError);
    expect(e).toMatchObject({ kind: 'projectChanged', message: 'the project changed' });
    expect(errorMessage(e)).toBe('the project changed');
  });

  test('a string rejection is other', async () => {
    core.reject('no application menu');
    const e = await invokeCommand('export_audio_set_available', { available: true }).catch(
      (err: unknown) => err,
    );
    expect(e).toMatchObject({ kind: 'other', message: 'no application menu' });
  });
});

import { describe, expect, mock, test } from 'bun:test';
import { errorMessage } from '../src/lib/errors';
import { RECORDING_ERROR_KINDS, RecordingError } from '../src/lib/recording/types';

const calls: { cmd: string; args: unknown }[] = [];
let invokeImpl: (cmd: string, args: unknown) => unknown = () => undefined;
const listeners: { event: string; handler: (e: { payload: unknown }) => void }[] = [];
let unlistened = 0;

mock.module('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    return invokeImpl(cmd, args);
  },
}));
mock.module('@tauri-apps/api/event', () => ({
  listen: async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.push({ event, handler });
    return () => void unlistened++;
  },
}));

const { createTauriRecordingApi, parseHostNs, toRecordingError } =
  await import('../src/lib/recording/api');
const { EVENTS } = await import('../src/lib/ipc/bindings');

/** Run `fn` against a fresh call log; `impl` answers each invoke. */
async function invoked(
  fn: (api: ReturnType<typeof createTauriRecordingApi>) => Promise<unknown>,
  impl: (cmd: string, args: unknown) => unknown = () => undefined,
) {
  calls.length = 0;
  invokeImpl = impl;
  const result = await fn(createTauriRecordingApi());
  return { result, calls: [...calls] };
}

async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  throw new Error('expected a rejection');
}

const TEMPO = { bpm: 120, meter: '4/4', cps: 0.5, beatsPerBar: 4, tempoOverride: false };
const BACKING = {
  song: true,
  metronome: false,
  countIn: true,
  songPath: 'songs/a.js',
  startSection: null,
  loopSection: null,
};

describe('RecordingError', () => {
  test('is an Error carrying its kind', () => {
    const e = new RecordingError('microphone access denied', 'permissionDenied');
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe('RecordingError');
    expect(e.message).toBe('microphone access denied');
    expect(e.kind).toBe('permissionDenied');
  });

  test('the kinds mirror the Rust ones', () => {
    expect([...RECORDING_ERROR_KINDS]).toEqual([
      'permissionDenied',
      'noDevice',
      'deviceLost',
      'busy',
      'invalidName',
      'exists',
      'notFound',
      'projectChanged',
      'limitReached',
      'other',
    ]);
  });
});

describe('toRecordingError', () => {
  test.each([...RECORDING_ERROR_KINDS])('the Rust object with kind %s keeps both', (kind) => {
    const e = toRecordingError({ kind, message: `m-${kind}` });
    expect(e).toBeInstanceOf(RecordingError);
    expect(e.kind).toBe(kind);
    expect(e.message).toBe(`m-${kind}`);
  });

  test('an unknown kind becomes other, keeping the message', () => {
    const e = toRecordingError({ kind: 'songExists', message: 'x' });
    expect(e.kind).toBe('other');
    expect(e.message).toBe('x');
  });

  test('a bare string, an Error and other shapes become other', () => {
    expect(toRecordingError('disk full')).toMatchObject({ kind: 'other', message: 'disk full' });
    expect(toRecordingError(new Error('plain'))).toMatchObject({ kind: 'other', message: 'plain' });
    expect(toRecordingError({ kind: 'busy' })).toMatchObject({
      kind: 'other',
      message: '[object Object]',
    });
    expect(toRecordingError(null)).toMatchObject({ kind: 'other', message: 'null' });
  });

  test('a RecordingError passes through', () => {
    const e = new RecordingError('busy', 'busy');
    expect(toRecordingError(e)).toBe(e);
  });
});

describe('parseHostNs', () => {
  test('decimal strings, beyond 2^53', () => {
    expect(parseHostNs('0')).toBe(0n);
    expect(parseHostNs('18446744073709551615')).toBe(2n ** 64n - 1n);
    expect(parseHostNs('9007199254740993')).toBe(9007199254740993n);
  });

  test.each([['-1'], [''], ['1.5'], ['1e9'], [' 1'], ['123456789012345678901'], [42], [null]])(
    'rejects %p as other',
    (v) => {
      let err: unknown;
      try {
        parseHostNs(v);
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(RecordingError);
      expect((err as RecordingError).kind).toBe('other');
    },
  );
});

describe('createTauriRecordingApi', () => {
  test('status, devices and disarm take no arguments', async () => {
    for (const [method, cmd] of [
      ['status', 'recording_status'],
      ['devices', 'recording_devices'],
      ['disarm', 'recording_disarm'],
    ] as const) {
      const { calls } = await invoked((api) => api[method]());
      expect(calls).toEqual([{ cmd, args: undefined }]);
    }
  });

  test('arm passes only the given device and channel', async () => {
    expect((await invoked((api) => api.arm())).calls).toEqual([{ cmd: 'recording_arm', args: {} }]);
    expect((await invoked((api) => api.arm({ deviceId: 'usb-2', channel: 1 }))).calls).toEqual([
      { cmd: 'recording_arm', args: { deviceId: 'usb-2', channel: 1 } },
    ]);
    expect((await invoked((api) => api.arm({ channel: 0 }))).calls).toEqual([
      { cmd: 'recording_arm', args: { channel: 0 } },
    ]);
  });

  test('start passes root, songStem and fields; without a song stem it is absent', async () => {
    const fields = { tempo: TEMPO, backing: BACKING };
    const a = await invoked(
      (api) => api.start({ root: '/p', songStem: 'a', fields }),
      () => ({ stem: 'a-01' }),
    );
    expect(a.result).toEqual({ stem: 'a-01' });
    expect(a.calls).toEqual([
      { cmd: 'recording_start', args: { root: '/p', songStem: 'a', fields } },
    ]);
    const b = await invoked(
      (api) => api.start({ root: '/p', fields }),
      () => ({ stem: 'memo-01' }),
    );
    expect(b.calls).toEqual([{ cmd: 'recording_start', args: { root: '/p', fields } }]);
  });

  test('stop passes the sidecar (or null) and no root', async () => {
    const sidecar = {
      output: { baseLatency: 0.005, outputLatency: null },
      tempo: TEMPO,
      backing: BACKING,
      timing: {
        bar1HostNs: '123',
        clockOffsetNs: { start: { offsetNs: '-12', rttNs: '400' }, end: null },
        outputStamps: [[1.5, '99']] as [number, string][],
      },
    };
    expect((await invoked((api) => api.stop({ sidecar }))).calls).toEqual([
      { cmd: 'recording_stop', args: { sidecar } },
    ]);
    expect((await invoked((api) => api.stop({ sidecar: null }))).calls).toEqual([
      { cmd: 'recording_stop', args: { sidecar: null } },
    ]);
  });

  test('the memo library commands', async () => {
    expect(
      (
        await invoked(
          (api) => api.list('/p'),
          () => [],
        )
      ).calls,
    ).toEqual([{ cmd: 'recording_list', args: { root: '/p' } }]);
    const r = await invoked(
      (api) => api.rename('/p', 'a-01', 'take'),
      () => 'take',
    );
    expect(r.result).toBe('take');
    expect(r.calls).toEqual([
      { cmd: 'recording_rename', args: { root: '/p', stem: 'a-01', newStem: 'take' } },
    ]);
    expect((await invoked((api) => api.delete('/p', 'a-01'))).calls).toEqual([
      { cmd: 'recording_delete', args: { root: '/p', stem: 'a-01' } },
    ]);
  });

  test('read turns the raw response into bytes', async () => {
    const buf = new Uint8Array([82, 73, 70, 70]).buffer;
    const r = await invoked(
      (api) => api.read('/p', 'a-01'),
      () => buf,
    );
    expect(r.calls).toEqual([{ cmd: 'recording_read', args: { root: '/p', stem: 'a-01' } }]);
    expect(r.result).toBeInstanceOf(Uint8Array);
    expect([...(r.result as Uint8Array)]).toEqual([82, 73, 70, 70]);
  });

  test('hostNowNs parses the decimal string', async () => {
    const r = await invoked(
      (api) => api.hostNowNs(),
      () => '9007199254740993',
    );
    expect(r.calls).toEqual([{ cmd: 'host_now_ns', args: undefined }]);
    expect(r.result).toBe(9007199254740993n);
    const e = await rejection(
      invoked(
        (api) => api.hostNowNs(),
        () => 12,
      ),
    );
    expect(e).toMatchObject({ kind: 'other', message: 'invalid host time 12' });
  });

  test('a rejected command throws a RecordingError with its kind', async () => {
    const reject = (kind: string) => () => {
      throw { kind, message: `no: ${kind}` };
    };
    const e = await rejection(invoked((api) => api.arm(), reject('permissionDenied')));
    expect(e).toBeInstanceOf(RecordingError);
    expect(e).toMatchObject({ kind: 'permissionDenied', message: 'no: permissionDenied' });
    const f = await rejection(invoked((api) => api.rename('/p', 'a', 'b'), reject('exists')));
    expect(f).toMatchObject({ kind: 'exists' });
    const g = await rejection(
      invoked(
        (api) => api.list('/p'),
        () => {
          throw 'the project changed';
        },
      ),
    );
    expect(g).toMatchObject({ kind: 'other', message: 'the project changed' });
    const h = await rejection(
      invoked((api) => api.start({ root: '/p', fields: {} as never }), reject('noProject')),
    );
    expect(h).toBeInstanceOf(Error);
    expect(h).toMatchObject({ kind: 'other', message: 'no: noProject' });
    expect(errorMessage(h)).toBe('no: noProject');
  });

  test('onLevel and onState listen to the recording events and unlisten', async () => {
    listeners.length = 0;
    const api = createTauriRecordingApi();
    const levels: unknown[] = [];
    const states: unknown[] = [];
    const offLevel = await api.onLevel((e) => levels.push(e));
    const offState = await api.onState((e) => states.push(e));
    expect(listeners.map((l) => l.event)).toEqual([EVENTS.recordingLevel, EVENTS.recordingState]);
    expect(EVENTS.recordingLevel).toBe('recording://level');
    expect(EVENTS.recordingState).toBe('recording://state');
    listeners[0]!.handler({ payload: { peak: 0.5, rms: 0.1 } });
    listeners[1]!.handler({ payload: { state: 'armed' } });
    listeners[1]!.handler({
      payload: { state: 'idle', error: { kind: 'limitReached', message: 'cap' } },
    });
    listeners[1]!.handler({
      payload: { state: 'idle', error: { kind: 'noProject', message: 'no project is open' } },
    });
    expect(levels).toEqual([{ peak: 0.5, rms: 0.1 }]);
    expect(states).toEqual([
      { state: 'armed' },
      { state: 'idle', error: { kind: 'limitReached', message: 'cap' } },
      { state: 'idle', error: { kind: 'other', message: 'no project is open' } },
    ]);
    const before = unlistened;
    offLevel();
    offState();
    expect(unlistened).toBe(before + 2);
  });
});

describe('fakeRecordingApi', () => {
  test('the memo library: list, rename, delete, read and their errors', async () => {
    const { fakeRecordingApi, entry } = await import('./helpers/fake-recording-api');
    const { api, fake } = fakeRecordingApi({ roots: ['/p'] });
    fake.memos.get('/p')!.push(entry('a-01'), entry('a-02'));
    fake.bytes.set('/p:a-01', new Uint8Array([1, 2]));
    expect((await api.list('/p')).map((m) => m.stem)).toEqual(['a-01', 'a-02']);
    expect(await api.rename('/p', 'a-01', 'idea')).toBe('idea');
    expect([...(await api.read('/p', 'idea'))]).toEqual([1, 2]);
    expect(await rejection(api.rename('/p', 'idea', 'a-02'))).toMatchObject({ kind: 'exists' });
    expect(await rejection(api.rename('/p', 'idea', 'a/b'))).toMatchObject({ kind: 'invalidName' });
    expect(await rejection(api.rename('/p', 'nope', 'x'))).toMatchObject({ kind: 'notFound' });
    await api.delete('/p', 'idea');
    expect((await api.list('/p')).map((m) => m.stem)).toEqual(['a-02']);
    expect(await rejection(api.read('/p', 'idea'))).toMatchObject({ kind: 'notFound' });
    expect(await rejection(api.list('/q'))).toMatchObject({ kind: 'projectChanged' });
    fake.failNext('list', new RecordingError('boom', 'other'));
    expect(await rejection(api.list('/p'))).toMatchObject({ kind: 'other', message: 'boom' });
    expect((await api.list('/p')).length).toBe(1);
  });

  test('a take: arm, start, stop add the memo; events reach the listeners', async () => {
    const { fakeRecordingApi } = await import('./helpers/fake-recording-api');
    const { api, fake } = fakeRecordingApi();
    expect((await api.arm()).state).toBe('armed');
    const fields = { tempo: TEMPO, backing: BACKING };
    expect(await api.start({ root: '/p', songStem: 'a', fields })).toEqual({ stem: 'a-01' });
    expect((await api.stop({ sidecar: null })).stem).toBe('a-01');
    expect(fake.stops).toEqual([null]);
    expect((await api.list('/p')).map((m) => m.stem)).toEqual(['a-01']);
    expect(fake.names()).toEqual(['arm', 'start', 'stop', 'list']);
    expect(await api.hostNowNs()).toBe(1_000_000_000_000n);
    expect(await api.hostNowNs()).toBe(1_000_001_000_000n);
    expect(fake.pings).toBe(2);
    const states: unknown[] = [];
    const off = await api.onState((e) => states.push(e));
    fake.emitState({ state: 'idle' });
    off();
    fake.emitState({ state: 'armed' });
    expect(states).toEqual([{ state: 'idle' }]);
  });
});

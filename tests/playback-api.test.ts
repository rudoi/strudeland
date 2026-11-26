import { describe, expect, test } from 'bun:test';
import { errorMessage } from '../src/lib/errors';
import { FakeChannel, mockTauriCore, mockTauriModule } from './helpers/tauri-core-mock';

const core = mockTauriCore();
// The host clock comes through the recording API, which also imports `listen`.
mockTauriModule('@tauri-apps/api/event', { listen: async () => () => {} });

const { PlaybackError, tauriPlaybackApi, toPlaybackError } =
  await import('../src/lib/audio/playback-api');

async function invoked(
  fn: () => Promise<unknown>,
  impl: (cmd: string, args: unknown) => unknown = () => undefined,
) {
  core.answer(impl);
  const result = await fn();
  return { result, calls: [...core.calls] };
}

async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  throw new Error('expected a rejection');
}

describe('PlaybackError', () => {
  test('is an Error carrying its kind', () => {
    const e = new PlaybackError('the playback session ended', 'stale');
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe('PlaybackError');
    expect(e.kind).toBe('stale');
    expect(e.message).toBe('the playback session ended');
  });
});

describe('toPlaybackError', () => {
  test('a known kind keeps it; an unknown kind and a plain Error are engine', () => {
    const stale = toPlaybackError({ kind: 'stale', message: 'gone' });
    expect([stale.kind, stale.message]).toEqual(['stale', 'gone']);
    for (const kind of ['output', 'badBatch', 'engine'] as const) {
      expect(toPlaybackError({ kind, message: 'm' }).kind).toBe(kind);
    }
    const bogus = toPlaybackError({ kind: 'bogus', message: 'odd' });
    expect([bogus.kind, bogus.message]).toEqual(['engine', 'odd']);
    const plain = toPlaybackError(new Error('boom'));
    expect([plain.kind, plain.message]).toEqual(['engine', 'boom']);
    const text = toPlaybackError('just text');
    expect([text.kind, text.message]).toEqual(['engine', 'just text']);
    const same = new PlaybackError('no device', 'output');
    expect(toPlaybackError(same)).toBe(same);
  });
});

describe('tauriPlaybackApi', () => {
  test('start: seed and a channel whose onmessage is the callback; resolves to the session', async () => {
    const events: unknown[] = [];
    const onEvent = (e: unknown) => events.push(e);
    const { result, calls } = await invoked(
      () => tauriPlaybackApi.start(7, onEvent),
      () => ({ session: 3 }),
    );
    expect(result).toBe(3);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.cmd).toBe('playback_start');
    const args = calls[0]!.args as { seed: number; channel: FakeChannel<unknown> };
    expect(Object.keys(args).sort()).toEqual(['channel', 'seed']);
    expect(args.seed).toBe(7);
    const channel = args.channel;
    expect(channel).toBeInstanceOf(FakeChannel);
    channel.onmessage({ kind: 'anchor', frame: 1, hostNs: '2' });
    expect(events).toEqual([{ kind: 'anchor', frame: 1, hostNs: '2' }]);
  });

  test('schedule, stop and setSeed: command names and argument shapes', async () => {
    const batch = {
      values: [],
      haps: [[0.1, 0.5, 0, 0]] as [number, number, number, number][],
      cps: 1,
    };
    const { calls } = await invoked(async () => {
      await tauriPlaybackApi.schedule(2, batch);
      await tauriPlaybackApi.stop(2);
      await tauriPlaybackApi.setSeed(2, -5);
      await tauriPlaybackApi.setLimiter(2, true);
    });
    expect(calls).toEqual([
      { cmd: 'playback_schedule', args: { session: 2, batch } },
      { cmd: 'playback_stop', args: { session: 2 } },
      { cmd: 'playback_seed', args: { session: 2, seed: -5 } },
      { cmd: 'playback_limiter', args: { session: 2, on: true } },
    ]);
  });

  test('hostNowNs parses the decimal string to a bigint', async () => {
    const { result, calls } = await invoked(
      () => tauriPlaybackApi.hostNowNs(),
      () => '18446744073709551615',
    );
    expect(result).toBe(18446744073709551615n);
    expect(calls).toEqual([{ cmd: 'host_now_ns', args: undefined }]);
  });

  test.each([['-1'], ['1.5'], [42], [null]])('hostNowNs rejects %p as engine', async (v) => {
    const e = await rejection(
      invoked(
        () => tauriPlaybackApi.hostNowNs(),
        () => v,
      ),
    );
    expect(e).toBeInstanceOf(PlaybackError);
    expect((e as InstanceType<typeof PlaybackError>).kind).toBe('engine');
    expect((e as Error).message).toBe(`invalid host time ${JSON.stringify(v)}`);
  });

  test('the host clock comes from the recording API, not a Tauri call of its own', async () => {
    const src = await Bun.file(new URL('../src/lib/audio/playback-api.ts', import.meta.url)).text();
    expect(src).not.toMatch(/['"]host_now_ns['"]/);
    expect(src).toContain("from '../recording/api'");
  });

  test('rejections become PlaybackError: stale, unknown kind and a plain Error', async () => {
    const cases: [unknown, string, string][] = [
      [
        { kind: 'stale', message: 'the playback session ended' },
        'stale',
        'the playback session ended',
      ],
      [{ kind: 'bogus', message: 'odd' }, 'engine', 'odd'],
      [{ kind: 'projectChanged', message: 'not ours' }, 'engine', 'not ours'],
      [new Error('ipc down'), 'engine', 'ipc down'],
      ['a string', 'engine', 'a string'],
    ];
    for (const [thrown, kind, message] of cases) {
      core.reject(thrown);
      const e = await rejection(tauriPlaybackApi.stop(1));
      expect(e).toBeInstanceOf(PlaybackError);
      expect([(e as InstanceType<typeof PlaybackError>).kind, (e as Error).message]).toEqual([
        kind,
        message,
      ]);
      expect(errorMessage(e)).toBe(message);
    }
    core.answer(() => undefined);
  });
});

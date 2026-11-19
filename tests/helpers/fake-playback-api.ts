/**
 * An in-memory `PlaybackApi`: every call but `hostNowNs` is logged in `fake.calls` (name and
 * arguments; `start`'s callback is not logged), sessions are numbered from 1, `emit` plays a
 * player event to the open session's callback, `failNext` makes the next call (of one method, or
 * any) reject with a `PlaybackError`, `holdSchedules` keeps `schedule` promises pending until
 * `release()`, and `hostNowNs` reads `fake.hostNs()` (settable).
 */
import {
  PlaybackError,
  type PlaybackApi,
  type PlaybackBatch,
  type PlaybackErrorKind,
  type PlaybackEvent,
} from '../../src/lib/audio/playback-api';

type Method = keyof PlaybackApi;

export function fakePlaybackApi(opts: { hostNs?: () => bigint } = {}) {
  let nextSession = 1;
  let onEvent: ((e: PlaybackEvent) => void) | null = null;
  let failure: { kind: PlaybackErrorKind; method: Method | null } | null = null;
  const held: (() => void)[] = [];

  const fake = {
    calls: [] as { method: Exclude<Method, 'hostNowNs'>; args: unknown[] }[],
    pings: 0,
    /** The host clock `hostNowNs` answers with. */
    hostNs: opts.hostNs ?? (() => 1_000_000_000n),
    /** The open session (the last `start`), null after its `stop`. */
    session: null as number | null,
    /** When true, `schedule` resolves only at `release()`. */
    holdSchedules: false,
    /** The batches `schedule` received, in call order. */
    get batches(): PlaybackBatch[] {
      return fake.calls
        .filter((c) => c.method === 'schedule')
        .map((c) => c.args[1] as PlaybackBatch);
    },
    /** Calls of one method. */
    of(method: Exclude<Method, 'hostNowNs'>): unknown[][] {
      return fake.calls.filter((c) => c.method === method).map((c) => c.args);
    },
    /** Delivers an event to the open session's callback (none: dropped). */
    emit(e: PlaybackEvent) {
      onEvent?.(e);
    },
    /** The callback of the last `start`, even after a later one (to play a stale session). */
    lastCallback: null as ((e: PlaybackEvent) => void) | null,
    /** The next call (of `method`, or of any method) rejects with `kind`. */
    failNext(kind: PlaybackErrorKind, method: Method | null = null) {
      failure = { kind, method };
    },
    /** Resolves every held `schedule`. */
    release() {
      for (const r of held.splice(0)) r();
    },
  };

  const check = (method: Method) => {
    if (failure && (failure.method === null || failure.method === method)) {
      const { kind } = failure;
      failure = null;
      throw new PlaybackError(kind, `fake ${method} failed (${kind})`);
    }
  };

  const api: PlaybackApi = {
    async start(seed, cb) {
      fake.calls.push({ method: 'start', args: [seed] });
      check('start');
      const s = nextSession++;
      fake.session = s;
      onEvent = cb;
      fake.lastCallback = cb;
      return s;
    },
    async schedule(session, batch) {
      // A deep copy: the log shows the batch as it crossed.
      fake.calls.push({ method: 'schedule', args: [session, structuredClone(batch)] });
      check('schedule');
      if (fake.holdSchedules) await new Promise<void>((r) => held.push(r));
    },
    async stop(session) {
      fake.calls.push({ method: 'stop', args: [session] });
      if (fake.session === session) {
        fake.session = null;
        onEvent = null;
      }
      check('stop');
    },
    async setSeed(session, seed) {
      fake.calls.push({ method: 'setSeed', args: [session, seed] });
      check('setSeed');
    },
    async setLimiter(session, on) {
      fake.calls.push({ method: 'setLimiter', args: [session, on] });
      check('setLimiter');
    },
    async hostNowNs() {
      fake.pings++;
      check('hostNowNs');
      return fake.hostNs();
    },
  };

  return { api, fake };
}

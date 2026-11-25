import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, describe, expect, test } from 'bun:test';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

const { createSession } = await import('../src/lib/project/session');
const { createEditorState } = await import('../src/lib/editor/setup');
const { createControlHandler, replyTo } = await import('../src/lib/control/handler');
const { ControlError, ErrorCode } = await import('../src/lib/control/protocol');
const { EditorView } = await import('@codemirror/view');
const { undo } = await import('@codemirror/commands');
const { get } = await import('svelte/store');
import type { ControlHandler, ControlPlayback } from '../src/lib/control/handler';
import type { Session } from '../src/lib/project/session';
import { deferred, fakeProjectApi, flush } from './helpers/fake-project-api';
import { stubRuntime } from './helpers/stub-runtime';

const A = `const meta = { title: 'Alpha', bpm: 120 };\n$: s("a")\n`;
const B = `$: s("b")\n`; // no header: title is the file name
const BAD = `const meta = { bpm: 'fast' };\n$: s("x")\n`; // a meta error
const PA = 'songs/a.js';
const PB = 'songs/b.js';
const PM = 'songs/m.js';

const PROJECTS = {
  '/p': { name: 'Proj', files: { [PA]: A, [PB]: B, [PM]: BAD } },
  '/empty': { name: 'Empty', files: {} },
};

async function setup(
  opts: {
    last?: string | null;
    failRead?: string[];
    playback?: ControlPlayback;
    vim?: () => boolean;
  } = {},
) {
  const { api, fake } = fakeProjectApi(PROJECTS);
  fake.last = opts.last === undefined ? '/p' : opts.last;
  for (const p of opts.failRead ?? []) fake.failRead.add(p);
  const rt = stubRuntime({ audio: 'running', live: true });
  const view = new EditorView({ parent: document.body });
  let session!: Session;
  session = createSession({
    api,
    runtime: rt.runtime,
    createState: (doc: string) =>
      createEditorState(doc, rt.runtime, { onDocChange: () => session.docChanged() }),
    isTauri: () => true,
  });
  session.attachView(view);
  await session.init();
  await flush();
  for (const p of opts.failRead ?? []) fake.failRead.delete(p);
  const handle = createControlHandler({
    session,
    runtime: rt.runtime,
    playback: opts.playback,
    vim: opts.vim,
  });
  const st = () => get(session.store);
  const item = (path: string) => st().songs.find((s) => s.path === path)!;
  const doc = () => view.state.doc.toString();
  /** The error a call rejects with, as `{ code, message }`. */
  const fail = async (method: Parameters<ControlHandler>[0], params: unknown) => {
    try {
      await handle(method, params);
    } catch (e) {
      if (!(e instanceof ControlError)) throw e;
      return { code: e.code, message: e.message };
    }
    throw new Error(`${method} did not fail`);
  };
  return { api, fake, rt, view, session, handle, st, item, doc, fail };
}

// ---------------------------------------------------------------- state

describe('state', () => {
  test('the exact summary of an open project', async () => {
    const h = await setup();
    h.view.dispatch({ changes: { from: h.doc().length, insert: '// x' } });
    await h.handle('eval', {});
    h.rt.patch({ tempoOverride: true });
    expect(await h.handle('state', {})).toEqual({
      project: { name: 'Proj', root: '/p' },
      songs: [
        { path: PA, title: 'Alpha', dirty: true, conflict: null },
        { path: PB, title: 'b', dirty: false, conflict: null },
        { path: PM, title: 'm', dirty: false, conflict: null },
      ],
      active: PA,
      playing: { path: PA, title: 'Alpha' },
      started: true,
      cps: 0.5,
      bpm: 120,
      meter: '4/4',
      audio: 'running',
      evalError: null,
      metaError: null,
      tempoOverride: true,
      playhead: { cycle: 0, songBar: null, section: null }, // started, no song()
      transport: { view: 'song', loopSection: null, armedSection: null },
      outputError: null,
      engine: 'superdough',
      live: null,
      levels: null,
      vim: false,
    });
  });

  test('vim comes from the dep', async () => {
    const h = await setup({ vim: () => true });
    expect(((await h.handle('state', {})) as { vim: boolean }).vim).toBe(true);
  });

  test('playing is null while stopped; metaError and the meter come through', async () => {
    const h = await setup();
    h.rt.patch({ meter: { num: 6, den: 8 }, cps: 1 / 3 });
    await h.session.switchSong(PM);
    const s = (await h.handle('state', {})) as any;
    expect(s.playing).toBeNull();
    expect(s.meter).toBe('6/8');
    expect(s.bpm).toBe(60); // 1/3 cps * 60 * 3 quarter notes
    expect(s.metaError).toContain('bpm');
  });

  test('seed is reported when the runtime has one, and left out otherwise', async () => {
    const h = await setup();
    expect(await h.handle('state', {})).not.toHaveProperty('seed');
    h.rt.patch({ seed: 7 });
    expect(((await h.handle('state', {})) as any).seed).toBe(7);
    h.rt.patch({ seed: 0 });
    expect(((await h.handle('state', {})) as any).seed).toBe(0);
    h.rt.patch({ seed: undefined });
    expect(await h.handle('state', {})).not.toHaveProperty('seed');
  });

  test('no project open', async () => {
    const h = await setup({ last: null });
    h.rt.patch({ evalError: 'old error' });
    expect(await h.handle('state', {})).toEqual({
      project: null,
      songs: [],
      active: null,
      playing: null,
      started: false,
      cps: 0.5,
      bpm: 120,
      meter: '4/4',
      audio: 'running',
      evalError: 'old error',
      metaError: null,
      tempoOverride: false,
      playhead: null,
      transport: { view: 'song', loopSection: null, armedSection: null },
      outputError: null,
      engine: 'superdough',
      live: null,
      levels: null,
      vim: false,
    });
  });
});

// ---------------------------------------------------------------- code.get

describe('code.get', () => {
  test('the active song by default, with live edits', async () => {
    const h = await setup();
    expect(await h.handle('code.get', {})).toEqual({
      song: PA,
      title: 'Alpha',
      code: A,
      dirty: false,
    });
    h.view.dispatch({ changes: { from: 0, insert: '// x\n' } });
    expect(await h.handle('code.get', {})).toEqual({
      song: PA,
      title: 'Alpha',
      code: '// x\n' + A,
      dirty: true,
    });
  });

  test('by stem and by path, without switching; stashed edits included', async () => {
    const h = await setup();
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// b\n' } });
    await h.session.switchSong(PA);
    const viewState = h.view.state;
    expect(await h.handle('code.get', { song: 'b' })).toEqual({
      song: PB,
      title: 'b',
      code: '// b\n' + B,
      dirty: true,
    });
    expect(await h.handle('code.get', { song: PB })).toMatchObject({ song: PB, dirty: true });
    expect(h.st().active).toBe(PA);
    expect(h.view.state).toBe(viewState);
  });

  test('a song removed while it is read is -32005', async () => {
    const h = await setup({ failRead: [PB] });
    const gate = deferred();
    h.fake.holdReads = gate.promise;
    const call = h.fail('code.get', { song: 'b' });
    await flush();
    h.fake.files().delete(PB);
    h.fake.emit({ removed: [PB] });
    h.fake.holdReads = null;
    gate.resolve();
    expect(await call).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: `no such song '${PB}' (it was removed)`,
    });
  });

  test('an unread song is loaded without switching; an unreadable one is -32006', async () => {
    const h = await setup({ failRead: [PB] });
    expect(await h.handle('code.get', { song: 'b' })).toMatchObject({ song: PB, code: B });
    expect(h.st().active).toBe(PA);
    const g = await setup({ failRead: [PB] });
    g.fake.failRead.add(PB);
    const e = await g.fail('code.get', { song: 'b' });
    expect(e.code).toBe(ErrorCode.SONG_UNLOADABLE);
    expect(e.message).toContain('permission denied');
    expect(g.st().active).toBe(PA);
  });
});

// ---------------------------------------------------------------- song resolution

describe('song resolution', () => {
  test('an unknown song lists the stems', async () => {
    const h = await setup();
    for (const method of ['code.get', 'code.set', 'eval', 'play'] as const) {
      const params = method === 'code.set' ? { song: 'nope', code: '' } : { song: 'nope' };
      expect(await h.fail(method, params)).toEqual({
        code: ErrorCode.NO_SUCH_SONG,
        message: `no such song 'nope' (available: a, b, m)`,
      });
    }
  });

  test('a ref may be the stem, the stem with .js, or the path', async () => {
    const h = await setup();
    for (const song of ['b', 'b.js', PB]) {
      expect(await h.handle('code.get', { song })).toMatchObject({ song: PB });
    }
    await h.handle('eval', { song: 'b.js' });
    expect(h.st().active).toBe(PB);
    expect((await h.fail('code.get', { song: 'nope.js' })).code).toBe(ErrorCode.NO_SUCH_SONG);
  });

  test('a project with no songs', async () => {
    const h = await setup({ last: '/empty' });
    expect(await h.fail('code.get', {})).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: 'no active song (the project has no songs)',
    });
    expect(await h.fail('eval', { song: 'a' })).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: `no such song 'a' (the project has no songs)`,
    });
  });

  test('no project open: -32004, but stop and state work', async () => {
    const h = await setup({ last: null });
    for (const [method, params] of [
      ['code.get', {}],
      ['code.set', { code: 'x' }],
      ['eval', {}],
      ['play', { song: 'a' }],
    ] as const) {
      expect(await h.fail(method, params)).toEqual({
        code: ErrorCode.NO_PROJECT,
        message: 'no project is open',
      });
    }
    expect(await h.handle('stop', {})).toEqual({ wasPlaying: false });
    expect(await h.handle('state', {})).toMatchObject({ project: null });
  });

  test('code.set, eval and play switch to the named song', async () => {
    const h = await setup();
    await h.handle('code.set', { song: 'b', code: B + '// c', eval: 'never' });
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe(B + '// c');
    await h.handle('eval', { song: PA });
    expect(h.st().active).toBe(PA);
    await h.handle('play', { song: 'b' });
    expect(h.st().active).toBe(PB);
    expect(h.rt.evaluated).toEqual([A, B + '// c']);
  });

  test('a switch that fails is -32006 with the session error', async () => {
    const h = await setup({ failRead: [PB] });
    h.fake.failRead.add(PB);
    for (const [method, params] of [
      ['code.set', { song: 'b', code: 'x' }],
      ['eval', { song: 'b' }],
      ['play', { song: PB }],
    ] as const) {
      const e = await h.fail(method, params);
      expect(e.code).toBe(ErrorCode.SONG_UNLOADABLE);
      expect(e.message).toStartWith(`cannot show '${params.song}': b: `);
      expect(e.message).toContain('permission denied');
    }
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
    expect(h.rt.evaluated).toEqual([]);
  });

  test('a song removed while the switch reads it is -32006; nothing is edited', async () => {
    const h = await setup({ failRead: [PB] });
    const gate = deferred();
    h.fake.holdReads = gate.promise;
    const call = h.fail('code.set', { song: 'b', code: 'x' });
    await flush();
    h.fake.files().delete(PB);
    h.fake.emit({ removed: [PB] });
    h.fake.holdReads = null;
    gate.resolve();
    const e = await call;
    expect(e.code).toBe(ErrorCode.SONG_UNLOADABLE);
    expect(h.doc()).toBe(A);
  });

  test('a song gone right before the edit is -32005', async () => {
    const h = await setup();
    const session: Session = { ...h.session, externalEdit: () => 'missing' };
    const handle = createControlHandler({ session, runtime: h.rt.runtime });
    const reply = await replyTo(handle, { id: 1, method: 'code.set', params: { code: 'x' } });
    expect(reply).toEqual({
      error: { code: ErrorCode.NO_SUCH_SONG, message: `no such song '${PA}' (it was removed)` },
    });
  });
});

// ---------------------------------------------------------------- code.set

describe('code.set', () => {
  test('edits the buffer, marks dirty, does not save; CRLF normalized', async () => {
    const h = await setup();
    const r = await h.handle('code.set', { code: '$: s("z")\r\n' });
    expect(r).toEqual({ song: PA, changed: true, dirty: true, conflict: null, evaluated: null });
    expect(h.doc()).toBe('$: s("z")\n');
    expect(h.fake.files().get(PA)).toBe(A);
    expect(h.fake.calls.filter((c) => c.startsWith('save:'))).toEqual([]);
    expect(h.st().notice).toEqual({ kind: 'claude-edit', path: PA, title: 'a' });
    undo({ state: h.view.state, dispatch: (tr) => h.view.dispatch(tr) });
    expect(h.doc()).toBe(A);
  });

  test('expect: a mismatch is -32010 and changes nothing; a match (CRLF) edits', async () => {
    const h = await setup();
    expect(await h.fail('code.set', { code: 'x', expect: 'old' })).toEqual({
      code: ErrorCode.SONG_CHANGED,
      message: 'the song changed since you read it',
    });
    expect(h.doc()).toBe(A);
    const crlf = A.replace(/\n/g, '\r\n');
    expect(await h.handle('code.set', { code: 'x', expect: crlf })).toMatchObject({
      changed: true,
    });
    expect(h.doc()).toBe('x');
  });

  test('a stale expect for another song fails without switching; a match switches', async () => {
    const h = await setup();
    expect(await h.fail('code.set', { song: 'b', code: 'x', expect: 'old' })).toEqual({
      code: ErrorCode.SONG_CHANGED,
      message: 'the song changed since you read it',
    });
    expect(h.st().active).toBe(PA);
    expect(h.doc()).toBe(A);
    const crlf = B.replace(/\n/g, '\r\n');
    expect(
      await h.handle('code.set', { song: 'b', code: 'x', expect: crlf, eval: 'never' }),
    ).toMatchObject({ song: PB, changed: true });
    expect(h.st().active).toBe(PB);
    expect(h.doc()).toBe('x');
  });

  test('reports a pending disk conflict', async () => {
    const h = await setup();
    h.view.dispatch({ changes: { from: 0, insert: '// mine\n' } });
    h.fake.files().set(PA, '// theirs\n');
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(await h.handle('code.set', { code: 'x' })).toMatchObject({
      changed: true,
      dirty: true,
      conflict: 'changed',
    });
  });

  describe('auto-eval', () => {
    test('re-evaluates when this song is the one playing', async () => {
      const h = await setup();
      await h.handle('eval', {});
      const r = (await h.handle('code.set', { code: A + '// more' })) as any;
      expect(r.evaluated).toEqual({
        ok: true,
        song: PA,
        title: 'Alpha',
        started: true,
        audio: 'running',
        error: null,
      });
      expect(h.rt.evaluated).toEqual([A, A + '// more']);
    });

    test('not while another song plays', async () => {
      const h = await setup();
      await h.handle('eval', { song: 'b' });
      const r = (await h.handle('code.set', { song: 'a', code: 'x' })) as any;
      expect(r.evaluated).toBeNull();
      expect(h.rt.evaluated).toEqual([B]);
      expect(h.st().active).toBe(PA);
    });

    test('not while stopped', async () => {
      const h = await setup();
      await h.handle('eval', {});
      h.rt.runtime.stop();
      const r = (await h.handle('code.set', { code: 'x', eval: 'auto' })) as any;
      expect(r.evaluated).toBeNull();
      expect(h.rt.evaluated).toEqual([A]);
    });

    test('not when the text is identical', async () => {
      const h = await setup();
      await h.handle('eval', {});
      const r = (await h.handle('code.set', { code: A })) as any;
      expect(r).toEqual({
        song: PA,
        changed: false,
        dirty: false,
        conflict: null,
        evaluated: null,
      });
      expect(h.rt.evaluated).toEqual([A]);
      expect(h.st().notice).toBeNull();
    });

    test('always evaluates, even unchanged and stopped; never does not, even playing', async () => {
      const h = await setup();
      const r = (await h.handle('code.set', { code: A, eval: 'always' })) as any;
      expect(r.changed).toBe(false);
      expect(r.evaluated).toMatchObject({ ok: true, song: PA });
      const n = (await h.handle('code.set', { code: 'y', eval: 'never' })) as any;
      expect(n).toMatchObject({ changed: true, evaluated: null });
      expect(h.rt.evaluated).toEqual([A]);
    });
  });

  test('a failed evaluate is a normal result with ok false', async () => {
    const h = await setup();
    h.rt.setResult(false);
    const r = (await h.handle('code.set', { code: 'boom', eval: 'always' })) as any;
    expect(r.changed).toBe(true);
    expect(r.evaluated).toEqual({
      ok: false,
      song: PA,
      title: 'a',
      started: false,
      audio: 'running',
      error: 'boom is not defined',
    });
  });
});

// ---------------------------------------------------------------- eval, play, stop

describe('the evaluate wire shape', () => {
  const WIRE_KEYS = ['audio', 'error', 'ok', 'song', 'started', 'title'];

  test("a session's errorKind never reaches evaluate, play or code set's evaluated", async () => {
    const h = await setup();
    h.session.evaluateBuffer = async (path) => ({
      ok: false,
      song: path,
      title: 'Alpha',
      started: false,
      audio: 'running',
      error: 'boom',
      errorKind: 'eval',
    });
    const e = (await h.handle('eval', {})) as any;
    const p = (await h.handle('play', {})) as any;
    const c = (await h.handle('code.set', { code: A + '// y', eval: 'always' })) as any;
    for (const out of [e, p, c.evaluated]) {
      expect(Object.keys(out).sort()).toEqual(WIRE_KEYS);
      expect(out).toEqual({
        ok: false,
        song: PA,
        title: 'Alpha',
        started: false,
        audio: 'running',
        error: 'boom',
      });
    }
  });
});

describe('eval and play', () => {
  test('evaluate the buffer without saving; both give the same outcome', async () => {
    const h = await setup();
    h.view.dispatch({ changes: { from: h.doc().length, insert: '// x' } });
    const e = await h.handle('eval', {});
    h.rt.runtime.stop();
    const p = await h.handle('play', {});
    const outcome = {
      ok: true,
      song: PA,
      title: 'Alpha',
      started: true,
      audio: 'running',
      error: null,
    };
    expect(e).toEqual(outcome);
    expect(p).toEqual(outcome);
    expect(h.rt.evaluated).toEqual([A + '// x', A + '// x']);
    expect(h.fake.files().get(PA)).toBe(A);
    expect(h.item(PA).dirty).toBe(true);
  });

  test('a meta error and a runtime error are ok false; suspended audio is reported', async () => {
    const h = await setup();
    expect(await h.handle('eval', { song: 'm' })).toMatchObject({ ok: false, song: PM });
    expect(((await h.handle('eval', { song: 'm' })) as any).error).toStartWith('meta: ');
    h.rt.setResult(false);
    expect(await h.handle('play', { song: 'a' })).toMatchObject({
      ok: false,
      error: 'boom is not defined',
    });
    h.rt.setResult(true);
    h.rt.patch({ audio: 'suspended' });
    expect(await h.handle('play', {})).toMatchObject({
      ok: true,
      started: false,
      audio: 'suspended',
    });
  });
});

describe('during memo playback and a take', () => {
  function playback() {
    const log: string[] = [];
    let taking = false;
    const p: ControlPlayback = {
      taking: () => taking,
      stopMemo: () => void log.push('stopMemo'),
      stopTake: async () => void log.push('stopTake'),
    };
    return { p, log, setTaking: (t: boolean) => (taking = t) };
  }

  test('eval, play and code.set stop a playing memo before they evaluate', async () => {
    const pb = playback();
    const h = await setup({ playback: pb.p });
    await h.handle('eval', {});
    await h.handle('play', {});
    await h.handle('code.set', { code: '$: s("c")\n', eval: 'always' });
    expect(pb.log).toEqual(['stopMemo', 'stopMemo', 'stopMemo']);
    expect(h.rt.options.map((o) => o?.autostart)).toEqual([undefined, undefined, undefined]);
  });

  test('during a take an evaluate never starts the song', async () => {
    const pb = playback();
    const h = await setup({ playback: pb.p });
    pb.setTaking(true);
    expect(await h.handle('play', {})).toMatchObject({ ok: true, started: false });
    await h.handle('code.set', { code: '$: s("c")\n', eval: 'always' });
    expect(h.rt.options.map((o) => o?.autostart)).toEqual([false, false]);
    expect(get(h.rt.runtime.store).started).toBe(false);
  });

  test('stop stops a memo, or during a take the take instead of the song', async () => {
    const pb = playback();
    const h = await setup({ playback: pb.p });
    const before = h.rt.stops();
    await h.handle('stop', {});
    expect(pb.log).toEqual(['stopMemo']);
    expect(h.rt.stops()).toBe(before + 1);
    pb.setTaking(true);
    h.rt.setStarted(true);
    expect(await h.handle('stop', {})).toEqual({ wasPlaying: true });
    expect(pb.log).toEqual(['stopMemo', 'stopTake']);
    expect(h.rt.stops()).toBe(before + 1);
  });
});

describe('stop', () => {
  test('reports whether it was playing', async () => {
    const h = await setup();
    const before = h.rt.stops();
    expect(await h.handle('stop', {})).toEqual({ wasPlaying: false });
    await h.handle('play', {});
    expect(await h.handle('stop', {})).toEqual({ wasPlaying: true });
    expect(get(h.rt.runtime.store).started).toBe(false);
    expect(h.rt.stops()).toBe(before + 2);
  });
});

// ---------------------------------------------------------------- params

describe('invalid params', () => {
  const cases: [string, Parameters<ControlHandler>[0], unknown, string][] = [
    ['non-object', 'code.get', [], 'expected an object'],
    ['null', 'eval', null, 'expected an object'],
    ['a string', 'code.set', 'x', 'expected an object'],
    ['unknown key', 'code.get', { song: 'a', foo: 1 }, `unknown parameter 'foo'`],
    ['unknown key on state', 'state', { verbose: true }, `unknown parameter 'verbose'`],
    ['unknown key on stop', 'stop', { now: true }, `unknown parameter 'now'`],
    ['non-object on stop', 'stop', 1, 'expected an object'],
    ['empty song', 'play', { song: '' }, `'song' must be a non-empty string`],
    ['numeric song', 'eval', { song: 3 }, `'song' must be a non-empty string`],
    ['missing code', 'code.set', {}, `'code' must be a string`],
    ['non-string code', 'code.set', { code: 1 }, `'code' must be a string`],
    [
      'bad eval',
      'code.set',
      { code: '', eval: 'alwys' },
      `'eval' must be "auto", "always" or "never"`,
    ],
    ['bad expect', 'code.set', { code: '', expect: 3 }, `'expect' must be a string`],
  ];
  for (const [name, method, params, detail] of cases) {
    test(name, async () => {
      const h = await setup();
      const stops = h.rt.stops();
      expect(await h.fail(method, params)).toEqual({
        code: ErrorCode.INVALID_PARAMS,
        message: `invalid params: ${detail}`,
      });
      expect(h.doc()).toBe(A);
      expect(h.rt.stops()).toBe(stops);
    });
  }

  test('params are checked before the project', async () => {
    const h = await setup({ last: null });
    expect((await h.fail('code.set', {})).code).toBe(ErrorCode.INVALID_PARAMS);
  });

  test('an empty code string is allowed', async () => {
    const h = await setup();
    expect(await h.handle('code.set', { code: '' })).toMatchObject({ changed: true });
    expect(h.doc()).toBe('');
  });
});

// ---------------------------------------------------------------- serial queue and replies

describe('requests', () => {
  test('run one at a time, in order', async () => {
    const h = await setup();
    const gate = deferred();
    h.rt.hold(gate.promise);
    const order: string[] = [];
    const first = h.handle('eval', {}).then(() => order.push('eval'));
    const second = h
      .handle('code.set', { song: 'b', code: 'z', eval: 'never' })
      .then(() => order.push('code.set'));
    const third = h.handle('state', {}).then((s: any) => order.push(`state:${s.active}`));
    await flush();
    expect(order).toEqual([]);
    expect(h.st().active).toBe(PA); // the switch waits behind the evaluate
    gate.resolve();
    await Promise.all([first, second, third]);
    expect(order).toEqual(['eval', 'code.set', `state:${PB}`]);
  });

  test('a request still queued after its timeout is skipped; a started one finishes', async () => {
    const h = await setup();
    let t = 0;
    const handle = createControlHandler({
      session: h.session,
      runtime: h.rt.runtime,
      now: () => t,
    });
    const gate = deferred();
    h.rt.hold(gate.promise);
    const first = handle('eval', {}, 1000); // starts at once, then waits on the evaluate
    const late = replyTo(handle, {
      id: 2,
      method: 'code.set',
      params: { song: 'b', code: 'z', eval: 'never' },
      timeoutMs: 1500,
    });
    const inTime = handle('state', {}, 5000);
    await flush();
    t = 1500;
    gate.resolve();
    expect(await first).toMatchObject({ ok: true }); // past its timeout, but already running
    expect(await late).toEqual({
      error: { code: ErrorCode.TIMEOUT, message: 'the app did not answer in 2s' },
    });
    expect(h.st().active).toBe(PA); // not switched
    expect(h.fake.files().get(PB)).toBe(B);
    expect(h.st().songs.find((s) => s.path === PB)?.dirty).toBe(false);
    expect(await inTime).toMatchObject({ active: PA });
  });

  test('a failed request does not block the next', async () => {
    const h = await setup();
    const bad = h.fail('code.set', {});
    const good = h.handle('code.get', {});
    expect((await bad).code).toBe(ErrorCode.INVALID_PARAMS);
    expect(await good).toMatchObject({ song: PA });
  });

  test('replyTo: a result, a ControlError, and an unexpected throw as -32603', async () => {
    const h = await setup();
    expect(await replyTo(h.handle, { id: 1, method: 'stop', params: {} })).toEqual({
      result: { wasPlaying: false },
    });
    expect(await replyTo(h.handle, { id: 2, method: 'eval', params: { song: '' } })).toEqual({
      error: {
        code: ErrorCode.INVALID_PARAMS,
        message: `invalid params: 'song' must be a non-empty string`,
      },
    });
    h.rt.runtime.stop = () => {
      throw new Error('canvas gone');
    };
    expect(await replyTo(h.handle, { id: 3, method: 'stop', params: {} })).toEqual({
      error: { code: ErrorCode.INTERNAL_ERROR, message: 'internal error: canvas gone' },
    });
    const throwsString: ControlHandler = async () => {
      throw 'plain string';
    };
    expect(await replyTo(throwsString, { id: 4, method: 'state', params: {} })).toEqual({
      error: { code: ErrorCode.INTERNAL_ERROR, message: 'internal error: plain string' },
    });
  });
});

// ---------------------------------------------------------------- events

const { buildStructure } = await import('../src/lib/strudel/structure');
/** intro 1-2, body 3-6, tail 7-8. */
const SONG_STRUCTURE = buildStructure({
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
});

/** A hap-like onset over `[begin, end)`. */
const onset = (begin: number, end: number, value: unknown) => ({
  whole: { begin, end },
  part: { begin, end },
  value,
  context: {},
  hasOnset: () => true,
});

/** The active song evaluated; optionally with a song structure. */
async function evaluated(opts: { structure?: boolean } = {}) {
  const h = await setup();
  await h.handle('eval', {});
  if (opts.structure) h.rt.patch({ structure: SONG_STRUCTURE });
  h.rt.setQuery([]);
  return h;
}

describe('events', () => {
  test('the exact result for a plain pattern: bars 1-4 by default', async () => {
    const h = await evaluated();
    h.rt.setQuery([onset(1.5, 2, { s: 'b', _x: 1 }), onset(0, 1, { s: 'a' })]);
    h.rt.patch({ tempoOverride: true, meter: { num: 3, den: 4 } });
    expect(await h.handle('events', {})).toEqual({
      song: { path: PA, title: 'Alpha' },
      from: 1,
      to: 4,
      stale: false,
      truncated: false,
      meter: '3/4',
      tempoOverride: true,
      events: [
        {
          bar: 1,
          beat: 1,
          begin: 0,
          end: 1,
          onset: true,
          section: null,
          value: { s: 'a' },
          locations: [],
        },
        {
          bar: 2,
          beat: 2.5,
          begin: 1.5,
          end: 2,
          onset: true,
          section: null,
          value: { s: 'b' },
          locations: [],
        },
      ],
    });
    expect(h.rt.queries()).toEqual([[0, 4]]);
  });

  test('a plain pattern: bars are not clamped, and section is rejected', async () => {
    const h = await evaluated();
    expect(await h.handle('events', { bars: { from: 10, to: 12 } })).toMatchObject({
      from: 10,
      to: 12,
    });
    expect(h.rt.queries()).toEqual([[9, 12]]);
    expect(await h.fail('events', { section: 'intro' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: 'invalid params: the evaluated pattern has no sections',
    });
  });

  test('a plain pattern: at most 1024 bars per query', async () => {
    const h = await evaluated();
    expect(await h.handle('events', { bars: { from: 5, to: 1028 } })).toMatchObject({
      from: 5,
      to: 1028,
    });
    expect(await h.fail('events', { bars: { from: 1, to: 4_000_000_000 } })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: 'invalid params: a pattern without song() is queried at most 1024 bars at a time',
    });
    expect(h.rt.queries()).toEqual([[4, 1028]]);
  });

  test('a song: the whole song by default, sections located', async () => {
    const h = await evaluated({ structure: true });
    h.rt.setQuery([onset(2, 3, { s: 'x' }), onset(7.5, 8, { s: 'y' })]);
    const r = (await h.handle('events', {})) as any;
    expect([r.from, r.to]).toEqual([1, 8]);
    expect(h.rt.queries()).toEqual([[0, 8]]);
    expect(r.events.map((e: any) => [e.bar, e.beat, e.section])).toEqual([
      [3, 1, { name: 'body', bar: 1 }],
      [8, 3, { name: 'tail', bar: 2 }],
    ]);
  });

  test('a song: to is clamped, from past the end is rejected', async () => {
    const h = await evaluated({ structure: true });
    expect(await h.handle('events', { bars: { from: 7, to: 20 } })).toMatchObject({
      from: 7,
      to: 8,
    });
    expect(await h.handle('events', { bars: { from: 8, to: 8 } })).toMatchObject({
      from: 8,
      to: 8,
    });
    expect(h.rt.queries()).toEqual([
      [6, 8],
      [7, 8],
    ]);
    expect(await h.fail('events', { bars: { from: 9, to: 9 } })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: 'invalid params: the song has 8 bars',
    });
  });

  test('a section selects its bars; the first one of a name; case-sensitive', async () => {
    const h = await evaluated({ structure: true });
    expect(await h.handle('events', { section: 'body' })).toMatchObject({ from: 3, to: 6 });
    // Names are unique in meta; a hand-built structure checks the first one wins.
    const twice = {
      ...SONG_STRUCTURE,
      sections: [...SONG_STRUCTURE.sections, { name: 'body', start: 8, bars: 1, loop: false }],
      totalBars: 9,
    };
    h.rt.patch({ structure: twice });
    expect(await h.handle('events', { section: 'body' })).toMatchObject({ from: 3, to: 6 });
    expect(await h.fail('events', { section: 'Body' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: unknown section 'Body' (sections: intro, body, tail, body)`,
    });
    h.rt.patch({ structure: SONG_STRUCTURE });
    expect(await h.fail('events', { section: 'x' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: unknown section 'x' (sections: intro, body, tail)`,
    });
  });

  test('all keeps continuations; limit truncates', async () => {
    const h = await evaluated();
    const cont = {
      ...onset(1, 2, { s: 'c' }),
      whole: { begin: 0.5, end: 2 },
      hasOnset: () => false,
    };
    h.rt.setQuery([onset(0, 1, { s: 'a' }), cont, onset(2, 3, { s: 'b' })]);
    const s = async (p: object) =>
      ((await h.handle('events', p)) as any).events.map((e: any) => e.value.s);
    expect(await s({})).toEqual(['a', 'b']);
    expect(await s({ all: true })).toEqual(['a', 'c', 'b']);
    expect(await s({ all: false })).toEqual(['a', 'b']);
    const r = (await h.handle('events', { limit: 1 })) as any;
    expect([r.events.length, r.truncated]).toEqual([1, true]);
    expect(((await h.handle('events', { limit: 2 })) as any).truncated).toBe(false);
  });

  test('nothing evaluated: never, or the runtime has no pattern', async () => {
    const h = await setup();
    const nothing = {
      code: ErrorCode.NOTHING_EVALUATED,
      message: 'nothing has been evaluated yet',
    };
    expect(await h.fail('events', {})).toEqual(nothing);
    await h.handle('eval', {});
    h.rt.setQuery(null);
    expect(await h.fail('events', {})).toEqual(nothing);
    // Evaluated per the session, but the runtime has no code.
    h.rt.setQuery([]);
    h.rt.patch({ activeCode: null });
    expect(await h.fail('events', {})).toEqual(nothing);
  });

  test('nothing evaluated: the evaluated song was removed, or the project changed', async () => {
    const h = await evaluated();
    await h.session.switchSong(PB);
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(await h.fail('events', {})).toEqual({
      code: ErrorCode.NOTHING_EVALUATED,
      message: 'the evaluated song is no longer in the project',
    });
    const g = await evaluated();
    await g.session.openRecent('/empty');
    await flush();
    expect((await g.fail('events', {})).code).toBe(ErrorCode.NOTHING_EVALUATED);
  });

  test('stale: the active buffer against the evaluated text', async () => {
    const h = await evaluated();
    expect(((await h.handle('events', {})) as any).stale).toBe(false);
    h.view.dispatch({ changes: { from: h.doc().length, insert: '// edit' } });
    expect(((await h.handle('events', {})) as any).stale).toBe(true);
  });

  test('stale: a non-active evaluated song through peekSong', async () => {
    const h = await evaluated();
    await h.session.switchSong(PB);
    expect(h.st().active).toBe(PB);
    expect(((await h.handle('events', {})) as any).stale).toBe(false);
    h.fake.files().set(PA, `${A}// theirs\n`);
    h.fake.emit({ changed: [PA] });
    await flush();
    expect(((await h.handle('events', {})) as any).stale).toBe(true);
  });

  test('stale: falls back to the saved text when the active buffer has none', async () => {
    const h = await evaluated();
    h.session.activeText = () => null;
    expect(((await h.handle('events', {})) as any).stale).toBe(false);
  });

  test('a pattern error becomes -32603 with its message', async () => {
    const h = await evaluated();
    h.rt.setQuery(() => {
      throw new Error('tail boom');
    });
    expect(await replyTo(h.handle, { id: 1, method: 'events', params: {} })).toEqual({
      error: { code: ErrorCode.INTERNAL_ERROR, message: 'internal error: tail boom' },
    });
  });

  const bad: [string, unknown, string][] = [
    ['non-object', 3, 'expected an object'],
    ['unknown key', { bar: 1 }, `unknown parameter 'bar'`],
    [
      'bars and section',
      { bars: { from: 1, to: 2 }, section: 'a' },
      'bars and section are mutually exclusive',
    ],
    ['bars not an object', { bars: 3 }, `'bars' must be an object with 'from' and 'to'`],
    [
      'bars extra key',
      { bars: { from: 1, to: 2, by: 1 } },
      `'bars' must be an object with 'from' and 'to'`,
    ],
    ['bars missing to', { bars: { from: 1 } }, `'bars.from' and 'bars.to' must be integers >= 1`],
    ['from < 1', { bars: { from: 0, to: 2 } }, `'bars.from' and 'bars.to' must be integers >= 1`],
    [
      'non-integer',
      { bars: { from: 1.5, to: 2 } },
      `'bars.from' and 'bars.to' must be integers >= 1`,
    ],
    [
      'string bars',
      { bars: { from: '1', to: 2 } },
      `'bars.from' and 'bars.to' must be integers >= 1`,
    ],
    ['from > to', { bars: { from: 3, to: 2 } }, `'bars.from' must not be after 'bars.to'`],
    ['empty section', { section: '' }, `'section' must be a non-empty string`],
    ['numeric section', { section: 1 }, `'section' must be a non-empty string`],
    ['non-boolean all', { all: 'yes' }, `'all' must be a boolean`],
    ['limit 0', { limit: 0 }, `'limit' must be an integer from 1 to 100000`],
    ['limit too large', { limit: 100001 }, `'limit' must be an integer from 1 to 100000`],
    ['fractional limit', { limit: 1.5 }, `'limit' must be an integer from 1 to 100000`],
  ];
  for (const [name, params, detail] of bad) {
    test(`invalid params: ${name}`, async () => {
      const h = await evaluated();
      expect(await h.fail('events', params)).toEqual({
        code: ErrorCode.INVALID_PARAMS,
        message: `invalid params: ${detail}`,
      });
      expect(h.rt.queries()).toEqual([]);
    });
  }

  test('params are checked before anything was evaluated', async () => {
    const h = await setup();
    expect((await h.fail('events', { limit: 0 })).code).toBe(ErrorCode.INVALID_PARAMS);
  });

  test('the largest limit is accepted', async () => {
    const h = await evaluated();
    expect(await h.handle('events', { limit: 100000 })).toMatchObject({ truncated: false });
  });
});

// ---------------------------------------------------------------- state: playhead, transport

describe('state: playhead, transport and outputError', () => {
  test('playhead is null while stopped, even with a position', async () => {
    const h = await setup();
    h.rt.setPosition({ cycle: 3, located: { section: 1, localCycle: 1, songBar: 3 } });
    expect(((await h.handle('state', {})) as any).playhead).toBeNull();
  });

  test('playhead while started: whole song bar and section bar, 1-based', async () => {
    const h = await evaluated({ structure: true });
    h.rt.setPosition({ cycle: 4.25, located: { section: 1, localCycle: 1.25, songBar: 3.25 } });
    expect(((await h.handle('state', {})) as any).playhead).toEqual({
      cycle: 4.25,
      songBar: 4,
      section: { name: 'body', bar: 2 },
    });
    h.rt.setPosition({ cycle: 3, located: { section: 1, localCycle: 1, songBar: 3 } });
    expect(((await h.handle('state', {})) as any).playhead).toEqual({
      cycle: 3,
      songBar: 4,
      section: { name: 'body', bar: 2 },
    });
  });

  test('playhead: cycle only for a plain pattern or past the end', async () => {
    const h = await evaluated();
    h.rt.setPosition({ cycle: 7.5, located: null });
    expect(((await h.handle('state', {})) as any).playhead).toEqual({
      cycle: 7.5,
      songBar: null,
      section: null,
    });
    h.rt.patch({ structure: SONG_STRUCTURE });
    h.rt.setPosition({ cycle: 12, located: null });
    expect(((await h.handle('state', {})) as any).playhead).toEqual({
      cycle: 12,
      songBar: null,
      section: null,
    });
  });

  /** The active song's header names `sections` (single-bar each; `body` is the loop). */
  const sectioned = (...names: string[]) =>
    `const meta = { title: 'Alpha', bpm: 120, sections: [${names
      .map((n) => `{ name: '${n}', bars: 2${n === 'body' ? ', loop: true' : ''} }`)
      .join(', ')}] };\n$: s("a")\n`;
  const SECTIONED = sectioned('intro', 'body', 'tail');

  /** Set the active song's text without evaluating it. */
  const write = (h: Awaited<ReturnType<typeof setup>>, code: string) =>
    h.handle('code.set', { code, eval: 'never' });

  test('transport: song, loop and armed by name, release', async () => {
    const h = await setup();
    const pb = h.session.playback;
    const t = async () => ((await h.handle('state', {})) as any).transport;
    await write(h, SECTIONED);
    expect(await t()).toEqual({ view: 'song', loopSection: null, armedSection: null });
    pb.arm({ start: 'tail' });
    expect(await t()).toEqual({ view: 'song', loopSection: null, armedSection: 'tail' });
    pb.arm({ loop: 'intro' });
    expect(await t()).toEqual({ view: 'loop', loopSection: 'intro', armedSection: 'intro' });
    pb.stopLooping();
    expect(await t()).toEqual({ view: 'song', loopSection: null, armedSection: null });
    // Playing: the loop is named from the evaluated song.
    await h.handle('eval', {});
    h.rt.patch({ structure: SONG_STRUCTURE });
    pb.arm({ loop: 'body' });
    expect(await t()).toEqual({ view: 'loop', loopSection: 'body', armedSection: null });
    h.rt.setPosition({ cycle: 3, located: null });
    h.rt.setHorizon(3);
    pb.stopLooping();
    expect(await t()).toEqual({ view: 'song', loopSection: null, armedSection: null });
    pb.playEnding();
    expect(await t()).toEqual({ view: 'release', loopSection: null, armedSection: null });
    // A stop rewinds the transport.
    await h.handle('stop', {});
    expect(await t()).toEqual({ view: 'song', loopSection: null, armedSection: null });
  });

  describe('the armed section is the one the timeline shows', () => {
    const t = async (h: Awaited<ReturnType<typeof setup>>) =>
      ((await h.handle('state', {})) as any).transport;

    test('with nothing evaluated yet', async () => {
      const h = await setup();
      await write(h, SECTIONED);
      h.session.playback.arm({ loop: 'body' });
      expect(get(h.rt.runtime.store).structure).toBeNull();
      expect(await t(h)).toEqual({ view: 'loop', loopSection: 'body', armedSection: 'body' });
    });

    test('with the header edited after the last evaluate', async () => {
      const h = await setup();
      await write(h, SECTIONED);
      await h.handle('eval', {});
      h.rt.patch({ structure: SONG_STRUCTURE });
      await h.handle('stop', {});
      await write(h, sectioned('intro', 'bridge', 'body', 'tail'));
      h.session.playback.arm({ start: 'bridge' });
      // Index 1 is `body` in the evaluated song; the header armed `bridge`.
      expect(get(h.rt.runtime.transport.store).armedSection).toBe(1);
      expect(await t(h)).toEqual({ view: 'song', loopSection: null, armedSection: 'bridge' });
    });

    test('after a failed evaluate', async () => {
      const h = await setup();
      await write(h, SECTIONED);
      await h.handle('eval', {});
      h.rt.patch({ structure: SONG_STRUCTURE });
      await h.handle('stop', {});
      await write(h, sectioned('bridge', 'intro', 'body', 'tail'));
      h.session.playback.arm({ start: 'tail' });
      h.rt.setResult(false);
      expect(((await h.handle('eval', {})) as any).ok).toBe(false);
      expect(await t(h)).toEqual({ view: 'song', loopSection: null, armedSection: 'tail' });
    });

    test('after a header edit while armed: it follows the name, or disarms', async () => {
      const h = await setup();
      await write(h, SECTIONED);
      h.session.playback.arm({ start: 'body' });
      await write(h, sectioned('intro', 'tail', 'body'));
      expect(get(h.rt.runtime.transport.store).armedSection).toBe(2);
      expect(await t(h)).toEqual({ view: 'song', loopSection: null, armedSection: 'body' });
      await write(h, sectioned('intro', 'tail', 'chorus'));
      expect(get(h.rt.runtime.transport.store).pendingStart).toBe(false);
      expect(await t(h)).toEqual({ view: 'song', loopSection: null, armedSection: null });
    });

    test('the same names whenever the header and the evaluated song agree', async () => {
      const h = await setup();
      await write(h, SECTIONED);
      await h.handle('eval', {});
      h.rt.patch({ structure: SONG_STRUCTURE });
      await h.handle('stop', {});
      h.session.playback.arm({ start: 'body' });
      const ts = get(h.rt.runtime.transport.store);
      expect(SONG_STRUCTURE.sections[ts.armedSection!]!.name).toBe('body');
      expect(await t(h)).toEqual({ view: 'song', loopSection: null, armedSection: 'body' });
    });
  });

  test('state reports the active meter and both endpoints of an audition range', async () => {
    const h = await evaluated({ structure: true });
    const structure = buildStructure({
      sections: [
        { name: 'intro', bars: 2 },
        { name: 'body', bars: 4 },
      ],
      meterChanges: [{ bar: 3, meter: '3/8' }],
    });
    await write(
      h,
      `const meta = { title: 'Alpha', bpm: 120, sections: [{ name: 'intro', bars: 2 }, { name: 'body', bars: 4 }], meterChanges: [{ bar: 3, meter: '3/8' }] };\n$: s("a")\n`,
    );
    h.rt.patch({ structure });
    h.rt.setPosition({ cycle: 3, located: { section: 1, localCycle: 1, songBar: 3 } });
    h.session.playback.arm({ loop: 'intro', through: 'body' });
    expect(await h.handle('state', {})).toMatchObject({
      meter: '3/8',
      cps: 4 / 3,
      bpm: 120,
      transport: { view: 'loop', loopSection: 'intro', loopThrough: 'body', armedSection: null },
    });
  });

  test('engine and live come through', async () => {
    const h = await setup();
    const live = {
      underruns: 1,
      lateHaps: 2,
      haps: 3,
      batches: 4,
      skippedHaps: 5,
      renderHeadroomMs: 38,
      batchDecodeUs: { p50: 100, max: 200 },
      invokeRttMs: { p50: 1, p95: 4, max: 9 },
    };
    h.rt.patch({ engine: 'rust', live });
    const state = (await h.handle('state', {})) as any;
    expect(state.engine).toBe('rust');
    expect(state.live).toEqual(live);
  });

  test('levels: null before a frame, a summary after, the limiter follows the store', async () => {
    const h = await setup();
    expect(((await h.handle('state', {})) as any).levels).toBeNull();
    h.rt.setLevels({ peak: [0, 1.274], overs: 12, limitDb: -2.3 });
    expect(((await h.handle('state', {})) as any).levels).toEqual({
      peakDb: [null, 2.1],
      overs: 12,
      limitDb: -2.3,
      limiter: false,
    });
    h.rt.patch({ limiter: true });
    h.rt.setLevels({ peak: [0.5, 0.0001], overs: 0, limitDb: 0 });
    expect(((await h.handle('state', {})) as any).levels).toEqual({
      peakDb: [-6, -80],
      overs: 0,
      limitDb: 0,
      limiter: true,
    });
    // f32 noise in the gain reduction is rounded like the peaks; a non-finite peak is full scale.
    h.rt.setLevels({ peak: [Infinity, 0.5], overs: 1, limitDb: -2.299999952316284 });
    expect(((await h.handle('state', {})) as any).levels).toEqual({
      peakDb: [0, -6],
      overs: 1,
      limitDb: -2.3,
      limiter: true,
    });
  });

  test('outputError comes through', async () => {
    const h = await setup();
    h.rt.patch({ outputError: 'sound nope not found! Is it loaded?' });
    expect(((await h.handle('state', {})) as any).outputError).toBe(
      'sound nope not found! Is it loaded?',
    );
  });
});

// ---------------------------------------------------------------- song and project operations

const dirtyActive = (h: Awaited<ReturnType<typeof setup>>) =>
  h.view.dispatch({ changes: { from: h.doc().length, insert: '// x' } });

/** The state summary, to prove a failed call changed nothing. */
const snapshot = async (h: Awaited<ReturnType<typeof setup>>) => ({
  state: await h.handle('state', {}),
  files: [...h.fake.files().entries()],
});

describe('song.new', () => {
  test('creates the song, switches to it and returns its ref', async () => {
    const h = await setup();
    expect(await h.handle('song.new', { name: 'intro' })).toEqual({
      song: { path: 'songs/intro.js', title: 'intro' },
    });
    expect(h.st().active).toBe('songs/intro.js');
    expect(h.fake.files().has('songs/intro.js')).toBe(true);
  });

  test('an existing song is SONG_EXISTS', async () => {
    const h = await setup();
    const before = await snapshot(h);
    expect(await h.fail('song.new', { name: 'a' })).toEqual({
      code: ErrorCode.SONG_EXISTS,
      message: 'song songs/a.js already exists',
    });
    expect(await snapshot(h)).toEqual(before);
  });

  test('a bad stem is INVALID_PARAMS with the library message', async () => {
    const h = await setup();
    expect(await h.fail('song.new', { name: 'a/b' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message:
        'invalid params: invalid song name "a/b" (use letters, digits, space, - and _; 1 to 64 characters)',
    });
  });

  test('the name is not trimmed or stripped of .js', async () => {
    const h = await setup();
    expect((await h.fail('song.new', { name: 'x.js' })).code).toBe(ErrorCode.INVALID_PARAMS);
    expect((await h.fail('song.new', { name: ' x' })).code).toBe(ErrorCode.INVALID_PARAMS);
  });

  test('params: missing, wrong type, unknown key', async () => {
    const h = await setup();
    expect((await h.fail('song.new', {})).message).toBe(`invalid params: 'name' must be a string`);
    expect((await h.fail('song.new', { name: 1 })).code).toBe(ErrorCode.INVALID_PARAMS);
    expect((await h.fail('song.new', { name: 'x', foo: 1 })).message).toBe(
      `invalid params: unknown parameter 'foo'`,
    );
  });

  test('no project open', async () => {
    const h = await setup({ last: null });
    expect(await h.fail('song.new', { name: 'x' })).toEqual({
      code: ErrorCode.NO_PROJECT,
      message: 'no project is open',
    });
  });

  test('a project change mid-call is an internal error', async () => {
    const h = await setup();
    const release = deferred();
    h.fake.holdReads = release.promise;
    const call = h.fail('song.new', { name: 'x' });
    await flush();
    await h.session.openPath('/empty', { discard: true });
    h.fake.holdReads = null;
    release.resolve();
    expect(await call).toEqual({
      code: ErrorCode.INTERNAL_ERROR,
      message: 'the project changed; try again',
    });
  });

  test('a song that cannot be read back is an internal error, not a hang', async () => {
    const h = await setup();
    const orig = h.api.createSong;
    h.api.createSong = async (root, stem) => {
      const p = await orig(root, stem);
      h.fake.failRead.add(p);
      return p;
    };
    const r = await replyTo(h.handle, { id: 1, method: 'song.new', params: { name: 'bad' } });
    expect(r).toEqual({
      error: {
        code: ErrorCode.INTERNAL_ERROR,
        message: 'internal error: songs/bad.js: permission denied',
      },
    });
  });
});

describe('song.switch', () => {
  test('shows the song and returns its ref', async () => {
    const h = await setup();
    expect(await h.handle('song.switch', { song: 'b' })).toEqual({
      song: { path: PB, title: 'b' },
    });
    expect(h.st().active).toBe(PB);
  });

  test('accepts a path and a stem with .js; switching to the active song is fine', async () => {
    const h = await setup();
    expect(await h.handle('song.switch', { song: 'songs/a.js' })).toEqual({
      song: { path: PA, title: 'Alpha' },
    });
    expect(await h.handle('song.switch', { song: 'b.js' })).toMatchObject({ song: { path: PB } });
  });

  test('song is required', async () => {
    const h = await setup();
    expect(await h.fail('song.switch', {})).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: 'song' is required`,
    });
  });

  test('unknown song', async () => {
    const h = await setup();
    expect(await h.fail('song.switch', { song: 'zz' })).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: `no such song 'zz' (available: a, b, m)`,
    });
  });

  test('an unreadable song is SONG_UNLOADABLE and the active song stays', async () => {
    const h = await setup({ failRead: [PB] });
    h.fake.failRead.add(PB);
    const r = await h.fail('song.switch', { song: 'b' });
    expect(r.code).toBe(ErrorCode.SONG_UNLOADABLE);
    expect(h.st().active).toBe(PA);
  });

  test('no project open; unknown key', async () => {
    const h = await setup({ last: null });
    expect((await h.fail('song.switch', { song: 'a' })).code).toBe(ErrorCode.NO_PROJECT);
    expect((await h.fail('song.switch', { song: 'a', x: 1 })).code).toBe(ErrorCode.INVALID_PARAMS);
  });
});

describe('song.rename', () => {
  test('renames the active song by default and keeps unsaved edits', async () => {
    const h = await setup();
    dirtyActive(h);
    expect(await h.handle('song.rename', { name: 'intro' })).toEqual({
      from: { path: PA, title: 'Alpha' },
      to: { path: 'songs/intro.js', title: 'Alpha' },
    });
    expect(h.st().active).toBe('songs/intro.js');
    expect(h.item('songs/intro.js').dirty).toBe(true);
    expect(h.doc()).toBe(A + '// x');
  });

  test('a named song; the title follows the stem without meta.title', async () => {
    const h = await setup();
    expect(await h.handle('song.rename', { song: 'b', name: 'c' })).toEqual({
      from: { path: PB, title: 'b' },
      to: { path: 'songs/c.js', title: 'c' },
    });
    expect(h.st().active).toBe(PA); // does not switch the editor
  });

  test('renaming to its own stem is INVALID_PARAMS', async () => {
    const h = await setup();
    expect(await h.fail('song.rename', { song: 'b', name: 'b' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: song 'b' is already named 'b'`,
    });
  });

  test('a case-only rename succeeds', async () => {
    const h = await setup();
    expect(await h.handle('song.rename', { song: 'b', name: 'B' })).toMatchObject({
      to: { path: 'songs/B.js' },
    });
  });

  test('onto an existing song is SONG_EXISTS and changes nothing', async () => {
    const h = await setup();
    const before = await snapshot(h);
    expect(await h.fail('song.rename', { song: 'b', name: 'a' })).toEqual({
      code: ErrorCode.SONG_EXISTS,
      message: 'song songs/a.js already exists',
    });
    expect(await snapshot(h)).toEqual(before);
  });

  test('an invalid stem is INVALID_PARAMS with the library message', async () => {
    const h = await setup();
    const r = await h.fail('song.rename', { song: 'b', name: 'a/b' });
    expect(r.code).toBe(ErrorCode.INVALID_PARAMS);
    expect(r.message).toStartWith('invalid params: invalid song name "a/b"');
  });

  test('unknown song; no active song; no project', async () => {
    const h = await setup();
    expect((await h.fail('song.rename', { song: 'zz', name: 'x' })).code).toBe(
      ErrorCode.NO_SUCH_SONG,
    );
    expect(
      (await setup({ last: null }).then((e) => e.fail('song.rename', { name: 'x' }))).code,
    ).toBe(ErrorCode.NO_PROJECT);
    const e = await setup({ last: '/empty' });
    expect(await e.fail('song.rename', { name: 'x' })).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: 'no active song (the project has no songs)',
    });
  });

  test('a song deleted on disk (deleted conflict) is NO_SUCH_SONG and says to save it first', async () => {
    const h = await setup();
    dirtyActive(h);
    h.fake.files().delete(PA);
    h.fake.emit({ removed: [PA] });
    await flush();
    expect(await h.fail('song.rename', { name: 'x' })).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: `song '${PA}' was deleted on disk; save it to recreate the file, then rename it`,
    });
  });

  test('a file gone from disk is NO_SUCH_SONG removed (songNotFound)', async () => {
    const h = await setup();
    h.fake.files().delete(PB); // no watcher event yet
    expect(await h.fail('song.rename', { song: 'b', name: 'x' })).toEqual({
      code: ErrorCode.NO_SUCH_SONG,
      message: `no such song '${PB}' (it was removed)`,
    });
  });

  test('params: missing name, wrong types, unknown key', async () => {
    const h = await setup();
    expect((await h.fail('song.rename', { song: 'b' })).message).toBe(
      `invalid params: 'name' must be a string`,
    );
    expect((await h.fail('song.rename', { song: 'b', name: 3 })).code).toBe(
      ErrorCode.INVALID_PARAMS,
    );
    expect((await h.fail('song.rename', { name: 'x', foo: 1 })).code).toBe(
      ErrorCode.INVALID_PARAMS,
    );
  });
});

describe('song.delete', () => {
  test('deletes a clean song and returns its ref', async () => {
    const h = await setup();
    expect(await h.handle('song.delete', { song: 'b' })).toEqual({
      song: { path: PB, title: 'b' },
      stopped: false,
    });
    expect(h.fake.files().has(PB)).toBe(false);
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PM]);
  });

  test('defaults to the active song, which moves to another', async () => {
    const h = await setup();
    expect(await h.handle('song.delete', {})).toMatchObject({ song: { path: PA } });
    expect(h.st().active).toBe(PB);
  });

  test('stopped is true for the playing song', async () => {
    const h = await setup();
    await h.handle('play', {});
    expect(await h.handle('song.delete', { song: 'a' })).toEqual({
      song: { path: PA, title: 'Alpha' },
      stopped: true,
    });
    expect(get(h.rt.runtime.store).started).toBe(false);
  });

  test('stopped is false when another song plays', async () => {
    const h = await setup();
    await h.handle('play', {});
    expect(await h.handle('song.delete', { song: 'b' })).toMatchObject({ stopped: false });
    expect(get(h.rt.runtime.store).started).toBe(true);
  });

  test('unsaved edits fail with UNSAVED unless discard', async () => {
    const h = await setup();
    dirtyActive(h);
    const before = await snapshot(h);
    expect(await h.fail('song.delete', {})).toEqual({
      code: ErrorCode.UNSAVED,
      message: `'a' has unsaved changes (use discard to delete it anyway)`,
    });
    expect(await snapshot(h)).toEqual(before);
    expect(await h.handle('song.delete', { discard: true })).toMatchObject({
      song: { path: PA },
    });
    expect(h.fake.files().has(PA)).toBe(false);
  });

  test('discard: false behaves as absent', async () => {
    const h = await setup();
    dirtyActive(h);
    expect((await h.fail('song.delete', { discard: false })).code).toBe(ErrorCode.UNSAVED);
  });

  test('unknown song; a file already gone counts as deleted', async () => {
    const h = await setup();
    expect((await h.fail('song.delete', { song: 'zz' })).code).toBe(ErrorCode.NO_SUCH_SONG);
    h.fake.files().delete(PB); // no watcher event yet
    expect(await h.handle('song.delete', { song: 'b' })).toEqual({
      song: { path: PB, title: 'b' },
      stopped: false,
    });
    expect(h.st().songs.map((s) => s.path)).toEqual([PA, PM]);
  });

  test('params and no project', async () => {
    const h = await setup();
    expect(await h.fail('song.delete', { discard: 'yes' })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: 'discard' must be a boolean`,
    });
    expect((await h.fail('song.delete', { x: 1 })).code).toBe(ErrorCode.INVALID_PARAMS);
    const e = await setup({ last: null });
    expect((await e.fail('song.delete', {})).code).toBe(ErrorCode.NO_PROJECT);
  });
});

describe('save', () => {
  test('defaults to the active song', async () => {
    const h = await setup();
    dirtyActive(h);
    expect(await h.handle('save', {})).toEqual({ saved: [{ path: PA, title: 'Alpha' }] });
    expect(h.fake.files().get(PA)).toBe(A + '// x');
    expect(h.item(PA).dirty).toBe(false);
  });

  test('nothing to save is an empty list', async () => {
    const h = await setup();
    expect(await h.handle('save', {})).toEqual({ saved: [] });
    expect(h.fake.calls.some((c) => c.startsWith('save:'))).toBe(false);
  });

  test('a named song', async () => {
    const h = await setup();
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    await h.session.switchSong(PA);
    expect(await h.handle('save', { song: 'b' })).toEqual({
      saved: [{ path: PB, title: 'b' }],
    });
  });

  test('all saves every dirty song in list order', async () => {
    const h = await setup();
    dirtyActive(h);
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    expect(await h.handle('save', { all: true })).toEqual({
      saved: [
        { path: PA, title: 'Alpha' },
        { path: PB, title: 'b' },
      ],
    });
  });

  test('all with nothing dirty; all: false means the active song', async () => {
    const h = await setup();
    expect(await h.handle('save', { all: true })).toEqual({ saved: [] });
    dirtyActive(h);
    expect(await h.handle('save', { all: false })).toMatchObject({ saved: [{ path: PA }] });
  });

  test('song and all are mutually exclusive', async () => {
    const h = await setup();
    expect(await h.fail('save', { song: 'a', all: true })).toEqual({
      code: ErrorCode.INVALID_PARAMS,
      message: `invalid params: 'song' and 'all' are mutually exclusive`,
    });
  });

  test('song with all: false saves that song', async () => {
    const h = await setup();
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    await h.session.switchSong(PA);
    expect(await h.handle('save', { song: 'b', all: false })).toEqual({
      saved: [{ path: PB, title: 'b' }],
    });
  });

  test('params, unknown song, no project', async () => {
    const h = await setup();
    expect((await h.fail('save', { all: 1 })).message).toBe(
      `invalid params: 'all' must be a boolean`,
    );
    expect((await h.fail('save', { x: 1 })).code).toBe(ErrorCode.INVALID_PARAMS);
    expect((await h.fail('save', { song: 'zz' })).code).toBe(ErrorCode.NO_SUCH_SONG);
    const e = await setup({ last: null });
    expect((await e.fail('save', {})).code).toBe(ErrorCode.NO_PROJECT);
    expect((await e.fail('save', { all: true })).code).toBe(ErrorCode.NO_PROJECT);
  });

  test('a failed write names the song and the ones saved before it', async () => {
    const h = await setup();
    dirtyActive(h);
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    h.fake.failSave.add(PB);
    const r = await h.fail('save', { all: true });
    expect(r.code).toBe(ErrorCode.INTERNAL_ERROR);
    expect(r.message).toStartWith(`could not save 'b': `);
    expect(r.message).toEndWith(' (saved: a)');
    expect(h.item(PA).dirty).toBe(false);
    expect(h.item(PB).dirty).toBe(true);
  });

  test('a failed write with nothing saved before has no suffix', async () => {
    const h = await setup();
    dirtyActive(h);
    h.fake.failSave.add(PA);
    const r = await h.fail('save', {});
    expect(r.message).toStartWith(`could not save 'a': `);
    expect(r.message).not.toContain('(saved');
  });
});

describe('project.open', () => {
  test('opens another project', async () => {
    const h = await setup();
    expect(await h.handle('project.open', { path: '/empty' })).toEqual({
      project: { name: 'Empty', root: '/empty' },
      alreadyOpen: false,
    });
    expect(h.st().project?.root).toBe('/empty');
  });

  test('works with no project open', async () => {
    const h = await setup({ last: null });
    expect(await h.handle('project.open', { path: '/p' })).toEqual({
      project: { name: 'Proj', root: '/p' },
      alreadyOpen: false,
    });
  });

  test('the open project is alreadyOpen and keeps edits', async () => {
    const h = await setup();
    dirtyActive(h);
    expect(await h.handle('project.open', { path: '/p' })).toEqual({
      project: { name: 'Proj', root: '/p' },
      alreadyOpen: true,
    });
    expect(h.item(PA).dirty).toBe(true);
  });

  test('unsaved edits: UNSAVED naming one song, then several', async () => {
    const h = await setup();
    dirtyActive(h);
    const before = await snapshot(h);
    expect(await h.fail('project.open', { path: '/empty' })).toEqual({
      code: ErrorCode.UNSAVED,
      message: '1 song has unsaved changes: a (use discard to open anyway)',
    });
    expect(await snapshot(h)).toEqual(before);
    await h.session.switchSong(PB);
    h.view.dispatch({ changes: { from: 0, insert: '// y\n' } });
    expect(await h.fail('project.open', { path: '/empty' })).toEqual({
      code: ErrorCode.UNSAVED,
      message: '2 songs have unsaved changes: a, b (use discard to open anyway)',
    });
  });

  test('discard opens anyway', async () => {
    const h = await setup();
    dirtyActive(h);
    expect(await h.handle('project.open', { path: '/empty', discard: true })).toMatchObject({
      alreadyOpen: false,
    });
    expect(h.st().project?.root).toBe('/empty');
  });

  test('a path that is not a project is OPEN_FAILED and nothing changes', async () => {
    const h = await setup();
    const before = await snapshot(h);
    expect(await h.fail('project.open', { path: '/nope' })).toEqual({
      code: ErrorCode.OPEN_FAILED,
      message: '/nope is not a strudeland project (no project.json)',
    });
    expect(await snapshot(h)).toEqual(before);
  });

  test('BUSY while a UI open waits on a dialog', async () => {
    const h = await setup();
    const release = deferred();
    h.fake.holdPicks = release.promise;
    const ui = h.session.openProject();
    await flush();
    expect(await h.fail('project.open', { path: '/empty' })).toEqual({
      code: ErrorCode.BUSY,
      message: 'the app is waiting on an open dialog; try again after it closes',
    });
    release.resolve();
    await ui;
  });

  test('params: missing path, wrong types, unknown key', async () => {
    const h = await setup();
    expect((await h.fail('project.open', {})).message).toBe(
      `invalid params: 'path' must be a string`,
    );
    expect((await h.fail('project.open', { path: '/p', discard: 1 })).message).toBe(
      `invalid params: 'discard' must be a boolean`,
    );
    expect((await h.fail('project.open', { path: '/p', x: 1 })).code).toBe(
      ErrorCode.INVALID_PARAMS,
    );
  });
});

describe('instrument.apply', () => {
  const pins = {
    lead: {
      schema: 1 as const,
      parameters: {},
      voices: [{ name: 'lead', chain: [{ control: 's', value: 'sine' }] }],
    },
  };
  const from = A.indexOf('s("a")');
  const to = from + 's("a")'.length;
  const request = { root: '/p', song: PA, id: 'lead', from, to, expect: A };

  test('applies the current project pin in one undo step without saving or evaluating', async () => {
    const h = await setup();
    h.fake.instrumentSnapshots.set('/p', pins);
    h.view.dispatch({ selection: { anchor: to, head: from } });
    expect(h.session.instrumentSelection()).toEqual({ root: '/p', song: PA, from, to, expect: A });
    const insert = '(s("a")).apply(instruments.lead())';
    expect(await h.handle('instrument.apply', request)).toEqual({
      song: PA,
      from,
      to: from + insert.length,
    });
    expect(h.doc()).toBe(A.slice(0, from) + insert + A.slice(to));
    expect(h.item(PA).dirty).toBe(true);
    expect(get(h.session.playback.store).evaluated).toBeNull();
    expect(await h.api.readSong('/p', PA)).toBe(A);
    expect(undo(h.view)).toBe(true);
    expect(h.doc()).toBe(A);
    h.view.destroy();
  });

  test('refuses invalid ranges, a missing pin, and stale or mismatched targets', async () => {
    const h = await setup();
    expect((await h.fail('instrument.apply', request)).code).toBe(ErrorCode.INVALID_PARAMS);
    h.fake.instrumentSnapshots.set('/p', pins);
    expect((await h.fail('instrument.apply', { ...request, from: from + 3 })).code).toBe(
      ErrorCode.INVALID_PARAMS,
    );
    expect((await h.fail('instrument.apply', { ...request, root: '/empty' })).code).toBe(
      ErrorCode.NO_PROJECT,
    );
    expect((await h.fail('instrument.apply', { ...request, song: PB })).code).toBe(
      ErrorCode.NO_SUCH_SONG,
    );
    expect((await h.fail('instrument.apply', { ...request, expect: A + '\n' })).code).toBe(
      ErrorCode.SONG_CHANGED,
    );
    expect(h.doc()).toBe(A);
    h.view.destroy();
  });

  test('a browser action rejects a pin that changed after it was displayed', async () => {
    const h = await setup();
    h.fake.instrumentSnapshots.set('/p', pins);
    await expect(
      h.session.applyInstrument({ ...request, pinFingerprint: 'stale' }),
    ).rejects.toThrow('The project pin changed.');
    expect(h.doc()).toBe(A);
    h.view.destroy();
  });

  test('uses the fresh snapshot on the next evaluation without waiting for the watcher', async () => {
    const h = await setup();
    h.fake.instrumentSnapshots.set('/p', pins);
    await h.handle('instrument.apply', request);
    await h.handle('eval', {});
    expect(h.rt.options.at(-1)?.instruments).toEqual(pins);
    h.view.destroy();
  });

  test('waits behind a delete already operating on the same song', async () => {
    const h = await setup();
    const pending = deferred();
    h.fake.holdDeletes = pending.promise;
    h.fake.instrumentSnapshots.set('/p', pins);
    const deleting = h.session.deleteSong(PA, { discard: false });
    await flush();
    const applying = h.session.applyInstrument(request);
    pending.resolve();
    expect(await deleting).toEqual({ ok: true, stopped: false });
    await expect(applying).rejects.toThrow('Open the requested song');
    expect(h.st().active).toBe(PB);
    h.view.destroy();
  });

  test('a pending edit cannot dirty the song after a UI open has checked it', async () => {
    const h = await setup();
    h.fake.instrumentSnapshots.set('/p', pins);
    const pending = deferred();
    const read = h.api.instruments;
    h.api.instruments = async (root, pin) => {
      await pending.promise;
      return read(root, pin);
    };
    const applying = h.fail('instrument.apply', request);
    await flush();
    const opening = h.session.openRecent('/empty');
    await flush();
    pending.resolve();
    expect((await applying).message).toContain('A project change is in progress');
    await opening;
    expect(h.st().project?.root).toBe('/empty');
    h.view.destroy();
  });

  test('rechecks the buffer after reading pins', async () => {
    const h = await setup();
    const pending = deferred();
    h.fake.instrumentSnapshots.set('/p', pins);
    const read = h.api.instruments;
    h.api.instruments = async (root, pin) => {
      await pending.promise;
      return read(root, pin);
    };
    const applying = h.fail('instrument.apply', request);
    await flush();
    h.view.dispatch({ changes: { from: A.length, insert: '// new edit' } });
    pending.resolve();
    expect((await applying).code).toBe(ErrorCode.SONG_CHANGED);
    expect(h.doc()).toBe(A + '// new edit');
    h.view.destroy();
  });
});

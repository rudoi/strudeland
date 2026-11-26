// The webview side of the control socket: validates a request's params, resolves the target song
// against the session and runs the method. Pure over `Session` and `Runtime` (no Tauri, no DOM), so
// it runs under `bun test`.

import { get } from 'svelte/store';
import {
  errorMessage,
  fileStem,
  InstrumentApplyError,
  shouldAutoEval,
  toLF,
  type InstrumentApplyResult,
  type Session,
  type SessionState,
} from '../project/session';
import { ProjectError } from '../project/api';
import type { PlayArbiter } from '../play-arbiter';
import type { PlaybackTransport } from '../project/song-playback';
import { songPath, stripSongExt } from '../project/ui';
import { peakDb, roundDb, type LevelHoldSnapshot } from '../levels/levels';
import type { Runtime } from '../strudel/runtime';
import {
  bpmFromCps,
  cpsAtMeter,
  isInt,
  isPlainObject,
  meterAtBar,
  meterString,
  type SongStructure,
} from '../strudel/structure';
import { barPosition, toEvents } from './events';
import {
  ControlError,
  ErrorCode,
  type AppStateSummary,
  type CodeSetResult,
  type ControlReply,
  type ControlRequest,
  type EvalOutcome,
  type EventsResult,
  type LevelsSummary,
  type Playhead,
  type ProjectOpenResult,
  type SaveResult,
  type SongDeleteResult,
  type SongRef,
  type SongRenameResult,
  type SongResult,
  type TransportSummary,
} from './protocol';

/** The held levels as the state reports them: peaks in dBFS rounded to 0.1, null for silence. */
export function levelsSummary(
  snap: LevelHoldSnapshot | null,
  limiter: boolean,
): LevelsSummary | null {
  if (snap === null) return null;
  const db = (p: number) => {
    const d = peakDb(p);
    return d === null ? null : roundDb(d);
  };
  return {
    peakDb: [db(snap.peak[0]), db(snap.peak[1])],
    overs: snap.overs,
    limitDb: roundDb(snap.limitDb),
    limiter,
  };
}

/**
 * Runs one method; throws a `ControlError` for a request it cannot carry out. A request given a
 * `timeoutMs` that is still queued when that time has passed is skipped with `-32002` (Rust has
 * already told the client it timed out); one that has started runs to the end.
 */
export type ControlHandler = (
  method: ControlRequest['method'],
  params: unknown,
  timeoutMs?: number,
) => Promise<unknown>;

const invalid = (detail: string) =>
  new ControlError(ErrorCode.INVALID_PARAMS, `invalid params: ${detail}`);
const removed = (target: string) =>
  new ControlError(ErrorCode.NO_SUCH_SONG, `no such song '${target}' (it was removed)`);
const songChanged = () =>
  new ControlError(ErrorCode.SONG_CHANGED, 'the song changed since you read it');
const unloadable = (label: string, detail: string) =>
  new ControlError(ErrorCode.SONG_UNLOADABLE, `${label}: ${detail}`);

/** `params` as an object whose keys are all in `allowed`. */
function object(params: unknown, allowed: readonly string[]): Record<string, unknown> {
  if (!isPlainObject(params)) throw invalid('expected an object');
  for (const key of Object.keys(params)) {
    if (!allowed.includes(key)) throw invalid(`unknown parameter '${key}'`);
  }
  return params;
}

function optSong(p: Record<string, unknown>): string | undefined {
  if (p.song === undefined) return undefined;
  if (typeof p.song !== 'string' || p.song === '') {
    throw invalid(`'song' must be a non-empty string`);
  }
  return p.song;
}

function requireString(p: Record<string, unknown>, key: string): string {
  const v = p[key];
  if (typeof v !== 'string') throw invalid(`'${key}' must be a string`);
  return v;
}

function optString(p: Record<string, unknown>, key: string): string | undefined {
  return p[key] === undefined ? undefined : requireString(p, key);
}

function optBool(p: Record<string, unknown>, key: string): boolean | undefined {
  const v = p[key];
  if (v !== undefined && typeof v !== 'boolean') throw invalid(`'${key}' must be a boolean`);
  return v;
}

const projectChanged = () =>
  new ControlError(ErrorCode.INTERNAL_ERROR, 'the project changed; try again');

/** The control error for a song operation's shared failure reasons. */
const songGone = (reason: 'projectChanged' | 'missing' | 'deleted', target: string) =>
  reason === 'projectChanged' ? projectChanged() : removed(target);

/** The control error for a failed project command; anything else is returned as is. */
function fromProjectError(e: unknown, target?: string): unknown {
  if (!(e instanceof ProjectError)) return e;
  switch (e.kind) {
    case 'exists':
      return new ControlError(ErrorCode.SONG_EXISTS, e.message);
    case 'invalidName':
      return invalid(e.message);
    case 'notFound':
      return target === undefined
        ? new ControlError(ErrorCode.NO_SUCH_SONG, e.message)
        : removed(target);
    case 'projectChanged':
      return projectChanged();
    default:
      return e;
  }
}

const needProject = (session: Session): void => {
  if (!get(session.store).project) {
    throw new ControlError(ErrorCode.NO_PROJECT, 'no project is open');
  }
};

/** The `song` param of a method that takes nothing else. */
const songParam = (params: unknown) => optSong(object(params, ['song']));

/** Default `events` limit; the largest one allowed. */
const EVENTS_LIMIT = 2000;
const EVENTS_LIMIT_MAX = 100000;
/** The widest `events` range for a pattern without `song()`, which has no last bar to clamp to. */
const PLAIN_BARS_MAX = 1024;

/** Validated `events` params. */
function eventsParams(params: unknown): {
  bars: { from: number; to: number } | undefined;
  section: string | undefined;
  all: boolean;
  limit: number;
} {
  const p = object(params, ['bars', 'section', 'all', 'limit']);
  let bars: { from: number; to: number } | undefined;
  if (p.bars !== undefined) {
    const b = p.bars;
    if (!isPlainObject(b) || Object.keys(b).some((k) => k !== 'from' && k !== 'to')) {
      throw invalid(`'bars' must be an object with 'from' and 'to'`);
    }
    if (!isInt(b.from, 1, Number.MAX_SAFE_INTEGER) || !isInt(b.to, 1, Number.MAX_SAFE_INTEGER)) {
      throw invalid(`'bars.from' and 'bars.to' must be integers >= 1`);
    }
    if (b.from > b.to) throw invalid(`'bars.from' must not be after 'bars.to'`);
    bars = { from: b.from, to: b.to };
  }
  if (p.section !== undefined && (typeof p.section !== 'string' || p.section === '')) {
    throw invalid(`'section' must be a non-empty string`);
  }
  const section = p.section as string | undefined;
  if (bars && section !== undefined) throw invalid('bars and section are mutually exclusive');
  if (p.all !== undefined && typeof p.all !== 'boolean') throw invalid(`'all' must be a boolean`);
  if (p.limit !== undefined && !isInt(p.limit, 1, EVENTS_LIMIT_MAX)) {
    throw invalid(`'limit' must be an integer from 1 to ${EVENTS_LIMIT_MAX}`);
  }
  return {
    bars,
    section,
    all: p.all === true,
    limit: (p.limit as number | undefined) ?? EVENTS_LIMIT,
  };
}

/** The 1-based inclusive song bars an `events` request covers (range rules in the protocol). */
function eventsRange(
  structure: SongStructure | null,
  bars: { from: number; to: number } | undefined,
  section: string | undefined,
): { from: number; to: number } {
  if (structure === null) {
    if (section !== undefined) throw invalid('the evaluated pattern has no sections');
    if (bars && bars.to - bars.from + 1 > PLAIN_BARS_MAX) {
      throw invalid(`a pattern without song() is queried at most ${PLAIN_BARS_MAX} bars at a time`);
    }
    return bars ?? { from: 1, to: 4 };
  }
  const total = structure.totalBars;
  if (section !== undefined) {
    const s = structure.sections.find((x) => x.name === section);
    if (!s) {
      const names = structure.sections.length
        ? `sections: ${structure.sections.map((x) => x.name).join(', ')}`
        : 'the song has no sections';
      throw invalid(`unknown section '${section}' (${names})`);
    }
    return { from: s.start + 1, to: s.start + s.bars };
  }
  const want = bars ?? { from: 1, to: total };
  if (want.from > total) throw invalid(`the song has ${total} bars`);
  return { from: want.from, to: Math.min(want.to, total) };
}

const nothingEvaluated = (message = 'nothing has been evaluated yet') =>
  new ControlError(ErrorCode.NOTHING_EVALUATED, message);

/** The song `ref` names (its path, else `songs/${ref}.js`, a trailing `.js` allowed), or `-32005`. */
function lookup(st: SessionState, ref: string): string {
  const path = songPath(stripSongExt(ref));
  const hit = st.songs.find((s) => s.path === ref || s.path === path);
  if (hit) return hit.path;
  const available = st.songs.length
    ? `available: ${st.songs.map((s) => fileStem(s.path)).join(', ')}`
    : 'the project has no songs';
  throw new ControlError(ErrorCode.NO_SUCH_SONG, `no such song '${ref}' (${available})`);
}

/** Steps 2-4 of song resolution: the project must be open; the target defaults to the active song. */
function resolve(session: Session, ref: string | undefined): string {
  const st = get(session.store);
  if (!st.project) throw new ControlError(ErrorCode.NO_PROJECT, 'no project is open');
  if (ref !== undefined) return lookup(st, ref);
  if (st.active === null) {
    throw new ControlError(ErrorCode.NO_SUCH_SONG, 'no active song (the project has no songs)');
  }
  return st.active;
}

export function createControlHandler({
  session,
  runtime,
  arbiter,
  now = () => performance.now(),
  vim = () => false,
}: {
  session: Session;
  runtime: Runtime;
  /** Evaluate and stop as the toolbar does. */
  arbiter: Pick<PlayArbiter, 'evaluate' | 'stop'>;
  /** Milliseconds, for request deadlines; a test seam. */
  now?: () => number;
  /** Whether the editor has vim keybindings on. */
  vim?: () => boolean;
}): ControlHandler {
  /** Resolve the target and make it the active song (step 5 for every method but `code.get`). */
  async function show(ref: string | undefined): Promise<string> {
    const target = resolve(session, ref);
    if (get(session.store).active !== target) {
      await session.switchSong(target);
      const st = get(session.store);
      if (st.active !== target) {
        throw unloadable(`cannot show '${ref ?? target}'`, st.error ?? 'it could not be read');
      }
    }
    return target;
  }

  /** Whether `target` is still a song of the project at `root`. */
  function listed(target: string, root: string | undefined): boolean {
    const st = get(session.store);
    return !!st.project && st.project.root === root && st.songs.some((s) => s.path === target);
  }

  /** Step 6: the target must still be listed right before it is used. */
  function recheck(target: string): void {
    if (!listed(target, get(session.store).project?.root)) throw removed(target);
  }

  const item = (path: string) => get(session.store).songs.find((s) => s.path === path);

  function state(): AppStateSummary {
    const ss = get(session.store);
    const rs = get(runtime.store);
    const pb = get(session.playback.store);
    const open = ss.project !== null;
    const position = rs.started ? runtime.position() : null;
    const meter =
      position?.located && rs.structure?.meterChanges
        ? meterAtBar(rs.structure, position.located.songBar)
        : rs.meter;
    return {
      project: ss.project ? { name: ss.project.name, root: ss.project.root } : null,
      songs: open
        ? ss.songs.map((s) => ({
            path: s.path,
            title: s.title,
            dirty: s.dirty,
            conflict: s.conflict,
          }))
        : [],
      active: open ? ss.active : null,
      playing: open && pb.playing ? { path: pb.playing.path, title: pb.playing.title } : null,
      started: pb.started,
      cps: cpsAtMeter(rs.cps, rs.meter, meter),
      bpm: Math.round(bpmFromCps(rs.cps, rs.meter)),
      meter: meterString(meter),
      audio: rs.audio,
      evalError: rs.evalError,
      metaError: open ? ss.metaError : null,
      tempoOverride: rs.tempoOverride,
      ...(rs.seed === undefined ? {} : { seed: rs.seed }),
      playhead: playhead(position, rs.structure),
      transport: transport(pb.transport),
      outputError: rs.outputError,
      engine: rs.engine,
      live: rs.live,
      levels: levelsSummary(runtime.levels(), rs.limiter),
      vim: vim(),
    };
  }

  function playhead(
    position: ReturnType<Runtime['position']> | null,
    structure: SongStructure | null,
  ): Playhead | null {
    if (position === null) return null;
    const { cycle, located } = position;
    const pos = located && structure ? barPosition(located, structure) : null;
    return pos ? { cycle, ...pos } : { cycle, songBar: null, section: null };
  }

  /** The transport by section name: the loop and armed section as the module names them. */
  function transport({ view, loop, armed }: PlaybackTransport): TransportSummary {
    const through = view.kind === 'loop' ? (loop?.through ?? null) : null;
    return {
      view: view.kind,
      loopSection: view.kind === 'loop' ? (loop?.section ?? null) : null,
      ...(through !== null && { loopThrough: through }),
      armedSection: armed,
    };
  }

  async function events(params: unknown): Promise<EventsResult> {
    const p = eventsParams(params);
    const rs = get(runtime.store);
    const ss = get(session.store);
    const evaluated = get(session.playback.store).evaluated;
    const code = rs.activeCode;
    if (evaluated === null || code === null) throw nothingEvaluated();
    const { from, to } = eventsRange(rs.structure, p.bars, p.section);
    const haps = runtime.query(from - 1, to);
    if (haps === null) throw nothingEvaluated();
    // The active song's buffer when it has one, else whatever the session can load for it.
    const text =
      (ss.active === evaluated.path ? session.activeText() : null) ??
      (await session.peekSong(evaluated.path))?.text ??
      null;
    if (text === null) throw nothingEvaluated('the evaluated song is no longer in the project');
    const { events, truncated } = toEvents(haps, {
      code,
      structure: rs.structure,
      meter: rs.meter,
      all: p.all,
      limit: p.limit,
    });
    return {
      song: { path: evaluated.path, title: evaluated.title },
      from,
      to,
      stale: text !== code,
      truncated,
      meter: meterString(rs.meter),
      tempoOverride: rs.tempoOverride,
      events,
    };
  }

  async function codeGet(params: unknown) {
    const song = songParam(params);
    const target = resolve(session, song);
    let code: string | null;
    let dirty: boolean | null = null;
    if (get(session.store).active === target) {
      code = session.activeText();
    } else {
      const root = get(session.store).project?.root;
      const peek = await session.peekSong(target);
      if (!peek) {
        if (!listed(target, root)) throw removed(target);
        throw unloadable(
          `cannot read '${song}'`,
          get(session.store).error ?? 'it could not be read',
        );
      }
      code = peek.text;
      dirty = peek.dirty;
    }
    const it = item(target);
    if (code === null || !it) throw removed(target);
    return { song: target, title: it.title, code, dirty: dirty ?? it.dirty };
  }

  async function codeSet(params: unknown): Promise<CodeSetResult> {
    const p = object(params, ['song', 'code', 'eval', 'expect']);
    const song = optSong(p);
    const code = requireString(p, 'code');
    const mode = p.eval === undefined ? 'auto' : p.eval;
    if (mode !== 'auto' && mode !== 'always' && mode !== 'never') {
      throw invalid(`'eval' must be "auto", "always" or "never"`);
    }
    const expect = optString(p, 'expect');
    // A stale `expect` fails before the editor switches; externalEdit checks again after.
    const pre = resolve(session, song);
    if (expect !== undefined && get(session.store).active !== pre) {
      const peek = await session.peekSong(pre);
      if (peek && toLF(expect) !== peek.text) {
        throw songChanged();
      }
    }
    const target = await show(song);
    recheck(target);
    const r = session.externalEdit(target, code, expect === undefined ? {} : { expect });
    if (r === 'missing') throw removed(target);
    if (r === 'stale') {
      throw songChanged();
    }
    const pb = get(session.playback.store);
    const playingPath = pb.evaluated?.path ?? null;
    const run =
      mode === 'always' ||
      (mode === 'auto' && r.changed && shouldAutoEval(pb.started, playingPath, target));
    const evaluated = run ? await evaluateGuarded(target) : null;
    return { song: target, changed: r.changed, dirty: r.dirty, conflict: r.conflict, evaluated };
  }

  async function instrumentApply(params: unknown): Promise<InstrumentApplyResult> {
    const p = object(params, ['root', 'song', 'id', 'from', 'to', 'expect']);
    const root = requireString(p, 'root');
    const id = requireString(p, 'id');
    const expect = requireString(p, 'expect');
    const song = optSong(p);
    const { from, to } = p;
    if (!isInt(from, 0, 0xffffffff) || !isInt(to, 0, 0xffffffff) || from >= to) {
      throw invalid("'from' and 'to' must be an increasing range of UTF-16 offsets");
    }
    try {
      return await session.applyInstrument({ root, song, id, from, to, expect });
    } catch (error) {
      if (!(error instanceof InstrumentApplyError)) throw error;
      switch (error.kind) {
        case 'project':
          throw new ControlError(ErrorCode.NO_PROJECT, error.message);
        case 'song':
          throw new ControlError(ErrorCode.NO_SUCH_SONG, error.message);
        case 'stale':
          throw songChanged();
        case 'invalid':
          throw invalid(error.message);
      }
    }
  }

  /**
   * As the toolbar's play: a playing memo stops first, and during a take the evaluate never
   * starts the song (a song the take plays takes the new code, as a re-evaluate does).
   */
  async function evaluateGuarded(target: string): Promise<EvalOutcome> {
    const r = await arbiter.evaluate(target);
    // The session's errorKind stays off the wire.
    const { errorKind: _errorKind, ...outcome } = r;
    return outcome;
  }

  async function evaluate(params: unknown): Promise<EvalOutcome> {
    const target = await show(songParam(params));
    recheck(target);
    return evaluateGuarded(target);
  }

  /** As the toolbar's stop: during a take it stops (and saves) the take, else a memo and the song. */
  async function stop() {
    const { wasPlaying, done } = arbiter.stop();
    await done;
    return { wasPlaying };
  }

  /** A `SongRef` for a song of the store, read now. */
  function ref(path: string): SongRef {
    const it = item(path);
    if (!it) throw removed(path);
    return { path, title: it.title };
  }

  async function songNew(params: unknown): Promise<SongResult> {
    const p = object(params, ['name']);
    const name = requireString(p, 'name');
    needProject(session);
    let path: string | null;
    try {
      path = await session.createSong(name);
    } catch (e) {
      throw fromProjectError(e);
    }
    if (path === null) throw projectChanged();
    const st = get(session.store);
    if (st.active !== path) {
      throw unloadable(`cannot show '${name}'`, st.error ?? 'it could not be read');
    }
    return { song: ref(path) };
  }

  async function songSwitch(params: unknown): Promise<SongResult> {
    const p = object(params, ['song']);
    const song = optSong(p);
    if (song === undefined) throw invalid(`'song' is required`);
    const target = await show(song);
    return { song: ref(target) };
  }

  async function songRename(params: unknown): Promise<SongRenameResult> {
    const p = object(params, ['song', 'name']);
    const target = resolve(session, optSong(p));
    const name = requireString(p, 'name');
    if (fileStem(target) === name) throw invalid(`song '${name}' is already named '${name}'`);
    const from = ref(target);
    let r;
    try {
      r = await session.renameSong(target, name);
    } catch (e) {
      throw fromProjectError(e, target);
    }
    if (!r.ok && r.reason === 'deleted') {
      // Still listed (kept in memory), so "no such song" would mislead.
      throw new ControlError(
        ErrorCode.NO_SUCH_SONG,
        `song '${target}' was deleted on disk; save it to recreate the file, then rename it`,
      );
    }
    if (!r.ok) throw songGone(r.reason, target);
    return { from, to: ref(r.to) };
  }

  async function songDelete(params: unknown): Promise<SongDeleteResult> {
    const p = object(params, ['song', 'discard']);
    const target = resolve(session, optSong(p));
    const discard = optBool(p, 'discard') === true;
    const song = ref(target);
    let r;
    try {
      r = await session.deleteSong(target, { discard });
    } catch (e) {
      throw fromProjectError(e, target);
    }
    if (!r.ok) {
      if (r.reason === 'unsaved') {
        throw new ControlError(
          ErrorCode.UNSAVED,
          `'${fileStem(target)}' has unsaved changes (use discard to delete it anyway)`,
        );
      }
      throw songGone(r.reason, target);
    }
    return { song, stopped: r.stopped };
  }

  async function save(params: unknown): Promise<SaveResult> {
    const p = object(params, ['song', 'all']);
    const song = optSong(p);
    const all = optBool(p, 'all');
    if (song !== undefined && all === true) {
      throw invalid(`'song' and 'all' are mutually exclusive`);
    }
    let r;
    let target: string | undefined;
    if (all === true) {
      needProject(session);
      r = await session.saveSongs({ all: true });
    } else {
      target = resolve(session, song);
      r = await session.saveSongs({ path: target });
    }
    if (!r.ok) {
      if (r.reason === 'failed') {
        const done = r.saved.length ? ` (saved: ${r.saved.map(fileStem).join(', ')})` : '';
        throw new ControlError(
          ErrorCode.INTERNAL_ERROR,
          `could not save '${fileStem(r.failedPath)}': ${r.error}${done}`,
        );
      }
      throw songGone(r.reason, target ?? get(session.store).active ?? '');
    }
    return { saved: r.saved.map(ref) };
  }

  async function projectOpen(params: unknown): Promise<ProjectOpenResult> {
    const p = object(params, ['path', 'discard']);
    const path = requireString(p, 'path');
    const discard = optBool(p, 'discard') === true;
    const r = await session.openPath(path, { discard });
    if (!r.ok) {
      if (r.reason === 'unsaved') {
        const n = r.dirty.length;
        const names = r.dirty.map(fileStem).join(', ');
        const lead = n === 1 ? '1 song has' : `${n} songs have`;
        throw new ControlError(
          ErrorCode.UNSAVED,
          `${lead} unsaved changes: ${names} (use discard to open anyway)`,
        );
      }
      if (r.reason === 'busy') {
        throw new ControlError(
          ErrorCode.BUSY,
          'the app is waiting on an open dialog; try again after it closes',
        );
      }
      throw new ControlError(ErrorCode.OPEN_FAILED, r.error);
    }
    const project = get(session.store).project;
    if (!project) throw projectChanged();
    return { project: { name: project.name, root: project.root }, alreadyOpen: r.alreadyOpen };
  }

  async function run(method: string, params: unknown): Promise<unknown> {
    switch (method) {
      case 'state':
        object(params, []);
        return state();
      case 'code.get':
        return codeGet(params);
      case 'code.set':
        return codeSet(params);
      case 'instrument.apply':
        return instrumentApply(params);
      case 'eval':
      case 'play':
        return evaluate(params);
      case 'stop':
        object(params, []);
        return stop();
      case 'events':
        return events(params);
      case 'song.new':
        return songNew(params);
      case 'song.switch':
        return songSwitch(params);
      case 'song.rename':
        return songRename(params);
      case 'song.delete':
        return songDelete(params);
      case 'save':
        return save(params);
      case 'project.open':
        return projectOpen(params);
      default:
        throw new ControlError(ErrorCode.METHOD_NOT_FOUND, `unknown method '${method}'`);
    }
  }

  // One request at a time, so two calls cannot interleave a switch and an edit.
  let chain: Promise<unknown> = Promise.resolve();
  return (method, params, timeoutMs) => {
    const arrived = now();
    const start = () => {
      if (timeoutMs !== undefined && now() - arrived >= timeoutMs) {
        // Same message as Rust's timeout reply, which the client already got.
        const secs = Math.ceil(timeoutMs / 1000);
        throw new ControlError(ErrorCode.TIMEOUT, `the app did not answer in ${secs}s`);
      }
      return run(method, params);
    };
    const next = chain.then(start, start);
    chain = next.catch(() => {});
    return next;
  };
}

/** Run a request and turn its outcome into a reply; never rejects. */
export async function replyTo(handler: ControlHandler, req: ControlRequest): Promise<ControlReply> {
  try {
    return { result: await handler(req.method, req.params, req.timeoutMs) };
  } catch (e) {
    if (e instanceof ControlError) return { error: { code: e.code, message: e.message } };
    return {
      error: { code: ErrorCode.INTERNAL_ERROR, message: `internal error: ${errorMessage(e)}` },
    };
  }
}

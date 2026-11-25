/**
 * The playing song and its transport, with one owner: the song whose last successful evaluate is
 * sounding, the rule for when the transport resets, and arming a section by name.
 *
 * It follows the runtime rather than shadowing it: `started` and the evaluated structure come from
 * `runtime.store`, the view and `pendingStart` from `runtime.transport.store`, and the armed names
 * are dropped whenever the runtime reports no pending start (every stop rewinds the transport).
 * The session constructs it, calls its verbs and exposes it as `session.playback`; it reads the
 * active song's header through `getHeader()` and never subscribes to the session.
 *
 * `runtime` is called through the object at call time, so a test can replace its methods.
 */
import { derived, get, writable, type Readable } from 'svelte/store';
import type { InstrumentSnapshot } from '../strudel/instruments';
import type { EvaluateOptions, Runtime } from '../strudel/runtime';
import type { SongStructure } from '../strudel/structure';
import type { View } from '../strudel/transport';

/** A song as the store names it. */
export interface PlaybackSong {
  path: string;
  title: string;
}

/** A loop by section name; `names` is every section in the range, in order. */
export interface PlaybackLoop {
  section: string;
  through?: string;
  names: string[];
}

export interface PlaybackTransport {
  /** The runtime's view, with indices into the evaluated structure. */
  view: View;
  /**
   * The loop: while stopped, the one armed here; while playing, the view's sections named from
   * the evaluated structure. Null without a loop.
   */
  loop: PlaybackLoop | null;
  /** The section the armed start or loop begins in; non-null exactly while a start is pending. */
  armed: string | null;
}

export interface PlaybackState {
  /** The song the last successful evaluate played: kept on stop, cleared on project change. */
  evaluated: PlaybackSong | null;
  /** `evaluated` while the runtime plays, else null. */
  playing: PlaybackSong | null;
  started: boolean;
  transport: PlaybackTransport;
}

/**
 * What to arm, by section name: start from a section, loop one section (or the header's run from
 * `loop` through `through`), or stop looping.
 */
export type ArmChoice = { start: string } | { loop: string; through?: string } | 'stopLooping';

/** The active song's header as the session reads it. */
export interface PlaybackHeader {
  active: string | null;
  /** The header's structure; null without an active song. */
  structure: SongStructure | null;
  /** The header's error; nothing can be armed while it has one. */
  metaError: string | null;
}

/** What `evaluate` passes to `runtime.evaluate`, and the header's `meta.title`. */
export interface PlaybackEvaluate {
  options: EvaluateOptions;
  metaTitle: unknown;
}

export interface SongPlaybackDeps {
  runtime: Pick<Runtime, 'store' | 'transport' | 'evaluate' | 'stop'>;
  getHeader(): PlaybackHeader;
  /** The session's project generation; an evaluate that resolves in another one is not recorded. */
  currentGen(): number;
  /** The song's title: `meta.title` when it is a non-blank string, else its file name. */
  titleFrom(path: string, title: unknown): string;
}

export interface SongPlayback {
  readonly store: Readable<PlaybackState>;
  /** Whether `arm(choice)` would arm anything now. */
  readonly canArm: Readable<(choice: ArmChoice) => boolean>;
  /** The evaluated song's `meta.title` as read (undefined when nothing is evaluated). */
  metaTitle(): unknown;
  /** The instrument pins the evaluated song was evaluated with. */
  instruments(): InstrumentSnapshot | undefined;
  /**
   * Evaluate `path`'s `text`. Synchronous up to `runtime.evaluate`: a different song taking over
   * playback resets the transport first, in the same tick. On success it becomes the evaluated
   * song, unless the project generation moved past `gen`. True iff the runtime evaluated it.
   */
  evaluate(path: string, text: string, opts: PlaybackEvaluate, gen: number): Promise<boolean>;
  /** The header or the instrument pins refused an evaluate: disarm while stopped. */
  refused(): void;
  /** The active song changed: while stopped the transport is the active song's, so disarm. */
  activeChanged(): void;
  /** The project changed: stop, reset the transport and forget the evaluated song. */
  projectChanged(): void;
  /** `from` was renamed to `to`: the evaluated song follows, retitled. */
  renamed(from: string, to: string): void;
  /**
   * `path` was removed (call after `activeChanged`). If it is the evaluated song: stop and forget
   * it; `stopped` is whether it was playing.
   */
  removed(path: string): { stopped: boolean };
  /**
   * Arm a section by name, against the evaluated structure while playing and the active header's
   * while stopped. Nothing while the header has an error or the names do not resolve.
   * `stopLooping` while stopped drops anything armed; while playing it leaves the loop at the next
   * bar.
   */
  arm(choice: ArmChoice): void;
  /** Leave the loop and play the ending (see `Transport.playEnding`). */
  playEnding(): void;
}

/** A choice resolved to indices in `structure`, with the names it publishes while stopped. */
type Resolved =
  | { kind: 'stopLooping'; structure: SongStructure }
  | { kind: 'start'; structure: SongStructure; index: number; name: string }
  | {
      kind: 'loop';
      structure: SongStructure;
      first: number;
      last: number;
      loop: PlaybackLoop;
    };

const sameSong = (a: PlaybackSong | null, b: PlaybackSong | null): boolean =>
  a === b || (a !== null && b !== null && a.path === b.path && a.title === b.title);

const sameLoop = (a: PlaybackLoop | null, b: PlaybackLoop | null): boolean =>
  a === b ||
  (a !== null &&
    b !== null &&
    a.section === b.section &&
    a.through === b.through &&
    a.names.length === b.names.length &&
    a.names.every((n, i) => n === b.names[i]));

const sameView = (a: View, b: View): boolean =>
  a.kind === b.kind &&
  (a.kind !== 'loop' ||
    (a.section === (b as typeof a).section && a.through === (b as typeof a).through));

function sameState(a: PlaybackState, b: PlaybackState): boolean {
  return (
    a.evaluated === b.evaluated &&
    sameSong(a.playing, b.playing) &&
    a.started === b.started &&
    sameView(a.transport.view, b.transport.view) &&
    sameLoop(a.transport.loop, b.transport.loop) &&
    a.transport.armed === b.transport.armed
  );
}

/** The loop a view plays, named from `structure`; null without a loop or a name for it. */
function viewLoop(view: View, structure: SongStructure | null): PlaybackLoop | null {
  if (view.kind !== 'loop' || !structure) return null;
  const last = view.through ?? view.section;
  const section = structure.sections[view.section]?.name;
  const through = structure.sections[last]?.name;
  if (section === undefined || through === undefined) return null;
  const names = structure.sections.slice(view.section, last + 1).map((s) => s.name);
  return view.through === undefined ? { section, names } : { section, through, names };
}

export function createSongPlayback(deps: SongPlaybackDeps): SongPlayback {
  const { runtime, getHeader, currentGen, titleFrom } = deps;

  let evaluated: {
    song: PlaybackSong;
    metaTitle: unknown;
    instruments: InstrumentSnapshot | undefined;
  } | null = null;
  /** What `arm` armed while stopped; dropped once the runtime reports no pending start. */
  let armed: { name: string; loop: PlaybackLoop | null } | null = null;

  function snapshot(): PlaybackState {
    const rs = get(runtime.store);
    const ts = get(runtime.transport.store);
    if (!ts.pendingStart) armed = null;
    const song = evaluated?.song ?? null;
    return {
      evaluated: song,
      playing: rs.started ? song : null,
      started: rs.started,
      transport: {
        view: ts.view,
        loop: rs.started ? viewLoop(ts.view, rs.structure) : (armed?.loop ?? null),
        armed: armed?.name ?? null,
      },
    };
  }

  let current = snapshot();
  const state = writable(current);

  function publish(): void {
    const next = snapshot();
    if (sameState(current, next)) return;
    current = next;
    state.set(next);
  }

  runtime.store.subscribe(publish);
  runtime.transport.store.subscribe(publish);

  /** Disarm while stopped: the transport is the active song's then. */
  function disarmIfStopped(): void {
    if (!get(runtime.store).started) runtime.transport.reset();
  }

  /**
   * The structure arming goes against: the evaluated one while playing, the active header's while
   * stopped (none while the header has an error).
   */
  function inForce(header: PlaybackHeader): SongStructure | null {
    if (get(runtime.store).started) return get(runtime.store).structure;
    return header.metaError === null ? header.structure : null;
  }

  function resolve(choice: ArmChoice): Resolved | null {
    const header = getHeader();
    const structure = inForce(header);
    if (!structure) return null;
    if (choice === 'stopLooping') return { kind: 'stopLooping', structure };
    // Names come from the header the user sees; none while it has an error.
    const visible = header.metaError === null ? header.structure : null;
    if (!visible) return null;
    const indexIn = (s: SongStructure, name: string, from = 0): number =>
      s.sections.findIndex((x, i) => i >= from && x.name === name);
    if ('start' in choice) {
      if (indexIn(visible, choice.start) < 0) return null;
      const index = indexIn(structure, choice.start);
      return index < 0 ? null : { kind: 'start', structure, index, name: choice.start };
    }
    const from = indexIn(visible, choice.loop);
    const to = choice.through === undefined ? from : indexIn(visible, choice.through, from);
    if (from < 0 || to < 0) return null;
    // The header's run must appear contiguously and in order in the structure in force.
    const names = visible.sections.slice(from, to + 1).map((s) => s.name);
    const first = indexIn(structure, names[0]!);
    if (first < 0 || names.some((name, i) => structure.sections[first + i]?.name !== name)) {
      return null;
    }
    const last = first + names.length - 1;
    const loop: PlaybackLoop =
      names.length === 1
        ? { section: names[0]!, names }
        : { section: names[0]!, through: names.at(-1)!, names };
    return { kind: 'loop', structure, first, last, loop };
  }

  // Recomputed when the runtime or the store moves; the header is read at call time.
  const canArm = derived(
    [runtime.store, state],
    () => (choice: ArmChoice) => resolve(choice) !== null,
  );

  return {
    store: { subscribe: state.subscribe },
    canArm,
    metaTitle: () => evaluated?.metaTitle,
    instruments: () => evaluated?.instruments,

    async evaluate(path, text, opts, gen) {
      // No await before `runtime.evaluate`: its synchronous resume stays inside the user gesture.
      // A different song taking over playback must not inherit the playing song's transport.
      // While stopped the transport belongs to the active song (reset on every switch), so a
      // start armed before Play is kept.
      if (get(runtime.store).started && evaluated?.song.path !== path) {
        runtime.transport.reset();
      }
      const ok = await runtime.evaluate(text, opts.options);
      if (ok && gen === currentGen()) {
        evaluated = {
          song: { path, title: titleFrom(path, opts.metaTitle) },
          metaTitle: opts.metaTitle,
          instruments: opts.options.instruments,
        };
        publish();
      }
      return ok;
    },

    refused: disarmIfStopped,
    activeChanged: disarmIfStopped,

    projectChanged() {
      runtime.stop(); // playback always stops when the project changes
      runtime.transport.reset();
      evaluated = null;
      publish();
    },

    renamed(from, to) {
      if (evaluated?.song.path !== from) return;
      evaluated = {
        ...evaluated,
        song: { path: to, title: titleFrom(to, evaluated.metaTitle) },
      };
      publish();
    },

    removed(path) {
      if (evaluated?.song.path !== path) return { stopped: false };
      const stopped = get(runtime.store).started;
      runtime.stop();
      evaluated = null;
      publish();
      return { stopped };
    },

    arm(choice) {
      const r = resolve(choice);
      if (!r) return;
      const { transport } = runtime;
      if (r.kind === 'stopLooping') transport.loopSection(null, r.structure);
      else if (r.kind === 'start') transport.startFrom(r.index, r.structure);
      else if (r.first === r.last) transport.loopSection(r.first, r.structure);
      else transport.loopRange(r.first, r.last, r.structure);
      // Names are kept only for a start the runtime now holds pending (never while playing).
      if (r.kind !== 'stopLooping' && get(runtime.transport.store).pendingStart) {
        armed =
          r.kind === 'start'
            ? { name: r.name, loop: null }
            : { name: r.loop.section, loop: r.loop };
      }
      publish();
    },

    playEnding() {
      runtime.transport.playEnding();
    },
  };
}

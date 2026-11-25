/**
 * The playing song and its transport, with one owner: the song whose last successful evaluate is
 * sounding, the rule for when the transport resets, and arming a section by name.
 *
 * It follows the runtime rather than shadowing it: `started` and the evaluated structure come from
 * `runtime.store`, the view and `pendingStart` from `runtime.transport.store`, and the armed names
 * are dropped whenever the runtime reports no pending start (every stop rewinds the transport).
 * The session constructs it, calls its verbs and exposes it as `session.playback`; it reads the
 * active song's header structure through `getHeader()` and never subscribes to the session.
 *
 * `runtime` is called through the object at call time, so a test can replace its methods.
 */
import { derived, get, writable, type Readable } from 'svelte/store';
import type { InstrumentSnapshot } from '../strudel/instruments';
import type { EngineId, EvaluateOptions, Runtime } from '../strudel/runtime';
import type { SongStructure } from '../strudel/structure';
import { sameView, type View } from '../strudel/transport';
import { stripSongExt } from './ui';

/** "songs/main.js" to "main". */
export const fileStem = (path: string): string =>
  stripSongExt(path.slice(path.lastIndexOf('/') + 1));

/** `meta.title` when it is a non-blank string, else the file name without ".js". */
export const titleFrom = (path: string, title: unknown): string =>
  typeof title === 'string' && title.trim() ? title : fileStem(path);

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
 * What to arm, by section name: start from a section, or loop one section (or the header's run
 * from `loop` through `through`).
 */
export type ArmChoice = { start: string } | { loop: string; through?: string };

/**
 * Where `start` started the song (`T`, the context time of bar 1's downbeat, and `cps`);
 * `missingSection` when the section is not in the evaluated song (nothing is left playing); null
 * when it could not start.
 */
export type PlaybackStartResult = { T: number; cps: number } | { missingSection: string } | null;

/** What `evaluate` passes to `runtime.evaluate`, and the header's `meta.title`. */
export interface PlaybackEvaluate {
  options: EvaluateOptions;
  metaTitle: unknown;
}

export interface SongPlaybackDeps {
  runtime: Pick<Runtime, 'store' | 'transport' | 'evaluate' | 'stop' | 'start' | 'cycleTime'>;
  /**
   * The active song's header structure; null without an active song or while the header has an
   * error (nothing can be armed while stopped then).
   */
  getHeader(): SongStructure | null;
  /** The session's project generation; an evaluate that resolves in another one is not recorded. */
  currentGen(): number;
}

export interface SongPlayback {
  readonly store: Readable<PlaybackState>;
  /** Whether `arm(choice)` would arm anything now. */
  readonly canArm: Readable<(choice: ArmChoice) => boolean>;
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
  /**
   * The active song's header may have changed (the session calls it on every publish). While
   * stopped, a pending arm follows its names: re-armed where they now resolve in the header,
   * disarmed when they no longer do (or the header has an error). Nothing while playing, nor
   * while an evaluate or a start is pending: the runtime may start from what is armed then (an
   * evaluate catches up once it resolves stopped).
   */
  headerChanged(): void;
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
   * while stopped. Nothing while the header has an error or the names do not resolve. True iff
   * the names resolved and the runtime was asked to arm.
   */
  arm(choice: ArmChoice): boolean;
  /**
   * While stopped, drop anything armed; while playing, leave the loop at the next bar. Nothing
   * while stopped without a header structure.
   */
  stopLooping(): void;
  /** Leave the loop and play the ending (see `Transport.playEnding`). */
  playEnding(): void;
  /**
   * Start the evaluated song from cycle 0 (a playing runtime is stopped first), with `section`
   * armed first against the evaluated structure. Synchronous up to `runtime.start`, so its resume stays
   * inside the user gesture. A failed start disarms, unless the runtime started meanwhile.
   * `T` is global cycle 1 with a count-in, else cycle 0.
   */
  start(opts: {
    section?: ArmChoice | null;
    countIn: boolean;
    backend?: EngineId;
  }): Promise<PlaybackStartResult>;
}

/**
 * A choice resolved to sections `first` to `last` of `structure`: started from (`loop` null) or
 * looped. Its name is its first section's.
 */
interface Resolved {
  structure: SongStructure;
  first: number;
  last: number;
  loop: PlaybackLoop | null;
}

const nameOf = (r: Resolved): string => r.structure.sections[r.first]!.name;

/** The choice that resolves to `r` again. */
function choiceOf(r: Resolved): ArmChoice {
  if (!r.loop) return { start: nameOf(r) };
  const { section, through } = r.loop;
  return through === undefined ? { loop: section } : { loop: section, through };
}

/** Where `r` arms: its section indices and the song bar it begins at. */
const place = (r: Resolved) => ({
  first: r.first,
  last: r.last,
  bar: r.structure.sections[r.first]!.start,
});

const samePlace = (a: Resolved, b: Resolved): boolean => {
  const [p, q] = [place(a), place(b)];
  return p.first === q.first && p.last === q.last && p.bar === q.bar;
};

const indexIn = (s: SongStructure, name: string, from = 0): number =>
  s.sections.findIndex((x, i) => i >= from && x.name === name);

/**
 * `choice` against `structure`, with names from `visible` (the structure the user sees): a
 * section must be in both, and a range's run in `visible` must appear contiguously and in order
 * in `structure`.
 */
function resolveIn(
  structure: SongStructure,
  visible: SongStructure,
  choice: ArmChoice,
): Resolved | null {
  if ('start' in choice) {
    if (indexIn(visible, choice.start) < 0) return null;
    const index = indexIn(structure, choice.start);
    return index < 0 ? null : { structure, first: index, last: index, loop: null };
  }
  const from = indexIn(visible, choice.loop);
  const to = choice.through === undefined ? from : indexIn(visible, choice.through, from);
  if (from < 0 || to < 0) return null;
  const names = visible.sections.slice(from, to + 1).map((s) => s.name);
  const first = indexIn(structure, names[0]!);
  if (first < 0 || names.some((name, i) => structure.sections[first + i]?.name !== name)) {
    return null;
  }
  const loop: PlaybackLoop =
    names.length === 1
      ? { section: names[0]!, names }
      : { section: names[0]!, through: names.at(-1)!, names };
  return { structure, first, last: first + names.length - 1, loop };
}

const sameLoop = (a: PlaybackLoop | null, b: PlaybackLoop | null): boolean =>
  a === b ||
  (a !== null &&
    b !== null &&
    a.section === b.section &&
    a.through === b.through &&
    a.names.length === b.names.length &&
    a.names.every((n, i) => n === b.names[i]));

function sameState(a: PlaybackState, b: PlaybackState): boolean {
  return (
    a.evaluated === b.evaluated &&
    a.playing === b.playing &&
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
  const { runtime, getHeader, currentGen } = deps;

  let evaluated: {
    song: PlaybackSong;
    metaTitle: unknown;
    instruments: InstrumentSnapshot | undefined;
  } | null = null;
  /**
   * What `arm` armed while stopped, as resolved: its names, and the place the runtime holds.
   * Dropped once the runtime reports no pending start.
   */
  let armed: Resolved | null = null;
  /** While `start` awaits `runtime.start`: its arm is against the evaluated song, not the header. */
  let starting = 0;
  /**
   * While `evaluate` awaits `runtime.evaluate`: an autostart starts from what is armed now, so a
   * header edit meanwhile must not move it.
   */
  let evaluating = 0;

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
        armed: armed ? nameOf(armed) : null,
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
  function inForce(header: SongStructure | null): SongStructure | null {
    return get(runtime.store).started ? get(runtime.store).structure : header;
  }

  function resolve(choice: ArmChoice): Resolved | null {
    // Names come from the header the user sees; none while it has an error.
    const visible = getHeader();
    const structure = inForce(visible);
    return structure && visible ? resolveIn(structure, visible, choice) : null;
  }

  /** Ask the runtime to arm `r`, and keep it while the start is pending. */
  function apply(r: Resolved): void {
    const { transport } = runtime;
    if (!r.loop) transport.startFrom(r.first, r.structure);
    else if (r.first === r.last) transport.loopSection(r.first, r.structure);
    else transport.loopRange(r.first, r.last, r.structure);
    // Kept only for a start the runtime now holds pending (never while playing).
    if (get(runtime.transport.store).pendingStart) armed = r;
    publish();
  }

  /**
   * While stopped, the armed names against the header now in force: an unchanged place keeps the
   * arm, a moved one (another index, or the same index at another bar) re-arms it, and names that
   * no longer resolve (or a range whose run changed) disarm. The runtime's notifications only
   * publish, so this never re-enters.
   */
  function headerChanged(): void {
    // `armed` is only ever set while a start is pending, and dropped on the notification that
    // ends it, so a start is pending here.
    if (!armed || starting > 0 || evaluating > 0 || get(runtime.store).started) return;
    const r = resolve(choiceOf(armed));
    // Disarm when the names no longer resolve, or the header's run from `loop` through `through`
    // changed.
    if (!r || !sameLoop(r.loop, armed.loop)) {
      runtime.transport.reset();
      return;
    }
    if (!samePlace(r, armed)) apply(r);
  }

  // Recomputed when the runtime or the store moves; the header is read at call time. The runtime
  // store is needed too: the evaluated structure changing while playing need not change `state`.
  const canArm = derived(
    [runtime.store, state],
    () => (choice: ArmChoice) => resolve(choice) !== null,
  );

  return {
    store: { subscribe: state.subscribe },
    canArm,
    instruments: () => evaluated?.instruments,

    async evaluate(path, text, opts, gen) {
      // No await before `runtime.evaluate`: its synchronous resume stays inside the user gesture.
      // A different song taking over playback must not inherit the playing song's transport.
      // While stopped the transport belongs to the active song (reset on every switch), so a
      // start armed before Play is kept.
      if (get(runtime.store).started && evaluated?.song.path !== path) {
        runtime.transport.reset();
      }
      evaluating++;
      let ok: boolean;
      try {
        ok = await runtime.evaluate(text, opts.options);
      } finally {
        evaluating--;
      }
      if (ok && gen === currentGen()) {
        evaluated = {
          song: { path, title: titleFrom(path, opts.metaTitle) },
          metaTitle: opts.metaTitle,
          instruments: opts.options.instruments,
        };
        publish();
      }
      // Still stopped (a failed or non-starting evaluate): the arm catches up with the header.
      headerChanged();
      return ok;
    },

    refused: disarmIfStopped,
    activeChanged: disarmIfStopped,
    headerChanged,

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
      if (!r) return false;
      apply(r);
      return true;
    },

    stopLooping() {
      const structure = inForce(getHeader());
      if (structure) runtime.transport.loopSection(null, structure);
      publish();
    },

    playEnding() {
      runtime.transport.playEnding();
    },

    async start({ section = null, countIn, backend }) {
      // No await before `runtime.start`: its synchronous resume stays inside the user gesture.
      if (get(runtime.store).started) runtime.stop();
      if (section !== null) {
        // After the stop (a stop rewinds the transport), against the evaluated structure.
        const structure = get(runtime.store).structure;
        const r = structure && resolveIn(structure, structure, section);
        if (!r) return { missingSection: 'start' in section ? section.start : section.loop };
        apply(r);
      }
      starting++;
      let ok: boolean;
      try {
        ok = await runtime.start({ countIn, backend });
      } finally {
        starting--;
      }
      if (!ok) {
        if (section !== null && !get(runtime.store).started) runtime.transport.reset();
        return null;
      }
      const T = runtime.cycleTime(countIn ? 1 : 0);
      if (T === null) {
        runtime.stop();
        return null;
      }
      return { T, cps: get(runtime.store).cps };
    },
  };
}

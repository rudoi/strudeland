/**
 * Starts and stops everything audible behind a take or a memo's playback (the song and the
 * metronome), so the recording controller and the memo player cannot diverge. It goes through
 * the runtime (never the scheduler) and the metronome (never the audio context).
 */
import { get, type Readable } from 'svelte/store';
import type { Metronome, MetronomeGrid } from '../audio/metronome';
import type { SessionEvalOutcome } from '../project/session';
import type { PlaybackStartResult, SongPlayback } from '../project/song-playback';
import type { Runtime } from '../strudel/runtime';
import type { Meter } from '../strudel/structure';
import { FIXED_METER_BACKING_MESSAGE } from './types';

export type PrepareOutcome =
  | { ok: true; cps: number; meter: Meter; tempoOverride: boolean }
  | { ok: false; kind: 'meta' | 'eval'; message: string };

/** A section to arm before the song starts, by name: started from, or looped. */
export interface SectionChoice {
  name: string;
  loop: boolean;
}

/**
 * Where the song started (`T`, `cps`); `missingSection` when the armed section is not in the
 * evaluated song (nothing is left playing); null when it could not start.
 */
export type StartSongResult = PlaybackStartResult;

export interface BackingStarter {
  /**
   * Evaluates the open song without starting it: how a take fails before the microphone opens.
   * The tempo is the runtime's after the evaluate (the song's own `setcps` wins).
   */
  prepareSong(): Promise<PrepareOutcome>;
  /**
   * Starts the song from cycle 0 (a playing runtime is stopped first), evaluating it first unless
   * `prepared`. `T` is the context time of bar 1's downbeat: global cycle 1 with a count-in, else
   * cycle 0. With `section`, that section of the evaluated song is armed first (looped or
   * started from), as the transport would; bar 1 is then that section's first bar.
   */
  startSong(opts?: {
    countIn?: boolean;
    prepared?: boolean;
    section?: SectionChoice | null;
  }): Promise<StartSongResult>;
  startMetronome(grid: MetronomeGrid): boolean;
  /** Stops the metronome, and the song when this starter started it and it still plays. Idempotent. */
  stopAll(): void;
}

export interface BackingDeps {
  runtime: Pick<Runtime, 'store' | 'stop'>;
  /** The song playback: it arms the section and starts the song. */
  playback: Pick<SongPlayback, 'start'>;
  session: {
    readonly store: Readable<{ active: string | null }>;
    evaluateBuffer(path: string, opts?: { autostart?: boolean }): Promise<SessionEvalOutcome>;
  };
  metronome: Pick<Metronome, 'start' | 'stop'>;
}

/** With the song off, the downbeat (a take's count-in first click, a memo's bar 1) is this far
 *  after now. */
export const LEAD_S = 0.25;

/** The failures a take and a memo's playback share, word for word. */
export const BACKING_MESSAGES = {
  audioOff: 'audio is not running: enable it in the status bar',
  songFailed: 'the song could not start',
  metronomeFailed: 'the metronome could not start',
  mixedMeter: FIXED_METER_BACKING_MESSAGE,
} as const;

export function createBackingStarter(deps: BackingDeps): BackingStarter {
  const { runtime, playback, session, metronome } = deps;
  let startedSong = false;

  async function prepareSong(): Promise<PrepareOutcome> {
    const active = get(session.store).active;
    if (active === null) return { ok: false, kind: 'eval', message: 'no song is open' };
    const r = await session.evaluateBuffer(active, { autostart: false });
    if (!r.ok) {
      const message = r.error ?? 'evaluate failed';
      return { ok: false, kind: r.errorKind === 'meta' ? 'meta' : 'eval', message };
    }
    const rs = get(runtime.store);
    if (rs.structure?.meterChanges) {
      return { ok: false, kind: 'meta', message: BACKING_MESSAGES.mixedMeter };
    }
    return { ok: true, cps: rs.cps, meter: rs.meter, tempoOverride: rs.tempoOverride };
  }

  return {
    prepareSong,
    async startSong(opts = {}) {
      if (!opts.prepared && !(await prepareSong()).ok) return null;
      if (get(runtime.store).structure?.meterChanges) return null;
      // A take's backing plays on superdough whatever the song's engine (its clock is the one
      // recording syncs to); the next stop returns to the song's.
      const r = await playback.start({
        section: opts.section ?? null,
        countIn: opts.countIn ?? false,
        backend: 'superdough',
      });
      if (r !== null && 'T' in r) startedSong = true;
      return r;
    },
    startMetronome: (grid) => metronome.start(grid),
    stopAll() {
      metronome.stop();
      if (!startedSong) return;
      startedSong = false;
      if (get(runtime.store).started) runtime.stop();
    },
  };
}

/**
 * Plays a voice memo on the live audio graph: alone, or in time with the metronome, the song, or
 * both. The WAV is read whole and decoded with `decodeWav` into an `AudioBuffer` on the backend's
 * context (the context resamples; never `decodeAudioData`). The song and the metronome go through
 * the backing starter the recording controller uses, so the two cannot diverge. Never creates a
 * context: `graph()` is called only once `isRunning()`.
 */
import { get, writable, type Readable } from 'svelte/store';
import {
  BACKING_MESSAGES,
  LEAD_S,
  type BackingStarter,
  type SectionChoice,
} from '../recording/backing';
import { formatDuration, tempoChanged, tempoNotice, type SongTempo } from '../recording/memo-ui';
import type { Sidecar } from '../recording/types';
import { errorMessage } from '../errors';
import { decodeWav } from '../render/wav';
import type { Runtime } from '../strudel/runtime';
import { parseMeter } from '../strudel/structure';
import type { AudioGraph } from './backend';

export type PlayHow = 'alone' | 'metronome' | 'song' | 'song+metronome';

export type MemoPlayErrorKind = 'tooLong' | 'audio' | 'read' | 'tempo';

export type MemoPlayerState =
  | { status: 'idle' }
  | { status: 'loading'; stem: string; how: PlayHow }
  | { status: 'playing'; stem: string; how: PlayHow }
  | { status: 'error'; stem: string; kind: MemoPlayErrorKind; message: string };

/** What the player needs of a memo (a `RecordingEntry` has it). */
export interface PlayableMemo {
  stem: string;
  /** Seconds; null when the WAV header could not be read. */
  duration: number | null;
  sidecar: Sidecar | null;
}

export interface MemoPlayer {
  readonly store: Readable<MemoPlayerState>;
  /** Never rejects: a failure is the `error` state. Call from a user gesture. */
  play(item: PlayableMemo, how: PlayHow): Promise<void>;
  /** Stops playback (and what it started) and clears an error. Idempotent. */
  stop(): void;
  dispose(): void;
}

export interface MemoPlayerDeps {
  /** The backend's `graph()`; called only once `isRunning()`. */
  graph: () => AudioGraph | null;
  isRunning: () => boolean;
  /** `runtime.resumeAudio`. */
  resumeAudio: () => Promise<boolean>;
  /** The WAV's bytes (`recording_read` for the open project). */
  readBytes: (stem: string) => Promise<Uint8Array>;
  backing: Pick<BackingStarter, 'startSong' | 'startMetronome' | 'stopAll'>;
  /** A playing song is stopped before a memo plays alone or with the metronome. */
  runtime: Pick<Runtime, 'store' | 'stop'>;
  /** The open song's tempo from its static header; null without a song. */
  songTempo: () => SongTempo | null;
}

/** Longer memos are not read: the whole file crosses IPC. */
export const MAX_PLAY_SECONDS = 900;

export const PLAYER_MESSAGES = {
  ...BACKING_MESSAGES,
  noTiming: 'this memo has no timing saved',
  noSong: 'no song is open',
} as const;

/** The failure when the section the memo was recorded against is gone. */
export const missingSectionMessage = (name: string): string =>
  `section "${name}" no longer exists in this song`;

/** The section the take armed (looped, or started from); null for a take from bar 0. */
export function memoSection(backing: Sidecar['backing']): SectionChoice | null {
  if (backing.loopSection !== null) return { name: backing.loopSection, loop: true };
  if (backing.startSection !== null) return { name: backing.startSection, loop: false };
  return null;
}

export const tooLongMessage = (duration: number): string =>
  `this memo is ${formatDuration(duration)} long; playback is limited to ${formatDuration(MAX_PLAY_SECONDS)}`;

/**
 * When and from where the memo starts so that its frame `bar1Frame` sounds at context time `T`:
 * `{ when, offset }` in seconds, `when` never before `now` and `offset` never negative; null when
 * the memo would already be over.
 */
export function memoStart(
  T: number,
  bar1Frame: number,
  sampleRate: number,
  now: number,
  durationSec: number,
): { when: number; offset: number } | null {
  const ideal = T - bar1Frame / sampleRate;
  if (ideal >= now) return { when: ideal, offset: 0 };
  const offset = now - ideal;
  if (offset >= durationSec) return null;
  return { when: now, offset };
}

/** Clicks per bar for a sidecar's tempo: the meter's numerator, as the take's metronome clicked. */
export function clicksPerBar(tempo: Sidecar['tempo']): number {
  return parseMeter(tempo.meter)?.num ?? Math.max(1, Math.round(tempo.beatsPerBar));
}

const usesSong = (how: PlayHow): boolean => how === 'song' || how === 'song+metronome';

export function createMemoPlayer(deps: MemoPlayerDeps): MemoPlayer {
  const store = writable<MemoPlayerState>({ status: 'idle' });
  /** Bumped by every play and stop; a stale step returns. */
  let token = 0;
  let source: AudioBufferSourceNode | null = null;
  let backingStarted = false;
  let disposed = false;

  const graphOrNull = (): AudioGraph | null => {
    if (!deps.isRunning()) return null;
    try {
      return deps.graph();
    } catch {
      return null;
    }
  };

  /** Silence what this player started; leaves the state alone. */
  function silence(): void {
    const s = source;
    source = null;
    if (s) {
      s.onended = null;
      try {
        s.stop();
      } catch {
        // never started, or already stopped
      }
      try {
        s.disconnect();
      } catch {
        // already disconnected
      }
    }
    if (backingStarted) {
      backingStarted = false;
      deps.backing.stopAll();
    }
  }

  function fail(t: number, stem: string, kind: MemoPlayErrorKind, message: string): void {
    if (t !== token) return;
    silence();
    store.set({ status: 'error', stem, kind, message });
  }

  function stop(): void {
    token++;
    silence();
    store.set({ status: 'idle' });
  }

  async function play(item: PlayableMemo, how: PlayHow): Promise<void> {
    if (disposed) return;
    stop();
    const t = token;
    const { stem, sidecar } = item;
    if (item.duration !== null && item.duration > MAX_PLAY_SECONDS) {
      return fail(t, stem, 'tooLong', tooLongMessage(item.duration));
    }
    const bar1Frame = sidecar?.timing.bar1Frame ?? null;
    if (how !== 'alone' && (sidecar === null || bar1Frame === null)) {
      return fail(t, stem, 'read', PLAYER_MESSAGES.noTiming);
    }
    if (usesSong(how)) {
      const song = deps.songTempo();
      if (song === null) return fail(t, stem, 'audio', PLAYER_MESSAGES.noSong);
      if (song.mixedMeter) return fail(t, stem, 'audio', PLAYER_MESSAGES.mixedMeter);
      // A take with a `setcps` recorded the evaluated tempo, which the static header cannot
      // show: only the check after the evaluate below applies.
      if (!sidecar!.tempo.tempoOverride && tempoChanged(sidecar!.tempo, song)) {
        return fail(t, stem, 'tempo', tempoNotice(sidecar!.tempo, song));
      }
    }
    store.set({ status: 'loading', stem, how });

    let running = false;
    try {
      running = await deps.resumeAudio();
    } catch (e) {
      console.error(e);
    }
    if (t !== token) return;
    if (!running) return fail(t, stem, 'audio', PLAYER_MESSAGES.audioOff);

    let samples: Float32Array;
    let sampleRate: number;
    try {
      const decoded = decodeWav(await deps.readBytes(stem));
      if (t !== token) return;
      const first = decoded.channels[0];
      if (!first || first.length === 0) throw new Error('the file has no audio');
      samples = first;
      sampleRate = decoded.sampleRate;
    } catch (e) {
      if (t !== token) return;
      return fail(t, stem, 'read', `could not read memo: ${errorMessage(e)}`);
    }

    if (!usesSong(how) && get(deps.runtime.store).started) deps.runtime.stop();

    // The song first: it evaluates, and starting it can take a while.
    let T: number | null = null;
    let cps = sidecar?.tempo.cps ?? 0;
    if (usesSong(how)) {
      backingStarted = true;
      const r = await deps.backing.startSong({ section: memoSection(sidecar!.backing) });
      if (t !== token) {
        // Stopped while the song was starting: silence it, unless a newer play owns the backing.
        if (r && !backingStarted) deps.backing.stopAll();
        return;
      }
      if (r && 'missingSection' in r) {
        return fail(t, stem, 'read', missingSectionMessage(r.missingSection));
      }
      if (!r) return fail(t, stem, 'audio', PLAYER_MESSAGES.songFailed);
      // A `setcps` only the evaluate reveals.
      const playing: SongTempo = { cps: r.cps, meter: get(deps.runtime.store).meter };
      if (tempoChanged(sidecar!.tempo, playing)) {
        return fail(t, stem, 'tempo', tempoNotice(sidecar!.tempo, playing));
      }
      T = r.T;
      cps = r.cps;
    }

    const graph = graphOrNull();
    if (!graph) return fail(t, stem, 'audio', PLAYER_MESSAGES.audioOff);
    const ctx = graph.context;
    const now = ctx.currentTime;
    const durationSec = samples.length / sampleRate;

    if (how === 'metronome') T = now + LEAD_S;
    if (how === 'metronome' || how === 'song+metronome') {
      backingStarted = true;
      const ok = deps.backing.startMetronome({
        T: T!,
        cps,
        num: clicksPerBar(sidecar!.tempo),
        from: 0,
      });
      if (!ok) return fail(t, stem, 'audio', PLAYER_MESSAGES.metronomeFailed);
    }

    let start: { when: number; offset: number } | null = { when: now, offset: 0 };
    if (how !== 'alone') {
      start = memoStart(T!, bar1Frame!, sampleRate, now, durationSec);
      if (start === null) return stop();
    }

    let node: AudioBufferSourceNode;
    try {
      const buffer = ctx.createBuffer(1, samples.length, sampleRate);
      buffer.copyToChannel(samples as Float32Array<ArrayBuffer>, 0);
      node = ctx.createBufferSource();
      node.buffer = buffer;
      node.connect(graph.destination);
      node.onended = () => {
        if (t === token && source === node) stop();
      };
      source = node;
      node.start(start.when, start.offset);
    } catch (e) {
      return fail(t, stem, 'audio', `could not play memo: ${errorMessage(e)}`);
    }
    store.set({ status: 'playing', stem, how });
  }

  return {
    store: { subscribe: store.subscribe },
    play,
    stop,
    dispose() {
      if (disposed) return;
      stop();
      disposed = true;
    },
  };
}

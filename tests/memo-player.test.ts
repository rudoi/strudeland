import { describe, expect, test } from 'bun:test';
import { get, writable } from 'svelte/store';
import {
  clicksPerBar,
  createMemoPlayer,
  MAX_PLAY_SECONDS,
  memoSection,
  memoStart,
  missingSectionMessage,
  PLAYER_MESSAGES,
  tooLongMessage,
  type MemoPlayerDeps,
  type PlayableMemo,
} from '../src/lib/audio/memo-player';
import type { MetronomeGrid } from '../src/lib/audio/metronome';
import { LEAD_S, type StartSongResult } from '../src/lib/recording/backing';
import type { SongTempo } from '../src/lib/recording/memo-ui';
import type { Sidecar } from '../src/lib/recording/types';
import { encodeWav } from '../src/lib/render/wav';
import type { RuntimeState } from '../src/lib/strudel/runtime';
import { fakeAudioGraph } from './helpers/fake-audio-graph';
import { tick } from './helpers/fake-project-api';

describe('memoStart', () => {
  test('a downbeat far enough ahead keeps offset 0', () => {
    // bar 1 at frame 24000 of 48 kHz: 0.5 s in; T = 10 s, so the memo starts at 9.5 s.
    expect(memoStart(10, 24000, 48000, 9, 60)).toEqual({ when: 9.5, offset: 0 });
    expect(memoStart(10, 24000, 48000, 9.5, 60)).toEqual({ when: 9.5, offset: 0 });
  });

  test('a late start begins now, partway in', () => {
    expect(memoStart(10, 48000, 48000, 9.5, 60)).toEqual({ when: 9.5, offset: 0.5 });
  });

  test('nothing left to play is null', () => {
    expect(memoStart(10, 48000, 48000, 12, 3)).toBeNull();
    // offset exactly the duration
    expect(memoStart(10, 48000, 48000, 11, 2)).toBeNull();
    expect(memoStart(10, 48000, 48000, 10.5, 2)).toEqual({ when: 10.5, offset: 1.5 });
  });

  test('bar 1 at frame 0 and a fractional frame', () => {
    expect(memoStart(4, 0, 44100, 3, 1)).toEqual({ when: 4, offset: 0 });
    expect(memoStart(4, 22050.5, 44100, 1, 10)).toEqual({ when: 4 - 22050.5 / 44100, offset: 0 });
  });

  test('never a negative offset or a past start', () => {
    for (const now of [0, 0.4, 0.5, 0.6, 1, 2.9]) {
      const r = memoStart(1, 24000, 48000, now, 3);
      expect(r === null || (r.offset >= 0 && r.when >= now)).toBe(true);
    }
  });
});

describe('clicksPerBar', () => {
  test('the meter numerator, as the take clicked', () => {
    expect(clicksPerBar(tempo({ meter: '6/8', beatsPerBar: 3 }))).toBe(6);
    expect(clicksPerBar(tempo({ meter: '3/4', beatsPerBar: 3 }))).toBe(3);
    expect(clicksPerBar(tempo({ meter: 'x', beatsPerBar: 4 }))).toBe(4);
  });
});

test('tooLongMessage', () => {
  expect(tooLongMessage(1100)).toBe('this memo is 18:20 long; playback is limited to 15:00');
  expect(MAX_PLAY_SECONDS).toBe(900);
});

function tempo(p: Partial<Sidecar['tempo']> = {}): Sidecar['tempo'] {
  return { bpm: 120, meter: '4/4', cps: 0.5, beatsPerBar: 4, tempoOverride: false, ...p };
}

function sidecar(bar1Frame: number | null, t: Partial<Sidecar['tempo']> = {}): Sidecar {
  return {
    schema: 1,
    createdAt: '2026-01-01T00:00:00Z',
    app: { version: '0.1.0' },
    input: { deviceId: 'mic', deviceName: 'Mic', channel: 0, channels: 1, sampleRate: 48000 },
    output: { baseLatency: 0.01, outputLatency: null },
    tempo: tempo(t),
    backing: {
      song: true,
      metronome: false,
      countIn: true,
      songPath: 'songs/a.js',
      startSection: null,
      loopSection: null,
    },
    timing: {
      bar1Frame,
      bar1HostNs: bar1Frame === null ? null : '1',
      clockOffsetNs: { start: null, end: null },
      anchors: [],
      outputStamps: [],
    },
    xruns: 0,
    recovered: false,
  };
}

const memo = (p: Partial<PlayableMemo> = {}): PlayableMemo => ({
  stem: 'a',
  duration: 2,
  sidecar: sidecar(24000),
  ...p,
});

/** Two seconds at 48 kHz, the first channel a ramp. */
const WAV = encodeWav(
  [Float32Array.from({ length: 96000 }, (_, i) => i / 96000), new Float32Array(96000)],
  48000,
  'f32',
);

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

function setup(opts: { running?: boolean; song?: SongTempo | null; currentTime?: number } = {}) {
  const g = fakeAudioGraph({ currentTime: opts.currentTime ?? 5 });
  const log: string[] = [];
  const grids: MetronomeGrid[] = [];
  const rt = writable({ started: false, cps: 0.5, meter: { num: 4, den: 4 } } as RuntimeState);
  let running = opts.running ?? true;
  let resume: () => Promise<boolean> = async () => running;
  let read: (stem: string) => Promise<Uint8Array> = async () => WAV;
  let songResult: StartSongResult = { T: 6, cps: 0.5 };
  const songOpts: unknown[] = [];
  let songHold: Promise<void> | null = null;
  let metronomeOk = true;
  const song = opts.song === undefined ? { cps: 0.5, meter: { num: 4, den: 4 } } : opts.song;
  const deps: MemoPlayerDeps = {
    graph: () => g.graph,
    isRunning: () => running,
    resumeAudio: () => (log.push('resume'), resume()),
    readBytes: (stem) => (log.push(`read:${stem}`), read(stem)),
    backing: {
      startSong: async (o) => {
        log.push('startSong');
        songOpts.push(o);
        if (songHold) await songHold;
        return songResult;
      },
      startMetronome: (grid) => (log.push('metronome'), grids.push(grid), metronomeOk),
      stopAll: () => void log.push('stopAll'),
    },
    runtime: {
      store: { subscribe: rt.subscribe },
      stop: () => {
        log.push('runtime.stop');
        rt.update((s) => ({ ...s, started: false }));
      },
    },
    songTempo: () => song,
  };
  const player = createMemoPlayer(deps);
  return {
    g,
    log,
    grids,
    rt,
    player,
    state: () => get(player.store),
    setRunning: (r: boolean) => (running = r),
    setResume: (f: () => Promise<boolean>) => (resume = f),
    setRead: (f: (stem: string) => Promise<Uint8Array>) => (read = f),
    setSong: (r: StartSongResult) => (songResult = r),
    /** Every `startSong` call's options, in order. */
    songOpts,
    holdSong: (p: Promise<void> | null) => (songHold = p),
    failMetronome: () => (metronomeOk = false),
  };
}

describe('the memo player', () => {
  test('starts idle', () => {
    expect(setup().state()).toEqual({ status: 'idle' });
  });

  test('a memo over the limit errors before any read; the limit itself plays', async () => {
    const s = setup();
    await s.player.play(memo({ duration: 901 }), 'alone');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'tooLong',
      message: 'this memo is 15:01 long; playback is limited to 15:00',
    });
    expect(s.log).toEqual([]);
    await s.player.play(memo({ duration: 900 }), 'alone');
    expect(s.state()).toMatchObject({ status: 'playing', stem: 'a', how: 'alone' });
  });

  test('suspended audio', async () => {
    const s = setup({ running: false });
    await s.player.play(memo(), 'alone');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'audio',
      message: 'audio is not running: enable it in the status bar',
    });
    expect(s.log).toEqual(['resume']);
  });

  test('a rejected resume is suspended audio', async () => {
    const s = setup();
    s.setResume(() => Promise.reject(new Error('nope')));
    const err = console.error;
    console.error = () => {};
    await s.player.play(memo(), 'alone');
    console.error = err;
    expect(s.state()).toMatchObject({ kind: 'audio', message: PLAYER_MESSAGES.audioOff });
  });

  test('a decode failure is a read error', async () => {
    const s = setup();
    s.setRead(async () => new Uint8Array([1, 2, 3]));
    await s.player.play(memo(), 'alone');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'read',
      message: 'could not read memo: not a RIFF/WAVE file',
    });
    expect(s.g.sources()).toEqual([]);
  });

  test('a read failure is a read error', async () => {
    const s = setup();
    s.setRead(() => Promise.reject(new Error('recording a not found')));
    await s.player.play(memo(), 'alone');
    expect(s.state()).toMatchObject({
      kind: 'read',
      message: 'could not read memo: recording a not found',
    });
  });

  test('alone plays the first channel now, from the start', async () => {
    const s = setup();
    await s.player.play(memo({ sidecar: null }), 'alone');
    expect(s.state()).toEqual({ status: 'playing', stem: 'a', how: 'alone' });
    expect(s.log).toEqual(['resume', 'read:a']);
    const [buf] = s.g.buffers;
    expect(buf).toMatchObject({ numberOfChannels: 1, length: 96000, sampleRate: 48000 });
    expect(buf!.data[0]![48000]).toBe(0.5);
    const [src] = s.g.sources();
    expect(src!.buffer).toBe(buf!);
    expect(src!.starts).toEqual([[5, 0]]);
    expect(s.g.destination.connects).toEqual([src]);
  });

  test('alone stops a playing song first', async () => {
    const s = setup();
    s.rt.update((st) => ({ ...st, started: true }));
    await s.player.play(memo(), 'alone');
    expect(s.log).toEqual(['resume', 'read:a', 'runtime.stop']);
  });

  test('metronome mode: the grid from now + 0.25 and the downbeat lined up', async () => {
    const s = setup({ currentTime: 5 });
    await s.player.play(memo({ sidecar: sidecar(6000, { cps: 0.75, meter: '3/4' }) }), 'metronome');
    const T = 5 + LEAD_S;
    expect(s.grids).toEqual([{ T, cps: 0.75, num: 3, from: 0 }]);
    // 6000 frames = 0.125 s before the downbeat.
    expect(s.g.sources()[0]!.starts).toEqual([[T - 0.125, 0]]);
    expect(s.state()).toEqual({ status: 'playing', stem: 'a', how: 'metronome' });
  });

  test('metronome mode with a long pre-roll starts partway in', async () => {
    const s = setup({ currentTime: 5 });
    await s.player.play(memo({ sidecar: sidecar(48000) }), 'metronome');
    expect(s.g.sources()[0]!.starts).toEqual([[5, 0.75]]);
  });

  test('a failed metronome is an audio error and stops the backing', async () => {
    const s = setup();
    s.failMetronome();
    await s.player.play(memo(), 'metronome');
    expect(s.state()).toMatchObject({ kind: 'audio', message: PLAYER_MESSAGES.metronomeFailed });
    expect(s.log.at(-1)).toBe('stopAll');
    expect(s.g.sources()).toEqual([]);
  });

  test('song mode uses the T from the backing', async () => {
    const s = setup({ currentTime: 5 });
    s.setSong({ T: 5.75, cps: 0.5 });
    await s.player.play(memo({ sidecar: sidecar(12000) }), 'song');
    expect(s.log).toEqual(['resume', 'read:a', 'startSong']);
    expect(s.grids).toEqual([]);
    expect(s.g.sources()[0]!.starts).toEqual([[5.5, 0]]);
    expect(s.state()).toEqual({ status: 'playing', stem: 'a', how: 'song' });
  });

  test('song+metronome runs the metronome on the song grid', async () => {
    const s = setup({ currentTime: 5 });
    s.setSong({ T: 5.75, cps: 0.5 });
    await s.player.play(memo({ sidecar: sidecar(12000) }), 'song+metronome');
    expect(s.grids).toEqual([{ T: 5.75, cps: 0.5, num: 4, from: 0 }]);
    expect(s.state()).toMatchObject({ status: 'playing', how: 'song+metronome' });
  });

  test('a song that cannot start is an audio error', async () => {
    const s = setup();
    s.setSong(null);
    await s.player.play(memo(), 'song');
    expect(s.state()).toMatchObject({ kind: 'audio', message: PLAYER_MESSAGES.songFailed });
    expect(s.log.at(-1)).toBe('stopAll');
  });

  test('no timing: the timed modes refuse before any read', async () => {
    for (const sc of [null, sidecar(null)]) {
      for (const how of ['metronome', 'song', 'song+metronome'] as const) {
        const s = setup();
        await s.player.play(memo({ sidecar: sc }), how);
        expect(s.state()).toEqual({
          status: 'error',
          stem: 'a',
          kind: 'read',
          message: 'this memo has no timing saved',
        });
        expect(s.log).toEqual([]);
      }
    }
  });

  test('song modes refuse without a song', async () => {
    const s = setup({ song: null });
    await s.player.play(memo(), 'song');
    expect(s.state()).toMatchObject({ kind: 'audio', message: PLAYER_MESSAGES.noSong });
    expect(s.log).toEqual([]);
  });

  test('a changed song tempo refuses song playback before anything starts', async () => {
    const s = setup({ song: { cps: 100 / 240, meter: { num: 4, den: 4 } } });
    await s.player.play(memo(), 'song');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'tempo',
      message: 'memo was recorded at 120 · 4/4; the song is now 100 · 4/4',
    });
    expect(s.log).toEqual([]);
    // Alone and with the metronome still play at the memo's own tempo.
    await s.player.play(memo(), 'metronome');
    expect(s.state()).toMatchObject({ status: 'playing', how: 'metronome' });
  });

  test('a tempo override found at evaluate time stops at once', async () => {
    const s = setup();
    s.setSong({ T: 6, cps: 0.6 });
    s.rt.update((st) => ({ ...st, cps: 0.6 }));
    await s.player.play(memo(), 'song');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'tempo',
      message: 'memo was recorded at 120 · 4/4; the song is now 144 · 4/4',
    });
    expect(s.log).toEqual(['resume', 'read:a', 'startSong', 'stopAll']);
    expect(s.g.sources()).toEqual([]);
  });

  test('a take with a setcps override skips the static check and plays at its evaluated tempo', async () => {
    // The header says 100 bpm; the take's `setcps` ran it (and the song still runs) at 144.
    const s = setup({ song: { cps: 100 / 240, meter: { num: 4, den: 4 } } });
    s.setSong({ T: 6, cps: 0.6 });
    s.rt.update((st) => ({ ...st, cps: 0.6 }));
    const m = memo({ sidecar: sidecar(24000, { bpm: 144, cps: 0.6, tempoOverride: true }) });
    await s.player.play(m, 'song');
    expect(s.state()).toEqual({ status: 'playing', stem: 'a', how: 'song' });
    // The evaluated tempo still has to match.
    s.setSong({ T: 6, cps: 0.5 });
    s.rt.update((st) => ({ ...st, cps: 0.5 }));
    await s.player.play(m, 'song');
    expect(s.state()).toMatchObject({ status: 'error', kind: 'tempo' });
  });

  test('song modes arm the section the take started from or looped', async () => {
    const s = setup();
    const at = (b: Partial<Sidecar['backing']>) => {
      const sc = sidecar(24000);
      sc.backing = { ...sc.backing, ...b };
      return memo({ sidecar: sc });
    };
    await s.player.play(memo(), 'song');
    await s.player.play(at({ startSection: 'verse' }), 'song');
    await s.player.play(at({ loopSection: 'chorus' }), 'song+metronome');
    expect(s.songOpts).toEqual([
      { section: null },
      { section: { name: 'verse', loop: false } },
      { section: { name: 'chorus', loop: true } },
    ]);
    expect(memoSection({ ...sidecar(0).backing, startSection: 'a' })).toEqual({
      name: 'a',
      loop: false,
    });
  });

  test('a section the song no longer has refuses playback and stops the backing', async () => {
    const s = setup();
    s.setSong({ missingSection: 'verse' });
    const sc = sidecar(24000);
    sc.backing = { ...sc.backing, startSection: 'verse' };
    await s.player.play(memo({ sidecar: sc }), 'song');
    expect(s.state()).toEqual({
      status: 'error',
      stem: 'a',
      kind: 'read',
      message: 'section "verse" no longer exists in this song',
    });
    expect(missingSectionMessage('verse')).toBe('section "verse" no longer exists in this song');
    expect(s.log.at(-1)).toBe('stopAll');
    expect(s.g.sources()).toEqual([]);
  });

  test('a memo already over when the song starts goes idle and stops the backing', async () => {
    const s = setup({ currentTime: 20 });
    s.setSong({ T: 6, cps: 0.5 });
    await s.player.play(memo(), 'song');
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.log.at(-1)).toBe('stopAll');
    expect(s.g.sources()).toEqual([]);
  });

  test('a second play stops the first; a late decode of the first never starts', async () => {
    const s = setup();
    const first = deferred<Uint8Array>();
    s.setRead(() => first.promise);
    const p1 = s.player.play(memo({ stem: 'a' }), 'alone');
    await Promise.resolve();
    expect(s.state()).toEqual({ status: 'loading', stem: 'a', how: 'alone' });
    s.setRead(async () => WAV);
    await s.player.play(memo({ stem: 'b' }), 'alone');
    expect(s.state()).toEqual({ status: 'playing', stem: 'b', how: 'alone' });
    first.resolve(WAV);
    await p1;
    expect(s.g.sources()).toHaveLength(1);
    expect(s.state()).toMatchObject({ stem: 'b' });
  });

  test('a second play stops a playing first and its backing', async () => {
    const s = setup();
    await s.player.play(memo({ stem: 'a' }), 'metronome');
    const [src] = s.g.sources();
    await s.player.play(memo({ stem: 'b' }), 'alone');
    expect(src!.stops).toBe(1);
    expect(src!.disconnected).toBe(true);
    expect(s.log.filter((l) => l === 'stopAll')).toHaveLength(1);
  });

  test('a play during a song start drops the stale start', async () => {
    const s = setup();
    const hold = deferred<void>();
    s.holdSong(hold.promise);
    const p1 = s.player.play(memo({ stem: 'a' }), 'song');
    await tick();
    s.player.stop();
    const before = s.log.length;
    hold.resolve();
    await p1;
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.g.sources()).toEqual([]);
    // The song that started late is stopped again.
    expect(s.log.slice(before)).toEqual(['stopAll']);
  });

  test('onended returns to idle and stops the backing', async () => {
    const s = setup();
    await s.player.play(memo(), 'metronome');
    s.g.sources()[0]!.end();
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.log.at(-1)).toBe('stopAll');
  });

  test('alone: onended starts no backing stop', async () => {
    const s = setup();
    await s.player.play(memo(), 'alone');
    s.g.sources()[0]!.end();
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.log).not.toContain('stopAll');
  });

  test('stop() during loading: nothing starts', async () => {
    const s = setup();
    const bytes = deferred<Uint8Array>();
    s.setRead(() => bytes.promise);
    const p = s.player.play(memo(), 'alone');
    await Promise.resolve();
    s.player.stop();
    bytes.resolve(WAV);
    await p;
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.g.sources()).toEqual([]);
  });

  test('stop() clears an error', async () => {
    const s = setup();
    await s.player.play(memo({ duration: 1000 }), 'alone');
    s.player.stop();
    expect(s.state()).toEqual({ status: 'idle' });
  });

  test('dispose stops and ignores later plays', async () => {
    const s = setup();
    await s.player.play(memo(), 'metronome');
    s.player.dispose();
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.g.sources()[0]!.stops).toBe(1);
    await s.player.play(memo(), 'alone');
    expect(s.state()).toEqual({ status: 'idle' });
    expect(s.g.sources()).toHaveLength(1);
  });

  test('no graph after resuming is an audio error', async () => {
    const s = setup();
    s.setResume(async () => {
      s.setRunning(false);
      return true;
    });
    await s.player.play(memo(), 'alone');
    expect(s.state()).toMatchObject({ kind: 'audio', message: PLAYER_MESSAGES.audioOff });
  });
});

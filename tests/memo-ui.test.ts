import { describe, expect, test } from 'bun:test';
import { get } from 'svelte/store';
import {
  backingChoice,
  channelChoices,
  createBackingChoice,
  DEFAULT_CHOICE,
  effectiveBacking,
  BUSY_TITLE,
  formatDuration,
  formatElapsed,
  hasTiming,
  inputLabel,
  memoChips,
  memoPlayItems,
  type MemoChip,
  memoRow,
  sortMemos,
  tempoChanged,
  tempoChangedTitle,
  tempoNotice,
  tempoText,
  MIC_SETTINGS_URL,
  pickerDisabled,
  recordButton,
  recordingNotices,
  takeRunning,
  toggle,
  toggleViews,
  type ToggleChoice,
} from '../src/lib/recording/memo-ui';
import type { MemoPlayerState } from '../src/lib/audio/memo-player';
import { entry } from './helpers/fake-recording-api';
import type {
  InputDevice,
  RecordingEntry,
  Sidecar,
  RecordingPhase,
  RecordingUiError,
  RecordingUiState,
} from '../src/lib/recording/types';

const mic: InputDevice = {
  id: 'mic',
  name: 'MacBook Pro Microphone',
  channels: 1,
  sampleRate: 48000,
  isDefault: true,
  bluetooth: false,
};
const iface: InputDevice = {
  id: 'iface',
  name: 'Scarlett 2i2',
  channels: 2,
  sampleRate: 48000,
  isDefault: false,
  bluetooth: false,
};
const airpods: InputDevice = {
  id: 'bt',
  name: 'AirPods',
  channels: 1,
  sampleRate: 24000,
  isDefault: false,
  bluetooth: true,
};

const state = (p: Partial<RecordingUiState> = {}): RecordingUiState => ({
  phase: 'armed',
  countInBeat: null,
  takeStartedAtMs: null,
  devices: [mic, iface, airpods],
  input: { deviceId: 'mic', channel: 0 },
  error: null,
  permission: 'authorized',
  warning: null,
  stem: null,
  lastStem: null,
  ...p,
});

const err = (kind: RecordingUiError['kind'], message = 'boom'): RecordingUiError => ({
  kind,
  message,
});

const PHASES: RecordingPhase[] = [
  'idle',
  'arming',
  'armed',
  'starting',
  'count-in',
  'recording',
  'stopping',
];

describe('takeRunning', () => {
  test('starting, count-in, recording and stopping', () => {
    expect(PHASES.filter(takeRunning)).toEqual(['starting', 'count-in', 'recording', 'stopping']);
  });
});

describe('formatElapsed', () => {
  test.each([
    [0, '0:00'],
    [7_000, '0:07'],
    [7_999, '0:07'],
    [65_000, '1:05'],
    [900_000, '15:00'],
    [3_599_999, '59:59'],
    [3_600_000, '1:00:00'],
    [3_725_000, '1:02:05'],
    [36_000_000, '10:00:00'],
  ])('%p ms is %p', (ms, text) => {
    expect(formatElapsed(ms)).toBe(text);
  });

  test('negative and non-finite are 0:00', () => {
    expect(formatElapsed(-5)).toBe('0:00');
    expect(formatElapsed(NaN)).toBe('0:00');
    expect(formatElapsed(Infinity)).toBe('0:00');
  });
});

describe('recordButton', () => {
  const look = (b: ReturnType<typeof recordButton>) => ({
    glyph: b.glyph,
    ariaLabel: b.ariaLabel,
    tone: b.tone,
    variant: b.variant,
    disabled: b.disabled,
    title: b.title,
    action: b.action,
  });
  const off = { glyph: 'dot', ariaLabel: 'record', tone: 'neutral', variant: 'default' } as const;

  test('no project', () => {
    expect(look(recordButton(state(), false))).toEqual({
      ...off,
      disabled: true,
      title: 'open a project to record',
      action: null,
    });
  });

  test('no controller reads as no project', () => {
    expect(recordButton(null, true).title).toBe('open a project to record');
    expect(recordButton(null, true).disabled).toBe(true);
  });

  test.each([
    ['noDevice', 'no input device found'],
    ['permissionDenied', 'microphone access denied'],
    ['deviceLost', 'input device lost'],
  ] as const)('idle with %p', (kind, title) => {
    expect(look(recordButton(state({ phase: 'idle', error: err(kind) }), true))).toEqual({
      ...off,
      disabled: true,
      title,
      action: null,
    });
  });

  test('idle without an error', () => {
    expect(recordButton(state({ phase: 'idle' }), true).title).toBe('the microphone is not ready');
  });

  test('arming', () => {
    expect(look(recordButton(state({ phase: 'arming' }), true))).toEqual({
      ...off,
      disabled: true,
      title: 'waiting for microphone access',
      action: null,
    });
  });

  test('armed records', () => {
    expect(look(recordButton(state(), true))).toEqual({
      glyph: 'dot',
      ariaLabel: 'record',
      tone: 'danger',
      variant: 'default',
      disabled: false,
      title: 'record a memo',
      action: 'record',
    });
  });

  test('armed with a kept error still records', () => {
    expect(recordButton(state({ error: err('limitReached') }), true).action).toBe('record');
  });

  test('starting', () => {
    expect(look(recordButton(state({ phase: 'starting' }), true))).toEqual({
      glyph: 'dot',
      ariaLabel: 'record',
      tone: 'danger',
      variant: 'default',
      disabled: true,
      title: 'starting the recording',
      action: null,
    });
  });

  test('count-in shows the beat and stops', () => {
    const b = recordButton(state({ phase: 'count-in', countInBeat: 3 }), true);
    expect(b).toMatchObject({ text: 'count-in', mono: '3' });
    expect(look(b)).toEqual({
      glyph: null,
      ariaLabel: 'count-in 3',
      tone: 'danger',
      variant: 'default',
      disabled: false,
      title: 'stop recording',
      action: 'stop',
    });
  });

  test('count-in before the first beat', () => {
    const b = recordButton(state({ phase: 'count-in', countInBeat: null }), true);
    expect(b.mono).toBeNull();
    expect(b.ariaLabel).toBe('count-in');
  });

  test('recording shows the square and the elapsed time', () => {
    const b = recordButton(state({ phase: 'recording', takeStartedAtMs: 0 }), true, 65_400);
    expect(b).toMatchObject({ text: '', mono: '1:05' });
    expect(look(b)).toEqual({
      glyph: 'square',
      ariaLabel: '1:05',
      tone: 'danger',
      variant: 'primary',
      disabled: false,
      title: 'stop recording',
      action: 'stop',
    });
  });

  test('stopping', () => {
    expect(look(recordButton(state({ phase: 'stopping' }), true))).toEqual({
      glyph: null,
      ariaLabel: 'saving…',
      tone: 'danger',
      variant: 'primary',
      disabled: true,
      title: 'saving the memo',
      action: null,
    });
  });

  test('without a project every phase is the disabled record', () => {
    for (const phase of PHASES) {
      const b = recordButton(state({ phase }), false);
      expect(b.disabled).toBe(true);
      expect(b.title).toBe('open a project to record');
    }
  });
});

describe('the backing choice', () => {
  test('defaults: song follows the open song, metronome off, count-in on', () => {
    expect(DEFAULT_CHOICE).toEqual({
      song: true,
      metronome: false,
      countIn: true,
      songTouched: false,
    });
    expect(get(createBackingChoice())).toEqual(DEFAULT_CHOICE);
    expect(get(backingChoice)).toEqual(DEFAULT_CHOICE);
  });

  test('a fresh store is not shared', () => {
    const a = createBackingChoice();
    a.set({ ...DEFAULT_CHOICE, metronome: true });
    expect(get(createBackingChoice()).metronome).toBe(false);
  });

  test('untouched song follows whether a song is open', () => {
    expect(effectiveBacking(DEFAULT_CHOICE, true)).toEqual({
      song: true,
      metronome: false,
      countIn: true,
    });
    expect(effectiveBacking({ ...DEFAULT_CHOICE, song: false }, true).song).toBe(true);
    expect(effectiveBacking(DEFAULT_CHOICE, false)).toEqual({
      song: false,
      metronome: false,
      countIn: false,
    });
  });

  test('a touched song keeps the choice, but needs a song', () => {
    const off: ToggleChoice = { ...DEFAULT_CHOICE, song: false, songTouched: true };
    expect(effectiveBacking(off, true).song).toBe(false);
    const on: ToggleChoice = { ...DEFAULT_CHOICE, song: true, songTouched: true };
    expect(effectiveBacking(on, true).song).toBe(true);
    expect(effectiveBacking(on, false).song).toBe(false);
  });

  test('count-in is forced off with the song and metronome both off', () => {
    const c: ToggleChoice = { song: false, metronome: false, countIn: true, songTouched: true };
    expect(effectiveBacking(c, true)).toEqual({ song: false, metronome: false, countIn: false });
    expect(effectiveBacking({ ...c, metronome: true }, true)).toEqual({
      song: false,
      metronome: true,
      countIn: true,
    });
    expect(effectiveBacking({ ...c, countIn: false, metronome: true }, true).countIn).toBe(false);
  });

  test('the first song click turns off what was shown and marks it touched', () => {
    const c = toggle(DEFAULT_CHOICE, 'song', true);
    expect(c).toEqual({ ...DEFAULT_CHOICE, song: false, songTouched: true });
    expect(toggle(c, 'song', true)).toEqual({ ...DEFAULT_CHOICE, song: true, songTouched: true });
  });

  test('a song click with no song open', () => {
    // Shown off (no song), so the click remembers on.
    expect(toggle({ ...DEFAULT_CHOICE, song: false }, 'song', false)).toEqual({
      ...DEFAULT_CHOICE,
      song: true,
      songTouched: true,
    });
  });

  test('metronome and count-in flip', () => {
    expect(toggle(DEFAULT_CHOICE, 'metronome', true).metronome).toBe(true);
    expect(toggle(DEFAULT_CHOICE, 'countIn', true).countIn).toBe(false);
    expect(toggle(DEFAULT_CHOICE, 'metronome', true).songTouched).toBe(false);
  });
});

describe('toggleViews', () => {
  const byKey = (v: ReturnType<typeof toggleViews>) => Object.fromEntries(v.map((t) => [t.key, t]));

  test('order, labels and enabled titles', () => {
    const v = toggleViews(DEFAULT_CHOICE, 'armed', true, true);
    expect(v.map((t) => [t.key, t.label, t.pressed, t.disabled, t.title])).toEqual([
      ['song', 'song', true, false, 'play the song while recording'],
      [
        'metronome',
        'metronome',
        false,
        false,
        'play a click while recording; wear headphones so it is not recorded',
      ],
      ['countIn', 'count-in', true, false, 'one bar of clicks before the take'],
    ]);
  });

  test('no song open: song off and disabled, count-in needs a source', () => {
    const v = byKey(toggleViews(DEFAULT_CHOICE, 'armed', true, false));
    expect(v.song).toMatchObject({ pressed: false, disabled: true, title: 'no song open' });
    expect(v.countIn).toMatchObject({
      pressed: true,
      disabled: true,
      title: 'needs the song or the metronome',
    });
    expect(v.metronome.disabled).toBe(false);
  });

  test('metronome on enables count-in without a song', () => {
    const v = byKey(toggleViews({ ...DEFAULT_CHOICE, metronome: true }, 'armed', true, false));
    expect(v.countIn).toMatchObject({
      disabled: false,
      title: 'one bar of clicks before the take',
    });
    expect(v.metronome.pressed).toBe(true);
  });

  test('all disabled without a project or a controller', () => {
    for (const v of [
      toggleViews(DEFAULT_CHOICE, 'armed', false, true),
      toggleViews(DEFAULT_CHOICE, null, true, true),
    ]) {
      expect(v.map((t) => t.disabled)).toEqual([true, true, true]);
    }
  });

  test('all disabled during a take, enabled in idle and arming', () => {
    for (const phase of PHASES) {
      const disabled = toggleViews(DEFAULT_CHOICE, phase, true, true).map((t) => t.disabled);
      expect(disabled).toEqual(takeRunning(phase) ? [true, true, true] : [false, false, false]);
    }
  });
});

describe('the device picker', () => {
  test('labels', () => {
    expect(inputLabel(state())).toBe('input: MacBook Pro Microphone');
    expect(inputLabel(state({ input: { deviceId: 'iface', channel: 1 } }))).toBe(
      'input: Scarlett 2i2, channel 2',
    );
    expect(inputLabel(state({ input: null }))).toBe('no input device');
    expect(inputLabel(state({ input: { deviceId: 'gone', channel: 0 } }))).toBe('no input device');
    expect(inputLabel(null)).toBe('no input device');
  });

  test('disabled without a project or during a take', () => {
    expect(pickerDisabled(state(), false)).toBe(true);
    expect(pickerDisabled(null, true)).toBe(true);
    for (const phase of PHASES) {
      expect(pickerDisabled(state({ phase }), true)).toBe(takeRunning(phase));
    }
  });

  test('channels for a multi-channel device only', () => {
    expect(channelChoices(state())).toEqual([]);
    expect(channelChoices(state({ input: { deviceId: 'iface', channel: 0 } }))).toEqual([0, 1]);
    expect(channelChoices(state({ input: null }))).toEqual([]);
    expect(channelChoices(null)).toEqual([]);
  });
});

describe('recordingNotices', () => {
  test('MIC_SETTINGS_URL', () => {
    expect(MIC_SETTINGS_URL).toBe(
      'x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone',
    );
  });

  test('none when all is well, or without a controller', () => {
    expect(recordingNotices(state())).toEqual([]);
    expect(recordingNotices(null)).toEqual([]);
  });

  test('permission denied links to settings', () => {
    const denied = state({
      phase: 'idle',
      error: err('permissionDenied', 'microphone access denied'),
    });
    expect(recordingNotices(denied, null, 'Win32')[0]?.action).toEqual({
      kind: 'link',
      label: 'open settings',
      href: 'ms-settings:privacy-microphone',
    });
    expect(recordingNotices(denied, null, 'Linux x86_64')[0]?.action).toBeUndefined();
    expect(recordingNotices(denied, null, 'MacIntel')).toEqual([
      {
        id: 'permission',
        tone: 'danger',
        text: 'microphone access denied',
        action: { kind: 'link', label: 'open settings', href: MIC_SETTINGS_URL },
      },
    ]);
  });

  test('no device', () => {
    expect(recordingNotices(state({ phase: 'idle', error: err('noDevice') }))).toEqual([
      { id: 'no-device', tone: 'warning', text: 'no input device found' },
    ]);
  });

  test('device lost', () => {
    expect(recordingNotices(state({ phase: 'idle', error: err('deviceLost') }))).toEqual([
      {
        id: 'device-lost',
        tone: 'warning',
        text: 'input device lost; any recording so far was saved',
      },
    ]);
  });

  test('limit reached is dismissable', () => {
    expect(recordingNotices(state({ error: err('limitReached') }))).toEqual([
      {
        id: 'limit-reached',
        tone: 'warning',
        text: 'recording stopped at the length limit; the memo was saved',
        action: { kind: 'dismiss' },
      },
    ]);
  });

  test.each(['busy', 'projectChanged', 'other', 'eval', 'meta', 'audio', 'clock'] as const)(
    'the %p error keeps its message',
    (kind) => {
      expect(recordingNotices(state({ error: err(kind, 'Device Busy') }))).toEqual([
        {
          id: 'recording-error',
          tone: 'danger',
          text: 'recording: Device Busy',
          action: { kind: 'dismiss' },
        },
      ]);
    },
  );

  test.each(['invalidName', 'exists', 'notFound'] as const)('%p is shown on the row', (kind) => {
    expect(recordingNotices(state({ error: err(kind) }))).toEqual([]);
  });

  test('a warning', () => {
    expect(recordingNotices(state({ warning: 'the tempo changed' }))).toEqual([
      { id: 'recording-warning', tone: 'warning', text: 'the tempo changed' },
    ]);
  });

  test('bluetooth while armed, counting in or recording', () => {
    const bt = { deviceId: 'bt', channel: 0 };
    const notice = {
      id: 'bluetooth',
      tone: 'warning' as const,
      text: 'bluetooth input: timing and quality suffer. use wired headphones',
    };
    for (const phase of PHASES) {
      const shown = phase === 'armed' || phase === 'count-in' || phase === 'recording';
      expect(recordingNotices(state({ phase, input: bt }))).toEqual(shown ? [notice] : []);
    }
  });

  test('order: the error, the warning, then bluetooth; one per id', () => {
    const n = recordingNotices(
      state({
        error: err('other'),
        warning: 'w',
        input: { deviceId: 'bt', channel: 0 },
      }),
    );
    expect(n.map((x) => x.id)).toEqual(['recording-error', 'recording-warning', 'bluetooth']);
    expect(new Set(n.map((x) => x.id)).size).toBe(n.length);
  });
});

describe('recordingNotices for memo playback', () => {
  const perr = (kind: 'tooLong' | 'audio' | 'read' | 'tempo', message = 'm'): MemoPlayerState => ({
    status: 'error',
    stem: 'a',
    kind,
    message,
  });

  test('nothing while idle, loading or playing', () => {
    expect(recordingNotices(state(), { status: 'idle' })).toEqual([]);
    expect(recordingNotices(state(), { status: 'loading', stem: 'a', how: 'alone' })).toEqual([]);
    expect(recordingNotices(null, { status: 'playing', stem: 'a', how: 'song' })).toEqual([]);
  });

  test('too long is a dismissable warning', () => {
    expect(recordingNotices(null, perr('tooLong', 'this memo is 18:20 long'))).toEqual([
      {
        id: 'play-too-long',
        tone: 'warning',
        text: 'this memo is 18:20 long',
        action: { kind: 'dismiss-playback' },
      },
    ]);
  });

  test('audio and read are dismissable dangers', () => {
    expect(recordingNotices(null, perr('audio', 'audio is off'))).toEqual([
      {
        id: 'play-audio',
        tone: 'danger',
        text: 'audio is off',
        action: { kind: 'dismiss-playback' },
      },
    ]);
    expect(recordingNotices(null, perr('read', 'could not read memo: x'))).toEqual([
      {
        id: 'play-read',
        tone: 'danger',
        text: 'could not read memo: x',
        action: { kind: 'dismiss-playback' },
      },
    ]);
  });

  test('tempo is a warning', () => {
    expect(recordingNotices(null, perr('tempo', 'memo was recorded at 120 · 4/4'))).toEqual([
      {
        id: 'play-tempo',
        tone: 'warning',
        text: 'memo was recorded at 120 · 4/4',
        action: { kind: 'dismiss-playback' },
      },
    ]);
  });

  test('after the recording notices', () => {
    const n = recordingNotices(state({ warning: 'w' }), perr('read'));
    expect(n.map((x) => x.id)).toEqual(['recording-warning', 'play-read']);
  });
});

describe('formatDuration', () => {
  test.each([
    [7, '0:07'],
    [65, '1:05'],
    [900, '15:00'],
    [1100, '18:20'],
    [3600, '1:00:00'],
    [7.0000001, '0:07'],
    [6.2, '0:07'],
    [0.3, '0:01'],
    [0, '0:00'],
    [-3, '0:00'],
    [NaN, '0:00'],
  ])('%p s is %p', (s, text) => {
    expect(formatDuration(s)).toBe(text);
  });

  test('an unknown duration is empty', () => {
    expect(formatDuration(null)).toBe('');
  });
});

function sc(p: { createdAt?: string; bar1Frame?: number | null; recovered?: boolean } = {}) {
  return {
    schema: 1,
    createdAt: p.createdAt ?? '2026-01-01T00:00:00Z',
    app: { version: '0.1.0' },
    input: { deviceId: 'mic', deviceName: 'Mic', channel: 0, channels: 1, sampleRate: 48000 },
    output: { baseLatency: 0, outputLatency: null },
    tempo: { bpm: 119.6, meter: '6/8', cps: 0.664444, beatsPerBar: 3, tempoOverride: false },
    backing: {
      song: false,
      metronome: true,
      countIn: false,
      songPath: null,
      startSection: null,
      loopSection: null,
    },
    timing: {
      bar1Frame: p.bar1Frame === undefined ? 100 : p.bar1Frame,
      bar1HostNs: null,
      clockOffsetNs: { start: null, end: null },
      anchors: [],
      outputStamps: [],
    },
    xruns: 0,
    recovered: p.recovered ?? false,
  } satisfies Sidecar;
}

describe('sortMemos', () => {
  test('newest first by createdAt, else the modification time; ties by stem', () => {
    const items: RecordingEntry[] = [
      entry('old', { modifiedMs: Date.parse('2025-01-01T00:00:00Z') }),
      entry('b', { sidecar: sc({ createdAt: '2026-03-01T00:00:00Z' }) }),
      // createdAt wins over a newer modification time
      entry('c', {
        sidecar: sc({ createdAt: '2025-06-01T00:00:00Z' }),
        modifiedMs: Date.parse('2027-01-01T00:00:00Z'),
      }),
      entry('a', { sidecar: sc({ createdAt: '2026-03-01T00:00:00Z' }) }),
      entry('mod', { modifiedMs: Date.parse('2026-02-01T00:00:00Z') }),
      // an unparseable createdAt falls back
      entry('bad', {
        sidecar: sc({ createdAt: 'yesterday' }),
        modifiedMs: Date.parse('2026-02-15T00:00:00Z'),
      }),
    ];
    expect(sortMemos(items).map((e) => e.stem)).toEqual(['a', 'b', 'bad', 'mod', 'c', 'old']);
    expect(items[0]!.stem).toBe('old');
  });

  test('empty', () => {
    expect(sortMemos([])).toEqual([]);
  });
});

describe('tempo', () => {
  const memo = { bpm: 120, meter: '4/4', cps: 0.5, beatsPerBar: 4, tempoOverride: false };
  const song = (bpm: number, num = 4, den = 4) => ({
    cps: bpm / 60 / ((num * 4) / den),
    meter: { num, den },
  });

  test('text', () => {
    expect(tempoText(119.6, '6/8')).toBe('120 · 6/8');
  });

  test('a match within 1e-6 cps', () => {
    expect(tempoChanged(memo, song(120))).toBe(false);
    expect(tempoChanged(memo, { cps: 0.5 + 9e-7, meter: { num: 4, den: 4 } })).toBe(false);
    expect(tempoChanged(memo, { cps: 0.5 + 2e-6, meter: { num: 4, den: 4 } })).toBe(true);
    expect(tempoChanged(memo, song(100))).toBe(true);
    expect(tempoChanged(memo, { cps: NaN, meter: { num: 4, den: 4 } })).toBe(true);
  });

  test('the disabled title and the notice', () => {
    expect(tempoChangedTitle(memo, song(100))).toBe(
      "the song's tempo changed since this memo (120 · 4/4 → 100 · 4/4)",
    );
    expect(tempoNotice(memo, song(90, 3, 4))).toBe(
      'memo was recorded at 120 · 4/4; the song is now 90 · 3/4',
    );
  });
});

describe('memo rows', () => {
  test('hasTiming', () => {
    expect(hasTiming(entry('a'))).toBe(false);
    expect(hasTiming(entry('a', { sidecar: sc({ bar1Frame: null }) }))).toBe(false);
    expect(hasTiming(entry('a', { sidecar: sc({ bar1Frame: 0 }) }))).toBe(true);
  });

  test('chips: tempo, then recovered or no timing', () => {
    const tempo: MemoChip = {
      id: 'tempo',
      text: '120 · 6/8',
      tone: 'neutral',
      title: 'recorded at 120 bpm, 6/8',
    };
    const noTiming: MemoChip = {
      id: 'no-timing',
      text: 'no timing',
      tone: 'neutral',
      title: 'no bar position saved: it can only play alone',
    };
    expect(memoChips(entry('a', { sidecar: sc() }))).toEqual([tempo]);
    expect(memoChips(entry('a'))).toEqual([noTiming]);
    expect(memoChips(entry('a', { sidecar: sc({ bar1Frame: null }) }))).toEqual([tempo, noTiming]);
    expect(memoChips(entry('a', { sidecar: sc({ bar1Frame: null, recovered: true }) }))).toEqual([
      tempo,
      {
        id: 'recovered',
        text: 'recovered',
        tone: 'warning',
        title: 'this take was cut short and recovered',
      },
    ]);
  });

  test('the row plays alone, stops its own playback, and waits for a take', () => {
    const idle: MemoPlayerState = { status: 'idle' };
    expect(memoRow('a', idle, 'armed')).toEqual({
      active: false,
      disabled: false,
      title: 'play a',
      action: 'play',
    });
    expect(memoRow('a', idle, null)).toMatchObject({ disabled: false });
    for (const status of ['loading', 'playing'] as const) {
      expect(memoRow('a', { status, stem: 'a', how: 'alone' }, 'armed')).toEqual({
        active: true,
        disabled: false,
        title: 'stop a',
        action: 'stop',
      });
      expect(memoRow('b', { status, stem: 'a', how: 'alone' }, 'armed')).toMatchObject({
        active: false,
        action: 'play',
      });
    }
    for (const phase of PHASES) {
      const busy = takeRunning(phase);
      expect(memoRow('a', idle, phase)).toMatchObject({
        disabled: busy,
        title: busy ? BUSY_TITLE : 'play a',
      });
    }
  });

  describe('play items', () => {
    const timed = entry('a', {
      sidecar: { ...sc(), tempo: { ...sc().tempo, bpm: 120, meter: '4/4', cps: 0.5 } },
    });
    const song = { cps: 0.5, meter: { num: 4, den: 4 } };
    const view = (e: RecordingEntry, s: typeof song | null, phase: RecordingPhase | null) =>
      memoPlayItems(e, s, phase).map((i) => [i.how, i.label, i.disabled, i.title]);

    test('all enabled with timing and a matching song', () => {
      expect(view(timed, song, 'armed')).toEqual([
        ['metronome', 'play with metronome', false, undefined],
        ['song', 'play with song', false, undefined],
        ['song+metronome', 'play with song and metronome', false, undefined],
      ]);
    });

    test('no timing disables all three', () => {
      for (const e of [entry('a'), entry('a', { sidecar: sc({ bar1Frame: null }) })]) {
        expect(memoPlayItems(e, song, 'armed').map((i) => [i.disabled, i.title])).toEqual([
          [true, 'no bar position saved'],
          [true, 'no bar position saved'],
          [true, 'no bar position saved'],
        ]);
      }
    });

    test('no song disables the song items', () => {
      expect(memoPlayItems(timed, null, 'armed').map((i) => [i.disabled, i.title])).toEqual([
        [false, undefined],
        [true, 'no song open'],
        [true, 'no song open'],
      ]);
    });

    test('a matching starting meter does not enable mixed-meter song backing', () => {
      const message = 'song backing requires a fixed time signature';
      expect(
        memoPlayItems(timed, { ...song, mixedMeter: true }, 'armed').map((i) => [
          i.disabled,
          i.title,
        ]),
      ).toEqual([
        [false, undefined],
        [true, message],
        [true, message],
      ]);
    });

    test('a changed tempo disables the song items with the tempo title', () => {
      const t = "the song's tempo changed since this memo (120 · 4/4 → 100 · 4/4)";
      expect(
        memoPlayItems(timed, { cps: 100 / 240, meter: { num: 4, den: 4 } }, 'armed').map((i) => [
          i.disabled,
          i.title,
        ]),
      ).toEqual([
        [false, undefined],
        [true, t],
        [true, t],
      ]);
    });

    test('a setcps take is not judged by the static header tempo', () => {
      const override = entry('a', {
        sidecar: {
          ...sc(),
          tempo: { ...sc().tempo, bpm: 144, meter: '4/4', cps: 0.6, tempoOverride: true },
        },
      });
      expect(memoPlayItems(override, song, 'armed').map((i) => i.disabled)).toEqual([
        false,
        false,
        false,
      ]);
    });

    test('a take disables them all', () => {
      for (const phase of PHASES) {
        const busy = takeRunning(phase);
        for (const i of memoPlayItems(timed, song, phase)) {
          expect([i.disabled, i.title]).toEqual(busy ? [true, BUSY_TITLE] : [false, undefined]);
        }
      }
    });
  });
});

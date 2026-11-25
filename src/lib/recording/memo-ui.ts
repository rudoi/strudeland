/**
 * The record controls' pure rules: the record button's look per phase, the backing toggles and
 * their defaults, the device picker's label, the elapsed-time format and the status-bar notices.
 * Components only render what these return. No Tauri, DOM or audio imports.
 */
import { writable, type Writable } from 'svelte/store';
import type { MemoPlayerState, PlayHow } from '../audio/memo-player';
import type { Tone } from '../theme/tones';
import { bpmFromCps, meterString, type Meter, type SongStructure } from '../strudel/structure';
import type {
  BackingChoice,
  InputDevice,
  RecordingEntry,
  RecordingPhase,
  RecordingUiState,
  SidecarTempo,
} from './types';

/** System Settings, Privacy & Security, Microphone. */
export const MIC_SETTINGS_URL =
  'x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone';

/**
 * A take is under way: the backing is fixed, the toolbar's play is off and its stop ends the take.
 * `null` (no controller) is not.
 */
export function takeRunning(phase: RecordingPhase | null): boolean {
  return (
    phase === 'starting' || phase === 'count-in' || phase === 'recording' || phase === 'stopping'
  );
}

/** A memo is audible: loading or playing. */
export function memoAudible(
  p: MemoPlayerState,
): p is Extract<MemoPlayerState, { status: 'loading' | 'playing' }> {
  return p.status === 'loading' || p.status === 'playing';
}

/** Milliseconds to `m:ss`, or `h:mm:ss` from one hour. Negative or non-finite input is 0:00. */
export function formatElapsed(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.floor(ms / 1000) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

// ---- the record button ----

export interface RecordButton {
  /** Which glyph leads the label: the record dot, the stop square, or none. */
  glyph: 'dot' | 'square' | null;
  /** The plain part of the label. */
  text: string;
  /** The mono part of the label (the count-in beat, the elapsed time). */
  mono: string | null;
  /** The label without the glyph. */
  ariaLabel: string;
  tone: Tone;
  variant: 'default' | 'primary';
  disabled: boolean;
  help: string;
  /** What a click does. */
  action: 'record' | 'stop' | null;
}

const DESKTOP_RECORDING_HELP = 'Microphone recording requires the desktop app.';
const STOP_RECORDING_HELP = 'End the take and save the memo in this project.';

const button = (p: Partial<RecordButton> & Pick<RecordButton, 'help'>): RecordButton => {
  const text = p.text ?? 'record';
  const mono = p.mono ?? null;
  return {
    glyph: 'dot',
    tone: 'neutral',
    variant: 'default',
    disabled: true,
    action: null,
    ...p,
    text,
    mono,
    ariaLabel: [text, mono].filter((x) => x).join(' '),
  };
};

/** Why record is off in `idle`, from the error that put it there. */
function idleHelp(state: RecordingUiState): string {
  switch (state.error?.kind) {
    case 'permissionDenied':
      return 'Microphone access denied.';
    case 'noDevice':
      return 'No input device found.';
    case 'deviceLost':
      return 'Input device lost.';
    default:
      return 'The microphone is not ready.';
  }
}

/** The record button for a state; `state` null is no controller (outside the app). */
export function recordButton(
  state: RecordingUiState | null,
  hasProject: boolean,
  elapsedMs = 0,
): RecordButton {
  if (state === null) return button({ help: DESKTOP_RECORDING_HELP });
  if (!hasProject) return button({ help: 'Open a project to record.' });
  switch (state.phase) {
    case 'idle':
      return button({ help: idleHelp(state) });
    case 'arming':
      return button({ help: 'Waiting for microphone access.' });
    case 'armed':
      return button({
        tone: 'danger',
        disabled: false,
        help: 'Start a microphone take in this project with your selected backing.',
        action: 'record',
      });
    case 'starting':
      return button({ tone: 'danger', help: 'Starting the recording.' });
    case 'count-in':
      return button({
        glyph: null,
        text: 'count-in',
        mono: state.countInBeat === null ? null : String(state.countInBeat),
        tone: 'danger',
        disabled: false,
        help: STOP_RECORDING_HELP,
        action: 'stop',
      });
    case 'recording':
      return button({
        glyph: 'square',
        text: '',
        mono: formatElapsed(elapsedMs),
        tone: 'danger',
        variant: 'primary',
        disabled: false,
        help: STOP_RECORDING_HELP,
        action: 'stop',
      });
    case 'stopping':
      return button({
        glyph: null,
        text: 'saving…',
        tone: 'danger',
        variant: 'primary',
        help: 'Saving the memo.',
      });
  }
}

// ---- the backing toggles ----

/** The remembered toggles; `songTouched` is set by the first click on `song`. */
export interface ToggleChoice {
  song: boolean;
  metronome: boolean;
  countIn: boolean;
  songTouched: boolean;
}

export const DEFAULT_CHOICE: ToggleChoice = {
  song: true,
  metronome: false,
  countIn: true,
  songTouched: false,
};

/** A fresh toggle store with the defaults. */
export const createBackingChoice = (): Writable<ToggleChoice> => writable({ ...DEFAULT_CHOICE });

/** The window's toggles: per window (one module per webview), never persisted. */
export const backingChoice = createBackingChoice();

/** `song` as remembered: whether a song is open until the user clicks it once. */
const songWanted = (choice: ToggleChoice, hasSong: boolean): boolean =>
  choice.songTouched ? choice.song : hasSong;

/** What `record` is given. */
export function effectiveBacking(choice: ToggleChoice, hasSong: boolean): BackingChoice {
  const song = songWanted(choice, hasSong) && hasSong;
  const metronome = choice.metronome;
  return { song, metronome, countIn: choice.countIn && (song || metronome) };
}

export type ToggleKey = 'song' | 'metronome' | 'countIn';

/** The choice after a click on `key`. */
export function toggle(choice: ToggleChoice, key: ToggleKey, hasSong: boolean): ToggleChoice {
  if (key === 'song') {
    return { ...choice, song: !songWanted(choice, hasSong), songTouched: true };
  }
  return { ...choice, [key]: !choice[key] };
}

export interface ToggleView {
  key: ToggleKey;
  label: string;
  pressed: boolean;
  disabled: boolean;
  help: string;
}

/** The three toggles' views, in toolbar order. */
export function toggleViews(
  choice: ToggleChoice,
  phase: RecordingPhase | null,
  hasProject: boolean,
  hasSong: boolean,
): ToggleView[] {
  const eff = effectiveBacking(choice, hasSong);
  const blocker =
    phase === null
      ? DESKTOP_RECORDING_HELP
      : !hasProject
        ? 'Open a project to choose recording backing.'
        : takeRunning(phase)
          ? 'Finish the current take before changing its backing.'
          : undefined;
  const off = blocker !== undefined;
  const hasBacking = eff.song || eff.metronome;
  return [
    {
      key: 'song',
      label: 'song',
      pressed: eff.song,
      disabled: off || !hasSong,
      help:
        blocker ??
        (hasSong
          ? 'Hear the open song while recording.'
          : 'Open a song to hear it while recording.'),
    },
    {
      key: 'metronome',
      label: 'metronome',
      pressed: eff.metronome,
      disabled: off,
      help: blocker ?? 'Hear a click while recording. Use headphones to keep it out of the take.',
    },
    {
      key: 'countIn',
      label: 'count-in',
      // Forced off shows the remembered value, dimmed by `disabled`.
      pressed: choice.countIn,
      disabled: off || !hasBacking,
      help:
        blocker ??
        (hasBacking
          ? 'Hear one bar of clicks before the first downbeat.'
          : 'Enable song or metronome backing to use a count-in.'),
    },
  ];
}

// ---- the device picker ----

/** The selected input's device; undefined without a selection or when it is not listed. */
function selectedDevice(state: RecordingUiState | null): InputDevice | undefined {
  const input = state?.input;
  return input ? state.devices.find((d) => d.id === input.deviceId) : undefined;
}

/** The picker button's `aria-label` and `title`. */
export function inputLabel(state: RecordingUiState | null): string {
  const input = state?.input;
  const device = selectedDevice(state);
  if (!input || !device) return 'no input device';
  return device.channels > 1
    ? `input: ${device.name}, channel ${input.channel + 1}`
    : `input: ${device.name}`;
}

/** The picker opens only with a project and between takes. */
export const pickerDisabled = (state: RecordingUiState | null, hasProject: boolean): boolean =>
  !hasProject || state === null || takeRunning(state.phase);

/** The channel indexes (0-based) to offer: none for a mono or unknown device. */
export function channelChoices(state: RecordingUiState | null): number[] {
  const device = selectedDevice(state);
  if (!device || device.channels <= 1) return [];
  return Array.from({ length: device.channels }, (_, i) => i);
}

// ---- status-bar notices ----

export type NoticeAction =
  | { kind: 'link'; label: string; href: string }
  /** Calls `recording.clearError()`. */
  | { kind: 'dismiss' }
  /** Calls `memos.stopPlayback()` (which clears a player error). */
  | { kind: 'dismiss-playback' };

export interface RecordingNotice {
  id: string;
  tone: 'warning' | 'danger';
  text: string;
  action?: NoticeAction;
}

/** The recording and memo-playback notices for the footer, in order, one per id. */
export function recordingNotices(
  state: RecordingUiState | null,
  player: MemoPlayerState | null = null,
  platform = globalThis.navigator?.platform ?? '',
): RecordingNotice[] {
  const out = state === null ? [] : takeNotices(state, platform);
  if (player?.status === 'error') {
    // Playback has stopped by now, so every one is dismissable.
    const { kind } = player;
    out.push({
      id: kind === 'tooLong' ? 'play-too-long' : `play-${kind}`,
      tone: kind === 'tooLong' || kind === 'tempo' ? 'warning' : 'danger',
      text: player.message,
      action: { kind: 'dismiss-playback' },
    });
  }
  return out;
}

function takeNotices(state: RecordingUiState, platform: string): RecordingNotice[] {
  const out: RecordingNotice[] = [];
  const e = state.error;
  if (e) {
    switch (e.kind) {
      case 'permissionDenied': {
        const href = platform.includes('Mac')
          ? MIC_SETTINGS_URL
          : platform.startsWith('Win')
            ? 'ms-settings:privacy-microphone'
            : null;
        out.push({
          id: 'permission',
          tone: 'danger',
          text: href ? e.message : e.message + ' · check the input device in sound settings',
          ...(href ? { action: { kind: 'link' as const, label: 'open settings', href } } : {}),
        });
        break;
      }
      case 'noDevice':
        out.push({ id: 'no-device', tone: 'warning', text: 'no input device found' });
        break;
      case 'deviceLost':
        out.push({
          id: 'device-lost',
          tone: 'warning',
          text: 'input device lost; any recording so far was saved',
        });
        break;
      case 'limitReached':
        out.push({
          id: 'limit-reached',
          tone: 'warning',
          text: 'recording stopped at the length limit; the memo was saved',
          action: { kind: 'dismiss' },
        });
        break;
      // Rename and delete show these inline, on their rows.
      case 'invalidName':
      case 'exists':
      case 'notFound':
        break;
      default:
        out.push({
          id: 'recording-error',
          tone: 'danger',
          text: `recording: ${e.message}`,
          action: { kind: 'dismiss' },
        });
    }
  }
  if (state.warning) out.push({ id: 'recording-warning', tone: 'warning', text: state.warning });
  if (
    selectedDevice(state)?.bluetooth &&
    (state.phase === 'armed' || state.phase === 'count-in' || state.phase === 'recording')
  ) {
    out.push({
      id: 'bluetooth',
      tone: 'warning',
      text: 'bluetooth input: timing and quality suffer. use wired headphones',
    });
  }
  return out;
}

// ---- the memo list ----

/**
 * Seconds to `m:ss`, or `h:mm:ss` from one hour, rounded up to the whole second (a 0.3 s memo is
 * 0:01); null (an unreadable header) is the empty string.
 */
export function formatDuration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) {
    return seconds === null ? '' : formatElapsed(0);
  }
  // Tolerate float noise: 7.0000001 s is 0:07.
  return formatElapsed(Math.ceil(seconds - 1e-3) * 1000);
}

/** When a memo was made: the sidecar's `createdAt`, else the WAV's modification time. */
function madeAt(e: RecordingEntry): number {
  const created = e.sidecar ? Date.parse(e.sidecar.createdAt) : NaN;
  return Number.isFinite(created) ? created : e.modifiedMs;
}

const byBytes = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** Newest first; ties by stem. A new array. */
export function sortMemos(items: readonly RecordingEntry[]): RecordingEntry[] {
  return [...items].sort((a, b) => madeAt(b) - madeAt(a) || byBytes(a.stem, b.stem));
}

/** The open song's tempo, from its static header. */
export interface SongTempo {
  cps: number;
  meter: Meter;
  mixedMeter?: boolean;
}

export function songTempoOf(s: SongStructure): SongTempo {
  return { cps: s.cps, meter: s.meter, ...(s.meterChanges && { mixedMeter: true }) };
}

/** `120 · 4/4`: bpm rounded to an integer, the meter as given. */
export const tempoText = (bpm: number, meter: string): string => `${Math.round(bpm)} · ${meter}`;

const memoTempoText = (t: SidecarTempo): string => tempoText(t.bpm, t.meter);
const songTempoText = (s: SongTempo): string =>
  tempoText(bpmFromCps(s.cps, s.meter), meterString(s.meter));

/** A cps difference larger than this is a different tempo. */
const TEMPO_MATCH_EPSILON = 1e-6;

/** The song plays at its own tempo, so a memo plays with it only when the tempos match. */
export const tempoChanged = (memo: SidecarTempo, song: SongTempo): boolean =>
  !(Math.abs(song.cps - memo.cps) <= TEMPO_MATCH_EPSILON);

/** The status-bar text when the song turned out to be at another tempo. */
export const tempoNotice = (memo: SidecarTempo, song: SongTempo): string =>
  `memo was recorded at ${memoTempoText(memo)}; the song is now ${songTempoText(song)}`;

/** Whether a memo knows where its bar 1 is (it can play in time). */
export const hasTiming = (e: Pick<RecordingEntry, 'sidecar'>): boolean =>
  e.sidecar !== null && e.sidecar.timing.bar1Frame !== null;

export interface MemoChip {
  id: 'tempo' | 'recovered' | 'no-timing';
  text: string;
  tone: Tone;
  title: string;
}

/** The chips under a memo row, in order. */
export function memoChips(e: RecordingEntry): MemoChip[] {
  const out: MemoChip[] = [];
  const s = e.sidecar;
  if (s) {
    out.push({
      id: 'tempo',
      text: memoTempoText(s.tempo),
      tone: 'neutral',
      title: `recorded at ${Math.round(s.tempo.bpm)} bpm, ${s.tempo.meter}`,
    });
  }
  if (s?.recovered) {
    out.push({
      id: 'recovered',
      text: 'recovered',
      tone: 'warning',
      title: 'this take was cut short and recovered',
    });
  } else if (!hasTiming(e)) {
    out.push({
      id: 'no-timing',
      text: 'no timing',
      tone: 'neutral',
      title: 'no bar position saved: it can only play alone',
    });
  }
  return out;
}

/** Playback is off while a take runs. */
const BUSY_PLAYBACK_HELP = 'Finish recording before playing a memo.';
const NO_TIMING_HELP = 'This memo has no saved bar position.';

export interface MemoRowView {
  /** Loading or playing. */
  active: boolean;
  disabled: boolean;
  help: string;
  /** What a click does. */
  action: 'play' | 'stop';
}

/** The row button (play alone, or stop when this memo plays or loads). */
export function memoRow(
  stem: string,
  player: MemoPlayerState,
  phase: RecordingPhase | null,
): MemoRowView {
  const active = memoAudible(player) && player.stem === stem;
  if (active) {
    return {
      active,
      disabled: false,
      help: 'Stop this memo and any backing it started.',
      action: 'stop',
    };
  }
  const busy = takeRunning(phase);
  return {
    active,
    disabled: busy,
    help: busy
      ? BUSY_PLAYBACK_HELP
      : 'Play this recording on its own, stopping any song already playing.',
    action: 'play',
  };
}

export interface PlayItemView {
  how: Exclude<PlayHow, 'alone'>;
  label: string;
  disabled: boolean;
  help: string;
}

/** The row menu's play items, in order. `song` is null without an open song. */
export function memoPlayItems(
  e: RecordingEntry,
  song: SongTempo | null,
  phase: RecordingPhase | null,
): PlayItemView[] {
  const metronomeBlocker = takeRunning(phase)
    ? BUSY_PLAYBACK_HELP
    : hasTiming(e)
      ? undefined
      : NO_TIMING_HELP;
  function songWhy(): string | undefined {
    if (metronomeBlocker) return metronomeBlocker;
    if (song === null) return 'Open a song to play it with this memo.';
    if (song.mixedMeter) return 'Song backing requires a fixed time signature.';
    // A `setcps` take's tempo is the evaluated one; playing checks it after the evaluate.
    const memo = e.sidecar!.tempo;
    if (!memo.tempoOverride && tempoChanged(memo, song)) {
      return `The song’s tempo changed since this memo (${memoTempoText(memo)} → ${songTempoText(song)}).`;
    }
    return undefined;
  }
  const songBlocker = songWhy();
  return [
    {
      how: 'metronome',
      label: 'play with metronome',
      disabled: metronomeBlocker !== undefined,
      help:
        metronomeBlocker ??
        'Play the memo against clicks at the recorded tempo and time signature.',
    },
    {
      how: 'song',
      label: 'play with song',
      disabled: songBlocker !== undefined,
      help: songBlocker ?? 'Align the memo’s saved downbeat to the open song.',
    },
    {
      how: 'song+metronome',
      label: 'play with song and metronome',
      disabled: songBlocker !== undefined,
      help: songBlocker ?? 'Align the memo’s saved downbeat to the open song and a click track.',
    },
  ];
}

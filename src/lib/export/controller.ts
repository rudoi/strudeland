import { writable } from 'svelte/store';
import { errorMessage } from '../errors';
import { ExportError } from './error';
import type { SessionState } from '../project/session';
import { authoredLoop, type SongStructure } from '../strudel/structure';
import type {
  ExportApi,
  ExportFormat,
  ExportPreset,
  ExportProgress,
  ExportRequest,
  ExportResult,
  ExportRole,
  ExportPreview,
  StemGrouping,
} from './api';

export const MODES = [
  {
    id: 'standard',
    label: 'standard render',
    detail: 'export the selected range as a regular audio file',
  },
  {
    id: 'stems',
    label: 'stems',
    detail: 'a pre-master reference mix and aligned stereo stems for your DAW',
  },
  {
    id: 'intro-loop',
    label: 'intro + loop',
    detail: 'play the opening and first pass once, then repeat the second file from its start',
  },
  {
    id: 'tagged-loop',
    label: 'tagged loop',
    detail: 'one file with any intro, a first pass, and a marked repeat',
  },
  {
    id: 'folded-loop',
    label: 'folded loop',
    detail:
      'one file with a clean first pass and a marked repeat that carries its own effects across the seam',
  },
  {
    id: 'one-shot',
    label: 'one-shot',
    detail: 'export the selected range and let its notes and effects decay',
  },
] as const;

type Mode = (typeof MODES)[number]['id'];
type Range = ExportRequest['length']['kind'];

export interface ExportSong {
  root: string;
  songPath: string;
  title: string;
  source: string;
  dirty: boolean;
  structure: SongStructure | null;
}

export function captureExportSong(state: SessionState, source: string | null): ExportSong | null {
  if (!state.available || !state.project || source === null) return null;
  const song = state.songs.find((song) => song.path === state.active);
  return song
    ? {
        root: state.project.root,
        songPath: song.path,
        title: song.title,
        source,
        dirty: song.dirty,
        structure: state.structure,
      }
    : null;
}

interface Options {
  mode: Mode;
  grouping: StemGrouping;
  range: Range;
  section: string;
  bars: string;
  format: ExportFormat;
  folder: string | null;
}

type Preview =
  | { kind: 'idle' | 'loading' }
  | { kind: 'ready'; target: ExportPreview; request: ExportRequest }
  | { kind: 'error'; message: string };

type Activity =
  | { kind: 'idle' | 'picking' | 'confirming' | 'planning' | 'encoding' | 'writing' }
  | { kind: 'rendering'; completed: number; total: number };

interface Failure {
  message: string;
  written: string[];
}

interface DialogState {
  options: Options;
  preview: Preview;
  activity: Activity;
  failure: Failure | null;
  result: ExportResult | null;
  cancelled: boolean;
  quit: 'ask' | 'waiting' | 'result' | 'settling' | null;
}

export const loopMode = (mode: Mode): mode is Exclude<ExportPreset, 'one-shot'> =>
  mode === 'tagged-loop' || mode === 'intro-loop' || mode === 'folded-loop';

export const showMp3Warning = ({ mode, format }: Options): boolean =>
  loopMode(mode) && format === 'mp3';

/** A loop preset with no loop to export: the song's header, as typed, marks neither a
 *  `meta.loop` range nor a section with `loop: true`. The planner reads the evaluated song,
 *  so a loop that only `song()` registers is not seen here (the dialog words its ranges
 *  from the header too). */
export const loopUnavailable = (
  song: Pick<ExportSong, 'structure'>,
  { mode }: Pick<Options, 'mode'>,
): boolean => loopMode(mode) && (song.structure === null || authoredLoop(song.structure) === null);

export const roleLabel = (role: ExportRole): string =>
  ({ intro: 'opening', loop: 'loop', single: 'audio' })[role];

function requestFor(
  song: ExportSong,
  { mode, grouping, range, section, bars, format, folder }: Options,
): ExportRequest | null {
  const args: Omit<ExportRequest, 'length'> = {
    root: song.root,
    songPath: song.songPath,
    folder,
    format: mode === 'stems' ? 'wav' : format,
    preset: mode === 'standard' || mode === 'stems' ? null : mode,
    ...(mode === 'stems' && { stems: grouping }),
  };
  if (loopMode(mode) || range === 'clip') return { ...args, length: { kind: 'clip' } };
  if (range === 'section')
    return section ? { ...args, length: { kind: 'section', name: section } } : null;
  const count = Number(bars);
  return /^\d+$/.test(bars) && Number.isSafeInteger(count) && count > 0
    ? { ...args, length: { kind: 'length', bars: count } }
    : null;
}

function failureFrom(error: unknown): Failure {
  return {
    message: errorMessage(error),
    written: error instanceof ExportError ? error.written : [],
  };
}

export function createExportDialog(song: ExportSong, api: ExportApi) {
  const firstSection = song.structure?.sections[0];
  let state: DialogState = {
    options: {
      mode: 'standard',
      grouping: 'orbit',
      range: firstSection ? 'clip' : 'length',
      section: firstSection?.name ?? '',
      bars: '',
      format: 'wav',
      folder: null,
    },
    preview: { kind: 'idle' },
    activity: { kind: 'idle' },
    result: null,
    failure: null,
    cancelled: false,
    quit: null,
  };
  const store = writable(state);
  let revision = 0;
  let disposed = false;
  let hasUnacknowledgedOutcome = false;
  let resolveQuit: ((approved: boolean) => void) | undefined;

  function update(change: Partial<DialogState>) {
    if (disposed) return;
    state = { ...state, ...change };
    store.set(state);
  }

  async function refresh() {
    if (disposed || state.activity.kind !== 'idle') return;
    const version = ++revision;
    const request = requestFor(song, state.options);
    if (!request) {
      update({ preview: { kind: 'idle' } });
      return;
    }
    update({ preview: { kind: 'loading' } });
    try {
      const target = await api.preview(request);
      if (version === revision) update({ preview: { kind: 'ready', target, request } });
    } catch (error) {
      if (version === revision)
        update({ preview: { kind: 'error', message: failureFrom(error).message } });
    }
  }

  const canClose = () => !state.quit && state.activity.kind === 'idle';

  function select(options: Partial<Options>) {
    if (disposed || !canClose()) return;
    hasUnacknowledgedOutcome = false;
    update({
      options: { ...state.options, ...options },
      preview: { kind: 'idle' },
      result: null,
      failure: null,
      cancelled: false,
    });
    void refresh();
  }

  async function pickFolder() {
    if (disposed || !canClose()) return;
    update({ activity: { kind: 'picking' } });
    try {
      const folder = await api.pickFolder();
      update({ activity: { kind: 'idle' } });
      if (folder !== null) select({ folder });
    } catch (error) {
      update({ activity: { kind: 'idle' }, failure: failureFrom(error) });
    }
  }

  function onProgress(progress: ExportProgress) {
    switch (progress.kind) {
      case 'confirming':
      case 'planning':
        update({ activity: { kind: progress.kind } });
        break;
      case 'planned':
        update({
          activity:
            progress.chunks > 0
              ? { kind: 'rendering', completed: 0, total: progress.chunks }
              : { kind: 'encoding' },
        });
        break;
      case 'chunkRendered': {
        const completed = state.activity.kind === 'rendering' ? state.activity.completed + 1 : 1;
        update({
          activity:
            completed < progress.of
              ? { kind: 'rendering', completed, total: progress.of }
              : { kind: 'encoding' },
        });
        break;
      }
      case 'encoded':
        // Stem WAVs are published one at a time. The manifest is still being written.
        if (state.activity.kind !== 'writing') update({ activity: { kind: 'encoding' } });
        break;
      case 'writing':
      case 'written':
        update({ activity: { kind: 'writing' } });
        break;
    }
  }

  async function start() {
    if (
      disposed ||
      !canClose() ||
      state.preview.kind !== 'ready' ||
      state.result ||
      loopUnavailable(song, state.options)
    )
      return;
    const { target, request } = state.preview;
    let receivingProgress = true;
    hasUnacknowledgedOutcome = false;
    try {
      update({ activity: { kind: 'planning' }, failure: null, cancelled: false });
      const result = await api.start(request, song.source, target, (progress) => {
        if (receivingProgress) onProgress(progress);
      });
      hasUnacknowledgedOutcome = true;
      update({
        activity: { kind: 'idle' },
        result: result && { ...result, warnings: [...new Set(result.warnings)] },
        cancelled: result === null,
        quit: outcomeDecision(),
      });
    } catch (error) {
      hasUnacknowledgedOutcome = true;
      update({
        activity: { kind: 'idle' },
        preview: { kind: 'idle' },
        failure: failureFrom(error),
        quit: outcomeDecision(),
      });
      void refresh();
    } finally {
      receivingProgress = false;
    }
  }

  function outcomeDecision(): DialogState['quit'] {
    return state.quit === 'ask' || state.quit === 'waiting' ? 'result' : state.quit;
  }

  function requestQuit(): Promise<boolean> {
    if (disposed || state.quit || state.activity.kind === 'picking') return Promise.resolve(false);
    const idle = state.activity.kind === 'idle';
    if (idle && !hasUnacknowledgedOutcome) {
      update({ quit: 'settling' });
      return Promise.resolve(true);
    }
    // IPC can deliver the export result before the Quit event, even if Quit arrived first.
    return new Promise<boolean>((resolve) => {
      resolveQuit = resolve;
      update({ quit: idle ? 'result' : 'ask' });
    });
  }

  function answerQuit(choice: 'wait' | 'cancel' | 'continue') {
    if (choice === 'wait') {
      if (state.quit === 'ask') update({ quit: 'waiting' });
      return;
    }
    if (!resolveQuit || (choice === 'continue' && state.quit !== 'result')) return;
    if (state.quit === 'result') hasUnacknowledgedOutcome = false;
    update({ quit: 'settling' });
    const resolve = resolveQuit;
    resolveQuit = undefined;
    resolve(choice === 'continue');
  }

  return {
    store: { subscribe: store.subscribe },
    refresh,
    select,
    pickFolder,
    start,
    canClose,
    requestQuit,
    answerQuit,
    finishQuit: () => update({ quit: null }),
    dispose: () => {
      disposed = true;
      resolveQuit?.(false);
      resolveQuit = undefined;
    },
  };
}

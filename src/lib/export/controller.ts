import { writable } from 'svelte/store';
import { kindedError } from '../command-error';
import { errorMessage } from '../errors';
import type { SessionState } from '../project/session';
import type { SongStructure } from '../strudel/structure';
import type {
  ExportApi,
  ExportFormat,
  ExportPreset,
  ExportProgress,
  ExportRequest,
  ExportResult,
  ExportRole,
  ExportTarget,
} from './api';

export const MODES = [
  {
    id: 'standard',
    label: 'standard render',
    detail: 'export the selected range as a regular audio file',
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
  range: Range;
  section: string;
  bars: string;
  format: ExportFormat;
  folder: string | null;
}

type Preview =
  | { kind: 'idle' | 'loading' }
  | { kind: 'ready'; files: ExportTarget[]; request: ExportRequest }
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
}

export const loopMode = (mode: Mode): mode is Exclude<ExportPreset, 'one-shot'> =>
  mode !== 'standard' && mode !== 'one-shot';

export const showMp3Warning = ({ mode, format }: Options): boolean =>
  loopMode(mode) && format === 'mp3';

export const roleLabel = (role: ExportRole): string =>
  ({ intro: 'opening', loop: 'loop', single: 'audio' })[role];

function requestFor(
  song: ExportSong,
  { mode, range, section, bars, format, folder }: Options,
): ExportRequest | null {
  const args = {
    root: song.root,
    songPath: song.songPath,
    folder,
    format,
    preset: mode === 'standard' ? null : mode,
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
  const written = (error as { written?: unknown } | null)?.written;
  return {
    message: kindedError(error)?.message ?? errorMessage(error),
    written: Array.isArray(written)
      ? written.filter((path): path is string => typeof path === 'string')
      : [],
  };
}

export function createExportDialog(song: ExportSong, api: ExportApi) {
  const firstSection = song.structure?.sections[0];
  let state: DialogState = {
    options: {
      mode: 'standard',
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
  };
  const store = writable(state);
  let revision = 0;
  let disposed = false;

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
      const files = await api.preview(request);
      if (version === revision) update({ preview: { kind: 'ready', files, request } });
    } catch (error) {
      if (version === revision)
        update({ preview: { kind: 'error', message: failureFrom(error).message } });
    }
  }

  function select(options: Partial<Options>) {
    if (disposed || state.activity.kind !== 'idle') return;
    update({
      options: { ...state.options, ...options },
      preview: { kind: 'idle' },
      result: null,
      failure: null,
    });
    void refresh();
  }

  async function pickFolder() {
    if (disposed || state.activity.kind !== 'idle') return;
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
        update({ activity: { kind: 'encoding' } });
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
      state.activity.kind !== 'idle' ||
      state.preview.kind !== 'ready' ||
      state.result
    )
      return;
    const { files, request } = state.preview;
    const collisions = files.filter((file) => file.exists).map((file) => file.path);
    let receivingProgress = true;
    try {
      if (collisions.length > 0) {
        update({ activity: { kind: 'confirming' } });
        const approved = await api.confirmReplacement(collisions);
        if (disposed || !approved) {
          update({ activity: { kind: 'idle' } });
          return;
        }
      }
      update({ activity: { kind: 'planning' }, failure: null });
      const result = await api.start(request, song.source, files, (progress) => {
        if (receivingProgress) onProgress(progress);
      });
      update({
        activity: { kind: 'idle' },
        result: { ...result, warnings: [...new Set(result.warnings)] },
      });
    } catch (error) {
      update({
        activity: { kind: 'idle' },
        preview: { kind: 'idle' },
        failure: failureFrom(error),
      });
      void refresh();
    } finally {
      receivingProgress = false;
    }
  }

  return {
    store: { subscribe: store.subscribe },
    refresh,
    select,
    pickFolder,
    start,
    canClose: () => state.activity.kind === 'idle',
    dispose: () => {
      disposed = true;
    },
  };
}

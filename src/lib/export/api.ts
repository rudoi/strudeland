import { Channel, invoke } from '@tauri-apps/api/core';
import { message } from '@tauri-apps/plugin-dialog';

export type ExportFormat = 'wav' | 'mp3' | 'ogg' | 'opus';
export type ExportPreset = 'intro-loop' | 'tagged-loop' | 'folded-loop' | 'one-shot';
export type ExportRole = 'single' | 'intro' | 'loop';
export type ExportLength =
  { kind: 'clip' } | { kind: 'section'; name: string } | { kind: 'length'; bars: number };

export interface ExportRequest {
  root: string;
  songPath: string;
  folder: string | null;
  format: ExportFormat;
  preset: ExportPreset | null;
  length: ExportLength;
}

export interface ExportTarget {
  role: ExportRole;
  path: string;
  exists: boolean;
}

export interface ExportResult {
  files: { role: ExportRole; path: string; godotLoopOffset: number | null }[];
  warnings: string[];
}

export type ExportProgress =
  | { kind: 'planned'; chunks: number }
  | { kind: 'chunkRendered'; index: number; of: number }
  | { kind: 'encoded'; pass: number }
  | { kind: 'writing' }
  | { kind: 'written'; path: string };

export interface ExportApi {
  pickFolder(): Promise<string | null>;
  preview(request: ExportRequest): Promise<ExportTarget[]>;
  confirmReplacement(paths: string[]): Promise<boolean>;
  start(
    request: ExportRequest,
    source: string,
    preview: ExportTarget[],
    onProgress: (progress: ExportProgress) => void,
  ): Promise<ExportResult>;
}

export const exportApi: ExportApi = {
  pickFolder: () => invoke('export_audio_pick_folder'),
  preview: (request) => invoke('export_audio_preview', { request }),
  async confirmReplacement(paths) {
    const replace = 'Replace';
    return (
      (await message(`Replace these files?\n\n${paths.join('\n')}`, {
        title: 'Export audio',
        kind: 'warning',
        buttons: { ok: replace, cancel: 'Cancel' },
      })) === replace
    );
  },
  start(request, source, preview, onProgress) {
    const channel = new Channel<ExportProgress>(onProgress);
    return invoke('export_audio_start', { request, source, preview, onProgress: channel });
  },
};

export function setExportMenuAvailable(available: boolean): Promise<void> {
  return invoke('export_audio_set_available', { available });
}

import { Channel, invoke } from '@tauri-apps/api/core';

export type ExportFormat = 'wav' | 'mp3' | 'ogg' | 'opus';
export type ExportPreset = 'intro-loop' | 'tagged-loop' | 'folded-loop' | 'one-shot';
export type StemGrouping = 'orbit' | 'voice' | 'instrument';
export type ExportRole = 'single' | 'intro' | 'loop';
export type ExportLength =
  { kind: 'clip' } | { kind: 'section'; name: string } | { kind: 'length'; bars: number };

export interface ExportRequest {
  root: string;
  songPath: string;
  folder: string | null;
  format: ExportFormat;
  preset: ExportPreset | null;
  stems?: StemGrouping;
  length: ExportLength;
}

export interface ExportTarget {
  role: ExportRole;
  path: string;
  exists: boolean;
}

export type ExportPreview = ExportTarget[] | { directory: string };

export type ExportResult =
  | {
      files: { role: ExportRole; path: string; godotLoopOffset: number | null }[];
      warnings: string[];
    }
  | { directory: string; paths: string[]; warnings: string[] };

export type ExportProgress =
  | { kind: 'confirming' }
  | { kind: 'planning' }
  | { kind: 'planned'; chunks: number }
  | { kind: 'chunkRendered'; index: number; of: number }
  | { kind: 'encoded'; pass: number }
  | { kind: 'writing' }
  | { kind: 'written'; path: string };

export interface ExportApi {
  pickFolder(): Promise<string | null>;
  preview(request: ExportRequest): Promise<ExportPreview>;
  start(
    request: ExportRequest,
    source: string,
    preview: ExportPreview,
    onProgress: (progress: ExportProgress) => void,
  ): Promise<ExportResult | null>;
}

export const exportApi: ExportApi = {
  pickFolder: () => invoke('export_audio_pick_folder'),
  preview: (request) => invoke('export_audio_preview', { request }),
  start(request, source, preview, onProgress) {
    const channel = new Channel<ExportProgress>(onProgress);
    return invoke('export_audio_start', { request, source, preview, onProgress: channel });
  },
};

export function setExportMenuAvailable(available: boolean): Promise<void> {
  return invoke('export_audio_set_available', { available });
}

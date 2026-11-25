import { invoke } from '@tauri-apps/api/core';
import type { InspectionRequest, InspectionResult } from './types';

export const inspectPianoRoll = (request: InspectionRequest) =>
  invoke<InspectionResult>('piano_roll_inspect', { request });

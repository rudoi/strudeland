import { invokeCommand } from '../ipc/invoke';
import type { InspectionRequest, InspectionResult } from './types';

export const inspectPianoRoll = (request: InspectionRequest) =>
  invokeCommand('piano_roll_inspect', { request }) as Promise<InspectionResult>;

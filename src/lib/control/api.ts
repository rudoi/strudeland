import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ControlInfo, ControlReply, ControlRequest } from './protocol';

// The Tauri seam of the control socket. `app.ts` only sees the `ControlApi` type, so tests fake it
// (`tests/helpers/fake-control-api.ts`).

export interface ControlApi {
  /** `invoke('control_attach')`; call after `onRequest`. */
  attach(): Promise<ControlInfo>;
  /** `listen('control://request')`; returns an unlisten function. */
  onRequest(cb: (req: ControlRequest) => void): Promise<() => void>;
  /** `invoke('control_reply', { id, reply })`. */
  reply(id: number, reply: ControlReply): Promise<void>;
}

export const CONTROL_REQUEST_EVENT = 'control://request';

export function createTauriControlApi(): ControlApi {
  return {
    attach: () => invoke<ControlInfo>('control_attach'),
    onRequest: (cb) => listen<ControlRequest>(CONTROL_REQUEST_EVENT, (event) => cb(event.payload)),
    reply: (id, reply) => invoke<void>('control_reply', { id, reply }),
  };
}

import { listen } from '@tauri-apps/api/event';
import { EVENTS } from '../ipc/bindings';
import { invokeCommand } from '../ipc/invoke';
import type { ControlInfo, ControlReply, ControlRequest } from './protocol';

// The Tauri seam of the control socket. `app.ts` only sees the `ControlApi` type, so tests fake it
// (`tests/helpers/fake-control-api.ts`).

export interface ControlApi {
  /** `invoke('control_attach')`; call after `onRequest`. */
  attach(): Promise<ControlInfo>;
  /** `listen(EVENTS.controlRequest)`; returns an unlisten function. */
  onRequest(cb: (req: ControlRequest) => void): Promise<() => void>;
  /** `invoke('control_reply', { id, reply })`. */
  reply(id: number, reply: ControlReply): Promise<void>;
}

export function createTauriControlApi(): ControlApi {
  return {
    attach: () => invokeCommand('control_attach') as Promise<ControlInfo>,
    onRequest: (cb) => listen<ControlRequest>(EVENTS.controlRequest, (event) => cb(event.payload)),
    reply: (id, reply) => invokeCommand('control_reply', { id, reply }) as Promise<void>,
  };
}

import { listen } from '@tauri-apps/api/event';
import { EVENTS } from './ipc/bindings';
import { invokeCommand } from './ipc/invoke';

export async function listenForAppQuit(handle: (id: number) => void): Promise<() => void> {
  const stop = await listen<number>(EVENTS.appQuitRequested, ({ payload }) => handle(payload), {
    target: 'main',
  });
  try {
    await invokeCommand('app_quit_listen');
    return stop;
  } catch (error) {
    stop();
    throw error;
  }
}

export function replyToAppQuit(id: number, approved: boolean): Promise<void> {
  return invokeCommand('app_quit_reply', { id, approved }) as Promise<void>;
}

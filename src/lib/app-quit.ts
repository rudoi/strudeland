import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export async function listenForAppQuit(handle: (id: number) => void): Promise<() => void> {
  const stop = await listen<number>('app://quit-requested', ({ payload }) => handle(payload), {
    target: 'main',
  });
  try {
    await invoke('app_quit_listen');
    return stop;
  } catch (error) {
    stop();
    throw error;
  }
}

export function replyToAppQuit(id: number, approved: boolean): Promise<void> {
  return invoke('app_quit_reply', { id, approved });
}

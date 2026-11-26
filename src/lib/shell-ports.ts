/**
 * The Tauri side of the shell's ports: the existing adapters, unwrapped. `app-quit.ts`,
 * `project/api.ts` and `export/api.ts` stay the only modules that call Tauri for Quit, the
 * project menu and the Export Audio item.
 */
import { listenForAppQuit, replyToAppQuit } from './app-quit';
import { setExportMenuAvailable } from './export/api';
import { onProjectMenu } from './project/api';
import type { MenuPort, QuitPort } from './shell';

export const quitPort: QuitPort = { listen: listenForAppQuit, reply: replyToAppQuit };

export const menuPort: MenuPort = {
  listen: onProjectMenu,
  setExportAvailable: setExportMenuAvailable,
};

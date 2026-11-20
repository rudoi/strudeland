import { get, writable } from 'svelte/store';
import type { ProjectMenuAction } from './api';
import type { Session } from './session';

type ProjectSession = Pick<
  Session,
  'openProject' | 'openRecent' | 'pickNewProjectFolder' | 'createProject'
>;

/** One project picker or creation flow at a time, across the menu, shortcut, and welcome screen. */
export function createProjectActions(session: ProjectSession) {
  const folder = writable<string | null>(null);
  let busy = false;

  async function whileBusy(action: () => Promise<void>) {
    busy = true;
    try {
      await action();
    } finally {
      busy = false;
    }
  }

  return {
    folder: { subscribe: folder.subscribe },
    async run(action: ProjectMenuAction) {
      if (busy || get(folder) !== null) return;
      await whileBusy(async () => {
        if (action === 'openProject') await session.openProject();
        else folder.set(await session.pickNewProjectFolder());
      });
    },
    async openRecent(path: string) {
      if (busy || get(folder) !== null) return;
      await whileBusy(() => session.openRecent(path));
    },
    async create(name: string) {
      const dir = get(folder);
      const trimmed = name.trim();
      if (busy || dir === null || !trimmed) return;
      await whileBusy(async () => {
        folder.set(null);
        await session.createProject(dir, trimmed);
      });
    },
    cancel() {
      if (!busy) folder.set(null);
    },
  };
}

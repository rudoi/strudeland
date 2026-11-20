import { writable } from 'svelte/store';
import type { ProjectAction } from './api';
import type { Session } from './session';

type ProjectSession = Pick<Session, 'openProject' | 'openRecent' | 'newProject'>;

/** One project picker or creation flow at a time, across the menu, shortcut, and welcome screen. */
export function createProjectActions(session: ProjectSession) {
  const folder = writable<string | null>(null);
  let busy = false;
  let destroyed = false;
  let finishNaming: ((name: string | null) => void) | undefined;

  async function runOnce(action: () => Promise<void>) {
    if (busy || destroyed) return;
    busy = true;
    try {
      await action();
    } finally {
      busy = false;
    }
  }

  async function askName(dir: string): Promise<string | null> {
    if (destroyed) return null;
    return new Promise((resolve) => {
      finishNaming = resolve;
      folder.set(dir);
    });
  }

  function finish(name: string | null) {
    const resolve = finishNaming;
    if (!resolve) return;
    finishNaming = undefined;
    folder.set(null);
    resolve(name);
  }

  return {
    folder: { subscribe: folder.subscribe },
    isBusy: () => busy,
    run: (action: ProjectAction) =>
      runOnce(() =>
        action === 'openProject' ? session.openProject() : session.newProject(askName),
      ),
    openRecent: (path: string) => runOnce(() => session.openRecent(path)),
    create(name: string) {
      const trimmed = name.trim();
      if (trimmed) finish(trimmed);
    },
    cancel: () => finish(null),
    destroy() {
      destroyed = true;
      finish(null);
    },
  };
}

import type { ProjectMenuAction } from '../../src/lib/project/api';
import type { MenuPort, QuitPort } from '../../src/lib/shell';

/**
 * The shell's ports in memory. `log` records `menu.listen`, `menu.unlisten`, `quit.listen`,
 * `quit.unlisten`, `reply:<id>:<approved>` and `exportAvailable:<bool>`. While `holdMenuListen`
 * or `holdQuitListen` is set, that listen waits for it before resolving (it is logged when
 * called). `replyFails` makes `reply` reject after logging. `onMenuListen` runs inside
 * `menu.listen`, before it waits.
 */
export function fakeShellPorts() {
  const log: string[] = [];
  let menuHandler: ((action: ProjectMenuAction) => void) | null = null;
  let quitHandler: ((id: number) => void) | null = null;
  const fake = {
    log,
    holdMenuListen: null as Promise<void> | null,
    holdQuitListen: null as Promise<void> | null,
    replyFails: false,
    onMenuListen: null as (() => void) | null,
    /** Delivers a menu action to the registered handler, if any. */
    emitMenu(action: ProjectMenuAction) {
      menuHandler?.(action);
    },
    /** Delivers a native Quit request to the registered handler, if any. */
    emitQuit(id: number) {
      quitHandler?.(id);
    },
  };
  const menu: MenuPort = {
    async listen(handle) {
      log.push('menu.listen');
      fake.onMenuListen?.();
      if (fake.holdMenuListen) await fake.holdMenuListen;
      menuHandler = handle;
      return () => {
        log.push('menu.unlisten');
        if (menuHandler === handle) menuHandler = null;
      };
    },
    async setExportAvailable(available) {
      log.push(`exportAvailable:${available}`);
    },
  };
  const quit: QuitPort = {
    async listen(handle) {
      log.push('quit.listen');
      if (fake.holdQuitListen) await fake.holdQuitListen;
      quitHandler = handle;
      return () => {
        log.push('quit.unlisten');
        if (quitHandler === handle) quitHandler = null;
      };
    },
    async reply(id, approved) {
      log.push(`reply:${id}:${approved}`);
      if (fake.replyFails) throw new Error('reply failed');
    },
  };
  return Object.assign(fake, { quit, menu });
}

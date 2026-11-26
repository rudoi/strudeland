import { invoke } from '@tauri-apps/api/core';
import { toCommandError } from './command-error';

/** The only `invoke` caller in `src/`: every rejection becomes a `CommandError`, so a raw
 *  object never reaches the UI. Each adapter casts the result at its call. */
export async function invokeCommand(
  name: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return await invoke(name, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

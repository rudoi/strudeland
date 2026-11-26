import { invoke } from '@tauri-apps/api/core';
import type { CommandArgs, CommandName } from './bindings';
import { toCommandError } from './command-error';

/** A command's invoke arguments as a rest tuple: optional when the command takes none (or only
 *  optional ones), required otherwise. */
export type ArgsOf<N extends CommandName> =
  Record<string, never> extends CommandArgs[N] ? [args?: CommandArgs[N]] : [args: CommandArgs[N]];

/** The only `invoke` caller in `src/`: every rejection becomes a `CommandError`, so a raw
 *  object never reaches the UI. The name and argument keys come from the generated bindings;
 *  each adapter casts the result at its call. */
export async function invokeCommand<N extends CommandName>(
  name: N,
  ...args: ArgsOf<N>
): Promise<unknown> {
  try {
    return await invoke(name, args[0] as Record<string, unknown> | undefined);
  } catch (e) {
    throw toCommandError(e);
  }
}

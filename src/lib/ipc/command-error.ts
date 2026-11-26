import { errorMessage } from '../errors';
import { COMMAND_ERROR_KINDS, type CommandErrorKind } from './bindings';

// The command error every fallible Tauri command rejects with (`src-tauri/src/command_error.rs`).
// No Tauri import, so pure modules (`recording/types.ts`, `export/error.ts`) can use it. Kept out
// of `errors.ts`, which the headless host bundles.

/** A command's rejection: the message the user sees and its kind. Each adapter narrows `K`. */
export class CommandError<K extends string = CommandErrorKind> extends Error {
  constructor(
    message: string,
    readonly kind: K,
    options?: ErrorOptions,
  ) {
    super(message, options);
    this.name = 'CommandError';
  }
}

/** `kind` if it is in `allowed`, else `fallback` (also for a non-string). */
export function narrowKind<K extends string>(kind: unknown, allowed: readonly K[], fallback: K): K {
  return (allowed as readonly unknown[]).includes(kind) ? (kind as K) : fallback;
}

/** Any rejection as a CommandError: `{ kind, message }` keeps a known kind (unknown → other),
 *  a string or another value is its text with kind other. The raw value is the `cause`. */
export function toCommandError(e: unknown): CommandError {
  if (e instanceof CommandError) return e as CommandError;
  if (typeof e === 'object' && e !== null && !(e instanceof Error)) {
    const { kind, message } = e as { kind?: unknown; message?: unknown };
    if (typeof message === 'string') {
      return new CommandError(message, narrowKind(kind, COMMAND_ERROR_KINDS, 'other'), {
        cause: e,
      });
    }
  }
  return new CommandError(errorMessage(e), 'other', { cause: e });
}

/** Any rejection as an adapter's `Error` subclass: an instance of it is kept, anything else goes
 *  through `toCommandError` and keeps its kind if it is in `allowed`, else `fallback`. The raw
 *  value stays the `cause`. */
export function narrowError<K extends string, E extends CommandError<K>>(
  e: unknown,
  Make: new (message: string, kind: K, options?: ErrorOptions) => E,
  allowed: readonly K[],
  fallback: K,
): E {
  if (e instanceof Make) return e;
  const c = toCommandError(e);
  return new Make(c.message, narrowKind(c.kind, allowed, fallback), { cause: c.cause });
}

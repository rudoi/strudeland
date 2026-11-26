import { errorMessage } from '../errors';

// The command error every fallible Tauri command rejects with (`src-tauri/src/command_error.rs`).
// No Tauri import, so pure modules (`recording/types.ts`, `export/error.ts`) can use it. Kept out
// of `errors.ts`, which the headless host bundles.

/** The Rust `CommandErrorKind`, in declaration order. */
export const COMMAND_ERROR_KINDS = [
  'projectChanged',
  'noProject',
  'notFound',
  'exists',
  'invalidName',
  'busy',
  'permissionDenied',
  'noDevice',
  'deviceLost',
  'limitReached',
  'stale',
  'output',
  'badBatch',
  'engine',
  'other',
] as const;
export type CommandErrorKind = (typeof COMMAND_ERROR_KINDS)[number];

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

/** `kind` if it is in `allowed`, else `fallback`. */
export function narrowKind<K extends string>(kind: string, allowed: readonly K[], fallback: K): K {
  return (allowed as readonly string[]).includes(kind) ? (kind as K) : fallback;
}

/** Any rejection as a CommandError: `{ kind, message }` keeps a known kind (unknown → other),
 *  a string or another value is its text with kind other. The raw value is the `cause`. */
export function toCommandError(e: unknown): CommandError {
  if (e instanceof CommandError) return e as CommandError;
  if (typeof e === 'object' && e !== null && !(e instanceof Error)) {
    const { kind, message } = e as { kind?: unknown; message?: unknown };
    if (typeof message === 'string') {
      const known = narrowKind(typeof kind === 'string' ? kind : '', COMMAND_ERROR_KINDS, 'other');
      return new CommandError(message, known, { cause: e });
    }
  }
  return new CommandError(errorMessage(e), 'other', { cause: e });
}

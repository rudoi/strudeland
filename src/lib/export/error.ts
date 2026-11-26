import { CommandError, toCommandError } from '../ipc/command-error';

// The export commands' rejection. Tauri-free (it imports only `../ipc/command-error`), so the
// dialog controller and its tests import nothing from Tauri at runtime.

/** A `CommandError` with the files already published when a run stopped part way. */
export class ExportError extends CommandError {
  constructor(
    message: string,
    kind: CommandError['kind'],
    readonly written: string[],
    options?: ErrorOptions,
  ) {
    super(message, kind, options);
    this.name = 'ExportError';
  }
}

/** A rejection of an export command to an `ExportError`: `written` comes off the raw
 *  `{ kind, message, written }` the `CommandError` keeps as its cause, else it is empty. */
export function toExportError(e: unknown): ExportError {
  if (e instanceof ExportError) return e;
  const c = e instanceof CommandError ? (e as CommandError) : toCommandError(e);
  const raw = (c.cause as { written?: unknown } | null | undefined)?.written;
  const written =
    Array.isArray(raw) && raw.every((p) => typeof p === 'string') ? (raw as string[]) : [];
  return new ExportError(c.message, c.kind, written, { cause: c.cause });
}

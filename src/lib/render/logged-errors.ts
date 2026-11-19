/**
 * Strudel core's `logger` messages as render errors, shared by the render page (which hears them
 * as `strudel.log` DOM events) and the headless host (which has no DOM and hears them through
 * `console.log`, the logger's other output). Pure.
 */

import type { RenderError } from './contract';
import { classifyError } from './plan';

/** Whether a logger message reports a failure. Adapted from the earlier render harness's filter
 *  to allow the success line `[superdough] AudioWorklets loaded`. */
export function isErrorLog(message: string, type: unknown): boolean {
  return (
    type === 'error' || /error|not found|is it loaded|could not load AudioWorklet/i.test(message)
  );
}

/** The `RenderError` of a logger message, or null when it does not report a failure. */
export function loggedError(message: string, type: unknown): RenderError | null {
  return isErrorLog(message, type) ? classifyError(message) : null;
}

/** The message of a `console.log` call made by core's `logger`, which logs
 *  `` `%c${message}` `` and a CSS string; null for any other `console.log`. The logger's `type`
 *  is not in the call, so a message seen this way is judged by its text alone. */
export function loggerMessage(args: readonly unknown[]): string | null {
  const [first, style] = args;
  return args.length === 2 &&
    typeof first === 'string' &&
    first.startsWith('%c') &&
    typeof style === 'string'
    ? first.slice(2)
    : null;
}

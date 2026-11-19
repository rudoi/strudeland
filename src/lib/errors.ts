/** Shared error helpers; no imports, so any module (the render page, the runtime, scripts) can
 *  use them. */

/** An error (or any thrown value) as a message string. */
export const errorMessage = (e: unknown): string => (e instanceof Error ? e.message : String(e));

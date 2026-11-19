/** The `{ kind, message }` object a Tauri command rejects with: its kind (unchecked) and message,
 *  else null (no string `message`). Kept out of `errors.ts`, which the headless host bundles. */
export function kindedError(e: unknown): { kind: unknown; message: string } | null {
  if (typeof e !== 'object' || e === null) return null;
  const { kind, message } = e as { kind?: unknown; message?: unknown };
  return typeof message === 'string' ? { kind, message } : null;
}

/** Sets globals (Web Audio and AudioWorklet names bun lacks) and returns a function that restores
 *  them. `onlyIfMissing` leaves a global that already exists alone. */
export function stubGlobals(
  values: Record<string, unknown>,
  { onlyIfMissing = false }: { onlyIfMissing?: boolean } = {},
): () => void {
  const g = globalThis as Record<string, unknown>;
  const saved = Object.keys(values).map((name) => [name, g[name]] as const);
  for (const [name, value] of Object.entries(values)) {
    if (!onlyIfMissing || g[name] === undefined) g[name] = value;
  }
  return () => {
    for (const [name, value] of saved) g[name] = value;
  };
}

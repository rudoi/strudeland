/** The pattern with `seed` applied to its random functions; `pattern` itself without a seed. */
export const applySeed = <P extends { seed(n: number): P }>(
  pattern: P,
  seed: number | undefined,
): P => (seed === undefined ? pattern : pattern.seed(seed));

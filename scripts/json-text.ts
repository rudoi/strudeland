/** JSON as the scripts write it: 2-space indent and a final newline. */
export const jsonText = (value: unknown): string => JSON.stringify(value, null, 2) + '\n';

/** Code-unit ("byte") order of two strings: the order `sort()` uses with no comparator. */
export const byteOrder = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** `record` with its keys in byte order (for JSON written in a stable order). */
export const sortedKeys = <T>(record: Record<string, T>): Record<string, T> =>
  Object.fromEntries(Object.entries(record).sort(([a], [b]) => byteOrder(a, b)));

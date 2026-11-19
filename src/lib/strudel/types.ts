/** Minimal structural types for the untyped Strudel JS packages. */
export interface Hap {
  value: Record<string, unknown>;
  whole?: unknown;
  isActive(currentTime: number): boolean;
}

/** A Strudel `Pattern` instance (the package is untyped). */
export type StrudelPattern = any;

/**
 * How a scheduled hap is routed: `default` to the repl's output only, `pattern` to its own
 * `.onSchedule` only, `both` to the two.
 */
export type ScheduleRoute = 'default' | 'pattern' | 'both';

/** Short synthetic signals for the scope tests. Deterministic. */

/** `n` samples of a sine of `period` samples, starting at `phase` radians, at `amp`. */
export function periodSine(n: number, period: number, phase = 0, amp = 1): Float32Array {
  return Float32Array.from(
    { length: n },
    (_, i) => amp * Math.sin((2 * Math.PI * i) / period + phase),
  );
}

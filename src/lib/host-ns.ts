/** A non-negative decimal nanosecond string (a Rust `u64` host time) to a bigint, else null. The
 *  recording API parses the `host_now_ns` command's result with it (playback reads its clock). */
export function hostNsOrNull(v: unknown): bigint | null {
  return typeof v === 'string' && /^\d{1,20}$/.test(v) ? BigInt(v) : null;
}

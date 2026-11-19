/**
 * The webview half of bar 1's position in a take, pure (no Tauri, DOM or Strudel imports): the
 * offset from `performance.now()` to the host clock, and the host time at which a context time
 * leaves the speakers. Nanoseconds are bigint (they exceed 2^53) and cross IPC as decimal strings.
 * Rust turns `bar1HostNs` into a frame of the WAV; nothing here knows about frames.
 */
import type { OutputStamp } from '../audio/output-clock';
import type { ClockOffsetJson } from './types';

export { clickInterval } from '../audio/metronome';

/** One ping: `performance.now()` before and after asking the host for its time. */
export interface ClockSample {
  beforeMs: number;
  afterMs: number;
  hostNs: bigint;
}

/** `hostNs − performance.now()` in ns (signed), and the round trip of the ping it came from. */
export interface ClockOffset {
  offsetNs: bigint;
  rttNs: bigint;
}

/** Pings `measureClockOffset` sends by default. */
export const CLOCK_PINGS = 20;
/** Answered pings below which a measurement fails. */
export const MIN_CLOCK_SAMPLES = 5;

const msToNs = (ms: number): bigint => BigInt(Math.round(ms * 1e6));

const validSample = (s: ClockSample): boolean =>
  Number.isFinite(s.beforeMs) && Number.isFinite(s.afterMs) && s.afterMs >= s.beforeMs;

/**
 * The offset from the sample with the smallest round trip (ties: the later one), taken at the
 * midpoint of its ping. Samples with a non-finite time or a negative round trip are ignored.
 * Throws when no valid sample is left.
 */
export function pickClockOffset(samples: readonly ClockSample[]): ClockOffset {
  let best: ClockSample | null = null;
  for (const s of samples) {
    if (!validSample(s)) continue;
    if (best === null || s.afterMs - s.beforeMs <= best.afterMs - best.beforeMs) best = s;
  }
  if (best === null) throw new Error('no clock samples');
  return {
    offsetNs: best.hostNs - msToNs((best.beforeMs + best.afterMs) / 2),
    rttNs: msToNs(best.afterMs - best.beforeMs),
  };
}

/**
 * `n` sequential pings of the host clock, then `pickClockOffset`. A failed ping is skipped; fewer
 * than `MIN_CLOCK_SAMPLES` answered pings throws.
 */
export async function measureClockOffset(
  deps: { hostNowNs(): Promise<bigint>; nowMs(): number },
  n = CLOCK_PINGS,
): Promise<ClockOffset> {
  const samples: ClockSample[] = [];
  for (let i = 0; i < n; i++) {
    const beforeMs = deps.nowMs();
    let hostNs: bigint;
    try {
      hostNs = await deps.hostNowNs();
    } catch {
      continue;
    }
    const sample = { beforeMs, afterMs: deps.nowMs(), hostNs };
    if (validSample(sample)) samples.push(sample);
  }
  if (samples.length < MIN_CLOCK_SAMPLES) {
    throw new Error(`the host clock answered ${samples.length} of ${n} pings`);
  }
  return pickClockOffset(samples);
}

/**
 * The host time at which context time `T` leaves the device: the stamp says `contextTime` is
 * heard at `performanceMs`, so `T` is heard `T − contextTime` seconds from then (before or after).
 * `outputLatency` is not added: the stamp already is the output time. `extraDelaySec` is a delay
 * in the graph after `T` that the stamp does not see (the master worklet's lookahead).
 */
export function downbeatHostNs(
  offsetNs: bigint,
  stamp: OutputStamp,
  T: number,
  extraDelaySec = 0,
): bigint {
  return (
    offsetNs + msToNs(stamp.performanceMs + (T - stamp.contextTime) * 1000 + extraDelaySec * 1000)
  );
}

/** An output stamp as the sidecar's `outputStamps` stores it: `[contextTime, hostNs]`. */
export function stampToSidecar(offsetNs: bigint, stamp: OutputStamp): [number, string] {
  return [stamp.contextTime, (offsetNs + msToNs(stamp.performanceMs)).toString()];
}

/** A clock offset as the sidecar stores it. */
export function offsetToSidecar(o: ClockOffset): ClockOffsetJson {
  return { offsetNs: o.offsetNs.toString(), rttNs: o.rttNs.toString() };
}

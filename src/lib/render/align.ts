/**
 * Onset-envelope alignment: the lag that best lines two envelopes up, by length-normalised
 * cross-correlation, and how well they correlate there. Works on envelopes (not PCM) so goldens,
 * which store envelopes, can be aligned. Pure.
 */

export const MIN_ALIGN_CORRELATION = 0.3;

export interface AlignOptions {
  /** Hop of the envelopes, seconds (default 0.01). */
  hopSec?: number;
  /** Largest lag searched, seconds (default 3). */
  maxLagSec?: number;
  /** Correlation needed to trust the result (default MIN_ALIGN_CORRELATION). */
  minCorrelation?: number;
}

export interface Alignment {
  /** Seconds to shift b by to line it up with a; positive = b is later. */
  offsetSec: number;
  /** Plain normalized correlation at that lag, in [-1, 1]; 0 when undefined. */
  correlation: number;
  /** correlation >= minCorrelation. */
  trusted: boolean;
  /** Set when not trusted: why. */
  note?: string;
}

const MIN_OVERLAP = 8;

/** Zero-mean, unit-L2 copy, or null when the envelope has no energy. */
function normalise(x: ArrayLike<number>): Float64Array | null {
  const n = x.length;
  if (n === 0) return null;
  let mean = 0;
  for (let i = 0; i < n; i++) mean += x[i]!;
  mean /= n;
  const out = new Float64Array(n);
  let norm = 0;
  for (let i = 0; i < n; i++) {
    out[i] = x[i]! - mean;
    norm += out[i]! * out[i]!;
  }
  norm = Math.sqrt(norm);
  if (!(norm > 1e-9)) return null;
  for (let i = 0; i < n; i++) out[i] = out[i]! / norm;
  return out;
}

/** Align onset envelope `b` to `a` (both from onsetEnvelope at the same hopSec). */
export function alignOnsets(
  a: ArrayLike<number>,
  b: ArrayLike<number>,
  opts: AlignOptions = {},
): Alignment {
  const hopSec = opts.hopSec ?? 0.01;
  const maxLagSec = opts.maxLagSec ?? 3;
  const minCorrelation = opts.minCorrelation ?? MIN_ALIGN_CORRELATION;

  const na = normalise(a);
  const nb = normalise(b);
  if (!na || !nb) {
    return { offsetSec: 0, correlation: 0, trusted: false, note: 'no onset energy' };
  }
  const la = na.length;
  const lb = nb.length;
  const maxLag = Math.max(0, Math.min(Math.round(maxLagSec / hopSec), Math.min(la, lb) - 1));

  let bestLag = 0;
  let bestScore = -Infinity;
  let found = false;
  for (let step = 0; step <= 2 * maxLag; step++) {
    // 0, +1, -1, +2, -2, ... so ties keep the smallest |lag| (and lag 0).
    const lag = step === 0 ? 0 : step % 2 === 1 ? (step + 1) / 2 : -step / 2;
    const lo = Math.max(0, -lag);
    const hi = Math.min(la, lb - lag);
    const overlap = hi - lo;
    if (overlap < MIN_OVERLAP) continue;
    let dot = 0;
    for (let i = lo; i < hi; i++) dot += na[i]! * nb[i + lag]!;
    const score = dot / Math.sqrt(overlap);
    if (score > bestScore) {
      bestScore = score;
      bestLag = lag;
      found = true;
    }
  }
  if (!found) {
    return { offsetSec: 0, correlation: 0, trusted: false, note: 'overlap too short to align' };
  }

  const lo = Math.max(0, -bestLag);
  const hi = Math.min(la, lb - bestLag);
  let dot = 0;
  let ea = 0;
  let eb = 0;
  for (let i = lo; i < hi; i++) {
    const x = na[i]!;
    const y = nb[i + bestLag]!;
    dot += x * y;
    ea += x * x;
    eb += y * y;
  }
  let correlation = dot / Math.sqrt(ea * eb);
  if (!Number.isFinite(correlation)) correlation = 0;
  const offsetSec = bestLag * hopSec;
  if (correlation >= minCorrelation) return { offsetSec, correlation, trusted: true };
  return {
    offsetSec,
    correlation,
    trusted: false,
    note: `correlation ${correlation.toFixed(3)} below ${minCorrelation} (estimate ${offsetSec.toFixed(3)} s)`,
  };
}

/**
 * The master meter and safety limiter, pure (no Web Audio): the JS port of
 * `crates/strudeland-live/src/master.rs`, run by the `master-meter` AudioWorklet
 * (`master-worklet.ts`) at the context's sample rate. Same algorithm and constants as the Rust
 * stage; every time constant is in seconds and converted per rate.
 *
 * `process` first measures the incoming block (levels describe the mix as written, before the
 * limiter), then writes the block delayed by the lookahead and, when the limiter is on, limited
 * to `CEILING_DB`. The delay line is always in the path, so toggling never moves audio. The gain
 * computer runs on every frame whether or not the limiter is on (`enabled` only decides whether
 * the gain is applied and reported), so turning it on limits the frames already in the delay
 * line too. Per frame
 * `n` with `LA` lookahead frames and the linear ceiling `c`: `req[n]` is 1 or `c / max|x|`; `m[n]`
 * the minimum of `req` over `[n - LA, n]`; `h[n]` an exponential release of `m` (instant attack);
 * `g[n]` the mean of `h` over `[n - LA, n]`, snapped to 1 within `GAIN_SNAP` and capped at
 * `req[n - LA]`; the output `y[n] = x[n - LA] * g[n]`, so `|y| <= c` exactly. Every buffer is
 * allocated in `createMasterDsp`; `process` allocates nothing.
 */
import type { LevelFrame } from '../levels/levels';

/** The limiter's ceiling (sample peak). */
export const CEILING_DB = -1.0;
/** The limiter's lookahead, which is also the master's delay. */
export const LOOKAHEAD_SEC = 0.0015;
export const RELEASE_SEC = 0.05;
/** A gain within this of 1.0 becomes exactly 1.0. */
export const GAIN_SNAP = 1e-6;
/** How often the worklet posts a level frame (the Rust player's `SCOPE_EVERY`). */
export const LEVEL_EVERY_SEC = 0.033;
/** The worklet's registered processor name. */
export const MASTER_PROCESSOR = 'master-meter';

/** The lookahead in frames at `rate`: `round(LOOKAHEAD_SEC * rate)` (72 at 48 kHz). */
export function lookaheadFrames(rate: number): number {
  return Math.round(LOOKAHEAD_SEC * rate);
}

/** True when `f` carries nothing: no signal on either channel, no overs, no limiting. */
export function isSilentFrame(f: LevelFrame): boolean {
  return (
    f.peak[0] === 0 && f.peak[1] === 0 && f.overs[0] === 0 && f.overs[1] === 0 && f.limitDb === 0
  );
}

/**
 * The worklet's post filter: true when `f` should be posted. A silent frame is posted only when
 * the previously posted one was not silent (so the meter falls to silence once), then skipped
 * until sound returns; the first frame is always posted.
 */
export function createSilenceGate(): (f: LevelFrame) => boolean {
  let lastSilent = false;
  let posted = false;
  return (f) => {
    const silent = isSilentFrame(f);
    if (silent && lastSilent && posted) return false;
    lastSilent = silent;
    posted = true;
    return true;
  };
}

export interface MasterDsp {
  /** Lookahead (and delay) in frames. */
  readonly lookahead: number;
  /** The linear ceiling. */
  readonly ceiling: number;
  readonly channels: number;
  enabled(): boolean;
  /**
   * Turns the limiter on or off. Nothing is reset: the gain computer runs on every frame either
   * way, so the frames already in the delay line are limited from the toggle on.
   */
  setEnabled(on: boolean): void;
  /**
   * Measures `input` (channels 0 and 1; a one-channel input counts for both), then writes the
   * delayed (and, when on, limited) block to `output`. The block length is `output[0].length`; a
   * missing input channel is silence. `input` and `output` may be the same arrays.
   */
  process(input: readonly Float32Array[], output: Float32Array[]): void;
  /** Frames measured since the last `takeFrame`. */
  pendingFrames(): number;
  /** The levels since the last call, and zeroes them; null when nothing was processed. */
  takeFrame(): LevelFrame | null;
  /** Clears the delay line and the limiter's state; keeps the enabled flag and the levels. */
  resetStream(): void;
  /** One frame of the limiter: the gain `g[n]` for the input frame's peak (exposed for tests). */
  gain(peak: number): number;
  /** Pushes `req` and returns the minimum over the last `LA + 1` (exposed for tests). */
  slidingMin(req: number): number;
}

/** A dsp for `rate` Hz and `channels` output channels (the limiter's gain is linked across all). */
export function createMasterDsp(rate: number, channels = 2): MasterDsp {
  const LA = lookaheadFrames(rate);
  if (!Number.isFinite(rate) || LA < 1) throw new Error(`bad sample rate ${rate}`);
  if (!Number.isInteger(channels) || channels < 1) throw new Error(`bad channel count ${channels}`);
  const window = LA + 1;
  const ceiling = Math.pow(10, CEILING_DB / 20);
  const release = Math.exp(-1 / (RELEASE_SEC * rate));

  let enabled = false;
  // The last LA input frames, interleaved by frame; `delayPos` is the oldest.
  const delay = new Float32Array(channels * LA);
  let delayPos = 0;
  // The sliding minimum: a monotonic deque of (frame, req) in a ring of LA + 1 slots.
  const dqFrame = new Float64Array(window);
  const dqReq = new Float64Array(window);
  let dqHead = 0;
  let dqLen = 0;
  let frame = 0;
  let held = 1;
  const boxed = new Float64Array(window).fill(1);
  let boxPos = 0;
  let boxSum = window;
  // req of the last LA + 1 frames, at frame % (LA + 1).
  const reqs = new Float64Array(window).fill(1);

  // The level accumulator.
  const peak = [0, 0];
  const sumSq = [0, 0];
  const overs = [0, 0];
  let frames = 0;
  let minGain = 1;

  const resetLimiter = () => {
    dqHead = 0;
    dqLen = 0;
    frame = 0;
    held = 1;
    boxed.fill(1);
    boxPos = 0;
    boxSum = window;
    reqs.fill(1);
  };

  const slidingMin = (req: number): number => {
    const n = frame;
    frame += 1;
    while (dqLen > 0) {
      const back = (dqHead + dqLen - 1) % window;
      if (dqReq[back]! >= req) dqLen -= 1;
      else break;
    }
    while (dqLen > 0 && dqFrame[dqHead]! + LA < n) {
      dqHead = (dqHead + 1) % window;
      dqLen -= 1;
    }
    // at most LA + 1 entries: every one left is within the window [n - LA, n]
    const at = (dqHead + dqLen) % window;
    dqFrame[at] = n;
    dqReq[at] = req;
    dqLen += 1;
    return dqReq[dqHead]!;
  };

  const gain = (p: number): number => {
    const req = p <= ceiling ? 1 : ceiling / p;
    const slot = frame % window;
    reqs[slot] = req;
    // the frame output now, `n - LA`, sits in the slot after this one
    const delayedReq = reqs[(slot + 1) % window]!;
    const m = slidingMin(req);
    held = m < held ? m : m - (m - held) * release;
    // the box filter: replace the oldest h, refresh the sum exactly once per wrap
    boxSum += held - boxed[boxPos]!;
    boxed[boxPos] = held;
    boxPos = (boxPos + 1) % window;
    if (boxPos === 0) {
      let s = 0;
      for (let i = 0; i < window; i++) s += boxed[i]!;
      boxSum = s;
    }
    let g = boxSum / window;
    if (1 - g < GAIN_SNAP) g = 1;
    return Math.min(g, delayedReq);
  };

  const finiteAbs = (x: number) => (Number.isFinite(x) ? Math.abs(x) : 0);

  return {
    lookahead: LA,
    ceiling,
    channels,
    enabled: () => enabled,
    setEnabled(on) {
      enabled = on;
    },
    process(input, output) {
      const n = output[0]?.length ?? 0;
      // measure: channels 0 and 1, a mono input for both
      const in0 = input[0];
      const in1 = input.length > 1 ? input[1] : input[0];
      for (let ch = 0; ch < 2; ch++) {
        const buf = ch === 0 ? in0 : in1;
        let p = peak[ch]!;
        let s = sumSq[ch]!;
        let o = overs[ch]!;
        for (let i = 0; i < n; i++) {
          const x = buf ? buf[i]! : 0;
          const a = Number.isFinite(x) ? Math.abs(x) : 1;
          if (a > p) p = a;
          s += a * a;
          if (a >= 1) o++;
        }
        peak[ch] = p;
        sumSq[ch] = s;
        overs[ch] = o;
      }
      frames += n;
      // delay, then limit
      for (let i = 0; i < n; i++) {
        const pos = channels * delayPos;
        let p = 0;
        for (let ch = 0; ch < channels; ch++) {
          const src = input[ch];
          const x = src ? src[i]! : 0;
          const d = delay[pos + ch]!;
          delay[pos + ch] = x;
          const a = finiteAbs(x);
          if (a > p) p = a;
          const out = output[ch];
          if (out) out[i] = d;
        }
        delayPos = (delayPos + 1) % LA;
        // the gain computer runs whether or not the limiter is on, so turning it on finds the
        // gain for the frames already in the delay line
        const g = gain(p);
        if (!enabled) continue;
        if (g < minGain) minGain = g;
        for (let ch = 0; ch < channels; ch++) {
          const out = output[ch];
          if (!out) continue;
          const d = out[i]!;
          // x * g in f64, rounded to f32 by the store (x bit for bit at g = 1); non-finite is 0
          out[i] = !Number.isFinite(d) ? 0 : g === 1 ? d : d * g;
        }
      }
    },
    pendingFrames: () => frames,
    takeFrame() {
      if (frames === 0) return null;
      const rms = (ch: number) => Math.fround(Math.sqrt(sumSq[ch]! / frames));
      const f: LevelFrame = {
        peak: [peak[0]!, peak[1]!],
        rms: [rms(0), rms(1)],
        overs: [overs[0]!, overs[1]!],
        limitDb: Math.fround(20 * Math.log10(minGain)),
      };
      peak[0] = peak[1] = 0;
      sumSq[0] = sumSq[1] = 0;
      overs[0] = overs[1] = 0;
      frames = 0;
      minGain = 1;
      return f;
    },
    resetStream() {
      delay.fill(0);
      delayPos = 0;
      resetLimiter();
    },
    gain,
    slidingMin,
  };
}

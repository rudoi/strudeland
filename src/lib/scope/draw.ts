/**
 * The frame drawing behind `Scope.svelte`: what to draw for each buffer from the clock (a trace, the
 * rest line, or nothing), on any 2D context. No DOM lookups, so tests drive it with a fake context.
 */
import { AFTERGLOW, createGain, restLine, tracePoints, triggerIndex } from './trace';

export type ScopeSize = 'sm' | 'md';

export interface ScopeSpec {
  /** CSS size of the tube, the fallback when it cannot be measured. */
  width: number;
  height: number;
  traceWidth: number;
  glowWidth: number;
  glowAlpha: number;
  /** Samples shown from the trigger. */
  samples: number;
}

export const SCOPE_SPECS: Record<ScopeSize, ScopeSpec> = {
  md: { width: 176, height: 28, traceWidth: 1.5, glowWidth: 5, glowAlpha: 0.35, samples: 1024 },
  sm: { width: 40, height: 16, traceWidth: 1, glowWidth: 3, glowAlpha: 0.3, samples: 512 },
};

/** Under reduced motion, at most one frame per this many milliseconds. */
export const REDUCED_MOTION_INTERVAL = 1000;

/** The colours the scope draws with, resolved once at mount (`resolveColor`). */
export interface ScopeColours {
  trace: string;
  glow: string;
  rest: string;
}

/** The part of `CanvasRenderingContext2D` the scope uses. */
export interface ScopeContext {
  globalCompositeOperation: string;
  globalAlpha: number;
  fillStyle: unknown;
  strokeStyle: unknown;
  lineWidth: number;
  lineCap: string;
  lineJoin: string;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  clearRect(x: number, y: number, w: number, h: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  stroke(): void;
}

/** What one call to `frame` did. */
export type FrameResult = 'trace' | 'rest' | 'skip';

export interface ScopeRenderer {
  /** The canvas was resized to `width` × `height` CSS px at `dpr` (which clears it). */
  resize(width: number, height: number, dpr: number): void;
  /** Draws a buffer from the clock, or the rest line for `null`. */
  frame(buf: Float32Array | null): FrameResult;
  setReducedMotion(on: boolean): void;
}

/** A device pixel ratio, with a bad one (not finite, or not positive) read as 1. */
const safeDpr = (dpr: number) => (Number.isFinite(dpr) && dpr > 0 ? dpr : 1);

/** The canvas backing size for a CSS size: at least 1 px, rounded, with a bad `dpr` read as 1. */
export function canvasPixels(
  width: number,
  height: number,
  dpr: number,
): { width: number; height: number } {
  const d = safeDpr(dpr);
  const px = (v: number) => Math.max(1, Math.round((Number.isFinite(v) ? v : 0) * d));
  return { width: px(width), height: px(height) };
}

export interface ScopeRendererOptions {
  ctx: ScopeContext;
  colours: ScopeColours;
  spec: ScopeSpec;
  /** Milliseconds; defaults to `performance.now()` (or `Date.now()`). */
  now?: () => number;
  reducedMotion?: boolean;
}

export function createScopeRenderer(opts: ScopeRendererOptions): ScopeRenderer {
  const { ctx, colours, spec } = opts;
  const now =
    opts.now ?? (() => (typeof performance !== 'undefined' ? performance.now() : Date.now()));
  const gain = createGain();
  let reduced = opts.reducedMotion ?? false;
  let width = spec.width;
  let height = spec.height;
  /** Whether the rest line is what is on the canvas now. */
  let resting = false;
  let lastTrace = -Infinity;
  /** The trace's x, y pairs, allocated once and rewritten each frame by `tracePoints`. */
  const points = new Float32Array(spec.samples * 2);
  const restPoints = new Float32Array(4);

  /** Strokes the first `n` points of `xy` (interleaved x, y). */
  const stroke = (xy: Float32Array, n: number, style: string, lw: number, alpha: number) => {
    if (n === 0) return;
    ctx.globalAlpha = alpha;
    ctx.strokeStyle = style;
    ctx.lineWidth = lw;
    ctx.beginPath();
    ctx.moveTo(xy[0]!, xy[1]!);
    for (let i = 1; i < n; i++) ctx.lineTo(xy[2 * i]!, xy[2 * i + 1]!);
    ctx.stroke();
  };

  const drawRest = () => {
    ctx.globalCompositeOperation = 'source-over';
    ctx.clearRect(0, 0, width, height);
    const line = restLine(width, height);
    restPoints[0] = line.x1;
    restPoints[1] = line.y;
    restPoints[2] = line.x2;
    restPoints[3] = line.y;
    stroke(restPoints, 2, colours.rest, spec.traceWidth, 1);
    ctx.globalAlpha = 1;
    resting = true;
  };

  const drawTrace = (buf: Float32Array) => {
    const g = gain.next(buf);
    const t = now();
    if (reduced && t - lastTrace < REDUCED_MOTION_INTERVAL) return 'skip' as const;
    lastTrace = t;
    if (reduced) {
      ctx.globalCompositeOperation = 'source-over';
      ctx.clearRect(0, 0, width, height);
    } else {
      // Fade the old frame: erase AFTERGLOW of it, so the trace leaves a short tail.
      ctx.globalCompositeOperation = 'destination-out';
      ctx.globalAlpha = AFTERGLOW;
      ctx.fillStyle = colours.trace;
      ctx.fillRect(0, 0, width, height);
      ctx.globalCompositeOperation = 'source-over';
    }
    const n = tracePoints(points, buf, triggerIndex(buf), spec.samples, width, height, g);
    // A wide low-alpha pass instead of shadowBlur (slow in WebKit), then the trace itself.
    stroke(points, n, colours.glow, spec.glowWidth, spec.glowAlpha);
    stroke(points, n, colours.trace, spec.traceWidth, 1);
    ctx.globalAlpha = 1;
    resting = false;
    return 'trace' as const;
  };

  return {
    resize(w, h, dpr) {
      const d = safeDpr(dpr);
      width = Number.isFinite(w) && w > 0 ? w : spec.width;
      height = Number.isFinite(h) && h > 0 ? h : spec.height;
      ctx.setTransform(d, 0, 0, d, 0, 0);
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      // Resizing cleared the canvas: put the rest line back if that is what was showing.
      if (resting) drawRest();
      lastTrace = -Infinity;
    },
    frame(buf) {
      if (buf === null) {
        if (resting) return 'skip';
        drawRest();
        return 'rest';
      }
      return drawTrace(buf);
    },
    setReducedMotion(on) {
      reduced = on;
    },
  };
}

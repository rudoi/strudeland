import { describe, expect, test } from 'bun:test';
import {
  REDUCED_MOTION_INTERVAL,
  SCOPE_SPECS,
  canvasPixels,
  createScopeRenderer,
  type ScopeContext,
} from '../src/lib/scope/draw';
import { AFTERGLOW } from '../src/lib/scope/trace';
import { periodSine } from './helpers/signals';

type Call = { op: string; args: unknown[]; state: Record<string, unknown> };

/** A 2D context that records each drawing call with the state it ran under. */
function fakeContext() {
  const calls: Call[] = [];
  const ctx: ScopeContext & { calls: Call[]; path: { x: number; y: number }[][] } = {
    calls,
    path: [],
    globalCompositeOperation: 'source-over',
    globalAlpha: 1,
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    lineCap: 'butt',
    lineJoin: 'miter',
    setTransform: (...args) => rec('setTransform', args),
    clearRect: (...args) => rec('clearRect', args),
    fillRect: (...args) => rec('fillRect', args),
    beginPath: () => {
      ctx.path.push([]);
      rec('beginPath', []);
    },
    moveTo: (x, y) => ctx.path[ctx.path.length - 1]!.push({ x, y }),
    lineTo: (x, y) => ctx.path[ctx.path.length - 1]!.push({ x, y }),
    stroke: () => rec('stroke', [ctx.path[ctx.path.length - 1]]),
  };
  function rec(op: string, args: unknown[]) {
    calls.push({
      op,
      args,
      state: {
        gco: ctx.globalCompositeOperation,
        alpha: ctx.globalAlpha,
        fill: ctx.fillStyle,
        stroke: ctx.strokeStyle,
        lw: ctx.lineWidth,
      },
    });
  }
  return ctx;
}

const colours = { trace: 'T', glow: 'G', rest: 'R' };

function setup(size: 'sm' | 'md' = 'md', reducedMotion = false) {
  const ctx = fakeContext();
  let t = 0;
  const r = createScopeRenderer({
    ctx,
    colours,
    spec: SCOPE_SPECS[size],
    now: () => t,
    reducedMotion,
  });
  return { ctx, r, tick: (ms: number) => (t += ms) };
}

const strokes = (ctx: ReturnType<typeof fakeContext>) => ctx.calls.filter((c) => c.op === 'stroke');

describe('SCOPE_SPECS', () => {
  test('match the proposal table', () => {
    expect(SCOPE_SPECS.md).toEqual({
      width: 176,
      height: 28,
      traceWidth: 1.5,
      glowWidth: 5,
      glowAlpha: 0.35,
      samples: 1024,
    });
    expect(SCOPE_SPECS.sm).toEqual({
      width: 40,
      height: 16,
      traceWidth: 1,
      glowWidth: 3,
      glowAlpha: 0.3,
      samples: 512,
    });
  });

  test('show no more samples than the clock buffer holds from a late trigger', () => {
    // triggerIndex returns at most half the buffer (2048 / 2), so 1024 samples always fit.
    expect(SCOPE_SPECS.md.samples).toBeLessThanOrEqual(1024);
  });
});

describe('canvasPixels', () => {
  test('scales by the device pixel ratio and rounds', () => {
    expect(canvasPixels(176, 28, 2)).toEqual({ width: 352, height: 56 });
    expect(canvasPixels(40, 16, 1.5)).toEqual({ width: 60, height: 24 });
    expect(canvasPixels(10.3, 10.6, 1)).toEqual({ width: 10, height: 11 });
  });

  test('reads a bad ratio as 1 and never goes below 1 px', () => {
    expect(canvasPixels(40, 16, 0)).toEqual({ width: 40, height: 16 });
    expect(canvasPixels(40, 16, NaN)).toEqual({ width: 40, height: 16 });
    expect(canvasPixels(0, -5, 2)).toEqual({ width: 1, height: 1 });
    expect(canvasPixels(NaN, 16, 1)).toEqual({ width: 1, height: 16 });
  });
});

describe('the rest line', () => {
  test('null clears and draws a flat centre line in the rest colour', () => {
    const { ctx, r } = setup();
    r.resize(176, 28, 2);
    expect(r.frame(null)).toBe('rest');
    expect(ctx.calls.some((c) => c.op === 'clearRect')).toBe(true);
    const s = strokes(ctx);
    expect(s).toHaveLength(1);
    expect(s[0]!.state.stroke).toBe('R');
    expect(s[0]!.args[0]).toEqual([
      { x: 0, y: 14 },
      { x: 176, y: 14 },
    ]);
  });

  test('a second null while resting draws nothing', () => {
    const { ctx, r } = setup();
    r.frame(null);
    const n = ctx.calls.length;
    expect(r.frame(null)).toBe('skip');
    expect(ctx.calls.length).toBe(n);
  });

  test('a resize while resting redraws the rest line at the new size and scale', () => {
    const { ctx, r } = setup('sm');
    r.frame(null);
    ctx.calls.length = 0;
    r.resize(80, 20, 3);
    expect(ctx.calls[0]).toMatchObject({ op: 'setTransform', args: [3, 0, 0, 3, 0, 0] });
    expect(strokes(ctx)[0]!.args[0]).toEqual([
      { x: 0, y: 10 },
      { x: 80, y: 10 },
    ]);
  });

  test('a resize while tracing draws nothing; the next frame does', () => {
    const { ctx, r } = setup();
    r.frame(periodSine(2048, 64, -1, 0.5));
    ctx.calls.length = 0;
    r.resize(100, 20, 1);
    expect(strokes(ctx)).toHaveLength(0);
  });

  test('a bad size falls back to the spec size', () => {
    const { ctx, r } = setup('sm');
    r.resize(0, NaN, 1);
    r.frame(null);
    expect(strokes(ctx)[0]!.args[0]).toEqual([
      { x: 0, y: 8 },
      { x: 40, y: 8 },
    ]);
  });
});

describe('a trace frame', () => {
  test('fades the old frame, then strokes the glow pass and the trace', () => {
    const { ctx, r } = setup('md');
    r.resize(176, 28, 1);
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('trace');
    const fade = ctx.calls.find((c) => c.op === 'fillRect')!;
    expect(fade.state).toMatchObject({ gco: 'destination-out', alpha: AFTERGLOW });
    expect(fade.args).toEqual([0, 0, 176, 28]);
    const [glow, trace] = strokes(ctx);
    expect(glow!.state).toMatchObject({ gco: 'source-over', stroke: 'G', lw: 5, alpha: 0.35 });
    expect(trace!.state).toMatchObject({ gco: 'source-over', stroke: 'T', lw: 1.5, alpha: 1 });
    expect(ctx.globalAlpha).toBe(1);
  });

  test('draws the spec sample count across the box, inside it', () => {
    const { ctx, r } = setup('sm');
    r.resize(40, 16, 1);
    r.frame(periodSine(2048, 64, -1, 2));
    const points = strokes(ctx)[1]!.args[0] as { x: number; y: number }[];
    expect(points).toHaveLength(512);
    expect(points[0]!.x).toBe(0);
    expect(points[points.length - 1]!.x).toBeCloseTo(40, 6);
    for (const p of points) {
      expect(p.y).toBeGreaterThanOrEqual(1);
      expect(p.y).toBeLessThanOrEqual(15);
    }
  });

  test('each frame redraws all its points (the point buffer is reused)', () => {
    const { ctx, r } = setup('sm');
    r.resize(40, 16, 1);
    r.frame(periodSine(2048, 64, -1, 2));
    r.frame(new Float32Array(2048));
    const points = strokes(ctx)[3]!.args[0] as { x: number; y: number }[];
    expect(points).toHaveLength(512);
    expect(points.every((p) => p.y === 8)).toBe(true);
  });

  test('starts at the rising zero crossing', () => {
    const { ctx, r } = setup('md');
    r.resize(176, 28, 1);
    r.frame(periodSine(2048, 64, -1, 0.5));
    const points = strokes(ctx)[1]!.args[0] as { x: number; y: number }[];
    // The trigger sample is at (or just past) zero, so the first point sits on the centre line.
    expect(points[0]!.y).toBeLessThanOrEqual(14);
    expect(points[0]!.y).toBeGreaterThan(12);
  });

  test('null after a trace goes back to rest', () => {
    const { ctx, r } = setup();
    r.frame(periodSine(2048, 64, -1, 0.5));
    ctx.calls.length = 0;
    expect(r.frame(null)).toBe('rest');
    expect(strokes(ctx)[0]!.state.stroke).toBe('R');
  });
});

describe('reduced motion', () => {
  test('clears instead of fading and draws at most one frame per second', () => {
    const { ctx, r, tick } = setup('md', true);
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('trace');
    expect(ctx.calls.some((c) => c.op === 'fillRect')).toBe(false);
    expect(ctx.calls.some((c) => c.op === 'clearRect')).toBe(true);
    tick(REDUCED_MOTION_INTERVAL - 1);
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('skip');
    tick(1);
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('trace');
  });

  test('can be switched on and off', () => {
    const { ctx, r } = setup('md');
    r.setReducedMotion(true);
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('trace');
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('skip');
    r.setReducedMotion(false);
    ctx.calls.length = 0;
    expect(r.frame(periodSine(2048, 64, -1, 0.5))).toBe('trace');
    expect(ctx.calls.some((c) => c.op === 'fillRect')).toBe(true);
  });

  test('the rest line is drawn at once even under reduced motion', () => {
    const { r } = setup('md', true);
    r.frame(periodSine(2048, 64, -1, 0.5));
    expect(r.frame(null)).toBe('rest');
  });
});

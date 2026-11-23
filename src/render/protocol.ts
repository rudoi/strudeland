/**
 * Messages between the render page (`src/render/main.ts`) and the Playwright driver
 * (`scripts/render/`). Types only; everything crossing `page.evaluate` is plain JSON, and PCM
 * crosses as base64 one channel at a time.
 */

import type { RenderRequest, RenderResult, ScheduleResult } from '../lib/render/contract';
import type { PlannedRender, RenderPlanRequest } from '../lib/render/job-plan';
import type { PageJobInfo } from '../lib/render/page-job';
import type { BINARY, UNARY } from './math';

export type { PageJobInfo } from '../lib/render/page-job';

/** What a page run does: `render` renders a chunk; `schedule` only queries; `job` plans the
 *  chunk's engine job (`src/lib/render/engine-job.ts`) and `plan` plans a whole render, both
 *  without rendering; `math` evaluates V8
 *  `Math.*` over the groups inside an AudioWorklet (`src/render/math.ts`). */
export type StartArgs =
  | {
      kind: 'render' | 'schedule' | 'job';
      request: RenderRequest;
      /** Where corpus samples are served, ending in `/` (`${origin}/samples/`). */
      samplesBase: string;
    }
  | {
      kind: 'plan';
      code: string;
      request: RenderPlanRequest;
      samples?: RenderRequest['samples'];
      samplesBase: string;
    }
  | { kind: 'math'; groups: MathGroupInput[] };

/** A superdough distortion algorithm (`distortionAlgorithms` in `helpers.mjs`). */
export type DistortName =
  'scurve' | 'soft' | 'hard' | 'cubic' | 'diode' | 'asym' | 'fold' | 'sinefold' | 'chebyshev';

/** A function the V8 math reference evaluates: `Math[fn](x)`, `Math.round(x)`, `x | 0`, or one
 *  of the binary ones (`isBinaryFn` in `math.ts`): `Math.pow(x, y)`, `Math.atan2(x, y)` (`x` is
 *  the first argument, `y`), `Math.hypot(x, y)`, `powOp` (`x ** y`) and
 *  `distortionAlgorithms[name](x, y)` (`distort:<name>`). */
export type MathFn = keyof typeof UNARY | keyof typeof BINARY | `distort:${DistortName}`;

/** One group of inputs. Every number is an f64 bit string: 16 lowercase hex digits of the
 *  big-endian IEEE-754 pattern. `ys` (same length as `xs`) is non-null exactly for the binary
 *  functions (`pow`, `atan2`, `hypot`, `powOp` and the `distort:<name>` groups). */
export interface MathGroupInput {
  id: string;
  fn: MathFn;
  xs: string[];
  ys: string[] | null;
}

/** The results of a group, in `xs` order, as f64 bit strings. */
export interface MathGroupOutput {
  id: string;
  out: string[];
}

export type Phase =
  'idle' | 'init' | 'evaluating' | 'querying' | 'loading-samples' | 'rendering' | 'done' | 'failed';

export interface PageStatus {
  phase: Phase;
  done: boolean;
  failed: boolean;
  error: string | null;
  /** The phase the page failed in (`phase` is then 'failed'); null otherwise. */
  failedPhase: Phase | null;
}

/** `timing.totalMs` is 0: the driver sets it. */
export type PageResult = Omit<RenderResult, 'channels'> & { channelCount: number };
export type PageSchedule = ScheduleResult;

export interface StrudelandRender {
  readonly ready: true;
  /** Returns at once; throws if called twice. */
  start(args: StartArgs): void;
  status(): PageStatus;
  /** Kind 'render'; throws unless done and not failed. */
  result(): PageResult;
  /** Kind 'schedule'; throws unless done and not failed. */
  schedule(): PageSchedule;
  /** Base64 of the Float32 channel, little endian. */
  channel(index: number): string;
  /** Kind 'job'; throws unless done and not failed. */
  job(): PageJobInfo;
  /** Slice `index` of the job's JSON, `sliceChars` characters each (default `JOB_SLICE_CHARS`):
   *  `ceil(jsonLength / sliceChars)` slices, concatenated in order (`assembleSlices`). */
  jobSlice(index: number, sliceChars?: number): string;
  /** Kind 'plan'; returns the whole batch plan without rendering. */
  plannedRender(): PlannedRender;
  /** Kind 'math'; throws unless done and not failed. */
  math(): MathGroupOutput[];
}

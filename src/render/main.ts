/**
 * The render page: evaluates one song with the app's runtime (offline backend, the app's
 * prebake), queries it, and renders one chunk through superdough into an `OfflineAudioContext`,
 * or (kind `job`) plans the chunk's engine job without rendering.
 * The Playwright driver (`scripts/render/`) talks to it through `window.strudelandRender`
 * (`protocol.ts`). One page renders one request.
 */

import './guard'; // first: no live AudioContext may exist in this page
import {
  createOfflineBackend,
  defaultDeps,
  registerCorpusSamples,
  renderOffline,
  type OfflineOutput,
} from '../lib/audio/offline';
import { classifyError, eventSources, selectHaps, windowPlan } from '../lib/render/plan';
import { get } from 'svelte/store';
import type { RenderError } from '../lib/render/contract';
import {
  evaluateSong,
  planJob,
  planRender,
  querySong,
  type PlannedRender,
  type PlannerRuntime,
} from '../lib/render/job-plan';
import { lookbackSeconds } from '../lib/render/lookback';
import { eventsFromValues } from '../lib/render/schedule';
import { prebake } from '../lib/strudel/prebake';
import { createRuntime } from '../lib/strudel/runtime';
import { loggedError } from '../lib/render/logged-errors';
import { dedupeErrors, encodeChannel, processorMessage, sliceText } from './pcm';
import mathWorkletUrl from './math-worklet?audioworklet';
import type {
  MathGroupInput,
  MathGroupOutput,
  PageJobInfo,
  PageResult,
  PageSchedule,
  PageStatus,
  Phase,
  StartArgs,
  StrudelandRender,
} from './protocol';
import { errorMessage } from '../lib/errors';

let phase: Phase = 'idle';
let failedPhase: Phase | null = null;
let error: string | null = null;
let started = false;
let renderResult: PageResult | null = null;
let scheduleResult: PageSchedule | null = null;
let output: OfflineOutput | null = null;
let jobInfo: PageJobInfo | null = null;
let jobJson: string | null = null;
let batchPlan: PlannedRender | null = null;
let mathOutputs: MathGroupOutput[] | null = null;
const captured: RenderError[] = [];

/** Turns superdough's log events, worklet warnings and uncaught errors into `RenderError`s. */
function installErrorCapture(): () => void {
  const onLog = (e: Event) => {
    const detail = (e as CustomEvent).detail ?? {};
    const err = loggedError(String(detail.message ?? ''), detail.type);
    if (err) captured.push(err);
  };
  const onError = (e: ErrorEvent) => captured.push(classifyError(`page error: ${e.message}`));
  const onRejection = (e: PromiseRejectionEvent) =>
    captured.push(classifyError(`unhandled rejection: ${errorMessage(e.reason)}`));
  const warn = console.warn;
  console.warn = (...args: unknown[]) => {
    const text = args.map((a) => errorMessage(a)).join(' ');
    if (text.includes('could not load AudioWorklet')) captured.push(classifyError(text));
    warn.apply(console, args);
  };
  document.addEventListener('strudel.log', onLog);
  window.addEventListener('error', onError);
  window.addEventListener('unhandledrejection', onRejection);
  return () => {
    document.removeEventListener('strudel.log', onLog);
    window.removeEventListener('error', onError);
    window.removeEventListener('unhandledrejection', onRejection);
    console.warn = warn;
  };
}

/** Kind `math`: evaluates the groups inside an AudioWorklet of a one-quantum
 *  `OfflineAudioContext` (V8's `Math.*`, as the processors see it); no runtime is created. */
async function runMath(groups: MathGroupInput[]): Promise<void> {
  phase = 'init';
  try {
    const ctx = new OfflineAudioContext(1, 128, 44100);
    await ctx.audioWorklet.addModule(mathWorkletUrl);
    const node = new AudioWorkletNode(ctx, 'v8-math-processor', { processorOptions: { groups } });
    const posted = processorMessage<{ outputs?: MathGroupOutput[]; error?: string }>(
      node,
      'v8-math-processor',
    );
    node.connect(ctx.destination);
    phase = 'rendering';
    // Together, so a processor error that rejects `posted` during the render is handled.
    const [, message] = await Promise.all([ctx.startRendering(), posted]);
    if (message.error !== undefined) throw new Error(`v8-math-processor: ${message.error}`);
    if (!Array.isArray(message.outputs)) throw new Error('v8-math-processor posted no outputs');
    mathOutputs = message.outputs;
    phase = 'done';
  } catch (e) {
    failedPhase = phase;
    error = errorMessage(e);
    phase = 'failed';
  }
}

async function run(args: StartArgs): Promise<void> {
  if (args.kind === 'math') {
    await runMath(args.groups);
    return;
  }
  const { kind, samplesBase } = args;
  const t0 = performance.now();
  phase = 'init';
  const uninstallCapture = installErrorCapture();
  try {
    const runtime = createRuntime({ backends: { superdough: createOfflineBackend() }, prebake });
    await registerCorpusSamples(
      (kind === 'plan' ? args.samples : args.request.samples) ?? {},
      samplesBase,
    );

    const rt: PlannerRuntime = {
      evaluate: (code, opts) =>
        runtime.evaluate(code, {
          ...opts,
          instruments: kind === 'plan' ? args.request.instruments : args.instruments,
        }),
      evalError: () => get(runtime.store).evalError,
      state: () => {
        const s = get(runtime.store);
        return { structure: s.structure, cps: s.cps, seed: s.seed };
      },
      query: (from, to, segments) => runtime.query(from, to, segments),
    };
    const onPhase = (p: Phase): void => {
      phase = p;
    };

    if (kind === 'plan') {
      batchPlan = await planRender(rt, args.code, args.request, captured, onPhase);
      phase = 'done';
      return;
    }
    const { request } = args;
    if (kind === 'job') {
      // The shared planner: the same plan the render path gives renderOffline, without a context.
      ({ info: jobInfo, json: jobJson } = await planJob(rt, request, captured, onPhase));
      phase = 'done';
      return;
    }

    const { structure, cps, seed, segments, timing } = await evaluateSong(
      rt,
      request.code,
      request.layout,
      onPhase,
    );
    const c1 = request.toBar + request.overlapBars;
    const query = querySong(rt, segments);

    // The full schedule [0, c1), selected once: the schedule result, the 'auto' lookback, and
    // every orbit the render must create up front (decision 14).
    const { selected, errors: selectErrors } = selectHaps(query(0, c1));
    const events = eventsFromValues(eventSources(selected), cps, timing);
    if (kind === 'schedule') {
      scheduleResult = { events, cps, structure, errors: dedupeErrors(selectErrors) };
      phase = 'done';
      return;
    }

    const lookbackSec =
      request.lookbackSec === 'auto' ? lookbackSeconds(events) : request.lookbackSec;
    const plan = windowPlan(request, cps, lookbackSec, 0, timing);
    const haps = query(plan.c0, plan.c1);

    const evalMs = performance.now() - t0;

    // renderOffline sets up the context, preloads ('loading-samples'), then renders.
    phase = 'loading-samples';
    output = await renderOffline(
      {
        haps,
        cps,
        seed,
        req: request,
        plan,
        timing,
        scheduleSelected: selected,
        onPhase: (p) => (phase = p),
      },
      defaultDeps,
    );

    const errors = dedupeErrors([...output.errors, ...captured]);
    renderResult = {
      sampleRate: request.sampleRate,
      frames: plan.frames,
      windowStartFrame: plan.windowStartFrame,
      contentFrames: plan.contentFrames,
      ...(plan.overlapFrames === undefined ? {} : { overlapFrames: plan.overlapFrames }),
      lookbackSecUsed: plan.lookbackSecUsed,
      bufferStartSec: plan.bufferStartSec,
      cps,
      events: output.events,
      stemPairs: output.stemPairs,
      errors,
      ok: errors.length === 0,
      timing: { evalMs, renderMs: output.renderMs, totalMs: 0 },
      request,
      channelCount: output.buffer.numberOfChannels,
    };
    phase = 'done';
  } catch (e) {
    failedPhase = phase;
    error = errorMessage(e);
    phase = 'failed';
  } finally {
    uninstallCapture();
  }
}

const api: StrudelandRender = {
  ready: true,
  start(args) {
    if (started) throw new Error('render page already started');
    started = true;
    void run(args);
  },
  status: (): PageStatus => ({
    phase,
    done: phase === 'done',
    failed: phase === 'failed',
    error,
    failedPhase,
  }),
  result() {
    if (!renderResult) throw new Error(`no render result (phase ${phase})`);
    return renderResult;
  },
  schedule() {
    if (!scheduleResult) throw new Error(`no schedule result (phase ${phase})`);
    return scheduleResult;
  },
  channel(index) {
    const buffer = output?.buffer;
    if (!buffer) throw new Error(`no rendered buffer (phase ${phase})`);
    if (!Number.isInteger(index) || index < 0 || index >= buffer.numberOfChannels) {
      throw new RangeError(`channel ${index} out of range (0..${buffer.numberOfChannels - 1})`);
    }
    return encodeChannel(buffer.getChannelData(index));
  },
  job() {
    if (!jobInfo) throw new Error(`no job (phase ${phase})`);
    return jobInfo;
  },
  jobSlice(index, sliceChars) {
    if (jobJson === null) throw new Error(`no job (phase ${phase})`);
    return sliceText(jobJson, index, sliceChars);
  },
  plannedRender() {
    if (!batchPlan) throw new Error(`no render plan (phase ${phase})`);
    return batchPlan;
  },
  math() {
    if (!mathOutputs) throw new Error(`no math outputs (phase ${phase})`);
    return mathOutputs;
  },
};

(window as any).strudelandRender = api;

/**
 * The `Renderer` on the vendored superdough in headless Chromium: one fresh render page per
 * `schedule`/`render` call, the browser and server reused across calls and opened lazily.
 * Imports Playwright (through `chromium.ts`): never import this from a test.
 */

import { errors, type Page } from 'playwright';
import type {
  RenderPlanInput,
  RenderRequest,
  RenderResult,
  Renderer,
  ScheduleResult,
} from '../../src/lib/render/contract';
import type { EngineJob } from '../../src/lib/render/engine-job';
import type { PlannedRender } from '../../src/lib/render/job-plan';
import { sliceCount } from '../../src/render/pcm';
import type {
  MathGroupInput,
  MathGroupOutput,
  PageJobInfo,
  PageResult,
  PageStatus,
  StartArgs,
} from '../../src/render/protocol';
import {
  CALL_TIMEOUT_MS,
  Chromium,
  POLL_MS,
  RENDER_TIMEOUT_MS,
  TRANSFER_TIMEOUT_MS,
  type RenderPage,
} from './chromium';
import type { JobSource } from './job-source';
import {
  assembleSlices,
  decodeChannel,
  HarnessError,
  mapBatched,
  RendererStalledError,
  withTimeout,
} from './transfer';

/** Channels transferred at once. */
const CHANNEL_BATCH = 2;

export class SuperdoughRenderer implements Renderer, JobSource {
  readonly engine = 'superdough' as const;
  private readonly chromium = new Chromium();

  /** The browser's version (opens it if needed); the Playwright pin fixes it. */
  version(): Promise<string> {
    return this.chromium.version();
  }

  async schedule(req: RenderRequest): Promise<ScheduleResult> {
    return this.withPage(await this.args('schedule', req), async ({ page, errors }) => {
      const result = await withTimeout(
        page.evaluate(() => (window as any).strudelandRender.schedule() as ScheduleResult),
        CALL_TIMEOUT_MS,
        'schedule result',
      );
      return { ...result, errors: [...result.errors, ...errors] };
    });
  }

  async render(req: RenderRequest): Promise<RenderResult> {
    const kickoff = performance.now();
    return this.withPage(await this.args('render', req), async ({ page, errors }) => {
      const meta = await withTimeout(
        page.evaluate(() => (window as any).strudelandRender.result() as PageResult),
        CALL_TIMEOUT_MS,
        'render result',
      );
      // A few channels at a time: each is a large base64 string in flight.
      const indices = Array.from({ length: meta.channelCount }, (_, i) => i);
      const channels = await mapBatched(indices, CHANNEL_BATCH, async (i) =>
        decodeChannel(
          await withTimeout(
            page.evaluate((index) => (window as any).strudelandRender.channel(index) as string, i),
            TRANSFER_TIMEOUT_MS,
            `channel ${i}`,
          ),
        ),
      );
      const { channelCount: _, ...rest } = meta;
      const allErrors = [...meta.errors, ...errors];
      return {
        ...rest,
        channels,
        errors: allErrors,
        ok: allErrors.length === 0,
        timing: { ...meta.timing, totalMs: performance.now() - kickoff },
      };
    });
  }

  /** The engine job for `req` (page kind `job`): the unmutated job, fetched in slices, and its
   *  `PageJobInfo` with the page listener's errors appended to `errors`. */
  /** The page's engine job for `req`: parsed, and `text`, the JSON exactly as the page wrote it
   *  (the harness writes that to disk rather than re-serializing `job`). */
  async job(req: RenderRequest): Promise<{ job: EngineJob; text: string; info: PageJobInfo }> {
    return this.withPage(await this.args('job', req), async ({ page, errors }) => {
      const info = await withTimeout(
        page.evaluate(() => (window as any).strudelandRender.job() as PageJobInfo),
        CALL_TIMEOUT_MS,
        'job info',
      );
      const slices: string[] = [];
      for (let i = 0; i < sliceCount(info.jsonLength); i++) {
        slices.push(
          await withTimeout(
            page.evaluate((index) => (window as any).strudelandRender.jobSlice(index) as string, i),
            TRANSFER_TIMEOUT_MS,
            `job slice ${i}`,
          ),
        );
      }
      const json = assembleSlices(slices);
      if (json.length !== info.jsonLength) {
        throw new HarnessError(
          'render-failed',
          `job JSON is ${json.length} characters, the page announced ${info.jsonLength}`,
        );
      }
      const job = JSON.parse(json) as EngineJob;
      return { job, text: json, info: { ...info, errors: [...info.errors, ...errors] } };
    });
  }

  async planRender(
    code: string,
    request: RenderPlanInput,
    samples?: RenderRequest['samples'],
  ): Promise<PlannedRender> {
    await this.chromium.open();
    return this.withPage(
      { kind: 'plan', code, request, samples, samplesBase: `${this.chromium.origin}/samples/` },
      async ({ page, errors }) => {
        const plan = await withTimeout(
          page.evaluate(() => (window as any).strudelandRender.plannedRender() as PlannedRender),
          TRANSFER_TIMEOUT_MS,
          'batch plan',
        );
        return { ...plan, errors: [...plan.errors, ...errors] };
      },
    );
  }

  /** V8's `Math.*` over `groups`, evaluated in an AudioWorklet of a fresh page (kind `math`). */
  async math(groups: MathGroupInput[]): Promise<MathGroupOutput[]> {
    return this.withPage({ kind: 'math', groups }, async ({ page }) =>
      withTimeout(
        page.evaluate(() => (window as any).strudelandRender.math() as MathGroupOutput[]),
        TRANSFER_TIMEOUT_MS,
        'math outputs',
      ),
    );
  }

  async dispose(): Promise<void> {
    await this.chromium.close();
  }

  /** The start arguments of a song page (opens the browser and server: `origin` needs them). */
  private async args(
    kind: 'render' | 'schedule' | 'job',
    request: RenderRequest,
  ): Promise<StartArgs> {
    await this.chromium.open();
    return { kind, request, samplesBase: `${this.chromium.origin}/samples/` };
  }

  /** Steps shared by every call: fresh page, kickoff, wait until done; the page always closes. */
  private async withPage<T>(args: StartArgs, read: (p: RenderPage) => Promise<T>): Promise<T> {
    const rp = await this.chromium.newPage();
    try {
      await withTimeout(
        rp.page.evaluate((a) => (window as any).strudelandRender.start(a), args),
        CALL_TIMEOUT_MS,
        'render kickoff',
      );
      await this.waitDone(rp.page);
      return await read(rp);
    } finally {
      await withTimeout(rp.page.close(), CALL_TIMEOUT_MS, 'page close').catch(() => {});
    }
  }

  /** Waits (polling inside the page) until the page is done or failed; a page still running
   *  after RENDER_TIMEOUT_MS is a stall. */
  private async waitDone(page: Page): Promise<void> {
    const status = () =>
      withTimeout(
        page.evaluate(() => (window as any).strudelandRender.status() as PageStatus),
        CALL_TIMEOUT_MS,
        'render status',
      );
    try {
      await page.waitForFunction(
        () => {
          const s = (window as any).strudelandRender.status() as PageStatus;
          return s.done || s.failed;
        },
        null,
        { polling: POLL_MS, timeout: RENDER_TIMEOUT_MS },
      );
    } catch (e) {
      if (!(e instanceof errors.TimeoutError)) throw e;
      const s = await status();
      throw new RendererStalledError(
        `render did not finish within ${RENDER_TIMEOUT_MS} ms (phase ${s.phase})`,
      );
    }
    const s = await status();
    if (s.failed) {
      const phase = s.failedPhase ?? s.phase;
      throw new HarnessError(
        phase === 'evaluating' ? 'eval-failed' : 'render-failed',
        `render failed in ${phase}: ${s.error}`,
      );
    }
  }
}

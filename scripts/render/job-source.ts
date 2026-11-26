/**
 * Where the Rust renderer gets its engine jobs: the render page in Chromium
 * (`SuperdoughRenderer`) or the headless host binary (`HostPlanner`: one single-chunk
 * `plan-render` process per job). Type-only: no Playwright.
 */

import type { RenderRequest } from '../../src/lib/render/contract';
import type { EngineJob } from '../../src/lib/render/engine-job';
import type { PageJobInfo } from '../../src/lib/render/page-job';

export interface JobSource {
  /** The engine job for `req` (code, layout, window, lookback, mode): parsed (a source may parse
   *  it on first read of `job`), the exact text it was written as, and the plan info. Mutations
   *  are not applied here. */
  job(req: RenderRequest): Promise<{ job: EngineJob; text: string; info: PageJobInfo }>;
  /** A version string for the report (Chromium's for the page, `strudeland-host <crate version>
   *  (<git sha>[-dirty])` for the host). */
  version(): Promise<string>;
  dispose(): Promise<void>;
}

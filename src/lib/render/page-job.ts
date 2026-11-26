/**
 * A planned engine job as the render page and the headless host serve it: the job's ASCII JSON
 * and its `PageJobInfo` (`pageJobInfo` for a single-chunk plan), plus the error bookkeeping
 * both share. Pure: type-only imports.
 */

import type { RenderError } from './contract';
import type { PlannedJob } from './engine-job';
import type { PlannedRender } from './job-plan';
import type { WindowPlan } from './plan';
import type { RenderEvent } from './schedule';
import type { SongStructure } from '../strudel/structure';

/** What the driver learns about a job without its (large) JSON, which it fetches with
 *  `jobSlice`. */
export interface PageJobInfo {
  /** Characters of the job's JSON (ASCII only, see `asciiJson`). */
  jsonLength: number;
  /** The chunk's unmutated events (equal to `ScheduleResult.events` when fromBar is 0 and
   *  lookbackSec is 0); a hap's `event` indexes them. */
  events: RenderEvent[];
  /** Deduped `selectHaps` errors of both selections, plus the captured page errors. */
  errors: RenderError[];
  plan: WindowPlan;
  cps: number;
  structure: SongStructure;
  stemPairs: number;
}

/** Errors without repeats of the same kind and message, first occurrence kept, in order. */
export function dedupeErrors(errors: readonly RenderError[]): RenderError[] {
  const seen = new Set<string>();
  return errors.filter((e) => {
    const key = `${e.kind}\u0000${e.message}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** `JSON.stringify(x)` with every non-ASCII character escaped (`\uXXXX`): it parses to the same
 *  value, and slicing it anywhere never splits a surrogate pair across two messages. */
export function asciiJson(x: unknown): string {
  return JSON.stringify(x).replace(
    /[\u007f-\uffff]/g,
    (c) => '\\u' + c.charCodeAt(0).toString(16).padStart(4, '0'),
  );
}

/** The page's job state: the job's ASCII JSON (served by `jobSlice`) and its `PageJobInfo`. */
export function pageJob(
  planned: PlannedJob,
  rest: Pick<PageJobInfo, 'errors' | 'plan' | 'cps' | 'structure'>,
): { info: PageJobInfo; json: string } {
  const json = asciiJson(planned.job);
  return {
    json,
    info: {
      jsonLength: json.length,
      events: planned.events,
      errors: rest.errors,
      plan: rest.plan,
      cps: rest.cps,
      structure: rest.structure,
      stemPairs: planned.stemPairs,
    },
  };
}

/** The `PageJobInfo` of a single-chunk plan (`RenderPlanRequest.chunk`): the one chunk's job
 *  length, events, plan and stem pairs, with the plan's errors, tempo and structure. Throws
 *  unless the plan has exactly one chunk with its `info`. */
export function pageJobInfo(planned: PlannedRender): PageJobInfo {
  const chunk = planned.chunks.length === 1 ? planned.chunks[0] : undefined;
  if (!chunk?.info) {
    throw new Error(`not a single-chunk plan (${planned.chunks.length} chunk(s))`);
  }
  return {
    jsonLength: chunk.json.length,
    events: chunk.info.events,
    errors: planned.errors,
    plan: chunk.plan,
    cps: planned.cps,
    structure: planned.structure,
    stemPairs: chunk.info.stemPairs,
  };
}

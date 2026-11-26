# 003: the batch window as a renderer capability

## Goal

`render:check --planner batch` takes each fixture's main window from a renderer that can render
a whole window, through the `Renderer` contract, instead of a `CheckDeps.batchWindow` hook that
overwrites `fx.assembled`.

- Add an optional `Renderer.renderWindow(base, events)`.
- `PipelineRenderer` wraps the host-planned `RustRenderer` and the `strudeland-render` process,
  and implements the method.
- `assembledOf` uses it when the renderer has it.
- Deleted: `CheckDeps.batchWindow`, `createBatchWindow` and the overwrite at `check.ts:618-621`.

**Nothing the harness renders or reports changes.** The same `strudeland-render` invocation
produces the same window, cross-checked the same way. Only the batch output file names change,
from the fixture name to a song hash. No planner file changes, so the engine refs do not
change.

## Prerequisites

- Stage 002 is committed and its full gate is green.
- Read [the proposal](../0124-one-planner.md) "Stage 003".
- Read `scripts/render/check.ts`:
  - the module doc;
  - `CheckDeps` (`:122-147`);
  - `assembledOf` (`:292-301`);
  - `prepareFixture` (`:493-551`);
  - the fixture flow (`:600-625`);
  - `createRenderer` (`:967-1017`) and `createBatchWindow` (`:1022-1046`);
  - `main` (`:1048-1139`).
- Read `scripts/render/batch-render.ts` and `src/lib/render/batch-window.ts`, as stage 001 left
  them.

## Step 1: the contract

In `src/lib/render/contract.ts`:

- `Renderer` gains `renderWindow?(base: RenderRequest, events: RenderEvent[]): Promise<AssembledWindow>`,
  with the proposal's doc comment.
- Add `import type { AssembledWindow } from './chunks';`. It is type-only, so `contract.ts`
  stays free of runtime imports. Update the module doc's "no imports but types" sentence if
  needed.
- `AssembledWindow` stays in `chunks.ts`.

`contract.ts` and `chunks.ts` are host inputs (`crates/strudeland-host/js/host.inputs.json`),
so this stage runs `host:build`. The bundle's bytes should not change, because the edits are
type-only. If `host.js` itself changes, **stop** and find out why.

## Step 2: `PipelineRenderer`

In `scripts/render/batch-render.ts`, add next to `BatchRenderer`:

```ts
/** `--planner batch`'s renderer: the host-planned `RustRenderer` for every request, and the whole
 *  Rust pipeline (`strudeland-render render`) for a fixture's main window (`renderWindow`). */
export class PipelineRenderer implements Renderer {
  readonly engine = 'rust' as const;
  constructor(o: { inner: RustRenderer; batch: BatchRenderer; outDir: string; renderVersion: string });
  schedule(req): Promise<ScheduleResult>;  // inner.schedule
  render(req): Promise<RenderResult>;      // inner.render
  version(): Promise<string>;              // inner.version
  get rustVersion(): string;               // inner.rustVersion
  readonly renderVersion: string;          // the strudeland-render build, for the run header
  renderWindow(base, events): Promise<AssembledWindow>;
  dispose(): Promise<void>;                // inner.dispose
}
```

`renderWindow(base, events)` runs these steps:

1. `const sched = await this.o.inner.schedule(base)`. This is the cached full-window job:
   `prepareFixture` already asked for it with the same `base`, and `RustRenderer` caches by
   `jobKey`.
2. `const timing = renderTiming(sched.structure, layoutSegments(base.layout, sched.structure), sched.cps)`.
   That is the derivation at `check.ts:518`, so it equals `fx.timing`.
3. `const songPath = join(this.o.outDir, 'batch', `${sha256(base.code)}.js`)`, then
   `writeSongFile(songPath, base.code)`. `writeSongFile` comes from `host-planner.ts`.
4. `return this.o.batch.window({ name: `${sha256(base.code).slice(0, 8)}-${base.fromBar}-${base.toBar}-${base.layout}`, songPath, base, cps: sched.cps, events, timing })`.
   The name is the stem of the WAV and `--info` files (`batch-render.ts:123-124`). The layout
   is in it because one song can be rendered in both layouts at the same bars (`song-linear`
   and `song-clip` are one song file), and the two must not overwrite each other.

`BatchRenderer`, `BatchTarget`, `batchRenderArgs`, the outcomes and the cross-checks do not
change. Update the module doc: `PipelineRenderer` is how `render:check` uses `BatchRenderer`.

## Step 3: `check.ts`

1. **`assembledOf(fx, tx, batch)`:**

   ```ts
   if (tx === undefined && fx.assembled) return fx.assembled;
   const r = fx.renderer;
   const a =
     tx === undefined && r.renderWindow
       ? await fx.limit.run(() => r.renderWindow!(fx.base, fx.sched.events))
       : assembleWindow(await chunkResults(fx, tx, batch), fx.sched.events);
   if (tx === undefined) fx.assembled = a;
   return a;
   ```

   With a mutation (`tx`), the window is still the joined chunk renders.
2. **The fixture flow** (`:618-621`) becomes
   `const assembled = assembledOf(fx, undefined, batch);`. Delete the `ctx` and `batchWindow`
   locals.
3. **Delete** `CheckDeps.batchWindow` and its doc, and `createBatchWindow`.
4. **`createRenderer`.** When `opts.planner === 'batch'`, it builds the render binary as
   `createBatchWindow` did: the same console lines, and `findRenderBinary`. It returns
   `new PipelineRenderer({ inner: rustRenderer, batch: new BatchRenderer({ bin, outDir }), outDir, renderVersion: version })`.
   Its return type gains `renderVersion: string | null`.
5. **`main`** reads `renderVersion` from `createRenderer` for the header line (`, render …`),
   and drops the `batchWindow` and `batchVersion` locals.
6. **The module doc.** `--planner batch` is `host` with a `PipelineRenderer`, whose
   `renderWindow` gives each fixture's main window. That window feeds the mix and bass gates,
   the parity features and `--keep-wav`. Twin, seam, stems and the mutations keep the
   per-request host plans.

## Step 4: tests (replace, don't layer)

| File | Change |
|---|---|
| `tests/render-check-flow.test.ts`, `describe('runCheck --planner batch')` (`:1059`) | Delete `batchOf`. Each case passes a fake renderer: the existing `FakeRenderer` plus a `renderWindow` that records `(base, events)` and returns the window `batchOf` returned. The assertions stay, with "the batch hook" reworded as "`renderWindow`": it is called once per fixture with `fx.base` and the schedule events, the main-window gates and features read its window, mutations still join chunk renders, and a `renderWindow` failure is the fixture's error. Add one case: a renderer without `renderWindow` joins chunks (the superdough and host paths). |
| A new `PipelineRenderer` block in `tests/render-batch-render.test.ts` | It uses a fake `inner` (a `schedule` returning a known `ScheduleResult` with a mixed-meter structure, plus spies) and a fake runner. It asserts that the song file is written at `<outDir>/batch/<sha>.js` with the song's text; that the output name is `<sha8>-<from>-<to>-<layout>`, so the same song in the two layouts writes two files; that the argument list is `batchRenderArgs(songPath, base, …)`; that the `timing` passed on equals `renderTiming(…)` of the schedule's structure (compare `secondsAt` at a few bars); that `schedule`, `render`, `version` and `dispose` delegate; and that a cross-check failure is a `RenderFailedError`. |
| `tests/render-batch-window.test.ts` | Unchanged. |

## Step 5: docs

- **`AGENTS.md`.**
  - The `render:check` row: `--planner batch` "plans like `host` and renders each fixture's
    main window through the whole Rust pipeline (a `PipelineRenderer`: `strudeland-render`,
    cargo-built, or `STRUDELAND_RENDER_BIN`)".
  - The render pipeline layout bullet: `scripts/render/batch-render.ts` holds `BatchRenderer`
    and `PipelineRenderer`, which is the `Renderer` with `renderWindow`.
- **`crates/strudeland-render/README.md`.** If it describes the batch planner's hook, line 38
  now says that `render:check --planner batch` renders each main window through it.

## Step 6: the full gate

1. Run `bun run format`.
2. Run the full gate in [000-prompt.md](000-prompt.md): steps 1 and 6-13. Steps 2-5 do not
   apply, because no planner file changes.
   - **Step 1** (`host:build`) may rewrite `host.inputs.json`. `host.js` must not change:
     `git diff --quiet -- crates/strudeland-host/js/host.js`.
   - As a cross-check, run `L refs-check bun run engine:refs`. It must report `refs are
     current`. Any difference is a **stop**.
   - **Step 13**: `equal`. The `rust-batch` mode's kept WAVs, including the
     `strudeland-render` outputs under their new names, compare as a sorted hash list, so a
     rename is not a difference.
3. Confirm that the hook is gone:

   ```sh
   git grep -n 'batchWindow\|createBatchWindow' -- scripts src tests
   ```

   It prints nothing.

## STOP conditions

Stop and report to the user, changing nothing to absorb it, on any of these:

- any `reports-check` or `ab.sh check` difference;
- a change to `host.js`'s bytes;
- `engine:refs` reporting stale refs;
- a gate that reads a different main window: a different `fx.assembled` source for any mode
  other than batch;
- a need to change a threshold, golden or `SKILL.md`.

## Commit message

```
refactor(render): render the batch main window through a renderer capability

Renderer gains an optional renderWindow(base, events) that renders a whole
window through the renderer's own production path. PipelineRenderer wraps
the host-planned Rust renderer and the strudeland-render process and
implements it; render:check takes a fixture's main window from it when the
renderer has one, so the batchWindow hook that overwrote fx.assembled is
gone. Mutations still join chunk renders, and the batch window, its
cross-checks and every gate are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

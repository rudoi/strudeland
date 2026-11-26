# 0124 loss check

The three stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| A/B guard over 96 cases (corpus windows, song presets, `--loops`, sections, stems, mixed meter, the scratch `frac-737`), recorded twice; the four `render:check` report baselines; the provenance-only refs rule | 001 (before any edit) |
| Frame fields always in the plan; the Rust joiner only checks sums; `overlap_frames` on `ChunkWindow`; `batch-window.ts` reads them from `info`; `joiner.json` gains `contentFrames` | 001 |
| `planJob` deleted; the single-chunk `chunk` request runs through `planRender`; `PlannedChunk.info`; `BATCH_REUSE_SCHEDULE` and `reuseSchedule` deleted | 002 |
| Rust `plan_job*`, `Work::Plan`, `PlannedJob`, `RenderRequest` and the bin's `plan` subcommand deleted; `HostPlanner.job`, bench, `perf_budget` and the live-pressure recipe moved to `plan-render` | 002 |
| `host:parity`: per-chunk `--batch` kept plus a whole-plan check; non-batch compares `info` without errors; `--mixed-meter` adds single-chunk requests | 002 |
| Tests ported (validation messages, never_panics, `joiner_identity`); one single-chunk-equals-`planRender` test | 002 |
| `Renderer.renderWindow`, `PipelineRenderer`; `CheckDeps.batchWindow` and the `fx.assembled` overwrite deleted | 003 |
| AGENTS.md command table, the host and render READMEs | 001-003 |
| Ride-alongs: #151 (the `schedule.ts` comment), #66 item 1 (`isRecord`) | 001, 002 |
| No audio, job-text, threshold, golden, CLI or SKILL.md change | every stage (A/B, reports, refs and vector guards) |

# 0116: drop the vestigial render plan request fields; one length validator

## Goal

The render plan request carries only what the planner reads. The length request is validated in
one place: the TS planner, which every planner path runs. Every user-facing output is unchanged.

## Background

The [re-verification](0116-render-request-cleanup/reverify.md) has the evidence.

- **`RenderPlanRequest.mode` and `sampleRate` each accept exactly one value.** Both languages
  validate them, and nothing reads them.
- **Loops, bars and window are validated twice, with different messages and error kinds.**
  Users never see the Rust copy: the CLI's clap ranges and the app's export check come first.

## Scope

In:

1. **Remove `mode` and `sampleRate` from `RenderPlanRequest`.**
   - Rust: `crates/strudeland-host/src/request.rs`.
   - TS: `src/lib/render/job-plan.ts`.
   - Every in-tree sender: `src/lib/render/host-parity-lib.ts`, `scripts/render/host-parity.ts`,
     `crates/strudeland-render/src/plan.rs`, and the tests.
   - The single-job `RenderRequest` and its `Mode` (mix or stems) are unchanged.
2. **Rust `RenderPlanRequest::validate()` keeps only what the types and the host need:** the
   instrument snapshot and `timeoutSec`.
   - The loops, bars and window checks go.
   - `MAX_LOOPS` goes from `strudeland-host` (and its re-export in `lib.rs`).
   - TS `validateRenderRequest` stays the single length validator, minus its mode and sampleRate
     checks.
3. **A bad length request is a "bad request:", and the dev binaries keep exit 2 for it,** the
   bad-input code AGENTS.md documents.
   - **The TS planner reports a bad `--loops` value as a bad request.** Today `LOOPS_MESSAGE` is
     a flagged `meta` error (`job-plan.ts:342`). It becomes `badRequest` (kind `plan`,
     `bad request: --loops must be …`), like bars and window already are.
   - **Song-dependent length errors stay flagged `meta` errors,** with their exit codes
     unchanged. These are "no sections", "unknown section", "past the end", "--loops needs
     meta.loop" and "past the render limit". A `MetaFlag` can't tell a bad value from a song that
     can't satisfy the request, since the same `--loops` flag names both.
   - **`strudeland-host plan-render`** maps only a `HostError::Plan` whose message starts with
     `bad request:` to bad input (exit 2). `MetaFlag` stays a host error (exit 1, pinned by
     `tests/bin.rs:304-320`).
   - **`strudeland-render`** already maps `MetaFlag` to exit 2 through the shared `exit_class`.
     Its binary's `From<RenderError> for Failure` also maps a `bad request:` plan error to exit 2.
     The shared `exit_class` in `error.rs`, which the CLI's exit 4 and `pipeline.rs` rely on,
     stays untouched.
   - **The user-facing CLI is unaffected:** clap's ranges reject a bad `--loops` before the
     planner sees it.
4. **Docs:** `crates/strudeland-host/README.md`'s wire example.

Out:

- **A shared clap `LengthArgs` for the CLI and the harness.** It would save about 7 lines and
  risks the CLI's `--help` text.
- **The CLI's `Preset` mirror.** It stays: the host crate is clap-free, and the orphan rule
  applies.
- **`src-tauri/src/export_audio.rs`'s early length check.** It stays as an app UI guard.
- **The TS export controller's form-to-request mapping.** Its input is different.

## Technical design

### Wire contract

`RenderPlanRequest = { length, preset?, includeEvents?, instruments?, timeoutSec? }`.

The struct keeps `deny_unknown_fields`. A request that still carries `mode` or `sampleRate` is
rejected as bad input. This is a deliberate hard break; every sender is in this repo.

### Rust

- **`request.rs`:**
  - delete the two fields, the length arms of `validate()` and `MAX_LOOPS`;
  - update the doc comments (`:205-208`, `:241-244`, and `host.rs:143-147`).
- **`strudeland-render/src/plan.rs`:** build the request without `Mode` and `SAMPLE_RATE`.
- **Exit mapping, as in Scope item 3:**
  - `strudeland-host.rs` `host_error`: only `Plan("bad request: …")` goes to `Exit::BadInput`.
  - `strudeland-render.rs` `From<RenderError> for Failure`: the same.
  - `error.rs` `exit_class` and the CLI's `run.rs` are unchanged.
- **Remove the now-unused imports** (`Mode`, `SAMPLE_RATE` in `strudeland-render/src/plan.rs`),
  and check `joiner_identity.rs` and `never_panics.rs`.
- **Every `RenderPlanRequest` literal loses the two fields.** These are listed by the refine
  review:
  - `crates/strudeland-host/tests/plan_render.rs`: `:16-26`, `:135`, `:182`, `:326-342`,
    `:355`, `:379`, `:393`, `:412`, `:464`, `:494`, `:515`;
  - `crates/strudeland-host/tests/never_panics.rs`: `:10-18`, `:145-175`;
  - `crates/strudeland-host/tests/bin.rs`: `:251`, `:310`, `:324`, `:344-360`;
  - `request.rs` tests: `:476`, `:491`, `:496`, `:536`, `:544`, `:550`, `:583-590`, `:649`,
    `:678`, `:705`, `:706`, `:721`;
  - `crates/strudeland-render/tests/joiner_identity.rs:36`.

  Single-job `RenderRequest` literals stay: `host.rs:49` and `:217`, `perf_budget.rs`,
  `common/mod.rs` and `bench.rs`.
- **Docs:**
  - `strudeland-host.rs:17-21` (the module doc and wire example);
  - `request.rs:3-5`;
  - the README wire example (`:56-58`).
  - The exit sentences don't change: song errors stay exit 1 in `strudeland-host`, and bad
    requests were already exit 2.

### TS

- **`job-plan.ts`:**
  - drop the two fields from the type;
  - drop their checks in `validateRenderRequest`;
  - drop the comment that says the host checks the same rules first.
- **`host-parity-lib.ts` and `scripts/render/host-parity.ts`:** drop the fields.

### Tests (replace, don't layer)

- **`request.rs` unit tests:**
  - drop the mode, sampleRate and length cases;
  - add "a request with `mode` is rejected as an unknown field".
- **`crates/strudeland-host/tests/plan_render.rs`:** the length cases now expect the bundle's
  `MetaFlag`/`Plan("bad request: …")`. The mode cases go.
- **`tests/bin.rs`:** bad bars, loops and window still exit 2, now through the new mapping.
- **`tests/never_panics.rs`:** drop `sample_rate: 0`, and widen to `Plan | MetaFlag`.
- **`crates/strudeland-render/tests/joiner_identity.rs`:** the constructor changes.
- **`crates/strudeland-render` harness tests:** `--loops 0` and a bad window exit 2.
- **`tests/render-job-plan.test.ts`:** drop the mode and sampleRate rows; the length rows stay.
- **`tests/host-entry.test.ts`:**
  - the `renderRequest` helper (`:188`) drops the fields;
  - the error rows at `:311-315` lose the `mode: stems` and `sampleRate` cases;
  - the `{}` row's needle becomes `'length'`.
- **`tests/render-host-parity.test.ts:158-164`:** drop the fields.
- **`tests/render-job-plan.test.ts:679-700`:** the `--loops 0`, `1001`, `1.5` and `NaN` rows now
  expect `plan` with `bad request: --loops must be …`. Check `crates/strudeland-render/tests/pipeline.rs:341`
  for the same expectation, and update it if it reaches the planner.
- **"invalid request:" prefix tests** (`tests/plan_render.rs:349`, `tests/host.rs:223`) switch to
  a `timeoutSec` or snapshot case to keep the prefix.
- **Every bad-input test JSON must lose `mode`.** Otherwise `deny_unknown_fields` rejects it, and
  it passes for the wrong reason.

### Regeneration (AGENTS.md)

- **`bun run host:build`:** `job-plan.ts` and `contract.ts` are bundled.
- **`bun run engine:refs --write`:** `job-plan.ts` is in `PLANNER_FILES`. Needs Chromium. The
  diff must be provenance only (`plannerSha256` and index hashes). The measured-floor cases are
  re-rendered as usual.
- **`bun run host:parity`, `host:parity --batch` and `host:parity --mixed-meter`.** Each needs
  Chromium, and each must be byte-equal.
- **`cargo test -p strudeland-host`, `-p strudeland-render` and `--workspace`.**
- **Not needed:**
  - `render:vectors`: `loudness.ts` and `chunks.ts` are untouched.
  - `render:check`: job bytes are unchanged, as `host:parity` proves.

## Dependencies

- **Blocks:** #117 (shared export module), #124 (one planner).

## Risks

- **A missed sender fails hard,** because of `deny_unknown_fields`. Grep every plan-render
  request builder.
- **The refs diff could hide a real change.** Check it is only provenance.
- **Clippy can fail on unused imports.**
- **Test JSON that still carries `mode` passes spuriously.** Strip it from every plan-request
  literal.

## Complexity

Small to medium. About 15 files, two of them regenerated. Fracture and Singletonize are skipped;
the Loss check is light.

## Decisions

1. **Remove the fields only from `RenderPlanRequest`.** The single-job `RenderRequest`'s `mode`
   (live harness stems) stays.
2. **A hard break on the TS↔Rust plan request.** No shim; every sender is in-tree.
3. **A bad length request exits 2 in both dev binaries,** keeping AGENTS.md's bad-input
   contract. The TS planner reports a bad `--loops` value as a bad request, and only bad-request
   plan errors map to 2. Song-dependent flagged errors keep their codes. (The refine review
   corrected the rule from "every MetaFlag".)
4. **No shared `LengthArgs`.** It fails the deletion test for its payoff.
5. **The CLI's `Preset` mirror stays.**
6. **Delete `MAX_LOOPS` from `strudeland-host`.** TS `structure.ts`'s `MAX_LOOPS` is the
   authority.

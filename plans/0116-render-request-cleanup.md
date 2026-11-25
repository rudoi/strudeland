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
3. **The dev binaries keep exit 2 for a bad length request,** the bad-input code AGENTS.md
   documents.
   - `strudeland-host plan-render` maps a bundle `MetaFlag`, or a `Plan` error starting with
     `bad request:`, to bad input (exit 2).
   - `strudeland-render` maps the same to exit 2. Today `--loops 0` there exits 1, so this makes
     it consistent.
   - The error texts become the TS planner's. These are dev harness binaries, not `strudeland`
     subcommands.
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
- **Exit mapping:**
  - `crates/strudeland-host/src/bin/strudeland-host.rs` `host_error` (and `read_plan_request`):
    a `HostError::MetaFlag`, or a `HostError::Plan` whose message starts with `bad request:`, maps
    to `Exit::BadInput` (2).
  - `crates/strudeland-render/src/bin/strudeland-render.rs`: the same rule, mapped to exit 2.
  - The CLI's own mapping (`run.rs`) is unchanged.

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
- **`tests/host-entry.test.ts` and `tests/render-host-parity.test.ts`:** drop the fields.

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

## Complexity

Small to medium. About 15 files, two of them regenerated. Fracture and Singletonize are skipped;
the Loss check is light.

## Decisions

1. **Remove the fields only from `RenderPlanRequest`.** The single-job `RenderRequest`'s `mode`
   (live harness stems) stays.
2. **A hard break on the TS↔Rust plan request.** No shim; every sender is in-tree.
3. **A bad length request exits 2 in both dev binaries,** keeping AGENTS.md's bad-input
   contract. The error text becomes the TS planner's.
4. **No shared `LengthArgs`.** It fails the deletion test for its payoff.
5. **The CLI's `Preset` mirror stays.**
6. **Delete `MAX_LOOPS` from `strudeland-host`.** TS `structure.ts`'s `MAX_LOOPS` is the
   authority.

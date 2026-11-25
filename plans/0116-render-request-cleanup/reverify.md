# 0116 re-verification (step 0)

Checked on `main` at 7216aad.

## Confirmed

- **`RenderPlanRequest` carries `mode` and `sampleRate`,** in Rust and in TS. Each accepts one
  value and is checked in both languages:
  - Rust: `crates/strudeland-host/src/request.rs:192-218`, checked in `validate()` at `:248`
    and `:251`.
  - TS: `src/lib/render/job-plan.ts:245-253`, checked in `validateRenderRequest` at `:315-317`.
- **`planRender` never reads either field.** The per-chunk base request hard-codes
  `mode: 'mix'` (`job-plan.ts:580-589`). Removing them leaves job bytes unchanged.
- **Loops, bars and window are validated in both languages.**
  - Rust: `request.rs:258-273`.
  - TS: `job-plan.ts:339-361`.
  - Only the loops message matches. Rust reports a bad loops value as `HostError::Plan`, with
    exit class Failed; TS reports it as `MetaFlag`, with exit class Usage.

## Corrected from the review

- **"stems" is not dead.** The single-job `RenderRequest` (`request.rs:98-107`,
  `contract.ts:38-39`) has a real `mode: 'stems'`, which the harness stem gates use. `Mode`,
  `SAMPLE_RATE` and the single-job fields stay. Only the plan request's two fields go.
- **The `strudeland render` CLI never reaches the Rust length checks.**
  - clap enforces `--length 1..=100000`, `--loops 1..=1000` and the conflicts (`cli.rs:161-174`).
  - Its exit 4 comes from `MetaFlag` or `Usage`.
  - App export has its own early UI check (`src-tauri/src/export_audio.rs:169-179`).
- **Two dev binaries do reach the Rust length checks:** `strudeland-render` and
  `strudeland-host plan-render`.
- **A shared flags-to-`LengthRequest` constructor fails the deletion test.** The CLI's version
  is 7 lines. The harness's complexity is its window flags. The TS export controller builds the
  request from a form, not from flags.
- **The CLI's `Preset` mirror is a legitimate adapter.** The host crate has no clap, and the
  orphan rule blocks deriving `ValueEnum` on its types.

## Persisted bytes

- **No committed golden, vector, `--info` file or ref embeds the plan request.**
- **Refs change only in provenance.** `job-plan.ts` is in `PLANNER_FILES`, so `plannerSha256`
  changes.
- **`host.js` must be rebuilt.**

## Conclusion

Worth doing, with a narrower scope.

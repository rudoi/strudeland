# 0117 re-verification (step 0)

Checked on `main` at 8fc7ad8, after #116.

## Confirmed

Every duplication the review named still holds. Its line numbers moved only where noted.

- **Preset to file-role mapping, four copies:**
  - CLI: `crates/strudeland-cli/src/render.rs:157-170`;
  - app: `src-tauri/src/export_audio.rs:202-206`;
  - `export_suffix`: `crates/strudeland-render/src/output.rs:124-147`;
  - `Part` roles: `crates/strudeland-render/src/export.rs:110-140`.
- **"Loop presets take no length", four copies with different wording:**
  - CLI: `render.rs:146-153`, a usage error naming `--preset`, exit 4;
  - app: `export_audio.rs:169-176`;
  - planner: `job-plan.ts:320-330`, unreachable behind both adapters;
  - UI: `src/lib/export/controller.ts:134`.
- **Stem limits are checked twice:** CLI `stem_options` (`render.rs:189-231`) and the app
  (`export_audio.rs:186-188`). Both build `OutputOptions`, which `render_stems` never reads.
- **Three destination shapes:**
  - `OutputTarget` (`output.rs:40-59`, always replaces);
  - `StemTarget` (`stems.rs:53-121`, never replaces);
  - the app's `PreviewFile` and `ExportRequest::write` (`export_audio.rs:77-81`, `:272-292`),
    which rebuild the mix paths and their own create-or-replace rules.
- **Render, match by role, write in order, report a partial write, dedupe warnings:**
  - CLI: `ExportPlan::write`, `render.rs:328-389`, and `run.rs:479-525`;
  - app: `run`, `export_audio.rs:312-381`.
- **A parity test keeps the two in step:** `export_audio.rs:574-634`.
- **The planner's length errors name CLI flags,** and four of them reach the desktop dialog
  (`job-plan.ts:401`, `:415-419`, `:455`, `:473`).

## New since the review

- **#116 made the TS planner the single length validator.** The app's preview-time length check
  (`export_audio.rs:177-179`) is still useful, because preview never plans. It stays in the app.
- **The `strudeland-render` harness binary calls `write_output`** (`bin/strudeland-render.rs:219`).

## Conclusion

Worth doing, as two PRs:

- **PR A (this task):** a behaviour-identical shared export module.
- **PR B (#144):** typed planner errors worded per adapter, and no loop export without a marked
  loop. Both are user-approved.

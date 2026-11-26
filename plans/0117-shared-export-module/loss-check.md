# 0117 loss check (light)

Against the original proposal (commit 3282215) and the refine review's findings.

## Preserved

- **The goal and the hard constraint.** CLI output, flags and exit codes, app text and behaviour,
  file names and paths, overwrite semantics and rendered bytes are all identical.
- **Two PRs.** #144 keeps the typed planner errors and the loop export button.
- **The module owns the export request, destination, role list, preview, render, ordered publish
  and the partial-write error.** The adapters keep clap and terminal output, or IPC, the confirm
  dialog, progress and Application Quit.
- **The placement:** `export.rs` becomes `loop_export.rs` and `export/` is new, inside
  `strudeland-render` (`host-jitless`).
- **A Pipeline seam with a real adapter and a fake.** The CLI's tests move onto the fake.
- **Two replace policies:** `Always` for the CLI, `CreateNew` for the app. Atomic publishing
  stays in `strudeland-project`.
- **Out of scope:** #144's items, unifying the overwrite policy, the app's length-validity check,
  and planner files.
- **Tests:**
  - the {preset × format × destination} table;
  - the publish tests (`Always` overwrites, unapproved replace refused, competing file kept with
    `written`, changed folder refused, stems refuse an existing directory with the manifest last);
  - the CLI output tests and the app integration tests are kept;
  - the parity test is deleted.
- **The glossary terms:** export request, destination, role, stem set.
- **The risks:** Application Quit, no-clobber, byte reproducibility, CLI test churn, SKILL.md.

## Modified (justified)

- **`ExportPreset::roles()` → `export::roles(Option<ExportPreset>)`** (finding 1). `ExportPreset`
  lives in `strudeland-host`, so the orphan rule decides it. The `None` row is one `Single` file
  through `render()`. The request's kind is `Mix { preset: Option<ExportPreset>, output }`.
- **"`output.rs`'s resolvers become crate-private; `write_output` stays public" → a full
  public-surface table** (finding 2):
  - `OutputTarget`, `write_output`, `render_export` and `StemTarget` stay public;
  - `resolve_output`, `resolve_export_output` and `project_output` are deleted;
  - the stem resolvers, `render_stems` and `check_available` become `pub(crate)`.

  `tests/stems.rs` moves onto the module, `tests/pipeline.rs` and the harness binary need no
  edit, and every importer is listed. Privatising moved from "in scope" to its own last stage, so
  stages 001-003 keep every old item.
- **"Shared rules become typed `ExportProblem`s" → only the loop-preset/length rule**
  (finding 3). The CLI's stem conflicts and the app's stems-compat and validity checks have
  different orders and wording, and neither is shared. The behaviour table's "Conflicts" row is
  corrected: the app checks preset/length before validity, and the CLI's stems branch never checks
  preset/length.
- **`prepare` then `Prepared::preview()` → `prepare` (CLI: resolve only, primary role first, no
  stats) and `prepare_previewed` (app: per role, resolve then stat, in table order)** (findings 3
  and 4, and the CLI/app role-order nice-to-have). One `preview()` after a full resolution would
  report a later role's symlink error before an earlier role's non-regular-file error in the app.
  The two entry points keep each adapter's precedence, and a test pins both. `exists` still comes
  only from the app's preview path, and the CLI never stats `--out`.
- **`Prepared::run(policy, progress)` → `run(pipeline, code, policy, preview, instruments,
  progress)`** (findings 3 and 5). Instruments are snapshotted by each adapter after its
  destinations. `CreateNew` decides per file from the approved preview snapshot. The approval
  check is a typed `ReplacementNotApproved` before rendering. The preview comparison,
  `check_folder` and the permit stay in the app.
- **Errors: three kinds → one typed `Error`** with no `Display`, plus `RunError { Failed,
  PartialWrite }` (finding 6). Each adapter has its own wording table. The chosen-folder re-check
  before every file is the module's `check_chosen_folder` publish step. A first-file failure is
  `Failed`.
- **"The CLI keeps printing only the last written path" → the partial line is printed only for a
  mix `PartialWrite`** (A absolute, B raw). Stems print the error and the `wrote` lines; the
  module still returns their written paths for the app (finding 7).
- **"Warnings are deduped in first-seen order" → `Event::Written { file, new_warnings }`**
  (finding 8). The CLI keeps its `wrote`/`warning` interleaving and its per-path rules: preset
  across files, non-preset and stems as today. The app's list is `Finished.warnings`.
- **"Progress: `Publishing` before the first write, `Written(path)` per file" → a separate
  `Event` enum.** `Publishing` and `Written` are for mixes only, and `Progress` is untouched, so
  the exhaustive matches stay. `Written` is synchronous before the next file.
- **The song stem "from the adapter."** The CLI's comes from its `Project::find`, through
  `Destination::from_out`. The app's is derived by the module from `Request::song`, because its
  two wordings and the stems resolver's canonicalize-first order must be kept.
- **The test list** grew to pin each finding: the per-adapter order, no stats under `Always`,
  approval before render, snapshot-based create/replace, first-file versus partial, stems
  partials, the event order, warnings, options passthrough, `RolesMismatch`, and both wording
  tables.

## Added (from the review's nice-to-haves)

- **Naming:** the module's types are reached as `export::Request`, `export::Preview` and so on,
  with no root re-exports, so they do not collide with the app's `Export*` types.
- **`Pipeline` is public with a defaulted `publish`,** and `testing::FakePipeline` is doc-hidden,
  following the `strudeland-live` pattern. The fake keeps injecting write failures for the CLI's
  tests.
- **`Finished` carries per-file role, summary, loop points and both paths,** and `StemExport` for
  stems.
- **Destination semantics are kind-dependent and tabled.**
- **Staging:** 001 module, 002 CLI, 003 app, 004 privatise and docs. Each commit is green.

## Unintentionally dropped

- None.

## Known non-observable difference

- **The CLI treats any rendered role mismatch as `RolesMismatch`** ("unexpected intro file").
  Before, only an unexpected intro was refused. The real pipeline always returns the table's
  roles, so this cannot be observed. It is recorded in Risks.

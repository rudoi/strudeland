# 001: the export module

## Goal

Add `strudeland_render::export` exactly as the proposal's Technical design specifies, with
`RealPipeline`, `testing::FakePipeline` and its tests. Rename the loop arrangement to
`loop_export.rs`. No caller changes. Every existing public item keeps its name, path and
behaviour.

## Prerequisites

- **The branch is checked out at the plan commit,** with a clean tree.
- **Read the proposal's sections:** Interface, Entry points and order, Destination resolution,
  Run, Publish rule, Wording, and Fake.

## Implementation

1. **Rename the loop arrangement:**
   - `git mv crates/strudeland-render/src/export.rs crates/strudeland-render/src/loop_export.rs`.
   - In `lib.rs`, use `mod loop_export;` and `pub use loop_export::{ExportFile, ExportRole,
     LoopPoints, render_export};`.
   - Change any `crate::export::` path in the crate to `crate::loop_export::` (today only
     `lib.rs`).
2. **Add `lib.rs` items:** `pub mod export;` and:

   ```rust
   /// The fake pipeline, for this crate's tests and the CLI's.
   #[doc(hidden)]
   pub mod testing {
       pub use crate::export::fake::FakePipeline;
   }
   ```

3. **Create `src/export/`:**
   - **`mod.rs`:** declare `mod request; mod destination; mod publish; mod error; mod pipeline;
     pub(crate) mod fake;` and re-export every public name in the proposal's Interface block.
     Then `Prepared` (private fields: `request: Request`, `targets: Targets` where `enum Targets
     { Mix { files: Vec<Target>, place: Place }, Stems { grouping, target: StemTarget } }`),
     `prepare`, `prepare_previewed` and `Prepared::{targets, stem_directory, preset, run}`.
   - **`request.rs`:** `roles`, `ExportProblem`, `check_length_for_preset`, `Request` and `Kind`.
     The suffix helper is `fn suffix(length, preset: Option<ExportPreset>, role) ->
     Result<Option<String>, Error>`: `None` gives `Ok(render_suffix(length))`, and `Some(p)`
     gives `export_suffix(length, p, role).map(Some).map_err(Error::Render)`.
   - **`destination.rs`:**
     - `Destination`, `from_out` and `project`;
     - `Preview` and `PreviewFile`;
     - the private `enum Place { Explicit, ChosenFolder(PathBuf), Project(Project) }`;
     - the mix and stems resolution tables.
     - **`prepare`'s mix order:** iterate `roles(preset).iter().rev()`, resolving each, then
       reverse the vector so it is stored in table order.
     - **`prepare_previewed`'s mix order:** derive the stem first when needed (the
       `InvalidSongName` check), then for each role in table order resolve the path and then call
       `render_exists`.
     - **Stems:** resolution goes through the `stems.rs` resolvers. Make `stems::project_target`
       `pub(crate)`. Only `prepare_previewed` calls `target.check_available()`.
   - **`publish.rs`:**
     - `ReplacePolicy`;
     - `PublishStep` (pub `index`, `role` and `path`; private `mode: Mode { Replace, Create }`,
       chosen by policy and snapshot, and `place: &Place`);
     - `publish_file`, implementing the proposal's publish table exactly, in its column order;
     - `check_chosen_folder`: `if !f.is_dir() { FolderChanged }`, then `f.canonicalize()`, with an
       error giving `FolderUnreadable`, then `!= f` giving `FolderChanged`.
   - **`error.rs`:**
     - `Error`, `RunError`, `PartialWrite` and `WrittenPath`;
     - `impl From<Error> for RenderError` per the Wording table, CLI column. `Project` goes
       through `crate::output::io_error`; `RolesMismatch` maps to `RenderError::Render("the
       rendered files do not match the destinations")`. The CLI overrides it with "unexpected
       intro file" in stage 002.
   - **`pipeline.rs`:**
     - the `Pipeline` trait with the defaulted `publish`;
     - `pub struct RealPipeline;`, implemented with `crate::render`, `crate::render_export` and
       `crate::render_stems`.
   - **`fake.rs`:** `#[derive(Default)] pub struct FakePipeline` with the proposal's fields. Each
     render method:
     1. pushes `(code.to_owned(), opts.clone())` to `seen`;
     2. sets `preset` (`render_export`) or pushes `target.path()` to `stem_targets`
        (`render_stems`);
     3. emits `events`;
     4. returns its `take()`n result, or `Err(RenderError::Render("fake pipeline: no
        result".into()))`.

     `publish`:
     1. if `published.len() == publishes_before_error`, `take()` `publish_error` and return it if
        it is set;
     2. if `real_publish`, call `publish_file(step, bytes)?`;
     3. push `step.path` to `published` and return `Ok(())`.

     No `unwrap`, `expect` or indexing.
4. **`Prepared::run`** follows the proposal's Run section step by step:
   - **`RenderOptions`:** `RenderOptions { instruments, length: request.length.clone(), jobs:
     request.jobs, plan_timeout: request.plan_timeout, include_events: false, output }`.
   - **`new_warnings`:** a `Vec<String>` `seen` list; the file's `new_warnings` are the warnings
     not yet in `seen`, pushed as they are found.
   - **`Published::absolute`:** `std::path::absolute(&path).unwrap_or_else(|_| path.clone())`.
   - **Moving the bytes:** iterate `rendered.into_iter().zip(files)` and move `output.summary`
     into `Published` after publishing `&output.bytes`.
5. **Use the role table in the two existing places:**
   - `output::export_suffix`: before the suffix match, `if !export::roles(Some(preset)).contains(&role)`,
     return the existing `RenderError::Render("invalid output role for export preset")`. Keep the
     match for the strings; its `_` arm stays as the same error.
   - `loop_export.rs` `Part`s:
     - intro-loop: `let &[intro, repeat] = roles(Some(ExportPreset::IntroLoop)) else { return
       Err(render_error("invalid loop export roles")) };`;
     - folded and tagged: `let &[role] = roles(Some(preset)) else { … };`, replacing the inline
       `if preset == FoldedLoop`.

     Behaviour and bytes are identical.
6. **Do not touch** the CLI, the app, `tests/stems.rs` or `tests/pipeline.rs` yet. The old
   resolvers stay public. The duplicate resolution in `output.rs` is deleted in stage 004.

## Tests

Put the tests in `#[cfg(test)] mod tests` in the module's files, using `tempfile` and
`Project::create`. Use `FakePipeline` unless a case says `RealPipeline`, and a silent song
(`s('~')`) or `const meta = { bpm: 240, meter: '1/4' }; note('a4').s('sine')` for the real
renders.

- **`roles_table`:** the five rows exactly.
- **`loop_presets_take_only_a_plain_clip`:** {None, OneShot, TaggedLoop, FoldedLoop, IntroLoop} ×
  {`Clip{None}`, `Clip{Some(2)}`, `Section{"body"}`, `Length{4}`, `Window{0,4,Clip}`}. It is
  `Err(LoopPresetLength)` only for the three loop presets with anything but `Clip{None}`.
- **`destinations_resolve_every_role_without_creating_anything`:**
  - **The grid:** presets × `Format::ALL` × {`Explicit(dir/"pair.wav")`, `ChosenFolder(canonical
    dir)`, `Destination::from_out(None, song)` (`Project` found), `Destination::project(project)`}.
  - **Length:** `Section{"the drop!"}` for none and one-shot, `Clip{None}` for loop presets.
  - **Expected names:** `main-the-drop.<ext>`, `main-the-drop-one-shot.<ext>`,
    `main-tagged-loop.<ext>`, `main-folded-loop.<ext>`, and `main-intro.<ext>` +
    `main-loop.<ext>`.
  - **`Explicit`:** `pair.wav` as given, except intro-loop gives `pair-intro.<ext>` and
    `pair-loop.<ext>`.
  - **Both entry points:** assert `prepare(...).targets()` roles and paths, and
    `prepare_previewed` gives the same paths with `exists: false`.
  - **Nothing created:** `renders/` was removed first and is still absent. Afterwards, write a
    regular file at `renders` and assert that `ChosenFolder` still previews two files.
- **`resolver_names_and_errors_match_the_cli`** (from `output.rs`'s tests): project names for
  `Clip{None}` wav, `Section{"the drop!"}` opus, `Length{16}`, `Clip{Some(2)}` ogg and
  `Window{4,12,Clip}`, plus:
  - `Section{"!!"}` → `RenderError::from(err).to_string() == "<root>/renders: invalid render name
    \"!!\""`;
  - a song in a non-project `songs/`, a loose song and a missing song → `from_out(None, song)`
    errors usage `--out` with `--out is required: <song> is not in a strudeland project`;
  - `Explicit("rel/a.mp3")` and `Explicit("/abs/b.wav")` returned unchanged.
- **`each_entry_point_keeps_its_adapters_error_order`** (unix): symlink both `renders/main-intro.wav`
  and `renders/main-loop.wav` to outside files, using `Destination::from_out(None, song)` and
  intro-loop. `prepare`'s `Error::Project` source names `renders/main-loop.wav`;
  `prepare_previewed` with `Destination::project` names `renders/main-intro.wav`.
- **`prepare_never_stats_the_explicit_file`:** a directory at `--out`. `prepare` is `Ok`. `run`
  with `real_publish: true` under `Always` renders (`seen` has one entry), then returns `Failed`
  (nothing written). On unix, a symlink at `--out` pointing at `target.wav` is replaced by a
  regular file, and `target.wav` is unchanged.
- **`always_replaces_and_renders_is_created_on_publish`:** Project found, `real_publish`. An
  existing file is overwritten, and a missing `renders/` is created by the publish.
- **`create_new_needs_approval_before_rendering`:** a snapshot with `exists: true` and `approved:
  false` gives `Failed(ReplacementNotApproved)`, an empty `seen`, and the file unchanged. With no
  preview: `Failed(PreviewRequired)`. With a preview whose roles differ: `Failed(RolesMismatch)`.
- **`create_new_keeps_a_competing_file`:** for `ChosenFolder` and `Project::opened`, intro-loop,
  `real_publish`. The `Written` hook writes the second path. The result is `PartialWrite{written:
  [first], failed: Some(second), ..}`, and the competing bytes are kept.
- **`a_moved_chosen_folder_is_refused`:** the hook renames the folder after the first file. The
  result is `PartialWrite` with `source: FolderChanged{path: folder}`.
- **`a_first_file_failure_is_not_partial`:** `publish_error` at 0 gives `Failed`. At 1 it gives
  `PartialWrite` with `written.len() == 1`.
- **`an_approved_file_that_vanished_is_written`:** the snapshot says `exists: true`, the file is
  deleted before run, and `approved` is true. It is written.
- **`events_are_ordered_and_written_is_synchronous`:** an intro-loop run with `real_publish:
  true` and an explicit destination. The recorded order is `Pipeline(..)*`, `Publishing`,
  `Written`, `Written`. Inside the first `Written` hook, assert that the first file exists on disk
  and the second does not.
- **`warnings_are_new_per_file_and_deduped_across_files`:** the rows in the proposal.
- **`render_options_pass_through`:** mix with custom `OutputOptions`, `jobs: Some(3)`, timeout
  7.5 s and a non-default `InstrumentSnapshot` (or `Default` if one cannot be built simply):
  `seen[0].1` equals the expected struct exactly. Stems gives `Loudness::Off` and `Wav{F32}`.
- **`a_role_mismatch_publishes_nothing`:** the fake returns `[Single]` for intro-loop. The result
  is `Failed(RolesMismatch)` and `published` is empty.
- **`stem_sets_never_replace_and_publish_the_manifest_last`** (`RealPipeline`):
  - **`Explicit(existing dir)`:** `prepare` is `Ok`, `prepare_previewed` is `Err`, `run` is
    `Failed(Render(Io{AlreadyExists}))`, and the directory contents are unchanged.
  - **A fresh dir:** `Finished::Stems` with `export.files.last()` ending in `stems.json`, and no
    `Publishing` or `Written` events.
- **`stem_failures_report_written_paths`** (fake): `events: [Progress::Written{mix}]` with
  `stems: Some(Err(Io))` gives `PartialWrite{failed: None, written: [mix]}`. With no events it is
  `Failed`.
- **`errors_word_as_the_pipeline_did`:** `RenderError::from` for every `Error` variant, asserting
  the exact text.
- **`suffixes_follow_the_role_table`:** `export_suffix` errors for each (preset, role) not in
  `roles(Some(preset))`.

## Validation

- **Targeted:** `cargo test -p strudeland-render export::` while working.
- **The stage gates,** logged once as in 000-prompt: fmt check, clippy, `cargo test --workspace`.
  Everything outside the render crate must be unchanged and green.
- **`git diff --stat`** shows only `crates/strudeland-render/src/{lib.rs, loop_export.rs (rename),
  output.rs, stems.rs, export/*}`.

## Commit message

```
refactor(render): add the shared export module

Add strudeland_render::export: the export request, destinations, the
role table, the loop-preset length rule, preview, render, the role
check, ordered publish under a replace policy, and typed errors with
the partial write. A Pipeline seam has a real adapter and a doc-hidden
fake. The loop arrangement moves to loop_export.rs and uses the role
table, as does export_suffix. No caller changes yet.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

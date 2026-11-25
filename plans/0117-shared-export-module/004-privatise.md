# 004: privatise, and document the module

## Goal

Shrink `strudeland-render`'s public surface to what callers outside the crate use. Delete the
resolvers the module replaced, and move `tests/stems.rs` onto the module. Then add the glossary
terms and update the `AGENTS.md` Layout. There is no behaviour change.

## Prerequisites

- **Stage 003 is committed.**
- **Confirm the only importers outside the crate** of the items below are the ones listed:

  ```sh
  rg -n "resolve_output|resolve_export_output|resolve_stem_output|resolve_project_stem_output|resolve_stem_output_in|export_suffix|render_suffix|render_stems|check_available" --glob '!crates/strudeland-render/src/**'
  ```

  Expected: only `crates/strudeland-render/tests/stems.rs`. Any other hit is a caller stage 002
  or 003 missed; fix it there in this commit.

## Implementation

1. **`crates/strudeland-render/src/output.rs`:**
   - Delete `resolve_output`, `resolve_export_output` and `project_output`, with their unit tests:
     - `an_explicit_out_is_returned_unchanged`;
     - `a_project_song_goes_to_renders`;
     - `renders_is_created_only_when_the_file_is_written`;
     - `a_song_outside_a_project_needs_out`.

     Stage 001's `resolver_names_and_errors_match_the_cli` and
     `always_replaces_and_renders_is_created_on_publish` cover their assertions. Check each
     assertion against them before deleting, and add any that is missing to the module's tests.
   - Make `render_suffix` and `export_suffix` `pub(crate)`.
   - Keep `OutputTarget` (`file` and `path`), `write_output`, `resolve_format` and `io_error`
     (`pub(crate)`), and their tests (`writes_and_overwrites`,
     `a_missing_parent_is_an_io_error_and_nothing_is_written`, `project_errors_become_io`, and the
     format tests).
   - Drop `OutputTarget::project` and the `Some(project)` arm of `write_output` only if nothing
     constructs them any more: after `project_output` goes, `OutputTarget` is built only by
     `file`. Simplify `OutputTarget` to the path and `write_output` to `atomic_write_bytes`. Keep
     the doc comment accurate.
2. **`crates/strudeland-render/src/stems.rs`:**
   - Make `resolve_stem_output`, `resolve_project_stem_output`, `resolve_stem_output_in`,
     `project_target`, `StemTarget::check_available` and `render_stems` `pub(crate)`.
   - `StemTarget` and `StemTarget::path` stay `pub`, because `Pipeline::render_stems` names
     `&StemTarget`. So do `StemExport` and `StemGrouping`.
3. **`crates/strudeland-render/src/lib.rs`:**
   - `pub use output::{OutputTarget, resolve_format, write_output};`
   - `pub use stems::{StemExport, StemGrouping, StemTarget};`
   - `pub use loop_export::{ExportFile, ExportRole, LoopPoints, render_export};` is unchanged.
   - Fix any `unused`/`dead_code` warnings clippy reports. Delete the dead code; do not
     `#[allow]` it.
4. **`crates/strudeland-render/tests/stems.rs`:** move onto the module and keep every assertion.
   Add helpers:

   ```rust
   fn request(opts: &RenderOptions, grouping: StemGrouping, song: &Path) -> export::Request {
       export::Request { song: song.into(), length: opts.length.clone(),
           kind: export::Kind::Stems { grouping }, jobs: opts.jobs, plan_timeout: opts.plan_timeout }
   }
   /// Prepares and runs a stem set into `dir`, forwarding pipeline progress.
   fn stems(code: &str, opts: &RenderOptions, grouping: StemGrouping, dir: &Path,
            progress: &dyn Fn(Progress)) -> Result<StemExport, RenderError> {
       let prepared = export::prepare(request(opts, grouping, Path::new("song.js")),
           export::Destination::Explicit(dir.into())).map_err(RenderError::from)?;
       match prepared.run(&mut export::RealPipeline, code, export::ReplacePolicy::Always, None,
           opts.instruments.clone(), &|e| if let export::Event::Pipeline(p) = e { progress(p) }) {
           Ok(export::Finished::Stems { export, .. }) => Ok(export),
           Ok(_) => panic!("expected a stem set"),
           Err(export::RunError::Failed(e)) => Err(e.into()),
           Err(export::RunError::PartialWrite(p)) => Err(p.source.into()),
       }
   }
   ```

   - **The five tests:** each `resolve_stem_output(Some(dir), "song.js", …)` +
     `render_stems(…)` pair becomes `stems(…)`.
   - **`assert!(target.check_available().is_err())`** becomes
     `assert!(export::prepare_previewed(request(&opts, StemGrouping::Orbit, Path::new("song.js")),
     export::Destination::Explicit(directory.clone())).is_err())`.
   - **`project_path_is_previewable_and_silence_produces_only_a_mix`** uses
     `export::prepare_previewed(request(&opts, Orbit, &song), Destination::project(project.clone()))`.
     Assert `Preview::Stems { directory } == root/renders/unsaved-bars0-1-stems` and that it does
     not exist, then run the returned `Prepared` with `RealPipeline`.
   - The `RenderError::Io { path, source }` matches stay as they are: `Error::Render` unwraps to
     the same error.
5. **`crates/strudeland-render/tests/pipeline.rs`:** no edit. It imports only `OutputTarget`,
   `write_output`, `render`, `render_audio`, `finish`, `render_export`, `ExportPreset` and
   `ExportRole`, all public. **`src/bin/strudeland-render.rs`:** no edit (`OutputTarget::file`,
   `write_output`, `resolve_format`).
6. **`GLOSSARY.md`:** add an `## Audio export` section after the last existing section. Use the
   file's style: a bold term, a one or two sentence definition, then an optional `_Avoid_:` line.

   ```markdown
   ## Audio export

   **Export request**

   What one export renders: the song, its length request, and either a mix (with an optional
   preset and its output options) or a stem set (with its grouping).
   _Avoid_: render job, export plan

   **Destination**

   Where an export's files go: an explicit path, a chosen folder, or a project's `renders/`. The
   same destination names a mix's files and a stem set's directory differently.
   _Avoid_: target (one resolved file), output

   **Role**

   What one file of a mix export is for: `single`, `intro` or `loop`. A preset fixes its roles
   and their order.
   _Avoid_: part, kind

   **Stem set**

   One directory holding a pre-master reference mix, a float WAV per group and a `stems.json`
   manifest, rendered together and never replaced.
   _Avoid_: stems folder, stem export (the action)
   ```

7. **`AGENTS.md` Layout:**
   - **The render pipeline paragraph (the `crates/strudeland-render/` list):** replace "`export`
     (one-time loop openings, isolated repeats with post-roll folding, and shared static gain)"
     with "`loop_export` (one-time loop openings, isolated repeats with post-roll folding, and
     shared static gain), `export/` (the export module both the CLI and the app run: request,
     destinations, the role table, preview, the replace policies, ordered publish and the partial
     write, behind the `Pipeline` seam; `testing::FakePipeline` is its fake)".
   - Change "`crates/strudeland-cli/src/render.rs` is `strudeland render` (options, progress,
     summary, exit codes)" to "(flag rules, the export adapter, progress, summary, exit codes)".
   - **The desktop audio export paragraph:** change "uses the shared `strudeland-render`
     pipeline" to "runs `strudeland_render::export` with the create-new replace policy".

   Keep the existing wrapping style.

## Tests

- **`tests/stems.rs`:** moved, with the same assertions.
- **`output.rs`:** the resolver tests are deleted only where stage 001's module tests carry the
  same assertions. List each deleted test and its covering module test in the commit body.
- **No other test changes.**

## Validation

- **The stage gates,** logged once. Clippy must show no dead-code allowances added.
- **The prerequisite `rg`** finds nothing outside `crates/strudeland-render/src/`.
- **`cargo doc -p strudeland-render --no-deps`** builds without broken intra-doc links (an
  optional quick check).

## Commit message

```
refactor(render): keep the export resolvers private and document the module

The path resolvers the export module replaced are deleted or made
crate-private; OutputTarget, write_output, render_export and StemTarget
stay public for the harness, the pipeline tests and the Pipeline seam.
The stem integration tests run through the export module. The glossary
gains export request, destination, role and stem set, and AGENTS.md
names loop_export and export/.

Covered by the export module's tests:
- an_explicit_out_is_returned_unchanged, a_project_song_goes_to_renders,
  a_song_outside_a_project_needs_out: resolver_names_and_errors_match_the_cli
- renders_is_created_only_when_the_file_is_written:
  always_replaces_and_renders_is_created_on_publish

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

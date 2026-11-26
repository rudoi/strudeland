# 0117: one export module shared by the CLI and the app

## Goal

`strudeland render` and the desktop export dialog run the same export module,
`strudeland_render::export`. It owns:

- the export request;
- the destination and how each one resolves target paths;
- the role table;
- the loop-preset length rule;
- the preview (the `exists` snapshot);
- render, the role check and the ordered publish of a mix;
- the typed errors, including the partial write.

Each caller keeps only its adapter. The CLI keeps clap, its flag rules, the wording of errors,
exit codes, stderr progress and the summaries. The app keeps IPC, the chosen-folder state, its
length-validity check, the confirm dialog, the export permit for Application Quit, the wording of
errors and `ExportProgress`.

This is a refactor. The following stay identical:

- CLI output, flags and exit codes;
- app text and behaviour;
- output file names and paths;
- overwrite semantics;
- rendered bytes.

The typed planner errors and the loop export button are #144 (PR B), approved by the user on
2026-11-25.

## Background

The [re-verification](reverify.md) has the evidence. The refine review
added the eight must-fix findings this revision folds in. The
[loss check](0117-shared-export-module/loss-check.md) maps them.

### Current behaviour (kept exactly)

| Aspect | CLI (`crates/strudeland-cli/src/render.rs`, `run.rs`) | App (`src-tauri/src/export_audio.rs`) |
|---|---|---|
| Check order, mix | read song (usage), preset/length (usage `--preset`), format and encoder flags, loudness flags, destination (primary role first: `Loop` before `Intro`), instrument snapshot, render | project identity, preset/length ("loop exports use the marked loop section"), then length validity, song resolve, chosen folder, the song stem ("invalid song filename"), then per role in table order (`Intro` before `Loop`): path, then `exists`; on start the same again, preview comparison, instrument snapshot, approval, render |
| Check order, stems | read song, stem conflicts (`--preset`, `--format`, `--bit-depth`, `--loudness`, `--lufs`, `--true-peak`, then `--mp3-mode`/`--quality` via `from_flags`), destination, instrument snapshot, then `check_available` inside `render_stems`. It never checks preset/length. | project, preset/length, length validity, song, folder, "stem sets require a standard range and WAV format", destination, `check_available`; on start the same again, preview comparison, instrument snapshot, render |
| Destination, mix | `--out` file as given (intro-loop splits into `<stem>-intro.<ext>` and `<stem>-loop.<ext>`), or `Project::find` → `renders/<stem>[-suffix].<ext>`; outside a project, usage `--out is required: … is not in a strudeland project` | chosen folder (canonical as picked, joined, not re-canonicalized) or the open project's `renders/`, with the same names |
| Destination, stems | `--out` is the set directory (needs a file name), or project `renders/<stem>[-suffix]-stems` | chosen folder (parent canonicalized) or the open project |
| Overwrite, mix | always replaces. It never stats `--out`: a directory at `--out` fails only after render, and a symlink at `--out` is replaced by the rename. | the preview shows `exists` per file. "Replace audio files?" dialog. Each file is replaced if the preview said it exists (after a `render_exists` re-check), else created without clobbering. The chosen folder is re-checked before every file. |
| Overwrite, stems | never replaces | never replaces, no dialog |
| Partial write | mix with at least one file written: stderr `partial export: wrote A, failed to write B` (A absolute, B the raw target), then `error: …`, exit 1. Stems: the `wrote …` lines and `error: …` only. | `ExportError{message, written:[…]}`; `written` is empty when the first file fails |
| Warnings | preset: each file's new warnings right after its `wrote` line (`HashSet`, first seen, across files); no preset: all of the file's warnings after `wrote`; stems: the set's warnings after the render; JSON keeps per-file warnings | mix: deduped across files (`Vec::contains`); stems: the set summary's warnings |
| Progress | stderr lines (`planned`, `rendered`, `encoded`, `re-encoded`, `wrote <absolute>`) | `Confirming`, `Planning`, pipeline events, `Writing` once for a mix only, `Written` per file (the resolved path) |
| Options | flags | fixed: jobs `None`, timeout 120 s, mix `Loudness::default()` + `default_for(format)`, stems loudness off + 32-bit float WAV |

## Scope

In:

1. **The new `strudeland_render::export` module** (`crates/strudeland-render/src/export/`). See
   the Technical design for the full interface:
   - `Request`, `Kind`, `Destination`, `roles`;
   - `check_length_for_preset`, `prepare`, `prepare_previewed`, `Prepared::run`;
   - `Preview`, `ReplacePolicy`, `Event`;
   - `Finished`, `Published`, `Error`, `RunError`, `PartialWrite`;
   - `check_chosen_folder`;
   - the `Pipeline` seam with `RealPipeline`.

   The doc-hidden `strudeland_render::testing::FakePipeline` comes with it.
2. **One role table: `export::roles(Option<ExportPreset>)`.** `export_suffix`, the loop
   arrangement's `Part`s, `prepare` and `run` all use it. The CLI's and the app's own role
   matches are deleted.
3. **Only the loop-preset/length rule becomes shared** (`check_length_for_preset`):
   - The CLI's stem conflicts stay CLI flag rules.
   - The app's stems-compat and length-validity checks stay in the app.
4. **Two replace policies, for mixes only:**
   - `Always` for the CLI;
   - `CreateNew { approved }` for the app.

   Stems never replace.
5. **The CLI moves onto the module.** `RenderBackend`, `StrudelandBackend`, `Plan`, the CLI's
   `Destination` and `ExportPlan` are deleted.
6. **The app moves onto the module.** `Targets`, `ExportRequest::{targets, options, render_mix,
   write}` and `check_directory` are deleted. So is the parity test
   `names_and_roles_match_cli_for_every_format_and_preset`.
7. **Privatise** what nothing outside the crate needs (stage 004), and update
   `tests/stems.rs`.
8. **Docs:** `GLOSSARY.md` gains export request, destination, role and stem set. The `AGENTS.md`
   Layout names `loop_export.rs` and `export/`.

Out:

- **Typed planner length errors and the dialog's flag-free wording.** That is #144.
- **Disabling loop export without a marked loop.** That is #144.
- **Unifying the overwrite policy.** The CLI keeps replacing, as SKILL.md documents.
- **Moving the app's length-validity check into the module.** It would change CLI exit codes for
  `--section ""`.
- **Moving the CLI's stem conflicts or the app's stems-compat check into the module.** Their order
  and wording differ, and neither is shared.
- **Any edit to `job-plan.ts` or other planner files.** There is no host bundle or refs churn.
- **Any change to `SKILL.md`.** No CLI surface changes.

## Technical design

### Placement

- **`crates/strudeland-render/src/export.rs` (the loop arrangement) is renamed to
  `loop_export.rs`** with `git mv`. `lib.rs` keeps `pub use loop_export::{ExportFile,
  ExportRole, LoopPoints, render_export}`, so the public names do not move.
- **`crates/strudeland-render/src/export/`** holds the new module:
  - `mod.rs`: re-exports, `prepare`, `prepare_previewed`, `Prepared`, `run`;
  - `request.rs`: `Request`, `Kind`, `ExportProblem`, `check_length_for_preset`, `roles`;
  - `destination.rs`: `Destination`, target resolution, `Preview`;
  - `publish.rs`: `ReplacePolicy`, `PublishStep`, `publish_file`, `check_chosen_folder`;
  - `error.rs`: `Error`, `RunError`, `PartialWrite`, `WrittenPath`, `From<Error> for
    RenderError`;
  - `pipeline.rs`: `Pipeline`, `RealPipeline`;
  - `fake.rs`: `FakePipeline`.
- **`lib.rs`** declares `pub mod export;` and `#[doc(hidden)] pub mod testing { pub use
  crate::export::fake::FakePipeline; }`, the `strudeland-live` pattern. Nothing from `export` is
  re-exported at the crate root. Callers write `strudeland_render::export::Request` and so on,
  which avoids the app's `ExportRequest`, `ExportPreview`, `ExportProgress`, `ExportError` and
  `FinishedFile`.
- **It stays inside `strudeland-render`,** so the app's macOS `host-jitless` feature still
  applies.
- **`ExportPreset` lives in `strudeland-host`.** The orphan rule rules out an inherent
  `ExportPreset::roles()`, so the table is a free function.

### Interface

```rust
// request.rs
pub fn roles(preset: Option<ExportPreset>) -> &'static [ExportRole];
//   None | OneShot | TaggedLoop → [Single]; FoldedLoop → [Loop]; IntroLoop → [Intro, Loop].
//   None means one Single file rendered with render(), not render_export(OneShot).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportProblem { LoopPresetLength }
/// Err iff `preset` is a loop preset (not OneShot) and `length` is not `Clip { loops: None }`.
pub fn check_length_for_preset(length: &LengthRequest, preset: Option<ExportPreset>)
    -> Result<(), ExportProblem>;

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub song: PathBuf,                 // the song file; it may not exist (an unsaved app buffer)
    pub length: LengthRequest,
    pub kind: Kind,
    pub jobs: Option<usize>,
    pub plan_timeout: Option<Duration>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Mix { preset: Option<ExportPreset>, output: OutputOptions },
    Stems { grouping: StemGrouping },  // output fixed: Loudness::Off, Wav { bit_depth: F32 }
}

// destination.rs
pub enum Destination {
    Explicit(PathBuf),                                // CLI --out
    ChosenFolder(PathBuf),                            // app folder, as picked (canonical)
    Project { project: Project, stem: Option<String> },
}
impl Destination {
    /// CLI: `Explicit(out)`, else `Project::find(song)` with its stem, else
    /// `Error::Render(Usage { "--out", "--out is required: {song} is not in a strudeland project" })`.
    pub fn from_out(out: Option<&Path>, song: &Path) -> Result<Destination, Error>;
    /// App: the open project; the stem comes from `Request::song`.
    pub fn project(project: Project) -> Destination;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview { Mix(Vec<PreviewFile>), Stems { directory: PathBuf } }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFile { pub role: ExportRole, pub path: PathBuf, pub exists: bool }

// mod.rs
pub fn prepare(request: Request, destination: Destination) -> Result<Prepared, Error>;
pub fn prepare_previewed(request: Request, destination: Destination)
    -> Result<(Prepared, Preview), Error>;
pub struct Target { pub role: ExportRole, pub path: PathBuf }
impl Prepared {
    pub fn targets(&self) -> &[Target];            // mix files in table order; empty for stems
    pub fn stem_directory(&self) -> Option<&Path>; // Some for stems
    pub fn preset(&self) -> Option<ExportPreset>;  // the mix preset; None for stems
    pub fn run(
        self,
        pipeline: &mut dyn Pipeline,
        code: &str,
        policy: ReplacePolicy,
        preview: Option<&Preview>,
        instruments: InstrumentSnapshot,
        progress: &dyn Fn(Event<'_>),
    ) -> Result<Finished, RunError>;
}

// publish.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacePolicy { Always, CreateNew { approved: bool } }
pub struct PublishStep<'a> { pub index: usize, pub role: ExportRole, pub path: &'a Path, /* private: mode, place */ }
pub fn publish_file(step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error>;
pub fn check_chosen_folder(folder: &Path) -> Result<(), Error>;

// events and results
pub enum Event<'a> {
    Pipeline(Progress),                                      // forwarded unchanged
    Publishing,                                              // mix only, once, before the first file
    Written { file: &'a Published, new_warnings: &'a [String] }, // mix only, after each file
}
pub struct Published {
    pub role: ExportRole,
    pub path: PathBuf,       // the resolved target (the app reports this)
    pub absolute: PathBuf,   // std::path::absolute(path), or path if that fails (the CLI prints this)
    pub summary: Summary,
    pub loop_points: Option<LoopPoints>,
}
pub enum Finished {
    Mix { preset: Option<ExportPreset>, files: Vec<Published>, warnings: Vec<String> },
    Stems { grouping: StemGrouping, export: StemExport },
}

// error.rs
#[derive(Debug)]
pub enum Error {
    Render(RenderError),
    Project { at: PathBuf, source: ProjectError },
    FolderChanged { path: PathBuf },
    FolderUnreadable { path: PathBuf, source: io::Error },
    InvalidSongName,
    RolesMismatch,
    ReplacementNotApproved,
    PreviewRequired,
}
#[derive(Debug)]
pub enum RunError { Failed(Error), PartialWrite(PartialWrite) }
#[derive(Debug)]
pub struct PartialWrite {
    pub written: Vec<WrittenPath>,  // never empty
    pub failed: Option<PathBuf>,    // Some(target path) for a mix; None for stems
    pub source: Error,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenPath { pub path: PathBuf, pub absolute: PathBuf }
impl From<Error> for RenderError { /* the pipeline's wording, used by the CLI */ }

// pipeline.rs
pub trait Pipeline {
    fn render(&mut self, code: &str, opts: &RenderOptions, progress: &dyn Fn(Progress))
        -> Result<RenderOutput, RenderError>;
    fn render_export(&mut self, code: &str, opts: &RenderOptions, preset: ExportPreset,
        progress: &dyn Fn(Progress)) -> Result<Vec<ExportFile>, RenderError>;
    fn render_stems(&mut self, code: &str, opts: &RenderOptions, grouping: StemGrouping,
        target: &StemTarget, progress: &dyn Fn(Progress)) -> Result<StemExport, RenderError>;
    /// Publishes one mix file. The default is the real rule.
    fn publish(&mut self, step: &PublishStep<'_>, bytes: &[u8]) -> Result<(), Error> {
        publish_file(step, bytes)
    }
}
pub struct RealPipeline; // calls crate::render, render_export, render_stems; default publish
```

`Error` has no `Display`, so each adapter words it.

### Entry points and order

The module has three entry points. Each adapter calls them at its own point, so today's check
order holds.

1. **`check_length_for_preset`** is pure:
   - The CLI calls it only on the mix branch, before `mix_options`, and words it as usage
     `--preset`: "loop presets select their own bars; omit --section, --length and --loops".
   - The app calls it first, then its own validity match, and words it as "loop exports use the
     marked loop section".
   - `prepare` never calls it.
2. **`prepare` and `prepare_previewed`** resolve the destination only:
   - They take no instruments and never create anything.
   - `prepare` (CLI) never stats a mix target. It resolves mix roles in reverse table order, the
     CLI's primary-first order, and stores them in table order. For stems it resolves the
     `StemTarget` but does not call `check_available`; `render_stems` still does, after the CLI's
     instrument snapshot.
   - `prepare_previewed` (app) resolves each role in table order, then stats it with
     `render_exists` before moving on to the next role, as `targets()` does today. For stems it
     calls `check_available` after resolving. It returns the `Preview`.
   - `exists` comes only from `prepare_previewed`.
3. **`Prepared::run`** renders and publishes. Instruments are resolved by each adapter after its
   destinations, as today.

### Destination resolution (kind-dependent)

Mix, role `r`. The suffix is `render_suffix(length)` with no preset, else
`export_suffix(length, preset, r)`. The extension is `output.encoder.format().ext()`.

| Destination | Path | Errors |
|---|---|---|
| `Explicit(out)` | `out` unchanged, except intro-loop: `out.with_file_name("<file_stem>-<suffix>.<ext>")` | no file stem: `Render(Usage{"--out", "the split export needs a file name for --out"})` |
| `Project{stem: Some(s)}` | `project.render_path(s, suffix, ext)` | `Project{at: root/renders, source}` |
| `Project{stem: None}` | the stem is `request.song.file_stem()` as UTF-8; then `render_path` | `InvalidSongName` (once, before any role), then `Project{at: root/renders, source}` |
| `ChosenFolder(f)` | stem as above; `f.join(render_filename(stem, suffix, ext))` | `InvalidSongName`; `Project{at: f, source}` |

In `prepare_previewed`, `exists = render_exists(&path)` maps an error to `Project{at: path,
source}`.

Stems. Each resolver error is `Error::Render(e)`, unchanged:

| Destination | Call |
|---|---|
| `Explicit(out)` | `stems::resolve_stem_output(Some(out), song, length)` |
| `ChosenFolder(f)` | `stems::resolve_stem_output_in(f, song, length)` (canonicalizes the parent) |
| `Project{stem: Some(s)}` | `stems::project_target(&project, &s, length)` (made `pub(crate)`) |
| `Project{stem: None}` | `stems::resolve_project_stem_output(&project, song, length)` |

The stem has two sources:

- **The CLI's stem** comes from its adapter, through `Destination::from_out` and `Project::find`.
- **The app's stem** is derived from `Request::song`. Its two wordings, "invalid song filename"
  and the stems resolver's "a stem set needs a valid song name", and the resolver's
  canonicalize-first order must hold.

### Run

The module builds `RenderOptions` exactly:

- `instruments`, `length`, `jobs`, `plan_timeout` from the request;
- `include_events: false`;
- `output`: from `Kind::Mix`, or `Loudness::Off` + `Wav{bit_depth: F32}` for stems.

A fake pipeline asserts these.

A **mix** runs in this order:

1. **The preview check.** Under `CreateNew`, `preview` must be `Some(Preview::Mix(files))` with
   the same roles and paths as the targets. `None` is `PreviewRequired`; another shape is
   `RolesMismatch`.
2. **The approval check.** Under `CreateNew{approved: false}`, any `exists` is
   `ReplacementNotApproved`. Nothing has rendered yet.
3. **Render.**
   - With no preset: `pipeline.render` → one `ExportFile{Single, output, None}`.
   - Otherwise: `pipeline.render_export(preset)`.
   - Every `Progress` is forwarded as `Event::Pipeline`.
4. **The role check.** The rendered roles must equal the target roles, else `RolesMismatch`.
5. **`Event::Publishing`.**
6. **Publish each file in table order** with `pipeline.publish(step, bytes)`:
   - **On success,** compute `new_warnings`: the file's warnings not seen in this export, first
     seen, also deduping inside one file. Build the `Published` and call `Event::Written`
     synchronously before the next file.
   - **On failure with nothing written yet,** return `Failed(e)`.
   - **On a later failure,** return `PartialWrite{written, failed: Some(target path), source: e}`.
7. **`Finished::Mix`.** `warnings` is the seen list.

A **stem set** ignores `policy` and `preview`, because a set never replaces:

1. Call `pipeline.render_stems` with a progress wrapper. The wrapper forwards every event as
   `Event::Pipeline` and records each `Progress::Written` path (absolute already) as a
   `WrittenPath{path, absolute: path}`.
2. Map the result:
   - `Ok` → `Finished::Stems`;
   - `Err` with no `Written` seen → `Failed(Render(e))`;
   - otherwise `PartialWrite{written, failed: None, source: Render(e)}`.

There is no `Publishing` and no `Event::Written` for stems.

### Publish rule (`publish_file`)

Under `CreateNew`, the per-file `exists` comes from the preview snapshot. There is no live
re-stat to decide.

| Policy / snapshot | `Explicit` | `ChosenFolder(f)` | `Project(p)` |
|---|---|---|---|
| `Always` | `atomic_write_bytes` (never stats) | `check_chosen_folder(f)`, `atomic_write_bytes` | `p.write_render` |
| `CreateNew`, `exists: true` | `render_exists` (error if not a regular file, result ignored), `atomic_write_bytes` | `check_chosen_folder(f)`, `render_exists`, `atomic_write_bytes` | `render_exists`, `p.write_render` |
| `CreateNew`, `exists: false` | `atomic_create_bytes` | `check_chosen_folder(f)`, `atomic_create_bytes` | `p.create_render` |

- **Errors:** a `ProjectError` maps to `Project{at: path, source}`.
- **`check_chosen_folder(f)`:** `!f.is_dir()` gives `FolderChanged{path: f}`. A failed
  `canonicalize` gives `FolderUnreadable{path: f, source}`. A canonical form `!= f` gives
  `FolderChanged`. This is the app's `check_directory`, moved.

### Progress

`Event` is a separate enum. `Progress` is unchanged, so `ProgressLines::report` and `impl
From<Progress> for ExportProgress` stay exhaustive and untouched.

- **The CLI:**
  - `Pipeline(p)` → `report(p)`;
  - `Publishing` → nothing;
  - `Written` → `report(Progress::Written{path: file.absolute})`, then `warnings(new_warnings)`
    with a preset, or `warnings(&file.summary.warnings)` without one.
- **The app:**
  - `Pipeline(p)` → `p.into()`;
  - `Publishing` → `Writing`;
  - `Written` → `Written{path: file.path}`.

### Wording

| `Error` | CLI (`From<Error> for RenderError`, plus `cli_error` in render.rs) | App (`export_message` in export_audio.rs) |
|---|---|---|
| `Render(e)` | `e` | `e.to_string()` |
| `Project{at, source}` | `output::io_error(&at, source)` (today's text, for example `<renders>: invalid render name "!!"`) | `source.to_string()` |
| `FolderChanged{path}` | `Io{path, other("the chosen output folder changed")}` (unreachable) | `the selected output folder changed: <path>` |
| `FolderUnreadable{source,..}` | `Io{path, source}` (unreachable) | `source.to_string()` |
| `InvalidSongName` | `Render("invalid song filename")` (unreachable) | `invalid song filename` |
| `RolesMismatch` | `cli_error`: `Render("unexpected intro file")` | `the rendered files do not match the preview` |
| `ReplacementNotApproved` | `Render("replacement requires confirmation")` (unreachable) | `replacement requires confirmation` |
| `PreviewRequired` | `Render("the export has no preview")` (unreachable) | `the export has no preview` (unreachable) |

- **A CLI partial write:** for `RunError::PartialWrite{failed: Some(b), written, ..}` (a mix,
  `written` never empty), print `partial export: wrote <written.last().absolute>, failed to write
  <b>`, then return `Failure::Render(cli_error(source))`. With `failed: None` (stems), return the
  error only.
- **The app** maps `Failed(e)` to `ExportError{message: export_message(e), written: []}`, and
  `PartialWrite` to `{message: export_message(source), written: written.path…}`.
- **What stays in the app:**
  - the "the output files changed; review the new preview before exporting" comparison of
    `ExportPreview::from(fresh)` with the client's preview;
  - `ExportState::check_folder` ("choose the output folder again", then `check_chosen_folder`);
  - the permit.

### Fake (`testing::FakePipeline`)

The fields are public and `Default`:

| Field | Purpose |
|---|---|
| `render` | `Option<Result<RenderOutput, RenderError>>` |
| `export` | `Option<Result<Vec<ExportFile>, RenderError>>` |
| `stems` | `Option<Result<StemExport, RenderError>>` |
| `events` | `Vec<Progress>`, emitted by each render method before it returns |
| `seen` | `Vec<(String, RenderOptions)>` |
| `preset` | `Option<ExportPreset>` |
| `stem_targets` | `Vec<PathBuf>` |
| `published` | `Vec<PathBuf>`: successful publishes only |
| `publish_error` | `Option<Error>` |
| `publishes_before_error` | `usize` |
| `real_publish` | `bool`: `false` records only and touches no disk; `true` calls `publish_file` after recording |

- **A missing result** returns `Err(RenderError::Render("fake pipeline: no result".into()))`. The
  crate denies `unwrap`, `expect` and indexing outside tests, and this module compiles in non-test
  builds.
- **A publish failure:** when `published.len() == publishes_before_error` and `publish_error` is
  set, the fake returns that error.

### Public surface after stage 004

| Stays public | Becomes `pub(crate)` | Deleted |
|---|---|---|
| `OutputTarget`, `write_output` and `resolve_format` (the harness binary and `tests/pipeline.rs`); `StemTarget` and its `path()` (in the `Pipeline` signature); `StemExport`, `StemGrouping`; `ExportFile`, `ExportRole`, `LoopPoints`, `render_export` | `render_suffix`, `export_suffix`, `resolve_stem_output`, `resolve_project_stem_output`, `resolve_stem_output_in`, `StemTarget::check_available`, `render_stems` | `resolve_output`, `resolve_export_output`, `project_output`, with their `output.rs` unit tests, covered by the module's table |

### Importers to update

| File | Where | Change |
|---|---|---|
| `src-tauri/src/export_audio.rs` | :10-18 (imports) | `export::{…}` replaces `atomic_*`, `render_exists`, `render_filename`, `export_suffix`, `render_suffix`, `render*`, `resolve_*`, `StemTarget`, `ExportFile`, `BitDepth` |
| `src-tauri/src/export_audio.rs` | :507 | deleted with the parity test |
| `crates/strudeland-cli/src/render.rs` | :11-15 (imports), :20-98 (`RenderBackend`, `StrudelandBackend`), :107-189 (`Plan`, `Destination`, `ExportPlan`, `plan`), :328-389 (`ExportPlan::write`) | replaced by `prepare`, `run_export` and `cli_error` |
| `crates/strudeland-cli/src/run.rs` | :27, :135, :139-145, :209-218, :479-525 | `&mut dyn export::Pipeline` replaces `RenderBackend`; `run_render` is rewritten |
| `crates/strudeland-cli/tests/e2e.rs` | :500 | name only (`…through_the_real_pipeline`); no change |
| `crates/strudeland-render/tests/stems.rs` | | stage 004 moves it onto `export` |
| `crates/strudeland-render/tests/pipeline.rs` | | no edit: it imports only items that stay public |
| `crates/strudeland-render/src/bin/strudeland-render.rs` | | no edit |

### Tests (replace, don't layer)

**In `strudeland-render`, stage 001:**

- **`roles` table:** exactly five rows.
- **`check_length_for_preset` table:** every preset × `Clip{None}`, `Clip{Some(2)}`, `Section`,
  `Length`, `Window`.
- **The destination table:** {none, one-shot, tagged, folded, intro} × `Format::ALL` ×
  {`Explicit`, `ChosenFolder`, `Project` found, `Project` opened}. Each row asserts roles, paths
  and (`prepare_previewed`) `exists: false`, and that nothing is created (`renders/` removed
  first stays absent, and a `ChosenFolder` works while `renders` is a regular file).
- **The `output.rs` resolver assertions, carried over** so stage 004 can delete them:
  - the slugs `the drop!`, `len16`, `loops2`, `bars4-12`;
  - `!!` → the CLI text `<renders>: invalid render name "!!"` through `From`;
  - outside a project → usage `--out` for a song in a non-project `songs/`, a loose song and a
    missing song;
  - `renders/` is created only by the publish.
- **The per-adapter order:** both `main-intro.wav` and `main-loop.wav` are symlinks in `renders/`.
  `prepare` names `main-loop.wav`; `prepare_previewed` names `main-intro.wav`.
- **`prepare` never stats:** a directory at `--out` prepares fine, then fails to publish under
  `Always` after render as `Failed`. On unix, a symlink at `--out` is replaced by the publish and
  its target is untouched.
- **Publish:**
  - `Always` overwrites;
  - `CreateNew{approved: false}` with an existing file is `ReplacementNotApproved` and `seen` is
    empty;
  - `CreateNew` without a preview is `PreviewRequired`;
  - a competing file created from the `Written` hook is kept, with `PartialWrite{written: [first],
    failed: Some(second)}`, for `ChosenFolder` and `Project`;
  - a chosen folder renamed from the hook is `FolderChanged`;
  - a first-file failure is `Failed`, not `PartialWrite`;
  - a preview that said `exists: true` for a now-missing file replaces, so it creates.
- **Stems:**
  - an existing directory fails `prepare_previewed` and `run`, and leaves the directory as it was
    (`RealPipeline`, silent song);
  - the manifest is the last file;
  - a failure after one `Written` is `PartialWrite{failed: None}`; before it, `Failed`;
  - no `Publishing` event.
- **Events:** pipeline events, then `Publishing`, then one `Written` per file. `Written` happens
  before the next publish.
- **Warnings:** `["a","b"]` + `["b","c"]` gives `new_warnings` `["a","b"]` and `["c"]`;
  `["a","a"]` gives `["a"]`; `Finished.warnings` is `["a","b","c"]`.
- **`RenderOptions` passthrough:** mix and stems, exactly.
- **`RolesMismatch`:** `[Single]` for intro-loop publishes nothing.
- **`From<Error> for RenderError`:** every variant's text.

**In the CLI, stage 002:** every existing `render.rs` test is kept and moved onto `FakePipeline`.
The one that called `plan()` asserts through the fake instead. The byte-exact stderr and stdout
assertions stay as they are. `e2e.rs` is untouched.

**In the app, stage 003:**

- The integration tests stay: `split_wavs_…`, `stem_export_…`, `a_collision_during_publish_…`,
  `a_changed_preview_…`, `matching_preview_…`, `stem_preview_…`, `folder_song_…`,
  `reserve_…`, and the `app_quit` tests.
- The parity test is deleted.
- A new `export_messages_keep_their_wording` covers every `Error` variant, plus `PartialWrite`.

**`tests/stems.rs`, stage 004:** moved onto `export::prepare` / `prepare_previewed` / `run`,
with the same assertions.

## Dependencies

- **Builds on:** #116 (done).
- **Blocks:** #118 (one finishing path) and #144 (PR B).

## Risks

- **Error precedence.** Each entry point keeps its adapter's order: CLI primary first and no
  stats; app interleaved resolve and stat. The symlink test pins both.
- **Wording drift.** `Error` has no `Display`. Both mappings are tables with tests. The CLI's
  partial-write line and stderr order are pinned by the moved CLI tests.
- **Windows paths.** `Published` keeps `path` (the app) and `absolute` (the CLI) apart, so the app
  never sees a path rewritten by `std::path::absolute`.
- **Application Quit.** The module is synchronous and owns no lock. The app holds the permit
  across `run`, as today.
- **No-clobber.** `exists` is the snapshot the user approved. The publish tests and the kept app
  integration tests pin every guarantee.
- **Byte reproducibility.** The options are passed through unchanged (a fake asserts them), and
  no planner files change.
- **CLI role strictness.** The CLI used to send any non-intro file to the primary target. Now any
  role mismatch is `RolesMismatch` ("unexpected intro file"). The real pipeline always returns
  the table's roles, so this cannot be observed. It is listed for honesty.
- **CLI test churn.** About 30 tests use the fake. The field names map one to one: `result` →
  `render`, `write_error` → `publish_error`, `writes_before_error` → `publishes_before_error`,
  `written` → `published`.

## Complexity

Large: about 1,000-1,500 lines moved across three crates, with visible behaviour unchanged.
Fracture is skipped. There are four stages.

## Decisions

1. **Two PRs.** This one is behaviour-identical. Typed codes and loop export are #144 (user).
2. **The role table is the free function `roles(Option<ExportPreset>)`.** The orphan rule decides
   this. The `None` row is one `Single` file through `render()`.
3. **Only the loop-preset/length rule is shared,** as `ExportProblem::LoopPresetLength`, called by
   each adapter at its own point.
4. **Two prepare entry points.**
   - `prepare` resolves only: the CLI order, and no stats.
   - `prepare_previewed` resolves and stats per file: the app order. It is the only source of
     `exists`.
5. **`ReplacePolicy` covers mixes only.** `CreateNew` decides from the approved snapshot. Stems
   never replace.
6. **Typed `Error`, no `Display`.** Each adapter has a wording table. `From<Error> for
   RenderError` is the CLI's, because the CLI prints `RenderError`s.
7. **`RunError::PartialWrite` only after at least one file.** The CLI prints the partial line for
   a mix only. The app gets `written` for both kinds.
8. **Warnings are computed per file in the `Written` hook.** The CLI keeps its interleaving and
   its per-path dedupe rules.
9. **`Event` is a separate enum.** `Progress` is untouched.
10. **`Pipeline` is public, with `publish` as a defaulted method.** `RealPipeline` is the real
    adapter. `testing::FakePipeline` is doc-hidden.
11. **Naming:** the loop arrangement moves to `loop_export.rs`, `export/` is the new module, and
    nothing is re-exported at the root. The glossary gains export request, destination, role and
    stem set.
12. **Public surface:** see the table above.

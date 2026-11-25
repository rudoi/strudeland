# 0117: one export module shared by the CLI and the app

## Goal

`strudeland render` and the desktop export dialog run the same export module in
`strudeland-render`. It owns:

- the export request;
- the destination;
- the role list per preset;
- preview;
- render;
- ordered publish;
- the partial-write error.

Each caller keeps only its adapter: clap and terminal output for the CLI; IPC, the confirm
dialog, progress and Application Quit for the app.

This is a refactor. CLI output, flags and exit codes, app text and behaviour, output file names
and paths, overwrite rules and rendered bytes are all unchanged. The typed planner errors and the
loop export button are #144 (PR B), approved by the user on 2026-11-25.

## Background

The [re-verification](0117-shared-export-module/reverify.md) has the evidence.

### Current behaviour (kept exactly)

| Aspect | CLI | App |
|---|---|---|
| Destination (mix) | `--out` file as given (intro-loop splits into `x-intro` and `x-loop`), or project `renders/<stem>[-suffix].<ext>`; outside a project, a usage error naming `--out` | chosen folder or project `renders/`, with the same names |
| Overwrite (mix) | always replaces silently | preview shows whether each file exists; "Replace audio files?" dialog; new files are created without clobbering; preview and chosen folder are checked again before each write |
| Stems | `--out` new dir, or project `renders/<stem>[-suffix]-stems`; never replaces | chosen folder or project; never replaces; no dialog |
| Conflicts | CLI wording and order: stems conflicts, then preset/length | app wording and order: length, then preset/length, then song, folder, stems, paths |
| Partial write | stderr `partial export: wrote A, failed to write B`, then exit 1 | `ExportError{message, written:[…]}` |
| Warnings | preset: deduped in first-seen order on stderr; JSON keeps per-file warnings | deduped across files |
| Progress | stderr lines (`planned`, `rendered`, `encoded`, `wrote <path>` …) | `Confirming`, `Planning`, pipeline events, `Writing` once, `Written` per file |
| Options | flags | fixed defaults (equal to the CLI's defaults, so bytes match) |

## Scope

In:

1. **A new `strudeland_render::export` module:**
   - an `ExportRequest`: song stem, `LengthRequest`, kind (`Mix{preset, output}` or
     `Stems{grouping}`), jobs, timeout and instruments;
   - an `ExportDestination`: `ExplicitPath`, `ChosenFolder` or `Project`;
   - `prepare(request, destination)`, which resolves every target without creating anything;
   - `Prepared::preview()`;
   - `Prepared::run(policy, progress)`, which renders, checks roles against the targets,
     publishes in order and dedupes warnings.

   It returns `Finished`, or a `PartialWrite` that carries the written paths and the failed
   target.
2. **One role table: `ExportPreset::roles()`.** `export_suffix`, the loop arrangement's `Part`s
   and the module all use it.
3. **Shared rules become typed `ExportProblem`s,** which each adapter words with its current
   text and check order:
   - loop presets take only a plain clip;
   - stems require a standard range and WAV;
   - stems refuse preset, format, bit depth and loudness options.
4. **Two replace policies:**
   - `ReplacePolicy::Always` for the CLI;
   - `ReplacePolicy::CreateNew{approved}` for the app, which re-checks the chosen folder and the
     approved preview, and creates new files without clobbering.

   Atomic publishing stays in `strudeland-project`.
5. **The CLI moves onto the module.** It keeps clap, `resolve_format`, the encoder flags, the
   mapping from problem to flag and exit code, `ProgressLines` (with the exact partial-export
   line) and the summaries and JSON.
6. **The app moves onto the module.** It keeps IPC, chosen-folder state, the export permit for
   Application Quit, its preview-time length check, the confirm dialog, `ExportProgress`
   (including `Writing`, sent from a module hook) and the error serialization.
7. **Delete the parity test** `names_and_roles_match_cli_for_every_format_and_preset`.
8. **`output.rs`'s resolvers become crate-private.** `write_output` stays public for the harness
   binary.
9. **`GLOSSARY.md`:** export request, destination, role, stem set.

Out:

- **Typed planner length errors and the dialog's flag-free wording.** That is #144 (PR B).
- **Disabling loop export without a marked loop.** That is #144 too.
- **Unifying the overwrite policy.** The CLI keeps replacing, as SKILL.md documents.
- **Moving the app's preview-time length check into the module.** It would change CLI exit codes
  for `--section ""`.
- **Any edit to `job-plan.ts` or other planner files.** No host bundle or refs churn.

## Technical design

### Placement

- **`crates/strudeland-render/src/export.rs`** holds the loop arrangement today. It is renamed to
  `loop_export.rs`, keeping the public `pub use` names stable.
- **The new `export/` directory:**
  - `request.rs`: the request, kind and problems;
  - `destination.rs`: the destinations and target resolution;
  - `publish.rs`: the policies, ordered publish and the partial-write error;
  - `mod.rs`: `prepare`, `Prepared` and `run`.
- **It stays inside `strudeland-render`,** so the app's macOS `host-jitless` feature still
  applies.

### Pipeline seam

`run` renders through an internal `Pipeline` trait:

- **The real adapter** calls `render`, `render_export` and `render_stems`.
- **Tests** use a fake that returns `ExportFile`s, as the CLI's `RenderBackend` fake does today.

The CLI's `RenderBackend` tests move onto it. Two adapters, real and fake, make it a real seam.

### Progress

The module forwards pipeline `Progress` events and adds:

- `Publishing`, before the first write;
- `Written(path)`, per file.

The CLI's `ProgressLines` ignores `Publishing` and prints `wrote <path>` as today. The app maps
`Publishing` to `Writing`, and `Written` to `Written`.

### Errors

The module's errors are:

- `ExportProblem`, for a rule broken before rendering;
- `RenderError`, passed through unchanged;
- `PartialWrite { written, failed, source }`.

The adapters map them:

- **The CLI** prints its current text and exit codes. A partial write prints the last written path
  and the failed target, exactly as today.
- **The app** maps to `ExportError { message, written }`, with its current text.

### Tests (replace, don't layer)

- **One table in `strudeland-render`:** {none, one-shot, tagged, folded, intro} × every format ×
  {explicit, chosen folder, project}. Each row asserts the roles, paths and `exists`, and that
  nothing is created.
- **Publish tests:**
  - `Always` overwrites;
  - `CreateNew` refuses an unapproved existing file;
  - a competing file created mid-publish is kept, and `written` lists the earlier file;
  - a chosen folder that changed is refused;
  - stems refuse an existing directory, and keep manifest-last ordering.
- **The CLI's output-format tests stay.**
- **The app's integration tests stay:** `split_wavs_…`, `stem_export_…`,
  `a_collision_during_publish_…`, and the `app_quit` tests.
- **The parity test is deleted.**

## Dependencies

- **Builds on:** #116 (done).
- **Blocks:** #118 (one finishing path) and #144 (PR B).

## Risks

- **Application Quit.** The module is synchronous and owns no lock. The app holds the export
  permit across the whole call, as today.
- **No-clobber.** Every app guarantee above is pinned by the kept integration tests and the new
  publish tests.
- **Byte reproducibility.** Options are unchanged per adapter, and no planner files change.
- **CLI test churn.** About 30 tests use the `RenderBackend` fake. Move them to the `Pipeline`
  fake rather than rewriting them.
- **SKILL.md.** CLI output must stay byte-identical. The CLI's `e2e.rs` and output tests are the
  guard.

## Complexity

Large: about 1,000-1,500 lines moved across three crates, with visible behaviour unchanged.
Fracture is skipped. Singletonize runs through the stage files, plus a light Loss check.

## Decisions

1. **Two PRs.** This one is behaviour-identical. Typed codes and loop export are #144 (user).
2. **A replace policy per adapter.** The CLI replaces; the app confirms and creates new files
   without clobbering.
3. **Typed `ExportProblem`s,** with each adapter keeping its own wording and check order.
4. **Partial writes** carry every written path. The CLI keeps printing only the last one.
5. **Warnings are deduped in first-seen order.** The CLI JSON keeps per-file warnings.
6. **`Publishing` is a module hook.** The CLI prints nothing new for it.
7. **The app's preview-time length check stays in the app.**
8. **Naming:** the loop arrangement moves to `loop_export.rs`, and `export/` is the new module.
   The glossary gains export request, destination, role and stem set.
9. **Public surface:** the resolvers become crate-private; `write_output` stays public.

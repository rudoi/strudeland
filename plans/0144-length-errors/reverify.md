# #144 Step 0 re-verification (main 0113154)

#117 has landed: `crates/strudeland-render/src/export/` exists, and the CLI and the app both run
it. #129, #130, #132, #134 and #136 have landed too. None of them touches the length errors except
#129, which adds `sections` to the plan and leaves the error paths alone. The task is still valid
as filed.

## 1. The flagged length errors today

All of them are `PlanJobError('meta', message, flag)` (`src/lib/render/job-plan.ts:55-69`). The
flag is free text. Nothing reads it except the host slot.

| # | Site (job-plan.ts) | Exact text | Params | Flag | Reachable from |
|---|---|---|---|---|---|
| A | 329-335 | `loop presets cannot be combined with --section, --length or --loops` | none | `--preset` | none: the CLI (`render.rs:43`) and the app (`export_audio.rs:206`) check `check_length_for_preset` first |
| B | 391-397 | `--loops needs meta.loop or a section with loop: true in meta.sections` | none | `--loops` | CLI |
| C | 399-403 | the `StructureError` message from `withLoops` | message | `--loops` | none: loops are already validated and the loop is already checked |
| D | 420-425 | `this song has no sections; pass --length N to say how many bars to render` | none | `--length` | CLI, app (clip on a song without sections) |
| E | 430-435 | `--loops ${loops} makes the clip ${clipBars} bars, past the render limit of ${MAX} bars` | loops, clipBars, limit | `--loops` | CLI |
| F | 436-442 | `the song's clip is ${clipBars} bars, past the render limit of ${MAX} bars; pass --length N` (`… or --loops N` when there is an authored loop) | clipBars, limit, hasLoop | `--length` | CLI, app |
| G | 448-452 | `this song has no sections; --section needs meta.sections` | none | `--section` | CLI; app only when the header and the evaluated song disagree |
| H | 456-460 | `unknown section '${name}' (sections: a, b, …)` | name, sections | `--section` | CLI; app only on a header/eval mismatch |
| I | 463-468 | `section '${name}' ends at bar ${toBar}, past the render limit of ${MAX} bars` | name, toBar, limit | `--section` | CLI, app; loop presets reach it too, through the single-section path at 499-506 |
| J | 474-479 | `--length ${bars} is past the end of the song (${clipBars} bars)` | bars, clipBars | `--length` | CLI, app |
| K | 492-497 | `--preset needs meta.loop or a section with loop: true in meta.sections` | none | `--preset` | CLI, app (today's dialog lets you start this) |
| L | 508-513 | `the loop ends at bar ${end}, past the render limit of ${MAX} bars` | end, limit | `--preset` | CLI, app |

How an error travels:
- TS → slot. `src/host/main.ts:77-85` `failure()` writes `{ ok:false, kind, error, flag? }`.
- Host. `crates/strudeland-host/src/host.rs:213-245` `read_slot` maps `("meta", Some(flag))` to
  `HostError::MetaFlag { flag, message }`, defined at `error.rs:17-18`. Its Display is
  `meta: {message}`, and `kind()` is `meta` (`error.rs:45`).
- strudeland-render. `RenderError::Plan(HostError)` (`src/error.rs:13-14`). `plan_text` prints the
  bare message for MetaFlag (`:74-79`). `exit_class` treats MetaFlag as `Usage` (`:64-71`).
- CLI. `crates/strudeland-cli/src/run.rs:80-82` maps `ExitClass::Usage` to exit 4 and prints
  `error: {message}`.
- App. `src-tauri/src/export_audio.rs:144-147` has `export_message`:
  `export::Error::Render(e) => e.to_string()`, so the dialog gets the CLI text, flags included.
  `run_error` (`:167-173`) wraps it as `ExportError { kind: other, message, written }`.
  `src/lib/export/error.ts` → `controller.ts:143-147` `failureFrom` → the danger Notice at
  `ExportAudio.svelte:346-347`. The error only appears after the user presses export: preview
  (`prepare_previewed`) does not plan.
- The render page (`src/render/main.ts`) and `scripts/render/host-planner.ts` never read `flag`.
  `host:parity` does not compare `errors`.

Where each text is pinned:
- TS planner: `tests/render-job-plan.test.ts:625-680` (D, G, H, J, B, with flag) and `:1466-1515`
  (F with and without loop, E, I). Not pinned anywhere: A, C, K, L.
- Host entry: `tests/host-entry.test.ts:247-260` pins the slot shape `{…, flag:'--length'}` (D).
- Host: `crates/strudeland-host/tests/plan_render.rs:295-330` (D, G, H, J, B). `tests/bin.rs:283`
  pins D with the `strudeland-host: meta: ` prefix. Unit tests: `error.rs:76-79,101-105` and
  `host.rs:386-391` build MetaFlag with `flag`.
- strudeland-render: `tests/pipeline.rs:317,324` (D, matched on `flag == "--length"`), `:341,347`
  (E, `flag == "--loops"`). `tests/bin.rs:272,286` (D, H). `src/error.rs:128-133` (Display and
  exit class).
- CLI: `src/render.rs:1332-1341`, a synthetic MetaFlag that exits 4. `tests/e2e.rs:576-592` (K: exit
  4, stderr contains `loop: true`) and `:595-608` (D: exit 4).
- SKILL.md: `:488-491` and `:563-567` describe the behaviour and exit 4. No exact text is quoted,
  so it stays unchanged.
- IPC: `src/lib/ipc/bindings.ts` holds only command names and args (`:8-11`, `:60-63`), events
  and `COMMAND_ERROR_KINDS` (`:116`). The ExportError shape is not generated: `written` is parsed
  by hand in `error.ts:21-26`.

## 2. The dialog today

- Every reachable error (D, F, I, J, K, L, and G or H on a mismatch) appears verbatim after export
  is pressed. For example: `this song has no sections; pass --length N to say how many bars to
  render`.
- Loop preset on a song with no marked loop:
  - The radio is selectable. Presets are never disabled; only `locked` disables the fieldset
    (`ExportAudio.svelte:127`).
  - The range shows `marked loop` (`:212`).
  - The warning is computed at `:51-57` (`markedLoop` from `authoredLoop(source.structure)`, the
    header structure as typed, `controller.ts:74`). It is shown at `:266-270`:
    `loop export needs a <code>meta.loop</code> range or a section with <code>loop: true</code>`.
  - `canExport` (`:63`) ignores this. The preview goes `ready`, so export is enabled, and pressing
    it fails with K's CLI text.
- Dialog tests: `tests/export-dialog.test.ts`.

## 3. Design

| Decision | Options | Recommended |
|---|---|---|
| Where the code enum lives | (a) TS union in job-plan.ts plus a serde enum `LengthProblem` in `strudeland-host`, re-exported by strudeland-render; (b) the Rust enum in strudeland-render only | **(a)**. The host is the one that deserializes the slot, and `HostError` must carry the code |
| Slot shape | add `code`/params beside `flag`; or replace `flag` with `length: { code, …params }` | **Replace.** `flag` becomes a pure function of the code, in TS (`lengthFlag`) and in Rust (`LengthProblem::flag()`). Use `#[serde(tag = "code", rename_all = "camelCase")]`. An unknown code is a `Runtime("malformed result slot …")`. Every code is exercised through real V8 in `plan_render.rs` |
| `HostError::MetaFlag` | `{ flag, message }` → `{ problem: LengthProblem, message: String }` | **Do it**, with `fn flag()`. Display stays `meta: {message}` and `exit_class` is unchanged. Update the 6 construction sites listed in section 1 |
| CLI text | format it in Rust from the code and params; or carry the TS message | **Carry the message.** TS stays the single owner of the CLI wording, so CLI output is byte-identical by construction and needs no TS↔Rust format parity test |
| Params | per code: `loops`, `clipBars`, `hasLoop`, `name`, `sections: string[]`, `toBar`, `bars`, `end`, `limit`; C carries `message` | **Carry `limit`** rather than relying on `MAX_BARS`. No parity test pins `MAX_BARS == MAX_RENDER_BARS` today |
| Codes | A `presetWithLength`, B `loopsWithoutLoop`, C `loopsInvalid`, D `noSections`, E `loopsPastLimit`, F `clipPastLimit`, G `sectionWithoutSections`, H `unknownSection`, I `sectionPastLimit`, J `lengthPastEnd`, K `presetWithoutLoop`, L `loopPastLimit` | **12 codes, one per site**. Keep C so nothing becomes untyped |
| Where the app wording map lives | (a) Rust, in the shared export module: `crates/strudeland-render/src/export/wording.rs`, `pub fn dialog_text(&LengthProblem) -> String`, an exhaustive match, called from `export_audio.rs` `export_message`; (b) Rust, in `export_audio.rs`; (c) TS `src/lib/export/`, which needs the code over IPC | **(a)**, per the task note. #117's "each adapter words it" still holds for `export::Error`. This is the planner-problem table the CLI does not need, and the compiler forces every new code to get a dialog wording. With no IPC field and no new `CommandErrorKind`, `ipc:build` is not needed |

## 4. Obligations

- Planner edit: `job-plan.ts` is in `PLANNER_FILES` (`scripts/engine/provenance.ts:30-39`), and
  `job-plan.ts`, `src/host/main.ts` and `structure.ts` are in `host.inputs.json`. That means:
  - `bun run host:build`
  - `bun run engine:refs --write` (the provenance hash only; check that the diff touches nothing
    but the hash)
  - `bun run host:parity --batch`
  - `cargo test -p strudeland-host`
- Chromium is installed (`~/Library/Caches/ms-playwright/chromium-1248`, headless shell
  1217/1248).
- Engine jobs and goldens must not change. Only throw sites change, and no job text does. No
  `render:check --update-goldens`.
- IPC: under the recommendation nothing changes (the `ExportError` shape is the same, and only its
  message differs), so no `ipc:build`.
  - If option (c) were chosen, `ExportError` would gain a field. That is a shape change to an
    approved command's rejection. It is not covered by `bindings.ts`, but it is still a wire
    change.
  - #144 is user-approved for typed codes, and that approval covers the internal slot and
    `HostError` change.
- CLI: output and exit codes are unchanged, so SKILL.md is unchanged. The `--info` file and the
  control socket are untouched.

## 5. Dialog wordings (user-visible copy; flag in the PR)

These are lowercase like the rest of the dialog. The errors are app-worded, so lowercase applies.

| Code | Dialog text |
|---|---|
| noSections | `this song has no sections; choose a length in bars` |
| sectionWithoutSections | `this song has no sections; choose a length in bars` |
| unknownSection | `this song has no section '{name}' (sections: {list}); choose another section` |
| lengthPastEnd | `{bars} bars is past the end of the song ({clipBars} bars); choose {clipBars} bars or fewer`, with each count's `bars` written `bar` when the count is 1 (`2 bars is past the end of the song (1 bar); choose 1 bar or fewer`) (singular added in the plan refinement) |
| clipPastLimit | `the song's clip is {clipBars} bars, past the render limit of {limit} bars; choose a length in bars` |
| sectionPastLimit | `section '{name}' ends at bar {toBar}, past the render limit of {limit} bars` |
| loopPastLimit | `the loop ends at bar {end}, past the render limit of {limit} bars` |
| presetWithoutLoop | `loop export needs a meta.loop range or a section with loop: true`. This is a backstop after item 2 and matches the warning |
| presetWithLength | `loop exports use the marked loop section`, the existing app text (`export_audio.rs:207`) |
| loopsWithoutLoop | `the loop count needs a meta.loop range or a section with loop: true`. The app cannot reach it |
| loopsPastLimit | `{loops} loop passes make the clip {clipBars} bars, past the render limit of {limit} bars`. The app cannot reach it |
| loopsInvalid | `{message}`. The app cannot reach it |

## 6. Item 2: disable export for a loop preset without a marked loop

- Recommended: the controller adds an exported pure `loopUnavailable(song, options)`, true when
  the mode is a loop mode and `authoredLoop(song.structure)` is null.
  - The component uses it in `canExport` and in `exportHelp`: `Mark a loop in the song to export
    a loop preset.` (contextual help, sentence case per `docs/writing-contextual-help.md`).
  - The radios stay selectable, so the warning can explain why, and the warning stays unchanged.
  - Move the `markedLoop` derivation onto the same helper so the two cannot disagree.
- Known limit: the check reads the header as typed, while the planner reads the evaluated
  structure. A song whose header lacks the loop but whose `song()` registers one would be blocked.
  The dialog already words its range options from the header, so accept this and note it in the
  PR.

## 7. New callers or copies since filing

- No new consumer of `flag` and no new copy of these texts.
- New sites to update: the #117 export module's `export_message` path (`export_audio.rs:144`) and
  the CLI's `cli_error` (`render.rs:70`, which passes `Render` through).
- `src/lib/control/handler.ts:219` has a separate `unknown section '…'` text for the control
  socket. It is not a planner error and stays out of scope.
- The e2e preset test (`e2e.rs:576`) is new since filing and stays green, because the CLI text is
  unchanged.

## Recommendation

Proceed with the scope as filed, in one PR with two commits:

1. Typed codes. The TS union, the slot `length`, `LengthProblem` with `HostError::MetaFlag {
   problem, message }`, `export::dialog_text` and the `export_message` arm. Then `host:build`,
   `engine:refs --write` and `host:parity --batch`.
2. The dialog. `loopUnavailable` disables export, with its help text.

There is no stage split: neither commit is large, and commit 2 does not depend on commit 1. All
the decisions above take the recommended answer: carry the message, replace `flag`, the wording
in the Rust export module, no IPC change, and the 12 codes and wordings in section 5.

## Decisions (2026-11-27)

The visible changes were approved by the user on 2026-11-25. These answers follow the recommendations above.

- **Codes:**
  - There are 12 typed codes, one per error site. The TS planner throws a code plus params, and the slot field `flag` becomes `length: { code, …params }`.
  - In Rust, a serde `LengthProblem` enum lives in `strudeland-host` and is re-exported by `strudeland-render`. `HostError::MetaFlag` becomes `{ problem, message }`, and the flag is derived from the code.
  - The render limit travels as a parameter.
- **CLI:** the CLI carries the TS message unchanged. Its output stays byte-identical, exit 4 is kept and `SKILL.md` is unchanged.
- **Dialog wording:**
  - An exhaustive `dialog_text` goes in the new `crates/strudeland-render/src/export/wording.rs` and is called from `export_message`. This is the shared export module, as the task asks.
  - The wording follows the drafts in §5. That copy is user-visible, so the PR lists it.
- **No IPC shape change:** no `ipc:build` is needed.
- **Loop presets:**
  - A pure `loopUnavailable(song, options)` in the export controller disables export, with the help text "This song marks no loop; mark one to export a loop preset." (updated in the plan refinement: it leads with the reason, as `docs/writing-contextual-help.md` point 2 asks; it supersedes §6's draft)
  - The radios and the warning are unchanged.
  - Accepted limit, noted in the PR: the check reads the header as typed, the same way the dialog builds its range options.
- **Out of scope:** the control socket's "unknown section" text.
- **PR shape:** one PR in two commits, (1) the typed codes and dialog wording, then (2) the loop-preset gate.

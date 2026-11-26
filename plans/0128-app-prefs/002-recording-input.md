# 002: the recording input as an app data document

## Goal

`recording.json` becomes `RecordingInput` in `strudeland-project`, on stage 001's machinery, with
the same file name and the same bytes. `strudeland-record`'s `takes::prefs` keeps only the
selection logic. `Takes` takes `&AppData` instead of a file path. The app drops `prefs_path` and
its `PREFS_FILE`. `Takes` still reads at the same points, still writes only when `arm` was given
an argument, and a failed save still only logs.

## Prerequisites

- Stage 001 committed and green.
- The proposal section "`RecordingInput` (stage 002)".

## Files

- New: `crates/strudeland-project/src/recording_input.rs`.
- Edited:
  - `crates/strudeland-project/src/lib.rs`;
  - `crates/strudeland-record/src/takes/{prefs.rs,mod.rs,testing.rs,tests.rs}`;
  - `src-tauri/src/recording.rs`;
  - the adapter region of `src-tauri/src/recording/transcript.rs` (above `// --- frozen: begin ---`);
  - `src-tauri/src/recording/rig.rs`, only if it calls `prefs()` for a `Takes` verb;
  - `AGENTS.md`.
- Untouched (besides the shared list):
  - `src/`;
  - `crates/strudeland-project/src/{prefs,recents,app_data,instruments}.rs`, unless a doc line
    cross-references the new module;
  - `src-tauri/src/project.rs`.

## Steps

1. **Characterize first.** In `crates/strudeland-record/src/takes/prefs.rs`, extend
   `prefs_round_trip` on today's code with two assertions:
   - a `None` device writes exactly `"{\n  \"schema\": 1,\n  \"deviceId\": null,\n  \"channel\": 0\n}\n"`;
   - `{"schema":1,"deviceId":"x"}` reads as the defaults.

   Run `cargo test -p strudeland-record --lib takes::prefs`, which must pass. These literals,
   and the existing `coreaudio:abc` literal, move unchanged into step 2's tests.
2. **`recording_input.rs`**, exactly as the proposal's sketch:
   - `RECORDING_INPUT_FILE`, `RECORDING_INPUT_SCHEMA = 1`;
   - `RecordingInput { device_id, channel }` with `Default`, `Serialize`, `Deserialize` and
     `rename_all = "camelCase"`, no `serde(default)` and no `skip_serializing_if`;
   - `file(app)`, `load(app)` and `save(&self, app)` over `app_data::load`/`save`;
   - a module doc: the saved input choice, read by `strudeland-record`'s `Takes`.

   `lib.rs`: `mod recording_input;` and
   `pub use recording_input::{RECORDING_INPUT_FILE, RecordingInput};`.
3. **`takes/prefs.rs`**:
   - delete `PREFS_SCHEMA`, `Prefs`, `PrefsFile`, `parse_prefs`, `read_prefs` and `write_prefs`;
   - `resolve_selection(devices, input: &RecordingInput)` and
     `saved_channel(device, input: &RecordingInput)` keep their bodies, with `prefs.` renamed to
     `input.`;
   - the module doc: the input selection over the saved `RecordingInput`;
   - the file keeps its name. Renaming it is churn with no reader benefit.
4. **`takes/mod.rs`**:
   - delete `pub use prefs::{Prefs, write_prefs};` and change the private `use` to
     `use prefs::{resolve_selection, saved_channel};`;
   - add `fn read_input(app: &AppData) -> RecordingInput`, which loads, prints
     `eprintln!("recording: {problem}")` when there is one, and returns the value;
   - `status`, `devices`, `arm`, `armed_status` and `disarm` take `app: &AppData`. Each
     `read_prefs(prefs_path)` becomes `read_input(app)`, at the same line;
   - `arm`'s save (`:642-649`) builds `RecordingInput { device_id: Some(device.id.clone()),
     channel }` under the same `if`, and on `Err(e)` prints
     `eprintln!("recording: cannot save {}: {e}", RecordingInput::file(app).display())`;
   - the doc comments that say "prefs" say "the saved input".
5. **`takes/testing.rs`**: `TakesRig::app_data(&self) -> AppData` is
   `AppData::at(self.dir.join("data"))`. `prefs()` returns `RecordingInput::file(&self.app_data())`,
   the same path as today. `arm()` calls `self.takes.arm(&self.app_data(), None, None)`. The
   struct doc's "with its prefs under `dir/data`" becomes "with its app data under `dir/data`".
6. **`takes/tests.rs`**: mechanical:
   - `rig.prefs()` passed to a verb becomes `&rig.app_data()`;
   - `write_prefs(&rig.prefs(), &Prefs { .. })` becomes
     `RecordingInput { .. }.save(&rig.app_data())`;
   - `read_prefs(&rig.prefs())` becomes `RecordingInput::load(&rig.app_data()).value`;
   - `std::fs::*(rig.prefs())` file assertions stay as they are.

   The `// --- prefs ---` section heading becomes `// --- the saved input ---`.
7. **`src-tauri/src/recording.rs`**:
   - delete `PREFS_FILE` (`:34`) and `prefs_path` (`:427-432`);
   - in `recording_status`, `recording_devices`, `recording_arm` and `recording_disarm`,
     `let prefs = prefs_path(&app)?;` becomes
     `let app_data = app.state::<AppDataState>().get()?.clone();` (import
     `crate::project::AppDataState`), and the closure passes `&app_data`;
   - drop `PathBuf` from the imports if it becomes unused.
8. **The transcript's adapters** (above `// --- frozen: begin ---` only):
   - the import `use strudeland_record::takes::{Prefs, RepairOutcome, write_prefs};` becomes
     `use strudeland_record::takes::RepairOutcome;` plus `use strudeland_project::RecordingInput;`;
   - `arm`, `spawn_arm` (clone `self.rig.app_data()` into the thread), `disarm`, `status` and
     `devices` pass `&self.rig.app_data()`;
   - `write_prefs` builds a `RecordingInput` and calls `.save(&self.rig.app_data())`, keeping
     its label and `json!(result.is_ok())`.

   The frozen `prefs_text` keeps calling `r.prefs()`. The adapter method keeps the name
   `write_prefs`, because frozen flows call `s.write_prefs(...)`.
9. **`AGENTS.md:310`**: "(device prefs in `recording.json` in the app data dir)" becomes "(the
   saved input is `strudeland-project`'s `RecordingInput`, `recording.json` in the app data
   dir)". Add `recording.json` to the stage 001 `strudeland-project` bullet's document list.

## Tests

- **New, `recording_input.rs`:**
  - `today_text_loads_and_resaves_byte_identically`: the `coreaudio:abc` text and the
    `deviceId: null` text from step 1, each written by hand, load as the expected value, and
    `save` writes the same bytes back;
  - `channel_is_required_and_device_is_optional`: `{"schema":1,"channel":2}` gives `{None, 2}`;
    `{"schema":1,"deviceId":"x"}` gives the default with `Some` problem;
  - `bad_files_give_defaults_stay_in_place_and_are_reported`: the six texts of today's
    `prefs_parse_errors_give_defaults` (`:108-130`). Each gives `RecordingInput::default()` and
    a problem starting `ignoring <file>: ` that contains the serde message head without its
    position (`EOF while parsing an object`, `missing field \`schema\``,
    ``invalid type: integer `3`, expected a string``, ``invalid value: integer `-1`, expected u16``,
    `invalid type: string "0", expected u16`). The schema case gives exactly
    `ignoring <file>: schema must be 1`. The file still holds its bytes afterwards. A directory
    in the file's place gives a `cannot read <file>: ` problem.
- **Deleted, `takes/prefs.rs`:** `prefs_parse_errors_give_defaults` and `prefs_round_trip`. The
  tests above replace them.
- **Kept, `takes/prefs.rs`:** `selection_resolution` and `saved_channel_cases`, over
  `RecordingInput`, with the same assertions.
- **Kept, `takes/tests.rs`:** every test, with the adapter edits only. Among them are
  `arm_writes_prefs_only_when_it_carried_an_argument` (the file text is unchanged after a plain
  arm), `a_prefs_write_failure_does_not_fail_the_arm`, and
  `devices_and_idle_status_use_the_saved_choice`.
- **The transcript:** its `prefs` flow's four `"prefs file"` texts and `exclusive_input`'s saved
  input are the app-level byte check. The expected block is unchanged.
- **Targeted while working:**
  - `cargo test -p strudeland-project recording_input::`;
  - `cargo test -p strudeland-record --lib takes::`;
  - `cargo test -p strudeland-app --lib recording::`.

## Validation

Run the gates in [000-prompt.md](000-prompt.md) with `N=002`. Also:

```sh
git diff --stat HEAD -- src src-tauri/src/project.rs crates/strudeland-project/src/prefs.rs \
  crates/strudeland-project/src/recents.rs crates/strudeland-project/src/app_data.rs \
  > "$S/0128-002-own-untouched.log"; echo "exit $?" >> "$S/0128-002-own-untouched.log"
git grep -n 'recording.json' -- crates src-tauri > "$S/0128-002-names.log"; echo "exit $?" >> "$S/0128-002-names.log"
```

- `own-untouched.log` lists no files. It diffs against the stage 001 commit (`HEAD` before you
  commit).
- `names.log` shows `"recording.json"` defined once, in `recording_input.rs`. Other hits may be
  doc comments and test strings only.
- The transcript log is clean: the expected block and the frozen region are unchanged.

## STOP

- Today's `recording.json` text would load a different value or re-save different bytes.
- A file without `channel` would load the saved device.
- A `Takes` verb would read or write the file at a point other than today's, or a failed save
  would fail the arm.
- The transcript's frozen region or expected block would have to change, or a frozen call
  (`r.prefs()`, `s.write_prefs(...)`) would stop compiling without an edit inside the region.
- Anything else in the shared STOP list.

## Commit message

```
refactor(record): keep the saved input in the app data documents

recording.json becomes RecordingInput in strudeland-project, on the
same load and save as the other app data documents, with the same name
and bytes. Takes reads and writes it through AppData, at the same points
as before; the recorder crate keeps only the input selection.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

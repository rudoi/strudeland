# 001: `AppData` and the document machinery

## Goal

One owner for the app data dir. `AppData` locates it without Tauri, and one private `load`/`save`
pair implements the bad-file policy and the writer. `Prefs` and `Recents` move onto it.
`InstrumentLibrary::open_default` builds on it. The app resolves it once into managed state and
logs every problem the machinery returns. No saved byte, value, command result or binding
changes. The only new output is the stderr warning for a bad `prefs.json` or `recent.json`.

## Prerequisites

- The proposal sections "`app_data.rs`", "Prefs and Recents", "The instrument library" and
  "The app" (stage 001).
- Clean tree on `8c13c8c` plus the plan files.

## Files

- New: `crates/strudeland-project/src/app_data.rs`.
- Edited:
  - `crates/strudeland-project/src/{lib.rs,error.rs,prefs.rs,recents.rs,instruments.rs}`;
  - `src-tauri/src/{project.rs,lib.rs,command_error.rs}`;
  - `src-tauri/src/recording.rs`, one test call only (step 8);
  - `AGENTS.md`.
- Untouched (besides the shared list):
  - `crates/strudeland-record/`;
  - `src/`;
  - `src-tauri/src/recording.rs` except its `open_project` test call, and
    `src-tauri/src/recording/`;
  - `src-tauri/src/instruments.rs`.

## Steps

1. **Characterize first.** Before any other edit, add the two byte tests to today's code:
   - `prefs.rs`: `Prefs { vim, audition_notes, format_on_save, piano_roll_open }` all `true`
     saves exactly
     `"{\n  \"schema\": 1,\n  \"vim\": true,\n  \"auditionNotes\": true,\n  \"formatOnSave\": true,\n  \"pianoRollOpen\": true\n}\n"`.
     `Prefs::default()` saves the same text with every value `false`. Writing either text by hand
     and loading it gives the value back.
   - `recents.rs`: touching `/p/a` then `/p/b` and saving writes exactly
     `"{\n  \"schema\": 1,\n  \"projects\": [\n    \"/p/b\",\n    \"/p/a\"\n  ]\n}\n"`.

   Run `cargo test -p strudeland-project -- prefs:: recents::` (cargo takes one positional
   filter; several go after `--`). Both must pass on unchanged code.
   From here on, their expected literals do not change in this stage. Only the calls around them
   do (`&AppData` in place of a path).
2. **`error.rs`**: add `NoDataDir` with `#[error("cannot locate the user data directory")]`.
   In `src-tauri/src/command_error.rs`, add it to the `K::Other, None` arm (`:64-74`) and add a
   case to the table test next to the `Io` case (`:229`).
3. **`app_data.rs`**: write it exactly as the proposal's sketch:
   - `APP_IDENTIFIER` (`pub(crate)`), moved from `instruments.rs:18`;
   - `AppData { dir }` with `locate`, `at`, `dir`, `file`;
   - `Loaded<T>`;
   - private `OnDisk<T> { schema, #[serde(flatten)] value }`;
   - `pub(crate) fn load` and `pub(crate) fn save`.

   `load` follows the policy table row by row:
   - `fs::read` (bytes, not `read_to_string`, as `read_prefs` does today, so invalid UTF-8 stays
     a parse problem). `NotFound` gives the default with no problem. Any other error gives
     `cannot read {file.display()}: {e}`;
   - `serde_json::from_slice::<OnDisk<T>>`. An error gives `ignoring {file.display()}: {e}`;
   - a schema mismatch gives `ignoring {file.display()}: schema must be {schema}`.

   `save` is today's `Prefs::save` body, generic:
   - `create_dir_all` of a non-empty parent, with `ProjectError::io(parent, e)`;
   - `to_string_pretty(&OnDisk { schema, value })` with
     `.expect("app data documents always serialise")`;
   - push `'\n'`, then `crate::project::atomic_write(file, &json)`.

   `OnDisk` borrows for save, using `OnDisk<&T>` or a separate `OnDiskRef<'a, T>`, so no clone is
   needed. Pick whichever compiles cleanly. The bytes are the same.
4. **`prefs.rs`**:
   - delete `PrefsFile`;
   - `Prefs::load(app: &AppData) -> Loaded<Prefs>` is
     `app_data::load(&app.file(PREFS_FILE), PREFS_SCHEMA)`;
   - `Prefs::save(&self, app: &AppData)` is `app_data::save(&app.file(PREFS_FILE), PREFS_SCHEMA, self)`;
   - the module doc says the prefs are one document of the app data dir.
5. **`recents.rs`**:
   - `pub const RECENTS_FILE: &str = "recent.json"`;
   - replace `RecentsFile` with private `#[derive(Default, Serialize, Deserialize)] struct
     RecentsDoc { projects: Vec<PathBuf> }`, without `serde(default)`;
   - `Recents::load(app) -> Loaded<Recents>` loads the doc and then runs today's reverse `touch`
     loop. `file` is `app.file(RECENTS_FILE)`;
   - `touch_and_save(app, root) -> Option<String>` returns the load's problem;
   - `load_pruned(app) -> Loaded<Recents>`;
   - `list(app) -> Loaded<Vec<RecentProject>>`;
   - `save` writes `RecentsDoc { projects: self.entries.clone() }`, or a borrow, through
     `app_data::save(&self.file, RECENTS_SCHEMA, …)`;
   - `save_best_effort` is unchanged;
   - the module doc names `recent.json` and the app data dir.
6. **`instruments.rs`**:
   - delete `APP_IDENTIFIER`;
   - add `pub fn open_in(app: &AppData) -> Result<Self, ProjectError>`. It is today's
     `open_default` body with `base = app.dir().parent()` and the app dir name
     `app.dir().file_name()`, and returns `invalid(INSTRUMENTS_DIR, "cannot locate user data
     directory")` when either is missing. Keep the `create_dir_all`, `canonicalize`,
     `checked_directory`, `create_directory`, `checked_directory`, `create_directory`,
     `open_store` sequence exactly;
   - `open_default()` becomes `AppData::locate()`, with `.map_err` turning `NoDataDir` (the only
     error `locate` returns) into that same `invalid(...)`, then `open_in`. Its doc says it is
     the library in the located app data dir;
   - move `default_directory_matches_desktop_identifier` (`:1169-1175`) to `app_data.rs`
     unchanged, with `include_str!("../../../src-tauri/tauri.conf.json")`.
7. **`lib.rs`**:
   - `mod app_data;` and `pub use app_data::{AppData, Loaded};`;
   - add `RECENTS_FILE` to the `recents` re-export;
   - the crate doc's list gains "the app data dir and its documents (preferences, recent
     projects)".
8. **`src-tauri/src/project.rs`**:
   - `pub struct AppDataState(Result<AppData, String>)`, with `pub fn locate() -> Self` and
     `pub fn get(&self) -> Result<&AppData, CommandError>` (`CommandError::other(msg.clone())`);
   - `fn logged<T>(what: &str, loaded: Loaded<T>) -> T`;
   - delete `RECENTS_FILE`, `app_data_file` and `recents_path`;
   - `install`, `create_project`, `open_project` and `open_last` take `app_data: &AppData`.
     `install` logs `touch_and_save`'s problem as `recents`, and `open_last` logs
     `load_pruned`'s problem;
   - the commands:
     - `project_create`, `project_open` and `project_last` add
       `app_data: State<'_, AppDataState>` and keep `app` for the watcher;
     - `recent_list`, `prefs_get` and `prefs_set` replace `app: AppHandle` with it;
     - each calls `app_data.get()?` where it called `recents_path(&app)?` or `app_data_file`;
     - `prefs_get` returns `logged("prefs", Prefs::load(app_data))`;
     - `prefs_set` returns `prefs.save(app_data).map_err(CommandError::from)`;
   - drop the `PREFS_FILE` and `PathBuf` imports if they become unused, and update the module
     doc (`:3-5`);
   - in the tests, `Env` gains `fn app_data(&self) -> AppData { AppData::at(self.tmp.path().join("data")) }`,
     and every `&env.recents()` passed to a function becomes `&env.app_data()`. `env.recents()`
     stays `tmp/data/recent.json`, built as `self.app_data().dir().join(RECENTS_FILE)`, for the
     file assertions. `recent_entries` uses `Recents::load(&self.app_data()).value`;
   - `src-tauri/src/recording.rs:962-963`, the test
     `opening_a_project_repairs_it_in_the_background`, calls
     `crate::project::open_project(&rig.projects, &recents, …)`. Pass
     `&AppData::at(rig.tmp.path())` instead (add `AppData` to the test module's
     `strudeland_project` import, `:589`) and drop its `recents` local (the file is still
     `rig.tmp/recent.json`). This is the only `recording.rs` edit in this stage.
9. **`src-tauri/src/lib.rs`**: `.manage(project::AppDataState::locate())` after
   `.manage(project::ProjectState::default())`.
10. **`AGENTS.md:159-160`**: the `strudeland-project` bullet says it owns the app data dir and
    its documents. It names `AppData` (Tauri-free location, the one load and save with one
    bad-file policy) and lists `prefs.json` and `recent.json`, with `recording.json` added in
    stage 002.

## Tests

- **New, `app_data.rs`:** the machinery list in the proposal's Tests, over a private `Doc { flag:
  bool, name: Option<String> }` with schema 1. These cover:
  - missing;
  - corrupt (`""`, `"{"`, `"[]"`), with the bytes unchanged afterwards;
  - unreadable (a directory in the file's place);
  - wrong schema (`2`, `0`, `"1"`, missing), with the exact `schema must be 1` text;
  - unknown keys;
  - parent creation;
  - atomic overwrite (one file left);
  - the two save errors;
  - exact `Doc` bytes;
  - `locate()` equals `dirs::data_dir().unwrap().join(APP_IDENTIFIER)`;
  - the moved identifier test.

  For the problem texts, assert `starts_with(&format!("ignoring {}: ", file.display()))` or the
  `cannot read` form, plus the exact schema text.
- **`prefs.rs`:**
  - keep the step 1 byte test, `missing_keys_default` (now over `Prefs::load(&app).value`) and
    the default assertions from `missing_file_gives_defaults`, as a `defaults_are_off` test;
  - delete `missing_file_gives_defaults`, `corrupt_file_gives_defaults`,
    `unreadable_file_gives_defaults`, `wrong_schema_gives_defaults`,
    `unknown_fields_are_ignored`, `save_creates_the_parent_dir`,
    `round_trip_writes_pretty_json_with_a_newline` and `save_error_is_returned`.
- **`recents.rs`:**
  - keep the step 1 byte test (it replaces `save_creates_the_parent_and_round_trips`; keep that
    test's `loaded == recents` assertion);
  - keep `load_reads_entries_in_order_deduped_and_capped`,
    `touch_moves_to_front_and_dedupes`, `touch_caps_at_max_recents`,
    `prune_drops_folders_without_project_json`, `touch_and_save_moves_to_front_and_persists`,
    `touch_and_save_ignores_a_failed_write`, `load_pruned_writes_the_pruned_list_back`,
    `load_pruned_leaves_the_file_alone_when_nothing_is_pruned`,
    `open_each_skips_entries_that_do_not_open`, `list_returns_names_and_drops_dead_entries`,
    `recent_project_serialises_camel_case` and `save_reports_an_unwritable_parent`, each over
    `AppData::at(...)`;
  - `list_returns_names_and_drops_dead_entries`' last line (`Recents::list` of a missing file)
    uses its own empty dir, `Recents::list(&AppData::at(tmp.path().join("missing"))).value`, so
    it still lists nothing;
  - `touch_and_save_ignores_a_failed_write` uses `AppData::at(blocker)`, so the file is
    `blocker/recent.json`;
  - `corrupt_file_loads_empty` keeps only `{"schema":1}` and `{"schema":1,"projects":[1]}`, each
    giving an empty list and `Some` problem;
  - delete `missing_file_loads_empty`, `other_schema_loads_empty` and
    `save_leaves_no_temp_file_and_overwrites`.
- **`instruments.rs`:** a new `open_in_creates_the_library_under_the_app_data_dir`.
  `open_in(&AppData::at(tmp.join("data").join("com.rudoi.strudeland")))` succeeds, `path()` ends
  in `com.rudoi.strudeland/instruments` under the canonical `tmp/data`, and `entries()` is empty.
  The moved identifier test lives in `app_data.rs`.
- **`src-tauri`:** the `project.rs` tests pass with only the `Env` adapter edits, and
  `command_error.rs` gains its `NoDataDir` row.
- **Targeted while working:**
  - `cargo test -p strudeland-project -- app_data:: prefs:: recents:: instruments::`;
  - `cargo test -p strudeland-app --lib -- project:: command_error:: recording::tests::opening_a_project`.

## Validation

Run the gates in [000-prompt.md](000-prompt.md) with `N=001`. Also:

```sh
git diff --stat 8c13c8c -- crates/strudeland-record src src-tauri/src/recording \
  src-tauri/src/instruments.rs > "$S/0128-001-own-untouched.log"; echo "exit $?" >> "$S/0128-001-own-untouched.log"
git diff 8c13c8c -- src-tauri/src/recording.rs > "$S/0128-001-recording-rs.log"; echo "exit $?" >> "$S/0128-001-recording-rs.log"
```

- `own-untouched.log` lists no files.
- `recording-rs.log` shows only the `open_project` call in
  `opening_a_project_repairs_it_in_the_background` (and its dropped `recents` local).
- The step 1 byte tests are the byte check. Say in your report that they passed on unchanged
  code before the refactor.

## STOP

- Either step 1 byte test fails on unchanged code, or needs its literal edited after step 1.
- A kept `recents.rs` or `project.rs` test needs a changed assertion, not just a changed call.
- `open_default`'s path, checks or error text would differ from today.
- `bindings.ts`, a command's result or error kind, or a test in `tests/` would change.
- Anything else in the shared STOP list.

## Commit message

```
refactor(project): one app data module for prefs and recents

AppData locates the app data dir without Tauri and owns the one way its
documents are loaded and saved: a bad file gives the defaults, stays in
place and is reported to the caller. Prefs and the recent projects keep
their files and bytes; the instrument library builds its directory on
it, and the app resolves it once and logs what it reports.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

# 0128 step 0: re-verification on main 8c13c8c

The issue stands.

## The stores

| Store | Where | Bad file | Writers and readers |
|---|---|---|---|
| `prefs.json` | `crates/strudeland-project/src/prefs.rs` (`PREFS_SCHEMA=1`). Fields: `vim`, `auditionNotes`, `formatOnSave`, `pianoRollOpen`. | Silently falls back to defaults (`load` :43-50). `save` is atomic, pretty and ends in `\n`. | `prefs_get`/`prefs_set` (`src-tauri/src/project.rs:290-301`). `prefs_set` replaces the whole object. The frontend sends `{...initialLoad, ...edits}` (`store.ts:57-60`). |
| `recording.json` | Since #125, `crates/strudeland-record/src/takes/prefs.rs`. The filename constant is at `src-tauri/src/recording.rs:34`. It holds `{schema:1, deviceId, channel}`. | Falls back to defaults with an `eprintln!` and leaves the file in place (`read_prefs` :40-52). `write_prefs` is atomic. | `Takes::status/devices/arm/disarm(prefs_path)` read it on every call. Only `arm` writes it, and only when given explicit args; a failed write is logged. The path comes from `recording.rs:427-432`. |
| `recent.json` | `crates/strudeland-project/src/recents.rs` (`RECENTS_SCHEMA=1`, at most 10 projects). The filename constant is at `project.rs:29`. | Silently loads empty. | `project_create/open/last`, `recent_list`. |
| localStorage `strudeland.limiter` | `src/lib/audio/limiter-pref.ts` (`'1'`/`'0'`, off by default) | n/a | Read once at `createRuntime({limiter: loadLimiter()})` (`app.ts:116`). Written by a runtime-store subscriber (`app.ts:120-125`). |

- Nothing else in `src/` uses localStorage.
- The instrument library also lives in the app data dir (`InstrumentLibrary::open_default`, instruments.rs:440-453 with its doc comment). Both the CLI and the app use it.
- The app data dir is resolved in three places: `project.rs:239-244`, `recording.rs:427-432` and `instruments.rs:442`.
- Tauri 2.12.5's `app_data_dir` is `dirs::data_dir().join(identifier)` when tauri.conf sets no directory override, and it sets none. So a Tauri-free lookup gives the same path on every OS.
- `APP_IDENTIFIER` is defined in two places, `instruments.rs:18` and `strudeland-ctl/src/path.rs:9`. Tests check both against tauri.conf.
- The CLI and the control socket expose no prefs. `state.levels.limiter` reports the limiter value the runtime applied, not the stored one.
- `bindings.ts` types `prefs_set` as `{ prefs: unknown }`, so adding a field to `Prefs` needs no `ipc:build`.

## User decisions (2026-11-26)

- **The limiter moves into prefs.** Accepted visible changes:
  - In a plain browser (`bun run dev`) it no longer persists across reloads, like every other pref there.
  - It becomes known when `prefs_get` resolves, a few milliseconds after launch. A play pressed inside that window could start unlimited.
  - A failed save shows the existing "prefs: …" notice.
  - The current value migrates once. The localStorage key is removed only after a successful save.
- **No CLI `prefs` command.**

## Data safety

- `recording.json` stays its own document with the same name and the same bytes, so the device needs no migration. A test pins that today's text loads the same values and re-saves byte-identically.
- It is not folded into `Prefs`. `prefs_set` replaces the whole object from the frontend's initial-load snapshot, so a recording-input group there would be overwritten with stale values, and `prefs_set` and `arm` would race on one file.
- A corrupt `prefs.json` or `recent.json` gets a stderr warning under the one bad-file policy, and the file stays in place. Users don't see stderr.

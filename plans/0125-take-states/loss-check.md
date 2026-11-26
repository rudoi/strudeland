# 0125 loss check

The three stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| Transcript of every take flow, including the five new branch flows: events, results, files and key sidecar fields, cross-OS normalised, with frozen expected and frozen normaliser regions; draft PR with green Linux/Windows/macOS CI | 001 |
| `TakeState` enum, `leave`/`leave_with` (replaces the four `drop_ended` calls), `saved_channel`, explicit `pending_loss_stem`, all in place in `recording.rs` | 002 |
| `strudeland-record::takes`: `Takes` over ports, `TakeError` (manual `PartialEq`, `From<RecordError>`), `TakeEvent`, `TakesConfig.app_version`, prefs, `repair_on_open`, shared fakes in `testing` | 003 |
| `recording.rs` keeps the wire types, `From` impls (incl. `LevelEvent`), `SystemPlatform`/permission, `AppSink`, commands; `command_error.rs` `for_memo_error` | 003 |
| 49 lifecycle tests moved as named tests, 4 from 002, `take_error_texts`; the app keeps 16 (macOS) / 15 | 002-003 |
| AGENTS.md voice-memo bullet, record README takes section | 003 |
| No visible change: events, results, errors, sidecar incl. `app.version`, files, threading | every stage (frozen transcript, CI on three OSes) |

Kept quirks filed as #155 for the user.

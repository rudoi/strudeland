# 0128 loss check

The three stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| `AppData` in strudeland-project: `locate()`/`at()`, the one document machinery and bad-file policy; Prefs and Recents on it (same files, same bytes); `InstrumentLibrary::open_in`; `ProjectError::NoDataDir`; Tauri resolves once (`AppDataState`) | 001 |
| `RecordingInput` document (same `recording.json` name and bytes); `takes::prefs` keeps only the selection logic; Takes takes `&AppData`; `recording.rs` drops its path and filename | 002 |
| `Prefs.limiter`; the TS store's `setLimiter`/`edited`; StatusBar and app.ts wiring; `migrate.ts` (key removed only after a landed save); `limiter-pref.ts` deleted | 003 |
| Byte tests first for every document; machinery tests; the migration table; the frozen recording transcript unchanged | 001-003 |
| AGENTS.md lines each stage makes stale | 001-003 |
| No lost value, no byte change, no IPC binding change; only the user-approved limiter changes, the stderr warnings, the no-HOME text and the accepted array-form risk | every stage |

# 0126 loss check

The two stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| `tempoAtBar` in `structure.ts`; `Runtime.tempoNow()` with today's formula and no side effect; the Toolbar reads it; `state()` builds its tempo from the position it already read (one clock read) | 001 |
| `stubRuntime`, `editor-hooks.test.ts` and `setup.test.ts` gain `tempoNow`; the new runtime test pins today's values | 001 |
| `live-clock.ts`: `ClockPort`, `ClockSnapshot`, `edit` in today's order, `setLayout`, `reset`, `toCycle`/`toClock`, `audibleReference`, `cpsNow`, snapshot/restore; `editPattern`, `start` and `performSwap` use it | 002 |
| `tests/live-clock.test.ts` over a fake port; `:556` deleted, `:590` shrunk to the `getCps` wiring and one edit, `:540` kept | 002 |
| `host:build`, `host:parity` and `cargo test -p strudeland-host` per stage; no `PLANNER_FILES` edit | 001-002 |
| AGENTS.md layout line, GLOSSARY "Shown tempo" | 001-002 |
| No visible change except the accepted one-frame Toolbar resampling | both |

Follow-ups filed: #156 (audible tempo), #157 (engine router), #158 (playhead jump).

# 0120 loss check

The four stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| `codes::ALL`, the event-limit constants, `decode_result`, the methods.rs module doc | 001 (`ALL` becomes the `codes!` macro in 002) |
| Wire examples under `tests/wire/`, `tests/wire.rs`, the `wire-fixtures` feature, pruning the methods.rs round trips | 001 |
| TS readers: method names, error codes, stats key sets, the handler's accepted, invalid and shape checks; regex scrapes deleted | 002 |
| CLI typed `call`/`report`/`eval_like` (16 call sites); clap range from `EVENTS_LIMIT_MAX`; fixture-fed run.rs, e2e and MCP tests; help and SKILL.md limit test | 003 |
| control.rs screenshot and project.open tests read the examples; AGENTS.md rule and layout; GLOSSARY "Wire example" | 004 |
| Zero wire change: texts, codes, CLI output and SKILL.md are all unchanged | every stage; checked in testing and review |

Rejected, and recorded in the proposal: Rust pre-deserializing params (a wire change, see reverify.md), replacing `output.rs base()`, generating the TS limits, renaming the markers.

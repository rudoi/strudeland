# 0122 loss check

The two stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| Equality guard `eq.sh`: dump, signatures and renders for 226 jobs, baseline at 385724a | 001 |
| Split `controls.rs` into `capability`, `params` and `coerce`; `bus_controls` split; imports moved (no facade) | 001 |
| Doc paths that stop existing (AGENTS.md:212-213, README, controls.md:41, js.rs, bin.rs comment) | 001 |
| `SourceKind::NAMES` with `of` as a lookup; `allowed_sounds()` with the `SILENCE` guard; one `DEFAULT_SOUND` | 002 |
| `modulate::modulatable_targets()`; `MODULATOR_TARGETS` deleted; the `TODAY` pin | 002 |
| Tests: two-way chain-key read (replaces the length pin), every allowed sound renders, every `TARGETS` row derived or always skipped/thrown, silence-array pin, names match the builders | 002 |
| Docs on what is derived (README, controls.md:42) | 002 |
| No change to accepts/refuses, signatures, dump, texts, `never_panics` order or sound | both (eq.sh `equal`) |

Follow-up filed: #151 (the stale `schedule.ts` comment, to fold into the next planner edit).

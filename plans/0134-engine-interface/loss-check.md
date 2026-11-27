# 0134 loss check

The one stage covers the scoped request and reverify's Decisions. The proposal's Out list
(signatures JSON shape, the render crate's refusal text, the live warning, the pinned quirks,
refs and bundles) stays out by design.

| Plan item | Source | Stage |
|---|---|---|
| `capability` and `orbit` `pub(crate)` | request, Decision 5 | 001 step 2 |
| Root re-export of `UnsupportedSignature`, `unsupported_signatures`, `CHAIN_KEYS`; every outside `use` updated (engine bin, `fixtures.rs`, `micro.rs`, `bin.rs`, `never_panics.rs`, render `plan.rs`, `decay_bound.rs`) | request, reverify §5 | 001 steps 2, 5, 6, 7 |
| `signature_of` behind `#[cfg(test)]` | request, Decision 5 | 001 step 3 |
| Doc links to now-private items made plain (`live.rs:2` `Engine`, `capability.rs:452` `CHAIN_UNSUPPORTED_KEYS`) | rustdoc on the narrowed surface | 001 step 3 |
| `Engine` and `quantum_end_time` off the root (decided: removed; no outside user, no doc names them as API) | request ("decide") | 001 step 2 |
| The 32 test-only items (`signature_of` among them) and the 3 unused items the visibility change exposes (new finding, probe-measured) | proposal Background | 001 steps 2, 3 |
| `tests/orbit_pairs.rs` into `orbit.rs` unit tests with `include_str!` on the shared table | request, Decision 5 | 001 step 4 |
| `Rendered.unsupported` in place of formatted unsupported errors | request, Decision 5 | 001 step 5 |
| The binary's summary entries, exit 1 and `ErrorKind::Unsupported` built from the field, byte-identical (cap included) | request, Decision 1 | 001 step 5 |
| `render_one`'s unreachable text formatted in the render crate from the signature, byte-identical (cap included) | request, Decision 2 | 001 step 7 |
| `Rendered` literals (render `render.rs`, `pipeline_audio.rs`) and the tests that read it (engine `render.rs`, `fixtures.rs`, `never_panics.rs`, `render_one`'s test) | request, reverify §4 | 001 steps 5, 6, 7 |
| Clean-render checks keep their strength (`fixtures.rs`, `never_panics.rs` `exercise`, `stems.rs`, `capability.rs` `sound_render`, `joiner_identity.rs`) | proposal Risks | 001 steps 6, 7 |
| Signatures JSON, CLI/export refusal texts, live warning payload untouched | request, Decisions 1 to 4 | 000 Rules (frozen files), gates 6, 8, 9 |
| Baseline before any edit of the engine binary's `signatures` and `render` outputs and WAV bytes over the fixture jobs and hand-made unsupported jobs; byte-identical after | request | 001 step 1, gate 6 |
| Pub-surface listing before and after with only the planned changes | request | 001 step 1, gate 7 |
| Literal counts of the `s=… keys=…` and refusal format strings | request | 001 step 1, gate 8 |
| Gates: fmt, clippy (and the Windows target for the engine), `cargo test --workspace`, `bun test` | request | 000 gate 1 to 5 |
| AGENTS.md lines checked: no change needed (the bullet describes the modules, does not name `orbit_pairs.rs`); engine README sentence updated | request | proposal Scope, 001 step 8 |

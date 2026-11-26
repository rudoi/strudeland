# 0121 loss check

The four stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| Absolute bit guard: baseline release binary and digest crate, each built before any engine edit; `ab.sh`; durable copy | 001 |
| `live_equals_offline` gains the local-reverb jobs | 001 |
| Core lifecycle: `stop_voices`, `set_seed`, `create_orbit`, `frame`, `mix`, `bus_graph`, `drain_errors`; `live.rs` moved onto them; live tests stop reading `core.*` | 002 |
| One unsettled-voice flag; the `IrSettler` seam with its sync and worker adapters; `land_ir`/`cache_ir`; the dead helpers deleted | 003 |
| `render_core.rs` with private fields; `Slot` accessors; `push_error`/`finish`; tests moved; README and AGENTS.md | 004 |
| Zero bit change and no live-timing change | every stage (A/B guard, zero diffs) |

Rejected, and recorded in the proposal: merging the convolver builds, a watermark index, changing settle timing, deleting `LiveEngine::stop`, leaving Core in `render.rs`, and a generic settler.

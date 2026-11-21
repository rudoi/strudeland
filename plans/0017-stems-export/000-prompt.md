# Task 17 execution

Branch `llm/andrew/strudeland/0017-stems-export`. Use the existing worktree and configured Cargo target. The [proposal](proposal.md), [engine](engine.md), [pipeline](pipeline.md), and [interface](interfaces.md) contracts bind implementation. User authorized automated advancement through PR staging and a stop at `review_gate`.

| Stage | Work | Validation | Commit |
| --- | --- | --- | --- |
| Engine | Add opt-in catalog and single-graph capture | Engine reference identity, reconstruction and grouping tests | `feat(engine): capture sample-aligned stems` |
| Pipeline | Join pairs and publish scoped WAV sets after checked planning | Float WAV integration, buffer limits and destination tests | `feat(render): export aligned WAV stem sets` |
| Interfaces | Wire CLI and desktop via the common pipeline and update contributor and user docs | CLI and Tauri tests, frontend checks and desktop smoke | `feat(export): expose stems in the CLI and desktop` |

Engine, pipeline and interface editing may overlap through the declared Rust entry points. Run dependent checks after integration and coordinate one Cargo command at a time. Complete workspace and frontend verification, then simplify and review the final branch diff before publishing. Retain this file and the design records. No temporary tracked execution stage files are planned.

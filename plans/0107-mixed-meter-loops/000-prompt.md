# Implementation and validation order

Branch `llm/andrew/strudeland/0107-mixed-meter-loops`.

Read the [proposal](proposal.md), [arrangement](arrangement.md), [live](live.md), [render](render.md), and [coverage review](loss-check.md). Advance automatically through creation of the reviewable PR. Stop at the review gate for the owner's approval.

1. [Structure and clock](001-arrangement.md)
2. [Live playback and inspection](002-live.md)
3. [Render and export](003-render.md)
4. [Integration and validation](004-validate.md)

Once the contracts are fixed, live and render adapters may be implemented in parallel to the clock with disjoint source files. Coordinate Superdough and Rust phase parameters before running the integrated checks. Keep one Cargo command at a time. Follow the repository's generated artifact commands. The four numbered execution files are temporary. Retain this entry point, proposal, subsystem contracts, implementation log, and loss report.

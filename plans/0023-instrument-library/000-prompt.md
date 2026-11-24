# Implement the instrument library

Work on `llm/andrew/strudeland/0023-instrument-library` for Keitora task #23. Read the
[proposal](proposal.md), [storage contract](storage.md), [evaluation contract](evaluation.md)
and [loss check](loss-check.md).

1. [Store and validate definitions](001-storage.md).
2. [Compile and bind definitions during evaluation](002-evaluation.md).
3. [Wire desktop and offline entry points](003-entry-points.md).

Storage and the evaluation adapter can proceed together once their shared schema and control
table location are fixed. Desktop session/watcher and Rust export wiring have separate file
ownership, except the export caller constructed by the desktop. Integrate and regenerate
the host bundle after its TypeScript inputs are settled. Run one Cargo command at a time.

Run the checks from AGENTS.md, a browser/headless parity check for pinned voices, and a
desktop smoke test for this runtime change. The user requested auto-advance until the human
review gate. The numbered implementation files are temporary. Retain this entry point,
the proposal, contracts, loss check and implementation log at finalization.

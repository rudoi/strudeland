# Instrument browser execution

Work on `llm/andrew/strudeland/0024-instrument-browser`. Follow the [proposal](proposal.md),
[storage and preview](storage-and-preview.md), [browser and editor](browser-and-editor.md),
and [automation](automation.md). Keep these design records and the [comparison](loss-check.md).

| Stage | Work and dependencies | Validation |
|---|---|---|
| 1 | [Store and native preview](001-storage.md) | Real preset and kit audio, scoped storage and stale version tests |
| 2 | [Editor and RPC](002-editor.md) | Parser ranges and session edits, protocol parity |
| 3 | [Browser](003-browser.md) uses stages 1 and 2 | Fake-audio lifecycle, desktop capture |
| 4 | [CLI and MCP](004-automation.md) use stages 1 and 2 | CLI behavior and subprocess discovery and calls |

The stages can implement disjoint files in parallel against the defined interfaces.
Run one Cargo command at a time. Run the full canonical frontend and Rust validation
and the desktop and MCP smoke checks when integrated. Stop at the PR review gate.
Remove only the four numbered execution files during finalization.

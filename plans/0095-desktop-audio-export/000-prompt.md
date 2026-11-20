# Implement desktop audio export

Branch `llm/andrew/strudeland/0095-desktop-audio-export`

Follow the [proposal](proposal.md), [interface](contract.md), and [requirements check](loss-check.md).
Implement the native and frontend sides in parallel once the interface is settled. Run one Cargo
command at a time. The parent integrates the work and records evidence in `implementation.md`.

1. [Native export](010-native.md) covers destination planning, writers, Tauri, and Rust tests.
2. [Export dialog](020-dialog.md) covers the native File entry, captured source, dialog, and Bun tests.
   The native owner edits Rust menu dispatch and the dialog owner edits its frontend consumer.
3. [Verification](030-verification.md) covers the concurrent work together and checks the signed
   desktop UI before publication.

Keep this entry, the proposal, interface, requirements check, and implementation record. Remove
only the numbered execution stages 010 through 030 at finalization. Auto-advance stops at
`review_gate` for human review of the PR.

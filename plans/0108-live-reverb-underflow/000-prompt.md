# Live reverb underflow workflow

Branch `llm/andrew/strudeland/0108-live-reverb-underflow`.

Follow the [proposal](proposal.md) and [plan verification](loss-check.md). Keep source material,
raw traces, and their identifying metadata outside tracked files.

1. Establish the live baseline and attribute costs.
2. Implement and test the measured fixes. Verify real-time and numerical behavior against the
   baseline.
3. Run workspace checks and repeat the live comparison. Review, document results, and publish
   for human review. Finalization and merge require separate approval.

Keep this entry point, the proposal, the loss check, and the evidence summary. Remove the ordered
execution files during finalization.

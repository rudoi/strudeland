# Task 103 execution plan

Branch `llm/andrew/strudeland/0103-format-on-save`. The
[proposal](proposal.md), [contract](design.md), and [loss check](loss-check.md) govern this
work.

1. Implement and verify the [formatter and preference components](010-components.md).
2. Wire and verify the [session and app integration](020-integration.md).
3. Run the canonical gates and the desktop smoke test, run the pre-push quality and docs
   checks, and publish the PR for review.

The formatter and preference components own disjoint files. The parent integrates them
into the session and app after agreeing the formatter and preference APIs. The parent
runs Cargo validation in the shared configured target. Stop at `review_gate` for the user
to review. Retain this entry point, the proposal, contract, and loss check. Remove
the numbered execution instructions and implementation log at finalization.

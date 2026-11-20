# Integrated checks and desktop smoke

## Goal and prerequisites

Verify the integrated task branch and the actual desktop path before the review gate.

## Implementation

Reconcile with fetched main and run canonical Rust and frontend checks. Build and launch the
hardened macOS app and use File → Export Audio on synthetic songs. Use the CLI screenshot command
to capture the dialog and output summary and inspect the captures. Check WAV frame counts and the
repeat's `smpl` loop markers. For one intro bar and one loop bar at 120 BPM in 4/4, expect
176,400 and 88,200 frames respectively. For the intro-less case, expect two 88,200-frame files.
Check the open, submit and close interaction, and record the platform and actual evidence.

## Validation

Run simplify and a high-mode correctness review on the integrated diff. Resolve findings and
repeat affected checks. Update the user and maintainer documentation, then publish a PR with
the checks and desktop smoke results. Hold at the workflow review gate.

Commit message for the evidence `docs(export): record desktop export validation`.

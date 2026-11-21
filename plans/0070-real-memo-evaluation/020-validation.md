# Validation and review

## Goal

Verify note accuracy, timing and playback in local and synthetic cases.

## Prerequisites

Both implementation changes and focused tests are ready.

## Implementation

Rebase on the fetched base. Compare the local reference and unchanged commands
using both default and fitted phase. Render or query the generated notation
with Strudel. Run the full Rust workflow gates, synthetic evaluation, and release
performance budget.

## Validation

Record actual changes in matches, false attacks, and missed notes. Pass the
synthetic pins and performance budget, inspect any differences by tier, and
complete the simplify, high-thoroughness review, and documentation gates
before opening the PR.

## Paired playback check

The local reference contains 25 attacks across eight bars. Run the same memo,
sidecar, and flags on the baseline and candidate. Evaluate both emitted
mini-notation forms in the headless Strudel host at the recording tempo and
compare pitch and onset one to one within 50 ms. Count attacks before bar 1
separately, so they cannot inflate the in-range result.
The local scorer follows `eval::score::conp`'s smallest-onset-error matching and
also measures a 100 ms window. It reads the separately evaluated Strudel events
for playback and the reported note times for raw attacks.

| Settings | Matches before → after | Extra or mismatched before → after | Attacks before bar 1 |
| --- | --- | --- | --- |
| Default | 1 → 1 | 33 → 30 | 2 → 0 |
| Eighth notes | 0 → 0 | 27 → 26 | 2 → 0 |
| Eighth notes with fitted alignment | 15 → 15 | 16 → 11 | 1 → 0 |

Precision, recall, and F1 for reported raw attacks and actual playback.
Reported raw attacks exclude tied continuation records and reflect the notes
that survived quantization.

| Settings and measurement | 50 ms baseline → candidate (P / R / F1) | 100 ms baseline → candidate (P / R / F1) |
| --- | --- | --- |
| Default, reported raw | .030 / .040 / .034 → .032 / .040 / .036 | .030 / .040 / .034 → .032 / .040 / .036 |
| Default, playback | .029 / .040 / .034 → .032 / .040 / .036 | .029 / .040 / .034 → .032 / .040 / .036 |
| Eighth notes, both | .000 / .000 / .000 → .000 / .000 / .000 | .000 / .000 / .000 → .000 / .000 / .000 |
| Eighth notes with fitted alignment, reported raw | .407 / .440 / .423 → .423 / .440 / .431 | .444 / .480 / .462 → .462 / .480 / .471 |
| Eighth notes with fitted alignment, playback | .484 / .600 / .536 → .577 / .600 / .588 | .484 / .600 / .536 → .577 / .600 / .588 |

The fitted alignment was chosen with this reference, so the comparison holds
it constant on both sides. Four held notes in that run preserve their attack
and duration without a second attack at the barline. The remaining ten misses
need more evidence before changing pitch or timing rules. See #100.

## Commit message

`docs(transcribe): record accuracy and playback validation`

## Local gates

- Rust workspace tests passed with 2,130 tests and six ignored. The first run
  could not access Unix sockets or filesystem notifications inside the sandbox.
  The retry with normal user permissions passed.
- Workspace Clippy passed with warnings denied. Rust formatting passed.
- The release synthetic summary matched the published baseline on all six tiers
  across 25 cases. The median release performance was 0.164 seconds for 60
  seconds of audio, within the 1-second budget.
- The CLI built on the task branch. Both generated notation forms produced
  identical pitches, onsets and durations in the headless Strudel host.
- Independent high-thoroughness review verified hold and repeat placement.
  Fixed the obsolete CLI warning, restored the provisional designation for the
  existing voicing threshold, and recorded both requested score tolerances.

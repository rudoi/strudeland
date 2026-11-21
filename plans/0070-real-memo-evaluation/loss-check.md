# Plan reconciliation

The refined plans retain the user's local reference, paired measurements, and
synthetic protection from the initial proposal. A matched onset count and the
number of audible re-attacks are separate measurements. The per-bar report is
preserved for existing consumers.

Automatic capture or beat-phase correction was considered. The recording clock
maps consistently and the audio lacks a reliable click anchor. Onset agreement
cannot recover a whole-step shift. The existing manual setting is tested, with
the fitted alignment disclosed. Better latency calibration is tracked in #67
and click bleed handling in #72.

The first PR covers the mechanics validated by one private annotation. Additional
independent annotated takes are needed to tune pitch thresholds and substantiate
general improvement. Collection and holdout comparisons are tracked in #100.
Alternative trackers remain under #69.

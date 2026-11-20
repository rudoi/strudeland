# Proposal comparison

The four preset outputs, loop metadata, Godot offset, MP3 warning and current default render are preserved. The split output now isolates and folds the loop body to meet the repeated playback requirement. This trades a continuous DSP transition from intro to body for an intro-free repeating asset. The tagged preset preserves that DSP transition. The decay duration is automatic in this release. The CLI remains the integration surface, with the game fixture serving as an export acceptance check.

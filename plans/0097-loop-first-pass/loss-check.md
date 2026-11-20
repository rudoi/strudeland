# Contract check

- Preserve preset names, encoded loop metadata formats, decoded frame units, supported encoders, authored outro exclusion, one-shot decay and standard rendering.
- Preserve a single file for `tagged-loop` and `folded-loop` plus the established split filenames and roles for `intro-loop`.
- Expand the split opening to include the first traversal and always emit it, including for songs that start on the loop. Keep its marker absent. Its separate repeat still starts at frame zero.
- Extend the single-file layouts by one body. Move the marker to the repeat. Whole-file replay now includes the opening, so update player instructions to use the marker.
- Preserve naturally sounding intro effects through the first downbeat. Blend the final part of the opening into the periodic reference so intro-only audio stays absent from recurrence.
- Keep the original repeat calculation for the isolated loop. A later engine pass with time-varying processing may differ from that folded reference.

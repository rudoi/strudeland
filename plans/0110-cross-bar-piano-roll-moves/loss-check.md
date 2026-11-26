# Requirements cross-check

| Requirement | Disposition |
| --- | --- |
| Exact two-bar source rewrites | Kept. Reparse the combined patch and prove the exact interval, neighboring notes, source shape, and unchanged sibling bars. |
| Fit and overlap | Kept. Endpoint contact is valid, collisions go through a bounded scoped edit when available, and unsafe edits keep the source intact. |
| Duration preservation across meters | Kept for the note's whole in the written meter timeline. Existing gate and mask rules remain in effect through playback. |
| Undo and retained anchor | Kept. Test one editor history item and an explicitly selected alternate anchor across undo and redo. |
| Effects on other uses | Strengthened. Verify each written section and every affected repeat. Offer a written occurrence when the shared move lacks proof. |
| Current-page gestures | Kept from the proposal. Existing page navigation remains available. |
| Scoped event fallback | Refined to preserve exact edges and safe probe context. Generated patterns that cannot be re-queried under a proved context need a separate runtime change. |

The [contract](contract.md) carries the verified behavior and the [refinement audit](refinement-audit.md)
records the evidence behind the narrower generated fallback.

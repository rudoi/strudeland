# Requirements cross-check

Compare the original task brief, the revised [proposal](proposal.md), and the three implementation
contracts. The pre-refinement proposal is retained in Git at `fd6ee4a`. The revised proposal and
refinement evidence entered Git at `ed0a364`.

| Requirement | Disposition and implementation evidence |
| --- | --- |
| One caret within the pattern activates the roll, including while stopped | Retained. Source and editor contract test innermost pitch values, effect arguments, delimiters, comments, and shared aliases. |
| Explicit selection, definition navigation, meter-aware notes, pitch fit, panel toggle and audition | Retained. Edit transaction and navigation remain distinct. Test repeats, both Expression and In song, mixed meter and continued holds. |
| Luoyuan grid accepts precise local edits | Retained. Real consumer and a synthetic equivalent define the oracle. Verify source footprint and exact pitched events, including glide controls. |
| Every shown note accepts a drag with immediate code changes | Expanded in refinement. The direct source edit and event edit use the same gesture and transaction. Verify transformed, generated, repeated and unattributed notes, plus patterns that cannot represent the gesture in their original notation. |
| Scroll to changed code and visibly acknowledge the edited notes | Added by refinement. Center the relevant code when off-screen, keep focus available for further gestures and add independent visual feedback with reduced-motion support. |
| Unify drag and keyboard edits with stale snapshot rejection and one-step undo | Retained and expanded to the new route. A rejected or cancelled gesture does not write or pulse. |
| Separate Make editable conversion and preview | Replaced by first-drag source write-back per UX refinement. The original generating expression remains present, and the source immediately shows what changed. |
| Offer shared definition at a use | Retained as a discoverable action; revised default writes to the local use or written occurrence. This avoids applying the first drag to additional voices unexpectedly. |
| General direct cross-bar serialization | Retained as separate work in Keitora #110. Test cross-bar fallback for notes whose authored codec rejects a gesture. |
| Playback, export and headless behavior | Retained. Source-changing operations need the same semantics across runtime paths, and resolved uses should follow the arrangement’s written timing. |

Verification that depends on implementation is still pending. A generated-note identity or runtime
boundary that cannot support the user contract is a validation finding, not evidence of completion.

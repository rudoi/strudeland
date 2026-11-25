# Piano roll loss check

Compared the [approved proposal](proposal.md), refinement audit, and decisions in commits
`be1d7e8`, `e7f7b9d` and `4279ceb` to the fractured design.

| Requirement | Recorded contract |
| --- | --- |
| Selected expressions, safe source changes, immutable reuse and known-use disclosure | [Source](source.md) |
| Expression first, arranged inspection, written meter, variable pitch range and explicit draft evaluation | [Inspection](inspection.md) |
| Five monophonic operations, independent chord pitch edits and single-step undo | [Source](source.md) |
| Persistent accessible toolbar toggle and idle/playing behavior | [Dock](dock.md), [proposal](proposal.md) |
| Explicit note and keyboard audition, preference-controlled automatic previews and verified synth fallback | [Dock](dock.md) |
| Cross-bar support conditional on proof, same musical duration across meters | [Source](source.md), [inspection](inspection.md) |

The proposal retains release acceptance and risks. The inspection contract expresses the integer
window and scope of provenance claims from the audit. No approved requirement was dropped.

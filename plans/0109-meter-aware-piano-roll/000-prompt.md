# Implement the meter-aware piano roll

Branch `llm/andrew/strudeland/0109-meter-aware-piano-roll`. Follow the [proposal](proposal.md),
[source](source.md), [inspection](inspection.md) and [dock](dock.md) contracts. The
[refinement audit](refinement-audit.md) records proven assumptions and prototype gates.

| Stage | Goal and prerequisites | Implementation and validation | Commit |
| --- | --- | --- | --- |
| 1. Source and geometry | Establish shared types. | Implement lexical selection, authored projection, exact edit planning and meter-aware grid. Test source operations, mapped fractions and shared targets. | `feat(editor): resolve and edit piano roll phrases` |
| 2. Host inspection | Use the shared observation types. Runs beside source work. | Add the bounded inspection operation, project pins and output verification. Exercise capture and timeout recovery. Regenerate and verify the committed bundle. | `feat(host): inspect selected Strudel patterns` |
| 3. App integration | Use both stage contracts. The grid surface can be built beside stages 1 and 2. | Connect editor/session, persisted disclosure, views, source navigation, pending work and audition. Exercise gesture isolation and keyboard paths. | `feat(editor): add the meter-aware piano roll dock` |
| 4. Testing and quality | Require all stages and relevant gates. | Rebase, run canonical frontend and Rust checks, verify the host bundle, smoke-test the app, run Simplify and high code review, and update docs. | Focused fixes and `docs(editor): document piano roll behavior` |

Cross-bar dragging remains conditional on proving the general serializer. The verified direct
editing path, isolated broad inspection, contextual view, and audition remain release
requirements. Record any gate failure and validation outcome. Retain this entry point, the
design documents, loss check and refinement audit for review and finalization.

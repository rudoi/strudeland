# Proposal coverage

[Proposal](proposal.md)

| Proposal commitment | Implementation contract and evidence |
| --- | --- |
| Multiple named sections can form one loop | [Arrangement](arrangement.md) resolves inclusive section endpoints. Exercise authored playback, timeline audition, release from every member, default clip counts and full-range export. |
| One named section can contain multiple meters | [Arrangement](arrangement.md) validates a song-wide bar map. A synthetic F/F/E fixture uses the same 35-quarter span both as one section and as a range. |
| Logical patterns and counting stay in bars | [Live](live.md) keeps logical query coordinates and maps only at playback. Verify held notes, CPS-sensitive operators, visual playhead, seeds and inspected beat positions. |
| Live and export end on the intended downbeat | [Render](render.md) uses one timing map and exact musical frame counts. Verify browser and host agreement, joins, stems, and decoded markers. |
| Existing songs, job data and audio packaging stay compatible | [Arrangement](arrangement.md) normalizes legacy markers, and [Render](render.md) carries optional timing fields. Existing jobs, single-meter fixtures and clean-first-pass exports serve as regression coverage. |
| Fixed-grid recording stays accurate | [Live](live.md) specifies an explicit guard for backed takes and fixed-grid memo alignment when the song uses mixed meter. |

Refinement clarified stable phase for fixed-Hz and cycle-synced effects, which requires a focused Superdough patch. Tests use a synthetic arrangement because the source track's precise meter map was not supplied. The proposal's user-visible requirements remain in scope.

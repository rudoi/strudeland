# 0132 loss check

The two stages cover the proposal in full. Nothing is dropped. The out-of-scope items are the
proposal's "Out" list, and the frozen-file and literal scripts check that they stay untouched.

| Proposal item | Stage |
|---|---|
| The baseline before any source edit: the synthetic memo grid and its requests, the base binary's outputs, `check-base.py` against the predictor, the eval table and the per-case eval dump | 001 (steps 1-4) |
| Red runs of the two regression tests on the base code (CLI 48 kHz take through `run_transcribe`; lib frame-rounded cut, seconds run first) | 001 (step 5) |
| `Options.bars: Option<(u32, u32)>` replaces `last_bar`; `span_start_seconds` and `memo_seconds` unchanged | 001 (step 7) |
| The library-only `bars` validation (`from == 0` or `from > to` is `BadOption`) and `bad_bars_requests_are_bad_options` | 001 (steps 7, 11) |
| `notation::window` (`WindowStart`, `Window`): exactly A..B for `bars`, today's rule otherwise; `window_rules` and `span_bar_window_is_todays_rule` | 001 (steps 8, 11) |
| `lib.rs` picks the window from `opts.bars`; the `MAX_NOTATION_BARS` text kept | 001 (step 9) |
| The three `last_bar` settings of `a_requested_last_bar_keeps_trailing_rest_bars` move to `bars`; every other test unchanged, `a_span_later…` on the seconds rule | 001 (step 10) |
| `--bars 1-B` over a pickup starts at 1 and keeps the bar-0 notes (`a_bars_request_from_bar_1_leaves_the_pickup_in_the_notes`) | 001 (step 11) |
| The CLI sets `bars: args.bars`; `resolve_span` and the order of work unchanged | 001 (step 12) |
| The `BARLINE_SLACK_SECONDS` doc, the README notation row, the AGENTS.md transcription bullet | 001 (steps 7, 13) |
| The grid after the fix, judged by `classify.py`; eval table and dump byte-identical | 001 (validation) |
| `suggest::suggest` returning `Suggestions` (`Debug, Clone, PartialEq`); its helpers private; `suggest_is_its_parts` | 002 (steps 1-2) |
| The crate doc and README intro stop saying "never paths"; "audio I/O" becomes "audio devices" in the README and `AGENTS.md:333` | 002 (steps 2-4) |
| Every grid output and the eval dump byte-identical to 001's | 002 (validation) |
| Every text in "Texts that must not change" (literal counts), the frozen files, `SKILL.md` unchanged | 001-002 (gate steps 10-11) |

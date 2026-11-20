# Requirements check

| Task requirement | Coverage |
| --- | --- |
| Export through the shared Rust pipeline | The native desktop commands call the same engine as the CLI. |
| Explain all five modes | The mode table defines the range and plain-language description for each choice. |
| Choose clip, section, or length | Standard and one-shot expose each range. The three loop modes select the marked section. |
| Format, destination, filenames | The form offers all four formats and a directory picker. The same target resolver supplies the preview and writer. |
| Progress, warnings, errors | The channel reports progress. Per-file results carry loop offsets and errors can report partial publication. |
| Loop marking and MP3 padding | The form explains the metadata requirement and warns about MP3 before rendering a loop. |
| Verify WAV intro and loop through the app | The implementation plan requires native UI smoke evidence and decoded output checks. |

The split and folded layouts use the merged first-pass renderer. The split preset always emits
an opening and a repeat. Folded loop prepends its clean first pass. The proposal records these
two changes from the original brief and keeps mode and filename compatibility with the CLI.

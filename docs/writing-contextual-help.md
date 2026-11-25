# Writing contextual help

Use this guide when adding or editing text that appears on hover or keyboard focus in Strudeland.

1. **Verify what the control does.** Read its handler, state and existing label. Identify what changes, which part of the song or project it affects, and when the change takes effect. Check whether the action changes the view, edits source, starts playback or writes a file. Use the same words as the surrounding UI when they describe the same thing.
2. **Write the useful difference.** Start with the outcome or the reason the action is unavailable. Add units, timing or scope when they affect the user's decision. Use one sentence for most hints and a second for a separate consequence. Write for both pointer and keyboard users. Keep full sentences in sentence case.
3. **Read the name and hint together.** The hint is also an accessible description. Read the control's name followed by the hint aloud. Cut repeated labels, praise, promises about ease, throat-clearing and generic feature descriptions. Every remaining phrase should describe a real outcome, prerequisite or next step.
4. **Check states and fit.** Focus the control, exercise its available states and compare the displayed hint with the result. Read the longest state at the minimum window size and at increased text size. Shorten wording before reducing type size. Keep essential warnings and remedies available in the relevant view when a user leaves the control.

| Control | Example | What to verify |
| --- | --- | --- |
| Edit shared definition | Show the original pattern. Edits can affect every use of it. | Opening changes the piano-roll view. Editing afterward can affect other uses. |
| Back to selection | Return to the pattern or occurrence you were viewing. | Returns the roll to its prior view and preserves completed edits. |
| Quarter-note BPM | Set the tempo in quarter notes per minute. Applies on the next evaluation. | Unit and effective time match the timeline. |
| Audition notes, off | Preview pitches you type while playback is stopped. | Starting a preview does not start song playback. |
| Audition notes, on | Turn off previews for pitches you type. | The setting changes typed-note previews. |
| Previous bars, unavailable | You’re already viewing the first bars of this pattern. | This reason belongs to the first-page boundary. |
| Clear clip | Clear the clip warning on the master meter. | The action clears the indicator. |

For toggles, use one description when the accessible state and label already explain the direction. Write different hints when enabling and disabling have distinct consequences worth mentioning. For unavailable actions, give the actual blocker. Add a next step only if there is one the user can take. Preserve the operative distinction when you shorten a hint. “Adjust playback settings” could describe dozens of controls and tells the reader nothing about quarter-note BPM, when it applies, or what turning a limiter off changes.

## References

- Apple’s guidance on action-oriented wording, settings, and recoverable errors. [Human Interface Guidelines on writing](https://developer.apple.com/design/human-interface-guidelines/writing)
- An editing pass for trimming filler and repetition and choosing consistent terms. [Apple WWDC25 UX writing session](https://developer.apple.com/videos/play/wwdc2025/404/)
- Guidance on leading with the useful information and reading copy aloud. [Microsoft Writing Style Guide](https://learn.microsoft.com/en-us/style-guide/top-10-tips-style-voice)

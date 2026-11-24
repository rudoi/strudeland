# Instrument library loss check

The refined contracts retain the global editable library, sound-chain presets, multi-voice
kits, parameterized factories, Strudel registration, portable project copies, and explicit
pin updates from task #23 and the initial proposal. Playback and offline export share one
definition format. Missing legacy directories carry an empty snapshot.

The refinement separates the fixed prebake operation from project-specific lexical factories.
This keeps project selection out of shared Pattern methods while retaining
`lead({ cutoff })` in song code. Explicit updates additionally require the current pin
fingerprint. Watching the project copies lets CLI changes take effect without app-side pin
controls. The browser and MCP surface remain #24.

The first-release definition supports ordered built-in controls and scalar parameters.
Author-supplied scripts, project-specific Pattern methods, and separate FX stage syntax
were considered during refinement. Those did not form part of the original committed
feature contract. No required scope was lost.

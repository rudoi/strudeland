# CLI and desktop contract

The CLI exposes `--stems orbit|voice|instrument`. In this mode `--out` supplies a new directory. Compatible explicit settings are WAV, 32f, and loudness off. Reject incompatible encoding, mastering, and preset settings. Report published file paths and the pre-master reference in the result.

The desktop dialog exposes Stems with an orbit, voice, or instrument selector and previews the new set directory. Preserve standard range selection and active-buffer export. The current export session and quit guard cover the full write. Show published files when the export completes or when a later write fails. See [proposal](proposal.md) for scope.

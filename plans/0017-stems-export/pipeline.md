# Pipeline and destination contract

See the [proposal](proposal.md) for formats and set semantics. Use ordinary checked host jobs, a shared catalog, bounded sample buffers, and identical join coordinates across output pairs. Publish `mix.wav`, consecutive numbered stems, and a JSON manifest last. Positional filenames keep arbitrary labels out of paths. Default to a scoped directory in the project's renders folder. For explicit directories, require the parent to exist and reserve the set directory exclusively. Emit a written-file event after each successful publication and retain partial files if a later write fails.

Verify using actual float WAVs, multiple chunk and worker counts, memory and output refusal, existing directories, and partial publication.

# 001: implement

## Goal

Make every change in the proposal's Scope, Technical design and Tests sections in one commit:

- remove the two plan-request fields from Rust and TS;
- cut Rust `validate()` down to the instrument snapshot and `timeoutSec`;
- delete `MAX_LOOPS` from `strudeland-host`;
- report a bad `--loops` value as a bad request in TS;
- map bad-request plan errors to exit 2 in the two dev binaries;
- update every plan-request literal;
- update the docs.

Then regenerate: `bun run host:build` and `bun run engine:refs --write` (needs Chromium). Commit
the regenerated files in the same commit.

## Validation

Log each command's output and exit code once:

- `bun test`;
- `bun run check`;
- `bun run lint`;
- `cargo test -p strudeland-host`;
- `cargo test -p strudeland-render`;
- `cargo clippy --workspace --all-targets -- -D warnings`.

Check that the refs diff is provenance only. Testing then runs `host:parity`, `--batch` and
`--mixed-meter`, plus the full workspace tests.

## Commit

`refactor(render): drop the vestigial plan request fields and validate lengths once`

# Contributing to Strudeland

Follow the [source preview setup](README.md#apple-silicon-source-preview) to build and run the app
on Apple Silicon, or the [Windows and Ubuntu setup](README.md#windows-and-ubuntu-desktop-builds).
The [repository guide](AGENTS.md) describes the code layout, test commands and
project conventions.

## Report a bug

Search [GitHub Issues](https://github.com/rudoi/strudeland/issues) before opening a new report.
Include the source revision, OS and version, expected behavior, what actually happened, and steps
to reproduce it. For audio bugs, include the playback engine or export format if relevant.

Use a small song or project you can share publicly. Synthetic examples make good fixtures.
Before attaching logs, screenshots, or projects, review them for personal paths, recordings,
metadata and material you lack permission to redistribute. Send suspected vulnerabilities
through [private security reporting](SECURITY.md).

## Propose a change

Open an issue first for a larger feature or design change so maintainers can discuss its scope.
Fork the repository, branch from `main`, and open a pull request against `main`. Describe the
change, link the relevant issue, and list the checks you ran. Include tests for new behavior and
keep the diff focused. Review the [license](LICENSE) and
[third-party notices](THIRD-PARTY-NOTICES.md) before contributing external material.

## Verify your changes

Run the following checks from the repository root to match
[macOS CI](.github/workflows/macos.yml) and [Windows and Ubuntu CI](.github/workflows/native.yml).

```sh
bun install --frozen-lockfile
bun run build
bun test
bun run check
bun run lint
```

Use the preview Rust toolchain for the workspace checks.

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

For desktop behavior, use `bun run tauri dev` and try the affected workflow in the app.
The [repository guide](AGENTS.md) lists additional render, parity and performance checks by area.

For changes involving native APIs, report the target desktop, package, webview version and the
runtime path you exercised. For audio timing, also report the device, negotiated sample rate and
loopback measurements. Hosted fake-device and build checks cover reproducible regressions. The
desktop acceptance pass exercises installed packages with real playback and capture.

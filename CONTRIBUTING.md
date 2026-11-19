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

Run the Chromium-based host and render checks on demand. Use `bun run host:parity` and
`bun run host:parity --batch` after installing Chromium with `bunx playwright install chromium`.
Run the timing-sensitive live benchmark on a quiet machine with
`cargo test --locked -p strudeland-live --test dropout dropout_short`. Hosted PR checks skip this
wall-clock benchmark.

Run browser checks on macOS or Ubuntu while Bun's Windows Chromium launch issue is open.
See the [upstream Windows issue](https://github.com/oven-sh/bun/issues/27977).

## Installers

The **Installers** workflow builds an Apple Silicon DMG and a Windows x64 MSI.
Run it manually from `main` or on a same-repository PR that changes the installer workflow or
staging script. The Actions downloads include a SHA-256 checksum, the exact source revision,
and third-party notices. The test DMG uses an ad-hoc signature and the test MSI is unsigned.

To build locally, run `bun install --frozen-lockfile` first. On macOS, run
`bun run tauri build --ci --bundles dmg --config '{"bundle":{"macOS":{"signingIdentity":"-"}}}' -- --locked`.
On Windows, run `bun run tauri build --ci --no-sign --bundles msi -- --locked`. Tauri writes
installers to `target/release/bundle`.

For a Developer ID-signed and notarized DMG, configure the GitHub `release` environment to
allow deployments from `main`. Add `MACOS_CERTIFICATE_P12` (base64 of a Developer ID Application
identity exported as a `.p12`), `MACOS_CERTIFICATE_PASSWORD`, `APPLE_API_KEY_P8`, `APPLE_API_KEY`,
and `APPLE_API_ISSUER` as environment secrets. Set `MACOS_SIGNING_SHA1` as an environment
variable to the full fingerprint reported by `security find-identity -v -p codesigning`.
Run the workflow from `main` with **Sign and notarize the macOS DMG** enabled. The job verifies
both the disk image and the app inside it before uploading the result.

# Strudeland

A desktop app for writing and performing music with [Strudel](https://strudel.cc). Live-code
patterns, arrange them into sections, and keep your songs in a regular folder.

![Strudeland playing a synth arrangement, with the code editor, section timeline, and live output meter visible](docs/strudeland.png)

- Edit patterns with live note highlighting, an oscilloscope, and optional Vim keybindings.
- Arrange songs with named sections, loops, and per-song tempo and meter.
- Keep songs, reusable instruments, voice memos, and renders together in a project.
- Export synth arrangements to WAV, MP3, Ogg Vorbis, or Opus.

Get the Apple Silicon DMG or Windows x64 MSI from [GitHub Releases](https://github.com/rudoi/strudeland/releases)
when available. Open the DMG and drag Strudeland to Applications, or run the MSI on Windows.
The release includes SHA-256 checksums, third-party notices, and a link to the exact source.

## Apple Silicon source preview

You’ll need Git, the Xcode Command Line Tools (`xcode-select --install`), and CMake.
CI uses Bun 1.3.11 and Rust 1.94.1 with `clippy` and `rustfmt`.

```sh
git clone https://github.com/rudoi/strudeland.git
cd strudeland
bun install --frozen-lockfile
bun run tauri dev
```

Choose **new project**, select an empty folder, and click **create**. Click **play** to hear
the starter song. Use Command on macOS and Control on Windows and Linux. With that modifier,
S saves, Enter saves and evaluates, O opens a project, and N adds a song.

## Windows and Ubuntu desktop builds

The initial Windows target is Windows 11 25H2 x64 with WebView2 Evergreen and a WASAPI audio
device. The initial Linux target is Ubuntu Desktop 24.04 LTS amd64 on GNOME Wayland with
WebKitGTK 4.1 and the desktop's default ALSA-compatible audio route. Use local NTFS or ext4
project folders. Native desktop acceptance for these targets is tracked in the platform PR.

To build from source, use Git, Bun 1.3.11, Rust 1.94.1 with Clippy and rustfmt, and CMake.
On Windows, install the Visual Studio C++ build tools with a Windows SDK, use the x64 MSVC Rust
toolchain, and make sure WebView2 Evergreen is installed. See the
[Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).

On Ubuntu, install the native development packages before running Bun or Cargo.

```sh
sudo apt update
sudo apt install build-essential cmake file libasound2-dev libayatana-appindicator3-dev \
  librsvg2-dev libssl-dev libwebkit2gtk-4.1-dev libxdo-dev pkg-config
```

Use `bun install --frozen-lockfile` and `bun run tauri dev` from the repository root.
For a local package build, use `bun run tauri build --bundles msi -- --locked` on Windows
or `bun run tauri build --bundles deb -- --locked` on Ubuntu.

On Ubuntu, install a locally built Debian package with `sudo apt install ./path-to-strudeland.deb`.
To install the separate CLI from source, run `cargo install --locked --path crates/strudeland-cli`
and check `strudeland --version`. The GUI and CLI have distinct executable names.

If Windows denies microphone access, use **Settings → Privacy & security → Microphone** to
allow desktop apps and retry. On Ubuntu, select and test an input in **Settings → Sound**.
If capture timing is unavailable, the recorder can keep the audio as a gridless memo.

## Projects

Songs are plain Strudel scripts in `songs/*.js`. A song’s `meta` header sets its tempo, meter, and
sections. The editor watches the project for changes on disk.

Playing or rendering a song evaluates its JavaScript. Review songs from other people before
running them, and stop playback before applying changes you haven’t reviewed.

### Sharing voice memos

New projects ignore local recordings in Git. For an existing project, add these lines to its
root `.gitignore`.

```gitignore
/recordings/*
!/recordings/.gitkeep
```

Previously tracked files stay in Git until you remove them from the index. Earlier commits may
still contain them. Review recordings and their timing sidecars before sharing a project.

## Command line

Install the CLI to control the running app or work with song files and voice memos from a shell.

```sh
cargo install --locked --path crates/strudeland-cli
strudeland --help
strudeland render songs/main.js --format wav
```

The CLI connects to a per-user endpoint.

- macOS uses `~/Library/Application Support/com.rudoi.strudeland/ctl.sock`.
- Ubuntu uses `$XDG_RUNTIME_DIR/com.rudoi.strudeland/ctl.sock`. The runtime and socket
  parent directories must be owned by the current user with mode `0700`.
- Windows uses a local named pipe at `\\.\pipe\com.rudoi.strudeland.<user-SID>.ctl` restricted to
  the current user.

Live playback uses Strudel’s Superdough by default. Offline rendering uses the Rust engine and
supports a subset of Strudel sounds. See the [render guide](crates/strudeland-render/README.md)
for supported exports and limitations.

## Contributing

See [Contributing](CONTRIBUTING.md) for development checks and pull requests. File bugs in
[GitHub Issues](https://github.com/rudoi/strudeland/issues) and report suspected vulnerabilities
[privately](SECURITY.md).

## License

Strudeland is [AGPL-3.0-or-later](LICENSE) and includes
[Strudel](https://codeberg.org/uzu/strudel) by the TidalCycles/uzu community.
See the [third-party notices](THIRD-PARTY-NOTICES.md) for bundled code and assets.

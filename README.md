# Strudeland

A desktop app for writing and performing music with [Strudel](https://strudel.cc). Live-code
patterns, arrange them into sections, and keep your songs in a regular folder.

![Strudeland playing a synth arrangement, with the code editor, section timeline, and live output meter visible](docs/strudeland.png)

- Edit patterns with live note highlighting, an oscilloscope, and optional Vim keybindings.
- Arrange songs with named sections, loops, and per-song tempo and meter.
- Keep songs, reusable instruments, voice memos, and renders together in a project.
- Export synth arrangements to WAV, MP3, Ogg Vorbis, or Opus, or export aligned WAV stems.

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
For a local package build, use `bun run app:bundle msi` on Windows or
`bun run tauri build --bundles deb -- --locked` on Ubuntu.

On Ubuntu, install a locally built Debian package with `sudo apt install ./path-to-strudeland.deb`.
See [Command line](#command-line) for CLI installation.

If Windows denies microphone access, use **Settings → Privacy & security → Microphone** to
allow desktop apps and retry. On Ubuntu, select and test an input in **Settings → Sound**.
If capture timing is unavailable, the recorder can keep the audio as a gridless memo.

## Projects

Use **File → New Project…** to choose a folder and name a new project. Use **File → Open Project…**
or Command+O on macOS (Ctrl+O on Windows and Ubuntu) to open an existing project folder.
Both commands work while a project is already open. Strudeland prompts you about unsaved edits
before switching.

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

## Export audio

Open a song and choose **File → Export Audio…** to render the current editor buffer, including
unsaved edits. Choose WAV, MP3, Ogg Vorbis, or Opus. Check the filenames in the preview before
exporting to the project's `renders/` folder or another folder.

Standard render and one-shot can export the clip, one section, or a length in bars. One-shot
lets the notes and effects decay. Mark a section with `loop: true` to use the loop modes.
**Intro + loop** writes an opening and a repeat. Play the opening once, then repeat the second
file from its start. **Tagged loop** and **folded loop** mark a repeat in a single file. Tagged
loop includes any intro, and folded loop starts at the loop's first bar.

The result shows the Godot loop offset in seconds when the file has loop points. Use WAV for
sample-exact repeats. MP3 encoder padding can leave a gap.

Choose **Stems** to export by orbit, voice label (`.label('lead')`), or instrument sound.
The files share one start frame and duration. Import the numbered stereo WAVs at the same
position to reconstruct `mix.wav` at unity gain. `stems.json` identifies each file.
Voice and instrument exports include separate shared effects for each orbit. All files
are pre-master 32-bit float WAVs. Each export creates a set directory in the chosen folder.

On macOS, Quit during an export offers **Wait for Export** and **Cancel Quit**. If you wait, the
dialog shows the outcome and lists any files already written before you decide whether to
continue quitting. Strudeland still prompts you about unsaved songs.

## Command line

The CLI controls the running app and works with song files and voice memos from a shell.

- **macOS DMG.** Copy Strudeland to Applications, open that copy, and choose
  **strudeland → install command line tool…**. The app creates `~/.local/bin/strudeland`.
  Add `$HOME/.local/bin` to your shell's PATH if needed. An update at the same app path
  keeps the link working. To remove it or reinstall after moving the app, delete the symlink.
  The bundled CLI interprets song JavaScript, so planning complex renders can take longer.
- **Windows x64 MSI.** The installer puts `strudeland.exe` under Program Files in
  **Strudeland CLI** and adds that folder to PATH. Open a new PowerShell or Command Prompt
  after installation. Uninstalling the MSI removes the CLI and its PATH entry.
- **From source.** Run `cargo install --locked --path crates/strudeland-cli`.

```sh
strudeland --version
strudeland --help
strudeland render songs/main.js --format wav
strudeland render songs/main.js --preset intro-loop --out soundtrack.wav
strudeland render songs/main.js --stems voice --out soundtrack-stems
strudeland transcribe memo-01 --grid 8
```

Mark the loop section with `loop: true` in the song's metadata. The split preset always writes
`soundtrack-intro.wav` and `soundtrack-loop.wav`. Play the first file once to hear any intro
and the first pass through the loop. Then repeat `soundtrack-loop.wav` from frame zero.

`strudeland transcribe` proposes Strudel code from a sung or hummed voice memo.
`--grid 8` uses eighth notes. The generated pattern holds sustained notes across barlines.

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

# Strudeland

A desktop app for writing and performing music with [Strudel](https://strudel.cc). Live-code
patterns, arrange them into sections, and keep your songs in a regular folder.

![Strudeland playing a synth arrangement, with the code editor, section timeline, and live output meter visible](docs/strudeland.png)

- Edit patterns with live note highlighting, an oscilloscope, and optional Vim keybindings.
- Arrange songs with named sections, section-range loops, and meter changes.
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

Use `song(meta, parts)` for a structured arrangement. Mark one section with `loop: true`
or set `loop: { from: 'verse', through: 'chorus' }` in `meta` to loop both named sections and
everything between them. The timeline's **loop through…** command auditions a temporary range.

`meta.meter` sets the first bar's time signature. For example,
`meterChanges: [{ bar: 5, meter: '7/8' }, { bar: 6, meter: '4/4' }]` makes bar 5 a 7/8 measure,
even within a section. Bar numbers are one-based. One pattern cycle still plays one written
bar at the declared quarter-note BPM. Playback and export both follow these meter changes.
Song-backed recording and playing a memo against the song require a fixed time signature.
Mixed-meter songs use the header BPM for timing. Pattern-level `.cpm()` requires fixed meter.

Enable **Audition notes** in the status bar to hear a short preview when you finish typing
a pitch such as `g3` in a note pattern while the song is stopped. The preview uses the pattern's
fixed built-in synth, for example `note("g3").s("supersaw")`, and works on unsaved edits. The
note gets the playback outline while the preview sounds.
A brief pause or a delimiter completes the note. The checkbox starts off and remembers your
choice. Sample instruments and computed sounds are outside the preview's supported patterns.

Turn on **Format on Save** in the status bar to put chained calls on separate, indented lines
and lay out song metadata, named parts, and groups of patterns. Explicit 16- or 32-step 4/4
patterns also get visible beat spacing, with two rows of 16 steps for a 32-step bar.
Formatting keeps the pattern's timing and can be undone in one step. The setting starts off.

```js
$: s("bd ~ sd ~")
  .gain(.8)
  .room(.25)
```

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

## Instrument library

Open **Instruments** beside the song list to browse the **Global Library** or **Project
Pins**. Select an instrument to see its voices and parameter defaults. While the song
is stopped, choose **Preview** to hear a short phrase using the selected version.
If the audio engine cannot preview a sound or control, the browser shows the reason.

Choose **Pin to Project** to make a portable copy. If a newer library version is
available, the browser lets you preview both versions and choose **Update Pin**.
To apply a pin, select a complete expression such as `note("c4 e4")` in the song,
open **Instruments**, select the project pin, and choose **Apply to Selection**.
The change is one undo step. Save and evaluate the song when you are ready to use it.

Store reusable instrument definitions as JSON files in the global library. Run
`strudeland instrument library` to find its location and list its entries. For example,
save this as `lead.json` in that directory or run
`strudeland instrument create lead --file lead.json` from another directory.

```json
{
  "schema": 1,
  "parameters": { "cutoff": 1200 },
  "voices": [
    {
      "name": "lead",
      "chain": [
        { "control": "s", "value": "sawtooth" },
        { "control": "lpf", "value": { "param": "cutoff" } },
        { "control": "gain", "value": 0.25 }
      ]
    }
  ]
}
```

From your project directory, run `strudeland instrument pin lead` to copy the definition
into the project's `instruments/` directory. Keep the JSON files with your songs to share
the same definitions for playback and export. The store uses `instruments/.strudeland.lock`
for cooperating readers and writers. Newly created projects ignore it in Git. Inside a song,
apply the instrument to an existing pattern.

```js
const { lead } = instruments;
note("c4 e4 g4").apply(lead({ cutoff: 900 }))
```

You can layer multiple voices in a definition. The
[supported controls](crates/strudeland-project/instrument-controls.json) use Strudel's
usual effect order. A parameter override uses the same type as its default. Call
`lead()` to use all defaults.

When you edit a global definition, projects keep their existing pins. Run
`strudeland instrument list` for the current fingerprints, then
`strudeland instrument update lead --expect HASH` to replace the pin with the current
library version. The next editor evaluation or render picks up the updated pin.

`strudeland instrument apply lead --from N --to N --expect-file buffer.js` applies a
pin to the open editor. Use the exact buffer from `strudeland code get`, including
unsaved edits, and a zero-based UTF-16 selection with an exclusive end. Supply
`--project DIR` and `--song songs/main.js` to assert the project and song.

For an MCP client, configure the `strudeland` executable with the argument `mcp` and
stdio transport. It exposes `instrument_list`, `instrument_create`, `instrument_pin`,
`instrument_apply`, and `instrument_update`. Applying requires the desktop app with
the target song active. The [CLI skill](crates/strudeland-cli/skill/SKILL.md) documents
the arguments and results.

## Export audio

Open a song and choose **File → Export Audio…** to render the current editor buffer, including
unsaved edits. Choose WAV, MP3, Ogg Vorbis, or Opus. Check the filenames in the preview before
exporting to the project's `renders/` folder or another folder.

Standard render and one-shot can export the clip, one section, or a length in bars. One-shot
lets the notes and effects decay. The loop modes use the authored loop section or range.
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

The split preset uses the authored loop range and always writes
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

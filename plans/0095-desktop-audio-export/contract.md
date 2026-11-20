# Desktop export interface

The UI captures the active editor text, project root, song path, title, dirty flag and static
structure synchronously when the dialog opens. It keeps the source fixed through preview and
retry. The host evaluates this source for execution. The native side verifies the project root
and resolves the project-relative song path before using it to derive filenames.

The desktop-only commands use the following interface through the export API:

- `export_audio_pick_folder()` opens the native directory dialog and returns its selected
  canonical path or null. Rust retains the selected folder and only accepts it as the custom
  destination. Cancelling keeps any prior selection.

- `export_audio_preview({ request })` accepts `{ root, songPath, folder, format, preset, length }`.
  `folder` and `preset` are nullable. A null folder means the project's `renders/`. The format
  is `wav`, `mp3`, `ogg`, or `opus`. The preset mirrors the renderer's kebab-case names.
  Range values mirror `LengthRequest` as `{kind:'clip'}`, `{kind:'section',name}` or
  `{kind:'length',bars}`. Loop presets send the clip variant without an override.
- Preview returns an ordered array of `{ role, path, exists }`. Use `single`, `intro`, or
  `loop` for the role. Show the split opening first, including for a song with no intro.
- `export_audio_start({ request, source, preview, onProgress })` accepts that exact preview.
  The UI asks before passing a preview that lists existing files. The native worker checks
  the target set and collision status again before it renders. Unapproved paths are always
  published with atomic no-replace. Result files carry `{ role, path, godotLoopOffset }` with
  a nullable offset in seconds. The result is `{ files, warnings }`. A failed command returns
  `{ message, written }` with any paths already published.
- Progress payloads are `{ kind:'planned', chunks }`, `{ kind:'chunkRendered', index, of }`,
  `{ kind:'encoded', pass }`, `{ kind:'writing' }` and `{ kind:'written', path }`. The UI counts chunk messages and
  treats encoding and writing as indeterminate. It can show planning as soon as it submits.

Use a native folder picker and confirmation. A cancelled picker preserves the current folder.
Editing a selection invalidates its preview. Refresh stale or failed previews and ignore late
responses for superseded options. Keep the dialog and captured song visible during work, and
guard global and native project actions while it is open.

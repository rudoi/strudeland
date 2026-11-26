# 002: generate the IPC bindings

## Goal

`bun run ipc:build` reads `src-tauri/src` and writes the committed `src/lib/ipc/bindings.ts`,
which holds the commands and their argument keys, the events and the command error kinds. A
`bun test` regenerates the bindings in memory and fails on any difference. Nothing imports the
bindings yet (003 does), so there is no behaviour change.

## Prerequisites

- **Stage 001 is committed.** `CommandErrorKind` is the only `*Kind` enum, and no command returns
  `Result<_, String>`.
- **Read the plan's** "The generator: `scripts/ipc/build.ts`".

## Implementation

1. **`scripts/ipc/build.ts`:** a module doc saying what it reads, what it writes and that
   `tests/ipc-bindings.test.ts` checks staleness, in the style of `scripts/host/build.ts:1-8`.
   - `export const REPO_ROOT`, and `export const BINDINGS = 'src/lib/ipc/bindings.ts'`.
   - `export function parseHandler(libRs: string): { module: string; name: string; line:
     number }[]`.
   - `export function parseCommand(source: string, file: string, name: string): { args: { key:
     string; type: string; optional: boolean }[]; error: 'command' | 'export' | 'text' | null }`.
   - `export function parseEvents(source: string, file: string): { value: string; line: number
     }[]`.
   - `export function parseKinds(source: string, file: string): { name: string; variants:
     string[] }[]`.
   - `export function render(model): string`, which owns the fixed formatting.
   - `export function generate(root: string): string`, which reads the files and returns the
     text.
   - `main(argv, root)` writes `BINDINGS` and prints `wrote src/lib/ipc/bindings.ts (N commands,
     M events, K kinds)`. It exits 0, or 1 with `file:line: reason` from the thrown error, or 2
     on any argument (it takes none).
   - `if (import.meta.main) process.exit(main(process.argv.slice(2), REPO_ROOT))`.
   - Errors carry `file:line`. Use a small `IpcParseError extends Error` with `file` and
     `line`.
   - **Parsing rules:** strip only whole-line `//` comments before matching. String literals
     hold `://`, so never strip trailing comments; a `//` inside a parameter list is a refusal.
     The rest is the plan's rules, including every refusal in "fails loudly".
2. **`package.json`:** `"ipc:build": "bun scripts/ipc/build.ts"`, placed next to `host:build`.
3. **`.prettierignore`:** `src/lib/ipc/bindings.ts`, under a comment:
   `# The generated IPC bindings (bun run ipc:build)`.
4. **Run `bun run ipc:build`** and commit `src/lib/ipc/bindings.ts`. Read it, and check:
   - 46 commands in `generate_handler!` order;
   - `recording_arm: { deviceId?: string | null; channel?: number | null }`;
   - `export_audio_start` has `request`, `source`, `preview` and `onProgress`, all `unknown`;
   - `playback_start: { seed: number; channel: unknown }`;
   - `instrument_definition: { root?: string | null; id: string; fingerprint: string }`;
   - 6 events, and the 15 kinds in the Rust order.

## Tests

**`tests/ipc-bindings.test.ts`.** Use `const REGENERATE = 'run \`bun run ipc:build\`'`, as in
`tests/render-vectors.test.ts:27`.

- **`the committed bindings are current`:** `readFileSync(BINDINGS)` equals
  `generate(REPO_ROOT)`, with `REGENERATE` in the failure message.
- **`the real tree`:**
  - `Object.keys(COMMANDS)` has 46 entries and includes `song_read`, `recording_read` and
    `export_audio_start`;
  - `EVENTS` has the six values from the reverify table;
  - `COMMAND_ERROR_KINDS` starts with `projectChanged` and ends with `other`.

  This imports the committed bindings, which proves they load.
- **Parser cases** on inline Rust snippets, each a small template string:
  - the injected `State<'_, A>`, `AppHandle`, `WebviewWindow` and `Window` parameters are
    dropped, and `State<'_, A>` doesn't split the list;
  - `device_id: Option<String>` gives `deviceId?: string | null`, and `on_progress:
    Channel<X>` gives `onProgress: unknown`;
  - a command with no parameters gives `[]` and `Record<string, never>`;
  - `Result<T, CommandError>` gives `command`, `ExportError` gives `export`, `&'static str`
    gives `text`, and a non-`Result` gives `null`;
  - events are keyed by camelCased value. A `u32` const, an indented const and a
    `#[cfg(test)]`-indented const are ignored;
  - kinds are camelCased; doc comments and `#[serde(…)]` attributes on the enum are skipped.
- **Refusals,** each asserting the thrown message names the file and line:
  - a registered name with no fn;
  - an unregistered `#[tauri::command]`;
  - `#[tauri::command(rename_all = "snake_case")]`;
  - `Result<(), String>` (the message says "return CommandError");
  - `Result<(), Box<dyn Error>>`;
  - a `//` in a parameter list;
  - two consts with one event value;
  - two parameters that camelCase alike;
  - a kind variant with fields or with `#[serde(rename = "x")]`.
- **`main`:** an argument exits 2. `main([], tmpRoot)` on a copied fixture tree writes the file
  and exits 0; the same tree with a `String` command exits 1 and writes nothing. Copy only
  `src-tauri/src/*.rs` into a `mkdtempSync` dir, as `tests/render-vectors.test.ts` does.

## Validation

- **The stage gates,** logged once.
- **`bun run ipc:build` twice in a row leaves `git status` clean** after the first commit of the
  file.
- **`git diff --stat`** touches only `scripts/ipc/build.ts`, `src/lib/ipc/bindings.ts`,
  `tests/ipc-bindings.test.ts`, `package.json` and `.prettierignore`.

## Commit message

```
refactor(app): generate the Tauri IPC bindings from the Rust commands

bun run ipc:build reads src-tauri/src and writes src/lib/ipc/bindings.ts:
every registered command with its invoke argument keys and simple value
types, every emitted event name, and the command error kinds. It fails
with the file and line on anything it cannot read, including a command
that still returns a String error. A bun test regenerates the bindings
and fails when the committed file is stale.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

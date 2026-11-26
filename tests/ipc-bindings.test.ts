/**
 * Staleness of the committed IPC bindings (`src/lib/ipc/bindings.ts`, written only by
 * `bun run ipc:build`), the generator's parsing rules and refusals, and the adapters' kind lists.
 */

import { describe, expect, test } from 'bun:test';
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  BINDINGS,
  generate,
  main,
  parseCommand,
  parseEvents,
  parseHandler,
  parseKinds,
  REPO_ROOT,
} from '../scripts/ipc/build';
import { PLAYBACK_ERROR_KINDS } from '../src/lib/audio/playback-api';
import { COMMAND_ERROR_KINDS, COMMANDS, EVENTS } from '../src/lib/ipc/bindings';
import { PROJECT_ERROR_KINDS } from '../src/lib/project/api';
import { RECORDING_ERROR_KINDS } from '../src/lib/recording/types';

const REGENERATE = 'run `bun run ipc:build`';
const F = 'src-tauri/src/x.rs';

/** Runs `fn` and returns the message it throws. */
function refusal(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    return (e as Error).message;
  }
  throw new Error('expected a refusal');
}

/** A tree with `lib.rs` registering `entries` and the given module sources. */
function tree(entries: string[], modules: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), 'ipc-bindings-'));
  mkdirSync(join(root, 'src-tauri/src'), { recursive: true });
  mkdirSync(join(root, 'src/lib/ipc'), { recursive: true });
  writeFileSync(
    join(root, 'src-tauri/src/lib.rs'),
    `pub fn run() {\n    builder\n        .invoke_handler(tauri::generate_handler![\n${entries
      .map((e) => `            ${e},\n`)
      .join('')}        ])\n}\n`,
  );
  for (const [name, source] of Object.entries(modules)) {
    writeFileSync(join(root, `src-tauri/src/${name}.rs`), source);
  }
  return root;
}

describe('ipc bindings', () => {
  test('the committed bindings are current', () => {
    const committed = readFileSync(join(REPO_ROOT, BINDINGS), 'utf8');
    expect(committed, `${BINDINGS} is stale: ${REGENERATE}`).toBe(generate(REPO_ROOT));
  });

  test('the real tree', () => {
    const names = Object.keys(COMMANDS);
    expect(names).toContain('song_read');
    expect(names).toContain('recording_read');
    expect(names).toContain('export_audio_start');
    expect(Object.values(EVENTS).sort()).toEqual([
      'app://quit-requested',
      'control://request',
      'project://changed',
      'project://menu',
      'recording://level',
      'recording://state',
    ]);
  });

  test('each domain kind list is a subset of the command error kinds', () => {
    const known: readonly string[] = COMMAND_ERROR_KINDS;
    for (const list of [PROJECT_ERROR_KINDS, RECORDING_ERROR_KINDS, PLAYBACK_ERROR_KINDS]) {
      expect(list.filter((k) => !known.includes(k))).toEqual([]);
    }
  });
});

describe('parseHandler', () => {
  test('reads the entries in order with their lines', () => {
    const lib = 'x\n.invoke_handler(tauri::generate_handler![\n    a::one,\n    b::two,\n])';
    expect(parseHandler(lib)).toEqual([
      { module: 'a', name: 'one', line: 3 },
      { module: 'b', name: 'two', line: 4 },
    ]);
    expect(parseHandler('tauri::generate_handler![a::one, b::two]')).toEqual([
      { module: 'a', name: 'one', line: 1 },
      { module: 'b', name: 'two', line: 1 },
    ]);
  });
});

describe('parseCommand', () => {
  test('drops the injected parameters, and State<_, Arc<B>> does not split the list', () => {
    const src = `#[tauri::command]
pub async fn go(
    a: State<'_, A>,
    bridge: State<'_, Arc<B>>,
    app: AppHandle,
    window: WebviewWindow,
    w: Window,
    s: tauri::State<'_, C>,
    root: String,
) -> Result<(), CommandError> {
}
`;
    expect(parseCommand(src, F, 'go').args).toEqual([
      { key: 'root', type: 'string', optional: false },
    ]);
  });

  test('a doc line mentioning the attribute is not one; doc lines above it are skipped', () => {
    const src = `//! Every \`#[tauri::command]\` here is registered.
//! #[tauri::command]
/// Docs.
// A comment.
#[tauri::command]
pub fn go(id: u32) -> Result<(), CommandError> {}
`;
    expect(parseCommand(src, F, 'go').args).toEqual([
      { key: 'id', type: 'number', optional: false },
    ]);
  });

  test('a comment between the attribute and the fn is skipped', () => {
    const src = '#[tauri::command]\n/// Docs.\npub fn go() {}\n';
    expect(parseCommand(src, F, 'go')).toEqual({ args: [], error: null });
  });

  test('a one-line list and a one-per-line list with a trailing comma parse alike', () => {
    const one = `#[tauri::command]
pub fn go(app: AppHandle, session: u64, on: bool) -> Result<(), CommandError> {}
`;
    const many = `#[tauri::command]
pub fn go(
    app: AppHandle,
    session: u64,
    on: bool,
) -> Result<(), CommandError> {}
`;
    expect(parseCommand(one, F, 'go')).toEqual(parseCommand(many, F, 'go'));
    expect(parseCommand(one, F, 'go').args).toEqual([
      { key: 'session', type: 'number', optional: false },
      { key: 'on', type: 'boolean', optional: false },
    ]);
  });

  test('keys are camelCased, Option is optional and Channel is unknown', () => {
    const src = `#[tauri::command]
pub async fn go(
    device_id: Option<String>,
    channel: Option<u16>,
    path: PathBuf,
    name: &str,
    on_progress: Channel<X>,
    items: Vec<String>,
) -> Result<(), ExportError> {}
`;
    expect(parseCommand(src, F, 'go').args).toEqual([
      { key: 'deviceId', type: 'string', optional: true },
      { key: 'channel', type: 'number', optional: true },
      { key: 'path', type: 'string', optional: false },
      { key: 'name', type: 'string', optional: false },
      { key: 'onProgress', type: 'unknown', optional: false },
      { key: 'items', type: 'unknown', optional: false },
    ]);
  });

  test('no parameters gives no keys and Record<string, never>', () => {
    const root = tree(['m::go'], {
      m: '#[tauri::command]\npub fn go() -> Result<(), CommandError> {}\n',
    });
    try {
      const text = generate(root);
      expect(text).toContain('  go: [],\n');
      expect(text).toContain('  go: Record<string, never>;\n');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('the error type', () => {
    const error = (ret: string) =>
      parseCommand(`#[tauri::command]\npub fn go()${ret} {}\n`, F, 'go').error;
    expect(error(' -> Result<String, CommandError>')).toBe('command');
    expect(error(' -> Result<(), ExportError>')).toBe('export');
    expect(error(" -> Result<(), &'static str>")).toBe('text');
    expect(error(' -> String')).toBeNull();
    expect(error(' -> ControlInfo')).toBeNull();
    expect(error('')).toBeNull();
    expect(error(' -> Result<Vec<Option<X>>, CommandError>')).toBe('command');
    expect(error(' -> Result<tauri::ipc::Response, CommandError>')).toBe('command');
    expect(error(' -> Result<HashMap<String, Vec<u8>>, CommandError>')).toBe('command');
    expect(error('\n-> Result<\n    Option<PathBuf>,\n    ExportError,\n>')).toBe('export');
  });
});

describe('parseEvents', () => {
  test('column-0 &str consts only', () => {
    const src = `const REQUEST_EVENT: &str = "app://quit-requested";
pub const LEVEL_EVENT: &str = "recording://level";
pub(crate) const MENU_EVENT: &str = "project://menu";
const OTHER: &str = "x://y";
    const CORE_EVENT: u32 = 1;
    const INNER_EVENT: &str = "inner://event";

#[cfg(test)]
mod tests {
    const TEST_EVENT: &str = "test://event";
}
`;
    expect(parseEvents(src, F)).toEqual([
      { value: 'app://quit-requested', line: 1 },
      { value: 'recording://level', line: 2 },
      { value: 'project://menu', line: 3 },
    ]);
  });

  test('events are keyed by camelCased value, and two consts of one name both count', () => {
    const root = tree([], {
      a: 'const REQUEST_EVENT: &str = "app://quit-requested";\n',
      b: 'pub const REQUEST_EVENT: &str = "control://request";\n',
    });
    try {
      const text = generate(root);
      expect(text).toContain(
        "export const EVENTS = {\n  appQuitRequested: 'app://quit-requested',\n  controlRequest: 'control://request',\n} as const;",
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('parseKinds', () => {
  test('camelCases the variants and skips doc comments and attributes', () => {
    const src = `/// The kinds.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum CommandErrorKind {
    ProjectChanged,
    /// Only on an event.
    LimitReached,
    #[default]
    BadBatch, Other,
}

#[serde(rename_all = "camelCase")]
pub enum Subject {
    Song,
}

pub enum PlainKind {
    Raw(u8),
}
`;
    expect(parseKinds(src, F)).toEqual([
      {
        name: 'CommandErrorKind',
        variants: ['projectChanged', 'limitReached', 'badBatch', 'other'],
      },
    ]);
  });

  test('a kind enum is rendered as a const array and its union', () => {
    const root = tree([], {
      e: '#[serde(rename_all = "camelCase")]\npub enum FooKind {\n    BarBaz,\n}\n',
    });
    try {
      expect(generate(root)).toContain(
        "/** `FooKind` (src-tauri/src/e.rs), in declaration order. */\nexport const FOO_KINDS = [\n  'barBaz',\n] as const;\nexport type FooKind = (typeof FOO_KINDS)[number];\n",
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('refusals', () => {
  /** The message `generate` throws on a tree. */
  function refused(entries: string[], modules: Record<string, string>): string {
    const root = tree(entries, modules);
    try {
      return refusal(() => generate(root));
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }

  test('a registered name with no fn', () => {
    expect(refused(['m::one', 'm::gone'], { m: '#[tauri::command]\npub fn one() {}\n' })).toBe(
      'src-tauri/src/lib.rs:5: no #[tauri::command] fn `gone` in src-tauri/src/m.rs',
    );
  });

  test('an unregistered #[tauri::command]', () => {
    expect(refused([], { m: '\n#[tauri::command]\npub fn stray() {}\n' })).toBe(
      'src-tauri/src/m.rs:2: `stray` is a #[tauri::command] not registered in generate_handler!',
    );
  });

  test('a #[tauri::command(…)] with arguments', () => {
    const src = '#[tauri::command(rename_all = "snake_case")]\npub fn go() {}\n';
    expect(refusal(() => parseCommand(src, F, 'go'))).toBe(
      `${F}:1: a #[tauri::command(…)] with arguments changes the keys`,
    );
  });

  test('a String error says return CommandError', () => {
    const src = '#[tauri::command]\npub fn go() -> Result<(), String> {}\n';
    const message = refusal(() => parseCommand(src, F, 'go'));
    expect(message).toStartWith(`${F}:2: `);
    expect(message).toContain('return CommandError');
  });

  test('another error type', () => {
    const src = '\n#[tauri::command]\npub fn go() -> Result<(), Box<dyn Error>> {}\n';
    expect(refusal(() => parseCommand(src, F, 'go'))).toStartWith(
      `${F}:3: unsupported error type \`Box<dyn Error>\``,
    );
  });

  test('a // comment in a parameter list', () => {
    const src =
      '#[tauri::command]\npub fn go(\n    root: String,\n    // note\n    id: u32,\n) {}\n';
    expect(refusal(() => parseCommand(src, F, 'go'))).toBe(
      `${F}:4: a // comment in \`go\`'s parameters`,
    );
  });

  test('two consts with one event value', () => {
    expect(
      refused([], {
        a: 'const A_EVENT: &str = "project://changed";\n',
        b: '\nconst B_EVENT: &str = "project://changed";\n',
      }),
    ).toBe('src-tauri/src/b.rs:2: event `project://changed` collides with src-tauri/src/a.rs:1');
  });

  test('two parameters that camelCase alike', () => {
    const src =
      '#[tauri::command]\npub fn go(\n    song_id: String,\n    song__id: String,\n) {}\n';
    expect(refusal(() => parseCommand(src, F, 'go'))).toBe(
      `${F}:4: two parameters of \`go\` camelCase to \`songId\``,
    );
  });

  test('a kind variant with fields', () => {
    const src = '#[serde(rename_all = "camelCase")]\npub enum AKind {\n    One,\n    Two(u8),\n}\n';
    expect(refusal(() => parseKinds(src, F))).toBe(
      `${F}:4: AKind variant \`Two(u8)\` is not a unit variant`,
    );
  });

  test('a kind variant with #[serde(rename)]', () => {
    const src =
      '#[serde(rename_all = "camelCase")]\npub enum AKind {\n    One,\n    #[serde(rename = "x")]\n    Two,\n}\n';
    expect(refusal(() => parseKinds(src, F))).toBe(
      `${F}:4: a #[serde(rename…)] on a AKind variant`,
    );
  });

  test('an unreadable fn after the attribute', () => {
    const src = '#[tauri::command]\nfn private() {}\n';
    expect(refusal(() => parseCommand(src, F, 'private'))).toStartWith(`${F}:2: `);
  });
});

describe('main', () => {
  /** A copy of the real `src-tauri/src/*.rs` in a fresh directory. */
  function copy(): string {
    const root = mkdtempSync(join(tmpdir(), 'ipc-main-'));
    mkdirSync(join(root, 'src-tauri/src'), { recursive: true });
    mkdirSync(join(root, 'src/lib/ipc'), { recursive: true });
    for (const f of readdirSync(join(REPO_ROOT, 'src-tauri/src'))) {
      if (f.endsWith('.rs'))
        cpSync(join(REPO_ROOT, 'src-tauri/src', f), join(root, 'src-tauri/src', f));
    }
    return root;
  }

  test('an argument exits 2', () => {
    expect(
      main(
        ['--write'],
        REPO_ROOT,
        () => {},
        () => {},
      ),
    ).toBe(2);
  });

  test('writes the bindings and exits 0', () => {
    const root = copy();
    try {
      const lines: string[] = [];
      expect(
        main(
          [],
          root,
          (l) => lines.push(l),
          () => {},
        ),
      ).toBe(0);
      expect(readFileSync(join(root, BINDINGS), 'utf8')).toBe(
        readFileSync(join(REPO_ROOT, BINDINGS), 'utf8'),
      );
      expect(lines).toEqual([`wrote ${BINDINGS} (46 commands, 6 events, 15 kinds)`]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('a String command exits 1 and writes nothing', () => {
    const root = copy();
    try {
      const file = join(root, 'src-tauri/src/commands.rs');
      writeFileSync(
        file,
        readFileSync(file, 'utf8').replace(
          'pub fn app_info() -> Result<AppInfo, CommandError> {',
          'pub fn app_info() -> Result<AppInfo, String> {',
        ),
      );
      const errors: string[] = [];
      expect(
        main(
          [],
          root,
          () => {},
          (l) => errors.push(l),
        ),
      ).toBe(1);
      expect(errors).toHaveLength(1);
      expect(errors[0]).toMatch(/^src-tauri\/src\/commands\.rs:\d+: .*return CommandError$/);
      expect(existsSync(join(root, BINDINGS))).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

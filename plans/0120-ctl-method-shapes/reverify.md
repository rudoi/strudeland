# 0120 step 0: re-verification on main 3a189da

The issue stands. Cited sites, as they are now:

- **Rpc trait.** `crates/strudeland-ctl/src/methods.rs:90-94` (`type Params: Serialize; type Result: DeserializeOwned`), impls at :129-208. There are 16 methods (`Method::ALL`, :36); `instrument.apply` is new since the review. The only typed caller is `Client::call::<M>` (`client.rs:100-106`), used by its own tests, and it owns the text `"invalid {} result: {e}"`.
- **The CLI calls untyped.**
  - `Transport::call(Method, Value) -> Value` is at `crates/strudeland-cli/src/run.rs:41-44`, implemented by `SocketTransport` (`transport.rs:34-46`).
  - The CLI's own `call<T>` (run.rs:158-166) repeats the "invalid {method} result" text. `params()` is at :168.
  - There are 16 call sites:
    - `report(...)` at run.rs:255, 272, 280, 300, 317, 327, 335, 343, 354, 371, 383 and 438;
    - `eval_like` (:815-824), which serves both Eval and Play;
    - `code.get` (:398);
    - `offline_project` (:531);
    - `run/instruments.rs:115`, which the MCP server reaches.
  - Raw values are used only by `emit`'s `--json` output (:171) and the `code.get --json` branch (:399-400).
  - The trait is used as `&mut dyn Transport`, with fakes `NoTransport` (render.rs:436) and `Fake` (run.rs:876).
- **The webview validates params by hand.**
  - The helpers are at `src/lib/control/handler.ts:82-125`, and `eventsParams` is at :163-197.
  - The per-method checks sit inside the handlers, interleaved with semantic checks. For example, `song.rename` and `song.delete` resolve the song before checking their other params.
- **Rust validates two methods ad hoc.**
  - `screenshot_path(&Value)` (`src-tauri/src/control.rs:283-318`) is tested at :1051-1117.
  - The `project.open` absolute-path check (:380-385) runs after the NOT_READY check.
- **Event limits are written in four places:**
  - methods.rs:473 ("Absent means 2000");
  - handler.ts:159-160 (`EVENTS_LIMIT`, `EVENTS_LIMIT_MAX`);
  - cli.rs:74-75 (help text and `range(1..=100000)`);
  - `SKILL.md:381`.
- **Wire shapes are cross-checked by regex** in `tests/control-protocol.test.ts`:
  - error codes (:19-21; protocol.rs:12 keeps one const per line for that regex);
  - LiveStats, BatchDecode and InvokeRtt (:71-114);
  - Levels (:116-135);
  - METHOD_NAMES (:137-144).

  Nothing else in `AppState`, no other result, and no params are cross-checked. So the TS result interfaces (`protocol.ts:94-220`) can drift silently.
- **State and events JSON are hand-written:**
  - methods.rs:1027 `STATE_EXAMPLE`, plus partial "older app" replies that test backward compatibility on purpose;
  - run.rs:930-941 `state_json()` and :1363 `events_json()`;
  - e2e.rs:64-77 and :143-153;
  - instrument_mcp.rs:217.

  `output.rs base()` is a typed literal, so the compiler already checks it.
- **#119 left the control path as it was.** `BridgeRequest` still forwards the raw params `Value` (control.rs:81-90).

## Why Rust will not validate params first

Deserializing params in control.rs with `deny_unknown_fields`, as the issue suggests, would change the wire:

- **Every text changes.** serde writes "unknown field `foo`, expected one of …", where the webview writes "invalid params: unknown parameter 'foo'".
- **The first error reported can change.** serde reports in document order; the webview reports unknown keys first, then fields in a fixed order.
- **Some requests accepted today would be refused.** Floats such as `{"limit": 5.0}`, and `bars.from` above `u32` but up to `MAX_SAFE_INTEGER`, pass the webview but not serde.
- **Some requests refused today would be accepted.** `{"song": null}`, `{"song": ""}` and `{"section": ""}` are all valid to serde.
- **Codes change for `song.rename` and `song.delete`.** They return NO_SUCH_SONG or NO_PROJECT before INVALID_PARAMS today (pinned by control-handler.test.ts:1495-1499).
- **NOT_READY can become INVALID_PARAMS.** Today, malformed params sent with no webview attached give NOT_READY.

The webview stays the single owner of params validation. Rust keeps only `screenshot` and the native absolute check for `project.open`, at their current places.

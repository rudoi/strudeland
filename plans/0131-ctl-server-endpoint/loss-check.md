# 0131 loss check

The two stages cover the plan in full. Nothing is dropped. The out-of-scope items are the
proposal's **Out** list; the macOS peer check is task #162.

| Plan item | Stage |
|---|---|
| Exact-text pins on the unchanged app code (the two strengthened tests and the four new ones), logged green before any ctl code | 001 |
| The literal inventory (quotes included) and the same-tests script | 001 (both scripts), 001-002 (literals) |
| ctl `Cargo.toml`: the `server` feature, the optional Unix tokio, the target dev-dependencies, no self dev-dependency; `Cargo.lock` byte-identical | 001 (written), 001-002 (lock gate) |
| `lib.rs`: `#[cfg(any(test, feature = "server"))] pub mod server;` and the module doc | 001 |
| `server/mod.rs`: `Bind` (Debug), `AcceptError` with `closed()` and `pause()`, `answer_one_ping` | 001 |
| `server/unix.rs`: the helpers copied verbatim, `bind`, `Endpoint` with `accept`, `accept_verified`, `close`, `Drop` | 001 |
| `server/windows.rs`: `bind` with `owned_existing`, the next-instance `accept`, `close`, `Drop` | 001 |
| ctl tests: the 14 moved items byte-identical, the round trip, close and drop, the rejected peer, the Linux and Windows tests, each `bind` on an entered runtime | 001 |
| Lints for macOS and both cross targets with `server` off, `--lib --features server` and `--all-targets`; Linux tests in Docker | 001-002 |
| The app on `server`: the manifest feature, `Server`, `set_server`, `close_server`, one `listen`, one `serve` | 002 |
| `start` keeps its signature and calls `start_on`; the plain start-path test on a temp path (Unix and Windows) | 002 |
| The endpoint code, `remove_socket` and the copied tests deleted from the app; the serve tests adapted with unchanged assertions | 002 |
| No platform `cfg` outside tests in `control.rs`; the `control: ` literals byte-identical | 002 |
| `bindings.ts` unchanged, `bun test` | 002 |
| AGENTS.md: the ctl bullet (001, neutral on who enables the feature; 002 adds "on in the app only") and the `control.rs` bullet; the `control.rs` module doc | 001-002 |

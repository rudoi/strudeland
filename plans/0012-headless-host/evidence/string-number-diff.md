# `String(number)` differential

Measured 2026-11-12 on the development Mac (Apple M3 Max, macOS 26.3).

Why: number-to-shortest-string is the one place where a V8 version difference would change job
bytes silently. The embedded V8 (`v8` crate `=152.2.0`, V8 `15.2.124.1-rusty`) is not the V8 of
the Chromium Playwright drives (Chromium `156.0.8078.4`, Playwright `1.64.1`).

## Method

- The set `S` (`scripts/host/string-number-lib.ts`, seed `0x5eed0012`, mulberry32): 200 000
  finite doubles, as hex bit patterns: 100 000 uniformly random bit patterns (NaN and infinity
  patterns redrawn), 25 000 `±k / 2^j` (`k` in 1..1 000 000, `j` in 0..63), 25 000 `±k / 10^j`
  (`k` in 1..1 000 000, `j` in 0..22), 50 000 products `a/100 × b/100` of two 3-digit decimals
  (`a, b` in 100..999). The first values are pinned by `tests/host-string-number.test.ts`.
- `bun scripts/host/string-number.probe.ts` writes the inputs to
  `target/string-number-inputs.json` and evaluates `String(x)` and `JSON.stringify([x])` for each in
  a headless Chromium page (`target/string-number-chromium.json`).
- `cargo test --release -p strudeland-host --test string_number -- --ignored --nocapture`
  evaluates the same two expressions in the embedded V8 (`Host::eval_script`, natives installed)
  and compares, input by input, with (a) Chromium and (b) the engine's `number_to_string`
  (`crates/strudeland-engine/src/js.rs`; for JSON, `[` + it + `]`).

## Result

```
string_number: 200000 inputs; Chromium 156.0.8078.4; embedded V8 15.2.124.1-rusty
String(x): Chromium vs embedded V8: 0 differences
JSON.stringify([x]): Chromium vs embedded V8: 0 differences
String(x): engine number_to_string vs embedded V8: 0 differences
JSON.stringify([x]): engine number_to_string vs embedded V8: 0 differences
```

No differences, so no first-20 list. The test passes; the embedded V8, Chromium's V8 and the
engine's port agree on every input.

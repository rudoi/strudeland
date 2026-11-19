# Math verification: embedded V8 against Chromium

Reference: `crates/strudeland-engine/tests/refs/v8-math.json`, evaluated in Chromium 156.0.8078.4
(V8 `b976f35`, Playwright 1.64.1, darwin-arm64). Run on 2026-11-12 on macOS arm64 with
`HOST_MATH_EVIDENCE=plans/0012-headless-host/evidence/math-verification.md cargo test -p
strudeland-host --test math`; the table below is that output. Every reference point of every
group the host runs matches bit for bit (`-0` and `+0` differ; any two NaNs are equal; the
`distort:*` groups are engine-internal and not run here). All fourteen functions of the
verification set match natively, so no `jsmath` port was made and `v8-math-choice.json` was not
written for this task. The `**` operator (`powOp`, 2051 points over five groups) matches
Chromium; no known difference is recorded.

Embedded V8 15.2.124.1-rusty; reference `crates/strudeland-engine/tests/refs/v8-math.json`.

| Name | Class | Groups | Points | Mismatches | V8 |
|---|---|---|---|---|---|
| `sin` | overridden | 2 | 1068 | 0 | 15.2.124.1-rusty |
| `cos` | overridden | 2 | 1068 | 0 | 15.2.124.1-rusty |
| `tanh` | overridden | 1 | 526 | 0 | 15.2.124.1-rusty |
| `exp` | overridden | 1 | 520 | 0 | 15.2.124.1-rusty |
| `expm1` | overridden | 1 | 518 | 0 | 15.2.124.1-rusty |
| `log1p` | overridden | 1 | 518 | 0 | 15.2.124.1-rusty |
| `cosh` | overridden | 1 | 518 | 0 | 15.2.124.1-rusty |
| `pow` | overridden | 10 | 4329 | 0 | 15.2.124.1-rusty |
| `random` | thrown | 0 | 0 | throws (checked) | 15.2.124.1-rusty |
| `E` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `LN10` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `LN2` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `LOG10E` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `LOG2E` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `PI` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `SQRT1_2` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `SQRT2` | constant | 0 | 0 | bit pattern equals `std::f64::consts` | 15.2.124.1-rusty |
| `abs` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `ceil` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `floor` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `trunc` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `sign` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `sqrt` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `fround` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `f16round` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `clz32` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `imul` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `max` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `min` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `round` | exact | 1 | 395 | 0 | 15.2.124.1-rusty |
| `sumPrecise` | exact | 0 | 0 | exact by the spec, not compared | 15.2.124.1-rusty |
| `tan` | verified native | 2 | 534 | 0 | 15.2.124.1-rusty |
| `asin` | verified native | 2 | 522 | 0 | 15.2.124.1-rusty |
| `acos` | verified native | 2 | 522 | 0 | 15.2.124.1-rusty |
| `atan` | verified native | 2 | 522 | 0 | 15.2.124.1-rusty |
| `atan2` | verified native | 2 | 180 | 0 | 15.2.124.1-rusty |
| `sinh` | verified native | 2 | 520 | 0 | 15.2.124.1-rusty |
| `asinh` | verified native | 2 | 520 | 0 | 15.2.124.1-rusty |
| `acosh` | verified native | 2 | 516 | 0 | 15.2.124.1-rusty |
| `atanh` | verified native | 2 | 519 | 0 | 15.2.124.1-rusty |
| `log` | verified native | 2 | 518 | 0 | 15.2.124.1-rusty |
| `log2` | verified native | 2 | 556 | 0 | 15.2.124.1-rusty |
| `log10` | verified native | 2 | 536 | 0 | 15.2.124.1-rusty |
| `cbrt` | verified native | 2 | 524 | 0 | 15.2.124.1-rusty |
| `hypot` | verified native | 2 | 177 | 0 | 15.2.124.1-rusty |
| `powOp` | `**` operator | 5 | 2051 | 0 | 15.2.124.1-rusty |
| `toInt32` | `x \| 0` operator | 1 | 24 | 0 | 15.2.124.1-rusty |

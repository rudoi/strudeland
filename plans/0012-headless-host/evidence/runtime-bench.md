# V8 benchmark and performance budget

Historical release measurements from 2026-11-12 on an Apple M3 Max (14 cores), 36 GB, macOS 26.3,
with Rust 1.94.1 and V8 15.2.124.1-rusty (`v8` crate 152.2.0). The archived aggregate results
are in [runtime-bench.json](runtime-bench.json). V8 was chosen after the
[QuickJS spot timings](quickjs-spot-timings.md).

## Full-song planning budget

The release `strudeland-host bench` planned the full linear timeline of 26 benchmark songs with a
fresh host per request. The planner initially queried each full timeline twice. Reusing the
full-schedule query when it matched the chunk window halved that work and produced byte-identical
jobs in all 26 cases. Other processes were compiling in the background during these measurements.

| Metric | Limit | Before query reuse | After query reuse |
|---|---|---|---|
| Slowest full-song plan | ≤ 2000 ms | 7908.9 ms | 3336.5 ms |
| Full-song plans over the limit | 0 | 3 | 1 |
| Median full-song plan | ≤ 500 ms | 650.6 ms | 393.3 ms |
| Median per-request load | ≤ 100 ms | 34.6 ms | 31.5 ms |

The budget stayed fixed. Profiling found that Strudel's rational-time arithmetic dominated the
query. The subsequent Fraction fast path brought the outlier to about 1.1 s.

A startup snapshot was unnecessary under the recorded rule. Median load remained below both
100 ms and 25% of median plan time. A clean host crate rebuild with cached dependencies took
17.8 s and yielded a stripped 44 MB release binary.

## Regression command

The release test automatically selects the widest window in the current default corpus.

```sh
cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture
```

# QuickJS spot timings (abandoned spike)

Measured 2026-11-12 on Apple Silicon with the QuickJS-NG host (`rquickjs =0.14.0`). Two 16-bar
release requests took 7.4–9.2 s in QuickJS. The same requests later took 0.64–0.74 s in V8,
a speedup of roughly 12×. Small corpus windows took about 70–150 ms each in the QuickJS debug
build.

For bass-line, clipping, duck, sine-basic and voice-mod, the QuickJS plan matched the Chromium
reference after dropping provenance and sorting object keys with `jq -S`.

These measurements informed the choice of V8 for the headless host.

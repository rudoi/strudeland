// Measure item: the quantum a worklet source first runs in. Five one-voice supersaws, one per
// 0.4 s slot of a 2 s bar (cps 0.5), each moved by `.late` so that its onset t (in seconds) is
// N/44100 with N = 128k + offset: offset 1 (N 257), 64 (N 17984), 127 (N 35711), and two on a
// quantum boundary, offset 0 with f32(t) > t (N 53376, k 417) and with f32(t) < t (N 70912,
// k 554). late = N/88200 - slot/5 cycles. `engine:refs --write` records the quantum of each
// voice's first non-zero sample against the rule (first quantum q with q*128/44100 > f32(t)).
const meta = { bpm: 120, meter: '4/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ ~ ~ ~").s('supersaw').unison(1).attack(0.001).legato(0.25).release(0.01).late(0.002913832199546485)
$: note("~ a3 ~ ~ ~").s('supersaw').unison(1).attack(0.001).legato(0.25).release(0.01).late(0.0039002267573695903)
$: note("~ ~ a3 ~ ~").s('supersaw').unison(1).attack(0.001).legato(0.25).release(0.01).late(0.004886621315192741)
$: note("~ ~ ~ a3 ~").s('supersaw').unison(1).attack(0.001).legato(0.25).release(0.01).late(0.005170068027210872)
$: note("~ ~ ~ ~ a3").s('supersaw').unison(1).attack(0.001).legato(0.25).release(0.01).late(0.003990929705215418)

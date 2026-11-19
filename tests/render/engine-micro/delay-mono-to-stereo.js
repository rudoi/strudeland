// Micro case: a mono delay becomes stereo. Bar 0: a mono note fills a 0.05 s delay (feedback 0.7)
// that keeps ringing. Bar 1: a panned (stereo) note sends to it; from the pause frame on the line
// is stereo, the history up-mixed with the same write index (and the node's output bus is
// re-created, so the feedback reads silence for that quantum). Only the delay is heard (dry 0).
// Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<a3 ~>").s('triangle').clip(0.6).dry(0).delay(1).delaytime(0.05).delayfeedback(0.7)
$: note("<~ [~ e4]>").s('sine').decay(0.04).sustain(0).pan(0.3).dry(0).delay(1).delaytime(0.05).delayfeedback(0.7)

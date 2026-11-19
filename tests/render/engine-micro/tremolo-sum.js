// Micro case: tremolo on a constant source, four 0.125 s notes one after the other: the AM gain's
// intrinsic max(1 - depth, 0) (a constructor value, no event) plus the LFO connection summed per
// frame, depth 0.5 (0.5 to 0.854) and 1.5 (gain 0 plus an LFO clipped flat at 1),
// tremolosync(8) as cps * 8 = 32 Hz, and tremolophase(0.25) added to the start phase from
// time = cycle / cps. No shape: the triangle with skew 1 (a rising ramp). Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: s("<[one ~] ~>").sustain(1).tremolo(16).tremolodepth(0.5)
$: s("<[~ one] ~>").sustain(1).tremolo(16).tremolodepth(1.5)
$: s("<~ [one ~]>").sustain(1).tremolosync(8)
$: s("<~ [~ one]>").sustain(1).tremolo(16).tremolophase(0.25)

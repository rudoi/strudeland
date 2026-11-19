// Micro case: the djf dead zone: 0.5 and 0.505 (between 0.49 and 0.51) copy the input and leave
// the filter state as it was; back at 0.2 the lowpass resumes from that stale state. Carriers and
// the held sawtooth as in djf-lp-hp. Four 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: note("<a2 ~ ~ ~>").s('sawtooth').clip(8)
$: s("one").gain(0).djf("<0.2 0.5 0.505 0.2>")

// Micro case: the djf sits after the orbit's summing node, so the delay return goes through it
// with the voices (a lowpass at (0.6 * 11)^4 = 1897.5 Hz): one voice and the return sound at
// the summing node. (A reverb return is filtered the same way; it is left out because the
// convolver itself nulls only at about -140 dB, the README's "Accelerate reductions".) Four 0.5 s
// bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: note("a3 ~").s('sawtooth').djf(0.3).delay(0.5).delaytime(0.06).delayfeedback(0.6)

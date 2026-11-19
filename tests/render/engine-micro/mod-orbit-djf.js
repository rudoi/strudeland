// Micro case: an lfo on the orbit djf's value (orbit-held, decision 13): each short note calls
// getDjf(0.3) (setValueAtTime(0.3, t)) and connects its lfo ({ c: 'djf', r: 2, da: 0.2 }) to
// the orbit's djf value, which filters the whole orbit; the orbit sums the inputs of its voices
// after the voices of the quantum. A held a2 (clip 3, no djf of its own) keeps the summing node
// connected for the whole render: without it the djf's input loses its last connection when a
// note's nodes are released (a main-thread timeout, a run-dependent quantum) and the worklet
// stops filtering. At most two notes sound at once. Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: note("<a2 ~ ~>").s('sawtooth').clip(3)
$: note("a3 ~ ~ ~ a3 ~ ~ ~").s('sawtooth').djf(.3).lfo({ c: 'djf', r: 2, da: .2 })

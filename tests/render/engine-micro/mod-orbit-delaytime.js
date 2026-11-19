// Micro case: an lfo on the orbit delay's delayTime (orbit-held, decision 13). Two short notes
// per bar feed the delay only (dry 0); each hap's lfo ({ c: 'delaytime', r: 3, dep: 0.5 }) reads
// currentValue from delayTime.value: what the last rendered quantum left, so bar 1's two haps
// read 0.02 (bar 0's time) although bar 1's first hap schedules setValueAtTime(0.03), and bar
// 2's read 0.03. depth 0.5 * currentValue, no range (delayTime). The lfos of two haps never
// overlap. (An fxr(0.2) that kept the voice alive past its sound made Chromium's renders differ
// between runs: the lfo then ends together with the main-thread releaseNodes of the chain.)
// Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: note("a3 ~ ~ ~ a3 ~ ~ ~").s('sawtooth').clip(.3).dry(0).delay(1).delayfeedback(.4).delaytime("<.02 .03 .03>").lfo({ c: 'delaytime', r: 3, dep: .5 })

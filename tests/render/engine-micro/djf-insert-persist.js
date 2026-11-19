// Micro case: when the orbit djf appears and what it keeps. Bar 0: a held sawtooth without djf
// (no djf node yet). Bar 1: a note with djf(0.2) at 0.75 s (frame 33075, mid-quantum): the node is
// inserted at bar 1's pause frame with `.value = 0.2`, so the held note is lowpassed from that
// pause, not from the onset. Bar 2: a note without djf, filtered at the last value. Bar 3: djf(0.8)
// at 1.75 s (frame 77175): the highpass starts at the first quantum at or after it (77184). At
// most two notes sound at once. Four 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: note("<a2 ~ ~ ~>").s('sawtooth').clip(8)
$: note("<~ [~ e3] ~ [~ e3]>").s('sawtooth').djf("<~ 0.2 ~ 0.8>")
$: note("<~ ~ [~ c3 ~ ~] ~>").s('sawtooth')

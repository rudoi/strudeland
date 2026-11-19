// Micro case: oscillators whose first quantum takes the a-rate path, which starts at phase 0
// whatever the fractional onset: a note 28.665 frames into the pause quantum (its frequency
// event is at that quantum's start) and a note with a zero-depth pitch envelope (detune events)
// at 11053.665 frames. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a4 ~").s('sawtooth').late(0.0013)
$: note("~ e5").s('sine').penv(0).late(0.0013)

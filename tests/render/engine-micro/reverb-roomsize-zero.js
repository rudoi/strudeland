// Micro case: roomsize 0 is an `ir` error. Bar 0 creates the reverb (0.15 s); in bar 1 another
// hap asks for roomsize 0: no IR (the bar-0 IR stays and rings on), and that voice is never
// connected to the orbit: only its delay send, made before the reverb send, is heard. The two
// sounds are separate patterns so that at most two inputs sound at once at the summing node
// (Chromium adds three or more in an order that changes between runs). Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<a3 ~>").s('triangle').decay(0.05).sustain(0).room(0.5).roomsize(0.15).roomlp(0)
$: note("<~ e4>").s('triangle').decay(0.05).sustain(0).room(0.5).roomsize(0).delay(0.4).delaytime(0.03).delayfeedback(0.4)

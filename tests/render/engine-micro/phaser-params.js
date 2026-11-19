// Micro case: the phaser controls, one 0.125 s note each: phaserdepth 0.2 gives Q 1.6 (centre
// 500, sweep 800), 1.5 clamps Q to 0.1, phaserdepth 0 builds no phaser, and centre 2000 with
// sweep 3000 at the default depth 0.75 (Q 0.5). Each note fades to exactly 0 with FXrelease
// 0.05 (the LFO's end moves with it), so superdough's cleanup, which waits for endWithRelease,
// never cuts a sounding notch ring. Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<[a3 ~] ~>").s('sawtooth').phaserrate(8).phasercenter(500).phasersweep(800).phaserdepth(0.2).FXrelease(0.05)
$: note("<[~ a3] ~>").s('sawtooth').phaserrate(8).phasercenter(500).phasersweep(800).phaserdepth(1.5).FXrelease(0.05)
$: note("<~ [a3 ~]>").s('sawtooth').phaserrate(8).phaserdepth(0).FXrelease(0.05)
$: note("<~ [~ a3]>").s('sawtooth').phaserrate(8).phasercenter(2000).phasersweep(3000).FXrelease(0.05)

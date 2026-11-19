// Micro case: the compressor's params on a constant source, one setting per 0.25 s: ratio 30
// clamps to 20 (threshold -30), knee 45 clamps to 40, attack 0 takes the 0.001 s floor, release 0
// gives an infinite release rate (the gain jumps to 1 and the NaN of 0 * inf is caught by
// min(1, NaN)), and the colon form "-20:4:6:0.01:0.2" is threshold:ratio:knee:attack:release.
// Every node is created before 1.63 s, so each one warms up on silence. Four 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 4 }] };

$: s("<[one ~] ~ ~ ~>").attack(0.001).sustain(1).compressor(-30).compressorRatio(30)
$: s("<[~ one] ~ ~ ~>").attack(0.001).sustain(1).compressor(-30).compressorKnee(45)
$: s("<~ [one ~] ~ ~>").attack(0.001).sustain(1).compressor(-30).compressorAttack(0)
$: s("<~ [~ one] ~ ~>").attack(0.001).sustain(1).compressor(-30).compressorRelease(0)
$: s("<~ ~ [one ~] ~>").attack(0.001).sustain(1).compressor("-20:4:6:0.01:0.2")

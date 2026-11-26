// Synthetic load for the scheduler, live dropout, callback allocation and IPC decode tests.
// Regenerate the checked-in job from the repository root:
// cargo run -p strudeland-host -- plan-render --song tests/bench/live-pressure.js \
//   --request '{"length":{"kind":"window","fromBar":0,"toBar":12,"layout":"linear"},"chunk":{"overlapBars":0,"lookbackSec":0,"mode":"mix"}}' \
//   | jq -j '.chunks[0].json' > crates/strudeland-live/tests/data/live-pressure.job.json
const meta = {
  title: 'Live pressure',
  bpm: 128,
  meter: '12/8',
  seed: 37,
  sections: [{ name: 'orchestra', bars: 12, loop: true }],
};

const echo = (p) =>
  p.orbit(1).room(0.32).roomsize(9).rlp(5400).rfade(7)
    .delay(0.22).delaytime(0.37).delayfeedback(0.38);
const chamber = (p) => p.orbit(2).room(0.48).roomsize(9).rlp(3200).rfade(7);

// Independent grids cause short bursts of simultaneous note starts and staggered releases.
const pulse = echo(
  note("<[63 51 57]*4 [65 54 62 47]*3 [61 58 49 68 55 64]*2>")
    .s("pulse").pw(0.38).pwrate(0.19).pwsweep(0.29)
    .lpf("<1700 2600 3900 2900>").hpf(180)
    .vib(5.1).vibmod(0.045).pan(0.28).clip(0.9)
    .attack(0.025).decay(0.19).sustain(0.39).release(0.46).gain(0.11),
);
const saw = echo(
  note("<[71 64 67 59 63]*2 [56 69 62 57 63]*2 [60 73 65 63 67]*2>")
    .s("sawtooth").lpf("<3800 5600 4550>").hpf(250)
    .vib(3.7).vibmod(0.061).pan("0.62 0.41").clip(0.85)
    .attack(0.012).decay(0.22).sustain(0.47).release(0.51).gain(0.073),
);
const hiss = echo(
  s("pink*6").hpf("<1400 1900 2400>").lpf(4700).lpq(0.3).pan("0.31 0.73")
    .vib(2.9).vibmod(0.024).attack(0.06).decay(0.18).sustain(0.3).release(0.27).gain(0.035),
);
const reed = chamber(
  note("<[49 56 43 53]*2 [53 58 46 50]*2 [51 44 59 48]*2>")
    .s("triangle").shape(0.48).lpf(1800).hpf(90).pan("0.8 0.22")
    .attack(0.095).decay(0.32).sustain(0.55).release(0.66).gain(0.092),
);
const bands = chamber(
  note("<[45,52,60,66] [42,53,57,64] [46,51,58,63] [41,48,56,62] [44,50,55,65]>*6")
    .s("sawtooth").lpf("<940 1180 1460>").lpq(0.28).hpf(110).pan(0.75)
    .attack(0.14).decay(0.46).sustain(0.41).release(2.42).gain(0.032),
);
const sub = chamber(
  note("<[[33,45] [37,49] [32,44] [40,52]] [[38,50] [31,43] [36,48] [34,46]]>")
    .s("sine").shape(0.27).lpf(1240).lpq(0.4)
    .penv("7 -5 9 -4").pattack(0.23).psustain(1)
    .attack(0.043).decay(0.48).sustain(0.65).release(0.86).gain(0.09),
);
const ticks = echo(
  note("<[80 77 86 83] [87 79 84 78]>").s("triangle").shape(0.33).lpf(6200).hpf(600)
    .pan("0.13 0.88").clip(0.58).attack(0.007).decay(0.07).sustain(0.33)
    .release(0.2).gain(0.057),
);
const accent = echo(
  note("<[[48,51,55,57,61,67,70,73,77,79,82] ~@7] ~ ~ ~>")
    .s("sawtooth").lpf(3100).hpf(120).pan(0.39)
    .vib(4.2).vibmod(0.038).clip(0.75)
    .attack(0.015).decay(0.2).sustain(0.27).release(0.74).gain(0.022),
);

$: song(meta, { orchestra: stack(pulse, saw, hiss, reed, bands, sub, ticks, accent) });

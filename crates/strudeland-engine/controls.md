# Control audit

Every control key a hap value can carry, classified for synth voices. The sources audited are the
`value.*` reads (direct, destructured, through the filter maps and through `applyFM`'s computed
names) in `superdough.mjs`, `synth.mjs`, `helpers.mjs`, `noise.mjs` and `superdoughoutput.mjs`
(`vendor/strudel/packages/superdough/`), plus every canonical control registered in
`vendor/strudel/packages/core/controls.mjs` (aliases resolve to the canonical name before
superdough sees the value), the secondary names of the multi-value controls (`room`'s `size`,
`color`'s `colour`), the list controls `partials` and `phases` (`core/pattern.mjs`), the
codemirror `markcss` control, and the sampler-only names the plan lists (`slice`, `splice`, `fit`).

- `read sites`: where superdough reads the key (`-`: never read for any sound).
- `class`: `implemented` (the engine reads it), `ignored` (superdough produces no sound from it for
  a synth voice: never read, sampler, wavetable, supersaw, pulse or bytebeat only, the analyser
  that is skipped offline, the `ir` sample index), or `unsupported` (read by
  superdough, not implemented: the sample IRs, `stretch` and the user-supplied DSP keys). A key
  missing from this table is unsupported.
- `engine module`: the module that implements it.
- `status`: `implemented` when the engine acts on it, `-` otherwise.
- `owner`: for an `unsupported` row, the historical audit identifier (`#<n>`). The
  `implemented` and `ignored` rows use `-`.

Rows pinned by a Chromium micro case (the README's References table): `s` (sine) and `note`
(`osc-sine`), and `attack`, `decay`, `sustain`, `release` (`adsr-defaults`, `adsr-attack-only`,
`adsr-sustain-only`, `adsr-decay-only`, `adsr-long-attack`, `adsr-short-hap`,
`param-linear-ramp`), the filter, pan and level rows (`filter-*`, `pan-*`, `level-*`), and the
shaper and transient rows (`coarse-*`, `crush-*`, `shape-*`, `distort-*`, `transient-*`), the
ladder rows `ftype` (`'ladder'`) and `drive` (`ladder-*`, `measure-worklet-teardown`), `djf`
(`djf-*`), the synth worklet rows `detune`, `unison`, `spread` and `pw` (`supersaw-*`,
`pulse-*`, `measure-worklet-start`; the sounds `supersaw` and `pulse`), and the `lfo` processor's
users: the `tremolo*` rows (`tremolo-*`, `lfo-shapes`), `pwrate` and `pwsweep` (`pulse-pwm`;
superdough reads them only for a `pulse`) and the 21 filter LFO rows `lp`/`hp`/`bp` x `rate`,
`sync`, `depth`, `depthfrequency`, `shape`, `dc`, `skew` (`filter-lfo-*`), and the native effects:
the five `compressor*` rows (`compressor-*`, judged on macOS only), `vowel` (`vowel-formants`, a
measured-floor case, and `vowel-unknown`), the four `phaser*` rows (`phaser-*`) and `FXrelease`
(`fxrelease-fade`), the four `duck*` rows (`duck-*`), the routing rows `bus`, `busgain` and `n`
(the sound `bus`; `bus-sound`), the modulator rows `lfo`, `env` and `bmod` (`mod-*`,
`env-staircase`, `bus-bmod`), and `FX` with the chain read sites of `delay*`, `room*` and `dry`
(`fx-chain-*`).

`ALLOWED_KEYS` in `src/controls.rs` is exactly the `implemented` and `ignored` rows; a unit test
parses this file and checks both directions. Sounds are `ALLOWED_SOUNDS`; the refused ones are
listed at the end.

| key | read sites | class | engine module | status | owner |
|---|---|---|---|---|---|
| FX | superdough.mjs:522, superdough.mjs:611 | implemented | chain | implemented | - |
| FXrelease | superdough.mjs:523 | implemented | chain | implemented | - |
| accelerate | - | ignored | - | - | - |
| activeLabel | - | ignored | - | - | - |
| amp | - | ignored | - | - | - |
| analyze | superdough.mjs:520 | ignored | - | - | - |
| anchor | - | ignored | - | - | - |
| attack | synth.mjs:48, synth.mjs:162, synth.mjs:249, synth.mjs:318, synth.mjs:379, synth.mjs:414 | implemented | adsr | implemented | - |
| bandf | superdough.mjs:756, superdough.mjs:758 | implemented | biquad | implemented | - |
| bandq | superdough.mjs:759 | implemented | biquad | implemented | - |
| bank | superdough.mjs:494 | implemented | render | implemented | - |
| begin | - | ignored | - | - | - |
| binshift | - | ignored | - | - | - |
| bmod | superdough.mjs:1043, superdough.mjs:1044, superdough.mjs:1045 | implemented | modulate | implemented | - |
| bpattack | superdough.mjs:760 | implemented | biquad | implemented | - |
| bpdc | superdough.mjs:773 | implemented | biquad, ladder, lfo | implemented | - |
| bpdecay | superdough.mjs:761 | implemented | biquad | implemented | - |
| bpdepth | superdough.mjs:770 | implemented | biquad, ladder, lfo | implemented | - |
| bpdepthfrequency | superdough.mjs:771 | implemented | biquad, ladder, lfo | implemented | - |
| bpenv | superdough.mjs:764 | implemented | biquad | implemented | - |
| bprate | superdough.mjs:768 | implemented | biquad, ladder, lfo | implemented | - |
| bprelease | superdough.mjs:763 | implemented | biquad | implemented | - |
| bpshape | superdough.mjs:772 | implemented | biquad, ladder, lfo | implemented | - |
| bpskew | superdough.mjs:774 | implemented | biquad, ladder, lfo | implemented | - |
| bpsustain | superdough.mjs:762 | implemented | biquad | implemented | - |
| bpsync | superdough.mjs:769 | implemented | biquad, ladder, lfo | implemented | - |
| bus | superdough.mjs:509, superdough.mjs:980 | implemented | bus | implemented | - |
| busgain | superdough.mjs:510 | implemented | bus | implemented | - |
| byteBeatExpression | synth.mjs:243 | ignored | - | - | - |
| byteBeatStartTime | synth.mjs:243 | ignored | - | - | - |
| ccn | - | ignored | - | - | - |
| ccv | - | ignored | - | - | - |
| channel | - | ignored | - | - | - |
| channels | superdough.mjs:532 | implemented | orbit | implemented | - |
| chord | - | ignored | - | - | - |
| chorus | - | ignored | - | - | - |
| clip | synth.mjs:87 | implemented | render | implemented | - |
| coarse | superdough.mjs:799, superdough.mjs:800 | implemented | shaper | implemented | - |
| color | - | ignored | - | - | - |
| colour | - | ignored | - | - | - |
| comb | - | ignored | - | - | - |
| compressor | superdough.mjs:853, superdough.mjs:856 | implemented | compressor | implemented | - |
| compressorAttack | superdough.mjs:859 | implemented | compressor | implemented | - |
| compressorKnee | superdough.mjs:858 | implemented | compressor | implemented | - |
| compressorRatio | superdough.mjs:857 | implemented | compressor | implemented | - |
| compressorRelease | superdough.mjs:860 | implemented | compressor | implemented | - |
| cps | - | ignored | - | - | - |
| crush | superdough.mjs:804, superdough.mjs:805 | implemented | shaper | implemented | - |
| ctlNum | - | ignored | - | - | - |
| ctranspose | - | ignored | - | - | - |
| curve | - | ignored | - | - | - |
| cut | - | ignored | - | - | - |
| cutoff | superdough.mjs:684, superdough.mjs:686 | implemented | biquad | implemented | - |
| decay | synth.mjs:48, synth.mjs:87, synth.mjs:162, synth.mjs:249, synth.mjs:318, synth.mjs:379, synth.mjs:414 | implemented | adsr | implemented | - |
| degree | - | ignored | - | - | - |
| delay | superdough.mjs:504, superdough.mjs:624, superdough.mjs:889 | implemented | delay, chain | implemented | - |
| delayfeedback | superdough.mjs:505, superdough.mjs:625, superdough.mjs:889 | implemented | delay, chain | implemented | - |
| delaysync | superdough.mjs:506, superdough.mjs:626, superdough.mjs:636 | implemented | delay, chain | implemented | - |
| delaytime | superdough.mjs:507, superdough.mjs:627, superdough.mjs:636, superdough.mjs:889 | implemented | delay, chain | implemented | - |
| deltaSlide | - | ignored | - | - | - |
| density | synth.mjs:421 | implemented | noise | implemented | - |
| detune | synth.mjs:157 | implemented | supersaw, pulse | implemented | - |
| dictionary | - | ignored | - | - | - |
| distort | superdough.mjs:814, superdough.mjs:815 | implemented | shaper | implemented | - |
| distorttype | superdough.mjs:620 | implemented | shaper | implemented | - |
| distortvol | superdough.mjs:621 | implemented | shaper | implemented | - |
| djf | superdough.mjs:501 | implemented | orbit, djf | implemented | - |
| drive | superdough.mjs:695, superdough.mjs:731, superdough.mjs:767 | implemented | ladder | implemented | - |
| dry | superdough.mjs:503, superdough.mjs:894, superdough.mjs:929 | implemented | voice, chain | implemented | - |
| duckattack | superdough.mjs:499 | implemented | orbit | implemented | - |
| duckdepth | superdough.mjs:500 | implemented | orbit | implemented | - |
| duckonset | superdough.mjs:498 | implemented | orbit | implemented | - |
| duckorbit | superdough.mjs:497 | implemented | orbit | implemented | - |
| duration | helpers.mjs:471, synth.mjs:63, synth.mjs:87, synth.mjs:157, synth.mjs:247, synth.mjs:314, synth.mjs:383, synth.mjs:429, synth.mjs:506 | implemented | render | implemented | - |
| end | - | ignored | - | - | - |
| enhance | - | ignored | - | - | - |
| env | superdough.mjs:1027, superdough.mjs:1028, superdough.mjs:1029 | implemented | modulate | implemented | - |
| expression | - | ignored | - | - | - |
| fadeInTime | - | ignored | - | - | - |
| fadeTime | - | ignored | - | - | - |
| fanchor | superdough.mjs:693, superdough.mjs:729, superdough.mjs:765 | implemented | biquad | implemented | - |
| fft | superdough.mjs:521 | ignored | - | - | - |
| fit | - | ignored | - | - | - |
| fmattack | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack2 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack3 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack4 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack5 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack6 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack7 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmattack8 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay2 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay3 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay4 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay5 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay6 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay7 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmdecay8 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmenv | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv2 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv3 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv4 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv5 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv6 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv7 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmenv8 | helpers.mjs:472 | implemented | voice | implemented | - |
| fmh | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh2 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh3 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh4 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh5 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh6 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh7 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmh8 | helpers.mjs:460 | implemented | voice | implemented | - |
| fmi | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi00 | - | ignored | - | - | - |
| fmi01 | - | ignored | - | - | - |
| fmi02 | - | ignored | - | - | - |
| fmi03 | - | ignored | - | - | - |
| fmi04 | - | ignored | - | - | - |
| fmi05 | - | ignored | - | - | - |
| fmi06 | - | ignored | - | - | - |
| fmi07 | - | ignored | - | - | - |
| fmi08 | - | ignored | - | - | - |
| fmi10 | - | ignored | - | - | - |
| fmi11 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi12 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi13 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi14 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi15 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi16 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi17 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi18 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi2 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi20 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi21 | - | ignored | - | - | - |
| fmi22 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi23 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi24 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi25 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi26 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi27 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi28 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi3 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi30 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi31 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi32 | - | ignored | - | - | - |
| fmi33 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi34 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi35 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi36 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi37 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi38 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi4 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi40 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi41 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi42 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi43 | - | ignored | - | - | - |
| fmi44 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi45 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi46 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi47 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi48 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi5 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi50 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi51 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi52 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi53 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi54 | - | ignored | - | - | - |
| fmi55 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi56 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi57 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi58 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi6 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi60 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi61 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi62 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi63 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi64 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi65 | - | ignored | - | - | - |
| fmi66 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi67 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi68 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi7 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi70 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi71 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi72 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi73 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi74 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi75 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi76 | - | ignored | - | - | - |
| fmi77 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi78 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi8 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi80 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi81 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi82 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi83 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi84 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi85 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi86 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmi87 | - | ignored | - | - | - |
| fmi88 | helpers.mjs:445 | implemented | voice | implemented | - |
| fmrelease | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease2 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease3 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease4 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease5 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease6 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease7 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmrelease8 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain2 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain3 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain4 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain5 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain6 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain7 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmsustain8 | helpers.mjs:466 | implemented | voice | implemented | - |
| fmwave | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave2 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave3 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave4 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave5 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave6 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave7 | helpers.mjs:461 | implemented | voice | implemented | - |
| fmwave8 | helpers.mjs:461 | implemented | voice | implemented | - |
| frameRate | - | ignored | - | - | - |
| frames | - | ignored | - | - | - |
| freeze | - | ignored | - | - | - |
| freq | helpers.mjs:611 | implemented | voice | implemented | - |
| fshift | - | ignored | - | - | - |
| fshiftnote | - | ignored | - | - | - |
| fshiftphase | - | ignored | - | - | - |
| ftype | superdough.mjs:681, superdough.mjs:694, superdough.mjs:730, superdough.mjs:766 | implemented | biquad, ladder | implemented | - |
| gain | superdough.mjs:617 | implemented | voice | implemented | - |
| gate | - | ignored | - | - | - |
| harmonic | - | ignored | - | - | - |
| hbrick | - | ignored | - | - | - |
| hcutoff | superdough.mjs:720, superdough.mjs:722 | implemented | biquad | implemented | - |
| hold | - | ignored | - | - | - |
| hours | - | ignored | - | - | - |
| hpattack | superdough.mjs:724 | implemented | biquad | implemented | - |
| hpdc | superdough.mjs:737 | implemented | biquad, ladder, lfo | implemented | - |
| hpdecay | superdough.mjs:725 | implemented | biquad | implemented | - |
| hpdepth | superdough.mjs:734 | implemented | biquad, ladder, lfo | implemented | - |
| hpdepthfrequency | superdough.mjs:735 | implemented | biquad, ladder, lfo | implemented | - |
| hpenv | superdough.mjs:728 | implemented | biquad | implemented | - |
| hprate | superdough.mjs:732 | implemented | biquad, ladder, lfo | implemented | - |
| hprelease | superdough.mjs:727 | implemented | biquad | implemented | - |
| hpshape | superdough.mjs:736 | implemented | biquad, ladder, lfo | implemented | - |
| hpskew | superdough.mjs:738 | implemented | biquad, ladder, lfo | implemented | - |
| hpsustain | superdough.mjs:726 | implemented | biquad | implemented | - |
| hpsync | superdough.mjs:733 | implemented | biquad, ladder, lfo | implemented | - |
| hresonance | superdough.mjs:723 | implemented | biquad | implemented | - |
| i | superdough.mjs:519, superdough.mjs:628, superdough.mjs:912 | ignored | - | - | - |
| imag | - | ignored | - | - | - |
| ir | superdough.mjs:516, superdough.mjs:908, superdough.mjs:910, superdough.mjs:916 | unsupported | - | - | #18 |
| irbegin | superdough.mjs:518, superdough.mjs:926 | unsupported | - | - | #18 |
| irspeed | superdough.mjs:517, superdough.mjs:925 | unsupported | - | - | #18 |
| kcutoff | - | ignored | - | - | - |
| krush | - | ignored | - | - | - |
| label | - | ignored | - | - | - |
| lbrick | - | ignored | - | - | - |
| leslie | - | ignored | - | - | - |
| lfo | superdough.mjs:1009, superdough.mjs:1010, superdough.mjs:1011 | implemented | modulate | implemented | - |
| lock | - | ignored | - | - | - |
| loop | - | ignored | - | - | - |
| loopBegin | - | ignored | - | - | - |
| loopEnd | - | ignored | - | - | - |
| lpattack | superdough.mjs:688 | implemented | biquad | implemented | - |
| lpdc | superdough.mjs:701 | implemented | biquad, ladder, lfo | implemented | - |
| lpdecay | superdough.mjs:689 | implemented | biquad | implemented | - |
| lpdepth | superdough.mjs:698 | implemented | biquad, ladder, lfo | implemented | - |
| lpdepthfrequency | superdough.mjs:699 | implemented | biquad, ladder, lfo | implemented | - |
| lpenv | superdough.mjs:692 | implemented | biquad | implemented | - |
| lprate | superdough.mjs:696 | implemented | biquad, ladder, lfo | implemented | - |
| lprelease | superdough.mjs:691 | implemented | biquad | implemented | - |
| lpshape | superdough.mjs:700 | implemented | biquad, ladder, lfo | implemented | - |
| lpskew | superdough.mjs:702 | implemented | biquad, ladder, lfo | implemented | - |
| lpsustain | superdough.mjs:690 | implemented | biquad | implemented | - |
| lpsync | superdough.mjs:697 | implemented | biquad, ladder, lfo | implemented | - |
| lrate | - | ignored | - | - | - |
| lsize | - | ignored | - | - | - |
| markcss | - | ignored | - | - | - |
| midibend | - | ignored | - | - | - |
| midichan | - | ignored | - | - | - |
| midicmd | - | ignored | - | - | - |
| midimap | - | ignored | - | - | - |
| midiport | - | ignored | - | - | - |
| miditouch | - | ignored | - | - | - |
| minutes | - | ignored | - | - | - |
| mode | - | ignored | - | - | - |
| mtranspose | - | ignored | - | - | - |
| n | synth.mjs:157, synth.mjs:241, synth.mjs:385, synth.mjs:507 | implemented | periodic, bus | implemented | - |
| noise | synth.mjs:506 | implemented | noise | implemented | - |
| note | helpers.mjs:611 | implemented | voice | implemented | - |
| nrpnn | - | ignored | - | - | - |
| nrpv | - | ignored | - | - | - |
| nudge | - | ignored | - | - | - |
| octave | helpers.mjs:611 | implemented | voice | implemented | - |
| octaveR | - | ignored | - | - | - |
| octaves | - | ignored | - | - | - |
| octer | - | ignored | - | - | - |
| octersub | - | ignored | - | - | - |
| octersubsub | - | ignored | - | - | - |
| offset | - | ignored | - | - | - |
| orbit | superdough.mjs:508 | implemented | orbit | implemented | - |
| oschost | - | ignored | - | - | - |
| oscport | - | ignored | - | - | - |
| overgain | - | ignored | - | - | - |
| overshape | - | ignored | - | - | - |
| pan | superdough.mjs:867, superdough.mjs:870 | implemented | panner | implemented | - |
| panchor | helpers.mjs:354 | implemented | osc | implemented | - |
| panorient | - | ignored | - | - | - |
| panspan | - | ignored | - | - | - |
| pansplay | - | ignored | - | - | - |
| panwidth | - | ignored | - | - | - |
| partials | synth.mjs:507 | implemented | periodic | implemented | - |
| pattack | helpers.mjs:343, helpers.mjs:350 | implemented | osc | implemented | - |
| pcurve | helpers.mjs:348 | implemented | osc | implemented | - |
| pdecay | helpers.mjs:343, helpers.mjs:350, synth.mjs:87 | implemented | osc | implemented | - |
| penv | helpers.mjs:343, helpers.mjs:347, synth.mjs:87 | implemented | osc | implemented | - |
| phasercenter | superdough.mjs:880 | implemented | chain, biquad, lfo | implemented | - |
| phaserdepth | superdough.mjs:623 | implemented | chain, biquad, lfo | implemented | - |
| phaserrate | superdough.mjs:874, superdough.mjs:878 | implemented | chain, biquad, lfo | implemented | - |
| phasersweep | superdough.mjs:881 | implemented | chain, biquad, lfo | implemented | - |
| phases | synth.mjs:532 | implemented | periodic | implemented | - |
| pitchJump | - | ignored | - | - | - |
| pitchJumpTime | - | ignored | - | - | - |
| polyTouch | - | ignored | - | - | - |
| postgain | superdough.mjs:496 | implemented | voice | implemented | - |
| prelease | helpers.mjs:343, helpers.mjs:350 | implemented | osc | implemented | - |
| progNum | - | ignored | - | - | - |
| psustain | helpers.mjs:343, helpers.mjs:350 | implemented | osc | implemented | - |
| pw | synth.mjs:314 | implemented | pulse | implemented | - |
| pwrate | synth.mjs:301 | implemented | pulse, lfo | implemented | - |
| pwsweep | synth.mjs:301 | implemented | pulse, lfo | implemented | - |
| real | - | ignored | - | - | - |
| release | superdough.mjs:502, synth.mjs:48, synth.mjs:162, synth.mjs:249, synth.mjs:318, synth.mjs:379, synth.mjs:414 | implemented | adsr | implemented | - |
| resonance | superdough.mjs:687 | implemented | biquad | implemented | - |
| ring | - | ignored | - | - | - |
| ringdf | - | ignored | - | - | - |
| ringf | - | ignored | - | - | - |
| room | superdough.mjs:511, superdough.mjs:906, superdough.mjs:928 | implemented | reverb, chain | implemented | - |
| roomdim | superdough.mjs:514, superdough.mjs:923 | implemented | reverb, chain | implemented | - |
| roomfade | superdough.mjs:512, superdough.mjs:921 | implemented | reverb, chain | implemented | - |
| roomlp | superdough.mjs:513, superdough.mjs:922 | implemented | reverb, chain | implemented | - |
| roomsize | superdough.mjs:515, superdough.mjs:920 | implemented | reverb, chain | implemented | - |
| s | superdough.mjs:493 | implemented | voice | implemented | - |
| scram | - | ignored | - | - | - |
| seconds | - | ignored | - | - | - |
| semitone | - | ignored | - | - | - |
| shape | superdough.mjs:809, superdough.mjs:810 | implemented | shaper | implemented | - |
| shapevol | superdough.mjs:619 | implemented | shaper | implemented | - |
| size | - | ignored | - | - | - |
| slice | - | ignored | - | - | - |
| slide | - | ignored | - | - | - |
| smear | - | ignored | - | - | - |
| songPtr | - | ignored | - | - | - |
| source | superdough.mjs:495 | unsupported | - | - | #55 |
| speed | - | ignored | - | - | - |
| splice | - | ignored | - | - | - |
| spread | synth.mjs:157 | implemented | supersaw | implemented | - |
| squiz | - | ignored | - | - | - |
| stepsPerOctave | - | ignored | - | - | - |
| stretch | superdough.mjs:469, superdough.mjs:651, superdough.mjs:652 | unsupported | - | - | #18 |
| sustain | synth.mjs:48, synth.mjs:162, synth.mjs:249, synth.mjs:318, synth.mjs:379, synth.mjs:414 | implemented | adsr | implemented | - |
| sustainpedal | - | ignored | - | - | - |
| sysexdata | - | ignored | - | - | - |
| sysexid | - | ignored | - | - | - |
| transient | superdough.mjs:657, superdough.mjs:664 | implemented | transient | implemented | - |
| transsustain | superdough.mjs:665 | implemented | transient | implemented | - |
| tremolo | superdough.mjs:820 | implemented | chain, lfo | implemented | - |
| tremolodepth | superdough.mjs:622 | implemented | chain, lfo | implemented | - |
| tremolophase | superdough.mjs:839 | implemented | chain, lfo | implemented | - |
| tremoloshape | superdough.mjs:833, superdough.mjs:838 | implemented | chain, lfo | implemented | - |
| tremoloskew | superdough.mjs:833 | implemented | chain, lfo | implemented | - |
| tremolosync | superdough.mjs:821, superdough.mjs:822 | implemented | chain, lfo | implemented | - |
| triode | - | ignored | - | - | - |
| tsdelay | - | ignored | - | - | - |
| uid | - | ignored | - | - | - |
| unison | synth.mjs:157 | implemented | supersaw | implemented | - |
| unit | - | ignored | - | - | - |
| val | - | ignored | - | - | - |
| velocity | superdough.mjs:618 | implemented | voice | implemented | - |
| vib | helpers.mjs:362 | implemented | osc | implemented | - |
| vibmod | helpers.mjs:362 | implemented | osc | implemented | - |
| voice | - | ignored | - | - | - |
| vowel | superdough.mjs:792, superdough.mjs:793 | implemented | vowel | implemented | - |
| warp | - | ignored | - | - | - |
| warpattack | - | ignored | - | - | - |
| warpdc | - | ignored | - | - | - |
| warpdecay | - | ignored | - | - | - |
| warpdepth | - | ignored | - | - | - |
| warpenv | - | ignored | - | - | - |
| warpmode | - | ignored | - | - | - |
| warprate | - | ignored | - | - | - |
| warprelease | - | ignored | - | - | - |
| warpshape | - | ignored | - | - | - |
| warpskew | - | ignored | - | - | - |
| warpsustain | - | ignored | - | - | - |
| warpsync | - | ignored | - | - | - |
| waveloss | - | ignored | - | - | - |
| workletInputs | superdough.mjs:643 | unsupported | - | - | #55 |
| workletSrc | superdough.mjs:639, superdough.mjs:642 | unsupported | - | - | #55 |
| wt | - | ignored | - | - | - |
| wtattack | - | ignored | - | - | - |
| wtdc | - | ignored | - | - | - |
| wtdecay | - | ignored | - | - | - |
| wtdepth | - | ignored | - | - | - |
| wtenv | - | ignored | - | - | - |
| wtphaserand | - | ignored | - | - | - |
| wtrate | - | ignored | - | - | - |
| wtrelease | - | ignored | - | - | - |
| wtshape | - | ignored | - | - | - |
| wtskew | - | ignored | - | - | - |
| wtsustain | - | ignored | - | - | - |
| wtsync | - | ignored | - | - | - |
| xsdelay | - | ignored | - | - | - |
| zcrush | - | ignored | - | - | - |
| zdelay | - | ignored | - | - | - |
| zmod | - | ignored | - | - | - |
| znoise | - | ignored | - | - | - |
| zrand | - | ignored | - | - | - |
| zzfx | - | ignored | - | - | - |

## Sounds the engine refuses

| sound | owner |
|---|---|
| bytebeat | #55 |
| sbd | #56 |
| wt_* | #18 |
| z_* | #53 |
| zzfx | #53 |
| (samples and soundfonts: any other name) | #18 |

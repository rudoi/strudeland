//! The voice's stage chain: the stages of one FX entry built in superdough's order
//! (superdough.mjs:612-1004), the [`VoiceBus`] a quantum of the voice's signal travels on between
//! them (one or two channels and the `AudioHandler` silence hint), and the per-voice
//! [`NodeTable`], superdough's `nodes` tracker (`nodes[key][name]`).
//!
//! Every stage is processed in place on the bus and fixes its output channel count from its
//! input count at build ([`Stage::channels_out`]): `Gain`, `Biquad` (one kernel per channel), the
//! vowel, the phaser's notch, the release gain and the worklets (`Ladder`, `Shaper`, `Transient`)
//! keep it, the compressor and the panner always output two channels (the panner's mono law for a
//! mono input, the stereo law for a stereo one). A live worklet never outputs a silent bus (its
//! tail time is infinite); a finished one outputs a silent zero bus.
//!
//! Param connections reach a stage through its [`StageInputs`] (one [`ParamInputs`] per
//! [`ParamSlot`]): the filter LFOs ([`ParamLfo`], `createFilter`) into a biquad's or a ladder's
//! `frequency`, the phaser's LFO (`getPhaser`) into its notch's `detune`; the tremolo's LFO is
//! inside its stage ([`Tremolo`]).
//!
//! [`Teardown`] is the teardown table: from which quantum each stage's input has no rendering
//! connection once the voice's source stops (Chromium disables the outputs of a finished source
//! and cascades it through the nodes after it), which the ladder and the transient act on.
//!
//! FX chains (`value.FX`, superdough.mjs:611-938): every entry of `FX = [...value.FX, value]` gets
//! its own stage list in the voice's flat list ([`build_stage`] per planned [`StageSpec`], grouped
//! by [`FxKey`], the main entry last), and a chain entry (not the main one) ends with its local
//! delay ([`LocalDelay`]) and local reverb ([`LocalReverb`]): `tail -> dry gain(dry ?? 1)` and
//! `tail -> effect -> wet gain(level)`, both summed into a 2-channel `sum` (the voice is stereo
//! from there). Their tails render the quantum holding `endWithRelease` and are cut from the next
//! one (with `FXrelease > release` the release gain is 0 there, so the cut is exact).

use crate::QUANTUM;
use crate::biquad::{Biquad, BiquadInputs, FilterKind};
use crate::coerce::{js_max, js_min, js_to_number, js_truthy};
use crate::compressor::Compressor;
use crate::delay::{FeedbackDelay, MAX_FEEDBACK};
use crate::job::Value;
use crate::jsmath;
use crate::ladder::{Ladder, LadderNode};
use crate::lfo::{self, Lfo, LfoProps, ShapeInput, get_lfo};
use crate::orbit::{Input, sum_stereo};
use crate::panner::{Stereo, StereoPanner};
use crate::param::{Param, ParamInputs};
use crate::params::{EntryParams, FilterParams, VoiceParams, gain_stage};
use crate::reverb::Reverb;
use crate::shaper::{Coarse, Crush, Distort, Shape, Shaper, ShaperKind};
use crate::transient::{Transient, TransientNode};
use crate::voice::{GainNode, VoiceError, create_filter, filter_envelope, hap_error, idl_float};
use crate::vowel::Vowel;
use crate::worklet::{ProcessorError, first_absent_after_stop, first_quantum_reaching};

/// One quantum of a voice's signal between stages. 1 KiB on the stack; never heap allocated.
#[derive(Debug, Clone)]
pub struct VoiceBus {
    /// Only `ch[..channels]` is meaningful; `ch[1]` is stale when `channels == 1`.
    pub ch: [[f32; QUANTUM]; 2],
    /// 1 or 2, never anything else.
    pub channels: usize,
    /// `AudioHandler`'s silence hint for the whole bus.
    pub silent: bool,
}

impl VoiceBus {
    /// A zeroed, silent mono bus.
    pub fn mono() -> VoiceBus {
        VoiceBus {
            ch: [[0.0; QUANTUM]; 2],
            channels: 1,
            silent: true,
        }
    }

    /// The meaningful channels, `ch[..channels]`.
    pub fn active(&mut self) -> &mut [[f32; QUANTUM]] {
        &mut self.ch[..self.channels]
    }

    /// The bus through the gain node `g` (`inputs` the connections into its `gain`), in place.
    pub fn apply_gain(&mut self, q: u64, g: &mut GainNode, inputs: &ParamInputs) {
        let silent = self.silent;
        self.silent = g.process(q, self.active(), silent, inputs);
    }

    /// A mono bus becomes stereo by copying its channel into both (the speakers up-mix of a
    /// 2-channel explicit input); the silence hint is unchanged. A stereo bus is left as is.
    pub fn upmix_to_stereo(&mut self) {
        if self.channels == 1 {
            self.ch[1] = self.ch[0];
            self.channels = 2;
        }
    }
}

/// Index of a stage in the voice's flat `Vec<Stage>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageId(pub usize);

/// Which AudioParam of a node; the numbering is per node kind, the order of the names
/// `modulate::node_param` finds (`Gain { GAIN = 0 }`, `Biquad { FREQUENCY = 0, Q = 1, GAIN = 2,
/// DETUNE = 3 }`, `Ladder { FREQUENCY = 0, Q = 1, DRIVE = 2 }`, `Panner { PAN = 0 }`, `Tremolo {
/// GAIN = 0 }`: its AM gain, a shaper `{ <its own> = 0, POSTGAIN = 1 }`, `Compressor { THRESHOLD
/// = 0, RATIO = 1, KNEE = 2, ATTACK = 3, RELEASE = 4 }`; an `lfo` or `env` node its descriptor
/// order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSlot(pub u8);

impl ParamSlot {
    /// `GainNode.gain`.
    pub const GAIN_GAIN: ParamSlot = ParamSlot(0);
    pub const BIQUAD_FREQUENCY: ParamSlot = ParamSlot(0);
    pub const BIQUAD_Q: ParamSlot = ParamSlot(1);
    pub const BIQUAD_GAIN: ParamSlot = ParamSlot(2);
    pub const BIQUAD_DETUNE: ParamSlot = ParamSlot(3);
    pub const LADDER_FREQUENCY: ParamSlot = ParamSlot(0);
    pub const LADDER_Q: ParamSlot = ParamSlot(1);
    pub const LADDER_DRIVE: ParamSlot = ParamSlot(2);
    /// `StereoPannerNode.pan`.
    pub const PANNER_PAN: ParamSlot = ParamSlot(0);
    /// A shaper worklet's own param (`coarse`, `crush`, `shape` or `distort`).
    pub const SHAPER_AMOUNT: ParamSlot = ParamSlot(0);
    pub const SHAPER_POSTGAIN: ParamSlot = ParamSlot(1);
    /// `DynamicsCompressorNode`'s params, in slot order.
    pub const COMPRESSOR: [ParamSlot; 5] = [
        ParamSlot(0), // threshold
        ParamSlot(1), // ratio
        ParamSlot(2), // knee
        ParamSlot(3), // attack
        ParamSlot(4), // release
    ];
    /// A local delay's `delayTime` and `feedback`.
    pub const DELAY_TIME: ParamSlot = ParamSlot(0);
    pub const DELAY_FEEDBACK: ParamSlot = ParamSlot(1);
}

/// The most params a stage exposes to connections (a compressor's five).
pub const MAX_PARAM_SLOTS: usize = 5;

/// What is connected into one stage for one quantum: whether its audio input has a rendering
/// connection ([`Teardown`]; only the ladder and the transient act on it) and the connection
/// inputs of its params, indexed by [`ParamSlot`].
#[derive(Debug, Clone)]
pub struct StageInputs {
    pub input_connected: bool,
    params: [ParamInputs; MAX_PARAM_SLOTS],
    /// The connections into the params of the tremolo's own `lfo` (modulators).
    pub lfo: lfo::LfoInputs,
    /// The connections into each vowel formant's `frequency` (modulators).
    pub formants: [ParamInputs; 5],
    /// The connections into a local delay's or reverb's wet gain (`delay_mix`, `room_mix`).
    pub wet: ParamInputs,
}

impl StageInputs {
    /// The audio input connected, no param connection.
    pub const NONE: StageInputs = StageInputs {
        input_connected: true,
        params: [ParamInputs::NONE; MAX_PARAM_SLOTS],
        lfo: lfo::NO_INPUTS,
        formants: [ParamInputs::NONE; 5],
        wet: ParamInputs::NONE,
    };

    /// The audio input connected or not, no param connection.
    pub fn input(connected: bool) -> StageInputs {
        StageInputs {
            input_connected: connected,
            ..Self::NONE
        }
    }

    /// The connections into the param at `slot`, for adding more (`None` past
    /// [`MAX_PARAM_SLOTS`]).
    pub fn param_mut(&mut self, slot: ParamSlot) -> Option<&mut ParamInputs> {
        self.params.get_mut(usize::from(slot.0))
    }

    /// The connections into the param at `slot` ([`ParamInputs::NONE`] when there are none).
    pub fn of(&self, slot: ParamSlot) -> &ParamInputs {
        static NONE: ParamInputs = ParamInputs::NONE;
        self.params.get(usize::from(slot.0)).unwrap_or(&NONE)
    }

    /// Connects one more node into the param at `slot`: the param is connected from now on, and
    /// `buf` (the node's output this quantum, `None` when it is silent) is summed after the
    /// earlier connections. A slot past [`MAX_PARAM_SLOTS`] is ignored.
    pub fn connect(&mut self, slot: ParamSlot, buf: Option<[f32; QUANTUM]>) {
        if let Some(p) = self.params.get_mut(usize::from(slot.0)) {
            p.connected = true;
            p.bufs.extend(buf);
        }
    }
}

/// The tremolo (superdough.mjs:820-851): `amGain = new GainNode(ac, { gain: max(1 - depth, 0)
/// })` with an `lfo` connected to its gain.
#[derive(Debug, Clone)]
pub struct Tremolo {
    pub lfo: Lfo,
    pub gain: GainNode,
    /// The gain's connections of the current quantum, kept to reuse its allocation.
    inputs: ParamInputs,
}

impl Tremolo {
    /// One quantum in place: the LFO first (the AM gain pulls it, also while its input is
    /// silent; `lfo` the modulators of its params), then the gain with the LFO's output as its
    /// first connection and the modulators of the gain (`gain`) after it. Returns the silence
    /// hint.
    fn process(
        &mut self,
        q: u64,
        channels: &mut [[f32; QUANTUM]],
        input_silent: bool,
        lfo: &lfo::LfoInputs,
        gain: &ParamInputs,
    ) -> bool {
        let mut buf = [0.0f32; QUANTUM];
        let lfo_silent = self.lfo.render(q, lfo, &mut buf);
        let inputs = &mut self.inputs;
        inputs.connected = true;
        inputs.bufs.clear();
        if !lfo_silent {
            inputs.bufs.push(buf);
        }
        inputs.bufs.extend_from_slice(&gain.bufs);
        self.gain.process(q, channels, input_silent, inputs)
    }
}

/// A filter LFO (`createFilter`'s `getParamLfo`): an `lfo` connected to the param at `slot` of
/// the stage `stage` (a biquad's or a ladder's `frequency`), rendered before that stage.
#[derive(Debug, Clone)]
pub struct ParamLfo {
    pub stage: StageId,
    pub slot: ParamSlot,
    pub lfo: Lfo,
}

/// One stage of a voice's chain.
#[derive(Debug, Clone)]
pub enum Stage {
    /// A `GainNode` (`Gain(0.3)`, `envGain`, the gain stage, `post`).
    Gain(GainNode),
    /// A `BiquadFilterNode`, one kernel per input channel.
    Biquad(Biquad),
    /// The ladder filter worklet (`ftype('ladder')`).
    Ladder(Box<LadderNode>),
    /// A `StereoPannerNode`: one or two channels in, two out.
    Panner(StereoPanner),
    /// A `coarse`, `crush`, `shape` or `distort` worklet.
    Shaper(Shaper),
    /// The `transient` worklet.
    Transient(Box<TransientNode>),
    /// The tremolo's AM gain and its LFO.
    Tremolo(Box<Tremolo>),
    /// The `vowel` filter: five bandpass formants summed into a makeup gain.
    Vowel(Box<Vowel>),
    /// A `DynamicsCompressorNode`: one or two channels in, two out.
    Compressor(Box<Compressor>),
    /// The phaser's notch `BiquadFilterNode`; its LFO is a [`ParamLfo`] on `detune`.
    Phaser(Biquad),
    /// The `FXrelease` gain (`gainNode(1)` with its hold and linear fade).
    Release(GainNode),
    /// A chain entry's local feedback delay with its dry and wet gains and the stereo `sum`.
    LocalDelay(Box<LocalDelay>),
    /// A chain entry's local reverb with its dry and wet gains and the stereo `sum`.
    LocalReverb(Box<LocalReverb>),
}

impl Stage {
    /// Processes `bus` in place for the quantum starting at frame `q`: sets `bus.channels` to
    /// [`Stage::channels_out`] of its input count and `bus.silent` to the output's silence hint.
    /// `inputs.input_connected` reaches the ladder and the transient; the memoryless shapers
    /// process the zeros their input delivers either way (each maps 0 to 0). The param connections
    /// of `inputs` reach a gain's `gain`, a biquad's `frequency` and `detune` and a ladder's
    /// `frequency`.
    pub fn process(&mut self, q: u64, bus: &mut VoiceBus, inputs: &StageInputs) {
        match self {
            Stage::Gain(g) => bus.apply_gain(q, g, inputs.of(ParamSlot::GAIN_GAIN)),
            Stage::Biquad(f) | Stage::Phaser(f) => {
                let silent = bus.silent;
                let slot = |s| inputs.of(s);
                let (fr, de, qq, ga) = (
                    slot(ParamSlot::BIQUAD_FREQUENCY),
                    slot(ParamSlot::BIQUAD_DETUNE),
                    slot(ParamSlot::BIQUAD_Q),
                    slot(ParamSlot::BIQUAD_GAIN),
                );
                let bi = if fr.connected || de.connected || qq.connected || ga.connected {
                    BiquadInputs {
                        frequency: fr,
                        detune: de,
                        q: qq,
                        gain: ga,
                    }
                } else {
                    BiquadInputs::NONE
                };
                bus.silent = f.process(q, bus.active(), silent, &bi);
            }
            Stage::Tremolo(t) => {
                let silent = bus.silent;
                let gain = inputs.of(ParamSlot::GAIN_GAIN);
                bus.silent = t.process(q, bus.active(), silent, &inputs.lfo, gain);
            }
            Stage::Vowel(v) => v.process_with(q, bus, &inputs.formants),
            Stage::Compressor(c) => {
                c.process_with(q, bus, &ParamSlot::COMPRESSOR.map(|k| inputs.of(k)));
            }
            Stage::Release(g) => bus.apply_gain(q, g, &ParamInputs::NONE),
            Stage::Panner(p) => {
                let input = bus.ch;
                let mut o: Stereo = [[0.0; QUANTUM]; 2];
                let pan = inputs.of(ParamSlot::PANNER_PAN);
                bus.silent = p.process_with(q, &input[..bus.channels], bus.silent, &mut o, pan);
                bus.ch = o;
                bus.channels = 2;
            }
            Stage::Ladder(l) => {
                bus.silent = l.process_with(
                    q,
                    bus.active(),
                    inputs.input_connected,
                    [
                        inputs.of(ParamSlot::LADDER_FREQUENCY),
                        inputs.of(ParamSlot::LADDER_Q),
                        inputs.of(ParamSlot::LADDER_DRIVE),
                    ],
                )
            }
            Stage::Shaper(s) => {
                let slots = [
                    inputs.of(ParamSlot::SHAPER_AMOUNT),
                    inputs.of(ParamSlot::SHAPER_POSTGAIN),
                ];
                bus.silent = s.process_with(q, bus.active(), slots);
            }
            Stage::Transient(t) => bus.silent = t.process(q, bus.active(), inputs.input_connected),
            Stage::LocalDelay(d) => d.process(q, bus, inputs),
            Stage::LocalReverb(r) => r.process(q, bus, inputs),
        }
    }

    /// The output channel count for an input of `channels_in` (1 or 2).
    pub fn channels_out(&self, channels_in: usize) -> usize {
        match self {
            Stage::Gain(_)
            | Stage::Biquad(_)
            | Stage::Ladder(_)
            | Stage::Shaper(_)
            | Stage::Transient(_)
            | Stage::Tremolo(_)
            | Stage::Vowel(_)
            | Stage::Phaser(_)
            | Stage::Release(_) => channels_in,
            Stage::Panner(_)
            | Stage::Compressor(_)
            | Stage::LocalDelay(_)
            | Stage::LocalReverb(_) => 2,
        }
    }

    /// The voice-lifetime clause of a stage with state that can sound without input,
    /// after it processed the quantum starting at `q`: the compressor once
    /// its pre-delay holds only zeros, the vowel once its five biquads propagate silence, a local
    /// delay or reverb only after its cut quantum; every other stage is done (a ringing biquad is
    /// caught by the voice's silence floor).
    pub fn is_done(&self, q: u64) -> bool {
        match self {
            Stage::Compressor(c) => c.is_done(),
            Stage::Vowel(v) => v.is_quiet_after(q),
            Stage::LocalDelay(d) => d.tail.is_cut(q),
            Stage::LocalReverb(r) => r.tail.is_cut(q),
            _ => true,
        }
    }

    /// `PropagatesSilence` at `q` of a tail-processing stage (a biquad, the phaser's notch, the
    /// vowel's biquads, the compressor); `None` for the others.
    pub fn propagates_silence(&self, q: u64) -> Option<bool> {
        match self {
            Stage::Biquad(f) | Stage::Phaser(f) => Some(f.propagates_silence(q)),
            Stage::Vowel(v) => Some(v.paths.iter().all(|p| p.filter.propagates_silence(q))),
            Stage::Compressor(c) => Some(c.propagates_silence(q)),
            _ => None,
        }
    }

    /// A processor error the stage met in its last quantum, reported once.
    pub fn take_error(&mut self) -> Option<ProcessorError> {
        match self {
            Stage::Shaper(s) => s.take_error(),
            Stage::Tremolo(t) => t.lfo.take_error(),
            _ => None,
        }
    }
}

/// Whether an input whose connection is gone from `first_absent` (`None`: never) is connected
/// at the quantum starting at `q`.
pub fn input_connected(first_absent: Option<u64>, q: u64) -> bool {
    first_absent.is_none_or(|d| q < d)
}

/// The source rows of the teardown table: what feeds the first stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceEnd {
    /// Scheduled sources (oscillator, noise, constant): the largest exclusive stop frame of the
    /// signal-path sources (`Schedule::end_frame`), `None` when no stop is scheduled. The
    /// source finishes in the quantum that holds the stop frame and the cascade runs after it.
    Scheduled(Option<u64>),
    /// A supersaw with its `end` param: the main-thread release at `end` cannot be
    /// earlier than one quantum after the quantum that holds `end`.
    Supersaw(f32),
    /// A pulse with its `end` param: it returns `false` at the first quantum whose start time
    /// reaches `end` (it has no input) and finishes; the cascade follows that quantum.
    Pulse(f32),
}

impl SourceEnd {
    /// The first quantum start from which the source's output has no rendering connection.
    pub fn disabled_from(self) -> Option<u64> {
        match self {
            SourceEnd::Scheduled(stop) => stop.map(first_absent_after_stop),
            SourceEnd::Supersaw(end) => Some(first_absent_after_stop(
                crate::render::frame_for_buffer_start(f64::from(end)),
            )),
            SourceEnd::Pulse(end) => {
                first_quantum_reaching(f64::from(end)).map(|q| q.saturating_add(QUANTUM as u64))
            }
        }
    }
}

/// The teardown table for one stage list: from which quantum start each stage's input has no
/// rendering connection. The walk is resolved quantum by quantum in chain order (equivalent to
/// the backward walk from each stage): every row but the tail-processing one is a function of the
/// row before it; a tail-processing stage (a biquad) disables its output at the first quantum
/// start `q >= before` at which it propagates silence (unverified against Chromium), which is
/// only known once rendered.
#[derive(Debug, Clone)]
pub struct Teardown {
    source: Option<u64>,
    /// Per stage, a tail-processing stage's resolved output-disabled frame.
    tails: Vec<Option<u64>>,
}

impl Teardown {
    /// The walk for a list of `stages` stages fed by `source`.
    pub fn new(source: SourceEnd, stages: usize) -> Teardown {
        Teardown {
            source: source.disabled_from(),
            tails: vec![None; stages],
        }
    }

    /// When the first stage's input loses its connection.
    pub fn source(&self) -> Option<u64> {
        self.source
    }

    /// Stage `k`'s output-disabled frame, given that its input is gone from `before`, after it
    /// processed the quantum starting at `q`: the frame from which stage `k + 1`'s input has no
    /// rendering connection.
    pub fn after(&mut self, k: usize, stage: &Stage, before: Option<u64>, q: u64) -> Option<u64> {
        use crate::QUANTUM_U64 as Q;
        match stage {
            // The cascade passes through a gain (the tremolo's and the release gain too; and,
            // assumed, the panner) in the same task.
            Stage::Gain(_) | Stage::Panner(_) | Stage::Tremolo(_) | Stage::Release(_) => before,
            // An effect worklet returns false in the first quantum its input is absent and
            // finishes; its output is disabled after that quantum.
            Stage::Shaper(_) | Stage::Ladder(_) => before.map(|b| b.saturating_add(Q)),
            // The transient returns false only from its end on, and finishes once it also has
            // no input.
            Stage::Transient(t) => {
                before.and_then(|b| t.end_quantum().map(|e| e.max(b).saturating_add(Q)))
            }
            // A local delay or reverb is cut after the quantum holding `endWithRelease`: its output keeps its connection until both its input is gone and the cut is
            // past (never without a cut).
            Stage::LocalDelay(d) => before.and_then(|b| d.tail.disabled_from(b)),
            Stage::LocalReverb(r) => before.and_then(|b| r.tail.disabled_from(b)),
            // The tail-processing nodes: a biquad (the phaser's notch), the vowel's biquads, the
            // compressor (its latency and tail).
            Stage::Biquad(_) | Stage::Phaser(_) | Stage::Vowel(_) | Stage::Compressor(_) => {
                let propagates = stage.propagates_silence(q) == Some(true);
                let slot = self.tails.get_mut(k)?;
                if slot.is_none()
                    && let Some(b) = before
                    && q >= b
                    && propagates
                {
                    *slot = Some(q);
                }
                *slot
            }
        }
    }
}

/// Which FX entry of a hap a node belongs to: the main chain, or entry `n` of `value.FX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FxKey {
    Main,
    Index(usize),
}

impl FxKey {
    /// The property name of the entry in superdough's `nodes` object (`main`, `0`, `1`, ...).
    pub fn text(self) -> String {
        match self {
            FxKey::Main => "main".into(),
            FxKey::Index(n) => n.to_string(),
        }
    }
}

/// A node of superdough's `nodes` tracker: a handle; the param is chosen by name when a
/// modulator resolves it (`modulate::node_param`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Node {
    /// A node of the voice's flat stage list (its kind is the plan's [`StageKind`]).
    Stage(StageId),
    /// The `Gain(amount)` of FM connection `n` (`Fm::connections`).
    FmAmount(usize),
    /// A node built but connected to nothing audible (the target-side `Gain(amt)` of an FM
    /// matrix entry); a modulator reads `value` as its `currentValue`.
    Detached { value: f32 },
    /// The voice's source node (its kind is the plan's [`SourceKind`]).
    Source,
    /// The vibrato oscillator (`vib`).
    Vibrato,
    /// The vibrato's `Gain(vibmod * 100)` (`vib_gain`).
    VibratoGain,
    /// The oscillator (or noise source) of FM modulator `n` (its position in `Fm::modulators`;
    /// `fm_<i>`).
    FmOsc(usize),
    /// The `lfo` built for stage `id`: a filter's LFO (`lpf_lfo`, `hpf_lfo`, `bpf_lfo`; a
    /// [`ParamLfo`]), the phaser's (`phaser_lfo`; a [`ParamLfo`]) or the tremolo's (`tremolo`).
    StageLfo(StageId),
    /// Formant filter `n` (0..5) of the vowel stage `id` (`vowel` holds the five biquads).
    VowelFilter(StageId, usize),
    /// The pulse source's width-modulation `lfo` (`pw_lfo`).
    PulseLfo,
    /// The `bus` sound's source: the bus, a `GainNode(1)` (`nodes.source = [bus]`).
    Bus,
    /// The orbit's feedback delay (`delay` of the main entry; orbit-held).
    OrbitDelay,
    /// The orbit's convolver (`room` of the main entry: no param a control reaches).
    OrbitReverb,
    /// The orbit's `djf` worklet (`djf`; orbit-held).
    OrbitDjf,
    /// The voice's `gainNode(delay)` send (`delay_mix` of the main entry).
    DelaySend,
    /// The voice's `gainNode(room)` send (`room_mix` of the main entry).
    RoomSend,
    /// The `lfo` modulator at this position of the voice's modulators (`lfo_<id>`).
    Lfo(usize),
    /// The `env` modulator at this position of the voice's modulators (`env_<id>`).
    Env(usize),
    /// The wet gain of the local delay or reverb stage `id` (`delay_mix`, `room_mix` of a chain
    /// entry; the stage itself is `delay`, `room`).
    StageWet(StageId),
}

/// What `nodes[key][name]` holds: an array whose elements can be `undefined`, or a bare node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeList {
    Array(Vec<Option<Node>>),
    NotArray(Node),
}

/// A resolved param: a node and which of its params (the slot numbering is per node kind, see
/// `modulate::node_param`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamRef {
    pub node: Node,
    pub slot: ParamSlot,
}

/// The sound a value without `s` plays (superdough's `s = 'triangle'` default).
pub const DEFAULT_SOUND: &str = "triangle";

/// What a voice's source is, as the node table sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A periodic-wave oscillator (every oscillator sound).
    Osc,
    /// `ConstantSourceNode(1)` (`one`; registered as a bare node).
    Constant,
    /// A looping noise buffer source.
    Noise,
    Supersaw,
    Pulse,
    /// The `bus` sound.
    Bus,
}

impl SourceKind {
    /// Every `s` the engine builds and its kind (`getOscillator`'s periodic-wave types and their
    /// aliases, `user`, the noises, `one`, `supersaw`, `pulse`, `bus`).
    pub const NAMES: &'static [(&'static str, SourceKind)] = &[
        ("one", SourceKind::Constant),
        ("supersaw", SourceKind::Supersaw),
        ("pulse", SourceKind::Pulse),
        ("bus", SourceKind::Bus),
        ("sine", SourceKind::Osc),
        ("sawtooth", SourceKind::Osc),
        ("square", SourceKind::Osc),
        ("triangle", SourceKind::Osc),
        ("sin", SourceKind::Osc),
        ("saw", SourceKind::Osc),
        ("sqr", SourceKind::Osc),
        ("tri", SourceKind::Osc),
        ("user", SourceKind::Osc),
        ("white", SourceKind::Noise),
        ("pink", SourceKind::Noise),
        ("brown", SourceKind::Noise),
        ("crackle", SourceKind::Noise),
    ];

    /// The kind of sound `s` (already defaulted to `triangle`), `None` for a sound the engine
    /// does not build (the build throws `sound .. not found`).
    pub fn of(s: &str) -> Option<SourceKind> {
        Self::NAMES
            .iter()
            .find(|(name, _)| *name == s)
            .map(|(_, kind)| *kind)
    }
}

/// The kind of a stage of the plan (what a [`Node::Stage`] exposes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Gain,
    Biquad,
    Ladder,
    Panner,
    Coarse,
    Crush,
    Shape,
    Distort,
    Transient,
    Tremolo,
    Vowel,
    Compressor,
    Phaser,
    Release,
    LocalDelay,
    LocalReverb,
}

impl StageKind {
    /// The kind of a built stage (what `Voice::build` checks each built stage against).
    pub fn of(stage: &Stage) -> StageKind {
        match stage {
            Stage::Gain(_) => StageKind::Gain,
            Stage::Biquad(_) => StageKind::Biquad,
            Stage::Ladder(_) => StageKind::Ladder,
            Stage::Panner(_) => StageKind::Panner,
            Stage::Shaper(s) => match &s.kind {
                ShaperKind::Coarse(_) => StageKind::Coarse,
                ShaperKind::Crush(_) => StageKind::Crush,
                ShaperKind::Shape(_) => StageKind::Shape,
                ShaperKind::Distort(_) => StageKind::Distort,
            },
            Stage::Transient(_) => StageKind::Transient,
            Stage::Tremolo(_) => StageKind::Tremolo,
            Stage::Vowel(_) => StageKind::Vowel,
            Stage::Compressor(_) => StageKind::Compressor,
            Stage::Phaser(_) => StageKind::Phaser,
            Stage::Release(_) => StageKind::Release,
            Stage::LocalDelay(_) => StageKind::LocalDelay,
            Stage::LocalReverb(_) => StageKind::LocalReverb,
        }
    }
}

/// One planned stage of a voice: what [`build_stage`] builds at this position of the chain.
/// Owned (keys and kinds only); the values are read from the `VoiceParams` at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSpec {
    /// `Gain(0.3)` after a synth source (not behind a worklet synth or the bus sound).
    SynthGain,
    /// `envGain` with the amp (or bus) envelope.
    EnvGain,
    Transient(FxKey),
    /// The entry's gain stage, `Gain(gain * velocity)`.
    EntryGain(FxKey),
    /// One filter copy: a biquad (two specs for a 24 dB filter) or a ladder.
    Filter {
        entry: FxKey,
        kind: FilterKind,
        ladder: bool,
    },
    Vowel(FxKey),
    Coarse(FxKey),
    Crush(FxKey),
    Shape(FxKey),
    Distort(FxKey),
    Tremolo(FxKey),
    Compressor(FxKey),
    Pan(FxKey),
    Phaser(FxKey),
    LocalDelay(FxKey),
    LocalReverb(FxKey),
    /// The `FXrelease` gain.
    Release,
    /// `post`, `Gain(postgain)`.
    Post,
}

impl StageSpec {
    /// The kind of stage this spec builds.
    pub fn kind(self) -> StageKind {
        match self {
            StageSpec::SynthGain
            | StageSpec::EnvGain
            | StageSpec::EntryGain(_)
            | StageSpec::Post => StageKind::Gain,
            StageSpec::Filter { ladder: true, .. } => StageKind::Ladder,
            StageSpec::Filter { ladder: false, .. } => StageKind::Biquad,
            StageSpec::Transient(_) => StageKind::Transient,
            StageSpec::Vowel(_) => StageKind::Vowel,
            StageSpec::Coarse(_) => StageKind::Coarse,
            StageSpec::Crush(_) => StageKind::Crush,
            StageSpec::Shape(_) => StageKind::Shape,
            StageSpec::Distort(_) => StageKind::Distort,
            StageSpec::Tremolo(_) => StageKind::Tremolo,
            StageSpec::Compressor(_) => StageKind::Compressor,
            StageSpec::Pan(_) => StageKind::Panner,
            StageSpec::Phaser(_) => StageKind::Phaser,
            StageSpec::LocalDelay(_) => StageKind::LocalDelay,
            StageSpec::LocalReverb(_) => StageKind::LocalReverb,
            StageSpec::Release => StageKind::Release,
        }
    }
}

/// One FM matrix side as planned: the modulator number (1..=8), its position in creation
/// order, and whether this side creates it (its first use in the walk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmSide {
    pub idx: usize,
    pub pos: usize,
    pub first_use: bool,
}

/// One entry of `applyFM`'s matrix walk as planned: the source side, then the target side
/// (`None` for the carrier), and whether the entry connects (its target is the carrier or a
/// modulator that is not noise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmStep {
    pub sides: [Option<FmSide>; 2],
    pub connects: bool,
}

/// `fm_<n>` for n in 1..=8 (`NodeName::Fm(n)`).
const FM_NAMES: [&str; 8] = [
    "fm_1", "fm_2", "fm_3", "fm_4", "fm_5", "fm_6", "fm_7", "fm_8",
];

/// `fm_<n>_gain` for n in 1..=8 (`NodeName::FmGain(n)`).
const FM_GAIN_NAMES: [&str; 8] = [
    "fm_1_gain",
    "fm_2_gain",
    "fm_3_gain",
    "fm_4_gain",
    "fm_5_gain",
    "fm_6_gain",
    "fm_7_gain",
    "fm_8_gain",
];

/// A key of superdough's `nodes` tracker that the plan or the modulator loop writes. The table
/// matches entries by value and looks them up by [`NodeName::as_str`], superdough's text; the two
/// agree because a `Mod` name (`lfo_<id>`/`env_<id>`) never spells a static or FM name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeName {
    /// `source`: the sound's source node (or the bus).
    Source,
    /// `pw_lfo`: the pulse's width LFO.
    PwLfo,
    /// `vib`: the vibrato oscillator.
    Vib,
    /// `vib_gain`: the vibrato depth.
    VibGain,
    /// `fm_<n>`, n in 1..=8: the FM modulator. `as_str` indexes by `n`, so build it with
    /// [`NodeName::fm`], which checks the range.
    Fm(u8),
    /// `fm_<n>_gain`, n in 1..=8: the FM amount. Build it with [`NodeName::fm_gain`].
    FmGain(u8),
    Transient,
    Gain,
    Lpf,
    LpfLfo,
    Hpf,
    HpfLfo,
    Bpf,
    BpfLfo,
    /// `vowel`: the five formant filters.
    Vowel,
    Coarse,
    Crush,
    Shape,
    Distort,
    Tremolo,
    TremoloGain,
    Compressor,
    Pan,
    Phaser,
    PhaserLfo,
    /// `delay`: the orbit delay on `Main`, the local delay on a chain entry.
    Delay,
    /// `delay_mix`: the delay send on `Main`, the local delay's wet gain on a chain entry.
    DelayMix,
    /// `room`: the orbit reverb on `Main`, the local reverb on a chain entry.
    Room,
    /// `room_mix`: the reverb send on `Main`, the local reverb's wet gain on a chain entry.
    RoomMix,
    /// `post`: the `postgain` stage.
    Post,
    /// `djf`: the orbit's DJ filter.
    Djf,
    /// `lfo_<id>` / `env_<id>` (the modulator loop's).
    Mod(String),
}

impl NodeName {
    /// superdough's key text.
    pub fn as_str(&self) -> &str {
        match self {
            NodeName::Source => "source",
            NodeName::PwLfo => "pw_lfo",
            NodeName::Vib => "vib",
            NodeName::VibGain => "vib_gain",
            NodeName::Fm(n) => FM_NAMES[usize::from(*n) - 1],
            NodeName::FmGain(n) => FM_GAIN_NAMES[usize::from(*n) - 1],
            NodeName::Transient => "transient",
            NodeName::Gain => "gain",
            NodeName::Lpf => "lpf",
            NodeName::LpfLfo => "lpf_lfo",
            NodeName::Hpf => "hpf",
            NodeName::HpfLfo => "hpf_lfo",
            NodeName::Bpf => "bpf",
            NodeName::BpfLfo => "bpf_lfo",
            NodeName::Vowel => "vowel",
            NodeName::Coarse => "coarse",
            NodeName::Crush => "crush",
            NodeName::Shape => "shape",
            NodeName::Distort => "distort",
            NodeName::Tremolo => "tremolo",
            NodeName::TremoloGain => "tremolo_gain",
            NodeName::Compressor => "compressor",
            NodeName::Pan => "pan",
            NodeName::Phaser => "phaser",
            NodeName::PhaserLfo => "phaser_lfo",
            NodeName::Delay => "delay",
            NodeName::DelayMix => "delay_mix",
            NodeName::Room => "room",
            NodeName::RoomMix => "room_mix",
            NodeName::Post => "post",
            NodeName::Djf => "djf",
            NodeName::Mod(s) => s,
        }
    }

    /// The FM modulator index `idx` (1..=8) as superdough numbers it.
    fn fm_index(idx: usize) -> u8 {
        debug_assert!(
            (1..=8).contains(&idx),
            "FM modulator index {idx} outside 1..=8"
        );
        u8::try_from(idx).expect("an FM modulator index fits a u8")
    }

    /// `fm_<idx>`, idx in 1..=8: FM modulator `idx`.
    pub fn fm(idx: usize) -> NodeName {
        NodeName::Fm(NodeName::fm_index(idx))
    }

    /// `fm_<idx>_gain`, idx in 1..=8: FM modulator `idx`'s amount.
    pub fn fm_gain(idx: usize) -> NodeName {
        NodeName::FmGain(NodeName::fm_index(idx))
    }

    /// A filter slot's stage name and its LFO's: `(lpf, lpf_lfo)` and so on.
    pub fn filter(kind: FilterKind) -> (NodeName, NodeName) {
        match kind {
            FilterKind::Lowpass => (NodeName::Lpf, NodeName::LpfLfo),
            FilterKind::Highpass => (NodeName::Hpf, NodeName::HpfLfo),
            FilterKind::Bandpass => (NodeName::Bpf, NodeName::BpfLfo),
            FilterKind::Notch => unreachable!("the phaser is not a filter slot"),
        }
    }
}

/// The plan of a voice: its source kind, the stage specs `Voice::build` walks, the FM matrix
/// walk, which FM modulators are noise, and superdough's `nodes` tracker in insertion order (`Object.keys`
/// order: a later write of a key replaces its list in place). [`NodeTable::plan`] is the only
/// constructor besides [`NodeTable::empty`]; the modulator loop adds `lfo_<id>`/`env_<id>`.
#[derive(Debug, Clone)]
pub struct NodeTable {
    entries: Vec<(FxKey, NodeName, NodeList)>,
    source: SourceKind,
    specs: Vec<StageSpec>,
    /// The FM matrix walk, one step per matrix entry in order.
    fm_steps: Vec<FmStep>,
    /// Per FM modulator position, whether it plays noise (no `frequency` param).
    fm_noise: Vec<bool>,
}

impl NodeTable {
    /// `nodes[key][name] = list`: replaces any earlier entry in place (the last writer wins).
    pub fn set(&mut self, key: FxKey, name: NodeName, list: NodeList) {
        match self
            .entries
            .iter_mut()
            .find(|(k, n, _)| *k == key && *n == name)
        {
            Some(e) => e.2 = list,
            None => self.entries.push((key, name, list)),
        }
    }

    /// `nodes[key][name].push(n)` of an element that may be `undefined` (a missing filter LFO);
    /// starts an array when the entry is missing, and turns a bare node into an array holding it
    /// first.
    fn push_element(&mut self, key: FxKey, name: NodeName, n: Option<Node>) {
        match self
            .entries
            .iter_mut()
            .find(|(k, m, _)| *k == key && *m == name)
        {
            Some((_, _, NodeList::Array(v))) => v.push(n),
            Some(e) => {
                if let NodeList::NotArray(first) = e.2 {
                    e.2 = NodeList::Array(vec![Some(first), n]);
                }
            }
            None => self.entries.push((key, name, NodeList::Array(vec![n]))),
        }
    }

    /// `nodes[key][name]`; `None` when superdough's entry is `undefined`.
    pub fn get(&self, key: FxKey, name: &str) -> Option<&NodeList> {
        self.entries
            .iter()
            .find(|(k, n, _)| *k == key && n.as_str() == name)
            .map(|(_, _, l)| l)
    }

    /// Whether `nodes[key]` exists (an FX entry of the hap).
    pub fn has_entry(&self, key: FxKey) -> bool {
        key == FxKey::Main || self.entries.iter().any(|(k, _, _)| *k == key)
    }

    /// `Object.keys(nodes[key])` in insertion order.
    pub fn keys(&self, key: FxKey) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(k, _, _)| *k == key)
            .map(|(_, n, _)| n.as_str())
            .collect()
    }

    /// The voice's source kind.
    pub fn source(&self) -> SourceKind {
        self.source
    }

    /// The kind of stage `id` of the plan.
    pub fn stage(&self, id: StageId) -> Option<StageKind> {
        self.specs.get(id.0).map(|s| s.kind())
    }

    /// The planned stages in chain order (stage `id` is `specs()[id.0]`).
    pub fn specs(&self) -> &[StageSpec] {
        &self.specs
    }

    /// The planned FM matrix walk `apply_fm` builds from, one step per matrix entry.
    pub fn fm_steps(&self) -> &[FmStep] {
        &self.fm_steps
    }

    /// Whether FM modulator `pos` plays noise.
    pub fn fm_is_noise(&self, pos: usize) -> bool {
        self.fm_noise.get(pos).copied().unwrap_or(false)
    }

    /// Appends a planned stage and returns its id.
    fn stage_id(&mut self, spec: StageSpec) -> StageId {
        self.specs.push(spec);
        StageId(self.specs.len() - 1)
    }

    /// Registers a single-node entry `nodes[key][name] = [node]`.
    fn one(&mut self, key: FxKey, name: NodeName, node: Node) {
        self.set(key, name, NodeList::Array(vec![Some(node)]));
    }

    /// The table of the entry `key` of `params` planned alone (no source, its stages from 0), as
    /// tests see it.
    #[cfg(test)]
    pub fn entry_plan(key: FxKey, params: &VoiceParams) -> NodeTable {
        let mut t = NodeTable::empty(SourceKind::Osc);
        let entry = params.entry(key).expect("an entry of params");
        plan_entry(key, entry, params.ftype_24db, &mut t);
        t
    }

    /// An empty table for a voice with source `source` and no stage.
    pub fn empty(source: SourceKind) -> NodeTable {
        NodeTable {
            entries: Vec::new(),
            source,
            specs: Vec::new(),
            fm_steps: Vec::new(),
            fm_noise: Vec::new(),
        }
    }

    /// The node table of a voice playing `params` with a source of kind `source`, in the order
    /// superdough fills it: the sound handle's nodes (`source`; `pw_lfo` for a
    /// pulse; `vib`, `vib_gain` and the FM matrix's `fm_<i>`/`fm_<i>_gain`, the vibrato before
    /// the FM for an oscillator and after it for the worklet synths), every FX entry's stages in
    /// order ([`plan_entry`]: the chain entries, then main), then `post`, `delay`, `delay_mix`,
    /// `room`, `room_mix` and `djf`. The stage list `Voice::build` walks: `Gain(0.3)` (not behind
    /// a worklet synth or the bus sound), `envGain`, the entries, the `FXrelease` gain, `post`.
    /// A pure function of the controls, so the build and `unsupported_signatures` agree.
    pub fn plan(params: &VoiceParams, source: SourceKind) -> NodeTable {
        let main = FxKey::Main;
        let mut t = NodeTable::empty(source);
        match source {
            SourceKind::Constant => t.set(main, NodeName::Source, NodeList::NotArray(Node::Source)),
            SourceKind::Bus => t.one(main, NodeName::Source, Node::Bus),
            _ => t.one(main, NodeName::Source, Node::Source),
        }
        let synth = matches!(
            source,
            SourceKind::Osc | SourceKind::Supersaw | SourceKind::Pulse
        );
        if source == SourceKind::Pulse {
            let node = (params.pwsweep != 0.0).then_some(Node::PulseLfo);
            t.set(main, NodeName::PwLfo, NodeList::Array(vec![node]));
        }
        let vib = synth && params.vib.is_some_and(|v| v > 0.0);
        let plan_vib = |t: &mut NodeTable| {
            if vib {
                t.one(main, NodeName::Vib, Node::Vibrato);
                t.one(main, NodeName::VibGain, Node::VibratoGain);
            }
        };
        if source == SourceKind::Osc {
            plan_vib(&mut t);
        }
        if synth {
            plan_fm(params, &mut t);
        }
        if matches!(source, SourceKind::Supersaw | SourceKind::Pulse) {
            plan_vib(&mut t);
        }
        if !matches!(
            source,
            SourceKind::Supersaw | SourceKind::Pulse | SourceKind::Bus
        ) {
            t.stage_id(StageSpec::SynthGain);
        }
        t.stage_id(StageSpec::EnvGain);
        for (key, entry) in params.entries() {
            plan_entry(key, entry, params.ftype_24db, &mut t);
        }
        if params.fx_release.is_some_and(|r| r > params.release_raw) {
            t.stage_id(StageSpec::Release);
        }
        let post = t.stage_id(StageSpec::Post);
        t.one(main, NodeName::Post, Node::Stage(post));
        let m = &params.main;
        if delay_applies(m) {
            t.one(main, NodeName::Delay, Node::OrbitDelay);
            t.one(main, NodeName::DelayMix, Node::DelaySend);
        }
        if m.room > 0.0 {
            t.one(main, NodeName::Room, Node::OrbitReverb);
            t.one(main, NodeName::RoomMix, Node::RoomSend);
        }
        if params.djf.is_some() {
            t.one(main, NodeName::Djf, Node::OrbitDjf);
        }
        t
    }
}

/// `applyFM`'s registrations (helpers.mjs:429-512): the matrix walk in its order, each side of an
/// entry creating its modulator on first use (`fm_<i> = [osc]`) and writing `fm_<i>_gain =
/// [gAmt]` (replacing): the source side's `Gain(amt)` is connection `n` ([`Node::FmAmount`]) when
/// the entry connects, the target side's (and the source side's of an entry whose target is a
/// noise modulator) is [`Node::Detached`]. Records the walk as one [`FmStep`] per entry, which
/// `apply_fm` builds from.
fn plan_fm(params: &VoiceParams, t: &mut NodeTable) {
    let mut created: [Option<usize>; 9] = [None; 9];
    let mut connections = 0;
    let is_noise = |idx: usize| crate::voice::noise_wave(&params.fm.mods[idx - 1]).is_some();
    for e in &params.fm.entries {
        let connects = e.target == 0 || !is_noise(e.target);
        let mut sides = [None, None];
        for (k, idx) in [e.source, e.target].into_iter().enumerate() {
            if idx == 0 {
                continue;
            }
            let (pos, first_use) = match created[idx] {
                Some(pos) => (pos, false),
                None => {
                    let pos = t.fm_noise.len();
                    t.fm_noise.push(is_noise(idx));
                    created[idx] = Some(pos);
                    t.one(FxKey::Main, NodeName::fm(idx), Node::FmOsc(pos));
                    (pos, true)
                }
            };
            sides[k] = Some(FmSide {
                idx,
                pos,
                first_use,
            });
            let node = if k == 0 && connects {
                Node::FmAmount(connections)
            } else {
                Node::Detached {
                    value: e.amount as f32,
                }
            };
            t.one(FxKey::Main, NodeName::fm_gain(idx), node);
        }
        t.fm_steps.push(FmStep { sides, connects });
        if connects {
            connections += 1;
        }
    }
}

/// The stages and registrations of one FX entry, in superdough's order (superdough.mjs:612-938):
/// the `transient` (a bare node), the gain stage, the lowpass, highpass and bandpass filters (two
/// biquads each for a 24 dB filter, one ladder each for `ftype('ladder')`) with their filter LFO
/// elements (`undefined` without one), the vowel's five formants, the `coarse`, `crush`, `shape`
/// and `distort` worklets, the tremolo (`tremolo`, `tremolo_gain`), the compressor, the panner
/// and the phaser (`phaser`, `phaser_lfo`), and for a chain entry (not `Main`) the local delay
/// (`delay` and its wet gain `delay_mix`) and reverb (`room`, `room_mix`). `ftype_24db` is the
/// hap's (the doubling reads `value.ftype`).
pub fn plan_entry(key: FxKey, params: &EntryParams, ftype_24db: bool, t: &mut NodeTable) {
    if params.transient.is_some() {
        let id = t.stage_id(StageSpec::Transient(key));
        t.set(
            key,
            NodeName::Transient,
            NodeList::NotArray(Node::Stage(id)),
        );
    }
    let id = t.stage_id(StageSpec::EntryGain(key));
    t.one(key, NodeName::Gain, Node::Stage(id));
    for (kind, filter) in params.filters() {
        let Some(f) = filter else {
            continue;
        };
        let (name, lfo_name) = NodeName::filter(kind);
        let has_lfo = filter_lfo_props(f, &EntryTimes::PLAN).is_some();
        let ladder = params.ftype_ladder;
        let copies = if !ladder && ftype_24db { 2 } else { 1 };
        let spec = StageSpec::Filter {
            entry: key,
            kind,
            ladder,
        };
        for k in 0..copies {
            let id = t.stage_id(spec);
            let lfo = has_lfo.then_some(Node::StageLfo(id));
            if k == 0 {
                t.one(key, name.clone(), Node::Stage(id));
                t.set(key, lfo_name.clone(), NodeList::Array(vec![lfo]));
            } else {
                t.push_element(key, name.clone(), Some(Node::Stage(id)));
                t.push_element(key, lfo_name.clone(), lfo);
            }
        }
    }
    if params.vowel.is_some() {
        let id = t.stage_id(StageSpec::Vowel(key));
        let filters = (0..5).map(|n| Some(Node::VowelFilter(id, n))).collect();
        t.set(key, NodeName::Vowel, NodeList::Array(filters));
    }
    let shapers = [
        (
            params.coarse.is_some(),
            StageSpec::Coarse(key),
            NodeName::Coarse,
        ),
        (
            params.crush.is_some(),
            StageSpec::Crush(key),
            NodeName::Crush,
        ),
        (
            params.shape.is_some(),
            StageSpec::Shape(key),
            NodeName::Shape,
        ),
        (
            params.distort.is_some(),
            StageSpec::Distort(key),
            NodeName::Distort,
        ),
    ];
    for (present, spec, name) in shapers {
        if present {
            let id = t.stage_id(spec);
            t.one(key, name, Node::Stage(id));
        }
    }
    if params.tremolo.is_some() {
        let id = t.stage_id(StageSpec::Tremolo(key));
        t.one(key, NodeName::Tremolo, Node::StageLfo(id));
        t.one(key, NodeName::TremoloGain, Node::Stage(id));
    }
    if params.compressor.is_some() {
        let id = t.stage_id(StageSpec::Compressor(key));
        t.one(key, NodeName::Compressor, Node::Stage(id));
    }
    if params.pan.is_some() {
        let id = t.stage_id(StageSpec::Pan(key));
        t.one(key, NodeName::Pan, Node::Stage(id));
    }
    if params.phaser.is_some() {
        let id = t.stage_id(StageSpec::Phaser(key));
        t.one(key, NodeName::Phaser, Node::Stage(id));
        t.one(key, NodeName::PhaserLfo, Node::StageLfo(id));
    }
    if key == FxKey::Main {
        return;
    }
    if delay_applies(params) {
        let id = t.stage_id(StageSpec::LocalDelay(key));
        t.one(key, NodeName::Delay, Node::Stage(id));
        t.one(key, NodeName::DelayMix, Node::StageWet(id));
    }
    if params.room > 0.0 {
        let id = t.stage_id(StageSpec::LocalReverb(key));
        t.one(key, NodeName::Room, Node::Stage(id));
        t.one(key, NodeName::RoomMix, Node::StageWet(id));
    }
}

/// `delay > 0 && delaytime > 0 && delayfeedback > 0` of an entry (JS `>`: a NaN fails): the main
/// entry's orbit delay send, a chain entry's local delay.
pub fn delay_applies(e: &EntryParams) -> bool {
    e.delay > 0.0 && e.delaytime > 0.0 && e.delayfeedback > 0.0
}

/// The param-connection down-mix of a stereo signal into a (mono) param (Blink's
/// `SumFromByDownMixing`, speakers, straight into the param's values: `v += 0.5 * L`, then `v +=
/// 0.5 * R`, two unfused f32 accumulations): the two terms, in that order (the param sums each
/// in turn).
pub fn downmix_terms(ch: &[[f32; QUANTUM]; 2]) -> [[f32; QUANTUM]; 2] {
    ch.map(|c| c.map(|x| 0.5 * x))
}

/// When an FX entry's voice plays: what its time-dependent nodes are built from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryTimes {
    /// The onset `t` (the filter envelopes' begin and the `transient`'s).
    pub begin: f64,
    /// `holdEnd = end = t + hapDuration` (the filter envelopes' end).
    pub hold_end: f64,
    /// `endWithRelease = end + max(release, FXrelease ?? 0)` (the `transient`'s end).
    pub end_with_release: f64,
    /// The scheduling (pause) frame.
    pub now: u64,
    /// The musical onset used for cycle-synced LFO phase.
    pub cycle: f64,
    /// Elapsed layout seconds at the onset for fixed-Hz modulation.
    pub phase_sec: f64,
    /// The local cycles per second at this onset.
    pub cps: f64,
}

impl EntryTimes {
    /// Times for planning only (which nodes exist does not depend on them).
    pub const PLAN: EntryTimes = EntryTimes {
        begin: 0.0,
        hold_end: 0.0,
        end_with_release: 0.0,
        now: 0,
        cycle: 0.0,
        phase_sec: 0.0,
        cps: 1.0,
    };
}

/// `param.value = v` on a worklet param (`getWorklet`): a WebIDL float (a non-finite value
/// throws), clamped to the range, with its `setValueAtTime(v, now)`.
pub(crate) fn set_worklet_value(p: &mut Param, v: f64, now: u64) -> Result<(), VoiceError> {
    p.set_value(idl_float(v, "value")?, now)?;
    Ok(())
}

/// `getWorklet(ac, 'ladder-processor', { frequency, q, drive })` (each `.value =` in that order,
/// each throwing on a non-finite value), then the filter envelope on its `frequency` as for a
/// biquad (`createFilter`).
fn build_ladder(
    f: &FilterParams,
    params: &EntryParams,
    begin: f64,
    hold_end: f64,
    now: u64,
) -> Result<Ladder, VoiceError> {
    let mut l = Ladder::new();
    set_worklet_value(&mut l.frequency, f.frequency, now)?;
    set_worklet_value(&mut l.q, f.q, now)?;
    set_worklet_value(&mut l.drive, params.drive, now)?;
    filter_envelope(&mut l.frequency, f, params.fanchor, begin, hold_end, now)?;
    Ok(l)
}

/// `createFilter`'s LFO (helpers.mjs:286-302 and `getParamLfo`, helpers.mjs:195-213), scheduled at
/// `now`: `rate = cps * sync` when `sync != null`; active when any of `depth`, `depthfrequency`,
/// `skew`, `shape`, `rate` is defined; `modDepth = depthfrequency ?? (depth ?? 1) * frequency`;
/// built only when `modDepth` is truthy, with `{ begin, end: holdEnd, depth: modDepth,
/// dcoffset (-0.5), skew, shape, frequency: rate ?? cps, min: -frequency + 30, max: 20000 -
/// frequency, curve: 1 }` (`frequency` is the filter's raw cutoff). Fixed rates use elapsed time
/// and cycle-synced rates use musical time.
pub fn filter_lfo(f: &FilterParams, times: &EntryTimes) -> Result<Option<Lfo>, VoiceError> {
    match filter_lfo_props(f, times) {
        Some(props) => Ok(Some(get_lfo(&props, times.now)?)),
        None => Ok(None),
    }
}

/// The props of [`filter_lfo`], `None` when no LFO is built (whether one is does not depend on
/// `times`).
pub fn filter_lfo_props(f: &FilterParams, times: &EntryTimes) -> Option<LfoProps> {
    let l = &f.lfo;
    let defined = |v: &Option<Value>| !matches!(v, None | Some(Value::Null));
    let rate = if defined(&l.sync) {
        Some(Value::Num(
            times.cps * l.sync.as_ref().map_or(0.0, js_to_number),
        ))
    } else {
        l.rate.clone()
    };
    let has_lfo = [&l.depth, &l.depthfrequency, &l.skew, &l.shape, &rate]
        .iter()
        .any(|v| v.is_some());
    if !has_lfo {
        return None;
    }
    // `depth = depth ?? 1`; `modDepth = depthfrequency ?? (depth ?? 1) * frequency`.
    let depth = if defined(&l.depth) {
        l.depth.as_ref().map_or(1.0, js_to_number)
    } else {
        1.0
    };
    let (truthy, mod_depth) = match &l.depthfrequency {
        Some(v) if defined(&l.depthfrequency) => (js_truthy(v), js_to_number(v)),
        _ => {
            let d = depth * f.frequency;
            (d != 0.0 && !d.is_nan(), d)
        }
    };
    if !truthy {
        return None;
    }
    let frequency = match &rate {
        Some(v) if defined(&rate) => js_to_number(v),
        _ => times.cps,
    };
    Some(LfoProps {
        begin: Some(times.begin),
        end: Some(times.hold_end),
        time: Some(if defined(&l.sync) || !defined(&l.rate) {
            times.cycle / times.cps
        } else {
            times.phase_sec
        }),
        depth: Some(mod_depth),
        dcoffset: l.dcoffset.as_ref().map(js_to_number),
        frequency: Some(frequency),
        skew: l.skew.as_ref().map(js_to_number),
        phaseoffset: None,
        curve: Some(1.0),
        shape: l.shape.as_ref().map(ShapeInput::from_value),
        min: Some(-f.frequency + 30.0),
        max: Some(20000.0 - f.frequency),
    })
}

/// Keeps a filter's LFO (when one was built) for the voice, connected to `slot` of `stage`.
fn push_filter_lfo(
    (stage, slot): (StageId, ParamSlot),
    lfo: Option<Lfo>,
    lfos: &mut Vec<ParamLfo>,
) {
    if let Some(lfo) = lfo {
        lfos.push(ParamLfo { stage, slot, lfo });
    }
}

/// Builds the stage `spec` names (each throwing as superdough does), reading its values from
/// `params` (the entry's through `params.entry(key)`, planned present): the `transient` worklet,
/// the gain stage `Gain(gain * velocity)`, one copy of a lowpass, highpass or bandpass filter (a
/// biquad, or a ladder for the entry's `ftype('ladder')`) with its filter LFO when one is built,
/// the vowel, the `coarse`, `crush`, `shape` and `distort` worklets, the tremolo, the compressor,
/// the panner, the phaser, a chain entry's local delay and local reverb, the `FXrelease` gain and
/// `post`. A filter's or the phaser's LFO goes to `lfos`, connected to stage `id`. Not for the
/// head specs (`SynthGain`, `EnvGain`), which `Voice::build` builds with the source's envelope.
pub(crate) fn build_stage(
    spec: StageSpec,
    id: StageId,
    params: &VoiceParams,
    times: &EntryTimes,
    lfos: &mut Vec<ParamLfo>,
) -> Result<Stage, VoiceError> {
    let EntryTimes {
        begin,
        hold_end,
        end_with_release,
        now,
        cycle,
        cps,
        phase_sec,
    } = *times;
    let entry = |key: FxKey| params.entry(key).expect("planned");
    Ok(match spec {
        StageSpec::SynthGain | StageSpec::EnvGain => unreachable!("Voice::build builds the head"),
        StageSpec::Transient(key) => {
            let e = entry(key);
            let attack = e.transient.expect("planned");
            // Right after the amp envelope: its input goes with the source (the gains pass the
            // disabling on).
            let node = TransientNode::new(Transient::new(
                Some(attack),
                e.transsustain,
                begin,
                end_with_release,
            ));
            Stage::Transient(Box::new(node))
        }
        StageSpec::EntryGain(key) => {
            let gain = idl_float(gain_stage(entry(key)), "gain")?;
            Stage::Gain(GainNode::constant(gain))
        }
        StageSpec::Filter {
            entry: key,
            kind,
            ladder,
        } => {
            let e = entry(key);
            let f = filter_of(e, kind).expect("planned");
            if ladder {
                // `createFilter` with `model === 'ladder'`: the same lowpass processor in every
                // slot, one per slot (`'ladder'` is never `'24db'`).
                let l = build_ladder(f, e, begin, hold_end, now)?;
                let lfo = filter_lfo(f, times)?;
                push_filter_lfo((id, ParamSlot::LADDER_FREQUENCY), lfo, lfos);
                Stage::Ladder(Box::new(LadderNode::new(l)))
            } else {
                let b = create_filter(kind, f, e.fanchor, begin, hold_end, now)?;
                let lfo = filter_lfo(f, times)?;
                push_filter_lfo((id, ParamSlot::BIQUAD_FREQUENCY), lfo, lfos);
                Stage::Biquad(b)
            }
        }
        StageSpec::Vowel(key) => {
            let letter = entry(key).vowel.as_ref().expect("planned");
            // `ac.createVowelFilter(fx.vowel)`: an unknown letter throws here.
            let v = Vowel::new(letter, now).map_err(hap_error)?;
            Stage::Vowel(Box::new(v))
        }
        StageSpec::Coarse(key) => {
            let v = entry(key).coarse.expect("planned");
            let mut c = Coarse::new();
            set_worklet_value(&mut c.coarse, v, now)?;
            Stage::Shaper(Shaper::new(ShaperKind::Coarse(c)))
        }
        StageSpec::Crush(key) => {
            let v = entry(key).crush.expect("planned");
            let mut c = Crush::new();
            set_worklet_value(&mut c.crush, v, now)?;
            Stage::Shaper(Shaper::new(ShaperKind::Crush(c)))
        }
        StageSpec::Shape(key) => {
            let e = entry(key);
            let v = e.shape.expect("planned");
            let mut s = Shape::new();
            set_worklet_value(&mut s.shape, v, now)?;
            set_worklet_value(&mut s.postgain, e.shapevol, now)?;
            Stage::Shaper(Shaper::new(ShaperKind::Shape(s)))
        }
        StageSpec::Distort(key) => {
            let e = entry(key);
            let v = e.distort.expect("planned");
            // `distorttype = getDefaultValue('distorttype')` (0) when undefined.
            let algorithm = e.distorttype.clone().unwrap_or(Value::Num(0.0));
            let mut d = Distort::new(&algorithm);
            set_worklet_value(&mut d.distort, v, now)?;
            set_worklet_value(&mut d.postgain, e.distortvol, now)?;
            Stage::Shaper(Shaper::new(ShaperKind::Distort(Box::new(d))))
        }
        StageSpec::Tremolo(key) => {
            let tr = entry(key).tremolo.as_ref().expect("planned");
            // `new GainNode(ac, { gain: Math.max(1 - tremolodepth, 0) })` (a non-finite gain
            // throws), then `getLfo` (each `.value =` may throw).
            let mut gain = GainNode::new(Param::gain(1.0));
            gain.gain
                .set_initial_value(jsmath::max(1.0 - tr.depth, 0.0), "gain")?;
            let props = LfoProps {
                skew: Some(tr.skew),
                frequency: Some(tr.frequency),
                depth: Some(tr.depth),
                time: Some(if tr.synced { cycle / cps } else { phase_sec }),
                dcoffset: Some(0.0),
                shape: tr.shape.as_ref().map(ShapeInput::from_value),
                phaseoffset: tr.phase,
                min: Some(0.0),
                max: Some(1.0),
                curve: Some(1.5),
                begin: Some(begin),
                end: Some(end_with_release),
            };
            let lfo = get_lfo(&props, now)?;
            Stage::Tremolo(Box::new(Tremolo {
                lfo,
                gain,
                inputs: ParamInputs::NONE,
            }))
        }
        StageSpec::Compressor(key) => {
            let c = entry(key).compressor.as_ref().expect("planned");
            // `getCompressor`: each `.value =` may throw (a non-finite value).
            let node = Compressor::new(c, now)?;
            Stage::Compressor(Box::new(node))
        }
        StageSpec::Pan(key) => {
            let pan = entry(key).pan.expect("planned");
            let mut p = StereoPanner::new();
            p.pan.set_value(idl_float(2.0 * pan - 1.0, "pan")?, now)?;
            Stage::Panner(p)
        }
        StageSpec::Phaser(key) => {
            let ph = entry(key).phaser.as_ref().expect("planned");
            let (notch, lfo) = build_phaser(ph, begin, end_with_release, now)?;
            lfos.push(ParamLfo {
                stage: id,
                slot: ParamSlot::BIQUAD_DETUNE,
                lfo,
            });
            Stage::Phaser(notch)
        }
        StageSpec::LocalDelay(key) => {
            let d = LocalDelay::new(entry(key), TailCut::at(end_with_release), now)?;
            Stage::LocalDelay(Box::new(d))
        }
        StageSpec::LocalReverb(key) => {
            let r = LocalReverb::new(entry(key), TailCut::at(end_with_release), now)?;
            Stage::LocalReverb(Box::new(r))
        }
        // The release gain when `FXrelease > release` (superdough.mjs:941-946, after the FX loop,
        // main entry only): `gainNode(1)`, `setValueAtTime(1, end + release)`, then a linear ramp
        // to 0 at `endWithRelease`.
        StageSpec::Release => {
            let mut g = GainNode::new(Param::gain(1.0));
            g.gain.set_value(1.0, now)?;
            g.gain
                .set_value_at_time(1.0, hold_end + params.release_raw, now)?;
            g.gain.linear_ramp_to(0.0, end_with_release, now)?;
            Stage::Release(g)
        }
        StageSpec::Post => Stage::Gain(GainNode::constant(idl_float(params.postgain, "gain")?)),
    })
}

/// The filter of `e` in slot `kind` (`Notch` is the phaser's, never a filter slot).
fn filter_of(e: &EntryParams, kind: FilterKind) -> Option<&FilterParams> {
    e.filters()
        .into_iter()
        .find(|(k, _)| *k == kind)
        .and_then(|(_, f)| f)
}

/// When a local delay or reverb is cut: it renders
/// the quantum holding `frame_for_param_event(endWithRelease)` (the first frame at which the
/// `FXrelease` ramp has reached 0) normally and outputs silence from the next quantum on. A
/// non-finite `endWithRelease` means no cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailCut {
    /// The first frame of the cut quantum.
    frame: Option<u64>,
}

impl TailCut {
    /// The cut of a hap whose `endWithRelease` is `end_with_release` seconds.
    pub fn at(end_with_release: f64) -> TailCut {
        use crate::QUANTUM_U64 as Q;
        let frame = end_with_release
            .is_finite()
            .then(|| crate::render::frame_for_param_event(end_with_release) / Q * Q);
        TailCut { frame }
    }

    /// The cut quantum's index (`frame / 128`), `None` without a cut.
    #[cfg(test)]
    pub fn quantum(self) -> Option<u64> {
        self.frame.map(|f| f / QUANTUM as u64)
    }

    /// Whether the quantum starting at `q` comes after the cut quantum: the stage outputs silence
    /// and is done.
    pub fn is_cut(self, q: u64) -> bool {
        self.frame.is_some_and(|f| q > f)
    }

    /// The first quantum start from which the stage's output has no rendering connection, given
    /// that its input is gone from `input_gone`: the later of that and the quantum after the cut.
    fn disabled_from(self, input_gone: u64) -> Option<u64> {
        self.frame
            .map(|f| input_gone.max(f.saturating_add(QUANTUM as u64)))
    }
}

/// `gainNode(level)` at frame `now`: `createGain()`, then `.value = level` (a WebIDL float, so a
/// non-finite level throws; it also schedules `setValueAtTime(level, now)`).
fn value_gain(level: f64, now: u64) -> Result<GainNode, VoiceError> {
    let mut g = GainNode::new(Param::gain(1.0));
    g.gain.set_value(idl_float(level, "gain")?, now)?;
    Ok(g)
}

/// The end of a local delay or reverb stage for one quantum: the effect's return `ret` through
/// the wet gain (`wet_inputs` its modulators), the dry branch (`bus` through `dry`, in place),
/// then both summed into the 2-channel explicit `sum`: the dry signal up-mixed by copy, the wet
/// one added (`AudioBus::SumFrom`, the dry branch connected first).
fn sum_dry_and_wet(
    q: u64,
    bus: &mut VoiceBus,
    ret: Input<'_>,
    wet: &mut GainNode,
    wet_inputs: &ParamInputs,
    dry: &mut GainNode,
) {
    let m = ret.channels.len();
    let mut wet_bus = [[0.0f32; QUANTUM]; 2];
    wet_bus[..m].copy_from_slice(ret.channels);
    let wet_silent = wet.process(q, &mut wet_bus[..m], ret.silent, wet_inputs);
    bus.apply_gain(q, dry, &ParamInputs::NONE);
    bus.upmix_to_stereo();
    let mut silent = [bus.silent; 2];
    let wet = Input {
        channels: &wet_bus[..m],
        silent: wet_silent,
    };
    sum_stereo(&mut bus.ch, &mut silent, wet);
    bus.silent = silent[0] && silent[1];
}

/// Outputs the silent stereo bus of a stage past its cut.
fn cut_output(bus: &mut VoiceBus) {
    bus.ch = [[0.0; QUANTUM]; 2];
    bus.channels = 2;
    bus.silent = true;
}

/// A chain entry's local delay (superdough.mjs:889-904), when `delay > 0 && delaytime > 0 &&
/// delayfeedback > 0`: `ac.createFeedbackDelay(1, delaytime, clamp(delayfeedback, 0, 0.98))`
/// (no `start(t)`), `wetDelay = gainNode(delay)` (no gain curve) and `dryDelay = gainNode(dry ??
/// 1)`; `tail -> dryDelay` and `tail -> delay -> wetDelay` sum into the 2-channel `sum`.
/// Superdough's `dry = gainNode(1)` in front of both is the identity and is not built.
#[derive(Debug, Clone)]
pub struct LocalDelay {
    pub delay: FeedbackDelay,
    pub wet: GainNode,
    pub dry: GainNode,
    pub tail: TailCut,
}

impl LocalDelay {
    /// Builds the local delay of entry `e` at frame `now`, each step throwing in superdough's
    /// order (the delay's `delayTime.value`, then `wetDelay`, then `dryDelay`).
    pub fn new(e: &EntryParams, tail: TailCut, now: u64) -> Result<LocalDelay, VoiceError> {
        let feedback = js_min(js_max(e.delayfeedback, 0.0), MAX_FEEDBACK);
        let delay = FeedbackDelay::new(e.delaytime, feedback, now)?;
        let wet = value_gain(e.delay, now)?;
        let dry = value_gain(e.dry.unwrap_or(1.0), now)?;
        Ok(LocalDelay {
            delay,
            wet,
            dry,
            tail,
        })
    }

    /// One quantum in place: the delay on the chain's tail (`feedback` and `delayTime` with their
    /// connections), its return through the wet gain, the dry gain, then the stereo sum. Past the
    /// cut a silent stereo bus.
    fn process(&mut self, q: u64, bus: &mut VoiceBus, inputs: &StageInputs) {
        if self.tail.is_cut(q) {
            cut_output(bus);
            return;
        }
        let n = bus.channels;
        self.delay
            .begin_with(q, n, inputs.of(ParamSlot::DELAY_FEEDBACK));
        self.delay.add_send(Input {
            channels: &bus.ch[..n],
            silent: bus.silent,
        });
        let ret = self.delay.finish_with(q, inputs.of(ParamSlot::DELAY_TIME));
        sum_dry_and_wet(q, bus, ret, &mut self.wet, &inputs.wet, &mut self.dry);
    }
}

/// A chain entry's local reverb (superdough.mjs:906-938), when `room > 0`: a new convolver per
/// voice, `createReverb(roomsize, roomfade, roomlp, roomdim)` (its IR request is settled at the
/// end of the pause through the run's `IrCache`), `wetReverb = gainNode(room)` and `dryReverb
/// = gainNode(dry ?? 1)`; the same dry and wet sum as the local delay.
#[derive(Debug, Clone)]
pub struct LocalReverb {
    pub reverb: Reverb,
    pub wet: GainNode,
    pub dry: GainNode,
    pub tail: TailCut,
}

impl LocalReverb {
    /// Builds the local reverb of entry `e` at frame `now`: the reverb (an `IrError` throws, as
    /// the orbit's does), then `wetReverb`, then `dryReverb`.
    pub fn new(e: &EntryParams, tail: TailCut, now: u64) -> Result<LocalReverb, VoiceError> {
        let reverb = Reverb::create(
            e.roomsize.as_ref(),
            e.roomfade.as_ref(),
            e.roomlp.as_ref(),
            e.roomdim.as_ref(),
            f64::from(crate::SAMPLE_RATE),
        )?;
        let wet = value_gain(e.room, now)?;
        let dry = value_gain(e.dry.unwrap_or(1.0), now)?;
        Ok(LocalReverb {
            reverb,
            wet,
            dry,
            tail,
        })
    }

    /// One quantum in place: the convolver on the chain's tail, its (stereo) return through the
    /// wet gain, the dry gain, then the stereo sum. Past the cut a silent stereo bus.
    fn process(&mut self, q: u64, bus: &mut VoiceBus, inputs: &StageInputs) {
        if self.tail.is_cut(q) {
            cut_output(bus);
            return;
        }
        let n = bus.channels;
        let ret = self.reverb.process(
            q,
            Input {
                channels: &bus.ch[..n],
                silent: bus.silent,
            },
        );
        sum_dry_and_wet(q, bus, ret, &mut self.wet, &inputs.wet, &mut self.dry);
    }
}

/// `getPhaser(begin, end, rate, depth, center, sweep)` (superdough.mjs:353-372) at frame `now`:
/// `getLfo({ frequency: rate, depth: sweep * 2, begin, end })` (a triangle from `-sweep` to
/// `sweep` cents), then one notch biquad (`numStages = 1`) with `gain.value = 1`,
/// `frequency.value = center + 282` and `Q.value = 2 - min(max(depth * 2, 0), 1.9)`, the LFO
/// connected to its `detune`. Each `.value =` throws on a non-finite value.
pub fn build_phaser(
    p: &crate::params::PhaserParams,
    begin: f64,
    end: f64,
    now: u64,
) -> Result<(Biquad, Lfo), VoiceError> {
    let props = LfoProps {
        frequency: Some(p.rate),
        depth: Some(p.sweep * 2.0),
        begin: Some(begin),
        end: Some(end),
        ..LfoProps::default()
    };
    let lfo = get_lfo(&props, now)?;
    let mut notch = Biquad::new(FilterKind::Notch);
    set_worklet_value(&mut notch.gain, 1.0, now)?;
    set_worklet_value(&mut notch.frequency, p.center + 282.0, now)?;
    let q = 2.0 - js_min(js_max(p.depth * 2.0, 0.0), 1.9);
    set_worklet_value(&mut notch.q, q, now)?;
    Ok((notch, lfo))
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
pub(crate) mod tests {
    use super::StageSpec as S;
    use super::*;
    use crate::job::{OrbitSpec, ValueMap};
    use crate::noise::NoiseCache;
    use crate::orbit::Orbit;
    use crate::panner::{pan_to_target_value, pan_with_sample_accurate_values};
    use crate::param::Param;
    use crate::params::{parse_entry_params, parse_voice_params};
    use crate::periodic::WaveTables;
    use crate::reverb::IrCache;
    use crate::shaper::Distort;
    use crate::voice::{StartInfo, Voice, VoiceOutput, scale};
    use crate::worklet;
    use serde_json::json;

    /// A deterministic, non-trivial test signal for channel `c`.
    fn signal(c: usize) -> [f32; QUANTUM] {
        let mut s = [0.0f32; QUANTUM];
        for i in 0..QUANTUM {
            s[i] = ((i as f32 * 0.37 + c as f32 * 1.3).sin() * 0.8) - 0.05 * c as f32;
        }
        s
    }

    fn bits(b: &[[f32; QUANTUM]]) -> Vec<u32> {
        b.iter().flatten().map(|x| x.to_bits()).collect()
    }

    fn stereo_bus(silent: bool) -> VoiceBus {
        VoiceBus {
            ch: [signal(0), signal(1)],
            channels: 2,
            silent,
        }
    }

    fn mono_bus(silent: bool) -> VoiceBus {
        let mut b = VoiceBus::mono();
        b.ch[0] = signal(0);
        b.silent = silent;
        b
    }

    #[test]
    fn voice_bus_mono_and_active() {
        let mut b = VoiceBus::mono();
        assert_eq!(b.channels, 1);
        assert!(b.silent);
        assert!(b.ch.iter().flatten().all(|x| x.to_bits() == 0));
        assert_eq!(b.active().len(), 1);
        b.channels = 2;
        b.active()[1][5] = 3.0;
        assert_eq!(b.active().len(), 2);
        assert_eq!(b.ch[1][5], 3.0);
    }

    #[test]
    fn stage_channels_out() {
        let gain = Stage::Gain(GainNode::constant(0.5));
        let biquad = Stage::Biquad(Biquad::new(FilterKind::Lowpass));
        let panner = Stage::Panner(StereoPanner::new());
        for n in [1, 2] {
            assert_eq!(gain.channels_out(n), n, "gain {n}");
            assert_eq!(biquad.channels_out(n), n, "biquad {n}");
            assert_eq!(panner.channels_out(n), 2, "panner {n}");
        }
        // Processing agrees with channels_out.
        for (mut s, n) in [(gain, 1), (biquad, 2), (panner, 1)] {
            let mut bus = if n == 1 {
                mono_bus(false)
            } else {
                stereo_bus(false)
            };
            let want = s.channels_out(n);
            s.process(0, &mut bus, &StageInputs::NONE);
            assert_eq!(bus.channels, want, "{s:?}");
        }
    }

    fn lowpass() -> Biquad {
        let mut b = Biquad::new(FilterKind::Lowpass);
        b.frequency.set_value(800.0, 0).unwrap();
        b.q.set_value(4.0, 0).unwrap();
        b
    }

    #[test]
    fn stereo_biquad_matches_two_mono_biquads() {
        let mut stage = Stage::Biquad(lowpass());
        let (mut left, mut right) = (lowpass(), lowpass());
        // Sounding quanta, then silent ones until the tail ends and silence propagates.
        let mut saw_silent_output = false;
        for k in 0..400u64 {
            let q = k * QUANTUM as u64;
            let input_silent = k >= 3;
            let mut bus = stereo_bus(input_silent);
            if input_silent {
                bus.ch = [[0.0; QUANTUM]; 2];
            }
            let mut l = [bus.ch[0]];
            let mut r = [bus.ch[1]];
            stage.process(q, &mut bus, &StageInputs::NONE);
            let sl = left.process(q, &mut l, input_silent, &BiquadInputs::NONE);
            let sr = right.process(q, &mut r, input_silent, &BiquadInputs::NONE);
            assert_eq!(sl, sr, "quantum {k}");
            assert_eq!(bus.silent, sl, "silence hint, quantum {k}");
            assert_eq!(bus.channels, 2);
            assert_eq!(bits(&bus.ch[..1]), bits(&l), "left, quantum {k}");
            assert_eq!(bits(&bus.ch[1..]), bits(&r), "right, quantum {k}");
            saw_silent_output |= bus.silent;
        }
        assert!(saw_silent_output, "the tail ended within the test");
        match &stage {
            Stage::Biquad(b) => assert_eq!(b.kernels().len(), 2),
            _ => unreachable!(),
        }
    }

    fn panner_at(pan: f32, now: u64) -> StereoPanner {
        let mut p = StereoPanner::new();
        p.pan.set_value(pan, now).unwrap();
        p
    }

    #[test]
    fn stereo_bus_into_the_panner_takes_the_stereo_law() {
        for pan in [-0.6f32, 0.0, 0.4] {
            // The first quantum holds the `setValueAtTime` (sample accurate), later ones the
            // target value.
            let mut stage = Stage::Panner(panner_at(pan, 0));
            let mut bus = stereo_bus(false);
            let input = bus.ch;
            stage.process(0, &mut bus, &StageInputs::NONE);
            let mut want: Stereo = [[0.0; QUANTUM]; 2];
            pan_with_sample_accurate_values(&input, &[pan; QUANTUM], &mut want);
            assert_eq!(bits(&bus.ch), bits(&want), "pan {pan}, first quantum");
            assert!(!bus.silent);
            assert_eq!(bus.channels, 2);
            let mut bus = stereo_bus(false);
            stage.process(128, &mut bus, &StageInputs::NONE);
            pan_to_target_value(&input, pan, &mut want);
            assert_eq!(bits(&bus.ch), bits(&want), "pan {pan}, later quantum");
        }
        // No -3 dB centre: centred, the right channel passes unchanged (gain sin(pi/2) = 1)
        // and the left one gets only cos(pi/2) (about 6e-17) of the right added.
        let mut stage = Stage::Panner(StereoPanner::new());
        let mut bus = stereo_bus(false);
        let input = bus.ch;
        stage.process(0, &mut bus, &StageInputs::NONE);
        let mut want: Stereo = [[0.0; QUANTUM]; 2];
        pan_to_target_value(&input, 0.0, &mut want);
        assert_eq!(bits(&bus.ch), bits(&want));
        assert_eq!(bits(&bus.ch[1..]), bits(&input[1..]));
        for i in 0..QUANTUM {
            assert!((bus.ch[0][i] - input[0][i]).abs() < 1e-15, "frame {i}");
        }
    }

    #[test]
    fn mono_bus_into_the_panner_takes_the_mono_law() {
        for pan in [-0.6f32, 0.0, 0.4] {
            let mut stage = Stage::Panner(panner_at(pan, 0));
            let mut bus = mono_bus(false);
            let input = [bus.ch[0]];
            stage.process(0, &mut bus, &StageInputs::NONE);
            let mut want: Stereo = [[0.0; QUANTUM]; 2];
            pan_with_sample_accurate_values(&input, &[pan; QUANTUM], &mut want);
            assert_eq!(bits(&bus.ch), bits(&want), "pan {pan}, first quantum");
            assert_eq!(bus.channels, 2);
            let mut bus = mono_bus(false);
            stage.process(128, &mut bus, &StageInputs::NONE);
            pan_to_target_value(&input, pan, &mut want);
            assert_eq!(bits(&bus.ch), bits(&want), "pan {pan}, later quantum");
        }
        // The -3 dB centre: both channels are the input times cos(pi/4) as a float.
        let mut stage = Stage::Panner(StereoPanner::new());
        let mut bus = mono_bus(false);
        let x = bus.ch[0];
        stage.process(0, &mut bus, &StageInputs::NONE);
        let g = std::f64::consts::FRAC_PI_4.cos() as f32;
        assert_eq!(bus.ch[0][1].to_bits(), (x[1] * g).to_bits());
        // A silent mono input past the zero tail gives a silent stereo zero bus.
        let mut bus = VoiceBus::mono();
        stage.process(1280, &mut bus, &StageInputs::NONE);
        assert!(bus.silent);
        assert_eq!(bus.channels, 2);
        assert!(bus.ch.iter().flatten().all(|x| x.to_bits() == 0));
    }

    #[test]
    fn constant_gain_matches_scale() {
        for g in [0.0f32, 0.3, 1.0, -1.0] {
            let node = GainNode::constant(g);
            assert!(node.gain.events().is_empty(), "no setValueAtTime event");
            assert_eq!(node.gain.value(), g);
            assert_eq!(node.gain.default_value(), g);
            let mut stage = Stage::Gain(node);
            // Sounding, silent (first silent quantum and later ones), sounding again, on mono
            // and stereo buses.
            for (k, input_silent) in [false, true, true, false, true].into_iter().enumerate() {
                for n in [1usize, 2] {
                    let q = (k * QUANTUM) as u64;
                    let mut bus = if n == 1 {
                        mono_bus(input_silent)
                    } else {
                        stereo_bus(input_silent)
                    };
                    let mut want = bus.ch;
                    let want_silent = scale(&mut want[..n], g, input_silent);
                    stage.process(q, &mut bus, &StageInputs::NONE);
                    assert_eq!(bus.silent, want_silent, "g {g}, quantum {k}, {n} ch");
                    assert_eq!(
                        bits(&bus.ch[..n]),
                        bits(&want[..n]),
                        "g {g}, quantum {k}, {n} ch"
                    );
                    assert_eq!(bus.channels, n);
                }
            }
        }
        // -0 is kept: a negative unity gain flips the sign of 0.
        let mut stage = Stage::Gain(GainNode::constant(-1.0));
        let mut bus = VoiceBus::mono();
        bus.silent = false;
        stage.process(0, &mut bus, &StageInputs::NONE);
        assert_eq!(bus.ch[0][0].to_bits(), (-0.0f32).to_bits());
        // `set_value` would make the first quantum sample accurate: a 0 gain is then not silent.
        let mut evented = GainNode::new(Param::gain(1.0));
        evented.gain.set_value(0.0, 0).unwrap();
        let mut bus = mono_bus(false);
        assert!(!evented.process(0, bus.active(), false, &ParamInputs::NONE));
    }

    /// Every [`NodeName`] variant, so a new one fails to compile until the tests list it.
    pub(crate) fn listed(n: &NodeName) -> bool {
        match n {
            NodeName::Source
            | NodeName::PwLfo
            | NodeName::Vib
            | NodeName::VibGain
            | NodeName::Fm(_)
            | NodeName::FmGain(_)
            | NodeName::Transient
            | NodeName::Gain
            | NodeName::Lpf
            | NodeName::LpfLfo
            | NodeName::Hpf
            | NodeName::HpfLfo
            | NodeName::Bpf
            | NodeName::BpfLfo
            | NodeName::Vowel
            | NodeName::Coarse
            | NodeName::Crush
            | NodeName::Shape
            | NodeName::Distort
            | NodeName::Tremolo
            | NodeName::TremoloGain
            | NodeName::Compressor
            | NodeName::Pan
            | NodeName::Phaser
            | NodeName::PhaserLfo
            | NodeName::Delay
            | NodeName::DelayMix
            | NodeName::Room
            | NodeName::RoomMix
            | NodeName::Post
            | NodeName::Djf
            | NodeName::Mod(_) => true,
        }
    }

    /// The static variants (every one but `Fm`, `FmGain` and `Mod`).
    pub(crate) const STATIC_NAMES: [NodeName; 29] = [
        NodeName::Source,
        NodeName::PwLfo,
        NodeName::Vib,
        NodeName::VibGain,
        NodeName::Transient,
        NodeName::Gain,
        NodeName::Lpf,
        NodeName::LpfLfo,
        NodeName::Hpf,
        NodeName::HpfLfo,
        NodeName::Bpf,
        NodeName::BpfLfo,
        NodeName::Vowel,
        NodeName::Coarse,
        NodeName::Crush,
        NodeName::Shape,
        NodeName::Distort,
        NodeName::Tremolo,
        NodeName::TremoloGain,
        NodeName::Compressor,
        NodeName::Pan,
        NodeName::Phaser,
        NodeName::PhaserLfo,
        NodeName::Delay,
        NodeName::DelayMix,
        NodeName::Room,
        NodeName::RoomMix,
        NodeName::Post,
        NodeName::Djf,
    ];

    #[test]
    fn node_names_are_superdough_keys() {
        let texts = [
            "source",
            "pw_lfo",
            "vib",
            "vib_gain",
            "transient",
            "gain",
            "lpf",
            "lpf_lfo",
            "hpf",
            "hpf_lfo",
            "bpf",
            "bpf_lfo",
            "vowel",
            "coarse",
            "crush",
            "shape",
            "distort",
            "tremolo",
            "tremolo_gain",
            "compressor",
            "pan",
            "phaser",
            "phaser_lfo",
            "delay",
            "delay_mix",
            "room",
            "room_mix",
            "post",
            "djf",
        ];
        for (name, text) in STATIC_NAMES.iter().zip(texts) {
            assert!(listed(name));
            assert_eq!(name.as_str(), text, "{name:?}");
        }
        for n in 1..=8usize {
            assert_eq!(NodeName::fm(n).as_str(), format!("fm_{n}"));
            assert_eq!(NodeName::fm_gain(n).as_str(), format!("fm_{n}_gain"));
        }
        assert_eq!(NodeName::Mod("lfo_x".into()).as_str(), "lfo_x");
        assert_eq!(
            NodeName::filter(FilterKind::Lowpass),
            (NodeName::Lpf, NodeName::LpfLfo)
        );
        assert_eq!(
            NodeName::filter(FilterKind::Highpass),
            (NodeName::Hpf, NodeName::HpfLfo)
        );
        assert_eq!(
            NodeName::filter(FilterKind::Bandpass),
            (NodeName::Bpf, NodeName::BpfLfo)
        );
    }

    #[test]
    fn node_table_set_replaces_push_appends_and_a_missing_key_is_none() {
        let mut t = NodeTable::empty(SourceKind::Osc);
        assert_eq!(t.get(FxKey::Main, "lpf"), None);
        t.set(
            FxKey::Main,
            NodeName::Lpf,
            NodeList::Array(vec![Some(Node::Stage(StageId(3)))]),
        );
        t.push_element(FxKey::Main, NodeName::Lpf, Some(Node::Stage(StageId(4))));
        assert_eq!(
            t.get(FxKey::Main, "lpf"),
            Some(&NodeList::Array(vec![
                Some(Node::Stage(StageId(3))),
                Some(Node::Stage(StageId(4)))
            ]))
        );
        // The same name under another FX entry is a different entry.
        assert_eq!(t.get(FxKey::Index(0), "lpf"), None);
        t.set(
            FxKey::Index(0),
            NodeName::Lpf,
            NodeList::Array(vec![Some(Node::Stage(StageId(9)))]),
        );
        assert_eq!(
            t.get(FxKey::Main, "lpf"),
            Some(&NodeList::Array(vec![
                Some(Node::Stage(StageId(3))),
                Some(Node::Stage(StageId(4)))
            ]))
        );
        // `set` replaces, whatever was there.
        t.set(
            FxKey::Main,
            NodeName::Lpf,
            NodeList::NotArray(Node::Detached { value: 0.5 }),
        );
        assert_eq!(
            t.get(FxKey::Main, "lpf"),
            Some(&NodeList::NotArray(Node::Detached { value: 0.5 }))
        );
        // A push onto a bare node makes an array holding it first.
        t.push_element(FxKey::Main, NodeName::Lpf, Some(Node::FmAmount(1)));
        assert_eq!(
            t.get(FxKey::Main, "lpf"),
            Some(&NodeList::Array(vec![
                Some(Node::Detached { value: 0.5 }),
                Some(Node::FmAmount(1))
            ]))
        );
        // A push onto a missing entry starts an array.
        t.push_element(
            FxKey::Index(2),
            NodeName::Hpf,
            Some(Node::Stage(StageId(1))),
        );
        assert_eq!(
            t.get(FxKey::Index(2), "hpf"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(1)))]))
        );
        // An `undefined` element (`lpf_lfo` without a filter LFO) is kept as `None`.
        t.set(FxKey::Main, NodeName::LpfLfo, NodeList::Array(vec![None]));
        assert_eq!(
            t.get(FxKey::Main, "lpf_lfo"),
            Some(&NodeList::Array(vec![None]))
        );
        t.push_element(FxKey::Main, NodeName::LpfLfo, Some(Node::Stage(StageId(0))));
        assert_eq!(
            t.get(FxKey::Main, "lpf_lfo"),
            Some(&NodeList::Array(vec![None, Some(Node::Stage(StageId(0)))]))
        );
        // Names are exact.
        assert_eq!(t.get(FxKey::Main, "LPF"), None);
        assert_eq!(t.get(FxKey::Main, ""), None);
        // `Object.keys` order: first insertion, a later `set` keeps the key's place.
        assert_eq!(t.keys(FxKey::Main), ["lpf", "lpf_lfo"]);
        t.set(
            FxKey::Main,
            NodeName::Gain,
            NodeList::NotArray(Node::Source),
        );
        t.set(FxKey::Main, NodeName::Lpf, NodeList::Array(vec![None]));
        assert_eq!(t.keys(FxKey::Main), ["lpf", "lpf_lfo", "gain"]);
        assert_eq!(t.keys(FxKey::Index(2)), ["hpf"]);
        assert!(t.has_entry(FxKey::Main) && t.has_entry(FxKey::Index(2)));
        assert!(!t.has_entry(FxKey::Index(1)));
    }

    fn voice(v: serde_json::Value) -> Voice {
        let params = parsed(v, 0.1);
        let mut orbit = Orbit::new(
            &OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1],
            },
            2,
        );
        Voice::new(
            &params,
            0,
            StartInfo {
                t: 0.0,
                hap_duration: 0.1,
                now_frame: 0,
                cycle: 0.0,
                phase_sec: 0.0,
                cps: 2.0,
            },
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut orbit,
        )
        .unwrap()
    }

    /// `v` parsed as a hap of `dur` seconds at 0.5 cps.
    pub(crate) fn parsed(v: serde_json::Value, dur: f64) -> VoiceParams {
        let map: ValueMap = serde_json::from_value(v).unwrap();
        parse_voice_params(&map, dur, 0.5).unwrap()
    }

    /// The plan of `v`, parsed by [`parsed`].
    pub(crate) fn planned(v: serde_json::Value, dur: f64) -> NodeTable {
        let params = parsed(v, dur);
        let s = crate::voice::sound_name(params.s.as_ref());
        NodeTable::plan(&params, SourceKind::of(&s).unwrap())
    }

    const M: FxKey = FxKey::Main;
    const X0: FxKey = FxKey::Index(0);

    /// A planned filter copy of the entry `entry`.
    fn filter(entry: FxKey, kind: FilterKind, ladder: bool) -> S {
        S::Filter {
            entry,
            kind,
            ladder,
        }
    }

    fn entry(t: &NodeTable, name: &str) -> Option<NodeList> {
        t.get(FxKey::Main, name).cloned()
    }

    fn one(id: usize) -> Option<NodeList> {
        Some(NodeList::Array(vec![Some(Node::Stage(StageId(id)))]))
    }

    #[test]
    fn fm_gain_node_table_last_writer() {
        // Matrix order: fmi (1 -> 0), fmi13 (1 -> 3), fmi2 (2 -> 1). Connections 0, 1, 2.
        let v = voice(json!({ "s": "sine", "fmi": 1, "fmi13": 3, "fmi2": 2 }));
        let fm = |i: usize| entry(v.node_table(), &format!("fm_{i}_gain"));
        // fm_1: source of fmi, source of fmi13, then the target side of fmi2 (Gain(2)).
        assert_eq!(
            fm(1),
            Some(NodeList::Array(vec![Some(Node::Detached { value: 2.0 })]))
        );
        // fm_2: the source side of fmi2, connection 2.
        assert_eq!(fm(2), Some(NodeList::Array(vec![Some(Node::FmAmount(2))])));
        // fm_3: the target side of fmi13.
        assert_eq!(
            fm(3),
            Some(NodeList::Array(vec![Some(Node::Detached { value: 3.0 })]))
        );
        assert_eq!(fm(4), None);
        assert_eq!(fm(0), None);

        // The source side is the last writer when no later entry touches the modulator.
        let v = voice(json!({ "s": "sine", "fmi": 1.5, "fmi2": 2 }));
        assert_eq!(
            entry(v.node_table(), "fm_1_gain"),
            Some(NodeList::Array(vec![Some(Node::Detached { value: 2.0 })]))
        );
        let v = voice(json!({ "s": "sine", "fmi": 1.5 }));
        assert_eq!(
            entry(v.node_table(), "fm_1_gain"),
            Some(NodeList::Array(vec![Some(Node::FmAmount(0))]))
        );

        // fmi13 into a noise modulator is skipped, but both sides are still written: the source
        // side's Gain(3) feeds nothing audible either.
        let v = voice(json!({ "s": "sine", "fmi": 1, "fmi13": 3, "fmwave3": "white" }));
        assert_eq!(
            entry(v.node_table(), "fm_1_gain"),
            Some(NodeList::Array(vec![Some(Node::Detached { value: 3.0 })]))
        );
        assert_eq!(
            entry(v.node_table(), "fm_3_gain"),
            Some(NodeList::Array(vec![Some(Node::Detached { value: 3.0 })]))
        );
        // A connection after a skipped one gets the next connection index.
        let v = voice(json!({ "s": "sine", "fmi13": 3, "fmwave3": "white", "fmi2": 2 }));
        assert_eq!(
            entry(v.node_table(), "fm_2_gain"),
            Some(NodeList::Array(vec![Some(Node::FmAmount(0))]))
        );
        // No FM: no entry.
        let v = voice(json!({ "s": "sine" }));
        assert_eq!(entry(v.node_table(), "fm_1_gain"), None);
    }

    #[test]
    fn fm_steps_follow_the_matrix_walk() {
        let side = |idx, pos, first_use| {
            Some(FmSide {
                idx,
                pos,
                first_use,
            })
        };
        let step = |sides, connects| FmStep { sides, connects };
        let plan = |v: serde_json::Value, source: SourceKind| {
            let p = parsed(v, 0.1);
            let t = NodeTable::plan(&p, source);
            (t.fm_steps().to_vec(), p.fm.entries.len())
        };
        let synths = [SourceKind::Osc, SourceKind::Supersaw, SourceKind::Pulse];
        let others = [SourceKind::Bus, SourceKind::Constant, SourceKind::Noise];
        let cases = [
            // Matrix order: fmi (1 -> 0), fmi13 (1 -> 3), fmi2 (2 -> 1).
            (
                json!({ "s": "sine", "fmi": 1, "fmi13": 3, "fmi2": 2 }),
                vec![
                    step([side(1, 0, true), None], true),
                    step([side(1, 0, false), side(3, 1, true)], true),
                    step([side(2, 2, true), side(1, 0, false)], true),
                ],
            ),
            // A noise target: the entry does not connect.
            (
                json!({ "s": "sine", "fmi": 1, "fmi13": 3, "fmwave3": "white" }),
                vec![
                    step([side(1, 0, true), None], true),
                    step([side(1, 0, false), side(3, 1, true)], false),
                ],
            ),
            // The connection after a skipped one is connection 0.
            (
                json!({ "s": "sine", "fmi13": 3, "fmwave3": "white", "fmi2": 2 }),
                vec![
                    step([side(1, 0, true), side(3, 1, true)], false),
                    step([side(2, 2, true), side(1, 0, false)], true),
                ],
            ),
            (json!({ "s": "sine" }), vec![]),
        ];
        for (v, expected) in cases {
            for source in synths {
                let (steps, entries) = plan(v.clone(), source);
                // One step per matrix entry: `apply_fm`'s zip drops none.
                assert_eq!(steps.len(), entries, "{source:?} {v}");
                assert_eq!(steps, expected, "{source:?} {v}");
            }
            for source in others {
                assert_eq!(plan(v.clone(), source).0, vec![], "{source:?} {v}");
            }
        }
    }

    #[test]
    fn main_entry_registers_gain_filters_pan_post() {
        let value = json!({
            "s": "saw", "cutoff": 500, "ftype": "24db", "hcutoff": 100, "pan": 0.2
        });
        let t = planned(value.clone(), 0.1);
        let lp = filter(M, FilterKind::Lowpass, false);
        let hp = filter(M, FilterKind::Highpass, false);
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::EntryGain(M),
                lp,
                lp,
                hp,
                hp,
                S::Pan(M),
                S::Post
            ]
        );
        assert_eq!(entry(&t, "gain"), one(2));
        assert_eq!(
            entry(&t, "lpf"),
            Some(NodeList::Array(vec![
                Some(Node::Stage(StageId(3))),
                Some(Node::Stage(StageId(4)))
            ]))
        );
        assert_eq!(
            entry(&t, "hpf"),
            Some(NodeList::Array(vec![
                Some(Node::Stage(StageId(5))),
                Some(Node::Stage(StageId(6)))
            ]))
        );
        assert_eq!(entry(&t, "bpf"), None);
        assert_eq!(entry(&t, "pan"), one(7));
        assert_eq!(entry(&t, "post"), one(8));
        assert_eq!(voice(value).output_channels(), 2);

        // 12 dB band-pass only, no pan: no `pan` entry, a mono voice.
        let value = json!({ "s": "saw", "bandf": 900, "dry": 0.5, "room": 0.3 });
        let t = planned(value.clone(), 0.1);
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::EntryGain(M),
                filter(M, FilterKind::Bandpass, false),
                S::Post
            ]
        );
        assert_eq!(entry(&t, "gain"), one(2));
        assert_eq!(entry(&t, "bpf"), one(3));
        assert_eq!(entry(&t, "lpf"), None);
        assert_eq!(entry(&t, "pan"), None);
        assert_eq!(entry(&t, "post"), one(4));
        assert_eq!(voice(value).output_channels(), 1);
        // `dry`, `bus` and `bmod` are never registered; the sends are (`room` is the orbit's
        // convolver, `room_mix` the voice's send), as is the source.
        for name in ["dry", "bus", "bmod", "delay", "delay_mix", "djf"] {
            assert_eq!(entry(&t, name), None, "{name}");
        }
        assert_eq!(
            entry(&t, "room"),
            Some(NodeList::Array(vec![Some(Node::OrbitReverb)]))
        );
        assert_eq!(
            entry(&t, "room_mix"),
            Some(NodeList::Array(vec![Some(Node::RoomSend)]))
        );
        assert_eq!(
            entry(&t, "source"),
            Some(NodeList::Array(vec![Some(Node::Source)]))
        );
        assert_eq!(t.get(FxKey::Index(0), "gain"), None);
        assert_eq!(
            t.keys(FxKey::Main),
            [
                "source", "gain", "bpf", "bpf_lfo", "post", "room", "room_mix"
            ]
        );
    }

    #[test]
    fn worklets_are_built_in_superdough_order_and_registered() {
        let value = json!({
            "s": "saw", "transient": 0.5, "cutoff": 500, "coarse": 4, "crush": 8,
            "shape": 0.3, "distort": 1, "pan": 0.2
        });
        let t = planned(value.clone(), 0.1);
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::Transient(M),
                S::EntryGain(M),
                filter(M, FilterKind::Lowpass, false),
                S::Coarse(M),
                S::Crush(M),
                S::Shape(M),
                S::Distort(M),
                S::Pan(M),
                S::Post
            ]
        );
        assert_eq!(
            entry(&t, "transient"),
            Some(NodeList::NotArray(Node::Stage(StageId(2)))),
            "transient is a bare node"
        );
        assert_eq!(entry(&t, "gain"), one(3));
        assert_eq!(entry(&t, "lpf"), one(4));
        assert_eq!(entry(&t, "coarse"), one(5));
        assert_eq!(entry(&t, "crush"), one(6));
        assert_eq!(entry(&t, "shape"), one(7));
        assert_eq!(entry(&t, "distort"), one(8));
        assert_eq!(entry(&t, "pan"), one(9));
        assert_eq!(entry(&t, "post"), one(10));
        assert_eq!(voice(value).output_channels(), 2);
        // Only defined controls build a worklet; `null` counts as defined.
        let value = json!({ "s": "sine", "crush": null });
        let t = planned(value.clone(), 0.1);
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::EntryGain(M),
                S::Crush(M),
                S::Post
            ]
        );
        assert_eq!(entry(&t, "coarse"), None);
        assert_eq!(entry(&t, "transient"), None);
        let v = voice(value);
        assert_eq!(
            v.output_channels(),
            1,
            "the worklets keep the channel count"
        );
        match &v.stages()[3] {
            Stage::Shaper(Shaper {
                kind: ShaperKind::Crush(c),
                ..
            }) => assert_eq!(c.crush.value(), 0.0, "`.value = null` is 0"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn worklet_param_values_and_defaults() {
        let v = voice(json!({
            "s": "sine", "shape": 0.6, "shapevol": 0.5, "distort": "2", "distortvol": null
        }));
        let (mut shape, mut distort) = (None, None);
        for s in v.stages() {
            if let Stage::Shaper(sh) = s {
                match &sh.kind {
                    ShaperKind::Shape(x) => shape = Some(x.clone()),
                    ShaperKind::Distort(x) => distort = Some(x.clone()),
                    _ => {}
                }
            }
        }
        let shape = shape.unwrap();
        assert_eq!((shape.shape.value(), shape.postgain.value()), (0.6, 0.5));
        assert_eq!(
            shape.shape.events().len(),
            1,
            "`.value =` schedules the value"
        );
        let distort = distort.unwrap();
        assert_eq!(distort.distort.value(), 2.0, "a numeric string converts");
        assert_eq!(distort.postgain.value(), 0.0, "a null distortvol is 0");
        // Absent shapevol / distortvol: the default 1 is still set.
        let v = voice(json!({ "s": "sine", "shape": 0.2, "distort": 1 }));
        for s in v.stages() {
            if let Stage::Shaper(sh) = s {
                match &sh.kind {
                    ShaperKind::Shape(x) => assert_eq!(x.postgain.value(), 1.0),
                    ShaperKind::Distort(x) => assert_eq!(x.postgain.value(), 1.0),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn worklet_value_throw_sites() {
        let err = |v: serde_json::Value| {
            let params = parsed(v, 0.1);
            let (stages, _, _, r) = walk_entry(&params, TIMES);
            let e = r.unwrap_err();
            (e, stages.len())
        };
        // Each `.value =` of a non-finite (or float-overflowing) value throws a TypeError, after
        // the stages before it were built.
        for (v, built) in [
            (json!({ "coarse": "x" }), 1),
            (json!({ "crush": 1e39 }), 1),
            (json!({ "coarse": 2, "shape": { "$num": "NaN" } }), 2),
            (
                json!({ "shape": 0.5, "shapevol": { "$num": "Infinity" } }),
                1,
            ),
            (json!({ "distort": "abc" }), 1),
            (json!({ "distort": 1, "distortvol": 1e300 }), 1),
            (json!({ "transient": 0.5, "coarse": [1, 2] }), 2),
        ] {
            let (e, n) = err(v.clone());
            assert_eq!(e.kind, crate::job::ErrorKind::Hap, "{v}");
            assert!(
                e.detail.contains("TypeError") && e.detail.contains("value"),
                "{v}: {}",
                e.detail
            );
            assert_eq!(n, built, "{v}");
        }
        // The transient's options never throw: NaN amounts are kept.
        let params = parsed(json!({ "transient": "x", "transsustain": null }), 0.1);
        let (stages, _, _, r) = walk_entry(&params, TIMES);
        r.unwrap();
        assert!(matches!(stages[0], Stage::Transient(_)));
    }

    #[test]
    fn worklet_stages_report_their_processor_errors_once() {
        let mut stage = Stage::Shaper(Shaper::new(ShaperKind::Distort(Box::new(Distort::new(
            &Value::Num(1.5),
        )))));
        let mut bus = mono_bus(false);
        stage.process(0, &mut bus, &StageInputs::NONE);
        assert!(!bus.silent);
        assert!(bus.ch[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(stage.take_error().map(|e| e.processor), Some("distort"));
        assert_eq!(stage.take_error(), None);
        let mut bus = mono_bus(false);
        stage.process(128, &mut bus, &StageInputs::NONE);
        assert!(bus.silent, "a finished worklet outputs a silent bus");
        // Other stages never report one.
        let mut gain = Stage::Gain(GainNode::constant(1.0));
        let mut bus = mono_bus(false);
        gain.process(0, &mut bus, &StageInputs::NONE);
        assert_eq!(gain.take_error(), None);
        // A live worklet is never silent, even on a silent zero input.
        let mut crush = Stage::Shaper(Shaper::new(ShaperKind::Crush(Crush::new())));
        let mut bus = VoiceBus::mono();
        crush.process(0, &mut bus, &StageInputs::NONE);
        assert!(!bus.silent);
        assert_eq!(crush.channels_out(2), 2);
        let mut bus = stereo_bus(true);
        bus.ch = [[0.0; QUANTUM]; 2];
        crush.process(128, &mut bus, &StageInputs::NONE);
        assert!(!bus.silent);
        assert_eq!(bus.channels, 2);
    }

    #[test]
    fn the_transient_stage_runs_from_its_begin_to_its_end() {
        // begin 0.0 (quantum 0 gated), end_with_release 0.11 s (frame 4851), input gone at 5376.
        let params = parsed(json!({ "transient": 0.5 }), 0.1);
        let (mut stages, _, _, r) = walk_entry(&params, TIMES);
        r.unwrap();
        let mut t = stages.remove(0);
        // The source stops at frame 5292: the input is gone from 5376.
        let gone = SourceEnd::Scheduled(Some(5292)).disabled_from();
        assert_eq!(gone, Some(5376));
        let mut run = |q: u64| {
            let mut bus = mono_bus(false);
            t.process(q, &mut bus, &StageInputs::input(input_connected(gone, q)));
            (bus.silent, bus.ch[0].iter().any(|x| *x != 0.0))
        };
        assert_eq!(run(0), (false, false), "ct == begin");
        assert_eq!(run(128), (false, true));
        assert_eq!(run(4736), (false, true), "ct 0.1074 < end");
        assert_eq!(run(4864), (false, false), "ct >= end: zeros, still alive");
        assert_eq!(run(5248), (false, false));
        assert_eq!(run(5376), (true, false), "input gone: finished");
    }

    /// Runs the teardown walk over `stages` for the quanta `0, 128, ..` below `until`, processing
    /// the biquads (signal while their input is connected, silence after); returns per stage the
    /// first quantum start at which its input has no connection.
    fn walk(stages: &mut [Stage], source: SourceEnd, until: u64) -> Vec<Option<u64>> {
        let mut td = Teardown::new(source, stages.len());
        let mut first = vec![None; stages.len()];
        let mut q = 0;
        while q < until {
            let mut before = td.source();
            for k in 0..stages.len() {
                let connected = input_connected(before, q);
                if !connected && first[k].is_none() {
                    first[k] = Some(q);
                }
                if let Stage::Biquad(_) = stages[k] {
                    let mut bus = if connected {
                        mono_bus(false)
                    } else {
                        VoiceBus::mono()
                    };
                    stages[k].process(q, &mut bus, &StageInputs::input(connected));
                }
                before = td.after(k, &stages[k], before, q);
            }
            q += 128;
        }
        first
    }

    fn ladder_stage() -> Stage {
        Stage::Ladder(Box::new(LadderNode::new(Ladder::new())))
    }

    fn transient_stage(end: f64) -> Stage {
        Stage::Transient(Box::new(TransientNode::new(Transient::new(
            Some(0.5),
            None,
            0.0,
            end,
        ))))
    }

    #[test]
    fn teardown_source_rows() {
        assert_eq!(SourceEnd::Scheduled(None).disabled_from(), None);
        // The measured rule: stop 13671 -> 13696; a stop on a quantum boundary: that boundary.
        assert_eq!(
            SourceEnd::Scheduled(Some(13671)).disabled_from(),
            Some(13696)
        );
        assert_eq!(
            SourceEnd::Scheduled(Some(13696)).disabled_from(),
            Some(13696)
        );
        assert_eq!(SourceEnd::Scheduled(Some(0)).disabled_from(), Some(0));
        // supersaw `end` 0.3f32 (13230.0005 frames): buffer frame 13231, one quantum after the
        // quantum holding it.
        assert_eq!(
            crate::render::frame_for_buffer_start(f64::from(0.3f32)),
            13231
        );
        assert_eq!(SourceEnd::Supersaw(0.3).disabled_from(), Some(13312));
        // pulse: returns false at the first quantum reaching `end` (13312), disabled after it.
        assert_eq!(SourceEnd::Pulse(0.3).disabled_from(), Some(13440));
        assert_eq!(SourceEnd::Pulse(0.0).disabled_from(), Some(128));
        assert_eq!(SourceEnd::Pulse(f32::NAN).disabled_from(), None);
        assert_eq!(SourceEnd::Pulse(f32::INFINITY).disabled_from(), None);
        // Never overflows.
        assert_eq!(
            SourceEnd::Supersaw(f32::MAX).disabled_from(),
            Some(u64::MAX)
        );
        assert_eq!(
            SourceEnd::Scheduled(Some(u64::MAX)).disabled_from(),
            Some(u64::MAX)
        );
    }

    #[test]
    fn teardown_gains_and_the_panner_pass_the_loss_on() {
        let mut stages = vec![
            Stage::Gain(GainNode::constant(0.3)),
            Stage::Panner(StereoPanner::new()),
            Stage::Gain(GainNode::constant(1.0)),
        ];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 8192);
        assert_eq!(first, vec![Some(5376); 3]);
        // No stop: never.
        let first = walk(&mut stages, SourceEnd::Scheduled(None), 8192);
        assert_eq!(first, vec![None; 3]);
    }

    #[test]
    fn teardown_worklet_effects_add_one_quantum_each() {
        // Chained ladders (an lpf then an hpf slot): the second loses its input 128 frames later
        // than the first; a shaper after them one more.
        let mut stages = vec![
            Stage::Gain(GainNode::constant(0.3)),
            ladder_stage(),
            ladder_stage(),
            Stage::Shaper(Shaper::new(ShaperKind::Crush(Crush::new()))),
            Stage::Gain(GainNode::constant(1.0)),
        ];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 8192);
        assert_eq!(
            first,
            vec![Some(5376), Some(5376), Some(5504), Some(5632), Some(5760)]
        );
        // A ladder after a supersaw, with a stub source.
        let mut stages = vec![Stage::Gain(GainNode::constant(1.0)), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Supersaw(0.3), 16384);
        assert_eq!(first, vec![Some(13312), Some(13312)]);
        let first = walk(&mut stages, SourceEnd::Pulse(0.3), 16384);
        assert_eq!(first, vec![Some(13440), Some(13440)]);
    }

    #[test]
    fn teardown_transient_row() {
        // end 0.11 s (end quantum 4864) before the input goes (5376): the ladder after it loses
        // its input at 5376 + 128.
        let mut stages = vec![transient_stage(0.11), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 8192);
        assert_eq!(first, vec![Some(5376), Some(5504)]);
        // end 0.2 s (frame 8820, end quantum 8832) after it: 8832 + 128.
        let mut stages = vec![transient_stage(0.2), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 16384);
        assert_eq!(first, vec![Some(5376), Some(8960)]);
        // An end no quantum reaches: never finishes, its output is never disabled.
        let mut stages = vec![transient_stage(f64::NAN), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 16384);
        assert_eq!(first, vec![Some(5376), None]);
        // No stop upstream: None.
        let mut stages = vec![transient_stage(0.11), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(None), 8192);
        assert_eq!(first, vec![None, None]);
    }

    #[test]
    fn teardown_biquad_row_is_dynamic() {
        // A resonant lowpass before a ladder (reachable through FX chains only): the ladder's
        // input goes at the first quantum q >= 5376 at which the biquad propagates silence.
        let mut stages = vec![Stage::Biquad(lowpass()), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(Some(5292)), 1 << 16);
        assert_eq!(first[0], Some(5376));
        let d = first[1].expect("the tail ends within the walk");
        assert!(d > 5376 && d.is_multiple_of(128), "{d}");
        let Stage::Biquad(b) = &stages[0] else {
            unreachable!()
        };
        assert!(b.propagates_silence(d));
        assert!(!b.propagates_silence(d - 128), "the first such quantum");
        // Never resolved without a stop.
        let mut stages = vec![Stage::Biquad(lowpass()), ladder_stage()];
        let first = walk(&mut stages, SourceEnd::Scheduled(None), 1 << 16);
        assert_eq!(first, vec![None, None]);
    }

    #[test]
    fn ladder_slots_replace_the_biquads_and_register() {
        let value = json!({
            "s": "saw", "cutoff": 900, "resonance": 8, "hcutoff": 100, "bandf": 500,
            "ftype": "ladder", "drive": 3, "pan": 0.1
        });
        let t = planned(value.clone(), 0.1);
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::EntryGain(M),
                filter(M, FilterKind::Lowpass, true),
                filter(M, FilterKind::Highpass, true),
                filter(M, FilterKind::Bandpass, true),
                S::Pan(M),
                S::Post
            ]
        );
        assert_eq!(entry(&t, "lpf"), one(3));
        assert_eq!(entry(&t, "hpf"), one(4));
        assert_eq!(entry(&t, "bpf"), one(5));
        let v = voice(value);
        assert_eq!(v.output_channels(), 2);
        let ladders: Vec<&Ladder> = v
            .stages()
            .iter()
            .filter_map(|s| match s {
                Stage::Ladder(l) => Some(&l.proc),
                _ => None,
            })
            .collect();
        // frequency from the slot's cutoff, q from its resonance key (default 1), drive shared.
        let got: Vec<(f32, f32, f32)> = ladders
            .iter()
            .map(|l| (l.frequency.value(), l.q.value(), l.drive.value()))
            .collect();
        assert_eq!(
            got,
            vec![(900.0, 8.0, 3.0), (100.0, 1.0, 3.0), (500.0, 1.0, 3.0)]
        );
        for l in &ladders {
            for p in [&l.frequency, &l.q, &l.drive] {
                assert_eq!(p.events().len(), 1, "`.value =` is one event");
            }
        }
        // Defaults: drive 0.69; the 12 dB numeric ftype 1 and '24db' stay biquads.
        let v = voice(json!({ "s": "saw", "cutoff": 900, "ftype": "ladder" }));
        match &v.stages()[3] {
            Stage::Ladder(l) => {
                assert_eq!(l.proc.drive.value(), 0.69);
                assert_eq!(l.proc.q.value(), 1.0);
            }
            other => panic!("{other:?}"),
        }
        let t = planned(json!({ "s": "saw", "cutoff": 900, "ftype": 1 }), 0.1);
        assert_eq!(t.specs()[3], filter(M, FilterKind::Lowpass, false));
        let v = voice(json!({ "s": "saw", "cutoff": 900, "ftype": "24db" }));
        assert_eq!(v.filters().len(), 2);
        // A null drive is `.value = null`: 0.
        let v = voice(json!({ "s": "saw", "cutoff": 900, "ftype": "ladder", "drive": null }));
        match &v.stages()[3] {
            Stage::Ladder(l) => assert_eq!(l.proc.drive.value(), 0.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ladder_envelope_events_and_throw_order() {
        // The filter envelope is on the ladder's frequency param: `.value =` then the ADSR.
        let v = voice(json!({
            "s": "saw", "cutoff": 300, "ftype": "ladder", "lpenv": 4, "lpattack": 0.05
        }));
        let Stage::Ladder(l) = &v.stages()[3] else {
            panic!("a ladder")
        };
        assert!(l.proc.frequency.events().len() > 2);
        let mut biquad = Biquad::new(FilterKind::Lowpass);
        biquad.frequency.set_value(300.0, 0).unwrap();
        let p = parsed(
            json!({
                "cutoff": 300, "lpenv": 4, "lpattack": 0.05
            }),
            0.1,
        );
        filter_envelope(
            &mut biquad.frequency,
            p.main.lowpass.as_ref().unwrap(),
            0.0,
            0.0,
            0.1,
            0,
        )
        .unwrap();
        assert_eq!(
            l.proc.frequency.events(),
            biquad.frequency.events(),
            "the same events as a biquad's"
        );
        // `.value =` order frequency, q, drive: the first non-finite one throws, after the gain
        // stage was built.
        let err = |v: serde_json::Value| {
            let params = parsed(v, 0.1);
            let (stages, table, _, r) = walk_entry(&params, TIMES);
            let e = r.unwrap_err();
            (
                e.detail,
                stages.len(),
                table.get(FxKey::Main, "lpf").is_some(),
            )
        };
        for v in [
            json!({ "ftype": "ladder", "cutoff": "x", "resonance": "y" }),
            json!({ "ftype": "ladder", "cutoff": 1e39 }),
            json!({ "ftype": "ladder", "cutoff": 500, "resonance": "x", "drive": "y" }),
            json!({ "ftype": "ladder", "cutoff": 500, "drive": { "$num": "Infinity" } }),
        ] {
            let (detail, built, lpf) = err(v.clone());
            assert!(
                detail.contains("TypeError") && detail.contains("value"),
                "{v}: {detail}"
            );
            assert_eq!((built, lpf), (1, true), "{v}: built, planned");
        }
        // A highpass ladder that throws leaves the lowpass ladder built.
        let (_, built, lpf) = err(json!({ "ftype": "ladder", "cutoff": 500, "hcutoff": "x" }));
        assert_eq!((built, lpf), (2, true));
    }

    #[test]
    fn a_ladder_voice_is_cut_one_quantum_after_its_source() {
        // q 40 (k = 5.2): the ring self-oscillates, so only the cut ends it. Stop 5292 -> 5376.
        let mut v = voice(json!({
            "s": "saw", "note": 45, "cutoff": 300, "resonance": 40, "ftype": "ladder"
        }));
        let mut out = VoiceOutput::new();
        let mut q = 0;
        while q < 5376 {
            assert!(
                v.process(q, &mut out, &mut crate::bus::BusBank::default()),
                "alive at {q}"
            );
            q += 128;
        }
        assert!(
            out.post[0].iter().any(|x| x.abs() > 1e-3),
            "the quantum before the cut rings"
        );
        assert!(
            !v.process(5376, &mut out, &mut crate::bus::BusBank::default()),
            "freed at the cut"
        );
        assert!(out.post[0].iter().all(|x| x.to_bits() == 0));
    }

    const TIMES: EntryTimes = EntryTimes {
        begin: 0.0,
        hold_end: 0.1,
        end_with_release: 0.11,
        now: 0,
        cycle: 0.0,
        phase_sec: 0.0,
        cps: 2.0,
    };

    /// Walks the main entry of `params`, planned alone (`NodeTable::entry_plan(FxKey::Main, ..)`),
    /// through `build_stage`: the stages built before the first throw, the table, the LFOs, and
    /// the throw.
    fn walk_entry(
        params: &VoiceParams,
        times: EntryTimes,
    ) -> (Vec<Stage>, NodeTable, Vec<ParamLfo>, Result<(), VoiceError>) {
        walk_table(NodeTable::entry_plan(FxKey::Main, params), params, times)
    }

    /// Walks the specs of `table` through `build_stage`, as [`walk_entry`] does.
    fn walk_table(
        table: NodeTable,
        params: &VoiceParams,
        times: EntryTimes,
    ) -> (Vec<Stage>, NodeTable, Vec<ParamLfo>, Result<(), VoiceError>) {
        let (mut stages, mut lfos) = (Vec::new(), Vec::new());
        let mut result = Ok(());
        for &spec in table.specs() {
            match build_stage(spec, StageId(stages.len()), params, &times, &mut lfos) {
                Ok(stage) => stages.push(stage),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        (stages, table, lfos, result)
    }

    #[test]
    fn the_walk_throws_at_the_first_planned_site() {
        let walk = |v: &serde_json::Value| {
            let params = parsed(v.clone(), 0.1);
            let (stages, table, _, r) = walk_entry(&params, TIMES);
            (stages.len(), table, r)
        };
        let base = json!({
            "transient": 0.5, "cutoff": 500, "vowel": "a", "coarse": 2, "crush": 4, "shape": 0.3,
            "distort": 0.5, "tremolo": 2, "compressor": -10, "pan": 0.3, "phaserrate": 2
        });
        let m = FxKey::Main;
        let (built, table, r) = walk(&base);
        assert_eq!(
            table.specs(),
            [
                StageSpec::Transient(m),
                StageSpec::EntryGain(m),
                StageSpec::Filter {
                    entry: m,
                    kind: FilterKind::Lowpass,
                    ladder: false
                },
                StageSpec::Vowel(m),
                StageSpec::Coarse(m),
                StageSpec::Crush(m),
                StageSpec::Shape(m),
                StageSpec::Distort(m),
                StageSpec::Tremolo(m),
                StageSpec::Compressor(m),
                StageSpec::Pan(m),
                StageSpec::Phaser(m),
            ]
        );
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(built, 12);
        for (key, value, text, before) in [
            ("velocity", json!("x"), "gain is not a finite", 1),
            ("resonance", json!("x"), "Q is not a finite", 2),
            // The filter LFO, after its biquad.
            ("lprate", json!("x"), "'value' on 'AudioParam'", 2),
            ("vowel", json!("zz"), "unknown vowel zz", 3),
            ("coarse", json!("x"), "value is not a finite", 4),
            ("shapevol", json!("x"), "value is not a finite", 6),
            ("distortvol", json!("x"), "value is not a finite", 7),
            ("tremolodepth", json!("x"), "'gain' on 'AudioParam'", 8),
            (
                "compressor",
                json!({ "$num": "NaN" }),
                "'value' on 'AudioParam'",
                9,
            ),
            ("pan", json!("x"), "pan is not a finite", 10),
            ("phaserrate", json!("x"), "'value' on 'AudioParam'", 11),
        ] {
            let mut v = base.clone();
            v[key] = value;
            let (built, _, r) = walk(&v);
            let e = r.expect_err(key);
            assert!(e.detail.contains(text), "{key}: {}", e.detail);
            assert_eq!(built, before, "{key}: built before the throw");
        }
        // The plan names the stages whatever the build does (the modulator loop never runs
        // after a build throw): the gain stage, then the lowpass that throws.
        let (built, table, r) = walk(&json!({ "cutoff": 100, "resonance": "x" }));
        assert!(r.is_err());
        assert_eq!(built, 1);
        assert_eq!(
            table.get(FxKey::Main, "gain"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(0)))]))
        );
        assert_eq!(
            table.get(FxKey::Main, "lpf"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(1)))]))
        );
    }

    #[test]
    fn entry_plan_registers_under_its_key() {
        // Entry 1 pans, the main entry does not: a walk of the main entry's specs would build
        // only its gain stage.
        let params = parsed(json!({ "FX": [{}, { "pan": 0.5 }] }), 0.1);
        let i = FxKey::Index(1);
        let (stages, table, _, r) = walk_table(NodeTable::entry_plan(i, &params), &params, TIMES);
        r.unwrap();
        assert_eq!(table.specs(), [StageSpec::EntryGain(i), StageSpec::Pan(i)]);
        assert_eq!(
            stages.iter().map(StageKind::of).collect::<Vec<_>>(),
            [StageKind::Gain, StageKind::Panner]
        );
        assert_eq!(table.get(FxKey::Main, "pan"), None);
        assert_eq!(
            table.get(FxKey::Index(1), "pan"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(1)))]))
        );
    }

    /// `walk_entry` on `v` (cps 2, cycle 1.5: `time = 0.75`, begin 0.25, hold end 0.5, end with
    /// release 0.51, scheduled at frame 128).
    fn built(v: serde_json::Value) -> Result<(Vec<Stage>, NodeTable, Vec<ParamLfo>), VoiceError> {
        let map: ValueMap = serde_json::from_value(v).unwrap();
        let params = parse_voice_params(&map, 0.25, 2.0).unwrap();
        let times = EntryTimes {
            begin: 0.25,
            hold_end: 0.5,
            end_with_release: 0.51,
            now: 128,
            cycle: 1.5,
            phase_sec: 0.75,
            cps: 2.0,
        };
        let (stages, table, lfos, r) = walk_entry(&params, times);
        r?;
        Ok((stages, table, lfos))
    }

    fn tremolo_of(stages: &[Stage]) -> &Tremolo {
        stages
            .iter()
            .find_map(|s| match s {
                Stage::Tremolo(t) => Some(&**t),
                _ => None,
            })
            .expect("a tremolo stage")
    }

    fn lfo_values(l: &Lfo) -> [f32; lfo::PARAMS] {
        std::array::from_fn(|k| l.params[k].value())
    }

    #[test]
    fn tremolo_condition_and_values() {
        let (stages, table, _) = built(json!({ "tremolo": 4 })).unwrap();
        let t = tremolo_of(&stages);
        // depth 1: amGain's constructor value max(1 - 1, 0) = 0, no event, default 1.
        assert_eq!(t.gain.gain.value(), 0.0);
        assert!(t.gain.gain.events().is_empty());
        assert_eq!(t.gain.gain.default_value(), 1.0);
        // begin t, time = cycle / cps, end endWithRelease, frequency, skew 1 (no shape), depth,
        // phaseoffset 0, shape 0, curve 1.5, dcoffset 0, min 0, max 1.
        assert_eq!(
            lfo_values(&t.lfo),
            [
                0.25, 0.75, 0.51, 4.0, 1.0, 1.0, 0.0, 0.0, 1.5, 0.0, 0.0, 1.0
            ]
        );
        assert!(t.lfo.params.iter().all(|p| p.events().len() == 1));
        assert_eq!(t.lfo.params[lfo::BEGIN].events()[0].time, 128.0 / 44100.0);
        let id = StageId(1);
        assert_eq!(table.specs()[1], S::Tremolo(M));
        assert_eq!(
            table.get(FxKey::Main, "tremolo"),
            Some(&NodeList::Array(vec![Some(Node::StageLfo(id))]))
        );
        assert_eq!(
            table.get(FxKey::Main, "tremolo_gain"),
            Some(&NodeList::Array(vec![Some(Node::Stage(id))]))
        );
        // `tremolosync` alone makes it, at cps * sync; it wins over `tremolo`.
        for v in [
            json!({ "tremolosync": 8 }),
            json!({ "tremolo": 3, "tremolosync": 8 }),
        ] {
            let (stages, _, _) = built(v.clone()).unwrap();
            assert_eq!(
                tremolo_of(&stages).lfo.params[lfo::FREQUENCY].value(),
                16.0,
                "{v}"
            );
        }
        // A null sync is not a sync; a null tremolo is defined (frequency 0).
        let (stages, _, _) = built(json!({ "tremolo": 3, "tremolosync": null })).unwrap();
        assert_eq!(tremolo_of(&stages).lfo.params[lfo::FREQUENCY].value(), 3.0);
        let (stages, _, _) = built(json!({ "tremolo": null })).unwrap();
        assert_eq!(tremolo_of(&stages).lfo.params[lfo::FREQUENCY].value(), 0.0);
        // No tremolo: none, whatever the other keys.
        let (_, table, _) = built(json!({ "tremolodepth": 0.5, "tremoloshape": "sine" })).unwrap();
        assert!(!table.specs().iter().any(|s| matches!(s, S::Tremolo(_))));
        assert_eq!(table.get(FxKey::Main, "tremolo"), None);
        // Depth: gain max(1 - d, 0), the LFO's depth d.
        for (d, gain) in [(0.25, 0.75f32), (1.5, 0.0), (-0.5, 1.5), (0.0, 1.0)] {
            let (stages, _, _) = built(json!({ "tremolo": 4, "tremolodepth": d })).unwrap();
            let t = tremolo_of(&stages);
            assert_eq!(t.gain.gain.value(), gain, "{d}");
            assert_eq!(t.lfo.params[lfo::DEPTH].value(), d as f32, "{d}");
        }
        // Shapes: names map, a number is `val % 5` as is; a shape makes skew 0.5.
        for (shape, want) in [
            (json!("square"), 4.0f32),
            (json!("triangle"), 0.0),
            (json!("nope"), 0.0),
            (json!(7), 2.0),
            (json!(6.5), 1.5),
            (json!(-1), -1.0),
        ] {
            let (stages, _, _) =
                built(json!({ "tremolo": 4, "tremoloshape": shape.clone() })).unwrap();
            let l = &tremolo_of(&stages).lfo;
            assert_eq!(l.params[lfo::SHAPE].value(), want, "{shape}");
            assert_eq!(l.params[lfo::SKEW].value(), 0.5, "{shape}");
        }
        // A null shape is no shape for the skew; tremoloskew wins; tremolophase is the offset.
        let (stages, _, _) = built(json!({ "tremolo": 4, "tremoloshape": null })).unwrap();
        assert_eq!(tremolo_of(&stages).lfo.params[lfo::SKEW].value(), 1.0);
        let (stages, _, _) = built(
            json!({ "tremolo": 4, "tremoloshape": "sine", "tremoloskew": 0.2, "tremolophase": 0.25 }),
        )
        .unwrap();
        let l = &tremolo_of(&stages).lfo;
        assert_eq!(l.params[lfo::SKEW].value(), 0.2);
        assert_eq!(l.params[lfo::PHASEOFFSET].value(), 0.25);
        // Built after the shapers, before the panner.
        let (_, table, _) = built(json!({ "tremolo": 4, "crush": 4, "pan": 0.3 })).unwrap();
        assert_eq!(
            table.specs(),
            [S::EntryGain(M), S::Crush(M), S::Tremolo(M), S::Pan(M)]
        );
    }

    #[test]
    fn tremolo_throws_stop_the_build() {
        // A non-finite gain throws in `new GainNode` (before the LFO), a non-finite LFO value
        // at its `.value =`; the stages before are kept.
        for v in [
            json!({ "crush": 1, "tremolo": 4, "tremolodepth": { "$num": "NaN" } }),
            json!({ "crush": 1, "tremolo": 4, "tremolodepth": { "$num": "-Infinity" } }),
            json!({ "crush": 1, "tremolo": 4, "tremolodepth": -1e39 }),
            json!({ "crush": 1, "tremolo": "x" }),
            json!({ "crush": 1, "tremolo": 4, "tremolodepth": 1e39 }),
            json!({ "crush": 1, "tremolo": 4, "tremolophase": { "$num": "Infinity" } }),
            json!({ "crush": 1, "tremolo": 4, "tremoloshape": { "$num": "Infinity" } }),
        ] {
            let map: ValueMap = serde_json::from_value(v.clone()).unwrap();
            let params = parse_voice_params(&map, 0.25, 2.0).unwrap();
            let (stages, table, _, r) = walk_entry(&params, TIMES);
            let e = r.unwrap_err();
            assert!(e.detail.contains("TypeError"), "{v}: {}", e.detail);
            assert_eq!(stages.len(), 2, "{v}: gain and crush built");
            assert!(table.get(FxKey::Main, "tremolo").is_some(), "planned");
        }
        // Huge finite depths build.
        for d in [1e30, -1e30] {
            assert!(
                built(json!({ "tremolo": 4, "tremolodepth": d })).is_ok(),
                "{d}"
            );
        }
    }

    /// Runs a tremolo stage over a constant 1.0 mono input from quantum 0, returning the
    /// output samples of quanta `0..n`.
    fn tremolo_run(v: serde_json::Value, n: u64) -> (Vec<f32>, Vec<bool>) {
        let (mut stages, _, _) = built(v).unwrap();
        let mut t = stages.remove(1);
        let (mut out, mut hints) = (Vec::new(), Vec::new());
        for k in 0..n {
            let mut bus = VoiceBus::mono();
            bus.ch[0] = [1.0; QUANTUM];
            bus.silent = false;
            t.process(k * 128, &mut bus, &StageInputs::NONE);
            out.extend_from_slice(&bus.ch[0]);
            hints.push(bus.silent);
        }
        (out, hints)
    }

    #[test]
    fn tremolo_gain_per_quantum() {
        // begin 0.25 s (frame 11025): the LFO first runs in the quantum starting at 11136.
        let first = 11136usize;
        assert!(worklet::started(worklet::current_time(first as u64), 0.25));
        assert!(!worklet::started(
            worklet::current_time(first as u64 - 128),
            0.25
        ));
        // depth 1: the intrinsic 0 alone before the LFO runs: zeros, but not a silent bus.
        let (out, hints) = tremolo_run(json!({ "tremolo": 4 }), 90);
        assert!(out[..first].iter().all(|x| x.to_bits() == 0));
        assert!(hints.iter().all(|h| !h));
        // From the first active quantum: (ramp(phase))^1.5 with phase = ffrac(0.75 * 4) = 0.
        let dt = 4.0 * worklet::INVSR;
        for n in [0usize, 1, 100] {
            let phase = n as f64 * dt;
            let want = jsmath::pow(phase, 1.5) as f32;
            assert_eq!(out[first + n].to_bits(), want.to_bits(), "frame {n}");
        }
        // Negative depth: intrinsic 1.5; pow(-0, 1.5) = 0 at phase 0 keeps 1.5, then NaN sums
        // are replaced by the default 1.
        let (out, _) = tremolo_run(json!({ "tremolo": 4, "tremolodepth": -0.5 }), 90);
        assert!(out[..first].iter().all(|x| *x == 1.5));
        assert_eq!(out[first], 1.5);
        assert!(out[first + 1..].iter().all(|x| *x == 1.0));
        // depth 0.5: 0.5 + clamp((0.5 * ramp)^1.5, 0, 1).
        let (out, _) = tremolo_run(json!({ "tremolo": 4, "tremolodepth": 0.5 }), 90);
        assert_eq!(out[0], 0.5);
        let want = 0.5f32 + jsmath::pow(0.5 * (10.0 * dt), 1.5) as f32;
        assert_eq!(out[first + 10], want);
        // tremolophase 0.25 and a shape: the start phase is ffrac(0.75 * 4 + 0.25) = 0.25.
        let (out, _) = tremolo_run(
            json!({ "tremolo": 4, "tremoloshape": "ramp", "tremolophase": 0.25 }),
            90,
        );
        assert_eq!(out[first], jsmath::pow(0.25, 1.5) as f32);
    }

    #[test]
    fn tremolo_pulls_its_lfo_on_a_silent_input_and_reports_shape_errors() {
        let (mut stages, _, _) = built(json!({ "tremolo": 4, "tremolodepth": 0.5 })).unwrap();
        let mut t = stages.remove(1);
        for k in 0..100u64 {
            let mut bus = VoiceBus::mono();
            t.process(k * 128, &mut bus, &StageInputs::NONE);
            assert!(bus.silent, "silent input, silent output");
            assert!(bus.ch[0].iter().all(|x| x.to_bits() == 0));
        }
        let Stage::Tremolo(tr) = &t else {
            unreachable!()
        };
        assert!(
            tr.lfo.proc.phase().is_some(),
            "the LFO ran while the input was silent"
        );
        // A fractional shape (6.5 % 5 = 1.5) throws in the LFO: reported once, the gain then
        // stays at its intrinsic value.
        let (mut stages, _, _) =
            built(json!({ "tremolo": 4, "tremoloshape": 6.5, "tremolodepth": 0.5 })).unwrap();
        let mut t = stages.remove(1);
        let mut errors = Vec::new();
        let mut last = [0.0f32; QUANTUM];
        for k in 0..100u64 {
            let mut bus = VoiceBus::mono();
            bus.ch[0] = [1.0; QUANTUM];
            bus.silent = false;
            t.process(k * 128, &mut bus, &StageInputs::NONE);
            errors.extend(t.take_error());
            last = bus.ch[0];
        }
        assert_eq!(errors, [lfo::SHAPE_ERROR]);
        assert!(last.iter().all(|x| *x == 0.5));
    }

    #[test]
    fn filter_lfo_rules() {
        let lfo_of = |v: serde_json::Value| {
            let (_, table, lfos) = built(v.clone()).unwrap();
            (table.get(FxKey::Main, "lpf_lfo").cloned(), lfos)
        };
        // Rule 2: active only if depth, depthfrequency, skew, shape or rate (after the sync
        // rewrite) is defined; `lpdc` alone is not; null counts as defined except for sync.
        for v in [
            json!({ "cutoff": 800 }),
            json!({ "cutoff": 800, "lpdc": 0.2 }),
            json!({ "cutoff": 800, "lpsync": null }),
        ] {
            let (entry, lfos) = lfo_of(v.clone());
            assert_eq!(entry, Some(NodeList::Array(vec![None])), "{v}");
            assert!(lfos.is_empty(), "{v}");
        }
        for v in [
            json!({ "cutoff": 800, "lprate": 3 }),
            json!({ "cutoff": 800, "lprate": null }),
            json!({ "cutoff": 800, "lpsync": 2 }),
            json!({ "cutoff": 800, "lpdepth": 0.5 }),
            json!({ "cutoff": 800, "lpdepth": null }),
            json!({ "cutoff": 800, "lpdepthfrequency": 100 }),
            json!({ "cutoff": 800, "lpskew": 0.2 }),
            json!({ "cutoff": 800, "lpshape": "saw" }),
        ] {
            let (entry, lfos) = lfo_of(v.clone());
            assert_eq!(
                entry,
                Some(NodeList::Array(vec![Some(Node::StageLfo(StageId(1)))])),
                "{v}"
            );
            assert_eq!(lfos.len(), 1, "{v}");
            assert_eq!(lfos[0].stage, StageId(1));
            assert_eq!(lfos[0].slot, ParamSlot::BIQUAD_FREQUENCY);
        }
        let props = |v: serde_json::Value| {
            let (_, _, lfos) = built(v).unwrap();
            lfo_values(&lfos[0].lfo)
        };
        // Rules 3 and 5: depth 1 by default (null too), modDepth = depth * frequency;
        // frequency = rate ?? cps; min -f + 30, max 20000 - f; begin, the hold end, time =
        // cycle / cps, curve 1, dcoffset -0.5, skew 0.5, shape 0, phaseoffset 0.
        assert_eq!(
            props(json!({ "cutoff": 800, "lprate": 3 })),
            [
                0.25, 0.75, 0.5, 3.0, 0.5, 800.0, 0.0, 0.0, 1.0, -0.5, -770.0, 19200.0
            ]
        );
        assert_eq!(
            props(json!({ "cutoff": 800, "lpdepth": null }))[lfo::DEPTH],
            800.0
        );
        assert_eq!(
            props(json!({ "cutoff": 800, "lprate": null }))[lfo::FREQUENCY],
            2.0
        );
        // Rule 1: sync -> cps * sync (wins over rate).
        assert_eq!(
            props(json!({ "cutoff": 800, "lprate": 3, "lpsync": 2 }))[lfo::FREQUENCY],
            4.0
        );
        // depthfrequency replaces depth * frequency.
        assert_eq!(
            props(json!({ "cutoff": 800, "lpdepth": 0.5, "lpdepthfrequency": 300 }))[lfo::DEPTH],
            300.0
        );
        assert_eq!(
            props(json!({ "cutoff": 800, "lpdepth": 0.5 }))[lfo::DEPTH],
            400.0
        );
        // dcoffset, skew and shape as given (null is 0, a shape name maps).
        let p = props(json!({ "cutoff": 800, "lpdc": -0.25, "lpskew": 0.3, "lpshape": "sine" }));
        assert_eq!(
            (p[lfo::DCOFFSET], p[lfo::SKEW], p[lfo::SHAPE]),
            (-0.25, 0.3, 1.0)
        );
        let p = props(json!({ "cutoff": 800, "lpdc": null, "lpskew": null, "lpshape": null }));
        assert_eq!(
            (p[lfo::DCOFFSET], p[lfo::SKEW], p[lfo::SHAPE]),
            (0.0, 0.0, 0.0)
        );
        // Rule 4: a falsy modDepth makes none (0, NaN), the entry stays `[undefined]`.
        for v in [
            json!({ "cutoff": 800, "lpdepth": 0 }),
            json!({ "cutoff": 0, "lprate": 2 }),
            json!({ "cutoff": 800, "lpdepth": 2, "lpdepthfrequency": 0 }),
            json!({ "cutoff": 800, "lpdepth": "x" }),
        ] {
            let (entry, lfos) = lfo_of(v.clone());
            assert_eq!(entry, Some(NodeList::Array(vec![None])), "{v}");
            assert!(lfos.is_empty(), "{v}");
        }
        // A non-finite rate throws at its `.value =` (after the filter was built).
        let e = built(json!({ "cutoff": 800, "lprate": "x" })).unwrap_err();
        assert!(e.detail.contains("TypeError"), "{}", e.detail);
    }

    #[test]
    fn filter_lfos_per_filter_kind_and_slope() {
        // 24 dB: two LFO instances with identical props, one per biquad.
        let (stages, table, lfos) =
            built(json!({ "cutoff": 800, "lprate": 3, "ftype": "24db" })).unwrap();
        assert_eq!(stages.len(), 3);
        assert_eq!(
            table.get(FxKey::Main, "lpf_lfo"),
            Some(&NodeList::Array(vec![
                Some(Node::StageLfo(StageId(1))),
                Some(Node::StageLfo(StageId(2)))
            ]))
        );
        assert_eq!(lfos.len(), 2);
        assert_eq!(lfo_values(&lfos[0].lfo), lfo_values(&lfos[1].lfo));
        assert_eq!((lfos[0].stage, lfos[1].stage), (StageId(1), StageId(2)));
        // 24 dB without an LFO: two `undefined` elements.
        let (_, table, _) = built(json!({ "cutoff": 800, "ftype": "24db" })).unwrap();
        assert_eq!(
            table.get(FxKey::Main, "lpf_lfo"),
            Some(&NodeList::Array(vec![None, None]))
        );
        // The highpass and bandpass maps, each with its own LFO and entry.
        let (_, table, lfos) = built(json!({
            "hcutoff": 200, "hprate": 3, "hpdepth": 0.5, "bandf": 1000, "bpdepth": 0.3
        }))
        .unwrap();
        assert_eq!(table.get(FxKey::Main, "lpf_lfo"), None);
        assert_eq!(
            table.get(FxKey::Main, "hpf_lfo"),
            Some(&NodeList::Array(vec![Some(Node::StageLfo(StageId(1)))]))
        );
        assert_eq!(
            table.get(FxKey::Main, "bpf_lfo"),
            Some(&NodeList::Array(vec![Some(Node::StageLfo(StageId(2)))]))
        );
        let (hp, bp) = (lfo_values(&lfos[0].lfo), lfo_values(&lfos[1].lfo));
        assert_eq!(
            (hp[lfo::DEPTH], hp[lfo::MIN], hp[lfo::MAX]),
            (100.0, -170.0, 19800.0)
        );
        assert_eq!(
            (bp[lfo::DEPTH], bp[lfo::MIN], bp[lfo::MAX]),
            (300.0, -970.0, 19000.0)
        );
        assert_eq!(bp[lfo::FREQUENCY], 2.0, "rate ?? cps");
        // The ladder: its frequency param.
        let (stages, table, lfos) =
            built(json!({ "cutoff": 800, "lprate": 3, "ftype": "ladder" })).unwrap();
        assert!(matches!(stages[1], Stage::Ladder(_)));
        assert_eq!(lfos[0].slot, ParamSlot::LADDER_FREQUENCY);
        assert_eq!(
            table.get(FxKey::Main, "lpf_lfo"),
            Some(&NodeList::Array(vec![Some(Node::StageLfo(StageId(1)))]))
        );
    }

    #[test]
    fn downmix_is_two_unfused_accumulations() {
        let tiny = f32::EPSILON; // 2^-23: half of it is lost against 1.0, the sum is not
        let ch = [[tiny; QUANTUM], [tiny; QUANTUM]];
        let bufs = downmix_terms(&ch);
        assert_eq!(bufs, [[tiny / 2.0; QUANTUM], [tiny / 2.0; QUANTUM]]);
        // Summed into a param at 1: (1 + 2^-24) + 2^-24 rounds to 1 twice, unlike 1 + 2^-23.
        let mut p = Param::gain(1.0);
        let mut out = [0.0f32; QUANTUM];
        p.fill_quantum_connected(0, &mut out, &bufs, true);
        assert_eq!(out[0], 1.0);
        assert_ne!(1.0 + tiny, 1.0);
    }

    #[test]
    fn the_plan_lists_every_source_and_stage() {
        // These voices cover every stage kind and every source kind, and a chain entry's local
        // delay and reverb.
        use StageSpec::*;
        let m = FxKey::Main;
        let x0 = FxKey::Index(0);
        let f = |entry, kind, ladder| Filter {
            entry,
            kind,
            ladder,
        };
        let (lp, hp, bp) = (
            FilterKind::Lowpass,
            FilterKind::Highpass,
            FilterKind::Bandpass,
        );
        let cases = [
            (
                json!({ "s": "sawtooth", "transient": 0.5, "cutoff": 500, "lprate": 2, "hcutoff": 100,
                        "bandf": 800, "ftype": "24db", "vowel": "a", "coarse": 2, "crush": 4,
                        "shape": 0.3, "distort": 0.2, "tremolo": 3, "compressor": -10, "pan": 0.2,
                        "phaserrate": 2, "FXrelease": 0.5 }),
                vec![
                    SynthGain,
                    EnvGain,
                    Transient(m),
                    EntryGain(m),
                    f(m, lp, false),
                    f(m, lp, false),
                    f(m, hp, false),
                    f(m, hp, false),
                    f(m, bp, false),
                    f(m, bp, false),
                    Vowel(m),
                    Coarse(m),
                    Crush(m),
                    Shape(m),
                    Distort(m),
                    Tremolo(m),
                    Compressor(m),
                    Pan(m),
                    Phaser(m),
                    Release,
                    Post,
                ],
            ),
            (
                json!({ "s": "supersaw", "cutoff": 500, "ftype": "ladder", "lpdepth": 0.5 }),
                vec![EnvGain, EntryGain(m), f(m, lp, true), Post],
            ),
            (
                json!({ "s": "pulse", "pwsweep": 0.2, "vib": 3, "fmi": 1 }),
                vec![EnvGain, EntryGain(m), Post],
            ),
            (
                json!({ "s": "one" }),
                vec![SynthGain, EnvGain, EntryGain(m), Post],
            ),
            (
                json!({ "s": "white", "delay": 0.5, "room": 0.3, "djf": 0.2 }),
                vec![SynthGain, EnvGain, EntryGain(m), Post],
            ),
            (
                json!({ "s": "bus", "n": 1 }),
                vec![EnvGain, EntryGain(m), Post],
            ),
            (
                json!({ "s": "sine", "FX": [{ "delay": 0.5, "delaytime": 0.2,
                        "delayfeedback": 0.3, "room": 0.3 }] }),
                vec![
                    SynthGain,
                    EnvGain,
                    EntryGain(x0),
                    LocalDelay(x0),
                    LocalReverb(x0),
                    EntryGain(m),
                    Post,
                ],
            ),
        ];
        for (v, specs) in cases {
            let p =
                parse_voice_params(&serde_json::from_value(v.clone()).unwrap(), 0.2, 0.5).unwrap();
            let mut orbit = Orbit::new(
                &OrbitSpec {
                    orbit: 1.0,
                    channels: vec![0, 1],
                },
                2,
            );
            let buses = crate::bus::BusBank::of_keys(vec!["1".into()]);
            let (voice, late) = Voice::build(
                &p,
                0,
                StartInfo {
                    t: 0.0,
                    hap_duration: 0.2,
                    now_frame: 0,
                    cycle: 0.0,
                    phase_sec: 0.0,
                    cps: 0.5,
                },
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut orbit,
                &buses,
            )
            .unwrap();
            assert!(late.is_none(), "{v}");
            let planned = voice.node_table().specs();
            assert_eq!(planned, specs.as_slice(), "{v}");
            // A `build_stage` arm that returns the wrong stage fails here.
            assert!(
                voice
                    .stages()
                    .iter()
                    .map(StageKind::of)
                    .eq(planned.iter().map(|s| s.kind())),
                "{v}: the built stages are the planned kinds"
            );
            // The head split `Voice::build` relies on.
            let head = planned.iter().position(|s| *s == EnvGain).unwrap() + 1;
            assert!(
                matches!(&planned[..head], [EnvGain] | [SynthGain, EnvGain]),
                "{v}"
            );
            assert!(
                !planned[head..]
                    .iter()
                    .any(|s| matches!(s, SynthGain | EnvGain)),
                "{v}"
            );
            let kind = SourceKind::of(&crate::voice::sound_name(p.s.as_ref())).unwrap();
            assert_eq!(voice.node_table().source(), kind, "{v}");
        }
    }

    #[test]
    fn stage_inputs_slots() {
        let mut i = StageInputs::input(false);
        assert!(!i.input_connected);
        assert_eq!(i.of(ParamSlot(0)), &ParamInputs::NONE);
        i.connect(ParamSlot(0), None);
        assert!(i.of(ParamSlot(0)).connected && i.of(ParamSlot(0)).bufs.is_empty());
        i.connect(ParamSlot(0), Some([1.0; QUANTUM]));
        i.connect(ParamSlot(0), Some([2.0; QUANTUM]));
        assert_eq!(i.of(ParamSlot(0)).bufs.len(), 2);
        assert_eq!(i.of(ParamSlot(0)).bufs[1][0], 2.0, "connection order");
        assert_eq!(i.of(ParamSlot(3)), &ParamInputs::NONE);
        // Out-of-range slots are ignored and read as unconnected.
        i.connect(ParamSlot(9), Some([1.0; QUANTUM]));
        assert_eq!(i.of(ParamSlot(9)), &ParamInputs::NONE);
        let none = StageInputs::NONE;
        assert!(none.input_connected);
        assert_eq!(none.of(ParamSlot(0)), &ParamInputs::NONE);
    }

    #[test]
    fn a_connected_ladder_frequency_and_a_filter_lfo_voice() {
        // A ladder with its LFO input: the frequency block is the value plus the input.
        let mut l = Ladder::new();
        l.frequency.set_value(500.0, 0).unwrap();
        let mut node = LadderNode::new(l);
        let mut io = [[0.0f32; QUANTUM]; 1];
        let inputs = ParamInputs {
            bufs: vec![[100.0; QUANTUM]],
            connected: true,
        };
        node.process(128, &mut io, true, &inputs);
        assert_eq!(node.proc.frequency.value(), 500.0, "the timeline value");
        // In a voice the LFO renders before its biquad and the output follows it: with an LFO
        // the voice differs from the plain one only from the LFO's first active quantum.
        let mut plain = voice(json!({ "s": "saw", "cutoff": 800 }));
        let mut lfo = voice(json!({ "s": "saw", "cutoff": 800, "lprate": 20, "lpdepth": 0.9 }));
        let (mut a, mut b) = (VoiceOutput::new(), VoiceOutput::new());
        plain.process(0, &mut a, &mut crate::bus::BusBank::default());
        lfo.process(0, &mut b, &mut crate::bus::BusBank::default());
        assert_eq!(a.out[0], b.out[0], "quantum 0: ct <= begin");
        plain.process(128, &mut a, &mut crate::bus::BusBank::default());
        lfo.process(128, &mut b, &mut crate::bus::BusBank::default());
        assert_ne!(a.out[0], b.out[0]);
    }

    #[test]
    fn native_effects_build_in_superdough_order_and_register() {
        let (stages, table, lfos) = built(json!({
            "cutoff": 800, "vowel": "o", "crush": 4, "tremolo": 2, "compressor": -10,
            "pan": 0.3, "phaserrate": 2
        }))
        .unwrap();
        assert_eq!(
            table.specs(),
            [
                S::EntryGain(M),
                filter(M, FilterKind::Lowpass, false),
                S::Vowel(M),
                S::Crush(M),
                S::Tremolo(M),
                S::Compressor(M),
                S::Pan(M),
                S::Phaser(M)
            ]
        );
        assert_eq!(
            stages.len(),
            table.specs().len(),
            "one built stage per spec"
        );
        let vowel: Vec<Option<Node>> = (0..5)
            .map(|n| Some(Node::VowelFilter(StageId(2), n)))
            .collect();
        assert_eq!(
            table.get(FxKey::Main, "vowel"),
            Some(&NodeList::Array(vowel))
        );
        assert_eq!(
            table.get(FxKey::Main, "compressor"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(5)))]))
        );
        assert_eq!(
            table.get(FxKey::Main, "phaser"),
            Some(&NodeList::Array(vec![Some(Node::Stage(StageId(7)))]))
        );
        assert_eq!(
            table.get(FxKey::Main, "phaser_lfo"),
            Some(&NodeList::Array(vec![Some(Node::StageLfo(StageId(7)))]))
        );
        // The phaser's LFO feeds its notch's detune.
        let pl: Vec<(usize, ParamSlot)> = lfos.iter().map(|l| (l.stage.0, l.slot)).collect();
        assert_eq!(pl, [(7, ParamSlot::BIQUAD_DETUNE)]);
        // Channel counts: the compressor makes the bus stereo.
        let ch: Vec<usize> = stages
            .iter()
            .scan(1, |n, s| {
                *n = s.channels_out(*n);
                Some(*n)
            })
            .collect();
        assert_eq!(ch, [1, 1, 1, 1, 1, 2, 2, 2]);
    }

    #[test]
    fn compressor_values_and_throw_sites() {
        let comp = |stages: &[Stage]| -> Compressor {
            stages
                .iter()
                .find_map(|s| match s {
                    Stage::Compressor(c) => Some((**c).clone()),
                    _ => None,
                })
                .unwrap()
        };
        let (stages, _, _) = built(json!({ "compressor": -20 })).unwrap();
        let c = comp(&stages);
        let v = [&c.threshold, &c.ratio, &c.knee, &c.attack, &c.release].map(|p| p.value());
        assert_eq!(v, [-20.0, 10.0, 10.0, 0.005, 0.05], "superdough's defaults");
        // `null` takes the `??` default; the colon form's five keys; out-of-range clamps.
        let (stages, _, _) = built(json!({
            "compressor": null, "compressorRatio": 30, "compressorKnee": 45,
            "compressorAttack": 0, "compressorRelease": null
        }))
        .unwrap();
        let c = comp(&stages);
        let v = [&c.threshold, &c.ratio, &c.knee, &c.attack, &c.release].map(|p| p.value());
        assert_eq!(v, [-3.0, 20.0, 40.0, 0.0, 0.05]);
        // A ratio without `compressor` builds nothing.
        let (stages, _, _) = built(json!({ "compressorRatio": 4 })).unwrap();
        assert!(!stages.iter().any(|s| matches!(s, Stage::Compressor(_))));
        // Non-finite values throw (a hap error); the stages before are kept.
        for v in [
            json!({ "crush": 1, "compressor": { "$num": "NaN" } }),
            json!({ "crush": 1, "compressor": -3, "compressorKnee": { "$num": "Infinity" } }),
            json!({ "crush": 1, "compressor": -3, "compressorRelease": "x" }),
            json!({ "crush": 1, "compressor": -3, "compressorAttack": 1e39 }),
        ] {
            let map: ValueMap = serde_json::from_value(v.clone()).unwrap();
            let params = parse_voice_params(&map, 0.25, 2.0).unwrap();
            let (stages, table, _, r) = walk_entry(&params, TIMES);
            let e = r.unwrap_err();
            assert!(e.detail.contains("non-finite"), "{v}: {}", e.detail);
            assert_eq!(
                &table.specs()[..stages.len()],
                [S::EntryGain(M), S::Crush(M)],
                "{v}"
            );
            assert!(table.get(FxKey::Main, "compressor").is_some(), "planned");
        }
    }

    #[test]
    fn vowel_throw_site() {
        for (v, msg) in [
            (
                json!({ "cutoff": 500, "vowel": "zz" }),
                "vowel: unknown vowel zz",
            ),
            (
                json!({ "cutoff": 500, "vowel": null }),
                "vowel: unknown vowel null",
            ),
            (
                json!({ "cutoff": 500, "vowel": 1 }),
                "vowel: unknown vowel 1",
            ),
        ] {
            let map: ValueMap = serde_json::from_value(v.clone()).unwrap();
            let params = parse_voice_params(&map, 0.25, 2.0).unwrap();
            let (stages, table, _, r) = walk_entry(&params, TIMES);
            let e = r.unwrap_err();
            assert_eq!(e.kind, crate::job::ErrorKind::Hap);
            assert_eq!(e.detail, msg);
            assert_eq!(
                &table.specs()[..stages.len()],
                [S::EntryGain(M), filter(M, FilterKind::Lowpass, false)],
                "built before the vowel"
            );
            assert!(table.get(FxKey::Main, "vowel").is_some(), "planned");
        }
        assert!(built(json!({ "vowel": "ø" })).is_ok());
    }

    fn notch_of(stages: &[Stage]) -> &Biquad {
        stages
            .iter()
            .find_map(|s| match s {
                Stage::Phaser(b) => Some(b),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn phaser_q_mapping_values_and_condition() {
        for (depth, q) in [
            (0.2, 1.6),
            (0.75, 0.5),
            (1.5, 0.1),
            (0.95, 0.1),
            (0.01, 1.98),
        ] {
            let (stages, _, lfos) =
                built(json!({ "phaserrate": 3, "phaserdepth": depth })).unwrap();
            let n = notch_of(&stages);
            assert_eq!(
                n.q.value(),
                (2.0 - f64::min(depth * 2.0, 1.9)) as f32,
                "{depth}"
            );
            assert_eq!(n.q.value(), q as f32, "{depth}");
            assert_eq!(n.frequency.value(), 1282.0);
            assert_eq!(n.gain.value(), 1.0);
            // Events at the pause frame: gain, frequency, Q (one each), none on detune.
            for p in [&n.gain, &n.frequency, &n.q] {
                assert_eq!(p.events().len(), 1);
                assert_eq!(p.events()[0].time, 128.0 / 44100.0);
            }
            assert!(n.detune.events().is_empty());
            let l = &lfos[0].lfo;
            let vals = lfo_values(l);
            assert_eq!(vals[lfo::FREQUENCY], 3.0);
            assert_eq!(vals[lfo::DEPTH], 4000.0, "sweep * 2");
            assert_eq!(vals[lfo::MIN], -2000.0);
            assert_eq!(vals[lfo::MAX], 2000.0);
            assert_eq!(vals[lfo::BEGIN], 0.25);
            assert_eq!(vals[lfo::END], 0.51f32, "endWithRelease");
            assert_eq!(vals[lfo::TIME], 0.25, "time = begin");
            assert_eq!(vals[lfo::DCOFFSET], -0.5);
            assert_eq!(vals[lfo::SKEW], 0.5);
        }
        // Centre and sweep; null controls are treated as absent.
        let (stages, _, lfos) = built(json!({
            "phaserrate": 8, "phasercenter": 500, "phasersweep": 800
        }))
        .unwrap();
        assert_eq!(notch_of(&stages).frequency.value(), 782.0);
        assert_eq!(lfo_values(&lfos[0].lfo)[lfo::DEPTH], 1600.0);
        let (stages, _, lfos) = built(json!({
            "phaserrate": null, "phasercenter": null, "phasersweep": null, "phaserdepth": null
        }))
        .unwrap();
        assert_eq!(notch_of(&stages).frequency.value(), 1282.0);
        assert_eq!(notch_of(&stages).q.value(), 0.5);
        assert_eq!(lfo_values(&lfos[0].lfo)[lfo::FREQUENCY], 1.0);
        // phaserdepth 0 (or negative, or NaN) and a missing phaserrate build no phaser.
        for v in [
            json!({ "phaserrate": 2, "phaserdepth": 0 }),
            json!({ "phaserrate": 2, "phaserdepth": -1 }),
            json!({ "phaserrate": 2, "phaserdepth": "x" }),
            json!({ "phaserdepth": 0.5, "phasercenter": 300 }),
        ] {
            let (stages, table, lfos) = built(v.clone()).unwrap();
            assert!(!stages.iter().any(|s| matches!(s, Stage::Phaser(_))), "{v}");
            assert!(
                lfos.is_empty() && table.get(FxKey::Main, "phaser").is_none(),
                "{v}"
            );
        }
        // Non-finite values throw: the LFO first (sweep), then the notch's frequency.
        for v in [
            json!({ "phaserrate": 2, "phasersweep": { "$num": "NaN" } }),
            json!({ "phaserrate": { "$num": "Infinity" } }),
            json!({ "phaserrate": 2, "phasercenter": { "$num": "NaN" } }),
        ] {
            let e = built(v.clone()).unwrap_err();
            assert!(e.detail.contains("TypeError"), "{v}: {}", e.detail);
        }
    }

    #[test]
    fn the_phaser_notch_follows_its_lfo_then_freezes() {
        // A phaser from frame 0 with its LFO ending at endWithRelease = 0.51 s: while the LFO
        // runs the notch takes 128 coefficient sets a quantum, outside it one (the centre).
        let (mut stages, _, mut lfos) = built(json!({ "phaserrate": 4 })).unwrap();
        let k = stages
            .iter()
            .position(|s| matches!(s, Stage::Phaser(_)))
            .unwrap();
        let end_q = lfos[0].lfo.end_quantum();
        let mut sample_accurate = Vec::new();
        for q in (0..end_q + 4 * 128).step_by(128) {
            let mut bus = VoiceBus::mono();
            bus.ch[0] = signal(q as usize);
            bus.silent = false;
            let mut inputs = StageInputs::input(true);
            let mut buf = [0.0f32; QUANTUM];
            let silent = lfos[0].lfo.render(q, &lfo::NO_INPUTS, &mut buf);
            inputs.connect(ParamSlot::BIQUAD_DETUNE, (!silent).then_some(buf));
            stages[k].process(q, &mut bus, &inputs);
            let Stage::Phaser(n) = &stages[k] else {
                unreachable!()
            };
            sample_accurate.push((q, n.kernels()[0].sample_accurate()));
        }
        // Before begin (0.25 s, frame 11025) and in the onset quantum: one set.
        assert!(
            sample_accurate
                .iter()
                .filter(|(q, _)| *q <= 11008)
                .all(|(_, s)| !s)
        );
        assert!(
            sample_accurate.iter().any(|(_, s)| *s),
            "the LFO moves the notch"
        );
        // After the LFO's end: frozen at the centre, one set.
        assert!(
            sample_accurate
                .iter()
                .filter(|(q, _)| *q >= end_q)
                .all(|(_, s)| !s)
        );
        let Stage::Phaser(n) = &stages[k] else {
            unreachable!()
        };
        assert_eq!(
            n.kernels()[0].get(0),
            crate::biquad::notch(crate::biquad::normalize_frequency(1282.0, 0.0), 0.5)
        );
    }

    #[test]
    fn compressor_makes_the_voice_stereo_and_pan_takes_the_stereo_law() {
        let value = json!({ "s": "sine", "compressor": -10, "pan": 0.5 });
        assert_eq!(voice(value.clone()).output_channels(), 2);
        let specs = planned(value, 0.1).specs().to_vec();
        let c = specs.iter().position(|s| *s == S::Compressor(M)).unwrap();
        assert_eq!(specs[c + 1], S::Pan(M));
        // Pan 0.5 on a stereo bus: the stereo law passes both channels unchanged (no -3 dB).
        let mut p = StereoPanner::new();
        p.pan.set_value(0.0, 0).unwrap();
        let mut stage = Stage::Panner(p);
        let mut bus = stereo_bus(false);
        let before = bus.ch;
        stage.process(0, &mut bus, &StageInputs::NONE);
        assert_eq!(bus.ch[1], before[1], "R = x");
        for (l, x) in bus.ch[0].iter().zip(before[0].iter()) {
            assert!(
                (l - x).abs() < 1e-6,
                "L = x + x * cos(pi / 2): no -3 dB centre"
            );
        }
        // Without the compressor a mono voice stays mono until the panner.
        assert_eq!(
            voice(json!({ "s": "sine", "vowel": "a" })).output_channels(),
            1
        );
        assert_eq!(
            voice(json!({ "s": "sine", "compressor": -10 })).output_channels(),
            2
        );
    }

    #[test]
    fn release_stage_rule() {
        // FXrelease > release: one release gain after the FX loop, before post.
        let value = json!({ "s": "sine", "release": 0.02, "FXrelease": 0.3, "phaserrate": 1 });
        let t = planned(value.clone(), 0.1);
        let n = t.specs().len();
        assert_eq!(&t.specs()[n - 3..], [S::Phaser(M), S::Release, S::Post]);
        let v = voice(value);
        let Stage::Release(g) = &v.stages()[n - 2] else {
            panic!()
        };
        let ev: Vec<(crate::param::RampKind, f32, f64)> = g
            .gain
            .events()
            .iter()
            .map(|e| (e.kind, e.value, e.time))
            .collect();
        use crate::param::RampKind::*;
        // `t = 0`, `hapDuration = 0.1`: hold to end + release, fade to end + FXrelease.
        assert_eq!(
            ev,
            [
                (SetValue, 1.0, 0.0),
                (SetValue, 1.0, 0.1 + 0.02),
                (Linear, 0.0, 0.1 + 0.3)
            ]
        );
        // FXrelease <= release, absent, null or NaN: no node.
        for extra in [
            json!({ "release": 0.3, "FXrelease": 0.3 }),
            json!({ "release": 0.3, "FXrelease": 0.1 }),
            json!({ "FXrelease": 0.005 }),
            json!({ "FXrelease": null }),
            json!({ "FXrelease": "x" }),
            json!({}),
        ] {
            let mut val = json!({ "s": "sine" });
            val.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let t = planned(val.clone(), 0.1);
            assert!(!t.specs().contains(&S::Release), "{val}");
            // The built voice walks that plan: no release gain among its stages.
            let v = voice(val.clone());
            assert_eq!(v.node_table().specs(), t.specs(), "{val}");
            assert!(
                !v.stages().iter().any(|s| matches!(s, Stage::Release(_))),
                "{val}"
            );
        }
        // The default release 0.01: FXrelease 0.02 builds; a null release is 0.
        let t = planned(json!({ "s": "sine", "FXrelease": 0.02 }), 0.1);
        assert!(t.specs().contains(&S::Release));
        let value = json!({ "s": "sine", "release": null, "FXrelease": 0.001 });
        assert!(planned(value.clone(), 0.1).specs().contains(&S::Release));
        let v = voice(value);
        let Some(Stage::Release(g)) = v.stages().iter().find(|s| matches!(s, Stage::Release(_)))
        else {
            panic!("null release is 0")
        };
        assert_eq!(g.gain.events()[1].time, 0.1);
        // The release gain is exactly 0 after its ramp (which ends at 0.101 s, frame 4455, in
        // the quantum at 4480). That quantum is sample accurate (three events): its frames past
        // the ramp's end are zeros flagged non-silent. From the next quantum the past events are
        // dropped (`RemoveOldEvents` keeps one), the param is not sample accurate any more and
        // `GainHandler`'s scalar path with gain 0 outputs a silent bus.
        let mut g = g.clone();
        for q in (0..44100).step_by(QUANTUM) {
            let mut ch = [[0.5f32; QUANTUM]];
            let silent = g.process(q, &mut ch, false, &ParamInputs::NONE);
            assert_eq!(silent, q > 4480, "{q}");
            if q >= 4480 {
                assert!(ch[0][QUANTUM - 1] == 0.0, "{q}");
            }
            if q > 4480 {
                assert!(ch[0].iter().all(|x| *x == 0.0), "{q}");
            }
        }
    }

    #[test]
    fn the_compressor_keeps_the_voice_until_its_pre_delay_drains() {
        // A constant source through a compressor: after the source stops, the voice lives while
        // the pre-delay still holds non-zero input, and is freed once it holds only zeros.
        let mut v = voice(json!({ "s": "one", "compressor": -10, "sustain": 1 }));
        let (_, stop) = v.source_frames();
        let mut out = VoiceOutput::new();
        let mut q = 0;
        let mut freed = None;
        while q < stop + 50 * 128 {
            if !v.process(q, &mut out, &mut crate::bus::BusBank::default()) {
                freed = Some(q);
                break;
            }
            q += 128;
        }
        let freed = freed.expect("freed");
        assert!(freed >= stop, "not before the source stops");
        let c = v
            .stages()
            .iter()
            .find_map(|s| match s {
                Stage::Compressor(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert!(c.is_done());
        assert!(v.stages().iter().all(|s| s.is_done(freed)));
    }

    // ---- FX chains

    /// A voice of `v` (0.1 s hap at t = 0, cps 0.5 for the controls) with its local reverbs
    /// settled; the build error otherwise.
    fn fx_voice(v: serde_json::Value) -> Result<Voice, crate::voice::VoiceError> {
        let params = parsed(v, 0.1);
        let mut orbit = Orbit::new(
            &OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1],
            },
            2,
        );
        let mut v = Voice::new(
            &params,
            0,
            StartInfo {
                t: 0.0,
                hap_duration: 0.1,
                now_frame: 0,
                cycle: 0.0,
                phase_sec: 0.0,
                cps: 2.0,
            },
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut orbit,
        )?;
        let mut cache = IrCache::new();
        for (_, r) in v.local_reverbs_mut() {
            r.settle_cached(0, &mut cache);
        }
        Ok(v)
    }

    fn local_delay(v: &Voice) -> Option<&LocalDelay> {
        v.stages().iter().find_map(|s| match s {
            Stage::LocalDelay(d) => Some(&**d),
            _ => None,
        })
    }

    fn local_reverb(v: &Voice) -> Option<&LocalReverb> {
        v.stages().iter().find_map(|s| match s {
            Stage::LocalReverb(r) => Some(&**r),
            _ => None,
        })
    }

    #[test]
    fn a_chain_entry_with_every_key_builds_in_superdough_order() {
        let t = planned(
            json!({
                "s": "sawtooth", "FXrelease": 0.3,
                "FX": [{
                    "transient": 0.5, "cutoff": 500, "hcutoff": 100, "bandf": 1000, "vowel": "a",
                    "coarse": 2, "crush": 4, "shape": 0.3, "distort": 0.5, "tremolo": 2,
                    "compressor": -10, "pan": 0.3, "phaserrate": 2, "delay": 0.5, "delaytime": 0.1,
                    "room": 0.3, "roomsize": 0.1
                }]
            }),
            0.1,
        );
        assert_eq!(
            t.specs(),
            [
                S::SynthGain,
                S::EnvGain,
                S::Transient(X0),
                S::EntryGain(X0),
                filter(X0, FilterKind::Lowpass, false),
                filter(X0, FilterKind::Highpass, false),
                filter(X0, FilterKind::Bandpass, false),
                S::Vowel(X0),
                S::Coarse(X0),
                S::Crush(X0),
                S::Shape(X0),
                S::Distort(X0),
                S::Tremolo(X0),
                S::Compressor(X0),
                S::Pan(X0),
                S::Phaser(X0),
                S::LocalDelay(X0),
                S::LocalReverb(X0),
                S::EntryGain(M),
                S::Release,
                S::Post
            ]
        );
        assert_eq!(
            t.keys(FxKey::Index(0)),
            [
                "transient",
                "gain",
                "lpf",
                "lpf_lfo",
                "hpf",
                "hpf_lfo",
                "bpf",
                "bpf_lfo",
                "vowel",
                "coarse",
                "crush",
                "shape",
                "distort",
                "tremolo",
                "tremolo_gain",
                "compressor",
                "pan",
                "phaser",
                "phaser_lfo",
                "delay",
                "delay_mix",
                "room",
                "room_mix"
            ]
        );
        let one = |n: Node| Some(NodeList::Array(vec![Some(n)]));
        assert_eq!(
            t.get(FxKey::Index(0), "delay").cloned(),
            one(Node::Stage(StageId(16)))
        );
        assert_eq!(
            t.get(FxKey::Index(0), "delay_mix").cloned(),
            one(Node::StageWet(StageId(16)))
        );
        assert_eq!(
            t.get(FxKey::Index(0), "room").cloned(),
            one(Node::Stage(StageId(17)))
        );
        assert_eq!(
            t.get(FxKey::Index(0), "room_mix").cloned(),
            one(Node::StageWet(StageId(17)))
        );
        assert_eq!(t.keys(FxKey::Main), ["source", "gain", "post"]);
        assert_eq!(t.stage(StageId(16)), Some(StageKind::LocalDelay));
        assert_eq!(t.stage(StageId(17)), Some(StageKind::LocalReverb));
        // Two chain entries, then main: each its own gain stage, in order.
        let value = json!({
            "s": "sine", "gain": 0.5, "FX": [{ "gain": 0.25 }, { "velocity": 0.5 }]
        });
        let t = planned(value.clone(), 0.1);
        assert!(t.has_entry(FxKey::Index(1)));
        assert!(!t.has_entry(FxKey::Index(2)));
        let v = fx_voice(value).unwrap();
        let gains: Vec<f32> = v.stages()[2..5]
            .iter()
            .map(|s| match s {
                Stage::Gain(g) => g.gain.value(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(gains, [0.25, 0.4, 0.5]);
    }

    #[test]
    fn a_chain_entry_gain_stage() {
        let gain = |e: serde_json::Value| {
            let v = fx_voice(json!({ "s": "sine", "FX": [e] })).unwrap();
            match &v.stages()[2] {
                Stage::Gain(g) => (g.gain.value(), g.gain.events().len()),
                other => panic!("{other:?}"),
            }
        };
        // Default 0.8 (one gain stage per entry: `.FX(lpf(500))` is 0.8 * 0.8 overall), a
        // constant gain (no timeline event).
        assert_eq!(gain(json!({})), (0.8, 0));
        assert_eq!(gain(json!({ "gain": 0.5, "velocity": 0.5 })), (0.25, 0));
        assert_eq!(
            gain(json!({ "gain": 2, "velocity": 0.3 })),
            ((2.0f64 * 0.3) as f32, 0)
        );
        // nanFallback: a gain whose number is NaN becomes 1.
        assert_eq!(gain(json!({ "gain": "x" })), (1.0, 0));
        assert_eq!(gain(json!({ "gain": null })), (0.0, 0));
        // A NaN velocity makes the product NaN: the WebIDL float throws.
        let e = fx_voice(json!({ "s": "sine", "FX": [{ "velocity": "x" }] })).unwrap_err();
        assert_eq!(e.kind, crate::job::ErrorKind::Hap);
    }

    #[test]
    fn the_24db_doubling_is_the_haps_and_the_ladder_the_entrys() {
        let count = |v: serde_json::Value, ladder: bool| {
            planned(v, 0.1)
                .specs()
                .iter()
                .filter(|s| matches!(s, S::Filter { ladder: l, .. } if *l == ladder))
                .count()
        };
        let counts = |v: serde_json::Value| (count(v.clone(), true), count(v, false));
        assert_eq!(
            count(
                json!({ "s": "sine", "ftype": "24db", "FX": [{ "cutoff": 500 }] }),
                false
            ),
            2
        );
        assert_eq!(
            count(
                json!({ "s": "sine", "FX": [{ "cutoff": 500, "ftype": "24db" }] }),
                false
            ),
            1
        );
        let value = json!({
            "s": "sine", "cutoff": 800, "drive": 5,
            "FX": [{ "cutoff": 500, "ftype": "ladder", "drive": 2 }]
        });
        assert_eq!(counts(value.clone()), (1, 1));
        let v = fx_voice(value).unwrap();
        let drive = v
            .stages()
            .iter()
            .find_map(|s| match s {
                Stage::Ladder(l) => Some(l.proc.drive.value()),
                _ => None,
            })
            .unwrap();
        assert_eq!(drive, 2.0, "the entry's drive");
        assert_eq!(
            counts(json!({ "s": "sine", "ftype": "ladder", "FX": [{ "cutoff": 500 }] })),
            (0, 1)
        );
    }

    #[test]
    fn local_delay_conditions_and_values() {
        let with = |e: serde_json::Value| fx_voice(json!({ "s": "sine", "FX": [e] })).unwrap();
        let plan = |e: serde_json::Value| planned(json!({ "s": "sine", "FX": [e] }), 0.1);
        let base = json!({ "delay": 0.5, "delaytime": 0.1, "delayfeedback": 0.5 });
        let t = plan(base.clone());
        assert!(t.specs().contains(&S::LocalDelay(X0)));
        assert!(t.get(X0, "delay").is_some());
        // Each of the three 0, negative or NaN disables it (JS `>`).
        for (k, bad) in [
            ("delay", json!(0)),
            ("delay", json!("x")),
            ("delaytime", json!(0)),
            ("delaytime", json!(-1)),
            ("delaytime", json!("x")),
            ("delayfeedback", json!(0)),
            ("delayfeedback", json!("x")),
        ] {
            let mut e = base.clone();
            e[k] = bad.clone();
            let t = plan(e.clone());
            assert!(!t.specs().contains(&S::LocalDelay(X0)), "{k} = {bad}");
            assert!(t.get(X0, "delay").is_none(), "{k}");
            // The built voice walks that plan: no local delay among its stages.
            let v = with(e);
            assert_eq!(v.node_table().specs(), t.specs(), "{k} = {bad}");
            assert!(local_delay(&v).is_none(), "{k} = {bad}");
        }
        // The default delay time is delaysync 3/16 cycles over cps; the default feedback 0.5.
        let v = with(json!({ "delay": 0.5 }));
        let d = local_delay(&v).unwrap();
        assert_eq!(d.delay.delay_time().value(), 0.375);
        assert_eq!(d.delay.feedback().value(), 0.5);
        // The feedback is clamped to 0.98 after the test; the delay time to the line's 1 s.
        let v = with(json!({ "delay": 0.5, "delaytime": 3, "delayfeedback": 5 }));
        let d = local_delay(&v).unwrap();
        assert_eq!(
            (d.delay.delay_time().value(), d.delay.feedback().value()),
            (1.0, 0.98)
        );
        // The wet gain is the raw level (no gain curve), the dry gain `dry ?? 1`; both are
        // `gainNode(x)`s (`.value =`, one `setValueAtTime`).
        let v = with(json!({ "delay": 0.3, "delaytime": 0.1 }));
        let d = local_delay(&v).unwrap();
        assert_eq!((d.wet.gain.value(), d.dry.gain.value()), (0.3, 1.0));
        assert_eq!(
            (d.wet.gain.events().len(), d.dry.gain.events().len()),
            (1, 1)
        );
        let v = with(json!({ "delay": 0.3, "delaytime": 0.1, "dry": 0.25 }));
        assert_eq!(local_delay(&v).unwrap().dry.gain.value(), 0.25);
        let v = with(json!({ "delay": 0.3, "delaytime": 0.1, "dry": null }));
        assert_eq!(local_delay(&v).unwrap().dry.gain.value(), 1.0);
        // Throws in superdough's order: the line's delayTime, the wet gain, the dry gain.
        for e in [
            json!({ "delay": 0.3, "delaytime": 1e300 }),
            json!({ "delay": 1e300, "delaytime": 0.1 }),
            json!({ "delay": 0.3, "delaytime": 0.1, "dry": "x" }),
        ] {
            let err = fx_voice(json!({ "s": "sine", "FX": [e.clone()] })).unwrap_err();
            assert_eq!(err.kind, crate::job::ErrorKind::Hap, "{e}");
        }
    }

    #[test]
    fn the_main_entry_never_builds_local_effects() {
        let local = |t: &NodeTable| -> Vec<S> {
            t.specs()
                .iter()
                .copied()
                .filter(|s| matches!(s, S::LocalDelay(_) | S::LocalReverb(_)))
                .collect()
        };
        let value = json!({
            "s": "sine", "delay": 0.5, "delaytime": 0.1, "room": 0.5, "roomsize": 0.1
        });
        let t = planned(value.clone(), 0.1);
        assert_eq!(local(&t), []);
        assert_eq!(
            t.get(FxKey::Main, "delay").cloned(),
            Some(NodeList::Array(vec![Some(Node::OrbitDelay)]))
        );
        assert_eq!(
            fx_voice(value).unwrap().send_levels(),
            (Some(0.5), Some(0.5))
        );
        // A chain entry's delay and room are local only: no orbit send.
        let value = json!({
            "s": "sine", "FX": [{ "delay": 0.5, "delaytime": 0.1, "room": 0.5, "roomsize": 0.1 }]
        });
        assert_eq!(
            local(&planned(value.clone(), 0.1)),
            [S::LocalDelay(X0), S::LocalReverb(X0)]
        );
        assert_eq!(fx_voice(value).unwrap().send_levels(), (None, None));
    }

    #[test]
    fn local_reverb_values_and_errors() {
        let v =
            fx_voice(json!({ "s": "sine", "FX": [{ "room": "0.5", "roomsize": 0.1 }] })).unwrap();
        let r = local_reverb(&v).unwrap();
        assert_eq!((r.wet.gain.value(), r.dry.gain.value()), (0.5, 1.0));
        assert_eq!(r.reverb.applied(), 1, "settled at the end of the pause");
        assert_eq!(r.reverb.convolver().unwrap().length(), 6615);
        for bad in [json!(0), json!(-1), json!("x"), json!(null)] {
            let v = fx_voice(json!({ "s": "sine", "FX": [{ "room": bad.clone() }] })).unwrap();
            assert!(local_reverb(&v).is_none(), "{bad}");
        }
        // An IR request that throws is a build throw: no voice. The lowpass's TypeError is a
        // `hap` error, an IR length the `ir` kind (as for the orbit's reverb).
        let e =
            fx_voice(json!({ "s": "sine", "FX": [{ "room": 0.5, "roomlp": "x" }] })).unwrap_err();
        assert_eq!(e.kind, crate::job::ErrorKind::Hap);
        let e =
            fx_voice(json!({ "s": "sine", "FX": [{ "room": 0.5, "roomsize": 0 }] })).unwrap_err();
        assert_eq!(e.kind, crate::job::ErrorKind::Ir);
        let e = fx_voice(json!({ "s": "sine", "FX": [{ "room": 0.5, "dry": "x" }] })).unwrap_err();
        assert_eq!(e.kind, crate::job::ErrorKind::Hap);
    }

    #[test]
    fn chain_entry_channel_rules() {
        let ch = |v: serde_json::Value| fx_voice(v).unwrap().output_channels();
        assert_eq!(ch(json!({ "s": "sine", "FX": [{ "cutoff": 500 }] })), 1);
        assert_eq!(ch(json!({ "s": "sine", "FX": [{ "pan": 0.5 }] })), 2);
        assert_eq!(
            ch(json!({ "s": "sine", "FX": [{ "delay": 0.5, "delaytime": 0.1 }] })),
            2
        );
        assert_eq!(
            ch(json!({ "s": "sine", "FX": [{ "room": 0.5, "roomsize": 0.1 }] })),
            2
        );
        assert_eq!(ch(json!({ "s": "supersaw", "FX": [{ "cutoff": 500 }] })), 2);
        // Mono until the entry's local stage, stereo after it (fold of `channels_out`).
        let value = json!({ "s": "sine", "FX": [{ "cutoff": 500, "delay": 0.5, "delaytime": 0.1 }, { "hcutoff": 50 }] });
        assert_eq!(
            planned(value.clone(), 0.1).specs()[3..6],
            [
                filter(X0, FilterKind::Lowpass, false),
                S::LocalDelay(X0),
                S::EntryGain(FxKey::Index(1))
            ]
        );
        let v = fx_voice(value).unwrap();
        let mut n = 1;
        let mut seq = Vec::new();
        for s in v.stages() {
            n = s.channels_out(n);
            seq.push(n);
        }
        assert_eq!(seq, [1, 1, 1, 1, 2, 2, 2, 2, 2]);
    }

    #[test]
    fn voice_bus_upmix_copies_the_mono_channel() {
        let mut b = mono_bus(false);
        b.ch[1] = [7.0; QUANTUM];
        b.upmix_to_stereo();
        assert_eq!(b.channels, 2);
        assert_eq!(bits(&b.ch[1..]), bits(&[signal(0)]));
        assert_eq!(bits(&b.ch[..1]), bits(&[signal(0)]));
        assert!(!b.silent, "the hint is unchanged");
        let mut s = mono_bus(true);
        s.upmix_to_stereo();
        assert!(s.silent && s.channels == 2);
        // A stereo bus is left as is.
        let mut st = stereo_bus(false);
        st.upmix_to_stereo();
        assert_eq!(bits(&st.ch), bits(&[signal(0), signal(1)]));
    }

    #[test]
    fn the_tail_cut_quantum() {
        // `cut_quantum = ceil(endWithRelease * 44100) / 128`: 640 and 767 are in quantum 5, 768
        // starts quantum 6.
        for (frames, quantum) in [(639.5, 5), (766.5, 5), (767.5, 6), (0.0, 0)] {
            let t = frames / 44100.0;
            let cut = TailCut::at(t);
            assert_eq!(cut.quantum(), Some(quantum), "{frames}");
            let q = quantum * 128;
            assert!(!cut.is_cut(q), "the cut quantum renders ({frames})");
            assert!(q < 128 || !cut.is_cut(q - 128));
            assert!(
                cut.is_cut(q + 128),
                "silent from the next quantum ({frames})"
            );
        }
        assert_eq!(crate::render::frame_for_param_event(639.5 / 44100.0), 640);
        assert_eq!(crate::render::frame_for_param_event(766.5 / 44100.0), 767);
        assert_eq!(crate::render::frame_for_param_event(767.5 / 44100.0), 768);
        // A non-finite endWithRelease: no cut.
        for t in [f64::NAN, f64::INFINITY] {
            let cut = TailCut::at(t);
            assert_eq!(cut.quantum(), None);
            assert!(!cut.is_cut(u64::MAX));
            assert_eq!(cut.disabled_from(256), None);
        }
        // The output connection outlives the input by the cut.
        let cut = TailCut::at(767.5 / 44100.0);
        assert_eq!(
            (cut.disabled_from(256), cut.disabled_from(1024)),
            (Some(896), Some(1024))
        );
    }

    #[test]
    fn local_delay_sums_dry_and_wet_and_is_cut() {
        let e = parse_entry_params(
            &serde_json::from_value(
                json!({ "delay": 0.5, "delaytime": 0.01, "delayfeedback": 0.5 }),
            )
            .unwrap(),
            0.5,
        );
        let cut = TailCut::at(1279.5 / 44100.0); // quantum 10
        let mut stage = Stage::LocalDelay(Box::new(LocalDelay::new(&e, cut, 0).unwrap()));
        // The reference: the same feedback delay and gains, by hand.
        let mut fd = FeedbackDelay::new(0.01, 0.5, 0).unwrap();
        let mut ref_wet = value_gain(0.5, 0).unwrap();
        let mut out = Vec::new();
        for k in 0..12u64 {
            let q = k * 128;
            let mut input = [0.0f32; QUANTUM];
            if k == 0 {
                input[0] = 1.0;
            }
            let silent = k > 0;
            let mut bus = VoiceBus::mono();
            bus.ch[0] = input;
            bus.silent = silent;
            stage.process(q, &mut bus, &StageInputs::NONE);
            assert_eq!(bus.channels, 2);
            assert_eq!(
                bits(&bus.ch[..1]),
                bits(&bus.ch[1..]),
                "the dry up-mix and a mono wet"
            );
            if k <= 10 {
                fd.begin(q, 1);
                fd.add_send(Input {
                    channels: &[input],
                    silent,
                });
                let ret = fd.finish(q);
                let mut wet = [ret.channels[0]];
                let wet_silent = ref_wet.process(q, &mut wet, ret.silent, &ParamInputs::NONE);
                let mut expect = if silent { [0.0f32; QUANTUM] } else { input };
                if !wet_silent {
                    for (x, w) in expect.iter_mut().zip(wet[0].iter()) {
                        *x = if silent { *w } else { *x + *w };
                    }
                }
                assert_eq!(bits(&bus.ch[..1]), bits(&[expect]), "quantum {k}");
                assert!(!stage.is_done(q), "not done before the cut ({k})");
            } else {
                assert!(
                    bus.silent && bits(&bus.ch).iter().all(|b| *b == 0),
                    "cut ({k})"
                );
                assert!(stage.is_done(q));
            }
            out.extend_from_slice(&bus.ch[0]);
        }
        // The dry impulse, then the first echo (0.5) near 441 frames and the second (0.25) one
        // loop later.
        assert_eq!(out[0], 1.0);
        let peak = |from: usize, to: usize| out[from..to].iter().fold(0.0f32, |m, x| m.max(*x));
        assert!((peak(430, 450) - 0.5).abs() < 0.01, "{}", peak(430, 450));
        assert!(
            (peak(1000, 1020) - 0.25).abs() < 0.01,
            "{}",
            peak(1000, 1020)
        );
        assert!(out[1..430].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn a_voice_with_a_local_tail_lives_until_the_cut() {
        // fxr 0.3: endWithRelease = 0.1 + 0.3 = 0.4 s, frame 17640 in quantum 137.
        let mut v = fx_voice(json!({
            "s": "sine", "FXrelease": 0.3,
            "FX": [{ "delay": 0.5, "delaytime": 0.05, "delayfeedback": 0.9 }]
        }))
        .unwrap();
        let mut out = VoiceOutput::new();
        let mut q = 0;
        let freed = loop {
            if !v.process(q, &mut out, &mut crate::bus::BusBank::default()) {
                break q;
            }
            q += 128;
            assert!(q < 44100, "freed");
        };
        assert_eq!(freed, 138 * 128, "freed right after the cut quantum");
        assert_eq!(TailCut::at(0.1 + 0.3).quantum(), Some(137));
    }

    #[test]
    fn source_names_match_the_builders() {
        use crate::noise::NoiseKind;
        let names = SourceKind::NAMES;
        for (i, (name, _)) in names.iter().enumerate() {
            assert!(
                !names[i + 1..].iter().any(|(n, _)| n == name),
                "{name} is listed once"
            );
        }
        for (name, kind) in names {
            assert_eq!(
                *kind == SourceKind::Noise,
                NoiseKind::from_name(name).is_some(),
                "{name}: a Noise row exactly when it names a noise"
            );
        }
        for noise in [
            NoiseKind::White,
            NoiseKind::Pink,
            NoiseKind::Brown,
            NoiseKind::Crackle,
        ] {
            // Exhaustive, so a new noise fails to compile until it is listed above.
            match noise {
                NoiseKind::White | NoiseKind::Pink | NoiseKind::Brown | NoiseKind::Crackle => {}
            }
            assert!(
                names.contains(&(noise.name(), SourceKind::Noise)),
                "{} is a Noise row",
                noise.name()
            );
        }
        assert_eq!(SourceKind::of(DEFAULT_SOUND), Some(SourceKind::Osc));
    }
}

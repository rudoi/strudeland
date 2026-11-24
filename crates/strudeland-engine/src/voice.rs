// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/gain_handler.cc, modules/webaudio/audio_handler.cc, platform/audio/audio_bus.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! One synth hap (`Voice`): source -> `Gain(0.3)` -> amp envelope -> gain stage -> filters ->
//! panner -> postgain -> dry, mono until the panner (stereo throughout behind a worklet synth). The
//! chain from `Gain(0.3)` to `post` is a list of [`Stage`]s processed on a [`VoiceBus`] (see
//! `chain`); each `value.FX` entry adds its own stages (with a local delay and reverb) before the
//! main entry's; `dry` forks from `post`.
//!
//! The oscillator sounds (`triangle`, `square`, `sawtooth`, `sine`, `user` and their aliases) are
//! periodic-wave oscillators built as superdough's `getOscillator` builds them (built-in or
//! `waveformN` tables, `frequency.value`, vibrato, pitch envelope, the FM matrix of `applyFM`, the
//! `noise` mix), `one` is a constant source, the noise sounds (`white`, `pink`, `brown`, `crackle`)
//! are looping noise buffer sources (`getNoiseOscillator`), and `supersaw` and `pulse` are stereo
//! worklet sources connected straight to the amp envelope (no `Gain(0.3)`), with the pitch envelope
//! and vibrato on their `detune` param and FM on their `frequency`. The `bus` sound reads one of
//! the job's buses (`bus`) as a stereo source straight into its `envGain` (linear ADSR with sustain
//! 1); a voice with `bus != null` sends `post` times `busgain` to a bus. The amp envelope (an a-rate
//! `Param` driven by `get_param_adsr`, applied as Chromium's `GainHandler::Process` does), the
//! filters (`createFilter`: 12 or 24 dB biquads, or the ladder worklet, with their envelopes and
//! LFOs), the shapers, the tremolo, the panner (only when `pan` is defined), `postgain`, the
//! sends, the orbit `djf` call, `dry`, the level rules, the stop time and the lifetime rule are
//! final. The pulse's width modulation is an `lfo` into its `pulsewidth`.
//!
//! The chain carries `AudioHandler`'s silence hint from node to node: a source that is not
//! playing outputs a silent bus, a gain node passes silence on (and makes it when its gain is
//! exactly 0), and a biquad whose input has been silent for longer than its tail time outputs
//! silence instead of ringing on (`AudioHandler::ProcessIfNecessary`, `PropagatesSilence`).

use crate::QUANTUM;
use crate::adsr::{Curve, get_adsr_values, get_param_adsr};
use crate::biquad::{Biquad, FilterKind};
use crate::bus::BusBank;
use crate::chain::{
    EntryTimes, MAX_PARAM_SLOTS, Node, NodeTable, ParamLfo, ParamRef, ParamSlot, SourceEnd,
    SourceKind, Stage, StageId, StageInputs, StageKind, Teardown, VoiceBus, build_entry,
    delay_applies, input_connected, push_stage, set_worklet_value,
};
use crate::controls::{
    ControlError, FilterParams, FmModParams, VoiceParams, frequency_from_value, js_to_string,
    pitch_curve,
};
use crate::controls::{js_max, js_min};
use crate::job::{ErrorKind, Value};
use crate::jsmath;
use crate::lfo::{Lfo, LfoProps, get_lfo};
use crate::modulate::{ModSet, ModTimes, ModView, OrbitParam};
use crate::noise::{DEFAULT_DENSITY, FM_DENSITY, NoiseCache, NoiseKind, NoiseSource};
use crate::orbit::Orbit;
use crate::osc::{ConstantSource, Input, Oscillator, ScheduleError};
#[cfg(test)]
use crate::panner::StereoPanner;
use crate::param::{Param, ParamError, ParamInputs, Rate};
use crate::periodic::{WaveError, WaveKind, WaveTables, uses_partials, waveform_n};
use crate::pulse::{self, PulseNode};
use crate::shaper::ShaperKind;
use crate::supersaw::{self, SupersawNode};

/// The fixed `gainNode(0.3)` after every synth source (synth.mjs).
pub const SYNTH_GAIN: f32 = 0.3;
/// A voice whose output stays below this for a whole quantum after its sources stop is freed.
pub const SILENCE_FLOOR: f32 = 1e-10;
/// The amp envelope defaults of synth voices (`getADSRValues(.., 'linear', defaults)`).
pub const AMP_ADSR_DEFAULTS: [f64; 4] = [0.001, 0.05, 0.6, 0.01];
/// The pitch envelope defaults (`getPitchEnvelope`).
pub const PITCH_ADSR_DEFAULTS: [f64; 4] = [0.2, 0.001, 1.0, 0.001];
/// The filter envelope defaults (`createFilter`: `getADSRValues(.., 'exponential', ..)`).
pub const FILTER_ADSR_DEFAULTS: [f64; 4] = [0.005, 0.14, 0.0, 0.1];
/// `clamp(.., 0, 20000)` of the filter envelope's range.
pub const FILTER_ENV_MAX: f64 = 20000.0;

/// When the voice is scheduled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartInfo {
    /// Onset in seconds after the lateness clamp.
    pub t: f64,
    /// Hap duration in seconds.
    pub hap_duration: f64,
    /// The pause frame the voice is scheduled at.
    pub now_frame: u64,
    /// The musical onset used for cycle-synced LFO phase.
    pub cycle: f64,
    /// Elapsed layout seconds at the onset for fixed-Hz modulation.
    pub phase_sec: f64,
    /// The local cycles per second at this onset.
    pub cps: f64,
}

/// The voice's effect sends (`effectSend`: `post -> gainNode(level) -> bus`), taken after `post`
/// and before `dry` (superdough.mjs:954-978), each a voice-owned `GainNode` (`createGain()`, then
/// `.value = level`, which also schedules `setValueAtTime(level, now)`) applied inside the voice.
/// The delay send feeds the orbit's feedback delay; the reverb send creates or updates the orbit's
/// reverb and feeds its convolver.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sends {
    /// `gainNode(delay)` into the orbit's feedback delay, when `delay > 0 && delaytime > 0 &&
    /// delayfeedback > 0` (`delay_mix`).
    pub delay: Option<GainNode>,
    /// `gainNode(room)` into the orbit's reverb, when `room > 0` (`room_mix`).
    pub room: Option<GainNode>,
    /// `effectSend(post, getBus(bus), busgain)` when `bus != null`: the bus's index in the job's
    /// [`BusBank`] and the voice-owned `gainNode(busgain)` (`createGain()`, then `.value =
    /// busgain`, which also schedules `setValueAtTime(busgain, now)`).
    pub bus: Option<(usize, GainNode)>,
}

impl Sends {
    /// Superdough's sends in order for a hap at `t` scheduled at frame `now`, each recorded in
    /// `self` once connected: when the delay condition holds, `orbit.getDelay(delaytime,
    /// delayfeedback, t)` (creates the orbit's delay or runs its guard; it can throw) and then
    /// `gainNode(delay)` (`.value = level`: a WebIDL float, so a non-finite level throws, after
    /// `getDelay` changed the orbit); then, when `room > 0`, `orbit.getReverb(roomsize, ...)`
    /// (an IR request can throw: the `ir` length error, or the lowpass's `TypeError`) and
    /// `gainNode(room)`; then, when `bus != null` (`bus` is its index in the job's bank),
    /// `gainNode(busgain)` (a non-finite `busgain` throws). On an error the sends connected before
    /// it stay in `self`.
    pub fn connect(
        &mut self,
        params: &VoiceParams,
        orbit: &mut Orbit,
        t: f64,
        now: u64,
        bus: Option<usize>,
    ) -> Result<(), VoiceError> {
        let m = &params.main;
        if delay_applies(m) {
            orbit.get_delay(m.delaytime, m.delaytime_is_number, m.delayfeedback, t, now)?;
            self.delay = Some(send_gain(m.delay, now)?);
        }
        if m.room > 0.0 {
            orbit.get_reverb(
                m.roomsize.as_ref(),
                m.roomfade.as_ref(),
                m.roomlp.as_ref(),
                m.roomdim.as_ref(),
            )?;
            self.room = Some(send_gain(m.room, now)?);
        }
        if let Some(bus) = bus {
            self.bus = Some((bus, send_gain(params.busgain, now)?));
        }
        Ok(())
    }

    /// Whether any send is connected.
    pub fn any(&self) -> bool {
        self.delay.is_some() || self.room.is_some() || self.bus.is_some()
    }
}

/// `gainNode(level)` at frame `now`: `createGain()`, then `.value = level` (a WebIDL float, so a
/// non-finite level throws; it also schedules `setValueAtTime(level, now)`).
fn send_gain(level: f64, now: u64) -> Result<GainNode, VoiceError> {
    let mut g = GainNode::new(Param::gain(1.0));
    g.gain.set_value(idl_float(level, "gain")?, now)?;
    Ok(g)
}

/// Applies a send's gain to one quantum of `post` (`channels` of `post`, `silent` its hint).
fn send_buf(
    g: &mut GainNode,
    q: u64,
    post: &[[f32; QUANTUM]; 2],
    channels: usize,
    silent: bool,
    inputs: &ParamInputs,
) -> SendBuf {
    let mut send = SendBuf {
        ch: *post,
        channels,
        silent,
    };
    send.silent = g.process(q, &mut send.ch[..channels], silent, inputs);
    send
}

/// One quantum of a send: `post` times the send's gain (1 or 2 channels) and its silence hint.
#[derive(Debug, Clone, PartialEq)]
pub struct SendBuf {
    /// Only `ch[..channels]` is meaningful.
    pub ch: [[f32; QUANTUM]; 2],
    pub channels: usize,
    pub silent: bool,
}

impl SendBuf {
    /// The send as a connection into its effect.
    pub fn input(&self) -> crate::orbit::Input<'_> {
        crate::orbit::Input {
            channels: &self.ch[..self.channels.clamp(1, 2)],
            silent: self.silent,
        }
    }
}

/// A voice's sends of one quantum: `post` scaled by each send's gain (the delay and reverb sends
/// feed the voice's orbit, the bus send the bus with its index).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendBuffers {
    pub delay: Option<SendBuf>,
    pub room: Option<SendBuf>,
    pub bus: Option<(usize, SendBuf)>,
}

/// One quantum of a voice's output.
#[derive(Debug, Clone)]
pub struct VoiceOutput {
    /// `post` (the last gain of the chain), the tap of the sends: channel 0 only for a mono
    /// voice.
    pub post: [[f32; QUANTUM]; 2],
    pub post_silent: bool,
    /// What the voice connects to its orbit's summing node: `Gain(dry)` of `post` when `dry` is
    /// set, else `post` itself.
    pub out: [[f32; QUANTUM]; 2],
    pub out_silent: bool,
    /// 1 for a mono voice, 2 behind a panner.
    pub channels: usize,
    /// The voice's sends of this quantum (`post` scaled by each send gain).
    pub sends: SendBuffers,
    /// The voice's modulators' outputs into its orbit's own params this quantum,
    /// in connection order: one entry per connection and summand (`None`: silent, but connected).
    pub orbit_inputs: Vec<(OrbitParam, Option<[f32; QUANTUM]>)>,
}

impl Default for VoiceOutput {
    fn default() -> Self {
        VoiceOutput::new()
    }
}

impl VoiceOutput {
    /// A silent mono output.
    pub fn new() -> VoiceOutput {
        VoiceOutput {
            post: [[0.0; QUANTUM]; 2],
            post_silent: true,
            out: [[0.0; QUANTUM]; 2],
            out_silent: true,
            channels: 1,
            sends: SendBuffers::default(),
            orbit_inputs: Vec::new(),
        }
    }

    /// The connection into the orbit's summing node.
    pub fn to_orbit(&self) -> crate::orbit::Input<'_> {
        crate::orbit::Input {
            channels: &self.out[..self.channels.clamp(1, 2)],
            silent: self.out_silent,
        }
    }

    /// The send tap (`post`).
    pub fn to_sends(&self) -> crate::orbit::Input<'_> {
        crate::orbit::Input {
            channels: &self.post[..self.channels.clamp(1, 2)],
            silent: self.post_silent,
        }
    }
}

/// Why a voice could not be built (superdough throws; nothing of the voice sounds).
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceError {
    pub kind: ErrorKind,
    pub detail: String,
}

pub(crate) fn hap_error(detail: String) -> VoiceError {
    VoiceError {
        kind: ErrorKind::Hap,
        detail,
    }
}

impl From<crate::reverb::IrError> for VoiceError {
    fn from(e: crate::reverb::IrError) -> Self {
        VoiceError {
            kind: e.kind(),
            detail: e.to_string(),
        }
    }
}

impl From<ControlError> for VoiceError {
    fn from(e: ControlError) -> Self {
        hap_error(e.to_string())
    }
}

impl From<ParamError> for VoiceError {
    fn from(e: ParamError) -> Self {
        hap_error(e.to_string())
    }
}

impl From<ScheduleError> for VoiceError {
    fn from(e: ScheduleError) -> Self {
        hap_error(e.to_string())
    }
}

impl From<WaveError> for VoiceError {
    fn from(e: WaveError) -> Self {
        hap_error(e.to_string())
    }
}

fn type_error(what: &str) -> VoiceError {
    hap_error(format!(
        "TypeError: {what} is not a finite floating-point value"
    ))
}

/// WebIDL `float` conversion (AudioParam values): NaN, infinities and values that round to an
/// infinite `f32` throw a `TypeError`.
pub(crate) fn idl_float(x: f64, what: &str) -> Result<f32, VoiceError> {
    let y = x as f32;
    if x.is_finite() && y.is_finite() {
        Ok(y)
    } else {
        Err(type_error(what))
    }
}

/// WebIDL `double` conversion (event and stop times): NaN and infinities throw a `TypeError`.
fn idl_double(x: f64, what: &str) -> Result<f64, VoiceError> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(type_error(what))
    }
}

/// The vibrato of a carrier: a sine `OscillatorNode` at `vib` Hz into `Gain(vibmod * 100)`,
/// connected to the carrier's `detune` (cents).
#[derive(Debug, Clone)]
pub struct Vibrato {
    pub osc: Oscillator,
    pub depth: Param,
}

impl Vibrato {
    /// One quantum of the vibrato chain into the carrier's `detune`: the oscillator (with the
    /// modulators of its `frequency` and `detune`) through the depth gain (with those of its
    /// `gain`).
    fn render(&mut self, q: u64, m: &ModView) -> [f32; QUANTUM] {
        let mut buf = [0.0f32; QUANTUM];
        render_osc(&mut self.osc, q, &mut buf, Node::Vibrato, m);
        let inputs = m.inputs(Node::VibratoGain, ParamSlot(0));
        apply_gain_connected(&mut self.depth, q, &mut buf, &inputs);
        buf
    }
}

/// An oscillator with no input of its own but the modulators connected to its `frequency` and
/// `detune` (as `node`).
fn render_osc(
    o: &mut Oscillator,
    q: u64,
    out: &mut [f32; QUANTUM],
    node: Node,
    m: &ModView,
) -> bool {
    let (freq, det) = (ParamSlot(0), ParamSlot(1));
    let mut f = |_: u64, bufs: &mut Vec<[f32; QUANTUM]>| m.extend(node, freq, bufs);
    let mut d = |_: u64, bufs: &mut Vec<[f32; QUANTUM]>| m.extend(node, det, bufs);
    let fi: Option<Input> = if m.connected(node, freq) {
        Some(&mut f)
    } else {
        None
    };
    let di: Option<Input> = if m.connected(node, det) {
        Some(&mut d)
    } else {
        None
    };
    o.process(q, out, fi, di)
}

/// An oscillator voice's source: the carrier, its modulators and the noise mix.
#[derive(Debug, Clone)]
pub struct OscSource {
    pub carrier: Oscillator,
    pub vibrato: Option<Vibrato>,
    /// The FM modulators and their connections (`applyFM`), when any matrix entry is active.
    pub fm: Option<Fm>,
    /// `drywet(osc, pinkNoise, noise)` when `noise` is truthy.
    pub noise_mix: Option<NoiseMix>,
}

impl OscSource {
    /// The source's output for one quantum (whether it carries signal): the carrier, whose
    /// `frequency` pulls the FM connections and whose `detune` pulls the vibrato chain only when
    /// it evaluates its params (the quanta it plays), then the noise mix.
    /// The modulators connected to the carrier's `frequency` and `detune`, the vibrato's
    /// oscillator and depth gain and the FM nodes (`m`) are summed after their own inputs.
    fn process(&mut self, quantum_start: u64, out: &mut [f32; QUANTUM], m: &ModView) -> bool {
        let OscSource {
            carrier,
            vibrato,
            fm,
            noise_mix,
        } = self;
        let (freq, det) = (ParamSlot(0), ParamSlot(1));
        let fm_connected = fm.as_ref().is_some_and(|f| f.connects(FmTarget::Carrier))
            || m.connected(Node::Source, freq);
        let vib_connected = vibrato.is_some() || m.connected(Node::Source, det);
        let mut fm_pull = |q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
            if let Some(fm) = fm.as_mut() {
                fm.inputs(FmTarget::Carrier, q, bufs, m);
            }
            m.extend(Node::Source, freq, bufs);
        };
        let mut vib_pull = |q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
            if let Some(vib) = vibrato.as_mut() {
                bufs.push(vib.render(q, m));
            }
            m.extend(Node::Source, det, bufs);
        };
        let fm_input: Option<Input> = if fm_connected {
            Some(&mut fm_pull)
        } else {
            None
        };
        let vib_input: Option<Input> = if vib_connected {
            Some(&mut vib_pull)
        } else {
            None
        };
        let playing = carrier.process(quantum_start, out, fm_input, vib_input);
        match noise_mix {
            Some(mix) => mix.process(quantum_start, out, !playing),
            None => playing,
        }
    }

    /// `stop(time)` of `getOscillator`: the FM modulators, the vibrato, the noise mix's pink
    /// source, then the carrier.
    fn stop(&mut self, time: f64) -> Result<(), ScheduleError> {
        if let Some(fm) = &mut self.fm {
            fm.stop(time)?;
        }
        if let Some(v) = &mut self.vibrato {
            v.osc.stop(time)?;
        }
        if let Some(m) = &mut self.noise_mix {
            m.pink.stop(time)?;
        }
        self.carrier.stop(time)
    }
}

/// `wetfade(d)` (helpers.mjs): 1 below 0.5, then `1 - (d - 0.5) / 0.5` (0 at 1, negative
/// beyond; NaN stays NaN).
pub fn wetfade(d: f64) -> f64 {
    if d < 0.5 { 1.0 } else { 1.0 - (d - 0.5) / 0.5 }
}

/// The `noise` mix of an oscillator voice (`getNoiseMix`): the oscillator through
/// `Gain(wetfade(noise))` and the cached pink buffer (a looping source started at `t`) through
/// `Gain(wetfade(1 - noise))`, summed by a unity `GainNode`.
#[derive(Debug, Clone)]
pub struct NoiseMix {
    pub pink: NoiseSource,
    pub dry: f32,
    pub wet: f32,
}

impl NoiseMix {
    /// Mixes the pink noise into `out` (the oscillator's output, `osc_silent` its silence hint);
    /// returns whether the mix carries signal.
    fn process(&mut self, quantum_start: u64, out: &mut [f32; QUANTUM], osc_silent: bool) -> bool {
        let mut dry = [*out];
        let dry_silent = scale(&mut dry, self.dry, osc_silent);
        let mut wet = [[0.0f32; QUANTUM]];
        let playing = self.pink.process(quantum_start, &mut wet[0]);
        let wet_silent = scale(&mut wet, self.wet, !playing);
        // The mix's input sums its connections (`SumFrom` skips a silent one and copies the
        // first one into the silent summing bus).
        match (dry_silent, wet_silent) {
            (true, true) => {
                out.fill(0.0);
                false
            }
            (false, true) => {
                *out = dry[0];
                true
            }
            (true, false) => {
                *out = wet[0];
                true
            }
            (false, false) => {
                for ((o, d), w) in out.iter_mut().zip(dry[0].iter()).zip(wet[0].iter()) {
                    *o = *d + *w;
                }
                true
            }
        }
    }
}

/// What an FM modulator plays: an oscillator (`fmwave` a periodic type) or a looping noise
/// buffer (`fmwave` one of the noise names).
#[derive(Debug, Clone)]
pub enum ModSource {
    Osc(Oscillator),
    Noise(NoiseSource),
}

/// Where an FM connection goes: the carrier's `frequency`, or the `frequency` of the modulator
/// at this position in `Fm::modulators`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmTarget {
    Carrier,
    Modulator(usize),
}

/// One FM modulator (`fm()` in helpers.mjs) and the per-quantum state of its nodes: like
/// Blink's `ProcessIfNecessary`, each node renders once per quantum, and only when something
/// pulls it (the carrier evaluates its `frequency` only in the quanta it plays, so a modulator
/// started at the scheduling frame first plays in the carrier's first quantum). A node pulled
/// again in the same quantum (a cycle in the matrix, such as `fmi11`) hands out its output bus
/// as it is, which still holds the previous quantum's output.
#[derive(Debug, Clone)]
pub struct FmModulator {
    /// The modulator number, 1..=8.
    #[cfg_attr(not(test), allow(dead_code))] // the tests check the creation order
    pub index: usize,
    /// `None` only while the source renders.
    source: Option<ModSource>,
    /// `envGain`, when any of `fm{attack,decay,sustain,release}{i}` is defined.
    pub envelope: Option<GainNode>,
    /// `carrier.frequency.value * fmh` as the `Gain(modfreq)` float.
    pub freq: f32,
    source_out: [f32; QUANTUM],
    source_silent: bool,
    source_quantum: Option<u64>,
    env_out: [f32; QUANTUM],
    env_silent: bool,
    env_quantum: Option<u64>,
}

impl FmModulator {
    pub fn source(&self) -> &ModSource {
        self.source.as_ref().expect("not rendering")
    }
}

/// One active matrix entry: modulator output -> `Gain(amount)` -> `Gain(freq)` -> target.
#[derive(Debug, Clone)]
pub struct FmConnection {
    /// The source modulator's position in `Fm::modulators`.
    pub source: usize,
    pub target: FmTarget,
    pub amount: GainNode,
    pub freq: GainNode,
}

/// The FM graph of one voice: the modulators in creation order and the connections in matrix
/// order (the order a param sums them in; Blink keeps a param's connections in a hash set, so
/// with two or more into one param its summing order, and the last bit of the sum, may differ).
#[derive(Debug, Clone)]
pub struct Fm {
    modulators: Vec<FmModulator>,
    connections: Vec<FmConnection>,
}

impl Fm {
    #[cfg(test)]
    pub fn modulators(&self) -> &[FmModulator] {
        &self.modulators
    }

    #[cfg(test)]
    pub fn connections(&self) -> &[FmConnection] {
        &self.connections
    }

    /// Whether `target`'s `frequency` has a connection.
    pub fn connects(&self, target: FmTarget) -> bool {
        self.connections.iter().any(|c| c.target == target)
    }

    /// The signals of the connections into `target` for the quantum at `q`, in connection
    /// order, silent ones left out.
    fn inputs(&mut self, target: FmTarget, q: u64, bufs: &mut Vec<[f32; QUANTUM]>, m: &ModView) {
        for k in 0..self.connections.len() {
            let c = &self.connections[k];
            if c.target != target {
                continue;
            }
            let source = c.source;
            let (buf, silent) = self.output(source, q, m);
            let c = &mut self.connections[k];
            let mut ch = [buf];
            let amount = m.inputs(Node::FmAmount(k), ParamSlot(0));
            let silent = c.amount.process(q, &mut ch, silent, &amount);
            let silent = c.freq.process(q, &mut ch, silent, &ParamInputs::NONE);
            if !silent {
                bufs.push(ch[0]);
            }
        }
    }

    /// Modulator `m`'s output (its envelope's, if it has one) and its silence hint.
    fn output(&mut self, m: usize, q: u64, mv: &ModView) -> ([f32; QUANTUM], bool) {
        if self.modulators[m].envelope.is_none() {
            return self.source_output(m, q, mv);
        }
        if self.modulators[m].env_quantum == Some(q) {
            let md = &self.modulators[m];
            return (md.env_out, md.env_silent);
        }
        self.modulators[m].env_quantum = Some(q);
        let (buf, silent) = self.source_output(m, q, mv);
        let md = &mut self.modulators[m];
        let mut ch = [buf];
        let env = md.envelope.as_mut().expect("checked above");
        md.env_silent = env.process(q, &mut ch, silent, &ParamInputs::NONE);
        md.env_out = ch[0];
        (md.env_out, md.env_silent)
    }

    /// Modulator `m`'s source output, rendered on its first pull in the quantum.
    fn source_output(&mut self, m: usize, q: u64, mv: &ModView) -> ([f32; QUANTUM], bool) {
        if self.modulators[m].source_quantum == Some(q) {
            let md = &self.modulators[m];
            return (md.source_out, md.source_silent);
        }
        self.modulators[m].source_quantum = Some(q);
        let mut source = self.modulators[m].source.take().expect("not rendering");
        let mut out = [0.0f32; QUANTUM];
        let node = Node::FmOsc(m);
        let playing = match &mut source {
            ModSource::Osc(o) => {
                let target = FmTarget::Modulator(m);
                let (freq, det) = (ParamSlot(0), ParamSlot(1));
                if self.connects(target) || mv.connected(node, freq) {
                    let mut pull = |q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
                        self.inputs(target, q, bufs, mv);
                        mv.extend(node, freq, bufs);
                    };
                    let mut det_pull =
                        |_: u64, bufs: &mut Vec<[f32; QUANTUM]>| mv.extend(node, det, bufs);
                    let di: Option<Input> = if mv.connected(node, det) {
                        Some(&mut det_pull)
                    } else {
                        None
                    };
                    o.process(q, &mut out, Some(&mut pull), di)
                } else {
                    render_osc(o, q, &mut out, node, mv)
                }
            }
            ModSource::Noise(n) => n.process(q, &mut out),
        };
        let md = &mut self.modulators[m];
        md.source = Some(source);
        md.source_out = out;
        md.source_silent = !playing;
        (out, !playing)
    }

    /// `stop(t)` of every modulator, in creation order (`toStop`).
    fn stop(&mut self, time: f64) -> Result<(), ScheduleError> {
        for m in &mut self.modulators {
            match m.source.as_mut().expect("not rendering") {
                ModSource::Osc(o) => o.stop(time)?,
                ModSource::Noise(n) => n.stop(time)?,
            }
        }
        Ok(())
    }
}

/// The `OscillatorType` a modulator's `fmwave` sets (`osc.type = type`: the value as a string;
/// `'custom'` throws, other names are ignored and leave `'sine'`).
fn modulator_wave(wave: Option<&Value>) -> Result<WaveKind, VoiceError> {
    let name = wave.map_or_else(|| "sine".to_string(), js_to_string);
    match name.as_str() {
        "custom" => Err(hap_error(
            "InvalidStateError: Failed to set the 'type' property on 'OscillatorNode': 'type' \
             cannot be set directly to 'custom'.  Use setPeriodicWave() to create a custom \
             Oscillator type."
                .to_string(),
        )),
        "square" => Ok(WaveKind::Square),
        "sawtooth" => Ok(WaveKind::Sawtooth),
        "triangle" => Ok(WaveKind::Triangle),
        _ => Ok(WaveKind::Sine),
    }
}

/// The noise a modulator plays instead of an oscillator, when its `fmwave` is a noise name.
pub(crate) fn noise_wave(p: &FmModParams) -> Option<NoiseKind> {
    match &p.wave {
        Some(Value::Str(w)) => NoiseKind::from_name(w),
        _ => None,
    }
}

/// Builds modulator `idx` (`fm(param, fmh ?? 1, fmwave ?? 'sine', voiceSeed)` and the optional
/// envelope) and returns it with its `modfreq = carrier_freq * ratio`: a noise wave is a looping
/// noise buffer (`getNoiseBuffer(type, 2, voiceSeed)`), any other wave an oscillator with
/// `frequency.value = modfreq`; both `start()` at the scheduling frame. The envelope is
/// `getADSRValues` with its own defaults, 0..1 over `[begin, begin + duration]`.
fn build_modulator(
    idx: usize,
    mods: &[FmModParams],
    carrier_freq: f32,
    begin: f64,
    ctx: &mut BuildContext,
) -> Result<(FmModulator, f64), VoiceError> {
    let p = &mods[idx - 1];
    let now = ctx.now;
    let modfreq = f64::from(carrier_freq) * p.ratio;
    let source = match noise_wave(p) {
        Some(kind) => {
            let mut n = NoiseSource::new(ctx.noise.get(kind, FM_DENSITY, ctx.seed));
            n.start(0.0, now)?;
            ModSource::Noise(n)
        }
        None => {
            let kind = modulator_wave(p.wave.as_ref())?;
            let mut o = Oscillator::new(ctx.tables.builtin(kind).expect("a built-in kind"));
            o.frequency
                .set_value(idl_float(modfreq, "frequency")?, now)?;
            o.start(0.0, now)?;
            ModSource::Osc(o)
        }
    };
    let envelope = if p.has_envelope {
        let mut env = Param::gain(1.0);
        let [a, d, s, r] = get_adsr_values(p.adsr, Curve::Linear, crate::adsr::DEFAULT_ADSR);
        let curve = if p.exponential {
            Curve::Exponential
        } else {
            Curve::Linear
        };
        let end = begin + ctx.duration;
        get_param_adsr(&mut env, a, d, s, r, 0.0, 1.0, begin, end, curve, now)?;
        Some(GainNode::new(env))
    } else {
        None
    };
    let modulator = FmModulator {
        index: idx,
        source: Some(source),
        envelope,
        freq: 0.0,
        source_out: [0.0; QUANTUM],
        source_silent: true,
        source_quantum: None,
        env_out: [0.0; QUANTUM],
        env_silent: true,
        env_quantum: None,
    };
    Ok((modulator, modfreq))
}

/// What building a voice's sources needs besides its controls.
struct BuildContext<'a> {
    tables: &'a mut WaveTables,
    noise: &'a mut NoiseCache,
    /// The voice seed (crackle buffers).
    seed: u32,
    /// The hap duration (FM envelopes).
    duration: f64,
    /// The scheduling frame.
    now: u64,
}

/// `applyFM(carrier.frequency, value, begin)` (helpers.mjs:429-512): walks the matrix in its
/// order, creating each modulator on first use (source side, then target side) and checking
/// each side's `gainNode(amt)` and `gainNode(modfreq)` as superdough builds them (the target
/// side's pair carries no signal); a connection to a noise modulator (no `frequency` param) is
/// skipped (`NodeTable::plan` registers the `fm_<i>` and `fm_<i>_gain` entries of this walk).
/// `None` when no entry is active.
fn apply_fm(
    params: &VoiceParams,
    carrier_freq: f32,
    begin: f64,
    ctx: &mut BuildContext,
) -> Result<Option<Fm>, VoiceError> {
    if params.fm.entries.is_empty() {
        return Ok(None);
    }
    let mut fm = Fm {
        modulators: Vec::new(),
        connections: Vec::new(),
    };
    // Modulator number -> (position, modfreq).
    let mut created: [Option<(usize, f64)>; 9] = [None; 9];
    for e in &params.fm.entries {
        let mut sides = [None, None];
        for (k, idx) in [e.source, e.target].into_iter().enumerate() {
            if idx == 0 {
                continue;
            }
            if created[idx].is_none() {
                let (m, modfreq) = build_modulator(idx, &params.fm.mods, carrier_freq, begin, ctx)?;
                created[idx] = Some((fm.modulators.len(), modfreq));
                fm.modulators.push(m);
            }
            let (pos, modfreq) = created[idx].expect("created above");
            let amount = idl_float(e.amount, "gain")?;
            let freq = idl_float(modfreq, "gain")?;
            fm.modulators[pos].freq = freq;
            sides[k] = Some((pos, amount, freq));
        }
        let (source, amount, freq) = sides[0].expect("the source is a modulator");
        let target = match sides[1] {
            None => FmTarget::Carrier,
            Some((pos, ..)) => {
                if matches!(fm.modulators[pos].source(), ModSource::Noise(_)) {
                    // superdough logs "failed to connect FM ..." and goes on.
                    continue;
                }
                FmTarget::Modulator(pos)
            }
        };
        fm.connections.push(FmConnection {
            source,
            target,
            amount: GainNode::constant(amount),
            freq: GainNode::constant(freq),
        });
    }
    Ok(Some(fm))
}

/// `getNoiseOscillator(type, t, density, voiceSeed)`: a looping source over the noise buffer,
/// started at `t`.
fn noise_source(
    kind: NoiseKind,
    density: f64,
    t: f64,
    ctx: &mut BuildContext,
) -> Result<NoiseSource, VoiceError> {
    let mut n = NoiseSource::new(ctx.noise.get(kind, density, ctx.seed));
    n.start(t, ctx.now)?;
    Ok(n)
}

/// What the voice's source is.
#[derive(Debug, Clone)]
pub enum Source {
    /// A periodic-wave oscillator (every oscillator sound).
    Osc(Box<OscSource>),
    /// `ConstantSourceNode(1)` (`s: 'one'`).
    Constant(ConstantSource),
    /// A looping noise buffer source (`white`, `pink`, `brown`, `crackle`).
    Noise(NoiseSource),
    /// A worklet synth (`supersaw`, `pulse`).
    Worklet(Box<WorkletSource>),
    /// The `bus` sound: a bus of the job's [`BusBank`] connected to `envGain` from the pause
    /// frame until `stop_frame`.
    Bus(BusSource),
}

/// The `bus` sound's source (synth.mjs:374-407): `getBus(value.n ?? 0)` connected to `envGain`
/// from the build on (no start, no `Gain(0.3)`; silent until the envelope opens) and
/// disconnected by a `webAudioTimeout` at `end = holdEnd + release + 0.01`, after the envelope
/// reached 0. The engine disconnects at `stop_frame = RoundUp128(ceil(end * sr))` (any frame of
/// that window gives the same audio; this one is deterministic). Stereo: the bus is a 2-channel
/// explicit sum.
#[derive(Debug, Clone, PartialEq)]
pub struct BusSource {
    /// The bus's index in the job's [`BusBank`].
    pub bus: usize,
    /// The first quantum start that no longer reads the bus.
    pub stop_frame: u64,
}

impl BusSource {
    /// Reads the bus for the quantum starting at `q` into `out` (2 channels); returns whether the
    /// output is not silent.
    fn process(&self, q: u64, out: &mut VoiceBus, buses: &mut BusBank) -> bool {
        out.channels = 2;
        if q >= self.stop_frame {
            out.ch = [[0.0; QUANTUM]; 2];
            out.silent = true;
            return false;
        }
        let (ch, silent) = buses.read(self.bus);
        out.ch = ch;
        out.silent = silent;
        !silent
    }
}

/// The `bus` sound's ADSR defaults (`getADSRValues(.., 'linear', [0.001, 0.05, 1, 0.01])`:
/// sustain 1, unlike the synths' 0.6).
pub const BUS_ADSR_DEFAULTS: [f64; 4] = [0.001, 0.05, 1.0, 0.01];

/// One scheduled synth hap.
#[derive(Debug, Clone)]
pub struct Voice {
    source: Source,
    #[cfg(test)]
    start_frame: u64,
    stop_frame: u64,
    /// The chain in order: `Gain(0.3)`, `envGain` (`gainNode(1)`, then `getParamADSR` on its
    /// gain), the stages of each FX entry ([`build_entry`]: the gain stage, the filters, the
    /// vowel, the shapers, the tremolo, the compressor, the panner, the phaser, and a chain
    /// entry's local delay and reverb), the chain entries first and the main entry last, the
    /// `FXrelease` gain, then `post` (`new GainNode(ac, { gain: postgain })`).
    stages: Vec<Stage>,
    /// The delay and reverb sends from `post`.
    sends: Sends,
    /// `new GainNode(ac, { gain: dry })` when `dry != null`; forks from `post` to the orbit only.
    dry: Option<GainNode>,
    /// Superdough's `nodes` tracker for this hap (the plan, then the modulators' entries).
    node_table: NodeTable,
    /// The `lfo`, `env` and `bmod` modulators and their connections.
    mods: ModSet,
    /// Superdough's error logs of the modulators the build skipped, not yet taken by the driver.
    logs: Vec<String>,
    /// The filter LFOs, each connected to a stage's `frequency` and rendered just before it.
    param_lfos: Vec<ParamLfo>,
    /// When each stage's input loses its connection after the source stops.
    teardown: Teardown,
    /// The output channel count (`channels_out` folded over `stages` from the mono source).
    channels: usize,
    /// Whether `post` (or `Gain(dry)`) is connected to the orbit's summing node: false when the
    /// build threw after a send was connected (the voice then sounds only through its sends).
    connected: bool,
    /// Set once the output became non-finite (the voice is silenced; see [`Voice::process`]).
    failed: bool,
    /// Processor errors met while rendering, not yet taken by the driver (Chromium outputs
    /// silence for good after one and the engine reports it).
    errors: Vec<String>,
    /// Superdough's `value._voiceSeed` for this hap.
    #[cfg(test)]
    seed: u32,
}

/// A `GainNode` in the voice chain: its gain param and its `AudioHandler` silence state.
#[derive(Debug, Clone, PartialEq)]
pub struct GainNode {
    pub gain: Param,
    last_non_silent_time: f64,
}

impl GainNode {
    pub fn new(gain: Param) -> GainNode {
        GainNode {
            gain,
            last_non_silent_time: 0.0,
        }
    }

    /// `new GainNode(ac, { gain: g })` with no timeline event: default and intrinsic value `g`.
    /// It processes exactly as [`scale`] by `g` (same samples, `-0` signs and silence hints); a
    /// `set_value` would add a `setValueAtTime` event, making its first quantum sample accurate.
    pub fn constant(g: f32) -> GainNode {
        GainNode::new(Param::gain(g))
    }

    /// `ProcessIfNecessary` + `GainHandler::Process` on `channels` in place (tail time 0):
    /// silent input past the last signal outputs silence and only evaluates the param
    /// (`ProcessOnlyAudioParams`); else the gain is applied as [`apply_gain`] does, a silent
    /// input bus (`CopyWith*From` zeroes) or a quantum gain of exactly 0 giving a silent output.
    /// Returns the output's silence hint.
    ///
    /// `inputs` are the connections into `gain` this quantum: with a connection the param is
    /// sample accurate every quantum (the timeline or intrinsic value plus each input per frame,
    /// NaN replaced by `default_value()`, then `Vclip`), also while the input is silent (the
    /// connected nodes are still pulled).
    pub fn process(
        &mut self,
        quantum_start: u64,
        channels: &mut [[f32; QUANTUM]],
        input_silent: bool,
        inputs: &ParamInputs,
    ) -> bool {
        self.process_observing_gain(quantum_start, channels, input_silent, inputs, None)
    }

    /// Optionally records the exact gain applied by the orbit output for stem capture.
    pub(crate) fn process_observing_gain(
        &mut self,
        quantum_start: u64,
        channels: &mut [[f32; QUANTUM]],
        input_silent: bool,
        inputs: &ParamInputs,
        observed: Option<&mut [f32; QUANTUM]>,
    ) -> bool {
        let now = quantum_start as f64 / SAMPLE_RATE_F64;
        if input_silent && self.last_non_silent_time < now {
            for c in channels.iter_mut() {
                c.fill(0.0);
            }
            let mut scratch = [0.0f32; QUANTUM];
            self.gain.fill_quantum_connected(
                quantum_start,
                observed.unwrap_or(&mut scratch),
                &inputs.bufs,
                inputs.connected,
            );
            return true;
        }
        let silent = gain_handler(
            &mut self.gain,
            quantum_start,
            channels,
            input_silent,
            inputs,
            observed,
        );
        if !input_silent {
            self.last_non_silent_time = crate::quantum_end_time(quantum_start);
        }
        silent
    }
}

use crate::SAMPLE_RATE_F64;

/// `GainHandler::Process` on every channel; returns whether the output bus is silent.
fn gain_handler(
    gain: &mut Param,
    quantum_start: u64,
    channels: &mut [[f32; QUANTUM]],
    input_silent: bool,
    inputs: &ParamInputs,
    observed: Option<&mut [f32; QUANTUM]>,
) -> bool {
    let sample_accurate = gain.has_sample_accurate_values(quantum_start, inputs.connected);
    if sample_accurate && gain.rate() == Rate::A {
        let mut scratch = [0.0f32; QUANTUM];
        let values = observed.unwrap_or(&mut scratch);
        gain.fill_quantum_connected(quantum_start, values, &inputs.bufs, inputs.connected);
        if input_silent {
            zero(channels);
            return true;
        }
        for c in channels.iter_mut() {
            for (x, g) in c.iter_mut().zip(values.iter()) {
                *x *= *g;
            }
        }
        return false;
    }
    let g = if sample_accurate {
        gain.final_value_of_bufs(quantum_start, &inputs.bufs, inputs.connected)
    } else {
        gain.audio_value(quantum_start)
    };
    if let Some(observed) = observed {
        observed.fill(g);
    }
    scale(channels, g, input_silent)
}

pub(crate) fn zero(channels: &mut [[f32; QUANTUM]]) {
    for c in channels.iter_mut() {
        c.fill(0.0);
    }
}

/// A constant gain (`CopyWithGainFrom`): exactly 0 or a silent input gives a silent zero bus,
/// else a float multiply. Returns whether the output is silent.
pub(crate) fn scale(channels: &mut [[f32; QUANTUM]], g: f32, input_silent: bool) -> bool {
    if g == 0.0 || input_silent {
        zero(channels);
        return true;
    }
    for c in channels.iter_mut() {
        for x in c.iter_mut() {
            *x *= g;
        }
    }
    false
}

/// The waveform a sound name registers (`waveformAliases`).
fn canonical(s: &str) -> &str {
    match s {
        "sin" => "sine",
        "saw" => "sawtooth",
        "sqr" => "square",
        "tri" => "triangle",
        other => other,
    }
}

/// `getOscillator(s, t, value)` (synth.mjs:505-569) for the periodic-wave sounds: the carrier
/// (built-in or `waveformN` table, `frequency.value`), vibrato, pitch envelope, FM, the noise
/// mix, `start(t)`.
fn build_oscillator(
    s: &str,
    params: &VoiceParams,
    t: f64,
    ctx: &mut BuildContext,
) -> Result<OscSource, VoiceError> {
    let now = ctx.now;
    let tables = &mut *ctx.tables;
    let partials = params.partials.as_ref();
    let truthy = partials.is_some_and(crate::controls::js_truthy);
    let s = if s == "user" && !truthy {
        "triangle"
    } else {
        s
    };
    let wave = if !uses_partials(partials) || s == "sine" {
        // `o.type = s`: 'user' is not an OscillatorType, so the assignment is ignored and the
        // node keeps its default 'sine'.
        let kind = WaveKind::builtin(s).unwrap_or(WaveKind::Sine);
        tables.builtin(kind).expect("a built-in kind")
    } else {
        let partials = partials.expect("uses_partials");
        let (real, imag) = waveform_n(partials, params.phases.as_ref(), s).map_err(hap_error)?;
        tables.custom(&real, &imag)?
    };
    let mut carrier = Oscillator::new(wave);
    let f = idl_float(frequency_from_value(params)?, "frequency")?;
    carrier.frequency.set_value(f, now)?;
    let vibrato = build_vibrato(params, t, tables, now)?;
    if params.pitch_env {
        pitch_envelope(&mut carrier.detune, params, t, t + params.duration, now)?;
    }
    // `fm()` reads `frequencyparam.value`: the carrier's float intrinsic value.
    let fm = apply_fm(params, carrier.frequency.value(), t, ctx)?;
    let noise_mix = match params.noise {
        Some(noise) => {
            let pink = noise_source(NoiseKind::Pink, DEFAULT_DENSITY, t, ctx)?;
            let dry = idl_float(wetfade(noise), "gain")?;
            let wet = idl_float(wetfade(1.0 - noise), "gain")?;
            Some(NoiseMix { pink, dry, wet })
        }
        None => None,
    };
    carrier.start(t, now)?;
    Ok(OscSource {
        carrier,
        vibrato,
        fm,
        noise_mix,
    })
}

/// `getVibratoOscillator(param, value, t)` (helpers.mjs:361-379): when `vib > 0`, a sine
/// oscillator at `vib` Hz started at `t` into `Gain(vibmod * 100)` (`vibmod` default 0.5).
fn build_vibrato(
    params: &VoiceParams,
    t: f64,
    tables: &mut WaveTables,
    now: u64,
) -> Result<Option<Vibrato>, VoiceError> {
    match params.vib {
        Some(vib) if vib > 0.0 => {
            let mut osc = Oscillator::new(tables.builtin(WaveKind::Sine).expect("sine"));
            osc.frequency.set_value(idl_float(vib, "frequency")?, now)?;
            let mut depth = Param::gain(1.0);
            let amount = params.vibmod.unwrap_or(0.5) * 100.0;
            depth.set_value(idl_float(amount, "gain")?, now)?;
            osc.start(t, now)?;
            Ok(Some(Vibrato { osc, depth }))
        }
        _ => Ok(None),
    }
}

/// The two worklet synths.
#[derive(Debug, Clone)]
pub enum WorkletSynth {
    Supersaw(SupersawNode),
    Pulse(PulseNode),
}

/// A worklet synth source (`supersaw`, `pulse`: stereo, no input) and the nodes connected to its
/// params: the vibrato into `detune` (a `Gain` node, so its silence hint is Blink's) and the FM
/// connections into `frequency`. The params are rendered every quantum while the processor
/// lives (`AudioWorkletHandler::Process` renders them before calling `process()`, whatever
/// `process()` then does), so the vibrato and the FM modulators are pulled from the voice's
/// scheduling frame on, not only from its onset; none of them is stopped before the processor
/// finishes (superdough stops them from the main thread after `end`, when nothing is audible).
#[derive(Debug, Clone)]
pub struct WorkletSource {
    pub synth: WorkletSynth,
    pub vibrato: Option<(Oscillator, GainNode)>,
    pub fm: Option<Fm>,
    /// The pulse's width-modulation `lfo` into `pulsewidth` (`pw_lfo`, when `pwsweep != 0`).
    pub pw_lfo: Option<Lfo>,
    /// The `end` param's value (the teardown and the lifetime of what follows).
    pub end: f32,
}

impl WorkletSource {
    /// One quantum of the source into `bus` (two channels). Returns the error the processor
    /// finished with in this quantum, if any.
    fn process(&mut self, q: u64, bus: &mut VoiceBus, m: &ModView) -> Option<String> {
        bus.channels = 2;
        let gone = match &self.synth {
            WorkletSynth::Supersaw(n) => n.gone(),
            WorkletSynth::Pulse(n) => n.gone(),
        };
        let mut out = [[0.0f32; QUANTUM]; 2];
        if gone {
            bus.ch = out;
            bus.silent = true;
            return None;
        }
        let mut detune = ParamInputs {
            bufs: Vec::new(),
            connected: self.vibrato.is_some(),
        };
        if let Some((osc, gain)) = self.vibrato.as_mut() {
            let mut buf = [[0.0f32; QUANTUM]];
            let playing = render_osc(osc, q, &mut buf[0], Node::Vibrato, m);
            let inputs = m.inputs(Node::VibratoGain, ParamSlot(0));
            if !gain.process(q, &mut buf, !playing, &inputs) {
                detune.bufs.push(buf[0]);
            }
        }
        let mut frequency = ParamInputs {
            bufs: Vec::new(),
            connected: self
                .fm
                .as_ref()
                .is_some_and(|f| f.connects(FmTarget::Carrier)),
        };
        if let Some(fm) = self.fm.as_mut() {
            fm.inputs(FmTarget::Carrier, q, &mut frequency.bufs, m);
        }
        let (silent, error) = match &mut self.synth {
            WorkletSynth::Supersaw(n) => {
                let mut inputs = supersaw::NO_INPUTS;
                inputs[supersaw::DETUNE] = detune;
                inputs[supersaw::FREQUENCY] = frequency;
                for (k, i) in inputs.iter_mut().enumerate() {
                    m.add(Node::Source, ParamSlot(k as u8), i);
                }
                let silent = n.render(q, &inputs, &mut out);
                (
                    silent,
                    n.take_error()
                        .map(|e| format!("{}: {}", e.processor, e.what)),
                )
            }
            WorkletSynth::Pulse(n) => {
                let mut inputs = pulse::NO_INPUTS;
                inputs[pulse::DETUNE] = detune;
                inputs[pulse::FREQUENCY] = frequency;
                let mut error = None;
                if let Some(lfo) = self.pw_lfo.as_mut() {
                    let mut buf = [0.0f32; QUANTUM];
                    let silent = lfo.render(q, &m.lfo_inputs(Node::PulseLfo), &mut buf);
                    let pw = &mut inputs[pulse::PULSEWIDTH];
                    pw.connected = true;
                    if !silent {
                        pw.bufs.push(buf);
                    }
                    error = lfo.take_error().map(|e| e.message());
                }
                for (k, i) in inputs.iter_mut().enumerate() {
                    m.add(Node::Source, ParamSlot(k as u8), i);
                }
                (n.render(q, &inputs, &mut out), error)
            }
        };
        bus.ch = out;
        bus.silent = silent;
        error
    }

    /// The source row of the teardown table.
    fn source_end(&self) -> SourceEnd {
        match self.synth {
            WorkletSynth::Supersaw(_) => SourceEnd::Supersaw(self.end),
            WorkletSynth::Pulse(_) => SourceEnd::Pulse(self.end),
        }
    }
}

/// `supersaw` (synth.mjs:153-219) and `pulse` (synth.mjs:297-372) up to their `envGain`, in
/// their order: the worklet node and its `.value =` sets (supersaw: `frequency, begin, end,
/// freqspread, voices, panspread`, each `undefined` replaced by the param's default; pulse:
/// `frequency, begin, end, pulsewidth`), the pitch envelope on `detune`, the vibrato into
/// `detune`, FM into `frequency`. Registers `source`, `vib`, `vib_gain` and `fm_<i>`. Returns the
/// source and the amp envelope's peak (`0.3 * (1 / sqrt(voices))` for supersaw, 1 for pulse).
fn build_worklet_synth(
    s: &str,
    params: &VoiceParams,
    t: f64,
    release: f64,
    ctx: &mut BuildContext,
) -> Result<(WorkletSource, f64), VoiceError> {
    let now = ctx.now;
    let frequency = frequency_from_value(params)?;
    let hold_end = t + params.duration;
    let end = hold_end + release + 0.01;
    let (mut synth, peak) = if s == "supersaw" {
        let voices = jsmath::clamp(params.unison, 1.0, 100.0);
        let panspread = if voices > 1.0 {
            jsmath::clamp(params.spread, 0.0, 1.0)
        } else {
            0.0
        };
        let mut n = SupersawNode::new(ctx.seed);
        let sets = [
            (supersaw::FREQUENCY, frequency),
            (supersaw::BEGIN, t),
            (supersaw::END, end),
            (supersaw::FREQSPREAD, params.freqspread),
            (supersaw::VOICES, voices),
            (supersaw::PANSPREAD, panspread),
        ];
        for (k, v) in sets {
            set_worklet_value(&mut n.params[k], v, now)?;
        }
        // `0.3 * gainAdjustment`, `gainAdjustment = 1 / Math.sqrt(voices)`.
        let gain_adjustment = 1.0 / voices.sqrt();
        (WorkletSynth::Supersaw(n), 0.3 * gain_adjustment)
    } else {
        let mut n = PulseNode::new();
        let sets = [
            (pulse::FREQUENCY, frequency),
            (pulse::BEGIN, t),
            (pulse::END, end),
            (pulse::PULSEWIDTH, params.pw),
        ];
        for (k, v) in sets {
            set_worklet_value(&mut n.params[k], v, now)?;
        }
        (WorkletSynth::Pulse(n), 1.0)
    };
    let (detune, carrier_freq) = match &mut synth {
        WorkletSynth::Supersaw(n) => {
            let f = n.params[supersaw::FREQUENCY].value();
            (&mut n.params[supersaw::DETUNE], f)
        }
        WorkletSynth::Pulse(n) => {
            let f = n.params[pulse::FREQUENCY].value();
            (&mut n.params[pulse::DETUNE], f)
        }
    };
    if params.pitch_env {
        pitch_envelope(detune, params, t, hold_end, now)?;
    }
    let vibrato =
        build_vibrato(params, t, ctx.tables, now)?.map(|v| (v.osc, GainNode::new(v.depth)));
    // `fm()` reads `frequencyparam.value`: the param's float intrinsic value.
    let fm = apply_fm(params, carrier_freq, t, ctx)?;
    Ok((
        WorkletSource {
            synth,
            vibrato,
            fm,
            pw_lfo: None,
            end: idl_float(end, "value")?,
        },
        peak,
    ))
}

/// The pulse's width modulation (synth.mjs:346-350): when `pwsweep != 0`, `getLfo(ac, {
/// frequency: pwrate, depth: pwsweep, begin, end })` (a triangle with `time = begin`, `dcoffset
/// -0.5`, `min = -0.5 * pwsweep`, `max = -0.5 * pwsweep + pwsweep`) connected to `pulsewidth`.
pub fn pulse_width_lfo(
    params: &VoiceParams,
    begin: f64,
    end: f64,
    now: u64,
) -> Result<Option<Lfo>, VoiceError> {
    if params.pwsweep == 0.0 {
        return Ok(None);
    }
    let props = LfoProps {
        frequency: Some(params.pwrate),
        depth: Some(params.pwsweep),
        begin: Some(begin),
        end: Some(end),
        ..LfoProps::default()
    };
    Ok(Some(get_lfo(&props, now)?))
}

/// `getPitchEnvelope(o.detune, value, t, holdEnd)` (helpers.mjs:341-358): `penv` semitones
/// (`nanFallback(penv, 1)`), `curve = ['linear', 'exponential'][pcurve ?? 0]`, ADSR defaults
/// `[0.2, 0.001, 1, 0.001]`, `panchor ?? psustain`, cents from `-cents * panchor` to
/// `cents - cents * panchor`.
pub fn pitch_envelope(
    detune: &mut Param,
    params: &VoiceParams,
    begin: f64,
    end: f64,
    now: u64,
) -> Result<(), VoiceError> {
    let penv = match params.penv {
        Some(x) if !x.is_nan() => x,
        _ => 1.0,
    };
    let curve = pitch_curve(params.pcurve.as_ref());
    let [a, d, s, r] = get_adsr_values(
        [
            params.pattack,
            params.pdecay,
            params.psustain,
            params.prelease,
        ],
        curve,
        PITCH_ADSR_DEFAULTS,
    );
    let anchor = params.panchor.unwrap_or(s);
    let cents = penv * 100.0;
    let min = 0.0 - cents * anchor;
    let max = cents - cents * anchor;
    get_param_adsr(detune, a, d, s, r, min, max, begin, end, curve, now)?;
    Ok(())
}

impl Voice {
    /// Builds the voice as `synth.mjs` and the superdough gain stage do, in their order (so the
    /// first throwing step is the one reported). `tables` holds the run's periodic waves,
    /// `noise` its noise buffers, `orbit` the bus the voice plays on (its delay and reverb are
    /// created or updated by the voice's sends, part way through the build, as superdough
    /// does). Any error is returned, also one after a send was connected (see [`Voice::build`]).
    /// The buses are a bank of the voice's own bus keys (its send, then the bus sound's read).
    #[cfg(test)]
    pub fn new(
        params: &VoiceParams,
        voice_seed: u32,
        start: StartInfo,
        tables: &mut WaveTables,
        noise: &mut NoiseCache,
        orbit: &mut Orbit,
    ) -> Result<Voice, VoiceError> {
        let buses = BusBank::of_keys(
            params
                .bus
                .iter()
                .cloned()
                .chain(std::iter::once(params.bus_sound_key.clone()))
                .collect(),
        );
        match Voice::build(params, voice_seed, start, tables, noise, orbit, &buses)? {
            (v, None) => Ok(v),
            (_, Some(e)) => Err(e),
        }
    }

    /// `Voice::new` as the driver needs it: when the build throws after a send was connected
    /// (an `ir` error from `getReverb`, a non-finite `room` or `dry` level), the source has
    /// started and the chain feeds those sends, but `post` never reaches the summing node
    /// (`connectToOutput` is not reached): the voice is returned unconnected with the error.
    /// `buses` holds every bus of the job (the bus send's and the bus sound's index).
    pub fn build(
        params: &VoiceParams,
        voice_seed: u32,
        start: StartInfo,
        tables: &mut WaveTables,
        noise: &mut NoiseCache,
        orbit: &mut Orbit,
        buses: &BusBank,
    ) -> Result<(Voice, Option<VoiceError>), VoiceError> {
        let s = sound_name(params.s.as_ref());
        let bus_index = |key: &str| {
            buses
                .index_of(key)
                .ok_or_else(|| hap_error(format!("bus {key} is not in the job's bus graph")))
        };
        let send_bus = params.bus.as_deref().map(bus_index).transpose()?;
        let not_found = || hap_error(format!("sound {s} not found"));
        let kind = SourceKind::of(&s).ok_or_else(not_found)?;
        let is_bus = kind == SourceKind::Bus;
        let adsr = get_adsr_values(
            [params.attack, params.decay, params.sustain, params.release],
            Curve::Linear,
            if is_bus {
                BUS_ADSR_DEFAULTS
            } else {
                AMP_ADSR_DEFAULTS
            },
        );
        let t = start.t;
        let now = start.now_frame;
        let mut ctx = BuildContext {
            tables,
            noise,
            seed: voice_seed,
            duration: params.duration,
            now,
        };
        let mut worklet_peak = None;
        let mut source = match kind {
            SourceKind::Constant => {
                let mut c = ConstantSource::new(1.0);
                c.start(t, now)?;
                Source::Constant(c)
            }
            SourceKind::Osc => Source::Osc(Box::new(build_oscillator(
                canonical(&s),
                params,
                t,
                &mut ctx,
            )?)),
            SourceKind::Noise => {
                let noise = NoiseKind::from_name(&s).ok_or_else(not_found)?;
                // `let { density } = value; getNoiseOscillator(s, t, density, ..)`: the default
                // parameter applies to `undefined` only.
                let density = params.density.unwrap_or(DEFAULT_DENSITY);
                Source::Noise(noise_source(noise, density, t, &mut ctx)?)
            }
            SourceKind::Supersaw | SourceKind::Pulse => {
                let (w, peak) = build_worklet_synth(&s, params, t, adsr[3], &mut ctx)?;
                worklet_peak = Some(peak);
                Source::Worklet(Box::new(w))
            }
            SourceKind::Bus => Source::Bus(BusSource {
                bus: bus_index(&params.bus_sound_key)?,
                stop_frame: 0,
            }),
        };
        let hold_end = t + start.hap_duration;
        // The synth sounds: `osc -> Gain(0.3) -> envGain`; the worklet synths connect straight
        // to `envGain`, whose envelope peaks at their own level.
        // The bus sound connects straight to its `envGain` (`gainNode(0)`, linear ADSR to 1).
        let mut stages = Vec::new();
        if worklet_peak.is_none() && !is_bus {
            stages.push(Stage::Gain(GainNode::constant(SYNTH_GAIN)));
        }
        let envelope = if is_bus {
            bus_envelope(adsr, t, hold_end, now)?
        } else {
            amp_envelope(adsr, t, hold_end, worklet_peak.unwrap_or(1.0), now)?
        };
        stages.push(Stage::Gain(GainNode::new(envelope)));
        // The pulse's `pw_lfo` after `envGain`'s envelope (synth.mjs:346-350); the node table
        // holds `[undefined]` without one.
        if let Source::Worklet(w) = &mut source
            && let WorkletSynth::Pulse(_) = w.synth
        {
            let end = t + params.duration + adsr[3] + 0.01;
            w.pw_lfo = pulse_width_lfo(params, t, end, now)?;
        }
        // `envEnd = holdEnd + release + 0.01` with the release `getADSRValues` returned.
        let env_end = idl_double(hold_end + adsr[3] + 0.01, "stop time")?;
        match &mut source {
            Source::Osc(o) => o.stop(env_end)?,
            Source::Constant(c) => c.stop(env_end)?,
            Source::Noise(n) => n.stop(env_end)?,
            // No stop: the processor gates itself on its `end` param (the same time).
            Source::Worklet(_) => {}
            // `webAudioTimeout(ac, .., begin, end)`: a `ConstantSourceNode` started at `begin`
            // and stopped at `end` (its throws); the bus is disconnected at the quantum
            // boundary from `end` on.
            Source::Bus(b) => {
                let mut timeout = ConstantSource::new(1.0);
                timeout.start(t, now)?;
                timeout.stop(env_end)?;
                b.stop_frame =
                    crate::render::round_up_128(crate::render::frame_for_param_event(env_end));
            }
        }
        let stop_frame = match &source {
            Source::Bus(b) => b.stop_frame,
            _ => crate::render::frame_for_buffer_start(env_end),
        };
        // The FX chain (superdough.mjs:612-1004): the transient, the gain stage, the filters, the
        // shapers, the panner, the last gain, the sends and `dry`, each built (and each
        // throwing) in this order.
        let times = EntryTimes {
            begin: t,
            hold_end,
            end_with_release: hold_end + params.full_release,
            now,
            cycle: start.cycle,
            phase_sec: start.phase_sec,
            cps: start.cps,
        };
        let mut param_lfos = Vec::new();
        for (key, entry) in params.entries() {
            build_entry(
                key,
                entry,
                params.ftype_24db,
                times,
                &mut stages,
                &mut param_lfos,
            )?;
        }
        // The release gain when `FXrelease > release` (superdough.mjs:941-946, after the FX loop,
        // main entry only): `gainNode(1)`, `setValueAtTime(1, end + release)`, then a linear ramp
        // to 0 at `endWithRelease`.
        if let Some(fx_release) = params.fx_release
            && fx_release > params.release_raw
        {
            let mut g = GainNode::new(Param::gain(1.0));
            g.gain.set_value(1.0, now)?;
            g.gain
                .set_value_at_time(1.0, hold_end + params.release_raw, now)?;
            g.gain.linear_ramp_to(0.0, times.end_with_release, now)?;
            push_stage(&mut stages, Stage::Release(g));
        }
        let postgain = idl_float(params.postgain, "gain")?;
        push_stage(&mut stages, Stage::Gain(GainNode::constant(postgain)));
        let mut sends = Sends::default();
        // The sends, then `orbitBus.getDjf(djf, t)` when `djf != null` (superdough.mjs:986-989),
        // then `dry`: a throw from any of them after a send was connected is a late error.
        let late = sends
            .connect(params, orbit, t, now, send_bus)
            .and_then(|()| match params.djf {
                Some(v) => orbit.get_djf(v, t, now).map_err(VoiceError::from),
                None => Ok(()),
            })
            .and_then(|()| params.main.dry.map(|d| idl_float(d, "gain")).transpose());
        let (dry, connected, late_error) = match late {
            Ok(dry) => (dry.map(GainNode::constant), true, None),
            Err(e) if sends.any() => (None, false, Some(e)),
            Err(e) => return Err(e),
        };
        let (source_channels, source_end) = match &source {
            Source::Worklet(w) => (2, w.source_end()),
            // The bus is a 2-channel explicit sum; its input to `envGain` goes at `stop_frame`.
            Source::Bus(b) => (2, SourceEnd::Scheduled(Some(b.stop_frame))),
            // Every signal-path source (carrier, noise mix, constant, noise) stops at `envEnd`.
            _ => (1, SourceEnd::Scheduled(Some(stop_frame))),
        };
        let channels = stages
            .iter()
            .fold(source_channels, |n, s| s.channels_out(n));
        let teardown = Teardown::new(source_end, stages.len());
        // The node table is the plan of exactly these nodes.
        let node_table = NodeTable::plan(params, kind);
        debug_assert!(
            node_table
                .stages()
                .iter()
                .copied()
                .eq(stages.iter().map(StageKind::of)),
            "the plan names the built stages"
        );
        #[cfg(test)]
        let start_frame = crate::render::frame_for_buffer_start(t);
        let mut voice = Voice {
            source,
            #[cfg(test)]
            start_frame,
            stop_frame,
            stages,
            sends,
            dry,
            node_table,
            mods: ModSet::default(),
            logs: Vec::new(),
            param_lfos,
            teardown,
            channels,
            connected,
            failed: false,
            errors: Vec::new(),
            #[cfg(test)]
            seed: voice_seed,
        };
        // The modulator loop runs once the voice is connected to its orbit; a late throw before
        // it (`busgain`, `djf`, `dry`) runs none.
        let late_error = match late_error {
            Some(e) => Some(e),
            None => voice.connect_modulators(
                params,
                orbit,
                buses,
                ModTimes {
                    begin: t,
                    end: times.end_with_release,
                    cps: start.cps,
                    cycle: start.cycle,
                    phase_sec: start.phase_sec,
                    now,
                },
            ),
        };
        Ok((voice, late_error))
    }

    /// Superdough's modulator loop on the built voice (`modulate::run_loop`): the modulators
    /// it connects are kept; the error that stopped it is returned (a late `hap` error: the voice
    /// sounds with the modulators before it).
    fn connect_modulators(
        &mut self,
        params: &VoiceParams,
        orbit: &Orbit,
        buses: &BusBank,
        times: ModTimes,
    ) -> Option<VoiceError> {
        if params.entries().all(|(_, e)| e.modulators.is_empty()) {
            return None;
        }
        let source = self.node_table.source();
        let mut table = std::mem::replace(&mut self.node_table, NodeTable::empty(source));
        let r = {
            let me = &*self;
            let cv = |p: ParamRef| me.param_value(p, orbit);
            crate::modulate::run_loop(params, &mut table, &cv, buses, times)
        };
        self.node_table = table;
        self.mods = r.set;
        self.logs = r.logs;
        r.error
    }

    /// The intrinsic value (`.value`) of a param of the voice or of its orbit (an orbit-held
    /// param: what the last rendered quantum left), `None` for a node without that param.
    fn param_value(&self, r: ParamRef, orbit: &Orbit) -> Option<f32> {
        let k = usize::from(r.slot.0);
        let biquad = |b: &Biquad| {
            [&b.frequency, &b.q, &b.gain, &b.detune]
                .get(k)
                .map(|p| p.value())
        };
        let lfo = |l: &Lfo| l.params.get(k).map(Param::value);
        let p = match r.node {
            Node::Stage(StageId(i)) => match self.stages.get(i)? {
                Stage::Gain(g) | Stage::Release(g) => g.gain.value(),
                Stage::Tremolo(t) => t.gain.gain.value(),
                Stage::Biquad(b) | Stage::Phaser(b) => biquad(b)?,
                Stage::Ladder(l) => {
                    let l = &l.proc;
                    [&l.frequency, &l.q, &l.drive].get(k)?.value()
                }
                Stage::Panner(p) => p.pan.value(),
                Stage::Shaper(s) => match &s.kind {
                    ShaperKind::Coarse(c) => c.coarse.value(),
                    ShaperKind::Crush(c) => c.crush.value(),
                    ShaperKind::Shape(x) => [&x.shape, &x.postgain].get(k)?.value(),
                    ShaperKind::Distort(x) => [&x.distort, &x.postgain].get(k)?.value(),
                },
                Stage::Compressor(c) => [&c.threshold, &c.ratio, &c.knee, &c.attack, &c.release]
                    .get(k)?
                    .value(),
                Stage::LocalDelay(d) => [d.delay.delay_time(), d.delay.feedback()].get(k)?.value(),
                Stage::Transient(_) | Stage::Vowel(_) | Stage::LocalReverb(_) => return None,
            },
            Node::StageWet(id) => match self.stages.get(id.0)? {
                Stage::LocalDelay(d) => d.wet.gain.value(),
                Stage::LocalReverb(r) => r.wet.gain.value(),
                _ => return None,
            },
            Node::StageLfo(id) => match self.stages.get(id.0)? {
                Stage::Tremolo(t) => lfo(&t.lfo)?,
                _ => lfo(&self.param_lfos.iter().find(|p| p.stage == id)?.lfo)?,
            },
            Node::VowelFilter(id, n) => match self.stages.get(id.0)? {
                Stage::Vowel(v) => biquad(&v.paths.get(n)?.filter)?,
                _ => return None,
            },
            Node::Source => match &self.source {
                Source::Osc(o) => [&o.carrier.frequency, &o.carrier.detune].get(k)?.value(),
                Source::Worklet(w) => match &w.synth {
                    WorkletSynth::Supersaw(n) => n.params.get(k)?.value(),
                    WorkletSynth::Pulse(n) => n.params.get(k)?.value(),
                },
                Source::Constant(_) | Source::Noise(_) | Source::Bus(_) => return None,
            },
            Node::Vibrato => {
                let osc = match &self.source {
                    Source::Osc(o) => &o.vibrato.as_ref()?.osc,
                    Source::Worklet(w) => &w.vibrato.as_ref()?.0,
                    _ => return None,
                };
                [&osc.frequency, &osc.detune].get(k)?.value()
            }
            Node::VibratoGain => match &self.source {
                Source::Osc(o) => o.vibrato.as_ref()?.depth.value(),
                Source::Worklet(w) => w.vibrato.as_ref()?.1.gain.value(),
                _ => return None,
            },
            Node::FmOsc(pos) => match self.fm()?.modulators.get(pos)?.source() {
                ModSource::Osc(o) => [&o.frequency, &o.detune].get(k)?.value(),
                ModSource::Noise(_) => return None,
            },
            Node::FmAmount(n) => self.fm()?.connections.get(n)?.amount.gain.value(),
            Node::Detached { value } => value,
            Node::PulseLfo => match &self.source {
                Source::Worklet(w) => lfo(w.pw_lfo.as_ref()?)?,
                _ => return None,
            },
            Node::DelaySend => self.sends.delay.as_ref()?.gain.value(),
            Node::RoomSend => self.sends.room.as_ref()?.gain.value(),
            Node::OrbitDelay => {
                let d = orbit.delay()?;
                [d.delay_time(), d.feedback()].get(k)?.value()
            }
            Node::OrbitDjf => orbit.djf()?.value.value(),
            Node::Bus | Node::OrbitReverb | Node::Lfo(_) | Node::Env(_) => return None,
        };
        Some(p)
    }

    /// The voice's FM graph, when it has one.
    fn fm(&self) -> Option<&Fm> {
        match &self.source {
            Source::Osc(o) => o.fm.as_ref(),
            Source::Worklet(w) => w.fm.as_ref(),
            _ => None,
        }
    }

    /// The end of the voice's pause (`settleIR`): each local reverb's pending IR request lands,
    /// in chain order, through the run's IR cache (audio seed `seed`).
    pub fn settle_local_reverbs(&mut self, seed: i32, cache: &mut crate::reverb::IrCache) {
        for s in &mut self.stages {
            if let Stage::LocalReverb(r) = s {
                r.settle(seed, cache);
            }
        }
    }

    /// Takes every local reverb's pending IR request, in chain order: `(stage index, request
    /// number, plan)`, for IRs built elsewhere ([`Self::apply_local_reverb`] lands them).
    pub fn take_local_reverb_requests(&mut self) -> Vec<(usize, u64, crate::reverb::IrPlan)> {
        let mut out = Vec::new();
        for (i, s) in self.stages.iter_mut().enumerate() {
            if let Stage::LocalReverb(r) = s
                && let Some((request, plan)) = r.reverb.take_pending()
            {
                out.push((i, request, plan));
            }
        }
        out
    }

    /// Lands a convolver built for request `request` of the local reverb at stage `stage` (a
    /// stage that is not a local reverb is ignored; the newest request wins).
    pub fn apply_local_reverb(
        &mut self,
        stage: usize,
        request: u64,
        convolver: impl FnOnce() -> crate::reverb::Convolver,
    ) {
        if let Some(Stage::LocalReverb(r)) = self.stages.get_mut(stage) {
            r.reverb.apply(request, convolver);
        }
    }

    /// Superdough's error logs of the modulators this voice's build skipped (each reported once).
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.logs)
    }

    /// The hap's voice seed (`value._voiceSeed`).
    #[cfg(test)]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// The source's first frame and its exclusive stop frame.
    #[cfg(test)]
    pub fn source_frames(&self) -> (u64, u64) {
        (self.start_frame, self.stop_frame)
    }

    #[cfg(test)]
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// The gain of stage `i`, a `Gain` stage.
    #[cfg(test)]
    fn gain_at(&self, i: usize) -> &Param {
        match &self.stages[i] {
            Stage::Gain(g) => &g.gain,
            other => panic!("stage {i} is not a gain: {other:?}"),
        }
    }

    /// The amp envelope param (tests inspect its timeline).
    #[cfg(test)]
    pub fn envelope(&self) -> &Param {
        self.gain_at(1)
    }

    /// The voice's biquads in chain order.
    #[cfg(test)]
    pub fn filters(&self) -> Vec<&Biquad> {
        self.stages
            .iter()
            .filter_map(|s| match s {
                Stage::Biquad(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// The panner, when `pan` is defined.
    #[cfg(test)]
    pub fn panner(&self) -> Option<&StereoPanner> {
        self.stages.iter().find_map(|s| match s {
            Stage::Panner(p) => Some(p),
            _ => None,
        })
    }

    /// The gain stage, `postgain` and `dry` gains.
    #[cfg(test)]
    pub fn levels(&self) -> (f32, f32, Option<f32>) {
        (
            self.gain_at(2).value(),
            self.gain_at(self.stages.len() - 1).value(),
            self.dry.as_ref().map(|d| d.gain.value()),
        )
    }

    /// The voice's stages in chain order.
    #[cfg(test)]
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// The voice's node table.
    #[cfg(test)]
    pub fn node_table(&self) -> &NodeTable {
        &self.node_table
    }

    /// The voice's modulators and their connections.
    #[cfg(test)]
    pub fn mods(&self) -> &ModSet {
        &self.mods
    }

    /// The skip logs not yet taken.
    #[cfg(test)]
    pub fn take_logs_ref(&self) -> &[String] {
        &self.logs
    }

    /// The delay, reverb and bus sends.
    #[cfg(test)]
    pub fn sends(&self) -> Sends {
        self.sends.clone()
    }

    /// The delay and reverb send levels (their gains' intrinsic values).
    #[cfg(test)]
    pub fn send_levels(&self) -> (Option<f32>, Option<f32>) {
        let level = |g: &Option<GainNode>| g.as_ref().map(|g| g.gain.value());
        (level(&self.sends.delay), level(&self.sends.room))
    }

    /// Whether the voice's output is connected to its orbit's summing node.
    #[cfg(test)]
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// The processor errors met since the last call (each reported once, as a `hap` error).
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    /// Whether the output became non-finite (the voice was silenced and freed).
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The voice's output channel count: 2 behind a stereo source or a panner, else 1.
    pub fn output_channels(&self) -> usize {
        self.channels
    }

    /// Renders one quantum starting at `quantum_start` into `out` (overwritten): `post` and the
    /// orbit connection (`Gain(dry)` of `post`, or `post`), channel 0 only for a mono voice, both
    /// for a panned one (see [`Voice::output_channels`]). Returns whether the voice is still
    /// alive: it is freed once its sources have stopped, every stage is done ([`Stage::is_done`]:
    /// a compressor's pre-delay drained, a vowel's biquads quiet) and `post` stayed below
    /// [`SILENCE_FLOOR`] for this whole quantum (`post` also feeds the sends). Call it for every
    /// quantum from the voice's pause frame on (the params' new-event clamping depends on it).
    pub fn process(
        &mut self,
        quantum_start: u64,
        out: &mut VoiceOutput,
        buses: &mut BusBank,
    ) -> bool {
        let q = quantum_start;
        // The modulators first (each param pulls its drivers before it is evaluated).
        self.mods.evaluate(q, buses, &mut self.errors);
        let m = self.mods.view();
        let mut bus = VoiceBus::mono();
        let playing = match &mut self.source {
            Source::Osc(o) => o.process(q, &mut bus.ch[0], &m),
            Source::Constant(c) => c.process(q, &mut bus.ch[0]),
            Source::Noise(n) => n.process(q, &mut bus.ch[0]),
            Source::Worklet(w) => {
                if let Some(e) = w.process(q, &mut bus, &m) {
                    self.errors.push(e);
                }
                !bus.silent
            }
            Source::Bus(b) => b.process(q, &mut bus, buses),
        };
        bus.silent = !playing;
        // The teardown table: the first stage's input goes with the source; each stage passes the
        // loss on by its row of the table.
        // A filter LFO is pulled by its stage's param, so it renders just before that stage.
        let mut before = self.teardown.source();
        let modulated = !self.mods.is_empty();
        for (k, s) in self.stages.iter_mut().enumerate() {
            let id = StageId(k);
            let mut inputs = StageInputs::input(input_connected(before, q));
            for pl in self.param_lfos.iter_mut().filter(|p| p.stage == id) {
                let mut buf = [0.0f32; QUANTUM];
                let silent = pl
                    .lfo
                    .render(q, &m.lfo_inputs(Node::StageLfo(id)), &mut buf);
                inputs.connect(pl.slot, (!silent).then_some(buf));
                if let Some(e) = pl.lfo.take_error() {
                    self.errors.push(e.message());
                }
            }
            if modulated {
                // The modulators of the stage's params, after its own connections.
                for slot in 0..MAX_PARAM_SLOTS as u8 {
                    if let Some(p) = inputs.param_mut(ParamSlot(slot)) {
                        m.add(Node::Stage(id), ParamSlot(slot), p);
                    }
                }
                match s {
                    Stage::Tremolo(_) => inputs.lfo = m.lfo_inputs(Node::StageLfo(id)),
                    Stage::Vowel(_) => {
                        for (n, f) in inputs.formants.iter_mut().enumerate() {
                            m.add(Node::VowelFilter(id, n), ParamSlot::BIQUAD_FREQUENCY, f);
                        }
                    }
                    Stage::LocalDelay(_) | Stage::LocalReverb(_) => {
                        m.add(Node::StageWet(id), ParamSlot::GAIN_GAIN, &mut inputs.wet);
                    }
                    _ => {}
                }
            }
            s.process(q, &mut bus, &inputs);
            if let Some(e) = s.take_error() {
                self.errors.push(e.message());
            }
            before = self.teardown.after(k, s, before, q);
        }
        let n = bus.channels;
        debug_assert_eq!(n, self.output_channels());
        out.channels = n;
        out.post[..n].copy_from_slice(&bus.ch[..n]);
        out.post_silent = bus.silent;
        out.out[..n].copy_from_slice(&bus.ch[..n]);
        out.out_silent = match &mut self.dry {
            Some(d) => d.process(q, &mut out.out[..n], out.post_silent, &ParamInputs::NONE),
            None => out.post_silent,
        };
        if !self.connected {
            zero(&mut out.out);
            out.out_silent = true;
        }
        // Deviation: Chromium renders NaN or infinite samples (a lowpass or highpass whose
        // resonance underflows to 0 below about -758 dB Q has a NaN coefficient; gains whose
        // product overflows the float range give infinities). The engine silences the voice
        // from this quantum on and the driver reports it.
        let non_finite = |b: &[[f32; QUANTUM]]| b.iter().flatten().any(|x| !x.is_finite());
        if non_finite(&out.post[..n]) || non_finite(&out.out[..n]) {
            zero(&mut out.post);
            zero(&mut out.out);
            out.post_silent = true;
            out.out_silent = true;
            // The orbit sends carry the silenced `post` (they still widen the orbit effects'
            // input); the bus send is dropped.
            let silent = || SendBuf {
                ch: [[0.0; QUANTUM]; 2],
                channels: n,
                silent: true,
            };
            out.sends.delay = self.sends.delay.as_ref().map(|_| silent());
            out.sends.room = self.sends.room.as_ref().map(|_| silent());
            out.sends.bus = None;
            self.failed = true;
            return false;
        }
        // The sends: `post -> gainNode(level) -> effect`. The delay and reverb sends go to the
        // orbit, the bus send to the driver.
        let (post, post_silent) = (out.post, out.post_silent);
        let gain = ParamSlot::GAIN_GAIN;
        out.sends.delay = self.sends.delay.as_mut().map(|g| {
            let inputs = m.inputs(Node::DelaySend, gain);
            send_buf(g, q, &post, n, post_silent, &inputs)
        });
        out.sends.room = self.sends.room.as_mut().map(|g| {
            let inputs = m.inputs(Node::RoomSend, gain);
            send_buf(g, q, &post, n, post_silent, &inputs)
        });
        out.sends.bus = self.sends.bus.as_mut().map(|(index, g)| {
            (
                *index,
                send_buf(g, q, &post, n, post_silent, &ParamInputs::NONE),
            )
        });
        let loudest = out.post[..n]
            .iter()
            .flatten()
            .fold(0.0f32, |m, x| m.max(x.abs()));
        // The modulators' outputs into the orbit's own params.
        out.orbit_inputs.clear();
        m.orbit_inputs(&mut out.orbit_inputs);
        let stopped = quantum_start >= self.stop_frame;
        let done = || self.stages.iter().all(|s| s.is_done(quantum_start));
        // A modulator feeding an orbit param keeps the voice alive until its end.
        let holds = || self.mods.holds_orbit(quantum_start);
        !(stopped && done() && !holds() && loudest < SILENCE_FLOOR)
    }
}

/// `createFilter(ac, t, end, params)` for a biquad (helpers.mjs:234-305): `filter.type`, then
/// `Q.value = q` and `frequency.value = frequency` (each also a `setValueAtTime` at `now`), then
/// the envelope when one is defined: ADSR `getADSRValues(.., 'exponential', [0.005, 0.14, 0,
/// 0.1])`, `offset = |env| * anchor`, range `clamp(2^-offset * f, 0, 20000)` to `clamp(2^(|env| -
/// offset) * f, 0, 20000)` (swapped for a negative `env`), `getParamADSR(frequency, .., begin,
/// end, 'exponential')`.
pub fn create_filter(
    kind: FilterKind,
    f: &FilterParams,
    anchor: f64,
    begin: f64,
    end: f64,
    now: u64,
) -> Result<Biquad, VoiceError> {
    let mut filter = Biquad::new(kind);
    filter.q.set_value(idl_float(f.q, "Q")?, now)?;
    filter
        .frequency
        .set_value(idl_float(f.frequency, "frequency")?, now)?;
    filter_envelope(&mut filter.frequency, f, anchor, begin, end, now)?;
    Ok(filter)
}

/// `createFilter`'s envelope on a filter's frequency param (a biquad's or a ladder's) when one
/// is defined: ADSR `getADSRValues(.., 'exponential', [0.005, 0.14, 0, 0.1])`, `offset = |env| *
/// anchor`, range `clamp(2^-offset * f, 0, 20000)` to `clamp(2^(|env| - offset) * f, 0, 20000)`
/// (swapped for a negative `env`), `getParamADSR(frequency, .., begin, end, 'exponential')`.
pub fn filter_envelope(
    frequency: &mut Param,
    f: &FilterParams,
    anchor: f64,
    begin: f64,
    end: f64,
    now: u64,
) -> Result<(), VoiceError> {
    if !f.has_envelope {
        return Ok(());
    }
    let [a, d, s, r] = get_adsr_values(f.adsr, Curve::Exponential, FILTER_ADSR_DEFAULTS);
    let env_abs = f.env.abs();
    let offset = env_abs * anchor;
    let clamp = |x: f64| js_min(js_max(x, 0.0), FILTER_ENV_MAX);
    let mut min = clamp(libm::pow(2.0, -offset) * f.frequency);
    let mut max = clamp(libm::pow(2.0, env_abs - offset) * f.frequency);
    if f.env < 0.0 {
        std::mem::swap(&mut min, &mut max);
    }
    get_param_adsr(
        frequency,
        a,
        d,
        s,
        r,
        min,
        max,
        begin,
        end,
        Curve::Exponential,
        now,
    )?;
    Ok(())
}

/// The bus sound's `envGain`: `gainNode(0)` (`.value = 0`, which also inserts `setValueAtTime(0,
/// now)`), then `getParamADSR(gain, a, d, s, r, 0, 1, begin, end, 'linear')`.
pub fn bus_envelope(adsr: [f64; 4], begin: f64, end: f64, now: u64) -> Result<Param, VoiceError> {
    let mut env = Param::gain(1.0);
    env.set_value(0.0, now)?;
    let [a, d, s, r] = adsr;
    get_param_adsr(
        &mut env,
        a,
        d,
        s,
        r,
        0.0,
        1.0,
        begin,
        end,
        Curve::Linear,
        now,
    )?;
    Ok(env)
}

/// The synth amp envelope: `gainNode(1)` (`.value = 1`, which also inserts `setValueAtTime(1,
/// now)`), then `getParamADSR(gain, a, d, s, r, 0, peak, begin, end, 'linear')` (`peak` 1, or
/// the supersaw's `0.3 * (1 / sqrt(voices))`).
pub fn amp_envelope(
    adsr: [f64; 4],
    begin: f64,
    end: f64,
    peak: f64,
    now: u64,
) -> Result<Param, VoiceError> {
    let mut env = Param::gain(1.0);
    env.set_value(1.0, now)?;
    let [a, d, s, r] = adsr;
    get_param_adsr(
        &mut env,
        a,
        d,
        s,
        r,
        0.0,
        peak,
        begin,
        end,
        Curve::Linear,
        now,
    )?;
    Ok(env)
}

/// [`apply_gain`] with connections into the gain: with one, the a-rate timeline plus the inputs
/// per frame (sample accurate every quantum).
pub fn apply_gain_connected(
    gain: &mut Param,
    quantum_start: u64,
    buf: &mut [f32; QUANTUM],
    inputs: &ParamInputs,
) {
    if !inputs.connected {
        apply_gain(gain, quantum_start, buf);
        return;
    }
    let mut values = [0.0f32; QUANTUM];
    gain.fill_quantum_connected(quantum_start, &mut values, &inputs.bufs, true);
    for (x, g) in buf.iter_mut().zip(values.iter()) {
        *x *= *g;
    }
}

/// `GainHandler::Process` on a connected input (`buf`, scaled in place): the per-frame a-rate
/// values while the param is sample accurate, else one value for the quantum (`FinalValue` or
/// `Value`), with a gain of exactly 0 writing zeros.
pub fn apply_gain(gain: &mut Param, quantum_start: u64, buf: &mut [f32; QUANTUM]) {
    let sample_accurate = gain.has_sample_accurate_values(quantum_start, false);
    if sample_accurate && gain.rate() == Rate::A {
        let mut values = [0.0f32; QUANTUM];
        gain.fill_quantum(quantum_start, &mut values, &[]);
        for (x, g) in buf.iter_mut().zip(values.iter()) {
            *x *= *g;
        }
        return;
    }
    let g = if sample_accurate {
        gain.final_value(quantum_start, &[])
    } else {
        gain.audio_value(quantum_start)
    };
    if g == 0.0 {
        buf.fill(0.0);
    } else {
        for x in buf.iter_mut() {
            *x *= g;
        }
    }
}

/// The engine's `s` string, including the default for an absent value.
pub fn sound_name(s: Option<&Value>) -> String {
    s.map_or_else(|| "triangle".to_string(), js_to_string)
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::chain::{FxKey, NodeList};
    use crate::controls::parse_voice_params;
    use crate::job::OrbitSpec;
    use crate::job::ValueMap;
    use crate::lfo;
    use crate::noise::NoiseCache;
    use crate::param::RampKind;
    use crate::periodic::{WaveKind, WaveTables};
    use crate::render::frame_for_buffer_start;
    use serde_json::json;

    use crate::SAMPLE_RATE_F64 as SR;

    fn params(v: serde_json::Value, dur: f64) -> VoiceParams {
        let map: ValueMap = serde_json::from_value(v).unwrap();
        parse_voice_params(&map, dur, 0.5).unwrap()
    }

    fn start(t: f64, dur: f64) -> StartInfo {
        StartInfo {
            t,
            hap_duration: dur,
            now_frame: 0,
            cycle: 0.0,
            phase_sec: 0.0,
            cps: 2.0,
        }
    }

    /// Renders a voice from frame 0 until it is freed (or `max` frames), returning channel 0.
    fn run(v: &mut Voice, max: u64) -> (Vec<f32>, u64) {
        let (out, _, q) = run_stereo(v, max);
        (out, q)
    }

    /// Renders both output channels (a mono voice repeats channel 0) and the end frame.
    fn run_stereo(v: &mut Voice, max: u64) -> (Vec<f32>, Vec<f32>, u64) {
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut q = 0;
        while q < max {
            let mut o = VoiceOutput::new();
            let alive = v.process(q, &mut o, &mut BusBank::default());
            let buf = o.out;
            l.extend_from_slice(&buf[0]);
            r.extend_from_slice(&buf[if v.output_channels() == 2 { 1 } else { 0 }]);
            q += QUANTUM as u64;
            if !alive {
                break;
            }
        }
        (l, r, q)
    }

    /// The envelope's timeline as `(time, value, kind)`.
    fn events(p: &Param) -> Vec<(f64, f32, RampKind)> {
        p.events()
            .iter()
            .map(|e| (e.time, e.value, e.kind))
            .collect()
    }

    /// The envelope values of frames `[0, n)`, rendered quantum by quantum through `apply_gain`
    /// on a constant 1 input.
    fn envelope_values(env: &mut Param, n: u64) -> Vec<f32> {
        let mut out = Vec::new();
        let mut q = 0;
        while q < n {
            let mut buf = [1.0f32; QUANTUM];
            apply_gain(env, q, &mut buf);
            out.extend_from_slice(&buf);
            q += QUANTUM as u64;
        }
        out.truncate(n as usize);
        out
    }

    #[test]
    fn the_voice_uses_the_synth_adsr_defaults() {
        use RampKind::*;
        // Nothing given: [0.001, 0.05, 0.6, 0.01] (not getADSRValues' own defaults).
        let v = Voice::new(
            &params(json!({ "s": "sine" }), 0.5),
            0,
            start(0.0, 0.5),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let e = events(v.envelope());
        assert_eq!(e[2], (0.001, 1.0, Linear));
        assert_eq!(e[3], (0.001 + 0.05, 0.6, Linear));
        assert_eq!(e[4], (0.5, 0.6, SetValue));
        assert_eq!(e[5], (0.51, 0.0, Linear));
        assert_eq!(
            v.source_frames().1,
            frame_for_buffer_start(0.5 + 0.01 + 0.01)
        );
        // Decay only: sustain 0.001.
        let v = Voice::new(
            &params(json!({ "s": "sine", "decay": 0.1 }), 0.5),
            0,
            start(0.0, 0.5),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let e = events(v.envelope());
        assert_eq!(e[3], (0.001 + 0.1, 0.001, Linear));
        // Attack only: sustain 1; a null attack counts as missing.
        let v = Voice::new(
            &params(json!({ "s": "sine", "attack": 0.2 }), 0.5),
            0,
            start(0.0, 0.5),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert_eq!(events(v.envelope())[3].1, 1.0);
        let v = Voice::new(
            &params(json!({ "s": "sine", "attack": null }), 0.5),
            0,
            start(0.0, 0.5),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert_eq!(events(v.envelope())[3], (0.001 + 0.05, 0.6, Linear));
    }

    #[test]
    fn envelope_events_for_the_three_shapes() {
        use RampKind::*;
        let ev = |adsr, end| events(&amp_envelope(adsr, 0.0, end, 1.0, 0).unwrap());
        // Normal: gainNode(1)'s event, then attack, decay, sustain hold, release.
        assert_eq!(
            ev([0.1, 0.2, 0.5, 0.3], 1.0),
            vec![
                (0.0, 1.0, SetValue),
                (0.0, 0.0, SetValue),
                (0.1, 1.0, Linear),
                (0.30000000000000004, 0.5, Linear),
                (1.0, 0.5, SetValue),
                (1.3, 0.0, Linear)
            ]
        );
        // Attack longer than the hap: ramp to the attack line's value at the end.
        assert_eq!(ev([2.0, 0.1, 0.5, 0.1], 1.0)[2], (1.0, 0.5, Linear));
        // Decay cut by the end: ramp to the decay line's value at the end.
        let e = ev([0.5, 1.0, 0.0, 0.1], 1.0);
        assert_eq!(e[2], (0.5, 1.0, Linear));
        assert_eq!(e[3], (1.0, 0.5, Linear));
        assert_eq!(e[4], (1.1, 0.0, Linear));
        // NaN goes through nanFallback (0).
        let e = ev([f64::NAN, f64::NAN, f64::NAN, f64::NAN], 1.0);
        assert_eq!(e[2], (0.0, 1.0, Linear));
        // A value that overflows f32 throws (TypeError), so the voice fails.
        assert!(amp_envelope([0.1, 0.1, -1e300, 0.1], 0.0, 1.0, 1.0, 0).is_err());
        // Scheduled at a later pause: gainNode(1)'s event sits at the pause time.
        let e = events(&amp_envelope(AMP_ADSR_DEFAULTS, 1.0, 1.5, 1.0, 256).unwrap());
        assert_eq!(e[0], (256.0 / SR, 1.0, SetValue));
    }

    #[test]
    fn envelope_values_by_frame() {
        let mut env = amp_envelope([0.01, 0.01, 0.5, 0.01], 0.0, 0.1, 1.0, 0).unwrap();
        let v = envelope_values(&mut env, 10_000);
        assert_eq!(v[0], 0.0);
        assert!((v[220] - 0.4988662).abs() < 1e-6, "half way up the attack");
        assert!((v[441] - 1.0).abs() < 1e-6, "attack end");
        assert!((v[882] - 0.5).abs() < 1e-6, "decay end");
        assert_eq!(v[2000], 0.5, "sustain");
        assert_eq!(v[4410], 0.5, "hold end");
        assert_eq!(v[4851], 0.0, "release end");
        assert!(v[4851..].iter().all(|x| *x == 0.0));
        // Before the onset event the gain's 1 (from `gainNode(1)`) applies.
        let mut late = amp_envelope(AMP_ADSR_DEFAULTS, 1.0, 2.0, 1.0, 0).unwrap();
        let v = envelope_values(&mut late, 44_200);
        assert_eq!(v[100], 1.0);
        assert_eq!(v[44099], 1.0);
        assert_eq!(v[44100], 0.0);
    }

    #[test]
    fn an_onset_just_after_a_frame_sounds_one_frame_at_the_full_gain() {
        // t * sr = 1000.0001: the source starts on frame 1000 (TimeToSampleFrame rounds to 1/1024
        // first) while the envelope's setValueAtTime(0, t) applies from frame 1001, so frame 1000
        // still has `gainNode(1)`'s gain.
        let t = 1000.0001 / SR;
        let p = params(json!({ "s": "one", "sustain": 1 }), 0.05);
        let mut v = Voice::new(
            &p,
            0,
            start(t, 0.05),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert_eq!(v.source_frames().0, 1000);
        let (out, _) = run(&mut v, 2048);
        assert_eq!(out[999], 0.0);
        assert_eq!(out[1000], SYNTH_GAIN * 0.8);
        // Frame 1001 is on the attack ramp from (t, 0) to (t + 0.001, 1).
        let ramp = (1001.0 - 1000.0001) / 44.1;
        assert!((out[1001] - (SYNTH_GAIN * 0.8 * ramp as f32)).abs() < 1e-7);
    }

    #[test]
    fn apply_gain_paths() {
        // No events: the k-rate `Value()` path with the intrinsic value; 0 writes zeros.
        let mut g = Param::gain(0.5);
        let mut buf = [2.0f32; QUANTUM];
        apply_gain(&mut g, 0, &mut buf);
        assert!(buf.iter().all(|x| *x == 1.0));
        let mut z = Param::gain(0.0);
        let mut buf = [f32::INFINITY; QUANTUM];
        apply_gain(&mut z, 0, &mut buf);
        assert!(
            buf.iter().all(|x| *x == 0.0),
            "a zero gain zeroes, even inf"
        );
        // One event in the past: constant, not sample accurate.
        let mut g = Param::gain(1.0);
        g.set_value_at_time(0.25, 0.0, 0).unwrap();
        let mut buf = [1.0f32; QUANTUM];
        apply_gain(&mut g, 128, &mut buf);
        assert!(buf.iter().all(|x| *x == 0.25));
        // A ramp inside the quantum: per frame.
        let mut g = Param::gain(1.0);
        g.set_value_at_time(0.0, 0.0, 0).unwrap();
        g.linear_ramp_to(1.0, 127.0 / SR, 0).unwrap();
        let mut buf = [1.0f32; QUANTUM];
        apply_gain(&mut g, 0, &mut buf);
        assert_eq!(buf[0], 0.0);
        assert_eq!(buf[127], 1.0);
        assert!(buf[64] > 0.49 && buf[64] < 0.51);
    }

    #[test]
    fn sine_peak_is_point_three_times_the_gain_stage() {
        let p = params(
            json!({ "s": "sine", "note": 69, "gain": 0.5, "sustain": 1 }),
            0.25,
        );
        let mut v = Voice::new(
            &p,
            0,
            start(0.0, 0.25),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert!(matches!(v.source(), Source::Osc(_)));
        let (out, _) = run(&mut v, 44100);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.3 * 0.5).abs() < 1e-3, "peak {peak}");
        assert!(peak <= 0.15 + 1e-6);
        // Default gain 0.8: 0.24 x the waveform.
        let p = params(json!({ "s": "sine", "note": 69, "sustain": 1 }), 0.25);
        let (out, _) = run(
            &mut Voice::new(
                &p,
                0,
                start(0.0, 0.25),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap(),
            44100,
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.24).abs() < 1e-3, "peak {peak}");
    }

    #[test]
    fn stop_time_and_lifetime() {
        // holdEnd 0.1 + release 0.05 + 0.01 = 0.16 s -> frame 7056.
        let p = params(json!({ "s": "sine", "release": 0.05 }), 0.1);
        let mut v = Voice::new(
            &p,
            0,
            start(0.0, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert_eq!(v.source_frames(), (0, 7056));
        let (out, end) = run(&mut v, 44100);
        assert!(out[7056..].iter().all(|x| *x == 0.0));
        // Freed in the first quantum that starts at or after the stop frame (7168), not before.
        assert_eq!(end, 7168 + 128);
        // A release below 0.01 is clamped by getADSRValues and the stop time uses the clamped one.
        let p = params(json!({ "s": "sine", "release": 0.001 }), 0.1);
        assert_eq!(
            Voice::new(
                &p,
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap()
            .source_frames()
            .1,
            5292
        );
    }

    #[test]
    fn a_voice_waiting_for_its_onset_is_not_freed() {
        let p = params(json!({ "s": "sine" }), 0.1);
        let mut v = Voice::new(
            &p,
            0,
            start(0.5, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let mut buf = VoiceOutput::new();
        assert!(
            v.process(0, &mut buf, &mut BusBank::default()),
            "silent before the onset but alive"
        );
        assert!(buf.out[0].iter().all(|x| *x == 0.0));
        assert!(buf.post_silent && buf.out_silent);
        assert_eq!(v.source_frames().0, 22050);
    }

    #[test]
    fn constant_and_noise_sources() {
        let p = params(json!({ "s": "one", "attack": 0.001, "sustain": 1 }), 0.1);
        let mut v = Voice::new(
            &p,
            0,
            start(0.0, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let (out, _) = run(&mut v, 44100);
        assert!((out[1000] - 0.24).abs() < 1e-6);
        let p = params(json!({ "s": "pink" }), 0.1);
        let mut v = Voice::new(
            &p,
            0,
            start(0.0, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert!(matches!(v.source(), Source::Noise(_)));
        let (out, _) = run(&mut v, 44100);
        assert!(out.iter().any(|x| *x != 0.0));
    }

    #[test]
    fn hap_error_paths() {
        let e = |v: serde_json::Value| {
            Voice::new(
                &params(v, 0.1),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap_err()
        };
        assert_eq!(e(json!({ "note": "x9" })).detail, "not a note: \"x9\"");
        assert!(
            e(json!({ "note": "c-" })).detail.contains("frequency"),
            "NaN frequency throws"
        );
        assert!(
            e(json!({ "freq": 1e39 })).detail.contains("frequency"),
            "f32 overflow throws"
        );
        assert!(
            e(json!({ "release": "x" })).detail.contains("stop time"),
            "NaN release"
        );
        assert!(
            e(json!({ "gain": 1e30, "velocity": 1e30 }))
                .detail
                .contains("gain")
        );
        assert!(e(json!({ "velocity": "x" })).detail.contains("gain"));
        assert_eq!(e(json!({ "s": "bytebeat" })).kind, ErrorKind::Hap);
        assert_eq!(
            e(json!({ "s": "bytebeat" })).detail,
            "sound bytebeat not found"
        );
        let ok = Voice::new(
            &params(json!({ "freq": 1e30 }), 0.1),
            0,
            start(0.0, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let Source::Osc(o) = ok.source() else {
            panic!("an oscillator")
        };
        assert_eq!(
            o.carrier.frequency.value(),
            (SR / 2.0) as f32,
            "clamped to Nyquist"
        );
        assert_eq!(sound_name(None), "triangle");
        assert_eq!(sound_name(Some(&Value::Num(2.0))), "2");
    }
    fn osc_source(v: &Voice) -> &OscSource {
        match v.source() {
            Source::Osc(o) => o,
            other => panic!("not an oscillator: {other:?}"),
        }
    }

    fn try_voice(v: serde_json::Value, dur: f64) -> Result<Voice, VoiceError> {
        Voice::new(
            &params(v, dur),
            0,
            start(0.0, dur),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
    }

    /// A standalone orbit 1 on a stereo destination.
    fn bus() -> Orbit {
        Orbit::new(
            &OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1],
            },
            2,
        )
    }

    fn voice(v: serde_json::Value, dur: f64) -> Voice {
        try_voice(v, dur).unwrap()
    }

    #[test]
    fn oscillator_kinds_and_aliases() {
        for (s, kind) in [
            ("sine", WaveKind::Sine),
            ("sin", WaveKind::Sine),
            ("sawtooth", WaveKind::Sawtooth),
            ("saw", WaveKind::Sawtooth),
            ("square", WaveKind::Square),
            ("sqr", WaveKind::Square),
            ("triangle", WaveKind::Triangle),
            ("tri", WaveKind::Triangle),
        ] {
            let v = voice(json!({ "s": s }), 0.1);
            assert_eq!(osc_source(&v).carrier.wave().kind(), kind, "{s}");
        }
        // No `s`: triangle. `user` without partials: triangle.
        assert_eq!(
            osc_source(&voice(json!({}), 0.1)).carrier.wave().kind(),
            WaveKind::Triangle
        );
        assert_eq!(
            osc_source(&voice(json!({ "s": "user" }), 0.1))
                .carrier
                .wave()
                .kind(),
            WaveKind::Triangle
        );
        // `user` with an empty (truthy) list: `o.type = 'user'` is ignored, the node stays sine.
        assert_eq!(
            osc_source(&voice(json!({ "s": "user", "partials": [] }), 0.1))
                .carrier
                .wave()
                .kind(),
            WaveKind::Sine
        );
        // Partials make a custom wave, except for sine; `n` is the partial count when
        // `partials` is nullish.
        for v in [
            json!({ "s": "sawtooth", "n": 4 }),
            json!({ "s": "square", "partials": [1, 0.5] }),
            json!({ "s": "user", "partials": [1], "n": 0 }),
            json!({ "s": "tri", "partials": null, "n": 3 }),
        ] {
            assert_eq!(
                osc_source(&voice(v.clone(), 0.1)).carrier.wave().kind(),
                WaveKind::Custom,
                "{v}"
            );
        }
        assert_eq!(
            osc_source(&voice(json!({ "s": "sine", "n": 4 }), 0.1))
                .carrier
                .wave()
                .kind(),
            WaveKind::Sine
        );
        assert_eq!(
            osc_source(&voice(json!({ "s": "sawtooth", "n": 0 }), 0.1))
                .carrier
                .wave()
                .kind(),
            WaveKind::Sawtooth
        );
    }

    #[test]
    fn custom_waves_are_cached_per_run() {
        let mut tables = WaveTables::new();
        let p = params(json!({ "s": "sawtooth", "n": 6 }), 0.1);
        let a = Voice::new(
            &p,
            0,
            start(0.0, 0.1),
            &mut tables,
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let b = Voice::new(
            &p,
            0,
            start(0.2, 0.1),
            &mut tables,
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        assert_eq!(tables.custom_len(), 1);
        assert_eq!(osc_source(&a).carrier.wave(), osc_source(&b).carrier.wave());
    }

    #[test]
    fn default_synth_peaks_at_point_two_four_times_the_waveform() {
        // Sustain 1 holds the envelope at 1; 0.3 * 0.8 * the normalized wave.
        for s in ["sine", "square", "sawtooth", "triangle"] {
            let mut v = voice(json!({ "s": s, "note": 57, "sustain": 1 }), 0.25);
            let (out, _) = run(&mut v, 44100);
            let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.2 && peak < 0.24 * 1.1, "{s}: {peak}");
        }
        let mut v = voice(json!({ "s": "sine", "note": 69, "sustain": 1 }), 0.25);
        let (out, _) = run(&mut v, 44100);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.24).abs() < 1e-4, "{peak}");
    }

    #[test]
    fn frequency_value_and_start() {
        let v = Voice::new(
            &params(json!({ "s": "saw", "note": 69 }), 0.1),
            0,
            StartInfo {
                t: 0.5,
                hap_duration: 0.1,
                now_frame: 256,
                cycle: 0.0,
                phase_sec: 0.0,
                cps: 2.0,
            },
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let o = osc_source(&v);
        assert_eq!(o.carrier.frequency.value(), 440.0);
        let e = o.carrier.frequency.events();
        assert_eq!((e.len(), e[0].time, e[0].value), (1, 256.0 / SR, 440.0));
        assert_eq!(o.carrier.schedule().start_frame(), 22050);
        assert_eq!(
            o.carrier.schedule().end_frame(),
            Some(frame_for_buffer_start(0.5 + 0.1 + 0.01 + 0.01))
        );
        assert!(o.vibrato.is_none());
        assert!(o.carrier.detune.events().is_empty());
    }

    #[test]
    fn vibrato() {
        let v = voice(json!({ "s": "sine", "vib": 5, "note": 69 }), 0.5);
        let vib = osc_source(&v).vibrato.as_ref().expect("vibrato");
        assert_eq!(vib.osc.frequency.value(), 5.0);
        assert_eq!(vib.osc.wave().kind(), WaveKind::Sine);
        assert_eq!(vib.depth.value(), 50.0, "vibmod 0.5 semitones = 50 cents");
        assert_eq!(
            vib.osc.schedule().end_frame(),
            osc_source(&v).carrier.schedule().end_frame()
        );
        let v = voice(json!({ "s": "sine", "vib": "4", "vibmod": 2 }), 0.5);
        let vib = osc_source(&v).vibrato.as_ref().unwrap();
        assert_eq!((vib.osc.frequency.value(), vib.depth.value()), (4.0, 200.0));
        // A null vibmod is 0 cents; vib <= 0, NaN or missing: no vibrato.
        let v = voice(json!({ "s": "sine", "vib": 4, "vibmod": null }), 0.5);
        assert_eq!(osc_source(&v).vibrato.as_ref().unwrap().depth.value(), 0.0);
        for vib in [json!(0), json!(-3), json!("x"), json!(null)] {
            let v = voice(json!({ "s": "sine", "vib": vib }), 0.5);
            assert!(osc_source(&v).vibrato.is_none(), "{vib}");
        }
        // The pitch wobbles by +-vibmod semitones: count zero crossings over 0.4 s at 440 Hz
        // with a deep slow vibrato vs without.
        let crossings = |v: serde_json::Value| {
            let mut voice = voice(v, 0.5);
            let (out, _) = run(&mut voice, 17640);
            out.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
        };
        let plain = crossings(json!({ "s": "sine", "note": 69, "sustain": 1 }));
        let up =
            crossings(json!({ "s": "sine", "note": 69, "sustain": 1, "vib": 1.25, "vibmod": 12 }));
        assert!((175..=177).contains(&plain), "{plain}");
        // Half a vibrato period (0.4 s at 1.25 Hz) spent above the pitch: more cycles.
        assert!(up > plain + 20, "{up} vs {plain}");
        // Errors: non-finite rate or depth throw.
        let e = |v: serde_json::Value| {
            Voice::new(
                &params(v, 0.1),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap_err()
        };
        assert!(
            e(json!({ "s": "sine", "vib": 1e39 }))
                .detail
                .contains("frequency")
        );
        assert!(
            e(json!({ "s": "sine", "vib": 2, "vibmod": "x" }))
                .detail
                .contains("gain")
        );
    }

    #[test]
    fn pitch_envelope_events() {
        use RampKind::*;
        let ev = |v: serde_json::Value| {
            let voice = voice(v, 0.5);
            osc_source(&voice)
                .carrier
                .detune
                .events()
                .iter()
                .map(|e| (e.time, e.value, e.kind))
                .collect::<Vec<_>>()
        };
        // Inactive without any p-control.
        assert!(ev(json!({ "s": "sine" })).is_empty());
        // penv alone: 1 semitone default overridden, defaults [0.2, 0.001, 1, 0.001], anchor =
        // sustain 1: min -1200, max 0, linear.
        assert_eq!(
            ev(json!({ "s": "sine", "penv": 12 })),
            vec![
                (0.0, -1200.0, SetValue),
                (0.2, 0.0, Linear),
                (0.201, 0.0, Linear),
                (0.5, 0.0, SetValue),
                (0.501, -1200.0, Linear),
            ]
        );
        // panchor 0, pcurve 1: exponential from 0.001 (min 0 mapped) to 1200.
        let e = ev(json!({ "s": "sine", "penv": 12, "panchor": 0, "pcurve": 1, "pattack": 0.1 }));
        assert_eq!(e[0], (0.0, 0.001, SetValue));
        assert_eq!(e[1], (0.1, 1200.0, Exponential));
        // A null penv still activates the envelope (cents 0); a null pattack alone does not.
        assert_eq!(ev(json!({ "s": "sine", "penv": null }))[0].1, 0.0);
        assert!(ev(json!({ "s": "sine", "pattack": null })).is_empty());
        // pdecay alone: sustain becomes 0.001 per getADSRValues, anchor follows it.
        let e = ev(json!({ "s": "sine", "pdecay": 0.1 }));
        let anchor = 0.001;
        assert_eq!(e[0].1, (0.0 - 100.0 * anchor) as f32);
        // NaN penv falls back to 1 semitone.
        assert_eq!(ev(json!({ "s": "sine", "penv": "x" }))[0].1, -100.0);
        // `one` ignores the pitch envelope and vibrato.
        let v = voice(json!({ "s": "one", "penv": 12, "vib": 5 }), 0.5);
        assert!(matches!(v.source(), Source::Constant(_)));
        // Negative penv with pcurve 1: min 1200, max 0 -> 0.001.
        let e = ev(json!({ "s": "sine", "penv": -12, "pcurve": 1 }));
        assert_eq!(e[0], (0.0, 1200.0, SetValue));
        assert_eq!(e[1], (0.2, 0.001, Exponential));
    }

    #[test]
    fn pitch_envelope_makes_the_first_quantum_a_rate() {
        // A zero-depth pitch envelope changes nothing but the path: the start at 28.665 frames
        // (in a later quantum) is a-rate, so its first frame has phase 0.
        let t = (1280.0 + 28.665) / SR;
        let with = |extra: serde_json::Value| {
            let mut m = json!({ "s": "sine", "note": 69, "sustain": 1 });
            for (k, v) in extra.as_object().unwrap() {
                m[k] = v.clone();
            }
            let mut v = Voice::new(
                &params(m, 0.2),
                0,
                start(t, 0.2),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap();
            run(&mut v, 4096).0
        };
        let plain = with(json!({}));
        let penv = with(json!({ "penv": 0 }));
        assert_eq!(plain[1308], 0.0);
        assert_eq!(penv[1308], 0.0);
        // Frame 1309: k-rate starts at phase 0.335 * incr, a-rate at 0 (times the envelope's
        // first ramp step).
        assert!(plain[1309].abs() > 0.0);
        assert!(
            penv[1309].abs() < plain[1309].abs() * 1e-3,
            "{} {}",
            penv[1309],
            plain[1309]
        );
    }

    #[test]
    fn partials_errors_are_hap_errors() {
        let e = |v: serde_json::Value| {
            Voice::new(
                &params(v, 0.1),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap_err()
        };
        assert!(
            e(json!({ "s": "saw", "n": -2 }))
                .detail
                .starts_with("RangeError")
        );
        assert!(
            e(json!({ "s": "user", "partials": "abc" }))
                .detail
                .starts_with("IndexSizeError")
        );
        assert!(
            e(json!({ "s": "tri", "partials": [1e39] }))
                .detail
                .starts_with("TypeError")
        );
        assert!(
            e(json!({ "s": "square", "partials": [1e38] }))
                .detail
                .contains("overflows")
        );
        // The wave is built before the frequency is read: its error wins.
        assert!(
            e(json!({ "s": "saw", "n": -2, "note": "x9" }))
                .detail
                .starts_with("RangeError")
        );
    }

    fn kinds(v: &Voice) -> Vec<FilterKind> {
        v.filters().iter().map(|f| f.kind()).collect()
    }

    #[test]
    fn filters_follow_superdough_order_and_24db_doubles_them() {
        use FilterKind::*;
        let all = json!({ "s": "saw", "bandf": 500, "hcutoff": 200, "cutoff": 3000 });
        assert_eq!(
            kinds(&voice(all.clone(), 0.1)),
            [Lowpass, Highpass, Bandpass]
        );
        for (ftype, doubled) in [
            (json!("24db"), true),
            (json!(2), true),
            (json!(5), true),
            (json!(-1), true),
            (json!(2.5), true),
            (json!(1), false),
            (json!(4), false),
            (json!(0), false),
            (json!("12db"), false),
            (json!("2"), false),
            (json!(null), false),
            (json!(true), false),
        ] {
            let mut v = all.clone();
            v["ftype"] = ftype.clone();
            let want: Vec<_> = if doubled {
                vec![Lowpass, Lowpass, Highpass, Highpass, Bandpass, Bandpass]
            } else {
                vec![Lowpass, Highpass, Bandpass]
            };
            assert_eq!(kinds(&voice(v, 0.1)), want, "ftype {ftype}");
        }
        // Only the filters whose frequency control is defined (a null one counts).
        assert!(voice(json!({ "s": "saw" }), 0.1).filters().is_empty());
        assert_eq!(
            kinds(&voice(json!({ "s": "saw", "hcutoff": null }), 0.1)),
            [Highpass]
        );
        assert_eq!(
            kinds(&voice(
                json!({ "s": "saw", "bandf": 300, "ftype": "24db" }),
                0.1
            )),
            [Bandpass, Bandpass]
        );
    }

    #[test]
    fn filter_q_and_frequency_values() {
        let v = voice(json!({ "s": "saw", "cutoff": 800, "resonance": 7 }), 0.1);
        let f = &v.filters()[0];
        assert_eq!((f.frequency.value(), f.q.value()), (800.0, 7.0));
        // `.value =` inserts a setValueAtTime at the pause frame, Q first.
        assert_eq!(f.frequency.events().len(), 1);
        assert_eq!(f.q.events().len(), 1);
        // Q defaults to 1 when undefined; a null Q is 0; strings are numbers.
        let v = voice(
            json!({ "s": "saw", "hcutoff": "900", "hresonance": null }),
            0.1,
        );
        assert_eq!(v.filters()[0].q.value(), 0.0);
        assert_eq!(v.filters()[0].frequency.value(), 900.0);
        let v = voice(json!({ "s": "saw", "bandf": 900 }), 0.1);
        assert_eq!(v.filters()[0].q.value(), 1.0);
        // The frequency param clamps to [0, nyquist].
        let v = voice(json!({ "s": "saw", "cutoff": 30000 }), 0.1);
        assert_eq!(v.filters()[0].frequency.value(), 22050.0);
        let v = voice(json!({ "s": "saw", "cutoff": -5 }), 0.1);
        assert_eq!(v.filters()[0].frequency.value(), 0.0);
    }

    fn freq_events(v: &Voice, i: usize) -> Vec<(f64, f32, RampKind)> {
        events(&v.filters()[i].frequency)
    }

    #[test]
    fn filter_envelopes() {
        use RampKind::*;
        let f = |x: f64| x as f32;
        // lpenv 2, cutoff 1000, defaults [0.005, 0.14, 0, 0.1], anchor 0: 1000 -> 4000 -> 1000.
        let v = voice(json!({ "s": "saw", "cutoff": 1000, "lpenv": 2 }), 0.5);
        let e = freq_events(&v, 0);
        assert_eq!(e[0], (0.0, 1000.0, SetValue));
        assert_eq!(e[1], (0.0, 1000.0, SetValue));
        assert_eq!(e[2], (0.005, 4000.0, Exponential));
        assert_eq!(e[3], (0.005 + 0.14, 1000.0, Exponential));
        assert_eq!(e[4], (0.5, 1000.0, SetValue));
        assert_eq!(e[5], (0.6, 1000.0, Exponential));
        // fanchor 0.5 with env 2: offset 1 -> 500 .. 2000; sustain 0.5 -> 1250.
        let v = voice(
            json!({ "s": "saw", "cutoff": 1000, "lpenv": 2, "fanchor": 0.5, "lpsustain": 0.5 }),
            0.5,
        );
        let e = freq_events(&v, 0);
        assert_eq!(e[1], (0.0, 500.0, SetValue));
        assert_eq!(e[2].1, 2000.0);
        assert_eq!(e[3].1, 1250.0);
        assert_eq!(e[5].1, 500.0);
        // A negative env swaps the range: from 1000 * 2^3 down to 1000.
        let v = voice(
            json!({ "s": "saw", "cutoff": 1000, "hpenv": -3, "hcutoff": 1000 }),
            0.5,
        );
        assert!(
            freq_events(&v, 0).len() == 1,
            "lpenv absent: no lowpass envelope"
        );
        let e = freq_events(&v, 1);
        assert_eq!(e[1].1, 8000.0);
        assert_eq!(e[2].1, 1000.0);
        assert_eq!(e[3].1, 8000.0);
        // The range is clamped to 20000.
        let v = voice(json!({ "s": "saw", "bandf": 15000, "bpenv": 3 }), 0.5);
        assert_eq!(freq_events(&v, 0)[2].1, 20000.0);
        // Any one ADSR control enables it with env 1 (an octave); `null` counts as defined.
        let v = voice(json!({ "s": "saw", "cutoff": 300, "lpattack": null }), 0.5);
        let e = freq_events(&v, 0);
        assert_eq!(e.len(), 6);
        assert_eq!(e[2].1, f(600.0));
        let v = voice(json!({ "s": "saw", "cutoff": 300, "lpdecay": 0.2 }), 0.5);
        let e = freq_events(&v, 0);
        // decay given, attack and sustain absent: attack 0.001, sustain 0.001 of the range.
        assert_eq!(e[2], (0.001, f(600.0), Exponential));
        assert_eq!(e[3], (0.001 + 0.2, f(300.0 + 0.001 * 300.0), Exponential));
        // env 0 or a NaN env: min == max == f (a null env is 0, a NaN one 1).
        let v = voice(json!({ "s": "saw", "cutoff": 300, "lpenv": null }), 0.5);
        assert_eq!(freq_events(&v, 0)[2].1, 300.0);
        let v = voice(json!({ "s": "saw", "cutoff": 300, "lpenv": "x" }), 0.5);
        assert_eq!(freq_events(&v, 0)[2].1, 600.0);
        // 24 dB: both biquads carry the envelope.
        let v = voice(
            json!({ "s": "saw", "cutoff": 300, "lpenv": 1, "ftype": "24db" }),
            0.5,
        );
        assert_eq!(freq_events(&v, 0), freq_events(&v, 1));
        assert_eq!(freq_events(&v, 1).len(), 6);
    }

    #[test]
    fn a_24db_filter_is_two_identical_biquads_in_series() {
        let one = |extra: serde_json::Value| {
            let mut v = json!({ "s": "one", "sustain": 1, "cutoff": 2000, "resonance": 3 });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            run(&mut voice(v, 0.05), 8192).0
        };
        let single = one(json!({}));
        let double = one(json!({ "ftype": "24db" }));
        // Feed the single-filter output through a second identical biquad.
        let mut second = Biquad::new(FilterKind::Lowpass);
        second.q.set_value(3.0, 0).unwrap();
        second.frequency.set_value(2000.0, 0).unwrap();
        let mut want = Vec::new();
        for (k, chunk) in single.chunks(QUANTUM).enumerate() {
            let mut buf = [[0.0f32; QUANTUM]; 1];
            buf[0][..chunk.len()].copy_from_slice(chunk);
            second.process(
                (k * QUANTUM) as u64,
                &mut buf,
                false,
                &crate::biquad::BiquadInputs::NONE,
            );
            want.extend_from_slice(&buf[0][..chunk.len()]);
        }
        let n = double.len().min(want.len());
        let (a, b) = (&double[..n], &want[..n]);
        // The silence hints differ at the very end only; the audible part is identical.
        assert_eq!(&a[..2048], &b[..2048]);
        assert_ne!(single[..2048], double[..2048]);
    }

    #[test]
    fn pan_only_when_defined() {
        let v = voice(json!({ "s": "sine" }), 0.1);
        assert!(v.panner().is_none());
        assert_eq!(v.output_channels(), 1);
        let v = voice(json!({ "s": "sine", "pan": 0.5 }), 0.1);
        assert_eq!(v.output_channels(), 2);
        assert_eq!(v.panner().unwrap().pan.value(), 0.0);
        assert_eq!(v.panner().unwrap().pan.events().len(), 1);
        // `2 * pan - 1`: null is -1, out of range clamps, strings are numbers.
        for (pan, want) in [
            (json!(null), -1.0),
            (json!(0), -1.0),
            (json!(1), 1.0),
            (json!(3), 1.0),
            (json!(-2), -1.0),
            (json!("0.75"), 0.5),
        ] {
            let v = voice(json!({ "s": "sine", "pan": pan }), 0.1);
            assert_eq!(v.panner().unwrap().pan.value(), want);
        }
    }

    #[test]
    fn a_centred_pan_loses_three_db_per_side() {
        let p = json!({ "s": "one", "sustain": 1, "pan": 0.5 });
        let (l, r, _) = run_stereo(&mut voice(p, 0.05), 4096);
        let mono = run(&mut voice(json!({ "s": "one", "sustain": 1 }), 0.05), 4096).0;
        assert_eq!(l[1000], r[1000]);
        let g = libm::cos(std::f64::consts::FRAC_PI_4) as f32;
        assert_eq!(l[1000], mono[1000] * g);
        // Hard left.
        let p = json!({ "s": "one", "sustain": 1, "pan": 0 });
        let (l, r, _) = run_stereo(&mut voice(p, 0.05), 4096);
        assert_eq!((l[1000], r[1000]), (mono[1000], 0.0));
    }

    #[test]
    fn dry_scales_the_orbit_connection_and_post_feeds_the_sends() {
        let mut v = voice(
            json!({ "s": "one", "sustain": 1, "postgain": 2, "dry": 0.25 }),
            0.05,
        );
        let mut o = VoiceOutput::new();
        v.process(512, &mut o, &mut BusBank::default());
        let post = 1.0f32 * SYNTH_GAIN * 0.8 * 2.0;
        assert_eq!(o.channels, 1);
        assert_eq!(o.post[0][10], post);
        assert_eq!(o.out[0][10], post * 0.25);
        assert!(!o.post_silent && !o.out_silent);
        assert_eq!(o.to_orbit().channels.len(), 1);
        assert_eq!(o.to_orbit().channels[0][10], post * 0.25);
        assert_eq!(o.to_sends().channels[0][10], post);
        // Without dry the orbit gets post itself.
        let mut v = voice(json!({ "s": "one", "sustain": 1, "pan": 0.25 }), 0.05);
        v.process(512, &mut o, &mut BusBank::default());
        assert_eq!(o.channels, 2);
        assert_eq!(o.out, o.post);
        assert_eq!(o.to_orbit().channels.len(), 2);
        // dry 0: the orbit connection is silent while post (the send tap) still plays, and the
        // voice stays alive on post.
        let mut v = voice(json!({ "s": "one", "sustain": 1, "dry": 0 }), 0.05);
        assert!(v.process(512, &mut o, &mut BusBank::default()));
        assert!(o.out_silent && !o.post_silent);
        assert!(o.out[0].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn sends_follow_superdoughs_conditions() {
        let sends = |v: serde_json::Value| voice(v, 0.05).send_levels();
        assert_eq!(sends(json!({ "s": "sine" })), (None, None));
        assert!(voice(json!({ "s": "sine" }), 0.05).sends().bus.is_none());
        assert_eq!(
            sends(json!({ "s": "sine", "delay": 0.5, "room": 0.25 })),
            (Some(0.5), Some(0.25))
        );
        // Each delay condition: level, time and feedback all > 0.
        for v in [
            json!({ "s": "sine", "delay": 0.5, "delayfeedback": 0 }),
            json!({ "s": "sine", "delay": 0.5, "delaytime": 0 }),
            json!({ "s": "sine", "delay": 0.5, "delaysync": -1 }),
            json!({ "s": "sine", "delay": 0 }),
            json!({ "s": "sine", "delay": -1 }),
        ] {
            assert_eq!(sends(v.clone()).0, None, "{v}");
        }
        assert_eq!(sends(json!({ "s": "sine", "room": 0 })).1, None);
        assert_eq!(sends(json!({ "s": "sine", "room": "0.5" })).1, Some(0.5));
        // A send level the float conversion rejects throws, and the hap fails (after postgain,
        // before dry).
        for v in [
            json!({ "s": "sine", "delay": 1e40 }),
            json!({ "s": "sine", "room": { "$num": "Infinity" } }),
        ] {
            let e = try_voice(v.clone(), 0.05).unwrap_err();
            assert!(e.detail.starts_with("TypeError"), "{v}: {}", e.detail);
        }
        // The voice's own output does not depend on its sends (they tap `post`), and the
        // output carries `post` scaled by each send level for the orbit bus.
        let mut with = voice(json!({ "s": "sine", "delay": 0.5, "room": 0.5 }), 0.05);
        let a = run(&mut with, 8192).0;
        let b = run(&mut voice(json!({ "s": "sine" }), 0.05), 8192).0;
        assert_eq!(a, b);
        let mut o = VoiceOutput::new();
        for q in [0, 128] {
            with.process(q, &mut o, &mut BusBank::default());
            for send in [o.sends.delay.as_ref(), o.sends.room.as_ref()] {
                let send = send.expect("both sends");
                assert_eq!((send.channels, send.silent), (1, o.post_silent), "{q}");
                let want: Vec<u32> = o.post[0].iter().map(|x| (x * 0.5).to_bits()).collect();
                let got: Vec<u32> = send.ch[0].iter().map(|x| x.to_bits()).collect();
                assert_eq!(got, want, "quantum {q}");
            }
            assert!(o.sends.bus.is_none());
        }
    }

    fn voice_on(v: serde_json::Value, orbit: &mut Orbit) -> Result<Voice, VoiceError> {
        Voice::new(
            &params(v, 0.05),
            0,
            start(0.1, 0.05),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            orbit,
        )
    }

    #[test]
    fn fm_inputs_precede_modulator_inputs_on_the_carrier_frequency() {
        let (v, late) = build_on(
            json!({ "s": "sine", "note": 57, "fmi": 1.3, "fmh": 1.5, "fmi20": 0.9, "fmh2": 2.3,
                    "lfo": { "__ids": { "$set": [0] },
                             "0": { "control": "note", "rate": 6, "depth": 0.02 } } }),
            &mut bus(),
        )
        .unwrap();
        assert!(late.is_none());
        let Source::Osc(src) = v.source() else {
            panic!("an oscillator");
        };
        let (mut got, mut fm_first, mut lfo_first) =
            ((**src).clone(), (**src).clone(), (**src).clone());
        let mut set = v.mods().clone();
        let none = ModSet::default();
        let mut reordered = false;
        for k in 0..40u64 {
            let q = k * 128;
            set.evaluate(q, &mut BusBank::default(), &mut Vec::new());
            let lfo = set.view().inputs(Node::Source, ParamSlot(0)).bufs;
            let mut a = [0.0f32; QUANTUM];
            got.process(q, &mut a, &set.view());
            let render = |o: &mut OscSource, lfo_first: bool| {
                let OscSource { carrier, fm, .. } = o;
                let fm = fm.as_mut().unwrap();
                let mut pull = |q: u64, bufs: &mut Vec<[f32; QUANTUM]>| {
                    if lfo_first {
                        bufs.extend_from_slice(&lfo);
                    }
                    fm.inputs(FmTarget::Carrier, q, bufs, &none.view());
                    if !lfo_first {
                        bufs.extend_from_slice(&lfo);
                    }
                };
                let mut out = [0.0f32; QUANTUM];
                carrier.process(q, &mut out, Some(&mut pull), None);
                out
            };
            let want = render(&mut fm_first, false);
            let other = render(&mut lfo_first, true);
            let bits = |x: &[f32; QUANTUM]| x.map(f32::to_bits);
            assert_eq!(bits(&a), bits(&want), "quantum {k}");
            reordered |= bits(&want) != bits(&other);
        }
        assert!(reordered, "the summing order is observable on this voice");
    }

    #[test]
    fn a_late_busgain_throw_runs_no_modulator() {
        let p = params(
            json!({ "s": "sine", "delay": 0.5, "delaytime": 0.1, "delayfeedback": 0.5, "bus": 1,
                    "busgain": { "$num": "NaN" },
                    "lfo": { "__ids": { "$set": [0] }, "0": { "control": "gain" } } }),
            0.05,
        );
        let (v, late) = Voice::build(
            &p,
            0,
            start(0.1, 0.05),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
            &BusBank::of_keys(vec!["1".into()]),
        )
        .unwrap();
        assert!(late.unwrap().detail.starts_with("TypeError"));
        assert!(!v.connected() && v.send_levels().0.is_some());
        assert!(v.mods().is_empty(), "the loop never ran");
        // Without the throw the lfo connects.
        let p = params(
            json!({ "s": "sine", "lfo": { "__ids": { "$set": [0] }, "0": { "control": "gain" } } }),
            0.05,
        );
        let (v, late) = Voice::build(
            &p,
            0,
            start(0.1, 0.05),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
            &BusBank::default(),
        )
        .unwrap();
        assert!(late.is_none() && v.mods().mods.len() == 1);
    }

    fn build_on(
        v: serde_json::Value,
        orbit: &mut Orbit,
    ) -> Result<(Voice, Option<VoiceError>), VoiceError> {
        Voice::build(
            &params(v, 0.05),
            0,
            start(0.1, 0.05),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            orbit,
            &BusBank::default(),
        )
    }

    #[test]
    fn the_room_send_creates_and_updates_the_orbit_reverb() {
        let mut orbit = bus();
        voice_on(json!({ "s": "sine", "room": 0 }), &mut orbit).unwrap();
        assert!(orbit.reverb().is_none(), "room > 0 only");
        let v = voice_on(
            json!({ "s": "sine", "room": 0.4, "roomsize": 0.1 }),
            &mut orbit,
        )
        .unwrap();
        assert_eq!(v.send_levels().1, Some(0.4));
        let r = orbit.reverb().unwrap();
        assert_eq!(r.settings().duration, Value::Num(0.1));
        assert_eq!(r.latest(), 1);
        // Same settings: no request; a new roomsize: one.
        voice_on(
            json!({ "s": "sine", "room": 0.4, "roomsize": 0.1 }),
            &mut orbit,
        )
        .unwrap();
        voice_on(json!({ "s": "sine", "room": 0.1 }), &mut orbit).unwrap();
        assert_eq!(orbit.reverb().unwrap().latest(), 1);
        voice_on(
            json!({ "s": "sine", "room": 0.4, "roomsize": 0.2 }),
            &mut orbit,
        )
        .unwrap();
        assert_eq!(orbit.reverb().unwrap().latest(), 2);
        // The IR waits for the pause to settle.
        assert!(orbit.reverb().unwrap().ir().is_none());
        assert!(orbit.settle_reverb(0, &mut crate::reverb::IrCache::new()));
        assert_eq!(orbit.reverb().unwrap().ir().unwrap()[0].len(), 13230);
    }

    #[test]
    fn an_ir_error_leaves_the_voice_unconnected() {
        // Without other sends the hap is lost: nothing of it reaches the orbit, and a fresh
        // orbit gets no reverb.
        let mut fresh = bus();
        let e = voice_on(
            json!({ "s": "sine", "room": 0.5, "roomsize": 0 }),
            &mut fresh,
        )
        .unwrap_err();
        assert_eq!(
            (e.kind, e.detail.as_str()),
            (ErrorKind::Ir, "reverb ir length 0")
        );
        assert!(fresh.reverb().is_none());
        assert!(
            build_on(
                json!({ "s": "sine", "room": 0.5, "roomsize": 0 }),
                &mut fresh
            )
            .is_err()
        );
        // A lowpass that throws is a hap error.
        let e = voice_on(
            json!({ "s": "sine", "room": 0.5, "roomsize": 0.1, "roomlp": "x" }),
            &mut fresh,
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Hap);
        assert!(e.detail.starts_with("TypeError"), "{}", e.detail);
        // With a delay send made first, the voice keeps playing into the delay, unconnected
        // to the summing node; the error is reported with it. `Voice::new` returns the error.
        let mut orbit = bus();
        let hap = json!({ "s": "one", "sustain": 1, "delay": 0.5, "delaytime": 0.1,
                          "room": 0.5, "roomsize": -1 });
        let (mut v, late) = build_on(hap.clone(), &mut orbit).unwrap();
        let late = late.unwrap();
        assert_eq!(
            (late.kind, late.detail.as_str()),
            (ErrorKind::Ir, "reverb ir length -66150")
        );
        assert!(!v.connected());
        assert_eq!(v.send_levels(), (Some(0.5), None));
        assert!(v.sends().bus.is_none());
        assert!(orbit.delay().is_some() && orbit.reverb().is_none());
        let mut o = VoiceOutput::new();
        let q = 4480; // 0.1 s is frame 4410
        assert!(v.process(q, &mut o, &mut BusBank::default()));
        assert!(
            !o.post_silent && o.post[0][127] != 0.0,
            "post feeds the delay"
        );
        assert!(
            o.out_silent && o.out[0].iter().all(|x| *x == 0.0),
            "no dry path"
        );
        assert!(o.to_orbit().silent);
        assert_eq!(voice_on(hap, &mut bus()).unwrap_err().kind, ErrorKind::Ir);
        // A level that throws after the room send connected: the room send stays.
        let (v, late) = build_on(
            json!({ "s": "sine", "room": 0.5, "roomsize": 0.1, "dry": 1e40 }),
            &mut bus(),
        )
        .unwrap();
        assert!(late.unwrap().detail.starts_with("TypeError"));
        assert_eq!(
            (v.connected(), v.send_levels().1, v.levels().2),
            (false, Some(0.5), None)
        );
        // A delay send with a room level that throws: the delay send stays.
        let (v, late) = build_on(
            json!({ "s": "sine", "delay": 0.5, "room": { "$num": "Infinity" } }),
            &mut bus(),
        )
        .unwrap();
        assert!(late.is_some());
        assert_eq!(v.send_levels(), (Some(0.5), None));
        assert!(v.sends().bus.is_none());
        // A complete build is connected with no error.
        let (v, late) = build_on(json!({ "s": "sine", "room": 0.5 }), &mut bus()).unwrap();
        assert!(v.connected() && late.is_none());
    }

    #[test]
    fn the_delay_send_creates_and_updates_the_orbit_delay() {
        let mut orbit = bus();
        // No delay send: no delay.
        voice_on(
            json!({ "s": "sine", "delay": 0.5, "delayfeedback": 0 }),
            &mut orbit,
        )
        .unwrap();
        assert!(orbit.delay().is_none());
        // The first send creates it (`.value =` at the pause frame, then the guard at t).
        voice_on(
            json!({ "s": "sine", "delay": 0.5, "delaytime": 0.25 }),
            &mut orbit,
        )
        .unwrap();
        let d = orbit.delay().unwrap();
        assert_eq!(d.delay_time().value(), 0.25);
        assert_eq!(
            d.delay_time().events().len(),
            1,
            "0.25 is exact: the guard skips"
        );
        assert_eq!(d.feedback().value(), 0.5);
        // A later hap with a non-exact time schedules at its t.
        voice_on(
            json!({ "s": "sine", "delay": 0.5, "delaytime": 0.3 }),
            &mut orbit,
        )
        .unwrap();
        let e = orbit.delay().unwrap().delay_time().events();
        assert_eq!((e.len(), e[1].value, e[1].time), (2, 0.3f32, 0.1));
        // The delay is the orbit's: a send of a voice that then fails still updated it
        // (`getDelay` runs before `gainNode(delay)` throws).
        let err = voice_on(
            json!({ "s": "sine", "delay": 1e40, "delayfeedback": 0.25 }),
            &mut orbit,
        )
        .unwrap_err();
        assert!(err.detail.starts_with("TypeError"), "{}", err.detail);
        assert_eq!(orbit.delay().unwrap().feedback().events().len(), 2);
        // A throwing getDelay (a delay time that overflows a float) fails the hap before the
        // send level; on a fresh orbit nothing is created.
        let mut fresh = bus();
        let err = voice_on(
            json!({ "s": "sine", "delay": 0.5, "delaytime": 1e39 }),
            &mut fresh,
        )
        .unwrap_err();
        assert!(err.detail.starts_with("TypeError"), "{}", err.detail);
        assert!(fresh.delay().is_none());
        // A failure before the sends (a bad sound) leaves the orbit alone.
        let mut untouched = bus();
        assert!(voice_on(json!({ "s": "nope", "delay": 0.5 }), &mut untouched).is_err());
        assert!(untouched.delay().is_none());
    }

    #[test]
    fn levels_multiply_in_chain_order() {
        // one (1) * 0.3 * envelope 1 * gain stage * postgain * dry, each a float multiply.
        let p = json!({ "s": "one", "sustain": 1, "gain": 0.7, "velocity": 0.9,
                         "postgain": 1.7, "dry": 0.45 });
        let v = voice(p.clone(), 0.05);
        let stage = (0.7f64 * 0.9) as f32;
        assert_eq!(v.levels(), (stage, 1.7, Some(0.45)));
        let out = run(&mut voice(p, 0.05), 4096).0;
        assert_eq!(out[1000], 1.0f32 * SYNTH_GAIN * stage * 1.7 * 0.45);
        // Defaults: postgain 1, no dry gain; a null postgain is 0, a null dry is no dry gain.
        assert_eq!(
            voice(json!({ "s": "one" }), 0.05).levels(),
            (0.8, 1.0, None)
        );
        let v = voice(json!({ "s": "one", "postgain": null, "dry": null }), 0.05);
        assert_eq!(v.levels(), (0.8, 0.0, None));
        let out = run(
            &mut voice(json!({ "s": "one", "postgain": null }), 0.05),
            4096,
        )
        .0;
        assert!(out.iter().all(|x| *x == 0.0));
        // Zero gain stage: silent, and freed once the source stops.
        let mut v = voice(json!({ "s": "sine", "gain": 0 }), 0.05);
        let (out, end) = run(&mut v, 44100);
        assert!(out.iter().all(|x| *x == 0.0));
        let stop = v.source_frames().1;
        assert!(end > stop && end <= stop + 2 * QUANTUM as u64);
    }

    #[test]
    fn a_resonant_filter_rings_past_the_stop_until_its_tail_ends() {
        let p = json!({ "s": "saw", "note": 45, "cutoff": 1500, "resonance": 15,
                         "release": 0.01 });
        let mut v = voice(p, 0.1);
        let stop = v.source_frames().1 as usize;
        let (out, end) = run(&mut v, 44100);
        // Ringing after the source stopped...
        assert!(out[stop..].iter().any(|x| *x != 0.0));
        // ...then exact silence once the biquad propagates silence; the voice is freed then.
        let last = out.iter().rposition(|x| *x != 0.0).unwrap();
        assert!(last > stop && last < stop + 8 * QUANTUM);
        assert!(end as usize <= last + 2 * QUANTUM);
        // The same voice without a filter ends at the stop.
        let mut plain = voice(json!({ "s": "saw", "note": 45, "release": 0.01 }), 0.1);
        let (out, _) = run(&mut plain, 44100);
        assert!(out[stop..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn chain_error_paths() {
        let e = |v: serde_json::Value| {
            Voice::new(
                &params(v, 0.1),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap_err()
        };
        for bad in [
            json!({ "s": "saw", "cutoff": "x" }),
            json!({ "s": "saw", "cutoff": 1000, "resonance": "x" }),
            json!({ "s": "saw", "hcutoff": 1000, "hresonance": 1e39 }),
            json!({ "s": "saw", "bandf": 1e39 }),
            json!({ "s": "saw", "pan": "left" }),
            json!({ "s": "saw", "postgain": "x" }),
            json!({ "s": "saw", "dry": 1e40 }),
        ] {
            let err = e(bad.clone());
            assert_eq!(err.kind, ErrorKind::Hap);
            assert!(err.detail.starts_with("TypeError"), "{bad}: {}", err.detail);
        }
        // The gain stage throws before the filters, the filters before the panner.
        // (A NaN gain falls back to 1; a NaN velocity makes the stage NaN.)
        assert!(
            e(json!({ "s": "saw", "velocity": "x", "cutoff": "y" }))
                .detail
                .contains("gain")
        );
        assert!(
            e(json!({ "s": "saw", "cutoff": "y", "pan": "x" }))
                .detail
                .contains("frequency")
        );
        assert!(
            e(json!({ "s": "saw", "cutoff": 100, "resonance": "x", "pan": "x" }))
                .detail
                .contains("Q")
        );
        // Extreme but finite values are fine: a huge env saturates at 20000, a huge NaN-free
        // attack just holds.
        assert!(
            Voice::new(
                &params(
                    json!({ "s": "saw", "cutoff": 100, "lpenv": 1e6, "lpattack": 1e9 }),
                    0.1
                ),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_non_finite_output_silences_and_frees_the_voice() {
        // Q = -1e6 dB: the resonance underflows to 0, alpha is infinite, a2 is NaN.
        let mut v = voice(
            json!({ "s": "saw", "cutoff": 1000, "resonance": -1e6 }),
            0.1,
        );
        let (out, end) = run(&mut v, 44100);
        assert!(v.failed());
        assert_eq!(end, QUANTUM as u64);
        assert!(out.iter().all(|x| *x == 0.0));
        // Gains whose product overflows.
        let mut v = voice(json!({ "s": "one", "gain": 1e30, "postgain": 1e30 }), 0.1);
        run(&mut v, 44100);
        assert!(v.failed());
        // A plain voice never fails.
        let mut v = voice(
            json!({ "s": "saw", "cutoff": 1000, "resonance": 1e6 }),
            0.05,
        );
        let (out, _) = run(&mut v, 44100);
        assert!(!v.failed());
        assert!(out.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn gain_node_silence_hints() {
        // Silent input before any signal: zeros, silent, the param still evaluated.
        let mut env = Param::gain(1.0);
        env.set_value(0.5, 0).unwrap();
        let mut g = GainNode::new(env);
        let mut buf = [[1.0f32; QUANTUM]; 1];
        assert!(
            !g.process(0, &mut buf, false, &ParamInputs::NONE),
            "first quantum: processed"
        );
        assert_eq!(buf[0][0], 0.5);
        // Silent input right after signal: still processed (zero tail), output silent.
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(g.process(128, &mut buf, true, &ParamInputs::NONE));
        // Later: propagated without processing.
        assert!(g.process(256, &mut buf, true, &ParamInputs::NONE));
        assert_eq!(buf[0], [0.0; QUANTUM]);
        // A gain of exactly 0 makes silence out of signal.
        let mut z = GainNode::new(Param::gain(0.0));
        let mut buf = [[1.0f32; QUANTUM]; 2];
        assert!(z.process(384, &mut buf, false, &ParamInputs::NONE));
        assert_eq!(buf, [[0.0; QUANTUM]; 2]);
        // A sample-accurate gain on stereo input scales both channels by the same values.
        let mut ramp = Param::gain(1.0);
        ramp.set_value_at_time(0.0, 0.0, 0).unwrap();
        ramp.linear_ramp_to(1.0, 127.0 / SR, 0).unwrap();
        let mut g = GainNode::new(ramp);
        let mut buf = [[2.0f32; QUANTUM]; 2];
        assert!(!g.process(0, &mut buf, false, &ParamInputs::NONE));
        assert_eq!(buf[0], buf[1]);
        assert_eq!((buf[0][0], buf[0][127]), (0.0, 2.0));
        // Constant gains: silent input or 0 give silence, else a multiply.
        let mut buf = [[3.0f32; QUANTUM]; 1];
        assert!(!scale(&mut buf, 0.5, false));
        assert_eq!(buf[0][7], 1.5);
        assert!(scale(&mut buf, 0.5, true));
        assert_eq!(buf[0][7], 0.0);
        let mut buf = [[3.0f32; QUANTUM]; 1];
        assert!(scale(&mut buf, 0.0, false));
        assert_eq!(buf[0][7], 0.0);
    }

    #[test]
    fn gain_node_with_a_connection() {
        // `new GainNode(ac, { gain: 0.5 })` with a connected input: sample accurate in every
        // quantum, intrinsic + input per frame, NaN replaced by the default 1, then Vclip.
        let mut gain = Param::gain(1.0);
        gain.set_initial_value(0.5, "gain").unwrap();
        let mut g = GainNode::new(gain);
        let mut lfo = [0.25f32; QUANTUM];
        lfo[1] = f32::NAN;
        lfo[2] = f32::INFINITY;
        lfo[3] = -0.5;
        let inputs = ParamInputs {
            bufs: vec![lfo],
            connected: true,
        };
        for q in [0u64, 1280, 128_000] {
            let mut buf = [[0.5f32; QUANTUM]; 2];
            assert!(!g.process(q, &mut buf, false, &inputs), "quantum {q}");
            assert_eq!(buf[0], buf[1]);
            assert_eq!(buf[0][0], 0.5 * 0.75);
            assert_eq!(buf[0][1], 0.5, "NaN sum -> default 1");
            assert_eq!(buf[0][2], 0.5 * f32::MAX, "clipped at FLT_MAX");
            assert_eq!(buf[0][3].to_bits(), 0.0f32.to_bits());
        }
        // Connected with a silent input this quantum (no buffers): the intrinsic value per
        // frame, and a gain of exactly 0 is not a silent output (the sample-accurate path).
        let mut zero_gain = Param::gain(1.0);
        zero_gain.set_initial_value(0.0, "gain").unwrap();
        let mut z = GainNode::new(zero_gain.clone());
        let connected = ParamInputs {
            bufs: Vec::new(),
            connected: true,
        };
        let mut buf = [[0.7f32; QUANTUM]; 1];
        assert!(!z.process(0, &mut buf, false, &connected));
        assert!(buf[0].iter().all(|x| x.to_bits() == 0));
        // Unconnected, the same gain gives a silent bus.
        let mut z = GainNode::new(zero_gain);
        let mut buf = [[0.7f32; QUANTUM]; 1];
        assert!(z.process(0, &mut buf, false, &ParamInputs::NONE));
        // A silent input: zeros and a silent output; the param is still evaluated with its
        // inputs (an event in that quantum is consumed).
        let mut ev = Param::gain(1.0);
        ev.set_value_at_time(0.25, 128.0 / SR, 0).unwrap();
        let mut g = GainNode::new(ev);
        let mut buf = [[0.0f32; QUANTUM]; 1];
        assert!(g.process(128, &mut buf, true, &inputs));
        assert!(buf[0].iter().all(|x| x.to_bits() == 0));
        assert_eq!(g.gain.value(), 0.25, "the timeline was rendered");
    }

    #[test]
    fn the_pulse_width_lfo() {
        let pulse = |v: serde_json::Value| try_voice(v, 0.25);
        let lfo_of = |v: &Voice| match v.source() {
            Source::Worklet(w) => w.pw_lfo.clone(),
            other => panic!("{other:?}"),
        };
        // No pwrate, no pwsweep: no LFO, `pw_lfo: [undefined]`.
        let v = pulse(json!({ "s": "pulse" })).unwrap();
        assert!(lfo_of(&v).is_none());
        assert_eq!(
            v.node_table().get(FxKey::Main, "pw_lfo"),
            Some(&NodeList::Array(vec![None]))
        );
        // pwsweep 0 with a rate: none either.
        let v = pulse(json!({ "s": "pulse", "pwrate": 3, "pwsweep": 0 })).unwrap();
        assert!(lfo_of(&v).is_none());
        // pwrate alone: sweep 0.3. The triangle with time = begin, dcoffset -0.5, curve 1,
        // min = -0.5 * 0.3, max = -0.5 * 0.3 + 0.3, ending with the pulse.
        let v = pulse(json!({ "s": "pulse", "pwrate": 2 })).unwrap();
        let l = lfo_of(&v).unwrap();
        let p = |k: usize| l.params[k].value();
        assert_eq!(p(lfo::FREQUENCY), 2.0);
        assert_eq!(p(lfo::DEPTH), 0.3);
        assert_eq!(p(lfo::SHAPE), 0.0);
        assert_eq!(p(lfo::SKEW), 0.5);
        assert_eq!(p(lfo::DCOFFSET), -0.5);
        assert_eq!(p(lfo::CURVE), 1.0);
        assert_eq!(p(lfo::MIN), (-0.5f64 * 0.3) as f32);
        assert_eq!(p(lfo::MAX), (-0.5f64 * 0.3 + 0.3) as f32);
        assert_eq!((p(lfo::BEGIN), p(lfo::TIME)), (0.0, 0.0));
        let Source::Worklet(w) = v.source() else {
            unreachable!()
        };
        assert_eq!(p(lfo::END), w.end, "the pulse's end");
        assert_eq!(
            v.node_table().get(FxKey::Main, "pw_lfo"),
            Some(&NodeList::Array(vec![Some(Node::PulseLfo)]))
        );
        // pwsweep alone: rate 1; null controls take the defaults.
        let v = pulse(json!({ "s": "pulse", "pwsweep": 0.5 })).unwrap();
        let l = lfo_of(&v).unwrap();
        assert_eq!(
            (
                l.params[lfo::FREQUENCY].value(),
                l.params[lfo::DEPTH].value()
            ),
            (1.0, 0.5)
        );
        let v = pulse(json!({ "s": "pulse", "pwrate": null, "pwsweep": null })).unwrap();
        assert!(lfo_of(&v).is_none());
        // A NaN sweep is `!= 0`: getLfo's `.value = NaN` throws, the build stops.
        let e = pulse(json!({ "s": "pulse", "pwsweep": "x" })).unwrap_err();
        assert!(e.detail.contains("TypeError"), "{}", e.detail);
        let e = pulse(json!({ "s": "pulse", "pwrate": "x" })).unwrap_err();
        assert!(e.detail.contains("TypeError"), "{}", e.detail);
        // Other sounds ignore both and register no `pw_lfo`.
        for s in ["supersaw", "sine"] {
            let v = try_voice(json!({ "s": s, "pwrate": 2, "pwsweep": "x" }), 0.25).unwrap();
            assert_eq!(v.node_table().get(FxKey::Main, "pw_lfo"), None, "{s}");
        }
        // The modulation sounds: with the LFO the pulse differs from the plain one only after
        // the LFO's first active quantum.
        let mut a = pulse(json!({ "s": "pulse", "note": 57 })).unwrap();
        let mut b =
            pulse(json!({ "s": "pulse", "note": 57, "pwrate": 8, "pwsweep": 0.8 })).unwrap();
        let (la, _, _) = run_stereo(&mut a, 4096);
        let (lb, _, _) = run_stereo(&mut b, 4096);
        assert_eq!(
            la[..128],
            lb[..128],
            "the onset quantum: the LFO outputs nothing yet"
        );
        assert_ne!(la[128..256], lb[128..256]);
    }

    // ---- noise sources, the noise mix and FM

    fn voice_at(v: serde_json::Value, dur: f64, t: f64, now: u64, noise: &mut NoiseCache) -> Voice {
        Voice::new(
            &params(v, dur),
            77,
            StartInfo {
                t,
                hap_duration: dur,
                now_frame: now,
                cycle: 0.0,
                phase_sec: 0.0,
                cps: 2.0,
            },
            &mut WaveTables::new(),
            noise,
            &mut bus(),
        )
        .unwrap()
    }

    fn noise_of(v: &Voice) -> &NoiseSource {
        match v.source() {
            Source::Noise(n) => n,
            other => panic!("not a noise source: {other:?}"),
        }
    }

    #[test]
    fn noise_sounds_play_their_buffers_through_the_synth_chain() {
        use crate::noise::noise_buffer;
        for (s, kind) in [
            ("white", NoiseKind::White),
            ("pink", NoiseKind::Pink),
            ("brown", NoiseKind::Brown),
        ] {
            let mut cache = NoiseCache::new(5);
            let mut v = voice_at(json!({ "s": s }), 0.1, 0.0, 0, &mut cache);
            let buf = noise_buffer(kind, 5, 0.02, 0);
            assert_eq!(noise_of(&v).buffer(), &buf[..], "{s}");
            assert!(cache.is_cached(kind));
            // The same chain on a constant 1 gives the per-frame gain.
            let mut one = voice_at(json!({ "s": "one" }), 0.1, 0.0, 0, &mut cache);
            let (out, _) = run(&mut v, 44100);
            let (gain, _) = run(&mut one, 44100);
            for i in 0..4000 {
                let want = f64::from(buf[i]) * f64::from(gain[i]);
                assert!((f64::from(out[i]) - want).abs() < 1e-6, "{s} {i}");
            }
            let stop = v.source_frames().1 as usize;
            assert!(out[stop..].iter().all(|x| *x == 0.0), "{s}: stopped");
            assert!(
                out[stop - 800..stop - 450].iter().any(|x| *x != 0.0),
                "{s}: plays on"
            );
        }
    }

    #[test]
    fn noise_voices_share_the_cached_buffer() {
        let mut cache = NoiseCache::new(0);
        let a = voice_at(json!({ "s": "pink" }), 0.1, 0.0, 0, &mut cache);
        let b = voice_at(json!({ "s": "pink", "note": 40 }), 0.1, 0.2, 0, &mut cache);
        assert!(std::ptr::eq(
            noise_of(&a).buffer().as_ptr(),
            noise_of(&b).buffer().as_ptr()
        ));
    }

    #[test]
    fn crackle_uses_the_voice_seed_and_the_density() {
        use crate::noise::noise_buffer;
        let mut cache = NoiseCache::new(0);
        for (density, want) in [
            (None, 0.02),
            (Some(json!(2)), 2.0),
            (Some(json!("30")), 30.0),
            (Some(json!(null)), 0.0),
        ] {
            let mut m = json!({ "s": "crackle" });
            if let Some(d) = density {
                m["density"] = d;
            }
            let v = voice_at(m, 0.1, 0.0, 0, &mut cache);
            assert_eq!(
                noise_of(&v).buffer(),
                &noise_buffer(NoiseKind::Crackle, 0, want, 77)[..],
                "{want}"
            );
        }
        assert!(!cache.is_cached(NoiseKind::Crackle));
    }

    #[test]
    fn noise_starts_on_the_next_frame_and_stops_at_env_end() {
        let mut cache = NoiseCache::new(0);
        let v = voice_at(json!({ "s": "white" }), 0.1, 100.5 / SR, 0, &mut cache);
        let n = noise_of(&v);
        assert_eq!(n.schedule().start_frame(), 101);
        assert_eq!(n.schedule().end_frame(), Some(v.source_frames().1));
    }

    #[test]
    fn the_noise_mix_uses_wetfade() {
        assert_eq!(wetfade(0.0), 1.0);
        assert_eq!(wetfade(0.49), 1.0);
        assert_eq!(wetfade(0.5), 1.0);
        assert_eq!(wetfade(0.75), 0.5);
        assert_eq!(wetfade(1.0), 0.0);
        assert_eq!(wetfade(1.5), -1.0);
        assert!(wetfade(f64::NAN).is_nan());
        let mut cache = NoiseCache::new(0);
        for (noise, dry, wet) in [
            (json!(0.3), 1.0, 0.6),
            (json!(0.8), 0.4, 1.0),
            (json!(1.2), -0.4, 1.0),
            (json!("0"), 1.0, 0.0),
            (json!(true), 0.0, 1.0),
        ] {
            let v = voice_at(
                json!({ "s": "saw", "noise": noise }),
                0.1,
                0.0,
                0,
                &mut cache,
            );
            let m = osc_source(&v).noise_mix.as_ref().expect("a mix");
            assert_eq!((m.dry, m.wet), (dry as f32, wet as f32), "{noise}");
            assert_eq!(m.pink.buffer(), cache.get(NoiseKind::Pink, 0.0, 0).as_ref());
        }
        for noise in [json!(0), json!(null), json!(""), json!(f64::NAN)] {
            let v = voice_at(
                json!({ "s": "saw", "noise": noise }),
                0.1,
                0.0,
                0,
                &mut cache,
            );
            assert!(osc_source(&v).noise_mix.is_none(), "{noise}");
        }
        let e = Voice::new(
            &params(json!({ "s": "saw", "noise": "abc" }), 0.1),
            0,
            start(0.0, 0.1),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap_err();
        assert!(e.detail.contains("gain"), "{}", e.detail);
    }

    #[test]
    fn a_fully_wet_mix_is_the_pink_voice() {
        let mut cache = NoiseCache::new(9);
        let mut mix = voice_at(json!({ "s": "saw", "noise": 1 }), 0.1, 0.01, 0, &mut cache);
        let mut pink = voice_at(json!({ "s": "pink" }), 0.1, 0.01, 0, &mut cache);
        let (a, _) = run(&mut mix, 44100);
        let (b, _) = run(&mut pink, 44100);
        let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
        assert_eq!(first, None, "{:?}", first.map(|i| (a[i], b[i])));
        assert_eq!(a.len(), b.len());
        // A dry-only mix (wet gain 0) is the oscillator alone.
        let mut dry = voice_at(
            json!({ "s": "saw", "noise": "0" }),
            0.1,
            0.01,
            0,
            &mut cache,
        );
        let mut osc = voice_at(json!({ "s": "saw" }), 0.1, 0.01, 0, &mut cache);
        assert_eq!(run(&mut dry, 44100).0, run(&mut osc, 44100).0);
    }

    fn fm_of(v: &Voice) -> &Fm {
        osc_source(v).fm.as_ref().expect("fm")
    }

    fn mod_osc(m: &FmModulator) -> &Oscillator {
        match m.source() {
            ModSource::Osc(o) => o,
            ModSource::Noise(_) => panic!("a noise modulator"),
        }
    }

    #[test]
    fn an_fm_pair_reads_the_carrier_frequency() {
        let mut cache = NoiseCache::new(0);
        let v = voice_at(
            json!({ "s": "sine", "note": 69, "fmi": 2, "fmh": 1.5 }),
            0.1,
            0.5,
            256,
            &mut cache,
        );
        let fm = fm_of(&v);
        assert_eq!(fm.modulators().len(), 1);
        let m = &fm.modulators()[0];
        assert_eq!(m.index, 1);
        assert_eq!(m.freq, 660.0);
        assert!(m.envelope.is_none());
        assert_eq!(mod_osc(m).frequency.value(), 660.0);
        assert_eq!(mod_osc(m).wave().kind(), WaveKind::Sine);
        let c: Vec<(usize, FmTarget, f32, f32)> = fm
            .connections()
            .iter()
            .map(|c| {
                (
                    c.source,
                    c.target,
                    c.amount.gain.value(),
                    c.freq.gain.value(),
                )
            })
            .collect();
        assert_eq!(c, [(0, FmTarget::Carrier, 2.0, 660.0)]);
        // `start()` with no time: the scheduling frame; the voice's stop time.
        assert_eq!(mod_osc(m).schedule().start_frame(), 256);
        assert_eq!(mod_osc(m).schedule().end_frame(), Some(v.source_frames().1));
        // fmh defaults to 1 and the modulator follows the carrier's float frequency (octave
        // included), not detune or vibrato.
        let v = voice_at(
            json!({ "s": "sine", "note": 60.3, "octave": 1, "fm": 0, "fmi": 1, "vib": 5 }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        let carrier = osc_source(&v).carrier.frequency.value();
        assert_eq!(fm_of(&v).modulators()[0].freq, carrier);
    }

    #[test]
    fn fm_modulators_first_play_with_the_carrier() {
        let mut cache = NoiseCache::new(0);
        let mut v = voice_at(
            json!({ "s": "sine", "fmi": 1 }),
            0.1,
            1000.0 / SR,
            0,
            &mut cache,
        );
        let mut buf = VoiceOutput::new();
        for q in 0..7u64 {
            v.process(q * 128, &mut buf, &mut BusBank::default());
        }
        let m = &fm_of(&v).modulators()[0];
        assert_eq!(
            mod_osc(m).schedule().state(),
            crate::osc::PlaybackState::Scheduled
        );
        assert_eq!(mod_osc(m).phase(), 0.0);
        v.process(896, &mut buf, &mut BusBank::default());
        let m = &fm_of(&v).modulators()[0];
        assert_eq!(
            mod_osc(m).schedule().state(),
            crate::osc::PlaybackState::Playing
        );
        assert!(mod_osc(m).phase() > 0.0);
    }

    #[test]
    fn fm_changes_the_sound_and_stays_finite() {
        let mut cache = NoiseCache::new(0);
        let mut plain = voice_at(json!({ "s": "sine" }), 0.1, 0.0, 0, &mut cache);
        let mut fm = voice_at(json!({ "s": "sine", "fmi": 3 }), 0.1, 0.0, 0, &mut cache);
        let (a, _) = run(&mut plain, 44100);
        let (b, _) = run(&mut fm, 44100);
        assert_ne!(a, b);
        assert!(b.iter().all(|x| x.is_finite() && x.abs() <= 0.25));
    }

    #[test]
    fn the_matrix_creates_modulators_on_first_use() {
        let mut cache = NoiseCache::new(0);
        // fmi (1 -> 0), fmi13 (1 -> 3), fmi2 (2 -> 1) in matrix order: modulators 1, 3, 2.
        let v = voice_at(
            json!({ "s": "sine", "note": 69, "fmi": 1, "fmi2": 2, "fmi13": 3, "fmh3": 4, "fmwave2": "square" }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        let fm = fm_of(&v);
        let order: Vec<usize> = fm.modulators().iter().map(|m| m.index).collect();
        assert_eq!(order, [1, 3, 2]);
        assert_eq!(mod_osc(&fm.modulators()[2]).wave().kind(), WaveKind::Square);
        let c: Vec<(usize, FmTarget, f32)> = fm
            .connections()
            .iter()
            .map(|c| (c.source, c.target, c.amount.gain.value()))
            .collect();
        assert_eq!(
            c,
            [
                (0, FmTarget::Carrier, 1.0),
                (0, FmTarget::Modulator(1), 3.0),
                (2, FmTarget::Modulator(0), 2.0),
            ]
        );
        // Each connection's frequency gain is its source's modfreq (440 * 4 for modulator 3).
        assert_eq!(fm.modulators()[1].freq, 1760.0);
        assert_eq!(fm.connections()[1].freq.gain.value(), 440.0);
    }

    #[test]
    fn fm_envelopes_follow_fmenv() {
        let mut cache = NoiseCache::new(0);
        let v = voice_at(
            json!({ "s": "sine", "fmi": 1, "fmattack": 0.02, "fmi2": 1, "fmenv2": "lin", "fmdecay2": null }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        let fm = fm_of(&v);
        let kinds = |m: &FmModulator| {
            m.envelope
                .as_ref()
                .expect("an envelope")
                .gain
                .events()
                .iter()
                .map(|e| e.kind)
                .collect::<Vec<_>>()
        };
        // fmenv defaults to 'exp': exponential ramps (from 0.001, not 0).
        let exp = kinds(&fm.modulators()[0]);
        assert!(exp.contains(&RampKind::Exponential), "{exp:?}");
        assert_eq!(
            fm.modulators()[0].envelope.as_ref().unwrap().gain.events()[0].value,
            0.001
        );
        // A null control still counts as defined; 'lin' is linear.
        let lin = kinds(&fm.modulators()[1]);
        assert!(lin.contains(&RampKind::Linear) && !lin.contains(&RampKind::Exponential));
        // No envelope controls: no envelope.
        let v = voice_at(json!({ "s": "sine", "fmi": 1 }), 0.1, 0.0, 0, &mut cache);
        assert!(fm_of(&v).modulators()[0].envelope.is_none());
    }

    #[test]
    fn noise_modulators_have_no_frequency_input() {
        let mut cache = NoiseCache::new(0);
        let v = voice_at(
            json!({ "s": "sine", "fmi": 0.5, "fmwave": "white", "fmi2": 1, "fmi3": 2, "fmwave3": "crackle" }),
            0.1,
            0.0,
            128,
            &mut cache,
        );
        let fm = fm_of(&v);
        assert_eq!(fm.modulators().len(), 3);
        match fm.modulators()[0].source() {
            ModSource::Noise(n) => {
                assert_eq!(n.buffer(), cache.get(NoiseKind::White, 0.0, 0).as_ref());
                assert_eq!(n.schedule().start_frame(), 128);
            }
            ModSource::Osc(_) => panic!("white is a noise modulator"),
        }
        match fm.modulators()[2].source() {
            ModSource::Noise(n) => assert_eq!(
                n.buffer(),
                &crate::noise::noise_buffer(NoiseKind::Crackle, 0, 2.0, 77)[..]
            ),
            ModSource::Osc(_) => panic!("crackle is a noise modulator"),
        }
        // fmi2 (2 -> 1) targets the white noise: skipped; fmi3 (3 -> 2) is kept.
        let c: Vec<(usize, FmTarget)> = fm
            .connections()
            .iter()
            .map(|c| (c.source, c.target))
            .collect();
        assert_eq!(c, [(0, FmTarget::Carrier), (2, FmTarget::Modulator(1))]);
        // A non-string fmwave is not a noise name: its String() selects the oscillator type.
        let v = voice_at(
            json!({ "s": "sine", "fmi": 1, "fmwave": ["sawtooth"] }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        assert_eq!(
            mod_osc(&fm_of(&v).modulators()[0]).wave().kind(),
            WaveKind::Sawtooth
        );
        let v = voice_at(
            json!({ "s": "sine", "fmi": 1, "fmwave": "bogus" }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        assert_eq!(
            mod_osc(&fm_of(&v).modulators()[0]).wave().kind(),
            WaveKind::Sine
        );
    }

    #[test]
    fn fm_error_paths() {
        let e = |v: serde_json::Value| {
            Voice::new(
                &params(v, 0.1),
                0,
                start(0.0, 0.1),
                &mut WaveTables::new(),
                &mut NoiseCache::new(0),
                &mut bus(),
            )
            .unwrap_err()
        };
        let custom = e(json!({ "s": "sine", "fmi": 1, "fmwave": "custom" }));
        assert_eq!(custom.kind, ErrorKind::Hap);
        assert!(
            custom.detail.starts_with("InvalidStateError"),
            "{}",
            custom.detail
        );
        assert!(custom.detail.contains("'custom'"));
        assert!(
            e(json!({ "s": "sine", "fmi": 1, "fmh": 1e40 }))
                .detail
                .contains("frequency")
        );
        // A noise modulator has no frequency param, but its Gain(modfreq) still converts it.
        assert!(
            e(json!({ "s": "sine", "fmi": 1, "fmh": 1e40, "fmwave": "pink" }))
                .detail
                .contains("gain")
        );
        assert!(
            e(json!({ "s": "sine", "fmi": "abc" }))
                .detail
                .contains("gain")
        );
        assert!(
            e(json!({ "s": "sine", "fmi": 1e39 }))
                .detail
                .contains("gain")
        );
        assert!(
            e(json!({ "s": "sine", "fmi": 1, "fmrelease": "Infinity" }))
                .detail
                .contains("TypeError")
        );
        // Falsy amounts build nothing.
        for amt in [json!(0), json!(null), json!(""), json!(false)] {
            let v = voice(json!({ "s": "sine", "fmi": amt }), 0.1);
            assert!(osc_source(&v).fm.is_none());
        }
        // `one` ignores FM and the noise mix.
        let v = voice(json!({ "s": "one", "fmi": 1, "noise": 0.5 }), 0.1);
        assert!(matches!(v.source(), Source::Constant(_)));
    }

    #[test]
    fn fm_cycles_feed_back_one_quantum_late() {
        let mut cache = NoiseCache::new(0);
        for m in [
            json!({ "s": "sine", "fmi": 1, "fmi11": 1 }),
            json!({ "s": "sine", "fmi": 1, "fmi11": 2, "fmattack": 0.01 }),
            json!({ "s": "sine", "fmi": 1, "fmi2": 1, "fmi12": 3 }),
        ] {
            let mut v = voice_at(m.clone(), 0.1, 0.0, 0, &mut cache);
            let (out, _) = run(&mut v, 44100);
            assert!(out.iter().all(|x| x.is_finite()), "{m}");
            assert!(out.iter().any(|x| *x != 0.0), "{m}");
        }
    }

    #[test]
    fn huge_fm_indexes_clip_the_carrier_frequency() {
        let mut cache = NoiseCache::new(0);
        let mut v = voice_at(
            json!({ "s": "sine", "fmi": 1e9, "fmi2": 1e9 }),
            0.1,
            0.0,
            0,
            &mut cache,
        );
        let (out, _) = run(&mut v, 44100);
        assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 0.25));
    }

    // ---- the worklet synths (supersaw, pulse)

    fn worklet_of(v: &Voice) -> &WorkletSource {
        match v.source() {
            Source::Worklet(w) => w,
            other => panic!("not a worklet synth: {other:?}"),
        }
    }

    fn supersaw_of(v: &Voice) -> &SupersawNode {
        match &worklet_of(v).synth {
            WorkletSynth::Supersaw(n) => n,
            WorkletSynth::Pulse(_) => panic!("a pulse"),
        }
    }

    fn pulse_of(v: &Voice) -> &PulseNode {
        match &worklet_of(v).synth {
            WorkletSynth::Pulse(n) => n,
            WorkletSynth::Supersaw(_) => panic!("a supersaw"),
        }
    }

    /// The `.value =` events of a param: one `setValueAtTime(v, now)`.
    fn set_value_of(p: &Param) -> f32 {
        assert_eq!(p.events().len(), 1, "{p:?}");
        assert_eq!(p.events()[0].kind, RampKind::SetValue);
        p.events()[0].value
    }

    #[test]
    fn worklet_synths_connect_straight_to_their_envelope_in_stereo() {
        let v = voice(json!({ "s": "supersaw", "note": 45 }), 0.2);
        // envGain first (no Gain(0.3)), then the gain stage and post.
        assert_eq!(v.stages().len(), 3);
        let Stage::Gain(env) = &v.stages()[0] else {
            panic!("envGain first")
        };
        let peak = (0.3 * (1.0 / 5.0f64.sqrt())) as f32;
        assert!(
            env.gain.events().iter().any(|e| e.value == peak),
            "{:?}",
            env.gain.events()
        );
        assert_eq!(v.output_channels(), 2);
        let p = voice(json!({ "s": "pulse", "note": 45 }), 0.2);
        let Stage::Gain(env) = &p.stages()[0] else {
            panic!("envGain first")
        };
        assert!(env.gain.events().iter().any(|e| e.value == 1.0));
        assert!(!env.gain.events().iter().any(|e| e.value == 0.3));
        assert_eq!(p.output_channels(), 2);
        // unison(2.5): 0.3 * (1 / sqrt(2.5)), evaluated in that order.
        let v = voice(json!({ "s": "supersaw", "unison": 2.5 }), 0.2);
        let Stage::Gain(env) = &v.stages()[0] else {
            panic!()
        };
        let peak = (0.3 * (1.0 / 2.5f64.sqrt())) as f32;
        assert!(env.gain.events().iter().any(|e| e.value == peak));
    }

    #[test]
    fn supersaw_values_in_order_and_defaults() {
        let t = 0.1;
        let v = Voice::new(
            &params(json!({ "s": "supersaw", "note": 57, "release": 0.2 }), 0.25),
            0,
            start(t, 0.25),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            &mut bus(),
        )
        .unwrap();
        let n = supersaw_of(&v);
        let ps = &n.params;
        assert_eq!(set_value_of(&ps[supersaw::FREQUENCY]), 220.0);
        assert_eq!(set_value_of(&ps[supersaw::BEGIN]), t as f32);
        assert_eq!(
            set_value_of(&ps[supersaw::END]),
            (t + 0.25 + 0.2 + 0.01) as f32
        );
        assert_eq!(set_value_of(&ps[supersaw::FREQSPREAD]), 0.18);
        assert_eq!(set_value_of(&ps[supersaw::VOICES]), 5.0);
        assert_eq!(set_value_of(&ps[supersaw::PANSPREAD]), 0.6);
        assert!(
            ps[supersaw::DETUNE].events().is_empty(),
            "detune is never set"
        );
        assert_eq!(worklet_of(&v).end, (t + 0.25 + 0.2 + 0.01) as f32);
        // unison clamps to 1..100; one voice has panspread 0; spread clamps to 0..1; detune
        // ?? n ?? 0.18.
        let p = |m: serde_json::Value| {
            let v = voice(m, 0.1);
            let n = supersaw_of(&v);
            (
                set_value_of(&n.params[supersaw::VOICES]),
                set_value_of(&n.params[supersaw::PANSPREAD]),
                set_value_of(&n.params[supersaw::FREQSPREAD]),
            )
        };
        assert_eq!(p(json!({ "s": "supersaw", "unison": 500 })).0, 100.0);
        assert_eq!(p(json!({ "s": "supersaw", "unison": 0 })), (1.0, 0.0, 0.18));
        assert_eq!(p(json!({ "s": "supersaw", "unison": null })).0, 1.0);
        assert_eq!(
            p(json!({ "s": "supersaw", "unison": 1, "spread": 1 })).1,
            0.0
        );
        assert_eq!(
            p(json!({ "s": "supersaw", "unison": 3, "spread": 7 })).1,
            1.0
        );
        assert_eq!(
            p(json!({ "s": "supersaw", "unison": 3, "spread": -7 })).1,
            0.0
        );
        assert_eq!(
            p(json!({ "s": "supersaw", "unison": 3, "spread": null })).1,
            0.0
        );
        assert_eq!(p(json!({ "s": "supersaw", "n": 3 })).2, 3.0);
        assert_eq!(p(json!({ "s": "supersaw", "n": 3, "detune": 0.4 })).2, 0.4);
        assert_eq!(p(json!({ "s": "supersaw", "n": 3, "detune": null })).2, 3.0);
        assert_eq!(p(json!({ "s": "supersaw", "n": null })).2, 0.18);
        assert_eq!(p(json!({ "s": "supersaw", "unison": 2.5 })).0, 2.5);
        // The voice seed reaches the processor (`processorOptions: { seed: value._voiceSeed }`).
        let v = voice_at(
            json!({ "s": "supersaw" }),
            0.1,
            0.0,
            0,
            &mut NoiseCache::new(0),
        );
        assert_eq!(supersaw_of(&v).proc, crate::supersaw::Supersaw::new(77));
        assert_ne!(supersaw_of(&v).proc, crate::supersaw::Supersaw::new(78));
    }

    #[test]
    fn pulse_values_in_order_and_defaults() {
        let v = voice(json!({ "s": "pulse", "note": 45 }), 0.2);
        let n = pulse_of(&v);
        assert_eq!(set_value_of(&n.params[pulse::FREQUENCY]), 110.0);
        assert_eq!(set_value_of(&n.params[pulse::BEGIN]), 0.0);
        assert_eq!(
            set_value_of(&n.params[pulse::END]),
            (0.2 + 0.01 + 0.01) as f32
        );
        assert_eq!(set_value_of(&n.params[pulse::PULSEWIDTH]), 0.5);
        assert!(n.params[pulse::DETUNE].events().is_empty());
        let w = |m| set_value_of(&pulse_of(&voice(m, 0.2)).params[pulse::PULSEWIDTH]);
        assert_eq!(w(json!({ "s": "pulse", "pw": 0.1 })), 0.1);
        assert_eq!(w(json!({ "s": "pulse", "pw": null })), 0.0);
        assert_eq!(
            w(json!({ "s": "pulse", "pw": 7 })),
            7.0,
            "the param range is not clamped"
        );
        // The supersaw's keys do nothing to a pulse, nor the noise mix or partials.
        let v = voice(
            json!({ "s": "pulse", "noise": 0.5, "unison": 3, "partials": [1, 0.5] }),
            0.2,
        );
        assert!(worklet_of(&v).fm.is_none());
    }

    #[test]
    fn worklet_synth_throw_sites_in_order() {
        let e = |m| try_voice(m, 0.1).unwrap_err();
        // `getFrequencyFromValue` first (a bad note name throws there), then each `.value =`.
        assert_eq!(
            e(json!({ "s": "supersaw", "note": "x9", "unison": "x" })).detail,
            "not a note: \"x9\""
        );
        assert!(
            e(json!({ "s": "pulse", "note": "c-" }))
                .detail
                .starts_with("TypeError")
        );
        for m in [
            json!({ "s": "supersaw", "detune": "x" }),
            json!({ "s": "supersaw", "detune": 1e39 }),
            json!({ "s": "supersaw", "unison": "x" }),
            json!({ "s": "supersaw", "unison": 3, "spread": "x" }),
            json!({ "s": "supersaw", "release": { "$num": "Infinity" } }),
            json!({ "s": "pulse", "pw": "x" }),
            json!({ "s": "pulse", "freq": 1e39 }),
        ] {
            let err = e(m.clone());
            assert_eq!(err.kind, ErrorKind::Hap, "{m}");
            assert!(err.detail.starts_with("TypeError"), "{m}: {}", err.detail);
        }
        // The pitch envelope, then the vibrato, then FM.
        assert!(
            e(json!({ "s": "pulse", "penv": 1e300, "pattack": 0.1, "vib": 1e39 }))
                .detail
                .contains("TypeError")
        );
        assert!(
            e(json!({ "s": "pulse", "vib": 1e39, "fmi": 1e39 }))
                .detail
                .contains("frequency")
        );
        assert!(
            e(json!({ "s": "supersaw", "fmi": 1e39 }))
                .detail
                .contains("gain")
        );
    }

    #[test]
    fn worklet_synth_node_table_entries() {
        let v = voice(
            json!({ "s": "supersaw", "vib": 5, "fmi": 2, "fmi13": 1 }),
            0.1,
        );
        let t = v.node_table();
        assert_eq!(
            t.get(FxKey::Main, "source"),
            Some(&NodeList::Array(vec![Some(Node::Source)]))
        );
        assert_eq!(
            t.get(FxKey::Main, "vib"),
            Some(&NodeList::Array(vec![Some(Node::Vibrato)]))
        );
        assert_eq!(
            t.get(FxKey::Main, "vib_gain"),
            Some(&NodeList::Array(vec![Some(Node::VibratoGain)]))
        );
        // Modulators in creation order: fm_1 (fmi), fm_3 (the target of fmi13).
        assert_eq!(
            t.get(FxKey::Main, "fm_1"),
            Some(&NodeList::Array(vec![Some(Node::FmOsc(0))]))
        );
        assert_eq!(
            t.get(FxKey::Main, "fm_3"),
            Some(&NodeList::Array(vec![Some(Node::FmOsc(1))]))
        );
        assert!(t.get(FxKey::Main, "fm_1_gain").is_some());
        let p = voice(json!({ "s": "pulse" }), 0.1);
        assert!(p.node_table().get(FxKey::Main, "vib").is_none());
        assert!(p.node_table().get(FxKey::Main, "fm_1").is_none());
        assert!(p.node_table().get(FxKey::Main, "source").is_some());
    }

    #[test]
    fn worklet_params_are_pulled_from_the_pause_frame() {
        // A pulse with vibrato and FM, onset at 0.25 s, scheduled at frame 0: the FM modulator
        // renders from quantum 0 (the worklet renders its params every quantum it lives), the
        // vibrato is connected but silent until its start.
        let mut v = voice_at(
            json!({ "s": "pulse", "vib": 5, "vibmod": 1, "fmi": 2 }),
            0.1,
            0.25,
            0,
            &mut NoiseCache::new(0),
        );
        let mut o = VoiceOutput::new();
        assert!(v.process(0, &mut o, &mut BusBank::default()));
        let w = worklet_of(&v);
        let fm = w.fm.as_ref().unwrap();
        assert_eq!(fm.modulators()[0].source_quantum, Some(0));
        assert!(o.out[0].iter().all(|x| *x == 0.0));
        assert!(!o.out_silent, "a live worklet is never silent");
        // The same voice scheduled at its onset quantum renders its modulator from there.
        let q0 = crate::render::frame_for_buffer_start(0.25) / 128 * 128;
        let mut late = voice_at(
            json!({ "s": "pulse", "vib": 5, "vibmod": 1, "fmi": 2 }),
            0.1,
            0.25,
            q0,
            &mut NoiseCache::new(0),
        );
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for q in (0..q0 + 8 * 128).step_by(128) {
            let mut o = VoiceOutput::new();
            v.process(q, &mut o, &mut BusBank::default());
            a.extend_from_slice(&o.out[0]);
            if q >= q0 {
                late.process(q, &mut o, &mut BusBank::default());
                b.extend_from_slice(&o.out[0]);
            }
        }
        assert_ne!(
            &a[q0 as usize..],
            &b[..],
            "the FM phase at the onset depends on the pause frame"
        );
    }

    #[test]
    fn worklet_synth_teardown_rows_and_lifetime() {
        let v = voice(json!({ "s": "supersaw" }), 0.1);
        let end = worklet_of(&v).end;
        assert_eq!(
            v.teardown.source(),
            SourceEnd::Supersaw(end).disabled_from()
        );
        let p = voice(json!({ "s": "pulse" }), 0.1);
        let end = worklet_of(&p).end;
        assert_eq!(p.teardown.source(), SourceEnd::Pulse(end).disabled_from());
        // A pulse renders stereo L = R until it is freed after its end.
        let mut p = voice(json!({ "s": "pulse", "note": 57 }), 0.1);
        let (l, r, q) = run_stereo(&mut p, 44100);
        assert_eq!(l, r);
        assert!(l.iter().any(|x| *x != 0.0));
        let stop = p.source_frames().1;
        assert!(q > stop && q <= stop + 2 * 128, "freed at {q}, stop {stop}");
        // A supersaw waiting for its onset is not freed.
        let mut s = voice_at(
            json!({ "s": "supersaw" }),
            0.1,
            0.5,
            0,
            &mut NoiseCache::new(0),
        );
        let mut o = VoiceOutput::new();
        assert!(s.process(0, &mut o, &mut BusBank::default()));
        assert!(s.process(128, &mut o, &mut BusBank::default()));
    }

    #[test]
    fn a_stereo_source_takes_the_panners_stereo_law_and_the_transient_runs_on_two_channels() {
        // pan(0.5) behind a pulse is (almost exactly) the identity: no -3 dB at the centre.
        let mut plain = voice(json!({ "s": "pulse", "note": 57 }), 0.1);
        let mut panned = voice(json!({ "s": "pulse", "note": 57, "pan": 0.5 }), 0.1);
        let (l0, _, _) = run_stereo(&mut plain, 4410);
        let (l1, r1, _) = run_stereo(&mut panned, 4410);
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            (peak(&l1) / peak(&l0) - 1.0).abs() < 1e-6,
            "{} {}",
            peak(&l1),
            peak(&l0)
        );
        assert!((peak(&r1) / peak(&l0) - 1.0).abs() < 1e-6);
        // A mono triangle behind the same pan loses 3 dB per side.
        let mut tri = voice(json!({ "s": "triangle", "note": 57 }), 0.1);
        let mut tri_pan = voice(json!({ "s": "triangle", "note": 57, "pan": 0.5 }), 0.1);
        let (t0, _, _) = run_stereo(&mut tri, 4410);
        let (t1, _, _) = run_stereo(&mut tri_pan, 4410);
        assert!((peak(&t1) / peak(&t0) - 0.5f32.sqrt()).abs() < 1e-4);
        // The transient on a pulse processes two channels; avgGain carries from channel 0 into
        // channel 1, so with L = R input the channels differ.
        let mut tr = voice(json!({ "s": "pulse", "note": 57, "transient": 0.5 }), 0.2);
        assert_eq!(tr.output_channels(), 2);
        assert!(matches!(tr.stages()[0], Stage::Gain(_)));
        assert!(matches!(tr.stages()[1], Stage::Transient(_)));
        let (l, r, _) = run_stereo(&mut tr, 8820);
        assert!(l.iter().any(|x| *x != 0.0));
        assert_ne!(l, r, "the shared avgGain makes channel 1 differ");
    }

    #[test]
    fn the_supersaw_voice_cap_is_reported_once() {
        let mut v = voice(json!({ "s": "supersaw" }), 0.1);
        let Source::Worklet(w) = &mut v.source else {
            panic!()
        };
        let WorkletSynth::Supersaw(n) = &mut w.synth else {
            panic!()
        };
        n.params[supersaw::VOICES].set_value(5000.0, 0).unwrap();
        let (out, _) = run(&mut v, 4410);
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(
            v.take_errors(),
            vec!["supersaw: voices out of range".to_string()]
        );
        assert!(v.take_errors().is_empty());
    }

    /// A voice built on a bank of `keys`.
    fn build_with_buses(
        v: serde_json::Value,
        dur: f64,
        t: f64,
        orbit: &mut Orbit,
        keys: &[&str],
    ) -> Result<(Voice, Option<VoiceError>), VoiceError> {
        Voice::build(
            &params(v, dur),
            0,
            start(t, dur),
            &mut WaveTables::new(),
            &mut NoiseCache::new(0),
            orbit,
            &BusBank::of_keys(keys.iter().map(|k| k.to_string()).collect()),
        )
    }

    /// A send of constant `l`/`r` (stereo) for one quantum.
    fn stereo_send(l: f32, r: f32) -> SendBuf {
        SendBuf {
            ch: [[l; QUANTUM], [r; QUANTUM]],
            channels: 2,
            silent: false,
        }
    }

    #[test]
    fn the_bus_sound_reads_its_bus_through_a_sustain_one_envelope() {
        use RampKind::*;
        let (mut v, late) = build_with_buses(
            json!({ "s": "bus", "n": 1 }),
            0.1,
            0.0,
            &mut bus(),
            &["0", "1"],
        )
        .unwrap();
        assert!(late.is_none());
        // `envGain` straight from the source: no `Gain(0.3)`; `gainNode(0)` then the linear ADSR
        // with the bus defaults [0.001, 0.05, 1, 0.01].
        assert_eq!(v.stages().len(), 3, "envGain, the gain stage and post");
        let Stage::Gain(env) = &v.stages()[0] else {
            panic!("{:?}", v.stages()[0]);
        };
        let e = events(&env.gain);
        assert_eq!(e[0], (0.0, 0.0, SetValue), "gainNode(0)");
        assert_eq!(e[2], (0.001, 1.0, Linear));
        assert_eq!(e[3], (0.001 + 0.05, 1.0, Linear), "sustain 1");
        assert_eq!(e[4], (0.1, 1.0, SetValue));
        assert_eq!(e[5], (0.1 + 0.01, 0.0, Linear));
        // `end = 0.1 + 0.01 + 0.01 = 0.12` s: ceil(5292.0) = 5292, rounded up to 5376.
        assert_eq!(v.source_frames().1, 5376);
        assert_eq!(v.output_channels(), 2);
        // `nodes.source = [bus]`: a gain node, so a `note` target (its `frequency`) throws in the
        // modulators.
        assert_eq!(
            v.node_table().get(FxKey::Main, "source"),
            Some(&NodeList::Array(vec![Some(Node::Bus)]))
        );
        // Bus 1 is index 1: a stereo feeder at sustain is read as is (envelope 1, the gain
        // stage's default 0.8, postgain 1).
        let mut buses = BusBank::of_keys(vec!["0".into(), "1".into()]);
        let mut o = VoiceOutput::new();
        let q = 4096; // 0.0929 s: sustain
        for k in (0..q).step_by(QUANTUM) {
            buses.begin_quantum();
            assert!(v.process(k as u64, &mut o, &mut buses));
            assert!(o.post_silent, "no feeder: silent at {k}");
        }
        buses.begin_quantum();
        buses.add_send(1, 0, &stereo_send(0.5, -0.25));
        buses.add_send(0, 1, &stereo_send(9.0, 9.0));
        assert!(v.process(q as u64, &mut o, &mut buses));
        assert_eq!(o.channels, 2);
        assert!(!o.post_silent);
        assert!(o.post[0].iter().all(|x| *x == 0.5 * 0.8f32));
        assert!(o.post[1].iter().all(|x| *x == -0.25 * 0.8f32));
        assert_eq!(o.out, o.post, "no dry: post reaches the orbit");
        assert!(o.sends.bus.is_none(), "the reader sends nothing");
        // From the stop frame on the bus is disconnected: a feeder no longer reaches the voice,
        // which is freed there.
        let mut alive = true;
        for k in ((q + QUANTUM)..5376).step_by(QUANTUM) {
            buses.begin_quantum();
            alive = v.process(k as u64, &mut o, &mut buses);
        }
        assert!(alive, "alive before the stop frame");
        buses.begin_quantum();
        buses.add_send(1, 0, &stereo_send(0.5, 0.5));
        assert!(
            !v.process(5376, &mut o, &mut buses),
            "freed at the stop frame"
        );
        assert!(o.post_silent && o.post.iter().flatten().all(|x| *x == 0.0));
    }

    #[test]
    fn the_bus_sound_envelope_follows_its_adsr_controls() {
        use RampKind::*;
        // `n ?? 0`: no `n` reads bus 0; explicit controls replace the defaults.
        let (v, _) = build_with_buses(
            json!({ "s": "bus", "attack": 0.02, "sustain": 0.5, "release": 0.1 }),
            0.25,
            0.0,
            &mut bus(),
            &["0"],
        )
        .unwrap();
        let Stage::Gain(env) = &v.stages()[0] else {
            panic!("{:?}", v.stages()[0]);
        };
        let e = events(&env.gain);
        // getADSRValues: with an attack given, a missing decay is 0.001.
        assert_eq!(e[2], (0.02, 1.0, Linear));
        assert_eq!(e[3], (0.02 + 0.001, 0.5, Linear));
        // end = 0.25 + 0.1 + 0.01: ceil(15876.000000000002) = 15877 -> 15872 + 128.
        assert_eq!(v.source_frames().1, 16000);
        // A bus the bank does not hold is an internal error (the driver's bank comes from the
        // job's graph, which holds every key).
        let e = build_with_buses(json!({ "s": "bus", "n": 2 }), 0.1, 0.0, &mut bus(), &["0"])
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Hap);
        assert!(e.detail.contains("bus 2 is not in the job"), "{}", e.detail);
        // A non-finite hap duration throws (the envelope's events, then the timeout's `stop`).
        let e =
            build_with_buses(json!({ "s": "bus" }), f64::NAN, 0.0, &mut bus(), &["0"]).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Hap);
        assert!(e.detail.contains("non-finite"), "{}", e.detail);
        let ok = build_with_buses(
            json!({ "s": "bus", "n": null }),
            0.1,
            0.0,
            &mut bus(),
            &["0"],
        );
        assert!(ok.is_ok(), "null n reads bus 0");
    }

    #[test]
    fn the_bus_send_is_post_times_busgain() {
        let mut orbit = bus();
        let hap = |extra: serde_json::Value| {
            let mut v = json!({ "s": "one", "attack": 0, "sustain": 1, "dry": 0 });
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        let (mut v, _) = build_with_buses(
            hap(json!({ "bus": 1, "busgain": 0.7 })),
            0.1,
            0.0,
            &mut orbit,
            &["1"],
        )
        .unwrap();
        let (_, g) = v.sends().bus.unwrap();
        // `gainNode(busgain)`: `.value = 0.7`, so a setValueAtTime at the pause frame.
        assert_eq!(g.gain.value(), 0.7f32);
        assert_eq!(g.gain.events().len(), 1);
        let mut buses = BusBank::of_keys(vec!["1".into()]);
        let mut o = VoiceOutput::new();
        for q in [0u64, 128, 1024] {
            v.process(q, &mut o, &mut buses);
            let (index, send) = o.sends.bus.as_ref().unwrap();
            assert_eq!(*index, 0);
            assert_eq!(send.channels, 1);
            assert!(!send.silent);
            let want: Vec<f32> = o.post[0].iter().map(|x| x * 0.7f32).collect();
            assert_eq!(send.ch[0].to_vec(), want, "quantum {q}");
            // `dry(0)` silences only the orbit path.
            assert!(o.out_silent && !o.post_silent);
        }
        // No `bus` (or `null`): no send. `0`, `''` and `false` send.
        for b in [json!(null), json!(0), json!(""), json!(false)] {
            let keys = ["0", "", "false"];
            let (mut v, _) =
                build_with_buses(hap(json!({ "bus": b })), 0.1, 0.0, &mut bus(), &keys).unwrap();
            v.process(0, &mut o, &mut BusBank::of_keys(vec![]));
            assert_eq!(o.sends.bus.is_some(), !b.is_null(), "{b}");
        }
        // The default busgain is 1; a `null` one is 0 (silent after the first quantum's event).
        let (mut v, _) =
            build_with_buses(hap(json!({ "bus": 1 })), 0.1, 0.0, &mut bus(), &["1"]).unwrap();
        v.process(0, &mut o, &mut buses);
        assert_eq!(o.sends.bus.as_ref().unwrap().1.ch[0], o.post[0]);
        let (mut v, _) = build_with_buses(
            hap(json!({ "bus": 1, "busgain": null })),
            0.1,
            0.0,
            &mut bus(),
            &["1"],
        )
        .unwrap();
        v.process(0, &mut o, &mut buses);
        assert!(
            o.sends.bus.as_ref().unwrap().1.ch[0]
                .iter()
                .all(|x| *x == 0.0)
        );
        v.process(128, &mut o, &mut buses);
        assert!(o.sends.bus.as_ref().unwrap().1.silent);
        // A stereo voice sends both channels.
        let (mut v, _) = build_with_buses(
            hap(json!({ "bus": 1, "pan": 0.2 })),
            0.1,
            0.0,
            &mut bus(),
            &["1"],
        )
        .unwrap();
        v.process(0, &mut o, &mut buses);
        let send = &o.sends.bus.as_ref().unwrap().1;
        assert_eq!(send.channels, 2);
        assert_eq!((send.ch[0], send.ch[1]), (o.post[0], o.post[1]));
    }

    #[test]
    fn a_non_finite_busgain_is_a_late_throw() {
        // Without an earlier send the hap is lost.
        let e = build_with_buses(
            json!({ "s": "one", "bus": 1, "busgain": { "$num": "NaN" } }),
            0.1,
            0.0,
            &mut bus(),
            &["1"],
        )
        .unwrap_err();
        assert!(e.detail.starts_with("TypeError"), "{}", e.detail);
        let e = build_with_buses(
            json!({ "s": "one", "bus": 1, "busgain": 1e40 }),
            0.1,
            0.0,
            &mut bus(),
            &["1"],
        )
        .unwrap_err();
        assert!(e.detail.starts_with("TypeError"), "{}", e.detail);
        // After the delay send: the voice keeps the delay send, unconnected, and no bus send.
        let mut orbit = bus();
        let (v, late) = build_with_buses(
            json!({ "s": "one", "delay": 0.5, "delaytime": 0.1, "bus": 1,
                    "busgain": { "$num": "Infinity" } }),
            0.1,
            0.0,
            &mut orbit,
            &["1"],
        )
        .unwrap();
        assert!(late.unwrap().detail.starts_with("TypeError"));
        assert!(!v.connected());
        assert_eq!(v.send_levels().0, Some(0.5));
        assert!(v.sends().bus.is_none());
        assert!(orbit.delay().is_some());
        // A throw after the bus send (a non-finite dry) keeps the bus send: the voice plays
        // into its bus only.
        let (mut v, late) = build_with_buses(
            json!({ "s": "one", "attack": 0, "sustain": 1, "bus": 1, "dry": 1e40 }),
            0.1,
            0.0,
            &mut bus(),
            &["1"],
        )
        .unwrap();
        assert!(late.is_some());
        assert!(!v.connected() && v.sends().bus.is_some());
        let mut o = VoiceOutput::new();
        v.process(0, &mut o, &mut BusBank::of_keys(vec!["1".into()]));
        assert!(o.out_silent);
        assert!(!o.sends.bus.as_ref().unwrap().1.silent);
    }
}

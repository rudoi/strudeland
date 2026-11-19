// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/audio_node_input.cc, modules/webaudio/audio_summing_junction.cc, platform/audio/audio_bus.cc, platform/audio/audio_channel.cc, modules/webaudio/channel_splitter_handler.cc, modules/webaudio/channel_merger_handler.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The orbit bus (superdough's `Orbit` and `SuperdoughOutput`, superdoughoutput.mjs) as Chromium
//! renders it:
//!
//! ```text
//! voice out (post, or Gain(dry)) ─► summingNode(Gain 1, 2 ch explicit) ─► output(Gain 1, 2 ch)
//!   ─► StereoPannerNode(pan 0) ─► ChannelSplitter(2) ─► ChannelMerger input channels[i] % count
//!   ─► destinationGain(1) ─► destination
//! ```
//!
//! Every input of the summing node is up-mixed to stereo (`SumFromByUpMixing`: mono `x` gives
//! `L = R = x`) and added into a bus zeroed each quantum (`AudioNodeInput::SumAllConnections`); a
//! silent connection is skipped and the first non-silent one is copied, later ones added
//! (`AudioChannel::SumFrom`). The gain-1 nodes copy (`CopyWithGainFrom` with gain exactly 1), so
//! they are the identity here, passing the silence hint on. The pan-0 panner has no events (the
//! k-rate path with float gains): `L = fma(R, cos(pi/2), L)` (the ~6.1e-17 leak), `R = R * 1`.
//! The splitter's two outputs feed the shared merger inputs `channels[i] % count` for each
//! `i`, a repeated (output, input) pair connecting once (`AudioNodeOutput` keeps a set). Each
//! merger input is a mono summing junction; its channel goes to the destination unchanged.
//!
//! Summing order: Blink keeps a junction's connections in a hash set
//! (`AudioSummingJunction::outputs_`), so the order connections are added in is not defined. The
//! engine adds them in connection order (voices in construction order into an orbit, orbits in
//! creation order into a merger input); sums of three or more non-zero signals can differ from
//! Chromium's in the last bit.
//!
//! The orbit's feedback delay (created by the first hap that sends to it, `getDelay`) is
//! processed first each quantum: the sends of its voices (in voice order, after the feedback),
//! then its return, which is added to the summing node before any voice.
//!
//! The orbit's reverb (`Reverb`, created by the first hap with `room > 0`, `getReverb`) keeps
//! its stored settings and IR requests; each pause is settled after its haps are scheduled
//! ([`OrbitBank::settle`]), generating the newest request's IR and building a new convolver for
//! it. Each quantum its room sends (in voice order, each scaled by its level) are summed into the
//! convolver's input (mono when every send is mono), and its return joins the summing node.
//!
//! The delay and reverb returns were connected to the summing node when they were created, before
//! the voices that send to them: they are added first, in creation order.
//!
//! The first hap with `djf != null` inserts the `djf` worklet between the summing node and the
//! output (`Orbit.getDjf`): from that hap's pause frame on the whole orbit, voices and returns,
//! goes through it ([`Orbit::get_djf`]), and it never leaves. It is always processing and never
//! silent; its input is assumed to stay connected (the summing node's outputs are never disabled
//! while it has an input, which the micro cases keep true).
//!
//! The orbit's `output` is a real `GainNode` (gain 1, a-rate, two channels): without events it
//! multiplies by exactly 1, the identity. Duck schedules on its gain: the engine ports
//! `offlineDuck` (`src/lib/audio/offline.ts`), which the references render (the harness installs
//! it on `Orbit.prototype`; superdough's own `Orbit.duck` is the live path): per orbit a JS-side
//! curve model ([`DuckCurve`], `gainAt`) gives the `held` value that clamps the ducked gain, and
//! the param gets `cancelAndHoldAtTime(t)` and two exponential ramps ([`Orbit::duck`]). The
//! controller (`SuperdoughAudioController.duck`) spreads one hap's `duck*` controls over its
//! target orbits ([`OrbitBank::duck`]); a target orbit the job lacks is a silent no-op. If
//! `offlineDuck` changes, the engine, the goldens and the refs move together.

use crate::QUANTUM;
use crate::controls::{js_max, js_min, js_to_string};
use crate::delay::{FeedbackDelay, get_delay, sum_into};
use crate::djf::Djf;
use crate::job::{EngineJob, OrbitSpec, Value, ValueMap, orbit_number};
use crate::js::number_to_string;
use crate::jsmath;
use crate::modulate::OrbitParam;
use crate::panner::{Stereo, StereoPanner};
use crate::param::{Param, ParamError, ParamInputs, idl_float};
use crate::reverb::{Convolver, IrCache, IrError, IrPlan, Reverb};
use crate::voice::{GainNode, VoiceOutput};

/// One automation event `offlineDuck` scheduled (JS `GainPoint`): a held value, or the end of an
/// exponential ramp from the point before it. All `f64`, as in JS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainPoint {
    pub time: f64,
    pub value: f64,
    pub ramp: bool,
}

/// What `offlineDuck` has scheduled on an orbit's output gain (the JS `duckCurves` entry): the
/// value the gain had before (`g.value`, 1) and the points.
#[derive(Debug, Clone, PartialEq)]
pub struct DuckCurve {
    pub initial: f64,
    pub points: Vec<GainPoint>,
}

/// `gainAt(initial, points, x)` (offline.ts) verbatim: the JS model of the curve at `x`. An
/// exponential ramp runs from the previous point to its own; a ramp whose end is not after the
/// previous point (or a ramp first) is not interpolated. `Math.pow` is `jsmath::pow`.
pub fn gain_at(initial: f64, points: &[GainPoint], x: f64) -> f64 {
    let mut prev: Option<&GainPoint> = None;
    for p in points {
        if p.time <= x {
            prev = Some(p);
            continue;
        }
        let Some(q) = prev.filter(|_| p.ramp) else {
            break;
        };
        let span = p.time - q.time;
        return q.value * jsmath::pow(p.value / q.value, (x - q.time) / span);
    }
    prev.map_or(initial, |p| p.value)
}

/// One hap's duck controls (`duckorbit`, `duckonset`, `duckattack`, `duckdepth`) as read from its
/// raw value map: `None` is an absent key (JS `undefined`).
#[derive(Debug, Clone, Copy)]
pub struct DuckControls<'a> {
    pub orbit: &'a Value,
    pub onset: Option<&'a Value>,
    pub attack: Option<&'a Value>,
    pub depth: Option<&'a Value>,
}

impl<'a> DuckControls<'a> {
    /// The duck controls of `value`, when it ducks (`duckorbit != null`).
    pub fn of(value: &'a ValueMap) -> Option<DuckControls<'a>> {
        let orbit = value
            .get("duckorbit")
            .filter(|v| !matches!(v, Value::Null))?;
        Some(DuckControls {
            orbit,
            onset: value.get("duckonset"),
            attack: value.get("duckattack"),
            depth: value.get("duckdepth"),
        })
    }
}

/// An element of a controller array: absent (`undefined`) or a value.
type Elem<'a> = Option<&'a Value>;

/// `[x].flat()` of a control: an array gives its elements (one level), any other value itself;
/// an absent control is the controller's default parameter `default`.
fn flat1<'a>(v: Option<&'a Value>, default: &'a Value) -> Vec<&'a Value> {
    match v.unwrap_or(default) {
        Value::Arr(xs) => xs.iter().collect(),
        other => vec![other],
    }
}

/// `arr[idx] ?? arr[0]`: `undefined` when both are nullish and `arr[0]` is absent, else the
/// first non-nullish of the two (or `arr[0]` itself, which may be `null`).
fn pick<'a>(arr: &[&'a Value], idx: usize) -> Elem<'a> {
    match arr.get(idx) {
        Some(v) if !matches!(v, Value::Null) => Some(v),
        _ => arr.first().copied(),
    }
}

/// The JS number a duck control element becomes where it is used: `Num` itself, `Bool` 0/1,
/// `undefined` and `null` as given; any other kind is NaN (a documented deviation: superdough
/// would coerce a string).
fn elem_number(v: Elem<'_>, undefined: f64, null: f64) -> f64 {
    match v {
        None => undefined,
        Some(Value::Null) => null,
        Some(Value::Num(x)) => *x,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(_) => f64::NAN,
    }
}

/// One connection into a summing junction: one (mono) or two channels and its silence hint.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub channels: &'a [[f32; QUANTUM]],
    pub silent: bool,
}

/// `AudioChannel::SumFrom`: a silent source is skipped; into a silent destination it is copied
/// (keeping `-0`), else added (`vDSP_vadd`, a plain float add per frame).
pub fn sum_channel(
    dst: &mut [f32; QUANTUM],
    dst_silent: &mut bool,
    src: &[f32; QUANTUM],
    src_silent: bool,
) {
    if src_silent {
        return;
    }
    if *dst_silent {
        *dst = *src;
        *dst_silent = false;
    } else {
        for (d, s) in dst.iter_mut().zip(src.iter()) {
            *d += *s;
        }
    }
}

/// `AudioBus::SumFrom` into a stereo bus with the speakers interpretation: mono up-mixes to both
/// channels, stereo sums channel-wise. `silent` holds one hint per destination channel.
pub fn sum_stereo(dst: &mut Stereo, silent: &mut [bool; 2], input: Input<'_>) {
    let (l, r) = dst.split_at_mut(1);
    let (sl, sr) = silent.split_at_mut(1);
    match input.channels {
        [mono] => {
            sum_channel(&mut l[0], &mut sl[0], mono, input.silent);
            sum_channel(&mut r[0], &mut sr[0], mono, input.silent);
        }
        [a, b, ..] => {
            sum_channel(&mut l[0], &mut sl[0], a, input.silent);
            sum_channel(&mut r[0], &mut sr[0], b, input.silent);
        }
        [] => {}
    }
}

/// The merger connections of an orbit with output `channels` on a destination of `count`
/// channels: `(splitter output i % 2, merger input channels[i] % count)` in `channels` order,
/// each pair once (connecting an output to the same input twice is a no-op).
pub fn route(channels: &[i64], count: usize) -> Vec<(usize, usize)> {
    let mut routes: Vec<(usize, usize)> = Vec::new();
    if count == 0 {
        return routes;
    }
    for (i, &ch) in channels.iter().enumerate() {
        let r = (i % 2, ch.rem_euclid(count as i64) as usize);
        if !routes.contains(&r) {
            routes.push(r);
        }
    }
    routes
}

/// The stem pair an orbit lands on (the twin of `orbitPair` in `src/lib/render/gates/stems.ts`):
/// `orbit <= 0` -> 0, else `(orbit - 1) % pairs` (JS `%`: the sign of the dividend). `pairs` may
/// be infinite (no wrap).
pub fn orbit_pair(orbit: f64, pairs: f64) -> f64 {
    if orbit <= 0.0 {
        0.0
    } else {
        (orbit - 1.0) % pairs
    }
}

/// The modulator connections into an orbit's own params this quantum: each live
/// voice's orbit-held inputs, in voice order, then connection order.
#[derive(Debug, Clone, Default)]
pub struct OrbitInputs {
    pub delay_time: ParamInputs,
    pub feedback: ParamInputs,
    pub djf: ParamInputs,
}

impl OrbitInputs {
    /// Sets `inputs` to the inputs of `count` orbits from the voices: voice `outputs[i]` feeds
    /// orbit `orbit_of[i]` (one pass in voice order; the buffers are reused).
    pub fn fill_all(
        inputs: &mut Vec<OrbitInputs>,
        count: usize,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
    ) {
        inputs.resize_with(count, OrbitInputs::default);
        for i in inputs.iter_mut() {
            for p in [&mut i.delay_time, &mut i.feedback, &mut i.djf] {
                p.connected = false;
                p.bufs.clear();
            }
        }
        for (o, &k) in outputs.iter().zip(orbit_of.iter()) {
            let Some(i) = inputs.get_mut(k) else {
                continue;
            };
            for (param, buf) in &o.orbit_inputs {
                let p = match param {
                    OrbitParam::DelayTime => &mut i.delay_time,
                    OrbitParam::Feedback => &mut i.feedback,
                    OrbitParam::DjfValue => &mut i.djf,
                };
                p.connected = true;
                p.bufs.extend(buf.iter().copied());
            }
        }
    }
}

/// One superdough `Orbit`: its summing node, output gain and pan-0 panner, routed to the merger.
#[derive(Debug, Clone)]
pub struct Orbit {
    orbit: f64,
    channels: Vec<i64>,
    routes: Vec<(usize, usize)>,
    /// `summingNode`'s summing bus this quantum.
    sum: Stereo,
    sum_silent: [bool; 2],
    /// `connectToDestination`'s `StereoPannerNode` (pan never set).
    panner: StereoPanner,
    /// The panner's output this quantum (the splitter copies its channels).
    out: Stereo,
    out_silent: bool,
    /// The orbit's feedback delay, once a hap has sent to it.
    delay: Option<FeedbackDelay>,
    /// The orbit's reverb, once a hap with `room > 0` created it.
    reverb: Option<Reverb>,
    /// The effect returns in the order they connected to the summing node.
    returns: Vec<Return>,
    /// The `djf` worklet between the summing node and the output, once a hap inserted it.
    djf: Option<Djf>,
    /// `output`: gain 1, the duck target.
    output: GainNode,
    /// `offlineDuck`'s model of `output.gain`, from the orbit's first duck call on.
    duck_curve: Option<DuckCurve>,
}

/// An effect return into an orbit's summing node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Return {
    Delay,
    Reverb,
}

impl Orbit {
    /// `new Orbit(ac)` and `connectToDestination(orbit.output, channels)` on a destination of
    /// `count` channels.
    pub fn new(spec: &OrbitSpec, count: usize) -> Orbit {
        Orbit {
            orbit: spec.orbit,
            channels: spec.channels.clone(),
            routes: route(&spec.channels, count),
            sum: [[0.0; QUANTUM]; 2],
            sum_silent: [true; 2],
            panner: StereoPanner::new(),
            out: [[0.0; QUANTUM]; 2],
            out_silent: true,
            delay: None,
            reverb: None,
            returns: Vec::new(),
            djf: None,
            output: GainNode::new(Param::gain(1.0)),
            duck_curve: None,
        }
    }

    /// `offlineDuck(t, onset, attack, depth)` called at frame `now` (the controller's defaults
    /// already applied): `held` from the curve model, `ducked = min(max(1 - sqrt(depth), 0.01),
    /// held)`, then on the output gain `cancelAndHoldAtTime(t)`, a ramp to `ducked` at
    /// `t + onset` and one to 1 at `t + onset + max(attack, 0.002)`. The curve is created by the
    /// first call even when it throws; its points change only when every step succeeded (a
    /// throw leaves the events the steps before it scheduled).
    pub fn duck(
        &mut self,
        t: f64,
        onset: f64,
        attack: f64,
        depth: f64,
        now: u64,
    ) -> Result<(), ParamError> {
        let gain = &mut self.output.gain;
        let curve = self.duck_curve.get_or_insert_with(|| DuckCurve {
            initial: f64::from(gain.value()),
            points: Vec::new(),
        });
        let held = gain_at(curve.initial, &curve.points, t);
        let a = js_max(attack, 0.002);
        let ducked = js_min(js_max(1.0 - depth.sqrt(), 0.01), held);
        gain.cancel_and_hold_at_time(t, now)?;
        const EXP: &str = "exponentialRampToValueAtTime";
        gain.exponential_ramp_to(idl_float(ducked, EXP)?, t + onset, now)?;
        gain.exponential_ramp_to(1.0, t + onset + a, now)?;
        curve.points.retain(|p| p.time < t);
        curve.points.extend([
            GainPoint {
                time: t,
                value: held,
                ramp: false,
            },
            GainPoint {
                time: t + onset,
                value: ducked,
                ramp: true,
            },
            GainPoint {
                time: t + onset + a,
                value: 1.0,
                ramp: true,
            },
        ]);
        Ok(())
    }

    /// The duck curve model, once the orbit was ducked.
    pub fn duck_curve(&self) -> Option<&DuckCurve> {
        self.duck_curve.as_ref()
    }

    /// The output gain (the duck target).
    pub fn output_gain(&self) -> &Param {
        &self.output.gain
    }

    /// `orbitBus.getDjf(value, t)` for a hap at `t` scheduled at frame `now`: the first call
    /// creates the worklet with `.value = value` (an event at `now`; a throw leaves no slot and
    /// the orbit unchanged), then every call schedules `value.setValueAtTime(value, t)`.
    pub fn get_djf(&mut self, value: f64, t: f64, now: u64) -> Result<(), ParamError> {
        if self.djf.is_none() {
            let mut d = Djf::new();
            d.value.set_value(idl_float(value, "value")?, now)?;
            self.djf = Some(d);
        }
        let d = self.djf.as_mut().expect("the slot exists");
        d.value
            .set_value_at_time(idl_float(value, "setValueAtTime")?, t, now)
    }

    /// The orbit's `djf` worklet, once inserted.
    pub fn djf(&self) -> Option<&Djf> {
        self.djf.as_ref()
    }

    /// `getDelay(delaytime, feedback, t)` for a hap at `t` scheduled at frame `now` (see
    /// `delay::get_delay`).
    pub fn get_delay(
        &mut self,
        delaytime: f64,
        delaytime_is_number: bool,
        feedback: f64,
        t: f64,
        now: u64,
    ) -> Result<(), ParamError> {
        let created = self.delay.is_none();
        let r = get_delay(
            &mut self.delay,
            delaytime,
            delaytime_is_number,
            feedback,
            t,
            now,
        );
        if created && self.delay.is_some() {
            self.returns.push(Return::Delay);
        }
        r
    }

    /// The orbit's feedback delay, once created.
    pub fn delay(&self) -> Option<&FeedbackDelay> {
        self.delay.as_ref()
    }

    /// `getReverb(roomsize, roomfade, roomlp, roomdim)` with a hap's raw settings (`None` is
    /// `undefined`): creates the reverb (nothing is created when its first request fails) or
    /// requests a new IR when a defined setting changed. An error is the hap's: superdough
    /// throws out of `getReverb`.
    pub fn get_reverb(
        &mut self,
        roomsize: Option<&crate::job::Value>,
        roomfade: Option<&crate::job::Value>,
        roomlp: Option<&crate::job::Value>,
        roomdim: Option<&crate::job::Value>,
    ) -> Result<(), IrError> {
        match &mut self.reverb {
            Some(r) => r.update(roomsize, roomfade, roomlp, roomdim).map(|_| ()),
            None => {
                // Right after `createReverb` the settings are compared with themselves (the
                // same values): no second request.
                let sr = f64::from(crate::SAMPLE_RATE);
                self.reverb = Some(Reverb::create(roomsize, roomfade, roomlp, roomdim, sr)?);
                self.returns.push(Return::Reverb);
                Ok(())
            }
        }
    }

    /// The orbit's reverb, once created.
    pub fn reverb(&self) -> Option<&Reverb> {
        self.reverb.as_ref()
    }

    /// Settles the reverb's IR requests of the pause (audio seed `seed`) through the run's IR
    /// cache; returns whether its IR changed.
    pub fn settle_reverb(&mut self, seed: i32, cache: &mut IrCache) -> bool {
        self.reverb
            .as_mut()
            .is_some_and(|r| r.settle_cached(seed, cache))
    }

    /// Takes the reverb's pending IR request (number and plan), for an IR built elsewhere.
    pub fn take_reverb_request(&mut self) -> Option<(u64, IrPlan)> {
        self.reverb.as_mut().and_then(Reverb::take_pending)
    }

    /// Lands a convolver built for the reverb's request `request` (newest request wins).
    pub fn apply_reverb(&mut self, request: u64, convolver: Convolver) -> bool {
        self.reverb
            .as_mut()
            .is_some_and(|r| r.apply(request, convolver))
    }

    /// The effect returns in summing order (creation order).
    pub fn returns(&self) -> &[Return] {
        &self.returns
    }

    /// Renders the orbit's effects for the quantum at `quantum_start` (fed by the voices `i`
    /// with `orbit_of[i] == index`) and adds their returns to the summing node in creation
    /// order. Call it after [`Orbit::begin`] and before the voices' [`Orbit::add`].
    pub fn process_returns(
        &mut self,
        quantum_start: u64,
        index: usize,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
        inputs: &OrbitInputs,
    ) {
        for k in 0..self.returns.len() {
            match self.returns[k] {
                Return::Delay => {
                    self.process_delay(quantum_start, index, outputs, orbit_of, inputs)
                }
                Return::Reverb => self.process_reverb(quantum_start, index, outputs, orbit_of),
            }
        }
    }

    /// Renders the orbit's reverb for the quantum at `quantum_start`: the room send of every
    /// voice `i` with `orbit_of[i] == index` (`post` already scaled by its `gainNode(room)`, in
    /// voice order) is summed into the convolver's input (as many channels as the widest send,
    /// mono up-mixed); the return is added to the summing node.
    pub fn process_reverb(
        &mut self,
        quantum_start: u64,
        index: usize,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
    ) {
        let Some(reverb) = &mut self.reverb else {
            return;
        };
        let senders = || {
            outputs
                .iter()
                .zip(orbit_of.iter())
                .filter(move |(_, k)| **k == index)
                .filter_map(|(o, _)| o.sends.room.as_ref())
        };
        let channels = senders().map(|s| s.channels.clamp(1, 2)).max().unwrap_or(1);
        let mut bus: Stereo = [[0.0; QUANTUM]; 2];
        let mut silent = [true; 2];
        for send in senders() {
            sum_into(&mut bus, &mut silent, channels, send.input());
        }
        let input = Input {
            channels: &bus[..channels],
            silent: silent[..channels].iter().all(|s| *s),
        };
        let ret = reverb.process(quantum_start, input);
        sum_stereo(&mut self.sum, &mut self.sum_silent, ret);
    }

    /// Renders the orbit's delay for the quantum at `quantum_start`, fed by the delay send of
    /// every voice `i` with `orbit_of[i] == index` (in voice order), and adds its return to the
    /// summing node. Call it after [`Orbit::begin`] and before the voices' [`Orbit::add`].
    pub fn process_delay(
        &mut self,
        quantum_start: u64,
        index: usize,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
        inputs: &OrbitInputs,
    ) {
        let Some(delay) = &mut self.delay else {
            return;
        };
        let senders = || {
            outputs
                .iter()
                .zip(orbit_of.iter())
                .filter(move |(_, k)| **k == index)
                .filter_map(|(o, _)| o.sends.delay.as_ref())
        };
        let channels = senders().map(|s| s.channels.clamp(1, 2)).max().unwrap_or(1);
        delay.begin_with(quantum_start, channels, &inputs.feedback);
        for send in senders() {
            delay.add_send(send.input());
        }
        let ret = delay.finish_with(quantum_start, &inputs.delay_time);
        sum_stereo(&mut self.sum, &mut self.sum_silent, ret);
    }

    /// The numeric orbit.
    pub fn orbit(&self) -> f64 {
        self.orbit
    }

    /// The 0-based output channels of the orbit's first hap.
    pub fn channels(&self) -> &[i64] {
        &self.channels
    }

    /// The `(splitter output, merger input)` connections.
    pub fn routes(&self) -> &[(usize, usize)] {
        &self.routes
    }

    /// Starts a quantum: the summing bus is zeroed (silent).
    pub fn begin(&mut self) {
        self.sum = [[0.0; QUANTUM]; 2];
        self.sum_silent = [true; 2];
    }

    /// Adds one connection into the summing node (in connection order).
    pub fn add(&mut self, input: Input<'_>) {
        sum_stereo(&mut self.sum, &mut self.sum_silent, input);
    }

    /// Ends the quantum: `summingNode`, the `djf` worklet when inserted, the `output` gain (1
    /// without duck events: an exact copy, or silence for a silent bus), then the pan-0 panner.
    /// Returns the orbit's output and its silence hint.
    #[cfg(test)]
    pub fn finish(&mut self, quantum_start: u64) -> (&Stereo, bool) {
        self.finish_with(quantum_start, &ParamInputs::NONE)
    }

    /// [`Orbit::finish`] with the modulator connections into the `djf` worklet's `value`.
    pub fn finish_with(&mut self, quantum_start: u64, djf_inputs: &ParamInputs) -> (&Stereo, bool) {
        let mut silent = self.sum_silent[0] && self.sum_silent[1];
        // The summing node's bus: zeros when silent (it was zeroed by `begin`).
        let mut work = self.sum;
        if let Some(djf) = &mut self.djf {
            silent = djf.process_with(quantum_start, &mut work, djf_inputs);
        }
        silent = self
            .output
            .process(quantum_start, &mut work, silent, &ParamInputs::NONE);
        self.out_silent = self
            .panner
            .process(quantum_start, &work, silent, &mut self.out);
        (&self.out, self.out_silent)
    }

    /// The panner's output of the last finished quantum and its silence hint.
    pub fn output(&self) -> (&Stereo, bool) {
        (&self.out, self.out_silent)
    }
}

/// Every orbit of a job (created up front, in `job.orbits` order) and the shared channel merger.
#[derive(Debug, Clone)]
pub struct OrbitBank {
    orbits: Vec<Orbit>,
    count: usize,
    /// Per destination channel, whether it is still silent this quantum (reused each quantum).
    silent: Vec<bool>,
    /// Per orbit, its modulator inputs this quantum (reused each quantum).
    inputs: Vec<OrbitInputs>,
}

impl OrbitBank {
    /// Creates the orbits of `job` in creation order on a destination of `job.channels.count`
    /// channels.
    pub fn new(job: &EngineJob) -> OrbitBank {
        let count = job.channels.count as usize;
        OrbitBank {
            orbits: job.orbits.iter().map(|o| Orbit::new(o, count)).collect(),
            count,
            silent: Vec::new(),
            inputs: Vec::new(),
        }
    }

    /// A bank with no orbits on a destination of `count` channels; orbits are added with
    /// [`Self::create`].
    pub fn empty(count: usize) -> OrbitBank {
        OrbitBank {
            orbits: Vec::new(),
            count,
            silent: Vec::new(),
            inputs: Vec::new(),
        }
    }

    /// Creates the orbit of `spec` on first use: a no-op when an orbit numbered `spec.orbit`
    /// exists, else it is appended (creation order is call order, as `OrbitBank::new` keeps
    /// `job.orbits` order).
    pub fn create(&mut self, spec: &OrbitSpec) {
        if self.orbits.iter().any(|o| o.orbit == spec.orbit) {
            return;
        }
        self.orbits.push(Orbit::new(spec, self.count));
    }

    pub fn orbits(&self) -> &[Orbit] {
        &self.orbits
    }

    /// The orbits, for scheduling (a voice's sends create or update its orbit's delay).
    pub fn orbits_mut(&mut self) -> &mut [Orbit] {
        &mut self.orbits
    }

    /// The end of a pause (`settleIR`, before the context resumes): every orbit applies the IR
    /// of its newest request (audio seed `seed`), through the run's IR cache.
    pub fn settle(&mut self, seed: i32, cache: &mut IrCache) {
        for o in &mut self.orbits {
            o.settle_reverb(seed, cache);
        }
    }

    /// The destination channel count.
    pub fn count(&self) -> usize {
        self.count
    }

    /// The orbit a hap's control map plays on: the first orbit whose number is
    /// `orbit_number(value)`. (Superdough keys orbits by `String(orbit)`; the job keeps only the
    /// number, so two keys with one number, such as `1` and `'x'`, share the first orbit's bus.)
    /// A hap whose orbit the job lacks cannot come from `planEngineJob`; the error is the
    /// detail of the hap error the driver reports.
    pub fn find(&self, value: &ValueMap) -> Result<usize, String> {
        let n = orbit_number(value);
        self.orbits
            .iter()
            .position(|o| o.orbit == n)
            .ok_or_else(|| format!("orbit {} not in the job", number_to_string(n)))
    }

    /// `SuperdoughAudioController.duck(duckorbit, t, duckonset, duckattack, duckdepth)` at frame
    /// `now`: for each target of `[duckorbit].flat()` in order, the first orbit whose number's
    /// `String` is the target's JS property key (`String(target)`, arrays and objects included;
    /// none: reported missing, below) is ducked with the target's element of
    /// each array (`arr[idx] ?? arr[0]`; absent controls are `[0]`, `[0.1]`, `[1]`). Per target:
    /// onset `undefined`/`null` is 0; attack is `Math.max(element, 0.002)` (`undefined` NaN,
    /// `null` 0.002); depth `undefined` is 1 (`offlineDuck`'s default), `null` 0. The first error
    /// is returned at once: later targets are not ducked, earlier ones stay.
    ///
    /// A target no orbit matches ducks nothing; its JS text (`${target}`) is pushed to `missing`,
    /// in target order, for the driver to report as superdough's error log `duck target orbit
    /// <target> does not exist` (targets before a throw are pushed, as superdough logs them).
    pub fn duck(
        &mut self,
        controls: &DuckControls<'_>,
        t: f64,
        now: u64,
        missing: &mut Vec<String>,
    ) -> Result<(), ParamError> {
        let (zero, tenth, one) = (Value::Num(0.0), Value::Num(0.1), Value::Num(1.0));
        let targets = flat1(Some(controls.orbit), controls.orbit);
        let onsets = flat1(controls.onset, &zero);
        let attacks = flat1(controls.attack, &tenth);
        let depths = flat1(controls.depth, &one);
        for (idx, target) in targets.into_iter().enumerate() {
            // `this.nodes[target]`: the JS property key of the target (`String(target)`, so
            // `[1]` and `[[1]]` are "1", `[1, 2]` is "1,2", an object "[object Object]").
            let key = js_to_string(target);
            let found = self
                .orbits
                .iter_mut()
                .find(|o| number_to_string(o.orbit) == key);
            let Some(orbit) = found else {
                missing.push(key);
                continue;
            };
            let onset = elem_number(pick(&onsets, idx), 0.0, 0.0);
            let attack = js_max(elem_number(pick(&attacks, idx), f64::NAN, 0.0), 0.002);
            let depth = elem_number(pick(&depths, idx), 1.0, 0.0);
            orbit.duck(t, onset, attack, depth, now)?;
        }
        Ok(())
    }

    /// Renders one quantum of the orbit graph: each voice output `outputs[i]` into the orbit
    /// `orbit_of[i]` in voice order, the orbits' panners, then the merger into `dest` (one
    /// buffer per destination channel; unconnected channels are zeros).
    pub fn process(
        &mut self,
        quantum_start: u64,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
        dest: &mut [[f32; QUANTUM]],
    ) {
        let inputs = &mut self.inputs;
        OrbitInputs::fill_all(inputs, self.orbits.len(), outputs, orbit_of);
        for (k, o) in self.orbits.iter_mut().enumerate() {
            o.begin();
            // The effect returns connected before any voice: they are added first.
            o.process_returns(quantum_start, k, outputs, orbit_of, &inputs[k]);
        }
        for (out, &k) in outputs.iter().zip(orbit_of.iter()) {
            if let Some(o) = self.orbits.get_mut(k) {
                o.add(out.to_orbit());
            }
        }
        let silent = &mut self.silent;
        silent.clear();
        silent.resize(dest.len(), true);
        for d in dest.iter_mut() {
            d.fill(0.0);
        }
        for (o, inputs) in self.orbits.iter_mut().zip(inputs.iter()) {
            o.finish_with(quantum_start, &inputs.djf);
            let (out, out_silent) = (&o.out, o.out_silent);
            for &(split, merge) in &o.routes {
                if let (Some(d), Some(s)) = (dest.get_mut(merge), silent.get_mut(merge)) {
                    sum_channel(d, s, &out[split], out_silent);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(orbit: f64, channels: &[i64]) -> OrbitSpec {
        OrbitSpec {
            orbit,
            channels: channels.to_vec(),
        }
    }

    fn job(count: u32, orbits: serde_json::Value) -> EngineJob {
        EngineJob::from_json(
            &json!({
                "schema": 2, "sampleRate": 44100, "frames": 128, "seed": 0, "cps": 0.5,
                "channels": { "count": count, "multiChannelOrbits": count > 2 },
                "orbits": orbits,
                "pauses": [{ "suspendSec": 0, "haps": [] }], "values": []
            })
            .to_string(),
        )
        .unwrap()
    }

    fn mono_out(v: f32) -> VoiceOutput {
        let mut o = VoiceOutput::new();
        o.out[0] = [v; QUANTUM];
        o.out_silent = false;
        o.channels = 1;
        o
    }

    fn stereo_out(l: f32, r: f32) -> VoiceOutput {
        let mut o = VoiceOutput::new();
        o.out = [[l; QUANTUM], [r; QUANTUM]];
        o.out_silent = false;
        o.channels = 2;
        o
    }

    /// The pan-0 panner's left gain: `cos(pi/2)` as a float (about 6.12e-17).
    fn leak() -> f32 {
        libm::cos(std::f64::consts::FRAC_PI_2) as f32
    }

    fn render(
        bank: &mut OrbitBank,
        outputs: &[VoiceOutput],
        orbit_of: &[usize],
    ) -> Vec<[f32; QUANTUM]> {
        let mut dest = vec![[7.0f32; QUANTUM]; bank.count()];
        bank.process(0, outputs, orbit_of, &mut dest);
        dest
    }

    #[test]
    fn routes_mix_stems_aliasing_and_explicit_channels() {
        // Mix: every orbit on [0, 1].
        assert_eq!(route(&[0, 1], 2), vec![(0, 0), (1, 1)]);
        // Stems, orbit 3 on 4 pairs: channels 4, 5.
        assert_eq!(route(&[4, 5], 8), vec![(0, 4), (1, 5)]);
        // Past 16 pairs: orbit 17 (channels 32, 33) aliases onto pair 0 of 32 channels.
        assert_eq!(route(&[32, 33], 32), vec![(0, 0), (1, 1)]);
        // Orbit 3 with only 2 pairs wraps to pair 0 too.
        assert_eq!(route(&[4, 5], 4), vec![(0, 0), (1, 1)]);
        // Explicit `channels`: one channel takes only the left; three alternate left/right.
        assert_eq!(route(&[2], 4), vec![(0, 2)]);
        assert_eq!(route(&[0, 1, 2], 4), vec![(0, 0), (1, 1), (0, 2)]);
        // A repeated connection is made once; both sides onto one channel stay two connections.
        assert_eq!(route(&[0, 1, 2], 2), vec![(0, 0), (1, 1)]);
        assert_eq!(route(&[0, 1], 1), vec![(0, 0), (1, 0)]);
        assert_eq!(route(&[3, 3], 4), vec![(0, 3), (1, 3)]);
        // Degenerate inputs never panic.
        assert!(route(&[], 2).is_empty());
        assert!(route(&[0, 1], 0).is_empty());
        assert_eq!(route(&[i64::MAX], 6), vec![(0, (i64::MAX % 6) as usize)]);
    }

    #[test]
    fn orbit_pairs_follow_the_ts_rule() {
        assert_eq!(orbit_pair(1.0, 16.0), 0.0);
        assert_eq!(orbit_pair(5.0, 16.0), 4.0);
        assert_eq!(orbit_pair(17.0, 16.0), 0.0);
        assert_eq!(orbit_pair(18.0, 16.0), 1.0);
        assert_eq!(orbit_pair(6.0, 3.0), 2.0);
        assert_eq!(orbit_pair(0.0, 16.0), 0.0);
        assert_eq!(orbit_pair(-3.0, 16.0), 0.0);
        assert_eq!(orbit_pair(1.5, 16.0), 0.5);
        assert_eq!(orbit_pair(40.0, f64::INFINITY), 39.0);
    }

    #[test]
    fn sum_channel_copies_into_silence_and_skips_silent_sources() {
        let mut d = [0.0f32; QUANTUM];
        let mut s = true;
        sum_channel(&mut d, &mut s, &[-0.0; QUANTUM], false);
        assert!(!s);
        assert!(d[0].is_sign_negative(), "copied, not added to +0");
        sum_channel(&mut d, &mut s, &[1.0; QUANTUM], true);
        assert!(
            d[0] == 0.0 && d[0].is_sign_negative(),
            "silent source skipped"
        );
        sum_channel(&mut d, &mut s, &[0.25; QUANTUM], false);
        assert_eq!(d[0], 0.25);
        sum_channel(&mut d, &mut s, &[0.5; QUANTUM], false);
        assert_eq!(d[0], 0.75);
    }

    #[test]
    fn mono_inputs_up_mix_and_stereo_inputs_sum_channel_wise() {
        let mut bus = [[0.0f32; QUANTUM]; 2];
        let mut silent = [true; 2];
        let m = [[0.5f32; QUANTUM]];
        sum_stereo(
            &mut bus,
            &mut silent,
            Input {
                channels: &m,
                silent: false,
            },
        );
        assert_eq!((bus[0][3], bus[1][3]), (0.5, 0.5));
        let st = [[0.25f32; QUANTUM], [-1.0; QUANTUM]];
        sum_stereo(
            &mut bus,
            &mut silent,
            Input {
                channels: &st,
                silent: false,
            },
        );
        assert_eq!((bus[0][3], bus[1][3]), (0.75, -0.5));
        sum_stereo(
            &mut bus,
            &mut silent,
            Input {
                channels: &st,
                silent: true,
            },
        );
        assert_eq!((bus[0][3], bus[1][3]), (0.75, -0.5));
        sum_stereo(
            &mut bus,
            &mut silent,
            Input {
                channels: &[],
                silent: false,
            },
        );
        assert_eq!(silent, [false; 2]);
    }

    #[test]
    fn the_pan_zero_panner_leaks_the_right_channel_into_the_left() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        o.begin();
        let st = [[0.5f32; QUANTUM], [0.25; QUANTUM]];
        o.add(Input {
            channels: &st,
            silent: false,
        });
        let (out, silent) = o.finish(0);
        assert!(!silent);
        assert_eq!(out[0][0], 0.25f32.mul_add(leak(), 0.5));
        assert_eq!(out[1][0], 0.25);
        // The leak is below the float resolution of 0.5 but survives a zero left channel.
        let st = [[0.0f32; QUANTUM], [1.0; QUANTUM]];
        o.begin();
        o.add(Input {
            channels: &st,
            silent: false,
        });
        let (out, _) = o.finish(128);
        assert_eq!(out[0][0], leak());
        assert!(out[0][0] > 6.1e-17 && out[0][0] < 6.2e-17);
        // A mono input reaches both channels unchanged.
        o.begin();
        o.add(Input {
            channels: &[[0.3f32; QUANTUM]],
            silent: false,
        });
        let (out, _) = o.finish(256);
        assert_eq!((out[0][9], out[1][9]), (0.3f32.mul_add(leak(), 0.3), 0.3));
        assert_eq!(out[0][9], 0.3, "x + x * 6e-17 rounds to x");
    }

    #[test]
    fn an_orbit_without_input_outputs_silence() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        o.begin();
        // `last_non_silent_time_` starts at 0, so the panner processes (zeros) at time 0.
        let (out, silent) = o.finish(0);
        assert!(!silent);
        assert!(
            out.iter()
                .flatten()
                .all(|x| *x == 0.0 && x.is_sign_positive())
        );
        o.begin();
        o.add(Input {
            channels: &[[1.0f32; QUANTUM]],
            silent: true,
        });
        assert!(o.finish(128).1, "silent inputs keep it silent");
        assert!(o.output().1);
    }

    #[test]
    fn mix_sums_every_orbit_onto_the_pair() {
        let j = job(
            2,
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]),
        );
        let mut bank = OrbitBank::new(&j);
        assert_eq!(bank.orbits().len(), 2);
        let outs = [mono_out(0.25), mono_out(0.5), stereo_out(0.0, 0.125)];
        let d = render(&mut bank, &outs, &[0, 1, 1]);
        assert_eq!(d.len(), 2);
        // Orbit 1: 0.25 both sides; orbit 2: (0.5 + 0, 0.5 + 0.125) with the leak on the left.
        let o2_l = 0.625f32.mul_add(leak(), 0.5);
        assert_eq!(d[0][0], 0.25 + o2_l);
        assert_eq!(d[1][0], 0.25 + 0.625);
    }

    #[test]
    fn stems_route_orbits_to_their_pairs_and_alias() {
        // 2 pairs: orbit 1 -> 0/1, orbit 2 -> 2/3, orbit 3 (channels 4/5) wraps onto 0/1, orbit
        // 0 -> 0/1; explicit channels [3] takes orbit 9's left only.
        let j = job(
            4,
            json!([
                { "orbit": 1, "channels": [0, 1] },
                { "orbit": 2, "channels": [2, 3] },
                { "orbit": 3, "channels": [4, 5] },
                { "orbit": 0, "channels": [0, 1] },
                { "orbit": 9, "channels": [3] }
            ]),
        );
        let mut bank = OrbitBank::new(&j);
        let outs = [
            mono_out(1.0),
            mono_out(2.0),
            mono_out(4.0),
            mono_out(8.0),
            stereo_out(16.0, 32.0),
        ];
        let d = render(&mut bank, &outs, &[0, 1, 2, 3, 4]);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0][0], 13.0);
        assert_eq!(d[1][0], 13.0);
        assert_eq!(d[2][0], 2.0);
        assert_eq!(d[3][0], 2.0 + 32.0f32.mul_add(leak(), 16.0));
        // Silence: nothing playing gives zeros everywhere (the 7.0 fill is overwritten).
        let d = render(&mut bank, &[], &[]);
        assert!(d.iter().flatten().all(|x| *x == 0.0));
    }

    #[test]
    fn voices_sum_in_connection_order() {
        // (a + b) + c rounds to 1; (c + b) + a keeps 2^-23.
        let (a, b, c) = (1.0f32, 2f32.powi(-24), 2f32.powi(-24));
        assert_ne!((a + b) + c, (c + b) + a);
        let j = job(2, json!([{ "orbit": 1, "channels": [0, 1] }]));
        let mut bank = OrbitBank::new(&j);
        let d = render(
            &mut bank,
            &[mono_out(a), mono_out(b), mono_out(c)],
            &[0, 0, 0],
        );
        assert_eq!(d[1][0], (a + b) + c);
        let d = render(
            &mut bank,
            &[mono_out(c), mono_out(b), mono_out(a)],
            &[0, 0, 0],
        );
        assert_eq!(d[1][0], (c + b) + a);
    }

    #[test]
    fn mono_destinations_get_both_sides() {
        let j = job(1, json!([{ "orbit": 1, "channels": [0, 1] }]));
        let mut bank = OrbitBank::new(&j);
        let d = render(&mut bank, &[stereo_out(0.5, 0.25)], &[0]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0][0], 0.25f32.mul_add(leak(), 0.5) + 0.25);
    }

    #[test]
    fn find_uses_the_first_orbit_with_the_number() {
        let j = job(
            2,
            json!([{ "orbit": 2, "channels": [0, 1] }, { "orbit": 1, "channels": [0, 1] }, { "orbit": 1, "channels": [0, 1] }]),
        );
        let bank = OrbitBank::new(&j);
        let m = |v: serde_json::Value| -> ValueMap { serde_json::from_value(v).unwrap() };
        assert_eq!(bank.find(&m(json!({}))), Ok(1), "orbit defaults to 1");
        assert_eq!(bank.find(&m(json!({ "orbit": "2" }))), Ok(0));
        assert_eq!(bank.find(&m(json!({ "orbit": "x" }))), Ok(1));
        assert_eq!(
            bank.find(&m(json!({ "orbit": 3.5 }))),
            Err("orbit 3.5 not in the job".to_string())
        );
        let empty = OrbitBank::new(&job(2, json!([])));
        assert_eq!(
            empty.find(&m(json!({}))),
            Err("orbit 1 not in the job".to_string())
        );
    }

    #[test]
    fn orbits_are_created_on_first_use_in_call_order() {
        let mut bank = OrbitBank::empty(2);
        assert_eq!(bank.count(), 2);
        assert!(bank.orbits().is_empty());
        let m = |v: serde_json::Value| -> ValueMap { serde_json::from_value(v).unwrap() };
        assert!(bank.find(&m(json!({}))).is_err());
        bank.create(&spec(3.0, &[0, 1]));
        bank.create(&spec(1.0, &[0, 1]));
        // Creating an existing orbit is a no-op (its channels are not replaced).
        bank.create(&spec(3.0, &[1]));
        let numbers: Vec<f64> = bank.orbits().iter().map(|o| o.orbit).collect();
        assert_eq!(numbers, [3.0, 1.0]);
        assert_eq!(bank.find(&m(json!({ "orbit": 3 }))), Ok(0));
        assert_eq!(bank.find(&m(json!({}))), Ok(1));
        // A bank grown this way renders as one built from a job with the same orbits.
        let mut built = OrbitBank::new(&job(
            2,
            json!([{ "orbit": 3, "channels": [0, 1] }, { "orbit": 1, "channels": [0, 1] }]),
        ));
        let outs = [mono_out(0.25), stereo_out(0.5, 0.125)];
        let a = render(&mut bank, &outs, &[0, 1]);
        let b = render(&mut built, &outs, &[0, 1]);
        assert_eq!(a, b);
        assert!(a[0][0] != 0.0);
    }

    #[test]
    fn unknown_orbit_indices_are_ignored() {
        let j = job(2, json!([{ "orbit": 1, "channels": [0, 1] }]));
        let mut bank = OrbitBank::new(&j);
        let d = render(&mut bank, &[mono_out(1.0)], &[5]);
        assert!(d.iter().flatten().all(|x| *x == 0.0));
    }

    /// `post` scaled by a send gain of `level`, as the voice hands it to the orbit.
    fn scaled_send(o: &VoiceOutput, level: f32) -> crate::voice::SendBuf {
        let mut ch = o.post;
        for c in ch.iter_mut() {
            for x in c.iter_mut() {
                *x *= level;
            }
        }
        crate::voice::SendBuf {
            ch,
            channels: o.channels,
            silent: o.post_silent || level == 0.0,
        }
    }

    fn sending(mut o: VoiceOutput, level: f32) -> VoiceOutput {
        o.post = o.out;
        o.post_silent = o.out_silent;
        o.sends.delay = Some(scaled_send(&o, level));
        o
    }

    #[test]
    fn the_delay_return_is_added_before_the_voices_and_fed_by_its_orbits_sends() {
        let j = job(
            2,
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]),
        );
        let mut bank = OrbitBank::new(&j);
        // Orbit 2 gets a delay of 0 s (a pass-through) with feedback 0.5.
        bank.orbits_mut()[1]
            .get_delay(0.0, true, 0.5, 0.0, 0)
            .unwrap();
        assert!(bank.orbits()[0].delay().is_none());
        assert_eq!(bank.orbits()[1].delay().unwrap().channels(), 1);
        // Voice 0 on orbit 1 sends 0.5 but orbit 1 has no delay; voice 1 on orbit 2 sends
        // 0.25 of 2; voice 2 on orbit 2 sends nothing.
        let outs = [
            sending(mono_out(1.0), 0.5),
            sending(mono_out(2.0), 0.25),
            mono_out(4.0),
        ];
        let d = render(&mut bank, &outs, &[0, 1, 1]);
        // Orbit 2's sum: the return 0.5 first, then 2 and 4.
        let o2 = (0.5f32 + 2.0) + 4.0;
        assert_eq!(d[1][0], 1.0 + o2);
        // Next quantum the feedback (0.5 * 0.5) joins the send.
        let d = render(&mut bank, &outs, &[0, 1, 1]);
        assert_eq!(d[1][0], 1.0 + ((0.25f32 + 0.5) + 2.0 + 4.0));
        // A stereo sender makes the delay stereo for good.
        let st = sending(stereo_out(0.0, 0.0), 1.0);
        render(&mut bank, &[st], &[1]);
        assert_eq!(bank.orbits()[1].delay().unwrap().channels(), 2);
        render(&mut bank, &[], &[]);
        assert_eq!(bank.orbits()[1].delay().unwrap().channels(), 2);
    }

    #[test]
    fn get_djf_first_call_creates_slot_and_event() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        assert!(o.djf().is_none());
        // First call at pause frame 22016 for a hap at 0.75 s: `.value = 0.2` at the pause (an
        // event at 22016 / sr) and `setValueAtTime(0.2, 0.75)`.
        o.get_djf(0.2, 0.75, 22016).unwrap();
        let d = o.djf().expect("the slot");
        let ev = d.value.events();
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].time, ev[0].value), (22016.0 / 44100.0, 0.2));
        assert_eq!((ev[1].time, ev[1].value), (0.75, 0.2));
        assert_eq!(d.value.value(), 0.2);
        // A later call (same pause) only schedules: the intrinsic value stays.
        o.get_djf(0.8, 1.75, 22016).unwrap();
        let d = o.djf().unwrap();
        assert_eq!(d.value.events().len(), 3);
        assert_eq!(d.value.value(), 0.2);
        assert_eq!(d.value.events()[2].value, 0.8);
    }

    #[test]
    fn get_djf_nonfinite_leaves_no_slot() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e39] {
            let e = o.get_djf(v, 0.0, 0).unwrap_err();
            assert_eq!(e, ParamError::NonFiniteValue { method: "value" }, "{v}");
            assert!(o.djf().is_none(), "{v}: no slot");
        }
        // With a slot, a non-finite value throws at `setValueAtTime` and leaves the slot.
        o.get_djf(0.3, 0.0, 0).unwrap();
        let e = o.get_djf(f64::NAN, 0.1, 0).unwrap_err();
        assert_eq!(
            e,
            ParamError::NonFiniteValue {
                method: "setValueAtTime"
            }
        );
        assert_eq!(o.djf().unwrap().value.events().len(), 2);
        // An orbit without a djf is unchanged by the failed calls: the identity path.
        let mut a = Orbit::new(&spec(1.0, &[0, 1]), 2);
        let _ = a.get_djf(f64::NAN, 0.0, 0);
        let mut b = Orbit::new(&spec(1.0, &[0, 1]), 2);
        for o in [&mut a, &mut b] {
            o.begin();
            o.add(Input {
                channels: &[[0.25f32; QUANTUM]],
                silent: false,
            });
        }
        assert_eq!(a.finish(0), b.finish(0));
    }

    #[test]
    fn the_djf_filters_the_whole_orbit_after_the_returns_and_is_never_silent() {
        // Orbit 2 with a pass-through delay: the djf sees return + voice, both channels.
        let j = job(2, json!([{ "orbit": 2, "channels": [0, 1] }]));
        let mut bank = OrbitBank::new(&j);
        let mut plain = OrbitBank::new(&j);
        for b in [&mut bank, &mut plain] {
            b.orbits_mut()[0].get_delay(0.0, true, 0.5, 0.0, 0).unwrap();
        }
        bank.orbits_mut()[0].get_djf(0.2, 0.0, 0).unwrap();
        let outs = [sending(mono_out(0.5), 0.5)];
        let mut want = crate::djf::Djf::new();
        want.value.set_value(0.2, 0).unwrap();
        want.value.set_value_at_time(0.2, 0.0, 0).unwrap();
        for k in 0..3u64 {
            let q = k * QUANTUM as u64;
            let mut d = vec![[0.0f32; QUANTUM]; 2];
            bank.process(q, &outs, &[0], &mut d);
            let mut p = vec![[0.0f32; QUANTUM]; 2];
            plain.process(q, &outs, &[0], &mut p);
            // The plain orbit's output is its sum (the pan-0 panner's right channel is exact).
            let mut sum = [p[1], p[1]];
            want.process(q, &mut sum);
            assert_eq!(d[1], sum[1], "quantum {k}: right channel = djf(sum)");
            assert_ne!(d[1], p[1], "quantum {k}: filtered");
        }
        // No input at all: the djf still outputs a non-silent (zero) bus.
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        o.get_djf(0.8, 0.0, 0).unwrap();
        o.begin();
        let (out, silent) = o.finish(1280);
        assert!(!silent);
        assert!(out.iter().flatten().all(|x| *x == 0.0));
        // Without a djf the same orbit past its panner's tail is silent.
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        o.begin();
        o.finish(0);
        o.begin();
        assert!(o.finish(1280).1);
    }

    #[test]
    fn get_delay_errors_reach_the_caller() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        assert!(o.get_delay(f64::INFINITY, true, 0.5, 0.0, 0).is_err());
        assert!(o.delay().is_none());
        o.get_delay(0.1, true, 0.5, 0.0, 0).unwrap();
        assert!(o.delay().is_some());
    }

    #[test]
    fn each_orbit_keeps_its_own_reverb_and_the_bank_settles_them() {
        use crate::job::Value;
        let j = job(
            2,
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]),
        );
        let mut bank = OrbitBank::new(&j);
        let size = Value::Num(0.01);
        let zero = Value::Num(0.0);
        // A failing first request creates nothing on that orbit.
        let e = bank.orbits_mut()[0]
            .get_reverb(Some(&zero), None, None, None)
            .unwrap_err();
        assert_eq!(e, IrError::Length(0.0));
        assert!(bank.orbits()[0].reverb().is_none());
        bank.orbits_mut()[1]
            .get_reverb(Some(&size), None, Some(&zero), None)
            .unwrap();
        // A second hap with the same settings makes no request (`hasChanged` is false).
        bank.orbits_mut()[1]
            .get_reverb(Some(&size), None, Some(&zero), None)
            .unwrap();
        assert_eq!(bank.orbits()[1].reverb().unwrap().latest(), 1);
        assert!(!bank.orbits_mut()[0].settle_reverb(0, &mut IrCache::new()));
        bank.settle(0, &mut IrCache::new());
        assert!(bank.orbits()[0].reverb().is_none());
        let r = bank.orbits()[1].reverb().unwrap();
        assert_eq!((r.applied(), r.ir().unwrap()[1].len()), (1, 662));
        // An error on an existing reverb keeps it and its IR.
        assert!(
            bank.orbits_mut()[1]
                .get_reverb(Some(&zero), None, None, None)
                .is_err()
        );
        bank.settle(0, &mut IrCache::new());
        let r = bank.orbits()[1].reverb().unwrap();
        assert_eq!(
            (r.latest(), r.applied(), r.ir().unwrap()[0].len()),
            (2, 1, 662)
        );
    }

    fn room(mut o: VoiceOutput, level: f32) -> VoiceOutput {
        o.post = o.out;
        o.post_silent = o.out_silent;
        o.sends.room = Some(scaled_send(&o, level));
        o
    }

    /// `roomsize` giving a one-frame IR (`round(1.5 d sr) = 1`), no lowpass.
    fn one_frame_reverb(o: &mut Orbit) {
        use crate::job::Value;
        o.get_reverb(
            Some(&Value::Num(1.0 / 66150.0)),
            None,
            Some(&Value::Num(0.0)),
            None,
        )
        .unwrap();
        assert!(o.settle_reverb(0, &mut IrCache::new()));
        assert_eq!(o.reverb().unwrap().ir().unwrap()[0].len(), 1);
    }

    /// The one-frame IR's taps after normalization.
    fn taps(o: &Orbit) -> [f32; 2] {
        let ir = o.reverb().unwrap().ir().unwrap();
        let scale = crate::reverb::normalization_scale(ir);
        [ir[0][0] * scale, ir[1][0] * scale]
    }

    #[test]
    fn the_reverb_return_sums_its_orbits_room_sends() {
        let j = job(2, json!([{ "orbit": 1, "channels": [0, 1] }]));
        let mut bank = OrbitBank::new(&j);
        one_frame_reverb(&mut bank.orbits_mut()[0]);
        assert_eq!(bank.orbits()[0].returns(), [Return::Reverb]);
        let k = taps(&bank.orbits()[0]);
        // Two mono senders (levels 0.5 and 0.25) and a stereo voice that does not send: the
        // convolver input is mono (1 * 0.5 + 2 * 0.25), feeding both IR channels.
        let outs = [
            room(mono_out(1.0), 0.5),
            room(mono_out(2.0), 0.25),
            stereo_out(3.0, 5.0),
        ];
        let d = render(&mut bank, &outs, &[0, 0, 0]);
        let wet = [1.0f32 * k[0], 1.0f32 * k[1]];
        let l = ((wet[0] + 1.0) + 2.0) + 3.0;
        let r = ((wet[1] + 1.0) + 2.0) + 5.0;
        assert_eq!((d[0][0], d[1][0]), (r.mul_add(leak(), l), r));
        // A stereo sender makes the input stereo: L and R convolve separately.
        let outs = [room(stereo_out(1.0, -1.0), 1.0)];
        let d = render(&mut bank, &outs, &[0]);
        let (l, r) = (k[0] + 1.0, -k[1] - 1.0);
        assert_eq!((d[0][5], d[1][5]), (r.mul_add(leak(), l), r));
        // A level that rounds to zero (or a silent post) adds nothing.
        let outs = [room(mono_out(1.0), 0.0)];
        let d = render(&mut bank, &outs, &[0]);
        assert_eq!(d[1][0], 1.0);
    }

    #[test]
    fn returns_are_added_in_creation_order() {
        let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
        one_frame_reverb(&mut o);
        o.get_delay(0.1, true, 0.5, 0.0, 0).unwrap();
        // A later `getDelay` (an update) adds no second return.
        o.get_delay(0.2, true, 0.5, 0.0, 0).unwrap();
        assert_eq!(o.returns(), [Return::Reverb, Return::Delay]);
        let mut p = Orbit::new(&spec(1.0, &[0, 1]), 2);
        assert!(p.get_delay(f64::NAN, true, 0.5, 0.0, 0).is_err());
        assert!(p.returns().is_empty());
        p.get_delay(0.1, true, 0.5, 0.0, 0).unwrap();
        one_frame_reverb(&mut p);
        assert_eq!(p.returns(), [Return::Delay, Return::Reverb]);
        // A reverb whose first request fails is not created and adds no return.
        let mut q = Orbit::new(&spec(1.0, &[0, 1]), 2);
        use crate::job::Value;
        assert!(
            q.get_reverb(Some(&Value::Num(0.0)), None, None, None)
                .is_err()
        );
        assert!(q.returns().is_empty());
    }

    #[test]
    fn a_voice_without_room_does_not_feed_the_reverb() {
        let j = job(
            2,
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]),
        );
        let mut bank = OrbitBank::new(&j);
        one_frame_reverb(&mut bank.orbits_mut()[1]);
        // The sender is on orbit 1 (no reverb there); orbit 2's reverb gets a silent input and
        // outputs zeros (processed: within its tail time at frame 0).
        let d = render(&mut bank, &[room(mono_out(1.0), 1.0)], &[0]);
        assert_eq!((d[0][0], d[1][0]), (1.0, 1.0));
    }

    // ---- the output gain and duck

    use crate::param::RampKind;

    use crate::SAMPLE_RATE_F64 as SR;

    /// One quantum of input per entry: (left, right, silent); some with `-0`, NaN and silence.
    fn identity_inputs() -> Vec<(f32, f32, bool)> {
        vec![
            (0.0, 0.0, true),
            (0.25, -0.5, false),
            (-0.0, 1e-30, false),
            (3.0, f32::NAN, false),
            (0.0, 0.0, true),
            (0.0, 0.0, true),
            (f32::MAX, -f32::MIN_POSITIVE, false),
            (0.0, 0.0, true),
        ]
    }

    #[test]
    fn orbit_output_is_identity_without_events() {
        for with_djf in [false, true] {
            let mut o = Orbit::new(&spec(1.0, &[0, 1]), 2);
            let mut panner = StereoPanner::new();
            let mut djf = Djf::new();
            if with_djf {
                o.djf = Some(Djf::new());
            }
            for (k, (l, r, silent)) in identity_inputs().into_iter().enumerate() {
                let q = (k * QUANTUM) as u64;
                let chans = [[l; QUANTUM], [r; QUANTUM]];
                o.begin();
                o.add(Input {
                    channels: &chans,
                    silent,
                });
                // The old path: the summing bus straight into the panner (through the djf).
                let mut sum: Stereo = if silent { [[0.0; QUANTUM]; 2] } else { chans };
                let mut hint = silent;
                if with_djf {
                    hint = djf.process(q, &mut sum);
                }
                let mut want: Stereo = [[0.0; QUANTUM]; 2];
                let want_silent = panner.process(q, &sum, hint, &mut want);
                let (got, got_silent) = o.finish(q);
                assert_eq!(got_silent, want_silent, "djf {with_djf} quantum {k}");
                for (a, b) in got.iter().flatten().zip(want.iter().flatten()) {
                    assert_eq!(a.to_bits(), b.to_bits(), "djf {with_djf} quantum {k}");
                }
            }
            assert!(o.output_gain().events().is_empty());
            assert_eq!(o.output_gain().value(), 1.0);
            assert!(o.duck_curve().is_none());
        }
    }

    /// `(time, value, ramp)` points.
    fn pts(xs: &[(f64, f64, bool)]) -> Vec<GainPoint> {
        xs.iter()
            .map(|&(time, value, ramp)| GainPoint { time, value, ramp })
            .collect()
    }

    #[test]
    fn gain_at_follows_the_js_model() {
        assert_eq!(gain_at(0.75, &[], 3.0), 0.75, "no points: initial");
        let held = pts(&[(0.5, 0.3, false)]);
        assert_eq!(gain_at(1.0, &held, 0.4), 1.0, "before a held point");
        assert_eq!(gain_at(1.0, &held, 0.5), 0.3, "at it (<=)");
        assert_eq!(gain_at(1.0, &held, 9.0), 0.3);
        let ramp = pts(&[(0.2, 1.0, false), (0.5, 0.1, true)]);
        let mid = gain_at(1.0, &ramp, 0.35);
        assert_eq!(
            mid,
            1.0 * jsmath::pow(0.1 / 1.0, (0.35 - 0.2) / (0.5 - 0.2))
        );
        assert_eq!(gain_at(1.0, &ramp, 0.2), 1.0, "at the ramp's start");
        assert_eq!(gain_at(1.0, &ramp, 0.5), 0.1, "at its end");
        // A ramp first is not interpolated (no previous point): the initial value.
        assert_eq!(gain_at(0.75, &pts(&[(0.5, 0.1, true)]), 0.25), 0.75);
        // A ramp whose end is not after the previous point jumps.
        let jump = pts(&[(0.3, 0.5, false), (0.3, 0.2, true)]);
        assert_eq!(gain_at(1.0, &jump, 0.3), 0.2);
        assert_eq!(gain_at(1.0, &jump, 0.29), 1.0);
        // NaN x: no point is <= it, the first point stops the scan.
        assert_eq!(gain_at(1.0, &ramp, f64::NAN), 1.0);
    }

    #[test]
    fn gain_at_matches_js_cases() {
        use crate::test_vectors::{load, num, within_ulps};
        let v = load("duck-gain-at.json");
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 12, "{} cases", cases.len());
        let mut n = 0;
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let initial = num(&c["initial"]);
            let points: Vec<GainPoint> = c["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| GainPoint {
                    time: num(&p["time"]),
                    value: num(&p["value"]),
                    ramp: p["ramp"].as_bool().unwrap(),
                })
                .collect();
            for s in c["samples"].as_array().unwrap() {
                let x =
                    f64::from_bits(u64::from_str_radix(s["xBits"].as_str().unwrap(), 16).unwrap());
                assert_eq!(x.to_bits(), num(&s["x"]).to_bits(), "{name}: x");
                let want =
                    f64::from_bits(u64::from_str_radix(s["bits"].as_str().unwrap(), 16).unwrap());
                let got = gain_at(initial, &points, x);
                // Bun's Math.pow (JavaScriptCore) versus jsmath::pow (V8's): 1 ulp.
                assert!(
                    within_ulps(got, want, 1),
                    "{name}: gainAt({x}) = {got}, JS {want}"
                );
                n += 1;
            }
        }
        assert!(n > 12 * 64, "{n} samples");
    }

    #[test]
    fn duck_replays_the_js_sequence() {
        use crate::test_vectors::{load, num, within_ulps};
        let v = load("duck-gain-at.json");
        let calls = v["calls"].as_array().unwrap();
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        for (k, d) in v["sequence"].as_array().unwrap().iter().enumerate() {
            let (t, onset, attack, depth) = (
                num(&d["t"]),
                num(&d["onset"]),
                num(&d["attack"]),
                num(&d["depth"]),
            );
            // The controller applies `Math.max(attack, 0.002)` before offlineDuck does it again.
            o.duck(t, onset, js_max(attack, 0.002), depth, 0).unwrap();
            let arg = |i: usize, j: usize| {
                let a = &calls[3 * k + i]["args"][j];
                f64::from_bits(u64::from_str_radix(a["bits"].as_str().unwrap(), 16).unwrap())
            };
            assert_eq!(calls[3 * k]["method"], "cancelAndHoldAtTime");
            assert_eq!(arg(0, 0), t);
            let points = &o.duck_curve().unwrap().points;
            let n = points.len();
            let ducked = points[n - 2].value;
            assert!(
                within_ulps(ducked, arg(1, 0), 1),
                "duck {k}: ducked {ducked}"
            );
            assert_eq!(points[n - 2].time, arg(1, 1), "duck {k}: fall end");
            assert_eq!(points[n - 1].time, arg(2, 1), "duck {k}: recovery end");
            assert_eq!(arg(2, 0), 1.0);
            // The points equal the JS case `sequence after duck k + 1`.
            let case = v["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == format!("sequence after duck {}", k + 1))
                .unwrap();
            let want = case["points"].as_array().unwrap();
            assert_eq!(want.len(), n, "duck {k}");
            for (p, w) in points.iter().zip(want) {
                assert!(within_ulps(p.value, num(&w["value"]), 1), "duck {k}");
                assert_eq!(p.time, num(&w["time"]), "duck {k}");
                assert_eq!(p.ramp, w["ramp"].as_bool().unwrap(), "duck {k}");
            }
        }
        assert_eq!(calls.len(), 12);
    }

    #[test]
    fn duck_first_call_inserts_pause_set_value() {
        let now = 19968;
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        o.duck(0.75, 0.0, 0.3, 0.75, now).unwrap();
        let curve = o.duck_curve().unwrap();
        assert_eq!(
            curve.initial, 1.0,
            "g.value is exactly 1 before the first duck"
        );
        let ducked = 1.0 - 0.75f64.sqrt();
        assert_eq!(
            curve.points,
            pts(&[(0.75, 1.0, false), (0.75, ducked, true), (1.05, 1.0, true)])
        );
        let e = o.output_gain().events();
        let got: Vec<(RampKind, u32, f64)> = e
            .iter()
            .map(|e| (e.kind, e.value.to_bits(), e.time))
            .collect();
        assert_eq!(
            got,
            vec![
                (RampKind::SetValue, 1.0f32.to_bits(), now as f64 / SR),
                (RampKind::Exponential, (ducked as f32).to_bits(), 0.75),
                (RampKind::Exponential, 1.0f32.to_bits(), 0.75 + 0.3),
            ],
            "no cancel on an empty timeline; the first ramp starts at the pause"
        );
        // Rendered: 1 at the pause frame, falling to the ducked value at 0.75 s, 1 again after.
        let mut gains = Vec::new();
        for k in 0..400u64 {
            let q = now + k * QUANTUM as u64;
            o.begin();
            o.add(Input {
                channels: &[[1.0f32; QUANTUM]],
                silent: false,
            });
            let (out, _) = o.finish(q);
            gains.extend_from_slice(&out[1]);
        }
        assert_eq!(gains[0], 1.0);
        assert!(gains[1] < 1.0, "the fall starts at the pause frame");
        let at = (0.75 * SR) as usize - now as usize;
        assert_eq!(gains[at], ducked as f32);
        assert!(
            gains[(1.05 * SR) as usize + 1 - now as usize..]
                .iter()
                .all(|x| *x == 1.0)
        );
    }

    #[test]
    fn duck_error_leaves_curve_unchanged() {
        // NaN depth (negative or NaN): `ducked` is NaN, the first ramp throws after the cancel.
        for depth in [f64::NAN, -0.5] {
            let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
            let e = o.duck(0.5, 0.0, 0.1, depth, 0).unwrap_err();
            assert_eq!(
                e,
                ParamError::NonFiniteValue {
                    method: "exponentialRampToValueAtTime"
                }
            );
            assert!(
                o.output_gain().events().is_empty(),
                "the cancel changed nothing"
            );
            let curve = o
                .duck_curve()
                .expect("created by the first call even when it throws");
            assert!(curve.points.is_empty());
        }
        // NaN attack: the first ramp is inserted, the second throws; the points stay.
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        o.duck(0.25, 0.0, 0.1, 0.5, 0).unwrap();
        let before = o.duck_curve().unwrap().clone();
        let n = o.output_gain().events().len();
        let e = o.duck(0.5, 0.05, f64::NAN, 0.5, 0).unwrap_err();
        assert_eq!(
            e,
            ParamError::NonFiniteTime {
                method: "exponentialRampToValueAtTime"
            }
        );
        assert_eq!(
            o.output_gain().events().len(),
            n + 1,
            "the first ramp stays"
        );
        assert_eq!(o.output_gain().events().last().unwrap().time, 0.55);
        assert_eq!(o.duck_curve().unwrap(), &before);
        // A NaN onset fails at the first ramp's time; a negative t at the cancel.
        assert!(o.duck(0.75, f64::NAN, 0.1, 0.5, 0).is_err());
        assert_eq!(
            o.duck(-1.0, 0.0, 0.1, 0.5, 0),
            Err(ParamError::NegativeTime {
                method: "cancelAndHoldAtTime",
                time: -1.0
            })
        );
        assert_eq!(o.duck_curve().unwrap(), &before);
    }

    #[test]
    fn duck_extremes() {
        // depth +inf: 1 - inf = -inf, floored at 0.01; depth 0: ducked = held (1); attack 0 is
        // floored at 0.002.
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        o.duck(0.5, 0.0, 0.0, f64::INFINITY, 0).unwrap();
        let p = &o.duck_curve().unwrap().points;
        assert_eq!((p[1].value, p[2].time), (0.01, 0.502));
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        o.duck(0.5, 0.0, 0.1, 0.0, 0).unwrap();
        assert_eq!(o.duck_curve().unwrap().points[1].value, 1.0);
        // -inf depth: sqrt is NaN.
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        assert!(o.duck(0.5, 0.0, 0.1, f64::NEG_INFINITY, 0).is_err());
        // A held value below the floor clamps ducked down to it.
        let mut o = Orbit::new(&spec(2.0, &[0, 1]), 2);
        o.duck(0.25, 0.0, 0.4, 0.8, 0).unwrap();
        o.duck(0.4, 0.0, 0.1, 0.1, 0).unwrap();
        let p = &o.duck_curve().unwrap().points;
        let held = p[2].value;
        assert!(
            (held - 0.10557280900008414f64.powf(0.625)).abs() < 1e-12,
            "{held}"
        );
        assert_eq!(p[3].value, held, "min(0.684, held)");
    }

    fn bank(orbits: &[f64]) -> OrbitBank {
        let specs: Vec<serde_json::Value> = orbits
            .iter()
            .map(|o| json!({ "orbit": o, "channels": [0, 1] }))
            .collect();
        OrbitBank::new(&job(2, json!(specs)))
    }

    fn controls_of(v: serde_json::Value) -> ValueMap {
        match Value::from_json(v).unwrap() {
            Value::Obj(m) => m,
            other => panic!("{other:?}"),
        }
    }

    /// The (fall end time, ducked value, recovery end time) of each ducked orbit's curve.
    fn ducks(b: &OrbitBank) -> Vec<Option<(f64, f64, f64)>> {
        b.orbits()
            .iter()
            .map(|o| {
                o.duck_curve().filter(|c| !c.points.is_empty()).map(|c| {
                    let n = c.points.len();
                    (
                        c.points[n - 2].time,
                        c.points[n - 2].value,
                        c.points[n - 1].time,
                    )
                })
            })
            .collect()
    }

    fn duck_with(b: &mut OrbitBank, v: serde_json::Value) -> Result<(), ParamError> {
        duck_missing(b, v).0
    }

    /// `duck_with` and the targets reported missing.
    fn duck_missing(
        b: &mut OrbitBank,
        v: serde_json::Value,
    ) -> (Result<(), ParamError>, Vec<String>) {
        let m = controls_of(v);
        let c = DuckControls::of(&m).expect("ducks");
        let mut missing = Vec::new();
        let r = b.duck(&c, 1.0, 0, &mut missing);
        (r, missing)
    }

    #[test]
    fn duck_onset_array_fallbacks() {
        let d = |depth: f64| (1.0 - depth.sqrt()).max(0.01);
        // Absent controls: the controller's defaults 0, 0.1, 1.
        let mut b = bank(&[2.0]);
        duck_with(&mut b, json!({ "duckorbit": 2 })).unwrap();
        assert_eq!(ducks(&b), vec![Some((1.0, 0.01, 1.1))]);
        // Per-target arrays; a shorter array falls back to its element 0; a null element too.
        let mut b = bank(&[2.0, 3.0]);
        duck_with(
            &mut b,
            json!({ "duckorbit": [2, 3], "duckonset": [0.1, null], "duckattack": [0.2],
                    "duckdepth": [0.25, 0.5] }),
        )
        .unwrap();
        assert_eq!(
            ducks(&b),
            vec![Some((1.1, d(0.25), 1.3)), Some((1.1, d(0.5), 1.3))]
        );
        // Null controls: onset 0, attack max(null, 0.002) = 0.002, depth sqrt(null) = 0.
        let mut b = bank(&[2.0]);
        duck_with(
            &mut b,
            json!({ "duckorbit": 2, "duckonset": null, "duckattack": null, "duckdepth": null }),
        )
        .unwrap();
        assert_eq!(ducks(&b), vec![Some((1.0, 1.0, 1.002))]);
        // Empty arrays (undefined elements): onset 0, depth 1 (offlineDuck's defaults), attack
        // max(undefined, 0.002) = NaN, which throws at the second ramp.
        let mut b = bank(&[2.0]);
        duck_with(
            &mut b,
            json!({ "duckorbit": 2, "duckonset": [], "duckdepth": [] }),
        )
        .unwrap();
        assert_eq!(ducks(&b), vec![Some((1.0, 0.01, 1.1))]);
        let mut b = bank(&[2.0]);
        assert_eq!(
            duck_with(&mut b, json!({ "duckorbit": 2, "duckattack": [] })),
            Err(ParamError::NonFiniteTime {
                method: "exponentialRampToValueAtTime"
            })
        );
        // Booleans are numbers; strings and objects are NaN (a documented deviation).
        let mut b = bank(&[2.0]);
        duck_with(
            &mut b,
            json!({ "duckorbit": 2, "duckonset": true, "duckdepth": false }),
        )
        .unwrap();
        assert_eq!(ducks(&b), vec![Some((2.0, 1.0, 2.1))]);
        let mut b = bank(&[2.0]);
        assert!(duck_with(&mut b, json!({ "duckorbit": 2, "duckdepth": "0.5" })).is_err());
        // The first error stops the targets after it; the ones before keep their duck.
        let mut b = bank(&[2.0, 3.0, 4.0]);
        let r = duck_with(
            &mut b,
            json!({ "duckorbit": [2, 3, 4], "duckdepth": [0.5, -1, 0.5] }),
        );
        assert!(r.is_err());
        assert_eq!(ducks(&b), vec![Some((1.0, d(0.5), 1.1)), None, None]);
        assert!(
            b.orbits()[1].duck_curve().is_some(),
            "the failing orbit's curve exists"
        );
        assert!(
            b.orbits()[2].duck_curve().is_none(),
            "the target after it is never reached"
        );
    }

    #[test]
    fn duck_missing_target_is_reported_and_ducks_nothing() {
        let mut b = bank(&[2.0]);
        let (r, missing) = duck_missing(&mut b, json!({ "duckorbit": [7, 2] }));
        r.unwrap();
        assert_eq!(
            ducks(&b),
            vec![Some((1.0, 0.01, 1.1))],
            "7 skipped, 2 still ducked"
        );
        assert_eq!(missing, ["7"]);
        let mut b = bank(&[2.0]);
        let (r, missing) = duck_missing(&mut b, json!({ "duckorbit": 7 }));
        r.unwrap();
        assert_eq!(missing, ["7"]);
        assert_eq!(ducks(&b), vec![None]);
        assert!(b.orbits()[0].output_gain().events().is_empty());
        // Every missing target in order, repeats included; the JS text of each.
        let (r, missing) = duck_missing(
            &mut b,
            json!({ "duckorbit": [7, "x", 7, 2.5, null, true, { "a": 1 }, -0.0] }),
        );
        r.unwrap();
        assert_eq!(
            missing,
            ["7", "x", "7", "2.5", "null", "true", "[object Object]", "0"]
        );
        assert_eq!(ducks(&b), vec![None]);
        // A target before a throw is reported; one after it is never reached.
        let mut b = bank(&[2.0]);
        let (r, missing) = duck_missing(
            &mut b,
            json!({ "duckorbit": [7, 2, 8], "duckdepth": [0.5, -1, 0.5] }),
        );
        assert!(r.is_err());
        assert_eq!(missing, ["7"]);
        // An empty target list ducks nothing and reports nothing; `null` does not duck at all.
        let (r, missing) = duck_missing(&mut b, json!({ "duckorbit": [] }));
        r.unwrap();
        assert!(missing.is_empty());
        assert!(DuckControls::of(&controls_of(json!({ "duckorbit": null }))).is_none());
        assert!(DuckControls::of(&controls_of(json!({ "duckdepth": 1 }))).is_none());
    }

    #[test]
    fn duck_target_key_matching() {
        for target in [json!(2), json!("2"), json!([2.0])] {
            let mut b = bank(&[1.0, 2.0]);
            duck_with(&mut b, json!({ "duckorbit": target })).unwrap();
            assert!(b.orbits()[1].duck_curve().is_some(), "{target}");
            assert!(b.orbits()[0].duck_curve().is_none(), "{target}");
        }
        for target in [json!("2.0"), json!(" 2"), json!(true)] {
            let mut b = bank(&[1.0, 2.0]);
            duck_with(&mut b, json!({ "duckorbit": target })).unwrap();
            assert!(
                b.orbits().iter().all(|o| o.duck_curve().is_none()),
                "{target}"
            );
        }
        // An array target (an element of `[duckorbit].flat()` that is itself an array) is keyed
        // by its JS string: `[[2]]` and `[[[2]]]` are "2" and duck orbit 2.
        for target in [json!([[2]]), json!([[[2.0]]])] {
            let mut b = bank(&[1.0, 2.0]);
            let (r, missing) = duck_missing(&mut b, json!({ "duckorbit": target }));
            r.unwrap();
            assert!(missing.is_empty(), "{target}");
            assert!(b.orbits()[1].duck_curve().is_some(), "{target}");
            assert!(b.orbits()[0].duck_curve().is_none(), "{target}");
        }
        // A multi-element array target is "1,2" and an object "[object Object]": no orbit has
        // either key, so both are reported missing and nothing ducks.
        let mut b = bank(&[1.0, 2.0]);
        let (r, missing) = duck_missing(
            &mut b,
            json!({ "duckorbit": [[1, 2], { "a": 2 }, [[2, null]]] }),
        );
        r.unwrap();
        assert_eq!(missing, ["1,2", "[object Object]", "2,"]);
        assert!(b.orbits().iter().all(|o| o.duck_curve().is_none()));
        // Fractional and negative orbits use JS's String(number).
        let mut b = bank(&[0.5, -1.0]);
        duck_with(&mut b, json!({ "duckorbit": ["0.5", -1] })).unwrap();
        assert!(b.orbits().iter().all(|o| o.duck_curve().is_some()));
        // Two orbits with one number: the first one.
        let mut b = bank(&[2.0, 2.0]);
        duck_with(&mut b, json!({ "duckorbit": 2 })).unwrap();
        assert!(b.orbits()[0].duck_curve().is_some() && b.orbits()[1].duck_curve().is_none());
    }
}

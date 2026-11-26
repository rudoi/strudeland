// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/offline_audio_context.cc, platform/audio/audio_utilities.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! The render driver: the pause loop of `renderOffline` on an offline context, in 128-frame
//! quanta. The frame conversions here are the only place the engine turns seconds into frames
//! (`OfflineAudioContext::suspendContext`'s quantization and `TimeToSampleFrame`).

use crate::bus::{BusBank, BusGraph};
use crate::controls::{parse_voice_params, prepare_value, unsupported_signatures_in};
use crate::job::{EngineJob, ErrorKind, JobError, OrbitSpec, RenderError, ValueMap, bar_of};
use crate::js::{Occurrence, voice_seed};
use crate::noise::NoiseCache;
use crate::orbit::{DuckControls, OrbitBank};
use crate::periodic::WaveTables;
use crate::reverb::IrCache;
use crate::stems::{StemCapture, StemLayout};
use crate::voice::{StartInfo, Voice, VoiceOutput, sound_name};
use crate::{QUANTUM, Rendered};

use crate::QUANTUM_U64 as Q;
use crate::SAMPLE_RATE_F64 as SR;
/// A hap later than the pause time by more than this is reported (`renderOffline`'s rule).
const LATE_EPSILON: f64 = 1e-6;
/// The render page keeps the first 300 characters of a captured message (`classifyError`).
const MAX_LOG_CHARS: usize = 300;

/// `RoundUpToMultiple(frame, 128)` (saturating at the top of the range).
pub fn round_up_128(frame: u64) -> u64 {
    match frame % Q {
        0 => frame,
        r => frame.saturating_add(Q - r),
    }
}

/// The frame an offline suspend at `suspend_sec` happens at: `(size_t)(when * sampleRate)`
/// rounded up to the quantum (129.9 frames -> 129 -> 256).
pub fn frame_for_pause(suspend_sec: f64) -> u64 {
    round_up_128((suspend_sec * SR).trunc() as u64)
}

/// The first frame an AudioParam event at time `t` applies to: `ceil(t * sr)`.
pub fn frame_for_param_event(t: f64) -> u64 {
    (t * SR).ceil() as u64
}

/// `TimeToSampleFrame(t, sr, kRoundUp)`: round `t * sr` to 1/1024 of a frame (half away from
/// zero), then up. Scheduled sources start on, and stop before, this frame.
pub fn frame_for_buffer_start(t: f64) -> u64 {
    ((t * SR * 1024.0).round() / 1024.0).ceil() as u64
}

/// Superdough's log for a duck target with no orbit (`errorLogger` of the error "duck target orbit
/// ${target} does not exist" with origin `superdough`, superdoughoutput.mjs:219), as the render page
/// captures it.
fn missing_duck_target(target: &str) -> String {
    format!("[superdough] error: duck target orbit {target} does not exist")
}

/// What a voice's messages are prefixed with (`hap <t>s <s>: `), formatted only on error.
struct HapTag {
    bar: Option<i64>,
    t: f64,
    s: String,
}

impl HapTag {
    /// The tag of a hap of `value` at `t` in bar `bar`.
    fn new(bar: Option<i64>, t: f64, value: &crate::job::ValueMap) -> HapTag {
        HapTag {
            bar,
            t,
            s: sound_name(value.get("s")),
        }
    }

    fn message(&self, detail: &str) -> String {
        format!("hap {:.3}s {}: {detail}", self.t, self.s)
    }

    /// The render error `detail` of this hap: its message, of kind `kind`, in its bar.
    fn error(&self, kind: ErrorKind, detail: &str) -> RenderError {
        RenderError::new(kind, self.message(detail), self.bar)
    }
}

/// A live voice and what the driver keeps with it.
pub(crate) struct Slot {
    pub(crate) voice: Voice,
    /// Its bar and the parts of its messages' prefix.
    tag: HapTag,
    /// The index of its orbit in `orbits`.
    pub(crate) orbit: usize,
    /// Its value's index: its bus level is read from the graph each quantum (voices render level
    /// by level, and a graph that grows can move a value's level).
    pub(crate) value: usize,
    /// Its creation sequence number (bus feeders sum in this order).
    pub(crate) seq: u64,
    /// Whether its local reverbs' IR requests are still to be settled (set at build; the
    /// streaming driver clears it when it settles or sends them; offline settles each pause's
    /// new voices by position).
    pub(crate) unsettled: bool,
}

/// Keeps the items of `v` whose flag in `keep` (same order) is set, in one in-place pass.
fn retain_flagged<T>(v: &mut Vec<T>, keep: &[bool]) {
    let mut flags = keep.iter();
    v.retain(|_| flags.next().copied().unwrap_or(true));
}

/// One hap for [`Core::schedule`]: its (mutated-before) value and timing. `t` is already clamped
/// to the render head; the late test is the driver's.
pub(crate) struct ScheduleArgs<'v> {
    pub(crate) value: &'v ValueMap,
    /// The value's index (its bus level).
    pub(crate) value_index: usize,
    /// Start in seconds from frame 0, at or after `now_frame`.
    pub(crate) t: f64,
    pub(crate) duration: f64,
    /// Absolute layout cycle of the onset.
    pub(crate) cycle: f64,
    pub(crate) phase_sec: Option<f64>,
    /// The hap's cycles per second (`delaysync`, tremolo sync, the modulators).
    pub(crate) cps: f64,
    /// The frame the hap is scheduled at (the render head).
    pub(crate) now_frame: u64,
}

/// The render core shared by the offline [`Engine`] and a streaming driver: the voices, buses,
/// orbits and caches, scheduled hap by hap and rendered a quantum at a time into `mix`.
pub(crate) struct Core {
    /// The audio seed (voice seeds, noise, reverb IRs).
    pub(crate) seed: i32,
    /// The live voices in creation order.
    pub(crate) slots: Vec<Slot>,
    /// Per voice (same order): its output of the quantum being rendered.
    pub(crate) outputs: Vec<VoiceOutput>,
    /// Per voice (same order): its orbit, refreshed each quantum for the orbit bank.
    pub(crate) orbit_of: Vec<usize>,
    /// The next voice's sequence number.
    pub(crate) next_seq: u64,
    /// The bus graph (levels; a cyclic job is refused before rendering).
    pub(crate) bus_graph: BusGraph,
    /// The buses, one quantum's feeders at a time.
    pub(crate) buses: BusBank,
    /// The orbits and the channel merger.
    pub(crate) orbits: OrbitBank,
    /// The periodic-wave tables of this run (built-ins on first use, custom waves cached).
    pub(crate) tables: WaveTables,
    /// The noise buffers of this run (white, pink and brown on first use, for the seed).
    pub(crate) noise: NoiseCache,
    /// The reverb IRs generated in this run (orbit and local reverbs share equal ones).
    pub(crate) ir_cache: IrCache,
    /// `voiceSeed`'s state across the whole run (the page sets the audio seed once per render).
    pub(crate) occurrence: Occurrence,
    /// Every voice seed computed, in call order (tests observe `occurrence` through it).
    #[cfg(test)]
    pub(crate) seeds: Vec<u32>,
    /// The next frame to render (always a multiple of the quantum).
    pub(crate) frame: u64,
    /// Whether a non-finite mix sample was already reported.
    pub(crate) mix_overflowed: bool,
    pub(crate) errors: Vec<RenderError>,
    /// The last quantum's mix, one buffer per destination channel (`orbits.count()`).
    pub(crate) mix: Vec<[f32; QUANTUM]>,
    /// Per voice of the quantum being rendered: whether it is still alive after it.
    pub(crate) alive: Vec<bool>,
}

impl Core {
    pub(crate) fn new(seed: i32, bus_graph: BusGraph, orbits: OrbitBank) -> Core {
        let buses = BusBank::new(&bus_graph);
        Core {
            seed,
            slots: Vec::new(),
            outputs: Vec::new(),
            orbit_of: Vec::new(),
            next_seq: 0,
            bus_graph,
            buses,
            mix: vec![[0.0f32; QUANTUM]; orbits.count()],
            orbits,
            tables: WaveTables::new(),
            noise: NoiseCache::new(seed),
            ir_cache: IrCache::new(),
            occurrence: Occurrence::new(),
            #[cfg(test)]
            seeds: Vec::new(),
            frame: 0,
            mix_overflowed: false,
            errors: Vec::new(),
            alive: Vec::new(),
        }
    }

    /// Appends one value to the bus graph (recomputing the levels; a voice already playing
    /// moves to its value's new level from the next quantum) and grows the bus bank to match.
    /// Returns the value's index. The streaming driver's entry point: offline builds the whole
    /// graph up front with `BusGraph::from_job`.
    pub(crate) fn add_value(&mut self, map: &ValueMap) -> usize {
        let index = self.bus_graph.push_value(map);
        self.buses.grow(self.bus_graph.keys());
        index
    }

    /// `OrbitBank::create`: a no-op when the number exists, else appended (first-add order).
    pub(crate) fn create_orbit(&mut self, spec: &OrbitSpec) {
        self.orbits.create(spec);
    }

    /// Drops every voice: clears the slots, their outputs and their orbit map together. Never
    /// resets the sequence counter, and never touches the orbits, the IR and noise caches,
    /// `occurrence`, the wave tables, the bus graph and buses, the frame, the mix or the errors
    /// (`alive` is rebuilt by every step). Effect tails on the orbits ring on.
    pub(crate) fn stop_voices(&mut self) {
        self.slots.clear();
        self.outputs.clear();
        self.orbit_of.clear();
    }

    /// Changes the audio seed: resets the noise and IR caches and returns true; the same seed
    /// is a no-op returning false.
    pub(crate) fn set_seed(&mut self, seed: i32) -> bool {
        if seed == self.seed {
            return false;
        }
        self.seed = seed;
        self.noise = NoiseCache::new(seed);
        self.ir_cache = IrCache::new();
        true
    }

    /// The next frame to render (always a multiple of the quantum).
    pub(crate) fn frame(&self) -> u64 {
        self.frame
    }

    /// The last quantum's mix, one buffer per destination channel.
    pub(crate) fn mix(&self) -> &[[f32; QUANTUM]] {
        &self.mix
    }

    /// The bus graph (levels, cyclic readers).
    pub(crate) fn bus_graph(&self) -> &BusGraph {
        &self.bus_graph
    }

    /// A driver's own error (offline: a late hap), in order with the core's.
    pub(crate) fn push_error(&mut self, e: RenderError) {
        self.errors.push(e);
    }

    /// Live: drains every error (the caller takes at most 32 and drops the rest).
    pub(crate) fn drain_errors(&mut self) -> std::vec::Drain<'_, RenderError> {
        self.errors.drain(..)
    }

    /// Offline: the errors and whether the mix overflowed.
    pub(crate) fn finish(self) -> (Vec<RenderError>, bool) {
        (self.errors, self.mix_overflowed)
    }

    /// The orbits (tests).
    #[cfg(test)]
    pub(crate) fn orbits(&self) -> &OrbitBank {
        &self.orbits
    }

    /// Schedules one hap: the duck, superdough's mutation, the voice seed, the voice build.
    pub(crate) fn schedule(&mut self, p: ScheduleArgs<'_>) {
        let ScheduleArgs {
            value,
            value_index,
            t,
            duration,
            cycle,
            phase_sec,
            cps,
            now_frame,
        } = p;
        let bar = Some(bar_of(cycle));
        // `audioController.duck` runs after `getOrbit` (the hap's own orbit must exist; a
        // missing one is reported below, as before) and before the silence return, so silence
        // markers duck too. Ducks run in job order, never sorted: a later call with an earlier
        // `t` cancels what earlier calls scheduled after it. A throw is the hap's error and the
        // hap builds no voice; what the duck scheduled before it stays. A target orbit that does
        // not exist is superdough's error log (no bar, kind `other`, the render page's text),
        // reported once per distinct message as the harness dedupes them.
        if let Some(controls) = DuckControls::of(value)
            && self.orbits.find(value).is_ok()
        {
            let mut missing = Vec::new();
            let ducked = self.orbits.duck(&controls, t, now_frame, &mut missing);
            for target in missing {
                self.push_log(missing_duck_target(&target));
            }
            if let Err(e) = ducked {
                let tag = HapTag::new(bar, t, value);
                self.errors.push(tag.error(ErrorKind::Hap, &e.to_string()));
                return;
            }
        }
        // Superdough's mutation (`duration`, the silence return, the `bank` rewrite), then
        // `voiceSeed` on the mutated value: a silence marker returns before it, so it does not
        // advance `occurrence`.
        let Some(value) = prepare_value(value, duration) else {
            return;
        };
        let seed = voice_seed(cycle, &value, self.seed, &mut self.occurrence);
        #[cfg(test)]
        self.seeds.push(seed);
        // The tag of the mutated value (its `s` carries the `bank` prefix).
        let tag = HapTag::new(bar, t, &value);
        let orbit = match self.orbits.find(&value) {
            Ok(k) => k,
            Err(detail) => {
                self.errors.push(tag.error(ErrorKind::Hap, &detail));
                return;
            }
        };
        let built = parse_voice_params(&value, duration, cps)
            .map_err(Into::into)
            .and_then(|params| {
                Voice::build(
                    &params,
                    seed,
                    StartInfo {
                        t,
                        hap_duration: duration,
                        now_frame,
                        cycle,
                        phase_sec: phase_sec.unwrap_or_else(|| cycle / cps),
                        cps,
                    },
                    &mut self.tables,
                    &mut self.noise,
                    &mut self.orbits.orbits_mut()[orbit],
                    &self.buses,
                )
            });
        match built {
            Ok((mut v, late)) => {
                // Superdough's error logs of the modulators it skipped (kind `other`, no bar,
                // once per distinct message as the harness dedupes them).
                for msg in v.take_logs() {
                    self.push_log(msg);
                }
                // A build that threw after a send was connected keeps the voice playing into
                // its sends (unconnected to the summing node) and reports the error; a throw in
                // the modulator loop keeps the voice and the modulators before it.
                if let Some(e) = late {
                    self.errors.push(tag.error(e.kind, &e.detail));
                }
                self.slots.push(Slot {
                    voice: v,
                    tag,
                    orbit,
                    value: value_index,
                    seq: self.next_seq,
                    unsettled: true,
                });
                self.next_seq += 1;
            }
            Err(e) => self.errors.push(tag.error(e.kind, &e.detail)),
        }
    }

    /// Reports one of superdough's error logs as the render page captures it: kind `other`, no
    /// bar, the text cut to the page's 300 characters, once per distinct message.
    fn push_log(&mut self, msg: String) {
        let msg: String = msg.chars().take(MAX_LOG_CHARS).collect();
        if !self
            .errors
            .iter()
            .any(|e| e.kind == ErrorKind::Other && e.message == msg)
        {
            self.errors
                .push(RenderError::new(ErrorKind::Other, msg, None));
        }
    }

    /// Renders the quantum at `frame` into `mix` and advances `frame`. The voices render level
    /// by level (level 0 first, in creation order within a level, each value's level read from
    /// the graph as the quantum starts), each voice's bus send added to the bank right after it,
    /// so a bus is complete before its first reader; then the orbits sum the voices in vector
    /// order. Without buses there is one level: the voices render in vector order.
    pub(crate) fn step(&mut self) {
        self.step_with_stems(None);
    }

    fn step_with_stems(&mut self, mut capture: Option<&mut StemCapture>) {
        let q = self.frame;
        let n_voices = self.slots.len();
        if self.outputs.len() < n_voices {
            self.outputs.resize_with(n_voices, VoiceOutput::new);
        }
        self.alive.clear();
        self.alive.resize(n_voices, true);
        self.buses.begin_quantum();
        for level in 0..=self.bus_graph.max_level() {
            for (i, slot) in self.slots.iter_mut().enumerate() {
                if self.bus_graph.level_of_value(slot.value) != level {
                    continue;
                }
                let v = &mut slot.voice;
                self.alive[i] = v.process(q, &mut self.outputs[i], &mut self.buses);
                for detail in v.take_errors() {
                    self.errors.push(slot.tag.error(ErrorKind::Hap, &detail));
                }
                if v.failed() {
                    let detail = "non-finite output (Chromium renders NaN or Infinity)";
                    self.errors.push(slot.tag.error(ErrorKind::Hap, detail));
                }
                if let Some((bus, send)) = &self.outputs[i].sends.bus {
                    self.buses.add_send(*bus, slot.seq, send);
                }
            }
        }
        self.orbit_of.clear();
        self.orbit_of.extend(self.slots.iter().map(|s| s.orbit));
        self.mix.resize(self.orbits.count(), [0.0; QUANTUM]);
        if let Some(c) = capture.as_deref_mut() {
            c.begin(&self.slots);
        }
        self.orbits.process_with_stems(
            q,
            &self.outputs[..n_voices],
            &self.orbit_of,
            &mut self.mix,
            capture,
        );
        // A voice renders its last quantum into the mix, then is freed (order kept).
        if self.alive.contains(&false) {
            retain_flagged(&mut self.slots, &self.alive);
        }
        // Deviation: a mix that overflows the float range is written as silence and
        // reported once (Chromium writes the infinities).
        if self
            .mix
            .iter()
            .flat_map(|c| c.iter())
            .any(|x| !x.is_finite())
        {
            for c in self.mix.iter_mut() {
                for x in c.iter_mut().filter(|x| !x.is_finite()) {
                    *x = 0.0;
                }
            }
            if !self.mix_overflowed {
                self.mix_overflowed = true;
                let msg = format!("non-finite mix at frame {q} (Chromium writes it)");
                self.errors
                    .push(RenderError::new(ErrorKind::Other, msg, None));
            }
        }
        self.frame += Q;
    }
}

/// The offline driver for one job: the job's pause loop over a [`Core`], copying each quantum's
/// mix into the job-length channels.
pub struct Engine<'a> {
    job: &'a EngineJob,
    channels: Vec<Vec<f32>>,
    core: Core,
    capture: Option<StemCapture>,
}

pub(crate) fn zeroed(len: usize) -> Result<Vec<f32>, JobError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| JobError::new(format!("cannot allocate {len} frames")))?;
    v.resize(len, 0.0);
    Ok(v)
}

impl<'a> Engine<'a> {
    /// Validates the job and allocates its output.
    pub fn new(job: &'a EngineJob) -> Result<Self, JobError> {
        job.validate()?;
        let frames = usize::try_from(job.frames).map_err(|_| JobError::new("frames: too large"))?;
        let channels = (0..job.channels.count)
            .map(|_| zeroed(frames))
            .collect::<Result<_, _>>()?;
        Ok(Engine {
            job,
            channels,
            core: Core::new(job.seed, BusGraph::from_job(job), OrbitBank::new(job)),
            capture: None,
        })
    }

    pub(crate) fn with_stems(job: &'a EngineJob, layout: &StemLayout) -> Result<Self, JobError> {
        if job.channels.count != 2 || job.channels.multi_channel_orbits {
            return Err(JobError::new(
                "stem capture requires a standard stereo mix job",
            ));
        }
        let mut engine = Self::new(job)?;
        engine.capture = Some(StemCapture::new(job, layout)?);
        Ok(engine)
    }

    /// Renders the job.
    pub fn run(self) -> Rendered {
        self.run_observing_overflow().0
    }

    pub(crate) fn run_observing_overflow(mut self) -> (Rendered, bool) {
        let job = self.job;
        let unsupported = unsupported_signatures_in(job, self.core.bus_graph());
        let played = unsupported.is_empty();
        if played {
            self.play();
        }
        let Engine {
            mut channels,
            core,
            capture,
            ..
        } = self;
        // Nothing has rendered on the unsupported path, so the overflow flag is still false.
        let (errors, overflowed) = if played {
            core.finish()
        } else {
            let errors = unsupported
                .into_iter()
                .map(|u| {
                    let msg = format!("s={} keys={}", u.s, u.keys.join(","));
                    RenderError::new(ErrorKind::Unsupported, msg, Some(u.first_bar))
                })
                .collect();
            (errors, false)
        };
        if let Some(capture) = capture {
            channels.extend(capture.channels);
        }
        (Rendered { channels, errors }, overflowed)
    }

    /// The pause loop: render to each pause frame, schedule its haps, settle the IRs; then
    /// render to the end.
    fn play(&mut self) {
        let job = self.job;
        for pause in &job.pauses {
            let pause_frame = frame_for_pause(pause.suspend_sec);
            self.render_to(pause_frame);
            let first_new = self.core.slots.len();
            for hap in &pause.haps {
                self.schedule(hap, pause_frame);
            }
            // `settleIR()` before the context resumes: each orbit's newest IR request lands, then
            // the local reverbs of this pause's voices in creation order (voices built earlier
            // have nothing pending), all through the run's IR cache.
            let core = &mut self.core;
            core.orbits.settle(core.seed, &mut core.ir_cache);
            for slot in &mut core.slots[first_new..] {
                slot.voice
                    .settle_local_reverbs(core.seed, &mut core.ir_cache);
            }
        }
        self.render_to(job.frames);
    }

    /// The late test (`renderOffline`'s rule: a hap more than [`LATE_EPSILON`] before the pause
    /// is reported, and every earlier hap starts at the pause), then the core's schedule with the
    /// job's one cps.
    fn schedule(&mut self, hap: &crate::job::HapRef, pause_frame: u64) {
        let job = self.job;
        let Some(value) = job.values.get(hap.v as usize) else {
            return;
        };
        let now = pause_frame as f64 / SR;
        let mut t = hap.t;
        if t < now {
            if now - t > LATE_EPSILON {
                let detail = format!("scheduled late (now {now:.6})");
                let tag = HapTag::new(Some(bar_of(hap.cycle)), t, value);
                self.core.push_error(tag.error(ErrorKind::Hap, &detail));
            }
            t = now;
        }
        self.core.schedule(ScheduleArgs {
            value,
            value_index: hap.v as usize,
            t,
            duration: hap.duration,
            cycle: hap.cycle,
            phase_sec: hap.phase_sec,
            cps: hap.cps.unwrap_or(job.cps),
            now_frame: pause_frame,
        });
    }

    /// Renders whole quanta until `target` (or the job end), truncating the last one.
    fn render_to(&mut self, target: u64) {
        let end = round_up_128(target.min(self.job.frames));
        let frames = self.job.frames;
        while self.core.frame() < end {
            let q = self.core.frame();
            self.core.step_with_stems(self.capture.as_mut());
            let n = (frames.saturating_sub(q)).min(Q) as usize;
            let start = q as usize;
            for (ch, m) in self.channels.iter_mut().zip(self.core.mix().iter()) {
                if let Some(dst) = ch.get_mut(start..start + n) {
                    dst.copy_from_slice(&m[..n]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render as render_job;
    use serde_json::json;

    #[test]
    fn retain_flagged_keeps_the_flagged_items_in_order() {
        let mut v = vec![1, 2, 3, 4, 5];
        retain_flagged(&mut v, &[true, false, true, false, true]);
        assert_eq!(v, [1, 3, 5]);
        retain_flagged(&mut v, &[false, false, false]);
        assert!(v.is_empty());
    }

    fn job(v: serde_json::Value) -> EngineJob {
        EngineJob::from_json(&v.to_string()).unwrap()
    }

    fn base(
        frames: u64,
        pauses: serde_json::Value,
        values: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "schema": 2, "sampleRate": 44100, "frames": frames, "seed": 0, "cps": 0.5,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": pauses, "values": values
        })
    }

    fn hap(t: f64, dur: f64, cycle: f64, v: u32) -> serde_json::Value {
        json!({ "t": t, "duration": dur, "cycle": cycle, "v": v, "event": 0 })
    }

    fn first_nonzero(x: &[f32]) -> Option<usize> {
        x.iter().position(|s| *s != 0.0)
    }

    #[test]
    fn frame_helpers() {
        assert_eq!(round_up_128(0), 0);
        assert_eq!(round_up_128(1), 128);
        assert_eq!(round_up_128(128), 128);
        assert_eq!(round_up_128(129), 256);
        assert_eq!(round_up_128(u64::MAX), u64::MAX);
        assert_eq!(frame_for_pause(0.0), 0);
        assert_eq!(frame_for_pause(129.9 / SR), 256, "129.9 -> 129 -> 256");
        assert_eq!(frame_for_pause(256.0 / SR), 256, "exact multiples stay");
        assert_eq!(frame_for_pause(127.9 / SR), 128);
        assert_eq!(
            frame_for_pause(1.95),
            86016,
            "1.95 s = 85995 frames -> 86016"
        );
        assert_eq!(frame_for_param_event(0.0), 0);
        assert_eq!(frame_for_param_event(1.0 / SR), 1);
        assert_eq!(frame_for_param_event(1.5 / SR), 2);
        assert_eq!(frame_for_buffer_start(0.0), 0);
        assert_eq!(frame_for_buffer_start(1.5 / SR), 2);
        assert_eq!(frame_for_buffer_start(10.0 / SR), 10);
        // Within 1/2048 of a frame of an integer rounds to it before the ceil.
        assert_eq!(frame_for_buffer_start((10.0 + 1.0 / 4096.0) / SR), 10);
        assert_eq!(frame_for_buffer_start((10.0 + 1.0 / 1000.0) / SR), 11);
        assert_eq!(
            frame_for_param_event((10.0 + 1.0 / 4096.0) / SR),
            11,
            "param events do not round"
        );
    }

    #[test]
    fn zero_hap_job_is_exact_silence() {
        let j = job(base(
            1000,
            json!([{ "suspendSec": 0, "haps": [] }]),
            json!([]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(r.channels.len(), 2);
        assert!(
            r.channels
                .iter()
                .all(|c| c.len() == 1000 && c.iter().all(|x| *x == 0.0))
        );
        assert!(r.errors.is_empty());
        let mut j6 = base(300, json!([{ "suspendSec": 0, "haps": [] }]), json!([]));
        j6["channels"]["count"] = json!(6);
        let r = render_job(&job(j6)).unwrap();
        assert_eq!(r.channels.len(), 6);
        assert!(r.channels.iter().all(|c| c.len() == 300));
        let r = render_job(&job(base(
            0,
            json!([{ "suspendSec": 0, "haps": [] }]),
            json!([]),
        )))
        .unwrap();
        assert!(r.channels.iter().all(Vec::is_empty));
    }

    #[test]
    fn bar_zero_is_scheduled_at_frame_zero_and_bar_j_at_its_pause() {
        let j = job(base(
            44100,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.05, 1.0, 0)] }
            ]),
            json!([{ "s": "sine", "note": 69 }]),
        ));
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let l = &r.channels[0];
        assert_eq!(l[0], 0.0, "sin(0)");
        assert_eq!(first_nonzero(l), Some(1));
        // The second hap starts at frame 13230 (0.3 s), after its pause frame 11136.
        let second = &l[11136..];
        let silent_until = 13230 - 11136;
        assert!(second[..silent_until].iter().all(|x| *x == 0.0));
        assert_eq!(first_nonzero(&second[silent_until..]), Some(1));
        assert_eq!(r.channels[0], r.channels[1], "mono into both mix channels");
    }

    #[test]
    fn late_haps_are_clamped_and_reported() {
        // The pause at 0.1 s quantizes to frame 4480 (0.10159 s): a hap at 0.1 s is late.
        let j = job(base(
            22050,
            json!([
                { "suspendSec": 0, "haps": [] },
                { "suspendSec": 0.1, "haps": [hap(0.1, 0.05, 3.0, 0), hap(4480.0 / SR - 1e-7, 0.05, 3.0, 0)] }
            ]),
            json!([{ "s": "sine", "note": 69 }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Hap,
                "hap 0.100s sine: scheduled late (now 0.101587)",
                Some(3)
            )],
            "within 1e-6 is not reported"
        );
        assert!(r.channels[0][..4480].iter().all(|x| *x == 0.0));
        assert_eq!(
            first_nonzero(&r.channels[0][4480..]),
            Some(1),
            "clamped to the pause frame"
        );
    }

    #[test]
    fn a_processor_error_renders_silence_and_is_reported_once() {
        // distorttype -1: getDistortionAlgorithm gives undefined, the first process() throws,
        // the worklet outputs zeros for good, as Chromium's does. A second voice still sounds.
        let j = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.1, 0.05, 0.0, 1)] }]),
            json!([
                { "s": "sawtooth", "distort": 2, "distorttype": -1 },
                { "s": "sawtooth", "distort": 2, "distorttype": 1.5 }
            ]),
        ));
        let r = render_job(&j).unwrap();
        assert!(r.channels.iter().all(|c| c.iter().all(|x| *x == 0.0)));
        assert_eq!(
            r.errors,
            vec![
                RenderError::new(
                    ErrorKind::Hap,
                    "hap 0.000s sawtooth: distort: processor error; Chromium outputs silence",
                    Some(0)
                ),
                RenderError::new(
                    ErrorKind::Hap,
                    "hap 0.100s sawtooth: distort: processor error; Chromium outputs silence",
                    Some(0)
                )
            ]
        );
        // A valid index sounds.
        let j = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sawtooth", "distort": 2, "distorttype": 10 }]),
        ));
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty());
        assert!(r.channels[0].iter().any(|x| *x != 0.0));
    }

    #[test]
    fn unsupported_jobs_render_silence_and_only_unsupported_errors() {
        let j = job(base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 2.0, 1), hap(0.0, 0.05, 0.0, 2)] }]),
            json!([{ "s": "sine" }, { "s": "bytebeat", "ir": 1 }, { "note": "zz" }]),
        ));
        let r = render_job(&j).unwrap();
        assert!(
            r.channels
                .iter()
                .all(|c| c.len() == 4410 && c.iter().all(|x| *x == 0.0))
        );
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Unsupported,
                "s=bytebeat keys=ir",
                Some(2)
            )]
        );
    }

    #[test]
    fn construction_errors_become_hap_errors_and_the_render_continues() {
        let j = job(base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 1.5, 0), hap(0.01, 0.05, 1.5, 1)] }]),
            json!([{ "s": "sine", "note": "zz" }, { "s": "sine", "note": 69 }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Hap,
                "hap 0.000s sine: not a note: \"zz\"",
                Some(1)
            )]
        );
        assert_eq!(
            first_nonzero(&r.channels[0]),
            Some(442),
            "the second hap still sounds"
        );
    }

    #[test]
    fn a_local_reverb_build_throw_leaves_the_voice_silent_with_one_hap_error() {
        // The entry's IR request throws (a lowpass `TypeError`): no voice, one hap error, and the
        // render goes on.
        let j = job(base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "FX": [{ "room": 0.5, "roomlp": "x" }] }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].kind, ErrorKind::Hap);
        assert!(r.errors[0].message.starts_with("hap 0.000s sine: "));
        assert!(r.channels.iter().flatten().all(|x| *x == 0.0));
    }

    #[test]
    fn local_reverbs_settle_through_the_run_cache() {
        // Two pauses, each with a voice whose entry builds an equal local reverb, and an orbit
        // reverb with the same settings: one IR generation for the run; each local reverb is
        // settled by its own pause (it sounds from its onset).
        let j = job(base(
            22050,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] },
                { "suspendSec": 0.2, "haps": [hap(0.25, 0.05, 0.5, 0)] }
            ]),
            json!([
                { "s": "sine", "FXrelease": 0.1, "FX": [{ "room": 0.5, "roomsize": 0.05 }] },
                { "s": "sine", "gain": 0, "room": 0.5, "roomsize": 0.05 }
            ]),
        ));
        let mut e = Engine::new(&j).unwrap();
        e.play();
        assert!(e.core.errors.is_empty(), "{:?}", e.core.errors);
        assert_eq!(e.core.ir_cache.generated(), 1);
        assert_eq!(e.core.ir_cache.len(), 1);
        let ch = &e.channels[0];
        assert!(ch[11025..11025 + 128].iter().any(|x| *x != 0.0));
        let solo = job(base(
            22050,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "FXrelease": 0.1, "FX": [{ "room": 0.5, "roomsize": 0.05 }] }]),
        ));
        let r = render_job(&solo).unwrap();
        // The first voice sounds as it does alone (the second only feeds the orbit reverb at
        // gain 0, which stays silent) until the third hap starts.
        assert_eq!(
            r.channels[0][..11025]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            ch[..11025].iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn silence_markers_and_partial_last_quantum() {
        let j = job(base(
            1000,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] }]),
            json!([{ "s": "~" }, { "s": "one", "sustain": 1 }]),
        ));
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty());
        assert_eq!(r.channels[0].len(), 1000);
        assert!(
            (r.channels[0][999] - 0.24).abs() < 1e-6,
            "rendered to the last frame"
        );
    }

    #[test]
    fn mono_jobs_sum_both_orbit_channels_onto_channel_zero() {
        // `connectToDestination` connects the splitter's outputs to merger inputs
        // `channels[i] % 1`: both sides of the orbit land on the one channel.
        let mut j = base(
            500,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "sustain": 1 }]),
        );
        j["channels"]["count"] = json!(1);
        let r = render_job(&job(j)).unwrap();
        assert_eq!(r.channels.len(), 1);
        let x = SYNTH_ONE * 0.8;
        assert_eq!(r.channels[0][400], x + x);
    }

    #[test]
    fn repeat_renders_are_bit_identical() {
        let j = job(base(
            30000,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.2, 0.0, 0), hap(0.05, 0.1, 0.0, 1)] },
                { "suspendSec": 0.3, "haps": [hap(0.31, 0.2, 1.0, 1)] }
            ]),
            json!([{ "s": "sine", "note": 60, "attack": 0.02 }, { "s": "sine", "note": "e4", "gain": 0.5 }]),
        ));
        let a = render_job(&j).unwrap();
        let b = render_job(&j).unwrap();
        let bits = |r: &Rendered| {
            r.channels
                .iter()
                .flatten()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&a), bits(&b));
        assert!(a.channels[0].iter().all(|x| x.is_finite()));
    }

    #[test]
    fn voices_are_freed_after_a_silent_quantum() {
        let j = job(base(
            44100,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.1, 0.0, 0)] }]),
            json!([{ "s": "sine" }]),
        ));
        let mut e = Engine::new(&j).unwrap();
        e.schedule(&j.pauses[0].haps[0], 0);
        assert_eq!(e.core.slots.len(), 1);
        // Stop at 0.1 + 0.01 + 0.01 = 0.12 s = frame 5292; the quantum at 5376 is the first silent one.
        e.render_to(5376);
        assert_eq!(
            e.core.slots.len(),
            1,
            "not freed before a full silent quantum"
        );
        e.render_to(5376 + 128);
        assert_eq!(e.core.slots.len(), 0, "freed after it");
    }

    /// The seeds the driver computes for a job's haps, in call order.
    fn driver_seeds(j: &EngineJob) -> Vec<u32> {
        let mut e = Engine::new(j).unwrap();
        for p in &j.pauses {
            let f = frame_for_pause(p.suspend_sec);
            for h in &p.haps {
                e.schedule(h, f);
            }
        }
        e.core.seeds
    }

    /// `voice_seed` over explicit post-mutation maps, one fresh `Occurrence` for the list.
    fn expected_seeds(seed: i32, calls: &[(f64, serde_json::Value)]) -> Vec<u32> {
        let mut occ = crate::js::Occurrence::new();
        calls
            .iter()
            .map(|(cycle, v)| {
                let crate::job::Value::Obj(m) = crate::job::Value::from_json(v.clone()).unwrap()
                else {
                    panic!("not a map")
                };
                voice_seed(*cycle, &m, seed, &mut occ)
            })
            .collect()
    }

    #[test]
    fn occurrence_follows_call_order_across_pauses() {
        let mut j = base(
            44100,
            json!([
                { "suspendSec": 0, "haps": [
                    hap(0.0, 0.1, 0.0, 0), hap(0.0, 0.1, 0.0, 0), hap(0.0, 0.1, 0.0, 1),
                    hap(0.0, 0.2, 0.0, 0), hap(0.0, 0.1, 0.0, 0)
                ] },
                { "suspendSec": 0.25, "haps": [
                    hap(0.3, 0.1, 1.0, 0), hap(0.3, 0.1, 0.0, 0), hap(0.3, 0.1, 1.0, 0)
                ] }
            ]),
            json!([{ "s": "sine", "note": 60 }, { "s": "saw" }]),
        );
        j["seed"] = json!(-7);
        let a = |d: f64| json!({ "s": "sine", "note": 60, "duration": d });
        let want = expected_seeds(
            -7,
            &[
                (0.0, a(0.1)),
                (0.0, a(0.1)),
                (0.0, json!({ "s": "saw", "duration": 0.1 })),
                (0.0, a(0.2)),
                (0.0, a(0.1)),
                (1.0, a(0.1)),
                (0.0, a(0.1)),
                (1.0, a(0.1)),
            ],
        );
        let got = driver_seeds(&job(j));
        assert_eq!(got, want);
        // Occurrences 0, 1, then 2 after the other values; each cycle change resets.
        assert_ne!(got[0], got[1]);
        assert_ne!(got[1], got[4]);
        assert_eq!(got[6], got[0], "cycle 0 again after cycle 1: occurrence 0");
        assert_eq!(got[7], got[5], "interleaved: reset again");
    }

    #[test]
    fn occurrence_counts_post_mutation_values() {
        // Different job values that superdough's mutation makes identical share the counter:
        // `duration` is replaced by the hap duration, and `bank` without `s` becomes
        // `bank_triangle` exactly like an explicit `s: 'triangle'`.
        let j = job(base(
            44100,
            json!([{ "suspendSec": 0, "haps": [
                hap(0.0, 0.5, 2.0, 0), hap(0.0, 0.5, 2.0, 1),
                hap(0.0, 0.5, 2.0, 2), hap(0.0, 0.5, 2.0, 3),
                hap(0.0, 0.5, 2.0, 4)
            ] }]),
            json!([
                { "s": "sine" },
                { "s": "sine", "duration": 9 },
                { "bank": "b" },
                { "bank": "b", "s": "triangle" },
                { "bank": "", "s": "triangle" }
            ]),
        ));
        // (`render` refuses the job, `b_triangle` is not an engine sound; the driver's hap
        // scheduling still shows the seeds.)
        let got = driver_seeds(&j);
        let want = expected_seeds(
            0,
            &[
                (2.0, json!({ "s": "sine", "duration": 0.5 })),
                (2.0, json!({ "s": "sine", "duration": 0.5 })),
                (
                    2.0,
                    json!({ "bank": "b", "s": "b_triangle", "duration": 0.5 }),
                ),
                (
                    2.0,
                    json!({ "bank": "b", "s": "b_triangle", "duration": 0.5 }),
                ),
                (2.0, json!({ "bank": "", "s": "triangle", "duration": 0.5 })),
            ],
        );
        assert_eq!(got, want);
        assert_ne!(got[0], got[1], "occurrence 1 after the mutation");
        assert_ne!(got[2], got[3]);
    }

    #[test]
    fn silence_markers_do_not_advance_occurrence_or_the_cycle() {
        let j = job(base(
            44100,
            json!([{ "suspendSec": 0, "haps": [
                hap(0.0, 0.1, 0.0, 0), hap(0.0, 0.1, 0.0, 1), hap(0.0, 0.1, 5.0, 2),
                hap(0.0, 0.1, 0.0, 3), hap(0.0, 0.1, 0.0, 0)
            ] }]),
            json!([{ "s": "sine" }, { "s": "~" }, { "s": "-", "bank": "x" }, { "s": "_" }]),
        ));
        let got = driver_seeds(&j);
        let a = json!({ "s": "sine", "duration": 0.1 });
        assert_eq!(got, expected_seeds(0, &[(0.0, a.clone()), (0.0, a)]));
        assert_ne!(
            got[0], got[1],
            "occurrence 1: the marker at cycle 5 did not reset"
        );
    }

    #[test]
    fn elapsed_phase_affects_fixed_hz_while_synced_controls_follow_the_musical_onset() {
        let make = |control: &str, phase: f64, cps: f64| {
            let mut h = hap(0.0, 0.1, 1.0, 0);
            h["phaseSec"] = json!(phase);
            h["cps"] = json!(cps);
            let mut v = json!({ "s": "sine", "freq": 220, "tremolodepth": 1 });
            v[control] = json!(2.0);
            job(base(
                4096,
                json!([{ "suspendSec": 0, "haps": [h] }]),
                json!([v]),
            ))
        };
        for control in ["tremolo", "tremolosync"] {
            let first = make(control, 0.25, 1.25);
            let shifted = make(control, 0.375, 1.25);
            let new_rate = make(control, 0.25, 0.5);
            assert_eq!(driver_seeds(&first), driver_seeds(&shifted));
            assert_eq!(driver_seeds(&first), driver_seeds(&new_rate));
            let reference = render_job(&first).unwrap().channels;
            assert_eq!(
                reference == render_job(&shifted).unwrap().channels,
                control == "tremolosync"
            );
            assert_eq!(
                reference == render_job(&new_rate).unwrap().channels,
                control == "tremolo"
            );
        }
    }

    #[test]
    fn the_job_seed_reaches_the_voice() {
        let mut j = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.25, 0)] }]),
            json!([{ "s": "sine", "note": 69 }]),
        );
        j["seed"] = json!(2147483647);
        let j = job(j);
        let mut e = Engine::new(&j).unwrap();
        e.schedule(&j.pauses[0].haps[0], 0);
        let want = expected_seeds(
            2147483647,
            &[(0.25, json!({ "s": "sine", "note": 69, "duration": 0.05 }))],
        );
        assert_eq!(e.core.slots[0].voice.seed(), want[0]);
        assert_eq!(e.core.seeds, want);
    }

    #[test]
    fn the_job_seed_selects_the_noise_buffers_and_voices_share_them() {
        let render_seed = |seed: i32| {
            let mut j = base(
                4410,
                json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 0)] }]),
                json!([{ "s": "white", "sustain": 1 }]),
            );
            j["seed"] = json!(seed);
            let j = job(j);
            let mut e = Engine::new(&j).unwrap();
            e.schedule(&j.pauses[0].haps[0], 0);
            e.schedule(&j.pauses[0].haps[1], 0);
            assert!(e.core.noise.is_cached(crate::noise::NoiseKind::White));
            render_job(&j).unwrap().channels[0].clone()
        };
        let a = render_seed(0);
        let b = render_seed(1);
        assert_ne!(a, b);
        // Two identical voices on one buffer: exactly twice one voice.
        let want = crate::noise::noise_buffer(crate::noise::NoiseKind::White, 0, 0.02, 0);
        let one = f64::from(want[1000]) * f64::from(SYNTH_ONE) * 0.8;
        assert!((f64::from(a[1000]) - 2.0 * one).abs() < 1e-6);
    }

    #[test]
    fn panned_voices_mix_in_stereo_and_mono_voices_on_both_channels() {
        let v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] }]),
            json!([{ "s": "one", "sustain": 1, "pan": 0 }, { "s": "one", "sustain": 1, "gain": 0.5 }]),
        );
        let r = render_job(&job(v)).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        // Hard-left voice (0.24) plus the centred mono voice (0.15) on both sides; the orbit's
        // pan-0 panner adds 0.15 * cos(pi/2) to the left, below its float resolution.
        let mono = SYNTH_ONE * 0.5;
        assert_eq!(r.channels[0][1000], SYNTH_ONE * 0.8 + mono);
        assert_eq!(r.channels[1][1000], 0.0 + mono);
        // A mono job with orbit channels [0] keeps the orbit's left: the voice's own left
        // (x * cos(pi/2)) plus the pan-0 panner's leak of its right.
        let mut v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "sustain": 1, "pan": 1 }]),
        );
        v["channels"]["count"] = json!(1);
        v["orbits"] = json!([{ "orbit": 1, "channels": [0] }]);
        let r = render_job(&job(v)).unwrap();
        assert_eq!(r.channels.len(), 1);
        let x = SYNTH_ONE * 0.8;
        let leak = libm::cos(std::f64::consts::FRAC_PI_2) as f32;
        assert_eq!(r.channels[0][1000], x.mul_add(leak, x * leak));
    }

    #[test]
    fn stems_route_each_orbit_to_its_pair_and_alias_past_the_pairs() {
        // Two pairs (4 channels): orbit 1 -> 0/1, orbit 2 -> 2/3, orbit 3 wraps onto 0/1.
        let mut v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [
                hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1), hap(0.0, 0.05, 0.0, 2)
            ] }]),
            json!([
                { "s": "one", "sustain": 1, "orbit": 1 },
                { "s": "one", "sustain": 1, "gain": 0.5, "orbit": 2 },
                { "s": "one", "sustain": 1, "gain": 0.25, "orbit": 3 }
            ]),
        );
        v["channels"] = json!({ "count": 4, "multiChannelOrbits": true });
        v["orbits"] = json!([
            { "orbit": 1, "channels": [0, 1] },
            { "orbit": 2, "channels": [2, 3] },
            { "orbit": 3, "channels": [4, 5] }
        ]);
        let r = render_job(&job(v)).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.channels.len(), 4);
        let (a, b, c) = (SYNTH_ONE * 0.8, SYNTH_ONE * 0.5, SYNTH_ONE * 0.25);
        for ch in [0, 1] {
            assert_eq!(r.channels[ch][1000], a + c, "orbits 1 and 3 share pair 0");
        }
        for ch in [2, 3] {
            assert_eq!(r.channels[ch][1000], b);
        }
    }

    #[test]
    fn an_orbit_with_no_haps_still_exists_silently_and_unknown_orbits_are_hap_errors() {
        let mut v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 2.0, 0), hap(0.0, 0.05, 2.0, 1)] }]),
            json!([{ "s": "one", "sustain": 1, "orbit": 2 }, { "s": "one", "sustain": 1, "orbit": 7 }]),
        );
        v["channels"] = json!({ "count": 4, "multiChannelOrbits": true });
        v["orbits"] =
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [2, 3] }]);
        let j = job(v);
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Hap,
                "hap 0.000s one: orbit 7 not in the job",
                Some(2)
            )]
        );
        assert!(
            r.channels[0]
                .iter()
                .chain(&r.channels[1])
                .all(|x| *x == 0.0)
        );
        assert_eq!(r.channels[2][1000], SYNTH_ONE * 0.8);
        // The skipped hap still advanced `occurrence` (its seed was computed first).
        assert_eq!(driver_seeds(&j).len(), 2);
    }

    #[test]
    fn explicit_channels_route_the_orbits_left_only() {
        let mut v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "sustain": 1, "channels": 2 }]),
        );
        v["orbits"] = json!([{ "orbit": 1, "channels": [1] }]);
        let r = render_job(&job(v)).unwrap();
        assert!(r.channels[0].iter().all(|x| *x == 0.0));
        assert_eq!(r.channels[1][1000], SYNTH_ONE * 0.8);
    }

    /// `one` through `Gain(0.3)`: the level of a held `one` before the gain stage.
    const SYNTH_ONE: f32 = 0.3;

    #[test]
    fn non_finite_voices_and_mixes_are_silenced_and_reported() {
        let v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] }]),
            json!([
                { "s": "sawtooth", "cutoff": 1000, "resonance": -1e6 },
                { "s": "one", "sustain": 1 }
            ]),
        );
        let r = render_job(&job(v)).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].kind, ErrorKind::Hap);
        assert!(
            r.errors[0]
                .message
                .starts_with("hap 0.000s sawtooth: non-finite output")
        );
        assert_eq!(r.errors[0].bar, Some(0));
        // The other voice still plays.
        assert_eq!(r.channels[0][1000], SYNTH_ONE * 0.8);
        assert!(r.channels.iter().flatten().all(|x| x.is_finite()));
        // Two finite voices whose sum overflows: the mix sample is zeroed, reported once.
        let v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "sustain": 1, "gain": 3e38, "postgain": 3 }]),
        );
        let r = render_job(&job(v)).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].kind, ErrorKind::Other);
        assert!(r.errors[0].message.starts_with("non-finite mix at frame 0"));
        assert!(r.channels.iter().flatten().all(|x| x.is_finite()));
    }

    #[test]
    fn delay_sends_echo_on_their_orbit_and_getdelay_errors_fail_the_hap() {
        // `one` (0.3 * 0.8 gain stage) for 0.05 s with a 0.01 s (441-frame) delay at level 1,
        // feedback 0.5, dry 0: the mix is only the delay's return.
        let v = base(
            8192,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "sustain": 1, "dry": 0, "delay": 1, "delaytime": 0.01, "delayfeedback": 0.5 }]),
        );
        let r = render_job(&job(v)).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let l = &r.channels[0];
        assert_eq!(first_nonzero(l), Some(1 + 441));
        // Steady state: x + 0.5 x after the first loop (441 + 128 frames).
        assert_eq!(l[1000], SYNTH_ONE * 0.8);
        assert_eq!(l[1500], SYNTH_ONE * 0.8 + (SYNTH_ONE * 0.8) * 0.5);
        assert_eq!(r.channels[0], r.channels[1]);
        // A delay time that overflows a float throws in getDelay: hap error, no sound.
        let v = base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "delay": 1, "delaytime": 1e39 }]),
        );
        let r = render_job(&job(v)).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].kind, ErrorKind::Hap);
        assert!(
            r.errors[0].message.contains("TypeError"),
            "{}",
            r.errors[0].message
        );
        assert!(r.channels.iter().flatten().all(|x| *x == 0.0));
    }

    #[test]
    fn the_djf_slot_persists_and_a_djf_throw_is_a_late_error() {
        // Pause 0: a held `one` without djf. Pause 0.1 s (frame 4480): a silent carrier with djf
        // 0.2 inserts the lowpass from that pause frame. Pause 0.19 s: a hap without djf on the
        // orbit is still filtered at the last value.
        let v = base(
            13230,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.3, 0.0, 0)] },
                { "suspendSec": 0.1, "haps": [hap(0.12, 0.05, 1.0, 1)] },
                { "suspendSec": 0.19, "haps": [hap(0.2, 0.05, 2.0, 2)] }
            ]),
            json!([
                { "s": "one", "attack": 0, "sustain": 1 },
                { "s": "one", "gain": 0, "djf": 0.2 },
                { "s": "sine", "note": 90 }
            ]),
        );
        let j = job(v);
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let l = &r.channels[1];
        // Before the pause frame: the constant passes; from it on the lowpass starts from rest.
        let dc = l[4479];
        assert!(dc > 0.1, "{dc}");
        assert!(
            l[4480] < dc * 0.2,
            "filtered from the pause frame: {}",
            l[4480]
        );
        let mut e = Engine::new(&j).unwrap();
        e.play();
        assert!(
            e.core.orbits.orbits()[0].djf().is_some(),
            "the slot never leaves"
        );
        // Unfiltered, the 90-note sine would swing the right channel by about +-0.24 around
        // the held constant; through a 375 Hz lowpass it barely moves.
        let tail = &l[9000..12000];
        let (lo, hi) = tail
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
        assert!(hi - lo < 0.05, "the later hap is filtered: {lo}..{hi}");

        // A non-finite djf throws after the sends: with a delay send connected the voice is kept
        // (heard through its echo) and the error reported; without a send, the hap is lost.
        let v = base(
            13230,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.1, 0.05, 0.0, 1)] }]),
            json!([
                { "s": "sine", "djf": { "$num": "NaN" }, "delay": 0.5, "delaytime": 0.1 },
                { "s": "sine", "djf": { "$num": "Infinity" } }
            ]),
        );
        let j = job(v);
        let r = render_job(&j).unwrap();
        assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
        assert!(
            r.errors
                .iter()
                .all(|e| e.kind == ErrorKind::Hap && e.message.contains("TypeError"))
        );
        let l = &r.channels[0];
        assert!(
            l[..4410].iter().all(|x| *x == 0.0),
            "no dry path, no second hap"
        );
        assert!(l[4410..8820].iter().any(|x| *x != 0.0), "the echo");
        let mut e = Engine::new(&j).unwrap();
        e.play();
        assert!(e.core.orbits.orbits()[0].djf().is_none(), "no slot");
    }

    #[test]
    fn reverb_requests_settle_at_each_pause() {
        let j = job(base(
            44100,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.05, 1.0, 2), hap(0.3, 0.05, 1.0, 0)] }
            ]),
            json!([
                { "s": "sine", "room": 0.5, "roomsize": 0.01, "roomlp": 0 },
                { "s": "sine", "room": 0.5, "roomsize": 0.02, "roomlp": 0 },
                { "s": "sine", "room": 0.5, "roomsize": 0.03 }
            ]),
        ));
        let mut e = Engine::new(&j).unwrap();
        let p0 = &j.pauses[0];
        for h in &p0.haps {
            e.schedule(h, 0);
        }
        // Before the pause settles: two requests, no IR.
        let r = e.core.orbits.orbits()[0].reverb().unwrap();
        assert_eq!((r.latest(), r.applied(), r.ir().is_none()), (2, 0, true));
        e.core.orbits.settle(j.seed, &mut e.core.ir_cache);
        let r = e.core.orbits.orbits()[0].reverb().unwrap();
        assert_eq!(r.applied(), 2, "the newest request of the pause");
        assert_eq!(r.ir().unwrap()[0].len(), 1323);
        // The whole run: in the second pause roomsize 0.03 (with the default lowpass, roomlp
        // being undefined) and then 0.01 (roomlp 0) are requested; the last one lands.
        let mut e = Engine::new(&j).unwrap();
        e.play();
        let r = e.core.orbits.orbits()[0].reverb().unwrap();
        assert_eq!((r.latest(), r.applied()), (4, 4));
        assert_eq!(r.settings().duration, crate::job::Value::Num(0.01));
        assert_eq!(r.ir().unwrap()[0].len(), 662);
        assert!(e.core.errors.is_empty(), "{:?}", e.core.errors);
        // The room sends play through the convolver: the render differs from the dry one from
        // the first frame of sound, and rings on after the dry notes end.
        let dry = j
            .values
            .iter()
            .map(|_| json!({ "s": "sine" }))
            .collect::<Vec<_>>();
        let without = job(base(
            44100,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 0.0, 1)] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.05, 1.0, 2), hap(0.3, 0.05, 1.0, 0)] }
            ]),
            json!(dry),
        ));
        let wet = render_job(&j).unwrap().channels;
        let dry = render_job(&without).unwrap().channels;
        assert_eq!(first_nonzero(&wet[0]), first_nonzero(&dry[0]));
        assert_ne!(wet, dry);
        // The second pair ends (envelope end ~0.36 s); the reverb (roomsize 0.01: 662 frames)
        // keeps sounding a little after the dry render is silent.
        let last = |c: &[f32]| c.iter().rposition(|x| *x != 0.0).unwrap();
        assert!(last(&wet[0]) > last(&dry[0]));
    }

    #[test]
    fn an_ir_swap_lands_at_the_pause_frame() {
        // One long note with a reverb send in bar 0; at the pause of bar 1 (0.2 s -> frame 8832)
        // a silent-by-gain hap requests a new IR. Up to the pause frame the render equals the
        // one without that request; from the pause frame on it differs (the old tail is cut).
        let pauses = json!([
            { "suspendSec": 0, "haps": [hap(0.0, 0.3, 0.0, 0)] },
            { "suspendSec": 0.2, "haps": [hap(0.25, 0.05, 1.0, 1)] }
        ]);
        let values = |second_size: f64| {
            json!([
                { "s": "sawtooth", "room": 0.8, "roomsize": 0.1, "roomlp": 0 },
                { "s": "sine", "gain": 0, "room": 0.5, "roomsize": second_size, "roomlp": 0 }
            ])
        };
        let same = render_job(&job(base(17640, pauses.clone(), values(0.1)))).unwrap();
        let swapped = render_job(&job(base(17640, pauses, values(0.05)))).unwrap();
        assert!(same.errors.is_empty() && swapped.errors.is_empty());
        let f = frame_for_pause(0.2) as usize;
        assert_eq!(f, 8832);
        for (a, b) in same.channels.iter().zip(swapped.channels.iter()) {
            assert_eq!(a[..f], b[..f], "identical before the swap");
            let first_diff = a.iter().zip(b.iter()).position(|(x, y)| x != y);
            assert_eq!(
                first_diff,
                Some(f),
                "the new convolver starts at the pause frame"
            );
        }
    }

    #[test]
    fn an_ir_error_is_reported_and_the_voice_not_connected() {
        let j = job(base(
            44100,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.05, 1.0, 1), hap(0.3, 0.05, 1.0, 2)] }
            ]),
            json!([
                { "s": "sine", "room": 0.5, "roomsize": 0.01 },
                { "s": "sine", "note": 72, "room": 0.5, "roomsize": 0 },
                { "s": "sine", "delay": 0.5, "delaytime": 0.1, "delayfeedback": 0.5,
                  "room": 0.5, "roomsize": { "$num": "NaN" } }
            ]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![
                RenderError::new(
                    ErrorKind::Ir,
                    "hap 0.300s sine: reverb ir length 0",
                    Some(1)
                ),
                RenderError::new(
                    ErrorKind::Ir,
                    "hap 0.300s sine: reverb ir length NaN",
                    Some(1)
                ),
            ]
        );
        // The first hap plays; the second is lost; the third sounds only through its delay
        // (0.1 s later).
        let l = &r.channels[0];
        assert!(l[..2205].iter().any(|x| *x != 0.0));
        let t = 13230;
        assert!(
            l[t..t + 4410].iter().all(|x| *x == 0.0),
            "no dry path at 0.3 s"
        );
        assert!(
            l[t + 4410..t + 4410 + 2205].iter().any(|x| *x != 0.0),
            "the echo"
        );
        let mut e = Engine::new(&j).unwrap();
        e.play();
        let rv = e.core.orbits.orbits()[0].reverb().unwrap();
        assert_eq!((rv.latest(), rv.applied()), (3, 1), "the first IR is kept");
        assert!(matches!(rv.settings().duration, crate::job::Value::Num(x) if x.is_nan()));
    }

    /// `base` with orbits 1 and 2.
    fn two_orbits(frames: u64, pauses: serde_json::Value, values: serde_json::Value) -> EngineJob {
        let mut v = base(frames, pauses, values);
        v["orbits"] =
            json!([{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }]);
        job(v)
    }

    /// An engine that scheduled every hap of `j` at its pause frame, without rendering.
    fn scheduled(j: &EngineJob) -> Engine<'_> {
        let mut e = Engine::new(j).unwrap();
        for p in &j.pauses {
            let f = frame_for_pause(p.suspend_sec);
            for h in &p.haps {
                e.schedule(h, f);
            }
        }
        e
    }

    fn duck_curve_points(e: &Engine<'_>, k: usize) -> Vec<(f64, f64)> {
        e.core.orbits.orbits()[k]
            .duck_curve()
            .map(|c| c.points.iter().map(|p| (p.time, p.value)).collect())
            .unwrap_or_default()
    }

    #[test]
    fn duck_ducks_the_target_orbit_from_the_pause_frame() {
        // A held `one` on orbit 2; a silent carrier on orbit 1 ducks it at 0.1 s from a pause at
        // frame 2048: the gain falls from 1 there.
        let j = two_orbits(
            13230,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.3, 0.0, 0)] },
                { "suspendSec": 2048.0 / 44100.0, "haps": [hap(0.1, 0.05, 1.0, 1)] }
            ]),
            json!([
                { "s": "one", "attack": 0, "sustain": 1, "orbit": 2 },
                { "s": "one", "gain": 0, "duckorbit": 2, "duckdepth": 0.75, "duckattack": 0.1 }
            ]),
        );
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let l = &r.channels[1];
        let dc = l[2047];
        assert!(dc > 0.1);
        assert_eq!(l[2048], dc, "1 at the pause frame");
        assert!(l[2049] < dc, "falling from the pause frame");
        let at = (0.1 * SR) as usize;
        let ducked = (1.0 - 0.75f64.sqrt()) as f32;
        assert!(
            (l[at] / dc - ducked).abs() < 1e-6,
            "{} vs {ducked}",
            l[at] / dc
        );
        assert_eq!(l[(0.2 * SR) as usize + 2], dc, "recovered");
    }

    #[test]
    fn duck_runs_before_silence_marker_return() {
        // A `~` hap with duckorbit ducks orbit 2, builds no voice and leaves `occurrence` alone.
        let j = two_orbits(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.05, 0.05, 0.0, 0), hap(0.06, 0.05, 0.0, 1)] }]),
            json!([
                { "s": "~", "duckorbit": 2 },
                { "s": "sine", "orbit": 2 }
            ]),
        );
        let e = scheduled(&j);
        assert_eq!(duck_curve_points(&e, 1).len(), 3, "ducked");
        assert_eq!(e.core.slots.len(), 1, "only the sine");
        assert_eq!(
            e.core.seeds,
            expected_seeds(
                0,
                &[(0.0, json!({ "s": "sine", "orbit": 2, "duration": 0.05 }))]
            ),
            "the marker does not advance occurrence"
        );
        assert!(e.core.errors.is_empty(), "{:?}", e.core.errors);
    }

    #[test]
    fn duck_in_job_order_not_time_order() {
        // One pause, the later-called hap has the earlier t: its cancel at 0.25 s removes the
        // first duck's ramps (after 0.25 s) and the curve keeps only the later duck.
        let j = two_orbits(
            22050,
            json!([{ "suspendSec": 0, "haps": [hap(0.375, 0.05, 0.75, 0), hap(0.25, 0.05, 0.5, 1)] }]),
            json!([
                { "s": "one", "gain": 0, "duckorbit": 2, "duckdepth": 0.9, "duckattack": 0.2 },
                { "s": "one", "gain": 0, "duckorbit": 2, "duckdepth": 0.5, "duckonset": 0.05, "duckattack": 0.1 }
            ]),
        );
        let e = scheduled(&j);
        let g = e.core.orbits.orbits()[1].output_gain();
        let kinds: Vec<_> = g.events().iter().map(|e| (e.kind, e.time)).collect();
        use crate::param::RampKind::*;
        assert_eq!(
            kinds,
            vec![
                (SetValue, 0.0),
                (CancelValues, 0.25),
                (Exponential, 0.3),
                (Exponential, 0.4)
            ],
            "the first duck's ramps are gone"
        );
        let pts = duck_curve_points(&e, 1);
        assert_eq!(
            pts.iter().map(|p| p.0).collect::<Vec<_>>(),
            vec![0.25, 0.3, 0.4]
        );
        // The same haps in time order keep both ducks.
        let j = two_orbits(
            22050,
            json!([{ "suspendSec": 0, "haps": [hap(0.25, 0.05, 0.5, 1), hap(0.375, 0.05, 0.75, 0)] }]),
            json!([
                { "s": "one", "gain": 0, "duckorbit": 2, "duckdepth": 0.9, "duckattack": 0.2 },
                { "s": "one", "gain": 0, "duckorbit": 2, "duckdepth": 0.5, "duckonset": 0.05, "duckattack": 0.1 }
            ]),
        );
        let e = scheduled(&j);
        let n = e.core.orbits.orbits()[1].output_gain().events().len();
        assert_eq!(n, 5, "set, both falls and both recoveries");
    }

    #[test]
    fn duck_skipped_when_own_orbit_missing() {
        // The hap's own orbit (5) is not in the job: no duck, and the existing hap error.
        let j = two_orbits(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.05, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "orbit": 5, "duckorbit": 2 }]),
        );
        let e = scheduled(&j);
        assert!(e.core.orbits.orbits()[1].duck_curve().is_none());
        assert_eq!(e.core.errors.len(), 1);
        assert!(
            e.core.errors[0].message.contains("orbit 5 not in the job"),
            "{:?}",
            e.core.errors
        );
    }

    #[test]
    fn a_duck_throw_is_a_hap_error_without_a_voice() {
        let j = two_orbits(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.05, 0.05, 0.0, 0), hap(0.06, 0.05, 0.0, 1)] }]),
            json!([
                { "s": "sine", "duckorbit": 2, "duckdepth": -1 },
                { "s": "sine", "duckorbit": 2, "duckattack": { "$num": "NaN" } }
            ]),
        );
        let e = scheduled(&j);
        assert!(e.core.slots.is_empty(), "neither hap builds a voice");
        assert_eq!(e.core.errors.len(), 2, "{:?}", e.core.errors);
        assert!(e.core.errors.iter().all(|x| x.kind == ErrorKind::Hap));
        assert!(
            e.core.errors[0]
                .message
                .contains("float value is non-finite"),
            "{}",
            e.core.errors[0].message
        );
        assert!(
            e.core.errors[1]
                .message
                .contains("double value is non-finite"),
            "{}",
            e.core.errors[1].message
        );
        // The second hap's first ramp stays scheduled: set, fall.
        assert_eq!(e.core.orbits.orbits()[1].output_gain().events().len(), 2);
        assert!(
            duck_curve_points(&e, 1).is_empty(),
            "the curve's points are unchanged"
        );
    }

    #[test]
    fn a_missing_duck_target_is_superdoughs_error_log_and_the_audio_is_unchanged() {
        // Orbit 7 is not in the job: superdough logs once per call (the page dedupes the text);
        // orbit 2 is still ducked and the voices sound as without the missing target.
        let values = |orbit: serde_json::Value, sine: serde_json::Value| {
            json!([
                { "s": "one", "attack": 0, "sustain": 1, "orbit": 2 },
                { "s": "one", "gain": 0, "duckorbit": orbit, "duckdepth": 0.75 },
                sine
            ])
        };
        let pauses = json!([{ "suspendSec": 0, "haps": [
            hap(0.0, 0.3, 0.0, 0), hap(0.05, 0.05, 0.0, 1), hap(0.1, 0.05, 0.0, 1),
            hap(0.15, 0.05, 0.0, 2)
        ] }]);
        let r = render_job(&two_orbits(
            13230,
            pauses.clone(),
            values(json!([7, 2]), json!({ "s": "sine", "duckorbit": [7, 9] })),
        ))
        .unwrap();
        let other = |target: &str| {
            RenderError::new(
                ErrorKind::Other,
                format!("[superdough] error: duck target orbit {target} does not exist"),
                None,
            )
        };
        assert_eq!(r.errors, vec![other("7"), other("9")], "no bar, deduped");
        let clean = render_job(&two_orbits(
            13230,
            pauses,
            values(json!(2), json!({ "s": "sine" })),
        ))
        .unwrap();
        assert!(clean.errors.is_empty(), "{:?}", clean.errors);
        assert_eq!(
            r.channels, clean.channels,
            "the miss is a no-op for the audio"
        );
        // A throw after a missing target: both are reported, the log first.
        let j = two_orbits(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.05, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "duckorbit": [7, 2], "duckdepth": [0.5, -1] }]),
        );
        let e = scheduled(&j);
        assert_eq!(e.core.errors.len(), 2, "{:?}", e.core.errors);
        assert_eq!(e.core.errors[0], other("7"));
        assert_eq!(e.core.errors[1].kind, ErrorKind::Hap);
        assert!(e.core.slots.is_empty());
    }

    fn lfos(configs: serde_json::Value) -> serde_json::Value {
        let n: Vec<usize> = (0..configs.as_array().unwrap().len()).collect();
        let mut m = json!({ "__ids": { "$set": n } });
        for (i, c) in configs.as_array().unwrap().iter().enumerate() {
            m[i.to_string()] = c.clone();
        }
        m
    }

    #[test]
    fn a_modulator_throw_keeps_the_voice_and_the_earlier_modulators() {
        let job_of = |lfo: serde_json::Value| {
            job(base(
                8820,
                json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.15, 0.0, 0)] }]),
                json!([{ "s": "sine", "gain": 0.5, "lfo": lfo }]),
            ))
        };
        let threw = render_job(&job_of(lfos(json!([
            { "control": "gain", "rate": 9 }, { "control": 5 }, { "control": "gain" }
        ]))))
        .unwrap();
        assert_eq!(threw.errors.len(), 1, "{:?}", threw.errors);
        assert_eq!(threw.errors[0].kind, ErrorKind::Hap);
        assert_eq!(threw.errors[0].bar, Some(0));
        let first = render_job(&job_of(lfos(json!([{ "control": "gain", "rate": 9 }])))).unwrap();
        assert!(first.errors.is_empty());
        assert_eq!(
            threw.channels, first.channels,
            "the voice sounds with lfo 0"
        );
        let plain = render_job(&job_of(lfos(json!([])))).unwrap();
        assert_ne!(plain.channels, first.channels, "the lfo is audible");
    }

    #[test]
    fn a_skipped_modulator_is_superdoughs_log_once() {
        let j = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.1, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "lfo": lfos(json!([{ "control": "dry" }])) }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Other,
                "[superdough] error: Could not connect to target 'dry' \u{2014} it does not exist. \
                 Available targets: source, gain, post"
                    .to_string(),
                None
            )]
        );
        let clean = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.1, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine" }]),
        ));
        assert_eq!(render_job(&clean).unwrap().channels, r.channels);
        // The page keeps the first 300 characters of a log.
        let long = "x".repeat(400);
        let j = job(base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "sine", "lfo": lfos(json!([{ "control": long }])) }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].message.chars().count(), 300);
    }

    /// FNV-1a 64 over every channel's sample bits, channel after channel.
    fn fnv(channels: &[Vec<f32>]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for c in channels {
            for x in c {
                for b in x.to_bits().to_le_bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        h
    }

    #[test]
    fn the_no_bus_loop_is_unchanged() {
        // A job without buses (filters, pan, delay, reverb on two orbits, noise, a worklet synth,
        // FM, a shaper, three pauses): one level in creation order, byte-identical to the
        // driver before buses (the hash is the stage-9 engine's render of this job).
        let j = job(json!({
            "schema": 2, "sampleRate": 44100, "frames": 33075, "seed": 7, "cps": 1,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }, { "orbit": 2, "channels": [0, 1] }],
            "pauses": [
                { "suspendSec": 0, "haps": [
                    hap(0.0, 0.2, 0.0, 0), hap(0.05, 0.1, 0.05, 1), hap(0.1, 0.3, 0.1, 2)
                ] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.2, 0.3, 3), hap(0.31, 0.2, 0.31, 0)] },
                { "suspendSec": 0.5, "haps": [hap(0.55, 0.1, 0.55, 4)] }
            ],
            "values": [
                { "s": "sawtooth", "note": 45, "cutoff": 900, "resonance": 5, "pan": 0.3,
                  "delay": 0.4, "delaytime": 0.05, "delayfeedback": 0.5 },
                { "s": "white", "gain": 0.3, "room": 0.5, "roomsize": 0.05, "orbit": 2 },
                { "s": "supersaw", "note": 57, "unison": 3, "gain": 0.5, "dry": 0.5, "room": 0.2,
                  "roomsize": 0.05 },
                { "s": "sine", "note": 69, "fmi": 2, "fmh": 1.5, "shape": 0.4, "orbit": 2 },
                { "s": "square", "note": 52, "hcutoff": 300, "ftype": "24db", "crush": 6,
                  "postgain": 0.7 }
            ]
        }));
        let e = Engine::new(&j).unwrap();
        assert_eq!(e.core.bus_graph.max_level(), 0);
        let r = render_job(&j).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(r.channels.iter().flatten().any(|&sample| sample != 0.0));
        let hash = fnv(&r.channels);

        // An unused bus adds a graph level without changing any scheduled voice. Compare
        // the two loop shapes on the same CPU, including CPUs without a reference hash.
        let mut with_bus = j.clone();
        with_bus
            .values
            .push(value_map(json!({ "s": "bus", "n": 1 })));
        assert_eq!(
            Engine::new(&with_bus).unwrap().core.bus_graph.max_level(),
            1
        );
        let with_bus = render_job(&with_bus).unwrap();
        assert!(with_bus.errors.is_empty(), "{:?}", with_bus.errors);
        assert_eq!(fnv(&with_bus.channels), hash);

        // The fixture exercises RustFFT's runtime-selected SIMD kernels via the periodic
        // wave tables and reverb. Keep the historical Apple Silicon pin and the exact
        // native Windows and Ubuntu x64 CI result for the AVX planner.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(hash, 0x0a91_bfa1_28b7_2697);
        #[cfg(all(target_arch = "x86_64", any(target_os = "linux", windows)))]
        if std::is_x86_feature_detected!("avx") && std::is_x86_feature_detected!("fma") {
            assert_eq!(hash, 0x0312_e335_f118_3509);
        }
    }

    #[test]
    fn a_feeder_scheduled_after_its_reader_still_sums() {
        // The reader (level 1) is created first, in pause 0; feeders (level 0) come in later
        // pauses. Each quantum renders level 0 first, so the reader hears a feeder from its
        // first sounding quantum. A freed feeder shifts the vectors; a later feeder still sums.
        let j = job(base(
            33075,
            json!([
                { "suspendSec": 0, "haps": [hap(0.0, 0.7, 0.0, 0)] },
                { "suspendSec": 0.25, "haps": [hap(0.3, 0.1, 0.3, 1)] },
                { "suspendSec": 0.55, "haps": [hap(0.6, 0.05, 0.6, 1)] }
            ]),
            json!([
                { "s": "bus", "n": 1 },
                { "s": "one", "attack": 0, "sustain": 1, "dry": 0, "bus": 1 }
            ]),
        ));
        let mut e = Engine::new(&j).unwrap();
        assert_eq!(e.core.bus_graph.max_level(), 1);
        e.play();
        assert!(e.core.errors.is_empty(), "{:?}", e.core.errors);
        // The writer's one * 0.3 * gain stage 0.8, then the reader's gain stage 0.8 (every
        // envelope, post and busgain is 1).
        let want = (0.3f32 * 0.8f32) * 0.8f32;
        let l = &e.channels[0];
        let at = |sec: f64| (sec * SR) as usize;
        assert!(
            l[..at(0.3)].iter().all(|x| *x == 0.0),
            "nothing before the feeder"
        );
        // The feeder's attack (0 becomes 0.001) is heard in its first sounding quantum.
        assert!(l[at(0.3) + 1] > 0.0, "the first sounding quantum");
        assert!(l[at(0.31)..at(0.39)].iter().all(|x| *x == want));
        assert!(
            l[at(0.45)..at(0.6)].iter().all(|x| *x == 0.0),
            "feeder gone"
        );
        assert!(
            l[at(0.61)..at(0.64)].iter().all(|x| *x == want),
            "second feeder"
        );
        assert_eq!(
            e.channels[0], e.channels[1],
            "the stereo bus carries the mono feeder"
        );
    }

    #[test]
    fn voices_render_level_by_level_and_vectors_stay_parallel() {
        // Bus A feeds a reader that sends to bus B, read by a second reader: levels 0, 1, 2.
        let j = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [
                hap(0.0, 0.15, 0.0, 2), hap(0.0, 0.15, 0.0, 1), hap(0.0, 0.05, 0.0, 0)
            ] }]),
            json!([
                { "s": "one", "attack": 0, "sustain": 1, "dry": 0, "bus": "A" },
                { "s": "bus", "n": "A", "dry": 0, "bus": "B" },
                { "s": "bus", "n": "B" }
            ]),
        ));
        let mut e = Engine::new(&j).unwrap();
        assert_eq!(e.core.bus_graph.max_level(), 2);
        e.play();
        assert!(e.core.errors.is_empty(), "{:?}", e.core.errors);
        let want = ((0.3f32 * 0.8f32) * 0.8f32) * 0.8f32;
        assert_eq!(e.channels[0][1000], want, "three gain stages, no latency");
        assert!(e.core.slots.is_empty());
        // While they play the levels and sequence numbers follow the voices.
        let mut e = Engine::new(&j).unwrap();
        e.schedule(&j.pauses[0].haps[0], 0);
        e.schedule(&j.pauses[0].haps[1], 0);
        e.schedule(&j.pauses[0].haps[2], 0);
        let levels: Vec<u32> = e
            .core
            .slots
            .iter()
            .map(|s| e.core.bus_graph.level_of_value(s.value))
            .collect();
        let seqs: Vec<u64> = e.core.slots.iter().map(|s| s.seq).collect();
        assert_eq!(levels, [2, 1, 0]);
        assert_eq!(seqs, [0, 1, 2]);
    }

    fn value_map(v: serde_json::Value) -> ValueMap {
        match crate::job::Value::from_json(v).unwrap() {
            crate::job::Value::Obj(m) => m,
            other => panic!("{other:?}"),
        }
    }

    /// A core with no values on a stereo destination with orbit 1 created on demand.
    fn bare_core(seed: i32) -> Core {
        let mut orbits = OrbitBank::empty(2);
        orbits.create(&crate::job::OrbitSpec {
            orbit: 1.0,
            channels: vec![0, 1],
        });
        Core::new(seed, BusGraph::default(), orbits)
    }

    /// Renders `quanta` quanta, the left channel of each mix appended.
    fn steps(core: &mut Core, quanta: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..quanta {
            core.step();
            out.extend_from_slice(&core.mix[0]);
        }
        out
    }

    fn args(value: &ValueMap, index: usize, t: f64, duration: f64, cps: f64) -> ScheduleArgs<'_> {
        ScheduleArgs {
            value,
            value_index: index,
            t,
            duration,
            cycle: 0.0,
            phase_sec: None,
            cps,
            now_frame: 0,
        }
    }

    /// Settles the core's pending reverb requests as the offline driver does at a pause.
    fn settle(core: &mut Core) {
        core.orbits.settle(core.seed, &mut core.ir_cache);
        for slot in &mut core.slots {
            if std::mem::take(&mut slot.unsettled) {
                slot.voice
                    .settle_local_reverbs(core.seed, &mut core.ir_cache);
            }
        }
    }

    /// Schedules a hap of value `index` at the render head.
    fn at_head<'v>(core: &Core, value: &'v ValueMap, index: usize, dur: f64) -> ScheduleArgs<'v> {
        ScheduleArgs {
            now_frame: core.frame(),
            ..args(value, index, core.frame() as f64 / SR, dur, 0.5)
        }
    }

    #[test]
    fn stop_voices_drops_every_voice_and_keeps_the_rest() {
        let value = value_map(json!({ "s": "sine", "room": 0.5, "roomsize": 0.05 }));
        let run = |stop: bool| {
            let mut core = bare_core(0);
            assert_eq!(core.add_value(&value), 0);
            core.schedule(args(&value, 0, 0.0, 0.5, 0.5));
            core.schedule(args(&value, 0, 0.0, 0.5, 0.5));
            settle(&mut core);
            steps(&mut core, 2);
            if stop {
                let kept = (
                    core.orbits().count(),
                    core.orbits().orbits().len(),
                    core.ir_cache.len(),
                    core.bus_graph().max_level(),
                    core.frame(),
                );
                assert_eq!(core.slots.len(), 2);
                core.stop_voices();
                assert!(core.slots.is_empty());
                assert!(core.outputs.is_empty());
                assert!(core.orbit_of.is_empty());
                let after = (
                    core.orbits().count(),
                    core.orbits().orbits().len(),
                    core.ir_cache.len(),
                    core.bus_graph().max_level(),
                    core.frame(),
                );
                assert_eq!(after, kept);
            }
            core.schedule(at_head(&core, &value, 0, 0.5));
            assert!(core.errors.is_empty(), "{:?}", core.errors);
            core
        };
        let (stopped, never) = (run(true), run(false));
        assert_eq!(stopped.slots.len(), 1);
        assert_eq!(
            stopped.slots[0].seq, 2,
            "the sequence continues after a stop"
        );
        assert_eq!(stopped.seeds.len(), 3);
        assert_eq!(stopped.seeds, never.seeds, "the voice seeds continue");
    }

    #[test]
    fn stop_voices_keeps_the_orbit_reverb_tail() {
        let value = value_map(json!({ "s": "sine", "room": 0.8 }));
        let mut core = bare_core(0);
        core.add_value(&value);
        core.schedule(args(&value, 0, 0.0, 0.05, 0.5));
        settle(&mut core);
        steps(&mut core, 4);
        core.stop_voices();
        assert!(
            steps(&mut core, 20).iter().any(|x| *x != 0.0),
            "the tail rings"
        );
    }

    #[test]
    fn set_seed_resets_the_caches_and_the_same_seed_is_a_no_op() {
        use crate::noise::NoiseKind;
        let values = [
            value_map(json!({ "s": "sine", "room": 0.8, "roomsize": 0.5 })),
            value_map(json!({ "s": "white" })),
            value_map(json!({ "s": "sine", "FX": [{ "room": 0.8, "roomsize": 0.5 }] })),
        ];
        let mut core = bare_core(0);
        for v in &values {
            core.add_value(v);
        }
        core.schedule(args(&values[0], 0, 0.0, 0.02, 0.5));
        core.schedule(args(&values[1], 1, 0.0, 0.02, 0.5));
        settle(&mut core);
        steps(&mut core, 1);
        assert!(core.errors.is_empty(), "{:?}", core.errors);
        assert_eq!(core.ir_cache.len(), 1);
        assert!(core.noise.is_cached(NoiseKind::White));

        assert!(!core.set_seed(0), "the same seed is a no-op");
        assert_eq!(core.ir_cache.len(), 1, "the same seed keeps the IR cache");
        assert!(core.noise.is_cached(NoiseKind::White));

        assert!(core.set_seed(5));
        assert_eq!(core.ir_cache.len(), 0, "another seed empties the IR cache");
        assert!(!core.noise.is_cached(NoiseKind::White), "and the noise");

        // A local reverb is a new reverb per voice, so it requests its IR, and the cleared cache
        // generates it again.
        core.schedule(at_head(&core, &values[2], 2, 0.02));
        settle(&mut core);
        assert!(core.errors.is_empty(), "{:?}", core.errors);
        assert_eq!(core.ir_cache.generated(), 1);
        assert_eq!(core.ir_cache.len(), 1);
    }

    #[test]
    fn create_orbit_keeps_first_add_order() {
        let mut core = Core::new(0, BusGraph::default(), OrbitBank::empty(2));
        for orbit in [3.0, 1.0, 3.0, 2.0] {
            core.create_orbit(&OrbitSpec {
                orbit,
                channels: vec![0, 1],
            });
        }
        let numbers: Vec<f64> = core.orbits().orbits().iter().map(|o| o.orbit()).collect();
        assert_eq!(numbers, [3.0, 1.0, 2.0]);
    }

    #[test]
    fn finish_returns_the_errors_and_the_overflow_flag() {
        let mut core = bare_core(0);
        let errors = vec![
            RenderError::new(ErrorKind::Hap, "first", Some(0)),
            RenderError::new(ErrorKind::Other, "second", None),
        ];
        for e in &errors {
            core.push_error(e.clone());
        }
        assert_eq!(core.finish(), (errors, false));
    }

    #[test]
    fn cps_is_per_hap() {
        // One value with a synced delay (`delaysync` 1/4 cycle): the echo lands at 0.25 s at
        // cps 1 and at 0.5 s at cps 0.5.
        let value = value_map(json!({ "s": "sine", "delay": 0.5, "delaysync": 0.25 }));
        let echo = |cps: f64| {
            let mut core = bare_core(0);
            assert_eq!(core.add_value(&value), 0);
            core.schedule(args(&value, 0, 0.0, 0.05, cps));
            assert!(core.errors.is_empty(), "{:?}", core.errors);
            let out = steps(&mut core, 200);
            // The first sound after the dry note's tail (well before 0.2 s).
            let from = (0.2 * SR) as usize;
            from + first_nonzero(&out[from..]).unwrap()
        };
        let (fast, slow) = (echo(1.0), echo(0.5));
        assert!((11025..11025 + 256).contains(&fast), "{fast}");
        assert!((22050..22050 + 256).contains(&slow), "{slow}");
    }

    #[test]
    fn a_growing_graph_moves_a_playing_voice_to_its_new_level() {
        // A reader of bus B (level 1 alone), then a reader of A feeding B and a feeder of A:
        // the first reader moves to level 2 and renders after its feeders from the next quantum,
        // exactly as with the whole graph known up front.
        let values = [
            value_map(json!({ "s": "bus", "n": "B" })),
            value_map(json!({ "s": "bus", "n": "A", "dry": 0, "bus": "B" })),
            value_map(json!({ "s": "one", "attack": 0, "sustain": 1, "dry": 0, "bus": "A" })),
        ];
        let late = 1280.0 / SR;
        let mut grown = bare_core(0);
        grown.add_value(&values[0]);
        assert_eq!(grown.bus_graph.level_of_value(0), 1);
        grown.schedule(args(&values[0], 0, 0.0, 0.3, 0.5));
        let mut a = steps(&mut grown, 10);
        grown.add_value(&values[1]);
        grown.add_value(&values[2]);
        assert_eq!(grown.bus_graph.level_of_value(0), 2);
        assert_eq!(grown.buses.index_of("A"), Some(1));
        for (k, v) in values.iter().enumerate().skip(1) {
            grown.schedule(ScheduleArgs {
                now_frame: 1280,
                ..args(v, k, late, 0.1, 0.5)
            });
        }
        a.extend(steps(&mut grown, 40));
        let mut known = bare_core(0);
        for v in &values {
            known.add_value(v);
        }
        known.schedule(args(&values[0], 0, 0.0, 0.3, 0.5));
        let mut b = steps(&mut known, 10);
        for (k, v) in values.iter().enumerate().skip(1) {
            known.schedule(ScheduleArgs {
                now_frame: 1280,
                ..args(v, k, late, 0.1, 0.5)
            });
        }
        b.extend(steps(&mut known, 40));
        assert!(grown.errors.is_empty(), "{:?}", grown.errors);
        assert!(a.iter().all(|x| x.is_finite()));
        assert!(a[1280..].iter().any(|x| *x != 0.0), "the chain sounds");
        let bits = |x: &[f32]| x.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn a_late_busgain_throw_keeps_the_delay_send() {
        let j = job(base(
            8820,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0)] }]),
            json!([{ "s": "one", "attack": 0, "sustain": 1, "delay": 0.5, "delaytime": 0.05,
                     "bus": 1, "busgain": { "$num": "NaN" } }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
        assert_eq!(r.errors[0].kind, ErrorKind::Hap);
        assert!(
            r.errors[0].message.contains("TypeError"),
            "{}",
            r.errors[0].message
        );
        // No dry path: the first sound is the first echo, 0.05 s in.
        let first = first_nonzero(&r.channels[0]).unwrap();
        assert!((2205..2400).contains(&first), "{first}");
    }

    #[test]
    fn a_bus_cycle_is_refused() {
        let j = job(base(
            4410,
            json!([{ "suspendSec": 0, "haps": [hap(0.0, 0.05, 0.0, 0), hap(0.0, 0.05, 1.0, 1)] }]),
            json!([{ "s": "bus", "n": 1, "bus": 2 }, { "s": "bus", "n": 2, "bus": 1 }]),
        ));
        let r = render_job(&j).unwrap();
        assert_eq!(
            r.errors,
            vec![RenderError::new(
                ErrorKind::Unsupported,
                "s=bus keys=bus-cycle",
                Some(0)
            )]
        );
        assert!(r.channels.iter().flatten().all(|x| *x == 0.0));
    }
}

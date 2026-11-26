//! The render core both drivers share (the offline `Engine` in `render` and
//! `live::LiveEngine`): the voices, buses, orbits and caches, scheduled hap by hap and rendered a
//! quantum at a time into the mix, and the IR settler seam its reverbs settle through.

use crate::QUANTUM;
use crate::bus::{BusBank, BusGraph};
use crate::job::{ErrorKind, OrbitSpec, RenderError, ValueMap, bar_of};
use crate::js::{Occurrence, voice_seed};
use crate::noise::NoiseCache;
use crate::orbit::{DuckControls, OrbitBank};
use crate::params::{parse_voice_params, prepare_value};
use crate::periodic::WaveTables;
use crate::reverb::{Convolver, IrCache, IrKey, Reverb};
use crate::stems::StemCapture;
use crate::voice::{StartInfo, Voice, VoiceOutput, sound_name};

use crate::QUANTUM_U64 as Q;

/// The render page keeps the first 300 characters of a captured message (`classifyError`).
const MAX_LOG_CHARS: usize = 300;

/// Superdough's log for a duck target with no orbit (`errorLogger` of the error "duck target orbit
/// ${target} does not exist" with origin `superdough`, superdoughoutput.mjs:219), as the render page
/// captures it.
fn missing_duck_target(target: &str) -> String {
    format!("[superdough] error: duck target orbit {target} does not exist")
}

/// What a voice's messages are prefixed with (`hap <t>s <s>: `), formatted only on error.
pub(crate) struct HapTag {
    bar: Option<i64>,
    t: f64,
    s: String,
}

impl HapTag {
    /// The tag of a hap of `value` at `t` in bar `bar`.
    pub(crate) fn new(bar: Option<i64>, t: f64, value: &crate::job::ValueMap) -> HapTag {
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
    pub(crate) fn error(&self, kind: ErrorKind, detail: &str) -> RenderError {
        RenderError::new(kind, self.message(detail), self.bar)
    }
}

/// A live voice and what the driver keeps with it.
pub(crate) struct Slot {
    voice: Voice,
    /// Its bar and the parts of its messages' prefix.
    tag: HapTag,
    /// The index of its orbit in `orbits`.
    orbit: usize,
    /// Its value's index: its bus level is read from the graph each quantum (voices render level
    /// by level, and a graph that grows can move a value's level).
    value: usize,
    /// Its creation sequence number (bus feeders sum in this order).
    seq: u64,
    /// Whether its reverbs' IR requests are still to be settled: set at build, cleared by
    /// `settle_reverbs`.
    unsettled: bool,
}

impl Slot {
    /// Its value's index (its bus level and stem group).
    pub(crate) fn value(&self) -> usize {
        self.value
    }

    /// The index of its orbit in the bank.
    pub(crate) fn orbit(&self) -> usize {
        self.orbit
    }

    #[cfg(test)]
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    #[cfg(test)]
    pub(crate) fn voice(&self) -> &Voice {
        &self.voice
    }
}

/// Which reverb an IR request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrTarget {
    /// An orbit's reverb: its index in the bank.
    Orbit(usize),
    /// A voice's local reverb: the voice's sequence number and the stage index in its chain.
    Local { seq: u64, stage: usize },
}

/// How a reverb's pending IR request is built and landed.
pub(crate) trait IrSettler {
    /// Settles one existing reverb at `target`. `seed` is the core's audio seed and `cache` the
    /// run's IR cache.
    fn settle(&mut self, target: IrTarget, reverb: &mut Reverb, seed: i32, cache: &mut IrCache);
}

/// The synchronous settle: `reverb.settle_cached(seed, cache)` (a miss builds with
/// `Convolver::new`, the Blink stage layout).
pub(crate) struct SyncSettler;

impl IrSettler for SyncSettler {
    fn settle(&mut self, _target: IrTarget, reverb: &mut Reverb, seed: i32, cache: &mut IrCache) {
        reverb.settle_cached(seed, cache);
    }
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

/// The render core shared by the offline [`Engine`](crate::render::Engine) and a streaming
/// driver: the voices, buses, orbits and caches, scheduled hap by hap and rendered a quantum at a
/// time into `mix`.
pub(crate) struct Core {
    /// The audio seed (voice seeds, noise, reverb IRs).
    seed: i32,
    /// The live voices in creation order.
    slots: Vec<Slot>,
    /// Per voice (same order): its output of the quantum being rendered.
    outputs: Vec<VoiceOutput>,
    /// Per voice (same order): its orbit, refreshed each quantum for the orbit bank.
    orbit_of: Vec<usize>,
    /// The next voice's sequence number.
    next_seq: u64,
    /// The bus graph (levels; a cyclic job is refused before rendering).
    bus_graph: BusGraph,
    /// The buses, one quantum's feeders at a time.
    buses: BusBank,
    /// The orbits and the channel merger.
    orbits: OrbitBank,
    /// The periodic-wave tables of this run (built-ins on first use, custom waves cached).
    tables: WaveTables,
    /// The noise buffers of this run (white, pink and brown on first use, for the seed).
    noise: NoiseCache,
    /// The reverb IRs generated in this run (orbit and local reverbs share equal ones).
    ir_cache: IrCache,
    /// `voiceSeed`'s state across the whole run (the page sets the audio seed once per render).
    occurrence: Occurrence,
    /// Every voice seed computed, in call order (tests observe `occurrence` through it).
    #[cfg(test)]
    seeds: Vec<u32>,
    /// The next frame to render (always a multiple of the quantum).
    frame: u64,
    /// Whether a non-finite mix sample was already reported.
    mix_overflowed: bool,
    errors: Vec<RenderError>,
    /// The last quantum's mix, one buffer per destination channel (`orbits.count()`).
    mix: Vec<[f32; QUANTUM]>,
    /// Per voice of the quantum being rendered: whether it is still alive after it.
    alive: Vec<bool>,
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

    /// The errors so far (tests).
    #[cfg(test)]
    pub(crate) fn errors(&self) -> &[RenderError] {
        &self.errors
    }

    /// The live voices in creation order (tests).
    #[cfg(test)]
    pub(crate) fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Every voice seed computed, in call order (tests).
    #[cfg(test)]
    pub(crate) fn seeds(&self) -> &[u32] {
        &self.seeds
    }

    /// The noise buffers of this run (tests).
    #[cfg(test)]
    pub(crate) fn noise(&self) -> &NoiseCache {
        &self.noise
    }

    /// The reverb IRs generated in this run (tests).
    #[cfg(test)]
    pub(crate) fn ir_cache(&self) -> &IrCache {
        &self.ir_cache
    }

    /// Every pending reverb request, in settle order, through `settler`: the orbits' reverbs in
    /// bank order, then each unsettled voice in creation order (its flag cleared), its local
    /// reverbs in chain order. Only existing reverbs are visited.
    pub(crate) fn settle_reverbs(&mut self, settler: &mut dyn IrSettler) {
        let (seed, cache) = (self.seed, &mut self.ir_cache);
        for (k, o) in self.orbits.orbits_mut().iter_mut().enumerate() {
            if let Some(r) = o.reverb_mut() {
                settler.settle(IrTarget::Orbit(k), r, seed, cache);
            }
        }
        for slot in self.slots.iter_mut().filter(|s| s.unsettled) {
            slot.unsettled = false;
            for (stage, r) in slot.voice.local_reverbs_mut() {
                settler.settle(
                    IrTarget::Local {
                        seq: slot.seq,
                        stage,
                    },
                    r,
                    seed,
                    cache,
                );
            }
        }
    }

    /// Lands a convolver built elsewhere on `target` (newest request wins; a target that is
    /// gone is skipped and `convolver` is not called).
    pub(crate) fn land_ir(
        &mut self,
        target: IrTarget,
        request: u64,
        convolver: impl FnOnce() -> Convolver,
    ) {
        if let Some(r) = self.target_reverb_mut(target) {
            r.apply(request, convolver);
        }
    }

    /// `IrCache::insert`: caches a convolver built elsewhere.
    pub(crate) fn cache_ir(&mut self, key: IrKey, convolver: Convolver) {
        self.ir_cache.insert(key, convolver);
    }

    /// The reverb at `target`, when it still exists.
    pub(crate) fn target_reverb_mut(&mut self, target: IrTarget) -> Option<&mut Reverb> {
        match target {
            IrTarget::Orbit(k) => self.orbits.orbits_mut().get_mut(k)?.reverb_mut(),
            IrTarget::Local { seq, stage } => self
                .slots
                .iter_mut()
                .find(|s| s.seq == seq)?
                .voice
                .local_reverb_mut(stage),
        }
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

    pub(crate) fn step_with_stems(&mut self, mut capture: Option<&mut StemCapture>) {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::SAMPLE_RATE_F64 as SR;
    use serde_json::json;

    #[test]
    fn retain_flagged_keeps_the_flagged_items_in_order() {
        let mut v = vec![1, 2, 3, 4, 5];
        retain_flagged(&mut v, &[true, false, true, false, true]);
        assert_eq!(v, [1, 3, 5]);
        retain_flagged(&mut v, &[false, false, false]);
        assert!(v.is_empty());
    }

    pub(crate) fn first_nonzero(x: &[f32]) -> Option<usize> {
        x.iter().position(|s| *s != 0.0)
    }

    pub(crate) fn value_map(v: serde_json::Value) -> ValueMap {
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
        core.settle_reverbs(&mut SyncSettler)
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

    /// Records each target it is handed, then settles it synchronously.
    struct Recording(Vec<IrTarget>);

    impl IrSettler for Recording {
        fn settle(
            &mut self,
            target: IrTarget,
            reverb: &mut Reverb,
            seed: i32,
            cache: &mut IrCache,
        ) {
            self.0.push(target);
            SyncSettler.settle(target, reverb, seed, cache);
        }
    }

    /// The stage indexes of a voice's local reverbs, in chain order.
    fn local_stages(slot: &Slot) -> Vec<usize> {
        slot.voice
            .stages()
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, crate::chain::Stage::LocalReverb(_)))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn settle_reverbs_visits_orbits_then_new_voices_in_order() {
        let mut core = Core::new(0, BusGraph::default(), OrbitBank::empty(2));
        for n in [1.0, 2.0] {
            let spec = crate::job::OrbitSpec {
                orbit: n,
                channels: vec![0, 1],
            };
            core.create_orbit(&spec);
        }
        let values = [
            value_map(json!({ "s": "sine", "room": 0.5, "roomsize": 0.05 })),
            value_map(json!({ "s": "sine", "orbit": 2, "room": 0.5, "roomsize": 0.1 })),
            value_map(json!({
                "s": "sine",
                "FX": [{ "room": 0.5, "roomsize": 0.05 }, { "room": 0.3, "roomsize": 0.2 }]
            })),
        ];
        for v in &values {
            core.add_value(v);
        }
        for i in [0, 2, 1] {
            core.schedule(args(&values[i], i, 0.0, 0.5, 0.5));
        }
        let seqs: Vec<u64> = core.slots.iter().map(|s| s.seq).collect();
        assert_eq!(seqs, [0, 1, 2]);
        assert!(local_stages(&core.slots[0]).is_empty());
        assert!(local_stages(&core.slots[2]).is_empty());
        let stages = local_stages(&core.slots[1]);
        let [a, b] = stages[..] else {
            panic!("{stages:?}")
        };
        assert!(a < b);

        let mut rec = Recording(Vec::new());
        core.settle_reverbs(&mut rec);
        assert_eq!(
            rec.0,
            [
                IrTarget::Orbit(0),
                IrTarget::Orbit(1),
                IrTarget::Local { seq: 1, stage: a },
                IrTarget::Local { seq: 1, stage: b },
            ]
        );
        assert!(core.slots.iter().all(|s| !s.unsettled));

        let mut rec = Recording(Vec::new());
        core.settle_reverbs(&mut rec);
        assert_eq!(rec.0, [IrTarget::Orbit(0), IrTarget::Orbit(1)]);

        core.schedule(args(&values[2], 2, 0.0, 0.5, 0.5));
        let last = core.slots.last().unwrap();
        assert_eq!(last.seq, 3);
        let stages = local_stages(last);
        let mut rec = Recording(Vec::new());
        core.settle_reverbs(&mut rec);
        assert_eq!(
            rec.0,
            [
                IrTarget::Orbit(0),
                IrTarget::Orbit(1),
                IrTarget::Local {
                    seq: 3,
                    stage: stages[0]
                },
                IrTarget::Local {
                    seq: 3,
                    stage: stages[1]
                },
            ]
        );
    }

    #[test]
    fn land_ir_skips_a_target_that_is_gone() {
        let value = value_map(json!({ "s": "sine", "FX": [{ "room": 0.5, "roomsize": 0.05 }] }));
        let mut core = bare_core(0);
        core.add_value(&value);
        core.schedule(args(&value, 0, 0.0, 0.5, 0.5));
        let called = std::cell::Cell::new(false);
        let one_frame = || {
            called.set(true);
            Convolver::new(&[vec![1.0], vec![1.0]], 1.0)
        };
        core.land_ir(IrTarget::Orbit(7), 1, one_frame);
        core.land_ir(IrTarget::Local { seq: 99, stage: 0 }, 1, one_frame);
        assert!(!called.get(), "a target that is gone builds nothing");

        let target = IrTarget::Local {
            seq: core.slots[0].seq,
            stage: local_stages(&core.slots[0])[0],
        };
        core.land_ir(target, 1, one_frame);
        assert!(called.get());
        assert_eq!(core.target_reverb_mut(target).unwrap().applied(), 1);
        called.set(false);
        core.land_ir(target, 1, one_frame);
        assert_eq!(core.target_reverb_mut(target).unwrap().applied(), 1);
        assert!(!called.get(), "an old request is ignored");
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
}

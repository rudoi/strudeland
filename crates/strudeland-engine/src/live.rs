//! Streaming mode: [`LiveEngine`] renders haps that arrive over time, a quantum at a time into a
//! caller's buffer, on the render core the offline [`crate::Engine`] uses. Values (and their
//! orbits) are added as they first appear, haps are scheduled at any frame (a late one starts at
//! the render head and is counted), unsupported values are skipped with one warning per
//! signature, and reverb IRs are built off the render path by an optional worker
//! ([`IrRequest`], [`IrBuilt`], [`build_ir`]).
//!
//! With `sync_ir` (or no worker attached) the IRs are built synchronously at the next quantum, as
//! offline settles them at the end of a pause: replaying a job's haps pause by pause gives the
//! offline output bit for bit (`tests/live_equals_offline.rs`).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use crate::bus::BusGraph;
use crate::controls::signature_of_with_bus;
use crate::job::{OrbitSpec, ValueMap};
use crate::noise::NoiseCache;
use crate::orbit::OrbitBank;
use crate::render::{Core, ScheduleArgs};
use crate::reverb::{Convolver, IrCache, IrKey, IrPlan, normalization_scale};
use crate::{QUANTUM, SAMPLE_RATE_F64 as SR};

/// At most this many core render errors become [`LiveEvent::Error`]s per
/// [`LiveEngine::take_events`]; the rest are dropped.
const MAX_ERRORS_PER_TAKE: usize = 32;

/// Options of a [`LiveEngine`].
#[derive(Debug, Default, Clone, Copy)]
pub struct LiveOptions {
    /// Build reverb IRs synchronously even with a worker attached (tests: the offline order).
    pub sync_ir: bool,
}

/// One hap to play.
#[derive(Debug, Clone, Copy)]
pub struct LiveHap {
    /// `floor(t_sec * 44100)`: where the hap is scheduled (informational; timing uses `t_sec`).
    pub frame: u64,
    /// Start in seconds of engine time (frame 0 is 0 s).
    pub t_sec: f64,
    pub duration_sec: f64,
    /// Absolute cycle of the onset.
    pub cycle: f64,
    /// Elapsed layout seconds for fixed-rate oscillators.
    pub phase_sec: Option<f64>,
    /// The index given to [`LiveEngine::add_value`].
    pub value: u32,
    /// The hap's cycles per second.
    pub cps: f64,
}

/// What the engine reports besides audio.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    /// The first skipped hap of an unsupported signature.
    Warning {
        signature: String,
        keys: Vec<String>,
        first_cycle: f64,
    },
    /// A misuse (an index out of order, an unknown value) or a render error.
    Error { message: String },
}

/// Cumulative hap counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LiveCounters {
    /// Haps accepted (played or late; not skipped).
    pub haps: u64,
    /// Haps scheduled before the render head (they start at the head).
    pub late_haps: u64,
    /// Haps skipped because their value is unsupported.
    pub skipped_haps: u64,
}

/// One reverb IR to build off the render path (plain data, `Send`).
#[derive(Debug)]
pub struct IrRequest {
    id: u64,
    key: IrKey,
    plan: IrPlan,
    seed: i32,
}

/// A built IR: the request's id and key and the convolver that plays it (plain data, `Send`).
#[derive(Debug)]
pub struct IrBuilt {
    id: u64,
    key: IrKey,
    convolver: Convolver,
}

/// Builds a requested IR: what offline does at a cache miss (`IrPlan::render`, then the
/// normalized convolver). Pure; the IR worker runs it.
pub fn build_ir(req: IrRequest) -> IrBuilt {
    let ir = req.plan.render(req.seed);
    IrBuilt {
        id: req.id,
        key: req.key,
        convolver: Convolver::new_live(&ir, normalization_scale(&ir)),
    }
}

/// Which reverb an IR request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// An orbit's reverb (its index in the bank).
    Orbit(usize),
    /// A voice's local reverb: the voice's sequence number and the stage index in its chain.
    Local { seq: u64, stage: usize },
}

/// The IR worker's channels and the requests in flight.
struct IrLink {
    requests: Sender<IrRequest>,
    built: Receiver<IrBuilt>,
    /// Keys sent and not yet built, with the id of the request that carries each.
    in_flight: HashMap<IrKey, u64>,
    /// Reverbs waiting for a key: target, request number, key.
    waiting: Vec<(Target, u64, IrKey)>,
    next_id: u64,
}

/// An unsupported value's sound and keys (`signature_of`'s result).
type Signature = (String, Vec<String>);

/// The streaming engine. `!Send` (the core holds `Rc`s): the thread that builds it renders it.
pub struct LiveEngine {
    core: Core,
    values: Vec<ValueMap>,
    /// Per `(value index, cps bits)`: its signature when unsupported, `None` when supported.
    /// Valid for the bus graph's `decided_generation`: a value that closes a bus cycle clears it.
    decided: HashMap<(u32, u64), Option<Signature>>,
    decided_generation: u64,
    /// The signatures already warned about.
    warned: HashSet<Signature>,
    events: Vec<LiveEvent>,
    counters: LiveCounters,
    options: LiveOptions,
    ir: Option<IrLink>,
}

impl LiveEngine {
    pub fn new(seed: i32) -> LiveEngine {
        LiveEngine::with_options(seed, LiveOptions::default())
    }

    pub fn with_options(seed: i32, options: LiveOptions) -> LiveEngine {
        LiveEngine {
            core: Core::new(seed, BusGraph::default(), OrbitBank::empty(2)),
            values: Vec::new(),
            decided: HashMap::new(),
            decided_generation: 0,
            warned: HashSet::new(),
            events: Vec::new(),
            counters: LiveCounters::default(),
            options,
            ir: None,
        }
    }

    /// Changes the audio seed of haps scheduled from now: clears the noise cache and the IR
    /// cache (an orbit reverb keeps its IR until its next request). The same seed again is a
    /// no-op (every evaluate re-sends it).
    pub fn set_seed(&mut self, seed: i32) {
        if seed == self.core.seed {
            return;
        }
        self.core.seed = seed;
        self.core.noise = NoiseCache::new(seed);
        self.core.ir_cache = IrCache::new();
    }

    /// Adds value `index` (0, 1, 2, ... in order; anything else is an error event and ignored)
    /// and, when `orbit` is given, creates that orbit (a no-op when its number exists).
    pub fn add_value(&mut self, index: u32, value: ValueMap, orbit: Option<OrbitSpec>) {
        if index as usize != self.values.len() {
            self.events.push(LiveEvent::Error {
                message: format!(
                    "value index {index} out of order, expected {}",
                    self.values.len()
                ),
            });
            return;
        }
        self.core.add_value(&value);
        self.values.push(value);
        if let Some(o) = orbit {
            self.core.orbits.create(&o);
        }
    }

    /// Schedules one hap: skipped (and warned about once per signature) when its value is
    /// unsupported, started at the render head when late.
    pub fn schedule(&mut self, hap: LiveHap) {
        let v = hap.value as usize;
        if v >= self.values.len() {
            self.events.push(LiveEvent::Error {
                message: format!("hap names unknown value {}", hap.value),
            });
            return;
        }
        debug_assert!(
            hap.frame == (hap.t_sec * SR).floor().max(0.0) as u64,
            "frame {} is not floor(t_sec * 44100) for t_sec {}",
            hap.frame,
            hap.t_sec
        );
        let generation = self.core.bus_graph.cycle_generation();
        if generation != self.decided_generation {
            // A later value closed a bus cycle: its readers are unsupported from now on.
            self.decided.clear();
            self.decided_generation = generation;
        }
        let key = (hap.value, hap.cps.to_bits());
        let decided = self.decided.entry(key).or_insert_with(|| {
            let cyclic = self.core.bus_graph.cyclic_readers()[v];
            signature_of_with_bus(&self.values[v], cyclic, hap.cps)
        });
        if let Some(signature) = decided {
            self.counters.skipped_haps += 1;
            if !self.warned.contains(signature) {
                self.warned.insert(signature.clone());
                let (signature, keys) = signature.clone();
                self.events.push(LiveEvent::Warning {
                    signature,
                    keys,
                    first_cycle: hap.cycle,
                });
            }
            return;
        }
        self.counters.haps += 1;
        let now = self.core.frame as f64 / SR;
        let mut t = hap.t_sec;
        if t < now {
            self.counters.late_haps += 1;
            t = now;
        }
        let now_frame = self.core.frame;
        self.core.schedule(ScheduleArgs {
            value: &self.values[v],
            value_index: v,
            t,
            duration: hap.duration_sec,
            cycle: hap.cycle,
            phase_sec: hap.phase_sec,
            cps: hap.cps,
            now_frame,
        });
    }

    /// The render head: the next frame [`Self::render_quantum`] renders.
    pub fn now_frame(&self) -> u64 {
        self.core.frame
    }

    /// Renders the next 128 frames into `out`, interleaved `L R`: built IRs land first, then the
    /// pending reverb requests are settled (synchronously) or sent to the worker.
    pub fn render_quantum(&mut self, out: &mut [f32; 2 * QUANTUM]) {
        self.poll_ir();
        self.settle_reverbs();
        self.core.step();
        let (l, r) = (&self.core.mix[0], &self.core.mix[1]);
        for i in 0..QUANTUM {
            out[2 * i] = l[i];
            out[2 * i + 1] = r[i];
        }
    }

    /// Drops every voice (effect tails on the orbits ring out); values, orbits, the IR cache,
    /// requests in flight and the counters stay.
    pub fn stop(&mut self) {
        self.core.slots.clear();
        self.core.outputs.clear();
        self.core.orbit_of.clear();
    }

    /// Drains the events, then up to 32 of the core's render errors (the rest are dropped).
    pub fn take_events(&mut self) -> Vec<LiveEvent> {
        let mut out = std::mem::take(&mut self.events);
        out.extend(
            self.core
                .errors
                .drain(..)
                .take(MAX_ERRORS_PER_TAKE)
                .map(|e| LiveEvent::Error { message: e.message }),
        );
        out
    }

    pub fn counters(&self) -> LiveCounters {
        self.counters
    }

    /// Attaches an IR worker: requests go out on `requests`, results come back on `built`
    /// (landed by [`Self::poll_ir`]). Ignored while `sync_ir` is set.
    pub fn attach_ir_worker(&mut self, requests: Sender<IrRequest>, built: Receiver<IrBuilt>) {
        self.ir = Some(IrLink {
            requests,
            built,
            in_flight: HashMap::new(),
            waiting: Vec::new(),
            next_id: 0,
        });
    }

    /// Lands every built IR: cached, then applied to each reverb waiting for its key (the
    /// newest request of a reverb wins; an older result that arrives later is ignored).
    pub fn poll_ir(&mut self) {
        if self.options.sync_ir {
            return;
        }
        let Some(link) = &mut self.ir else {
            return;
        };
        loop {
            let b = match link.built.try_recv() {
                Ok(b) => b,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            if link.in_flight.get(&b.key) == Some(&b.id) {
                link.in_flight.remove(&b.key);
            }
            let core = &mut self.core;
            link.waiting.retain(|(target, request, key)| {
                if *key != b.key {
                    return true;
                }
                apply(core, *target, *request, || b.convolver.fresh_live());
                false
            });
            core.ir_cache.insert(b.key, b.convolver);
        }
    }

    /// Step (2) of a quantum: the pending requests of the orbit reverbs and the new voices'
    /// local reverbs, settled as offline does at the end of a pause, or sent to the worker.
    fn settle_reverbs(&mut self) {
        let asynchronous = !self.options.sync_ir && self.ir.is_some();
        let core = &mut self.core;
        if !asynchronous {
            core.orbits.settle(core.seed, &mut core.ir_cache);
            for slot in core.slots.iter_mut().filter(|s| s.unsettled) {
                slot.unsettled = false;
                slot.voice
                    .settle_local_reverbs(core.seed, &mut core.ir_cache);
            }
            return;
        }
        let mut requests: Vec<(Target, u64, IrPlan)> = Vec::new();
        for (k, o) in core.orbits.orbits_mut().iter_mut().enumerate() {
            if let Some((request, plan)) = o.take_reverb_request() {
                requests.push((Target::Orbit(k), request, plan));
            }
        }
        for slot in core.slots.iter_mut().filter(|s| s.unsettled) {
            slot.unsettled = false;
            for (stage, request, plan) in slot.voice.take_local_reverb_requests() {
                let target = Target::Local {
                    seq: slot.seq,
                    stage,
                };
                requests.push((target, request, plan));
            }
        }
        for (target, request, plan) in requests {
            self.request_ir(target, request, plan);
        }
    }

    /// One reverb request in async mode: a cache hit applies now, else the reverb waits for
    /// the key, which is sent unless already in flight. A worker that is gone settles the
    /// request synchronously and drops the link.
    fn request_ir(&mut self, target: Target, request: u64, plan: IrPlan) {
        let seed = self.core.seed;
        let key = plan.key(seed);
        if let Some(c) = self.core.ir_cache.get(&key) {
            apply(&mut self.core, target, request, || c);
            return;
        }
        let Some(link) = &mut self.ir else {
            return self.build_now(
                target,
                request,
                IrRequest {
                    id: 0,
                    key,
                    plan,
                    seed,
                },
            );
        };
        if !link.in_flight.contains_key(&key) {
            let id = link.next_id;
            link.next_id += 1;
            let sent = link.requests.send(IrRequest {
                id,
                key: key.clone(),
                plan,
                seed,
            });
            if let Err(e) = sent {
                self.ir = None;
                return self.build_now(target, request, e.0);
            }
            link.in_flight.insert(key.clone(), id);
        }
        link.waiting.push((target, request, key));
    }

    /// Builds a cache miss on this thread after the IR worker disconnects.
    fn build_now(&mut self, target: Target, request: u64, ir: IrRequest) {
        let built = build_ir(ir);
        apply(&mut self.core, target, request, || {
            built.convolver.fresh_live()
        });
        self.core.ir_cache.insert(built.key, built.convolver);
    }
}

/// Applies a built convolver to its target (a voice that has ended is skipped).
fn apply(core: &mut Core, target: Target, request: u64, convolver: impl FnOnce() -> Convolver) {
    match target {
        Target::Orbit(k) => {
            if let Some(o) = core.orbits.orbits_mut().get_mut(k) {
                o.apply_reverb(request, convolver);
            }
        }
        Target::Local { seq, stage } => {
            if let Some(slot) = core.slots.iter_mut().find(|s| s.seq == seq) {
                slot.voice.apply_local_reverb(stage, request, convolver);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Value;
    use serde_json::json;
    use std::sync::mpsc;

    fn value_map(v: serde_json::Value) -> ValueMap {
        match Value::from_json(v).unwrap() {
            Value::Obj(m) => m,
            other => panic!("{other:?}"),
        }
    }

    fn orbit1() -> Option<OrbitSpec> {
        Some(OrbitSpec {
            orbit: 1.0,
            channels: vec![0, 1],
        })
    }

    fn hap(t: f64, duration: f64, value: u32, cps: f64) -> LiveHap {
        LiveHap {
            frame: (t * SR).floor() as u64,
            t_sec: t,
            duration_sec: duration,
            cycle: 0.0,
            phase_sec: None,
            value,
            cps,
        }
    }

    /// Renders `quanta` quanta; returns the left channel.
    fn render(e: &mut LiveEngine, quanta: usize) -> Vec<f32> {
        let mut left = Vec::new();
        let mut buf = [0.0f32; 2 * QUANTUM];
        for _ in 0..quanta {
            e.render_quantum(&mut buf);
            left.extend(buf.iter().step_by(2));
        }
        left
    }

    fn first_nonzero(x: &[f32]) -> Option<usize> {
        x.iter().position(|s| *s != 0.0)
    }

    fn sine() -> ValueMap {
        value_map(json!({ "s": "sine", "note": 69 }))
    }

    #[test]
    fn ir_messages_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IrRequest>();
        assert_send::<IrBuilt>();
    }

    #[test]
    fn values_grow_in_order_only() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, sine(), orbit1());
        e.add_value(0, sine(), None);
        e.add_value(2, sine(), None);
        assert_eq!(
            e.take_events(),
            [
                LiveEvent::Error {
                    message: "value index 0 out of order, expected 1".into()
                },
                LiveEvent::Error {
                    message: "value index 2 out of order, expected 1".into()
                },
            ]
        );
        e.add_value(1, sine(), None);
        assert!(e.take_events().is_empty());
        assert_eq!(e.values.len(), 2);
        e.schedule(hap(0.0, 0.1, 2, 0.5));
        assert_eq!(
            e.take_events(),
            [LiveEvent::Error {
                message: "hap names unknown value 2".into()
            }]
        );
        assert_eq!(e.counters(), LiveCounters::default());
    }

    #[test]
    fn orbits_are_created_in_first_add_order() {
        let spec = |orbit: f64| {
            Some(OrbitSpec {
                orbit,
                channels: vec![0, 1],
            })
        };
        let mut e = LiveEngine::new(0);
        e.add_value(0, sine(), spec(3.0));
        e.add_value(1, sine(), spec(1.0));
        e.add_value(2, sine(), spec(3.0));
        // Created even for an unsupported value.
        e.add_value(3, value_map(json!({ "s": "bd" })), spec(2.0));
        let numbers: Vec<f64> = e.core.orbits.orbits().iter().map(|o| o.orbit()).collect();
        assert_eq!(numbers, [3.0, 1.0, 2.0]);
    }

    #[test]
    fn a_future_hap_sounds_at_its_frame() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, sine(), orbit1());
        let t = 1000.5 / SR;
        e.schedule(hap(t, 0.05, 0, 0.5));
        let left = render(&mut e, 40);
        let first = first_nonzero(&left).unwrap();
        assert!(first >= (t * SR).floor() as usize, "{first}");
        assert!(first < 1010, "{first}");
        assert_eq!(e.counters().haps, 1);
        assert_eq!(e.counters().late_haps, 0);
        assert!(e.take_events().is_empty());
    }

    #[test]
    fn a_late_hap_starts_at_the_render_head() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, sine(), orbit1());
        render(&mut e, 10);
        assert_eq!(e.now_frame(), 1280);
        e.schedule(hap(0.001, 0.05, 0, 0.5));
        let left = render(&mut e, 10);
        assert_eq!(
            first_nonzero(&left),
            Some(1),
            "sin(0) at the head, then sound"
        );
        assert_eq!(
            e.counters(),
            LiveCounters {
                haps: 1,
                late_haps: 1,
                skipped_haps: 0
            }
        );
        assert!(
            e.take_events().is_empty(),
            "a late hap is counted, not reported"
        );
    }

    #[test]
    fn unsupported_values_are_skipped_with_one_warning_per_signature() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, value_map(json!({ "s": "bd" })), orbit1());
        e.add_value(1, value_map(json!({ "s": "bd", "gain": 0.5 })), None);
        e.add_value(2, value_map(json!({ "s": "hh" })), None);
        let mut h = hap(0.0, 0.1, 0, 0.5);
        h.cycle = 4.0;
        e.schedule(h);
        e.schedule(hap(0.1, 0.1, 0, 0.5));
        e.schedule(hap(0.2, 0.1, 1, 0.5));
        let events = e.take_events();
        assert_eq!(events.len(), 1, "{events:?}");
        let LiveEvent::Warning {
            signature,
            first_cycle,
            ..
        } = &events[0]
        else {
            panic!("{events:?}")
        };
        assert_eq!((signature.as_str(), *first_cycle), ("bd", 4.0));
        e.schedule(hap(0.3, 0.1, 2, 0.5));
        assert_eq!(
            e.take_events().len(),
            1,
            "a second signature warns once more"
        );
        assert_eq!(e.counters().skipped_haps, 4);
        assert_eq!(e.counters().haps, 0);
        assert!(render(&mut e, 20).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn a_bus_cycle_is_unsupported() {
        let mut e = LiveEngine::new(0);
        e.add_value(
            0,
            value_map(json!({ "s": "bus", "n": "A", "bus": "B" })),
            orbit1(),
        );
        e.add_value(
            1,
            value_map(json!({ "s": "bus", "n": "B", "bus": "A" })),
            None,
        );
        e.schedule(hap(0.0, 0.1, 0, 0.5));
        let events = e.take_events();
        let [LiveEvent::Warning { keys, .. }] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(keys.iter().any(|k| k == "bus-cycle"), "{keys:?}");
        assert_eq!(e.counters().skipped_haps, 1);
    }

    #[test]
    fn a_later_value_that_closes_a_bus_cycle_makes_earlier_readers_unsupported() {
        let mut e = LiveEngine::new(0);
        e.add_value(
            0,
            value_map(json!({ "s": "bus", "n": "A", "bus": "B" })),
            orbit1(),
        );
        e.schedule(hap(0.0, 0.1, 0, 0.5));
        assert!(e.take_events().is_empty(), "acyclic so far: playable");
        assert_eq!(e.counters().haps, 1);
        e.add_value(
            1,
            value_map(json!({ "s": "bus", "n": "B", "bus": "A" })),
            None,
        );
        e.schedule(hap(0.1, 0.1, 0, 0.5));
        let events = e.take_events();
        let [LiveEvent::Warning { keys, .. }] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(keys.iter().any(|k| k == "bus-cycle"), "{keys:?}");
        assert_eq!(e.counters().haps, 1);
        assert_eq!(e.counters().skipped_haps, 1);
    }

    #[test]
    fn a_later_value_that_raises_a_level_renders_finite() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, value_map(json!({ "s": "bus", "n": "B" })), orbit1());
        e.schedule(hap(0.0, 0.3, 0, 0.5));
        let mut out = render(&mut e, 10);
        e.add_value(
            1,
            value_map(json!({ "s": "bus", "n": "A", "dry": 0, "bus": "B" })),
            None,
        );
        e.add_value(
            2,
            value_map(json!({ "s": "one", "attack": 0, "sustain": 1, "dry": 0, "bus": "A" })),
            None,
        );
        assert_eq!(e.core.bus_graph.level_of_value(0), 2);
        let t = e.now_frame() as f64 / SR;
        e.schedule(hap(t, 0.1, 1, 0.5));
        e.schedule(hap(t, 0.1, 2, 0.5));
        out.extend(render(&mut e, 40));
        assert!(out.iter().all(|x| x.is_finite()));
        assert!(out[1280..].iter().any(|x| *x != 0.0), "the chain sounds");
        assert!(e.take_events().is_empty());
    }

    #[test]
    fn set_seed_changes_a_noise_voice() {
        let noise = || value_map(json!({ "s": "white" }));
        let run = |reseed: Option<i32>| {
            let mut e = LiveEngine::new(0);
            e.add_value(0, noise(), orbit1());
            if let Some(s) = reseed {
                e.set_seed(s);
            }
            e.schedule(hap(0.0, 0.05, 0, 0.5));
            render(&mut e, 10)
        };
        let (a, b, c) = (run(None), run(Some(7)), run(Some(0)));
        assert!(a.iter().any(|x| *x != 0.0));
        assert_ne!(a, b, "another seed, other noise");
        assert_eq!(a, c, "the same seed, the same noise");
    }

    #[test]
    fn set_seed_to_the_same_seed_keeps_the_caches() {
        let mut e = LiveEngine::with_options(0, LiveOptions { sync_ir: true });
        e.add_value(0, room(), orbit1());
        e.schedule(hap(0.0, 0.02, 0, 0.5));
        render(&mut e, 1);
        assert_eq!(e.core.ir_cache.len(), 1);
        e.set_seed(0);
        assert_eq!(e.core.ir_cache.len(), 1, "the same seed keeps the IR cache");
        e.set_seed(5);
        assert_eq!(e.core.ir_cache.len(), 0, "another seed clears it");
    }

    #[test]
    fn stop_drops_the_voices_and_keeps_the_values() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, sine(), orbit1());
        e.schedule(hap(0.0, 1.0, 0, 0.5));
        assert!(render(&mut e, 4).iter().any(|x| *x != 0.0));
        e.stop();
        assert!(render(&mut e, 4).iter().all(|x| *x == 0.0));
        e.add_value(1, sine(), None);
        assert!(e.take_events().is_empty(), "index 1 is next after a stop");
        let t = e.now_frame() as f64 / SR;
        e.schedule(hap(t, 0.1, 1, 0.5));
        assert!(render(&mut e, 4).iter().any(|x| *x != 0.0));
        assert_eq!(e.counters().haps, 2);
    }

    #[test]
    fn stop_keeps_an_orbit_reverb_tail() {
        let mut e = LiveEngine::new(0);
        e.add_value(0, value_map(json!({ "s": "sine", "room": 0.8 })), orbit1());
        e.schedule(hap(0.0, 0.05, 0, 0.5));
        render(&mut e, 4);
        e.stop();
        assert!(
            render(&mut e, 20).iter().any(|x| *x != 0.0),
            "the tail rings"
        );
    }

    fn room() -> ValueMap {
        value_map(json!({ "s": "sine", "room": 0.8, "roomsize": 0.5 }))
    }

    /// The reverb return of an orbit: the output after the dry note ended.
    fn after_dry(e: &mut LiveEngine) -> Vec<f32> {
        // The note lasts 0.02 s (882 frames) plus the release; read from 0.2 s.
        let left = render(e, 100);
        left[8820..].to_vec()
    }

    #[test]
    fn a_worker_ir_lands_when_polled() {
        let (req_tx, req_rx) = mpsc::channel();
        let (built_tx, built_rx) = mpsc::channel();
        let mut e = LiveEngine::new(0);
        e.attach_ir_worker(req_tx, built_rx);
        e.add_value(0, room(), orbit1());
        e.schedule(hap(0.0, 0.02, 0, 0.5));
        let before = after_dry(&mut e);
        assert!(
            before.iter().all(|x| *x == 0.0),
            "no IR yet: a silent return"
        );
        let reqs: Vec<IrRequest> = req_rx.try_iter().collect();
        assert_eq!(reqs.len(), 1);
        for r in reqs {
            built_tx.send(build_ir(r)).unwrap();
        }
        e.schedule(hap(e.now_frame() as f64 / SR, 0.02, 0, 0.5));
        let after = after_dry(&mut e);
        assert!(after.iter().any(|x| *x != 0.0), "the IR plays");
        assert!(e.take_events().is_empty());
        assert_eq!(e.ir.as_ref().unwrap().waiting.len(), 0);
    }

    #[test]
    fn the_newest_request_wins_whatever_order_results_arrive_in() {
        let (req_tx, req_rx) = mpsc::channel();
        let (built_tx, built_rx) = mpsc::channel();
        let mut e = LiveEngine::new(0);
        e.attach_ir_worker(req_tx, built_rx);
        e.add_value(0, room(), orbit1());
        e.add_value(
            1,
            value_map(json!({ "s": "sine", "room": 0.8, "roomsize": 2 })),
            None,
        );
        e.schedule(hap(0.0, 0.02, 0, 0.5));
        render(&mut e, 1);
        e.schedule(hap(e.now_frame() as f64 / SR, 0.02, 1, 0.5));
        render(&mut e, 1);
        let reqs: Vec<IrRequest> = req_rx.try_iter().collect();
        assert_eq!(reqs.len(), 2);
        let mut built: Vec<IrBuilt> = reqs.into_iter().map(build_ir).collect();
        let older = built.remove(0);
        let newer_len = built[0].convolver.length();
        built_tx.send(built.remove(0)).unwrap();
        render(&mut e, 1);
        built_tx.send(older).unwrap();
        render(&mut e, 1);
        let orbit = &e.core.orbits.orbits()[0];
        let r = orbit.reverb().unwrap();
        assert_eq!(r.applied(), 2);
        assert_eq!(r.convolver().unwrap().length(), newer_len);
        assert_eq!(newer_len, (2.0f64 * 1.5 * SR).round() as usize);

        e.schedule(hap(e.now_frame() as f64 / SR, 0.02, 0, 0.5));
        render(&mut e, 1);
        let r = e.core.orbits.orbits()[0].reverb().unwrap();
        assert_eq!(
            r.applied(),
            3,
            "returning to a cached room applies immediately"
        );
        assert_eq!(
            r.convolver().unwrap().length(),
            (0.5 * 1.5 * SR).round() as usize
        );
        assert!(req_rx.try_recv().is_err(), "the cache hit needs no rebuild");
    }

    #[test]
    fn a_dropped_worker_falls_back_to_a_synchronous_settle() {
        let (req_tx, req_rx) = mpsc::channel();
        let (_built_tx, built_rx) = mpsc::channel();
        drop(req_rx);
        let mut e = LiveEngine::new(0);
        e.attach_ir_worker(req_tx, built_rx);
        e.add_value(0, room(), orbit1());
        e.schedule(hap(0.0, 0.02, 0, 0.5));
        assert!(
            after_dry(&mut e).iter().any(|x| *x != 0.0),
            "built in place"
        );
        assert!(e.ir.is_none(), "the link is dropped");
    }

    #[test]
    fn sync_ir_ignores_an_attached_worker() {
        let (req_tx, req_rx) = mpsc::channel();
        let (_built_tx, built_rx) = mpsc::channel();
        let mut e = LiveEngine::with_options(0, LiveOptions { sync_ir: true });
        e.attach_ir_worker(req_tx, built_rx);
        e.add_value(0, room(), orbit1());
        e.schedule(hap(0.0, 0.02, 0, 0.5));
        assert!(after_dry(&mut e).iter().any(|x| *x != 0.0));
        assert_eq!(req_rx.try_iter().count(), 0);
    }

    #[test]
    fn a_local_reverb_ir_lands_through_the_worker() {
        let (req_tx, req_rx) = mpsc::channel();
        let (built_tx, built_rx) = mpsc::channel();
        let mut e = LiveEngine::new(0);
        e.attach_ir_worker(req_tx, built_rx);
        let local = value_map(json!({
            "s": "sine", "FX": [{ "room": 0.8, "roomsize": 0.5 }]
        }));
        e.add_value(0, local, orbit1());
        e.schedule(hap(0.0, 0.5, 0, 0.5));
        render(&mut e, 1);
        let applied = |e: &LiveEngine| {
            e.core.slots[0]
                .voice
                .stages()
                .iter()
                .find_map(|s| match s {
                    crate::chain::Stage::LocalReverb(r) => Some(r.reverb.applied()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(applied(&e), 0, "waiting for the worker");
        let reqs: Vec<IrRequest> = req_rx.try_iter().collect();
        assert_eq!(reqs.len(), 1);
        for r in reqs {
            built_tx.send(build_ir(r)).unwrap();
        }
        render(&mut e, 20);
        assert_eq!(applied(&e), 1, "landed on the voice");
        assert!(e.ir.as_ref().unwrap().waiting.is_empty());
        assert!(e.take_events().is_empty());
    }

    #[test]
    fn delaysync_follows_each_haps_cps() {
        // One value, two haps at cps 0.5 then 1.0: the echo lands half a cycle (1/4 of a
        // cycle of `delaysync`) after each, at 0.5 s and at 0.25 s.
        let echo = |cps: f64| {
            let mut e = LiveEngine::new(0);
            e.add_value(
                0,
                value_map(json!({ "s": "sine", "delay": 0.5, "delaysync": 0.25 })),
                orbit1(),
            );
            e.schedule(hap(0.0, 0.05, 0, cps));
            let out = render(&mut e, 200);
            let from = (0.2 * SR) as usize;
            from + first_nonzero(&out[from..]).unwrap()
        };
        let (slow, fast) = (echo(0.5), echo(1.0));
        assert!((22050..22050 + 256).contains(&slow), "{slow}");
        assert!((11025..11025 + 256).contains(&fast), "{fast}");
        // In one engine, the hap's own cps is used, not the first one's.
        let mut e = LiveEngine::new(0);
        e.add_value(
            0,
            value_map(json!({ "s": "sine", "delay": 0.5, "delaysync": 0.25 })),
            orbit1(),
        );
        e.schedule(hap(0.0, 0.05, 0, 0.5));
        e.schedule(hap(0.0, 0.05, 0, 1.0));
        assert_eq!(e.counters().haps, 2);
        assert_eq!(e.decided.len(), 2, "support is decided per cps");
    }
}

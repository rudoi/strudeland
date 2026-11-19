//! Superdough's buses (`SuperdoughAudioController.getBus`, superdoughoutput.mjs:238-243): one
//! 2-channel `GainNode(1)` per JS property key `String(bus)`, connected to nothing, fed by the
//! voices' `effectSend(post, bus, busgain)` (superdough.mjs:980-984) and read by the `bus` sound
//! (synth.mjs:374-407) and, from the modulators on, by `bmod`.
//!
//! [`BusGraph`] is the job-wide graph over the job's values (value level, not hap level): which
//! buses each value reads and sends to, the buses on a cycle (refused: the `bus-cycle`
//! signature) and, for an acyclic graph, the level each value renders at. [`BusBank`] holds one
//! quantum's feeders per bus and sums them in voice creation order.

use crate::QUANTUM;
use crate::controls::{js_to_string, prepare_value};
use crate::job::{EngineJob, Value, ValueMap};
use crate::orbit::{Input, sum_stereo};
use crate::voice::SendBuf;

/// The JS property key of a bus number (`buses[busNum]`): a number through `String(number)`, a
/// string as is, `true`/`false`, `null`, arrays and objects through their string; an absent value
/// (`undefined`) is the text `undefined`.
pub fn bus_key(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), js_to_string)
}

/// The bus a value sends to: `bus_key(value.bus)` when `bus != null` on the main value (chain
/// entries' `bus` is not read).
pub fn send_key(map: &ValueMap) -> Option<String> {
    match map.get("bus") {
        None | Some(Value::Null) => None,
        v => Some(bus_key(v)),
    }
}

/// The bus the `bus` sound reads: `getBus(value.n ?? 0)`.
pub fn sound_read_key(map: &ValueMap) -> String {
    match map.get("n") {
        None | Some(Value::Null) => bus_key(Some(&Value::Num(0.0))),
        v => bus_key(v),
    }
}

/// The buses a modulator map (`bmod`) reads: `bus_key(config.bus)` for every id of its `__ids`
/// set whose config is an object, resolved or not (conservative: a `bmod` whose target would skip
/// still counts).
fn bmod_reads(bmod: Option<&Value>, out: &mut Vec<String>) {
    let Some(Value::Obj(m)) = bmod else {
        return;
    };
    let Some(Value::Set(ids)) = m.get("__ids") else {
        return;
    };
    for id in ids {
        if let Some(Value::Obj(config)) = m.get(&js_to_string(id)) {
            out.push(bus_key(config.get("bus")));
        }
    }
}

/// The buses one (prepared) value reads, in order: the bus sound's (when `s` is exactly `bus`),
/// then each `bmod` id's bus on the main value and on every `FX` entry that is an object.
pub fn read_keys(map: &ValueMap) -> Vec<String> {
    let mut out = Vec::new();
    if matches!(map.get("s"), Some(Value::Str(s)) if s == "bus") {
        out.push(sound_read_key(map));
    }
    bmod_reads(map.get("bmod"), &mut out);
    if let Some(Value::Arr(entries)) = map.get("FX") {
        for e in entries {
            if let Value::Obj(e) = e {
                bmod_reads(e.get("bmod"), &mut out);
            }
        }
    }
    out
}

/// The job-wide bus graph: the buses, their levels and the cycle refusal.
#[derive(Debug, Clone, Default)]
pub struct BusGraph {
    /// Every bus key of the job, in first-seen order (the bank's dense indices).
    keys: Vec<String>,
    /// Per job value: the buses it reads (indices, deduplicated, in first-read order).
    reads: Vec<Vec<usize>>,
    /// Per job value: the bus it sends to.
    sends: Vec<Option<usize>>,
    /// Per bus: whether it lies on a directed cycle (a self-loop included).
    on_cycle: Vec<bool>,
    /// Per job value: its level (all 0 when a cycle exists).
    levels: Vec<u32>,
    max_level: u32,
    /// Bumped whenever the set of buses on a cycle changes (a decision cached against
    /// [`Self::cyclic_readers`] is stale after a bump).
    cycle_generation: u64,
}

impl BusGraph {
    /// The graph over `job.values`, each value as superdough sees it after its mutation (a
    /// silence marker builds no voice and has no edges; `bank` rewrites `s`).
    pub fn from_job(job: &EngineJob) -> BusGraph {
        let mut g = BusGraph::default();
        for map in &job.values {
            g.append(map);
        }
        g.recompute();
        g
    }

    /// Appends one value to the graph as [`Self::from_job`] does, then recomputes the cycles
    /// and levels (linear in values plus buses). Returns the value's index. A new value can
    /// raise the levels of earlier ones, or close a cycle (every level is then 0).
    pub fn push_value(&mut self, map: &ValueMap) -> usize {
        self.append(map);
        self.recompute();
        self.reads.len() - 1
    }

    /// The edges of one value (its reads and its send, keys interned in first-seen order),
    /// without the recompute.
    fn append(&mut self, map: &ValueMap) {
        let index = |keys: &mut Vec<String>, k: String| match keys.iter().position(|x| *x == k) {
            Some(i) => i,
            None => {
                keys.push(k);
                keys.len() - 1
            }
        };
        let (mut reads, mut send) = (Vec::new(), None);
        if let Some(v) = prepare_value(map, 0.0) {
            for k in read_keys(&v) {
                let i = index(&mut self.keys, k);
                if !reads.contains(&i) {
                    reads.push(i);
                }
            }
            send = send_key(&v).map(|k| index(&mut self.keys, k));
        }
        self.reads.push(reads);
        self.sends.push(send);
    }

    /// The cycles, then the levels when the graph is acyclic (all 0 otherwise).
    fn recompute(&mut self) {
        let on_cycle = self.find_cycles();
        // A new key starts off any cycle, so only a key whose membership flipped counts.
        let was = |i: usize| self.on_cycle.get(i).copied().unwrap_or(false);
        if on_cycle.iter().enumerate().any(|(i, &c)| c != was(i)) {
            self.cycle_generation += 1;
        }
        self.on_cycle = on_cycle;
        self.levels = vec![0; self.reads.len()];
        self.max_level = 0;
        if !self.on_cycle.contains(&true) {
            self.compute_levels();
        }
    }

    /// The edges `a -> b` (a value reads `a` and sends to `b`), as adjacency lists per bus.
    fn adjacency(&self) -> Vec<Vec<usize>> {
        let mut adj = vec![Vec::new(); self.keys.len()];
        for (reads, send) in self.reads.iter().zip(&self.sends) {
            if let Some(b) = *send {
                for &a in reads {
                    if !adj[a].contains(&b) {
                        adj[a].push(b);
                    }
                }
            }
        }
        adj
    }

    /// Tarjan's strongly connected components (iterative): a bus is on a cycle when its
    /// component has more than one bus or it has a self-loop.
    fn find_cycles(&self) -> Vec<bool> {
        let adj = self.adjacency();
        let n = adj.len();
        let mut on_cycle = vec![false; n];
        let mut idx = vec![usize::MAX; n];
        let mut low = vec![0usize; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut next = 0usize;
        for root in 0..n {
            if idx[root] != usize::MAX {
                continue;
            }
            // (node, next edge to visit)
            let mut work: Vec<(usize, usize)> = vec![(root, 0)];
            idx[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on_stack[root] = true;
            while let Some(&mut (v, ref mut e)) = work.last_mut() {
                if let Some(&w) = adj[v].get(*e) {
                    *e += 1;
                    if idx[w] == usize::MAX {
                        idx[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        work.push((w, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(idx[w]);
                    }
                    continue;
                }
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == idx[v] {
                    let mut component = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyclic = component.len() > 1 || adj[v].contains(&v);
                    for w in component {
                        on_cycle[w] = cyclic;
                    }
                }
            }
        }
        on_cycle
    }

    /// Levels of an acyclic graph: `bus_level(B)` is the largest level of the values that send
    /// to `B` (0 without one); a value's level is 0 when it reads no bus, else 1 + the largest
    /// level of the buses it reads. Buses are resolved in topological order (Kahn), so the
    /// recursive definition needs no stack.
    fn compute_levels(&mut self) {
        let adj = self.adjacency();
        let n = adj.len();
        let mut indegree = vec![0usize; n];
        for outs in &adj {
            for &b in outs {
                indegree[b] += 1;
            }
        }
        let mut senders: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (v, send) in self.sends.iter().enumerate() {
            if let Some(b) = *send {
                senders[b].push(v);
            }
        }
        let value_level = |reads: &[usize], bus_level: &[u32]| -> u32 {
            reads
                .iter()
                .map(|&a| bus_level[a].saturating_add(1))
                .max()
                .unwrap_or(0)
        };
        let mut bus_level = vec![0u32; n];
        let mut ready: Vec<usize> = (0..n).filter(|&b| indegree[b] == 0).collect();
        while let Some(b) = ready.pop() {
            bus_level[b] = senders[b]
                .iter()
                .map(|&v| value_level(&self.reads[v], &bus_level))
                .max()
                .unwrap_or(0);
            for &c in &adj[b] {
                indegree[c] -= 1;
                if indegree[c] == 0 {
                    ready.push(c);
                }
            }
        }
        for (v, reads) in self.reads.iter().enumerate() {
            self.levels[v] = value_level(reads, &bus_level);
        }
        self.max_level = self.levels.iter().copied().max().unwrap_or(0);
    }

    /// The bus keys on a cycle, in first-seen order (empty for an acyclic graph). A cyclic job is
    /// refused through [`Self::cyclic_readers`] before it renders.
    #[cfg(test)]
    pub fn cycles(&self) -> Vec<&str> {
        self.keys
            .iter()
            .zip(&self.on_cycle)
            .filter(|(_, c)| **c)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Per job value: whether it reads a bus that lies on a cycle (the `bus-cycle` rule).
    pub fn cyclic_readers(&self) -> Vec<bool> {
        self.reads
            .iter()
            .map(|r| r.iter().any(|&a| self.on_cycle[a]))
            .collect()
    }

    /// Changes whenever the buses on a cycle change (so [`Self::cyclic_readers`] may differ).
    pub fn cycle_generation(&self) -> u64 {
        self.cycle_generation
    }

    /// The level of job value `v` (0 for an unknown value or a cyclic graph).
    pub fn level_of_value(&self, v: usize) -> u32 {
        self.levels.get(v).copied().unwrap_or(0)
    }

    /// The largest value level: 0 when no value reads a bus.
    pub fn max_level(&self) -> u32 {
        self.max_level
    }

    /// Every bus key, in bank index order.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
}

type Stereo = [[f32; QUANTUM]; 2];

/// One quantum's buses: per bus its feeders (each a copy of a voice's scaled send with the
/// voice's creation sequence number) and the cached sum.
#[derive(Debug, Clone, Default)]
pub struct BusBank {
    keys: Vec<String>,
    feeders: Vec<Vec<(u64, SendBuf)>>,
    sums: Vec<Option<(Stereo, bool)>>,
}

impl BusBank {
    /// One bus per key of the graph (dense indices, allocated up front: superdough's creation
    /// time is unobservable).
    pub fn new(graph: &BusGraph) -> BusBank {
        BusBank::of_keys(graph.keys().to_vec())
    }

    /// Appends a bus for every key of `keys` past the bank's own (keys only ever append, so
    /// the bank's keys are a prefix of the graph's and existing indices stay).
    pub fn grow(&mut self, keys: &[String]) {
        for k in keys.iter().skip(self.keys.len()) {
            self.keys.push(k.clone());
            self.feeders.push(Vec::new());
            self.sums.push(None);
        }
    }

    /// A bank of the given keys.
    pub fn of_keys(keys: Vec<String>) -> BusBank {
        let n = keys.len();
        BusBank {
            keys,
            feeders: vec![Vec::new(); n],
            sums: vec![None; n],
        }
    }

    /// The index of the bus keyed `key`.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.keys.iter().position(|k| k == key)
    }

    /// Starts a quantum: clears the feeders and the cached sums.
    pub fn begin_quantum(&mut self) {
        for f in &mut self.feeders {
            f.clear();
        }
        for s in &mut self.sums {
            *s = None;
        }
    }

    /// A voice's send of this quantum into `bus` (already scaled by its `busgain` gain), with the
    /// voice's creation sequence number.
    pub fn add_send(&mut self, bus: usize, seq: u64, send: &SendBuf) {
        if let Some(f) = self.feeders.get_mut(bus) {
            f.push((seq, send.clone()));
            self.sums[bus] = None;
        }
    }

    /// The bus's output this quantum: its feeders sorted by creation sequence (stable) summed
    /// with the speakers up-mix (a silent feeder is skipped), then the bus `GainNode(1)` (an
    /// identity: no events, gain 1). A bus without feeders, or an unknown index, reads silent
    /// zeros. The sum is cached for the quantum.
    pub fn read(&mut self, bus: usize) -> (Stereo, bool) {
        let Some(feeders) = self.feeders.get_mut(bus) else {
            return ([[0.0; QUANTUM]; 2], true);
        };
        if let Some(s) = self.sums[bus] {
            return s;
        }
        feeders.sort_by_key(|(seq, _)| *seq);
        let mut out = [[0.0f32; QUANTUM]; 2];
        let mut silent = [true; 2];
        for (_, f) in feeders.iter() {
            sum_stereo(
                &mut out,
                &mut silent,
                Input {
                    channels: &f.ch[..f.channels.clamp(1, 2)],
                    silent: f.silent,
                },
            );
        }
        let result = (out, silent[0] && silent[1]);
        self.sums[bus] = Some(result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(values: serde_json::Value) -> EngineJob {
        let n = values.as_array().map_or(0, Vec::len);
        let haps: Vec<_> = (0..n)
            .map(|v| json!({ "t": 0, "duration": 0.1, "cycle": 0, "v": v, "event": v }))
            .collect();
        EngineJob::from_json(
            &json!({
                "schema": 2, "sampleRate": 44100, "frames": 128, "seed": 0, "cps": 0.5,
                "channels": { "count": 2, "multiChannelOrbits": false },
                "orbits": [{ "orbit": 1, "channels": [0, 1] }],
                "pauses": [{ "suspendSec": 0, "haps": haps }], "values": values
            })
            .to_string(),
        )
        .unwrap()
    }

    fn val(v: serde_json::Value) -> Value {
        Value::from_json(v).unwrap()
    }

    #[test]
    fn bus_key_is_the_js_property_key() {
        assert_eq!(bus_key(Some(&val(json!(1)))), "1");
        assert_eq!(bus_key(Some(&val(json!("1")))), "1");
        assert_eq!(bus_key(Some(&val(json!(1.0)))), "1");
        assert_eq!(bus_key(Some(&val(json!(1.5)))), "1.5");
        assert_eq!(bus_key(Some(&val(json!(-0.0)))), "0");
        assert_eq!(bus_key(Some(&val(json!(true)))), "true");
        assert_eq!(bus_key(Some(&val(json!([1])))), "1");
        assert_eq!(bus_key(Some(&val(json!([1, 2])))), "1,2");
        assert_eq!(bus_key(Some(&val(json!({ "a": 1 })))), "[object Object]");
        assert_eq!(bus_key(Some(&val(json!(null)))), "null");
        assert_eq!(bus_key(Some(&val(json!({ "$num": "NaN" })))), "NaN");
        assert_eq!(bus_key(None), "undefined");
    }

    #[test]
    fn sends_and_reads_of_a_value() {
        let m = |v: serde_json::Value| match val(v) {
            Value::Obj(m) => m,
            other => panic!("{other:?}"),
        };
        // `bus != null` sends, also for 0, '' and false.
        for (bus, key) in [(json!(0), "0"), (json!(""), ""), (json!(false), "false")] {
            assert_eq!(
                send_key(&m(json!({ "s": "sine", "bus": bus }))).as_deref(),
                Some(key)
            );
        }
        assert_eq!(send_key(&m(json!({ "s": "sine", "bus": null }))), None);
        assert_eq!(send_key(&m(json!({ "s": "sine" }))), None);
        // A chain entry's `bus` is not a send.
        assert_eq!(
            send_key(&m(json!({ "s": "sine", "FX": [{ "bus": 1 }] }))),
            None
        );
        // The bus sound reads `n ?? 0`.
        assert_eq!(read_keys(&m(json!({ "s": "bus" }))), ["0"]);
        assert_eq!(read_keys(&m(json!({ "s": "bus", "n": null }))), ["0"]);
        assert_eq!(read_keys(&m(json!({ "s": "bus", "n": "2" }))), ["2"]);
        assert!(read_keys(&m(json!({ "s": "sine", "n": 2 }))).is_empty());
        assert!(read_keys(&m(json!({ "s": "buss", "n": 2 }))).is_empty());
        // Every bmod id with an object config reads its bus (absent: `undefined`), on the main
        // value and on object FX entries; a non-object config or entry reads nothing.
        let bmod = json!({ "__ids": { "$set": [0, "a", 2] }, "0": { "bus": 3 }, "a": {}, "2": 5 });
        assert_eq!(
            read_keys(&m(json!({ "s": "sine", "bmod": bmod.clone() }))),
            ["3", "undefined"]
        );
        assert_eq!(
            read_keys(&m(
                json!({ "s": "bus", "n": 1, "FX": [{ "bmod": bmod }, 7] })
            )),
            ["1", "3", "undefined"]
        );
        assert!(read_keys(&m(json!({ "s": "sine", "bmod": { "0": { "bus": 1 } } }))).is_empty());
        assert!(read_keys(&m(json!({ "s": "sine", "bmod": 3 }))).is_empty());
    }

    #[test]
    fn levels_of_a_chain() {
        // A -> B -> C: a feeder of A, a reader of A feeding B, a reader of B feeding C, a reader
        // of C; and a voice that touches no bus.
        let g = BusGraph::from_job(&job(json!([
            { "s": "sine", "bus": "A" },
            { "s": "bus", "n": "A", "bus": "B" },
            { "s": "bus", "n": "B", "bus": "C" },
            { "s": "bus", "n": "C" },
            { "s": "sine" }
        ])));
        assert!(g.cycles().is_empty());
        let levels: Vec<u32> = (0..5).map(|v| g.level_of_value(v)).collect();
        assert_eq!(levels, [0, 1, 2, 3, 0]);
        assert_eq!(g.max_level(), 3);
        assert_eq!(g.keys(), ["A", "B", "C"]);
        // A reader of two buses takes the larger level + 1; a feeder of a bus at a higher level
        // than its other feeders raises the bus.
        let g = BusGraph::from_job(&job(json!([
            { "s": "sine", "bus": 1 },
            { "s": "bus", "n": 1, "bus": 2 },
            { "s": "sine", "bus": 2 },
            { "s": "bus", "n": 2, "bmod": { "__ids": { "$set": [0] }, "0": { "bus": 1 } } }
        ])));
        let levels: Vec<u32> = (0..4).map(|v| g.level_of_value(v)).collect();
        assert_eq!(levels, [0, 1, 0, 2]);
        assert_eq!(g.level_of_value(99), 0);
    }

    #[test]
    fn no_bus_has_level_zero() {
        let g = BusGraph::from_job(&job(json!([{ "s": "sine" }, { "s": "one", "dry": 0 }])));
        assert_eq!(g.max_level(), 0);
        assert!(g.keys().is_empty());
        assert!(g.cycles().is_empty());
        assert_eq!(g.cyclic_readers(), [false, false]);
        // A feeder without a reader is level 0 too.
        let g = BusGraph::from_job(&job(json!([{ "s": "sine", "bus": 1 }])));
        assert_eq!(g.max_level(), 0);
        assert_eq!(g.keys(), ["1"]);
    }

    #[test]
    fn silence_markers_and_bank_rewrites_have_no_edges() {
        let g = BusGraph::from_job(&job(json!([
            { "s": "~", "bus": 1 },
            { "s": "bus", "n": 1, "bus": 1, "bank": "x" },
            { "s": "-", "n": 1 }
        ])));
        assert!(
            g.cycles().is_empty(),
            "x_bus reads nothing, ~ sends nothing"
        );
        assert_eq!(g.cyclic_readers(), [false, false, false]);
        assert_eq!(g.max_level(), 0);
    }

    #[test]
    fn cycles() {
        // A self-loop: a voice reads and sends the same bus.
        let g = BusGraph::from_job(&job(json!([{ "s": "bus", "n": 1, "bus": 1 }])));
        assert_eq!(g.cycles(), ["1"]);
        assert_eq!(g.cyclic_readers(), [true]);
        assert_eq!(g.max_level(), 0, "no levels with a cycle");
        // A two-bus loop, with a reader outside the loop of a bus on it and an unrelated reader.
        let g = BusGraph::from_job(&job(json!([
            { "s": "bus", "n": 1, "bus": 2 },
            { "s": "bus", "n": 2, "bus": 1 },
            { "s": "bus", "n": 1 },
            { "s": "sine", "bus": 3 },
            { "s": "bus", "n": 3 }
        ])));
        assert_eq!(g.cycles(), ["1", "2"]);
        assert_eq!(g.cyclic_readers(), [true, true, true, false, false]);
        // A three-bus loop.
        let g = BusGraph::from_job(&job(json!([
            { "s": "bus", "n": "a", "bus": "b" },
            { "s": "bus", "n": "b", "bus": "c" },
            { "s": "bus", "n": "c", "bus": "a" }
        ])));
        assert_eq!(g.cycles(), ["a", "b", "c"]);
        // A diamond is no cycle: a -> b, a -> c, b -> d, c -> d.
        let g = BusGraph::from_job(&job(json!([
            { "s": "sine", "bus": "a" },
            { "s": "bus", "n": "a", "bus": "b" },
            { "s": "bus", "n": "a", "bus": "c" },
            { "s": "bus", "n": "b", "bus": "d" },
            { "s": "bus", "n": "c", "bus": "d" },
            { "s": "bus", "n": "d" }
        ])));
        assert!(g.cycles().is_empty());
        let levels: Vec<u32> = (0..6).map(|v| g.level_of_value(v)).collect();
        assert_eq!(levels, [0, 1, 1, 2, 2, 3]);
        // A bmod read without a resolved target still counts (conservative): a sine whose bmod
        // reads bus 1 and which sends to bus 1.
        let g = BusGraph::from_job(&job(json!([{
            "s": "sine", "bus": 1,
            "bmod": { "__ids": { "$set": [0] }, "0": { "control": "nope", "bus": 1 } }
        }])));
        assert_eq!(g.cycles(), ["1"]);
        assert_eq!(g.cyclic_readers(), [true]);
        // A bmod inside an FX entry reading the voice's own send bus.
        let g = BusGraph::from_job(&job(json!([{
            "s": "sine", "bus": 2,
            "FX": [{ "bmod": { "__ids": { "$set": [0] }, "0": { "bus": 2 } } }]
        }])));
        assert_eq!(g.cycles(), ["2"]);
        assert_eq!(
            BusGraph::from_job(&job(json!([{ "s": "bus", "n": 4, "bus": 4 }]))).cyclic_readers(),
            [true]
        );
    }

    #[test]
    fn a_long_chain_needs_no_recursion() {
        let values: Vec<_> = (0..5000)
            .map(|i| json!({ "s": "bus", "n": i, "bus": i + 1 }))
            .collect();
        let g = BusGraph::from_job(&job(json!(values)));
        assert!(g.cycles().is_empty());
        assert_eq!(g.max_level(), 5000);
        let mut values = values;
        values.push(json!({ "s": "bus", "n": 5000, "bus": 0 }));
        let g = BusGraph::from_job(&job(json!(values)));
        assert_eq!(g.cycles().len(), 5001);
    }

    /// The graph built one value at a time with `push_value`.
    fn pushed(j: &EngineJob) -> BusGraph {
        let mut g = BusGraph::default();
        for (k, map) in j.values.iter().enumerate() {
            assert_eq!(g.push_value(map), k);
        }
        g
    }

    fn same_graph(a: &BusGraph, b: &BusGraph) {
        assert_eq!(a.keys, b.keys);
        assert_eq!(a.reads, b.reads);
        assert_eq!(a.sends, b.sends);
        assert_eq!(a.on_cycle, b.on_cycle);
        assert_eq!(a.levels, b.levels);
        assert_eq!(a.max_level, b.max_level);
    }

    #[test]
    fn pushing_values_one_by_one_builds_the_job_graph() {
        for values in [
            json!([
                { "s": "sine", "bus": "A" },
                { "s": "bus", "n": "A", "bus": "B" },
                { "s": "bus", "n": "B", "bus": "C" },
                { "s": "bus", "n": "C" },
                { "s": "sine" }
            ]),
            json!([
                { "s": "bus", "n": 1, "bus": 2 },
                { "s": "bus", "n": 2, "bus": 1 },
                { "s": "bus", "n": 1 },
                { "s": "~", "bus": 3 }
            ]),
            json!([]),
        ] {
            let j = job(values);
            same_graph(&pushed(&j), &BusGraph::from_job(&j));
        }
    }

    #[test]
    fn a_pushed_value_raises_earlier_levels_and_can_close_a_cycle() {
        let mut g = BusGraph::default();
        let m = |v: serde_json::Value| match val(v) {
            Value::Obj(m) => m,
            other => panic!("{other:?}"),
        };
        // A reader of A feeding B, then a reader of B: levels 1 and 2.
        g.push_value(&m(json!({ "s": "bus", "n": "A", "bus": "B" })));
        g.push_value(&m(json!({ "s": "bus", "n": "B" })));
        assert_eq!((g.level_of_value(0), g.level_of_value(1)), (1, 2));
        assert_eq!(g.max_level(), 2);
        // A value reading C and feeding A raises bus A, so both earlier values move up.
        assert_eq!(
            g.push_value(&m(json!({ "s": "bus", "n": "C", "bus": "A" }))),
            2
        );
        let levels: Vec<u32> = (0..3).map(|v| g.level_of_value(v)).collect();
        assert_eq!(levels, [2, 3, 1]);
        assert_eq!(g.max_level(), 3);
        assert_eq!(g.keys(), ["A", "B", "C"]);
        assert_eq!(g.cycle_generation(), 0, "new keys alone close no cycle");
        // Feeding C from B closes A -> B -> C -> A: every level drops to 0.
        g.push_value(&m(json!({ "s": "bus", "n": "B", "bus": "C" })));
        assert_eq!(g.cycle_generation(), 1);
        assert_eq!(g.cycles(), ["A", "B", "C"]);
        assert_eq!(g.cyclic_readers(), [true, true, true, true]);
        assert_eq!(g.max_level(), 0);
        assert!((0..4).all(|v| g.level_of_value(v) == 0));
    }

    #[test]
    fn the_bank_grows_with_the_graph() {
        let m = |v: serde_json::Value| match val(v) {
            Value::Obj(m) => m,
            other => panic!("{other:?}"),
        };
        let mut g = BusGraph::default();
        let mut bank = BusBank::new(&g);
        g.push_value(&m(json!({ "s": "sine", "bus": 1 })));
        bank.grow(g.keys());
        assert_eq!(bank.index_of("1"), Some(0));
        bank.begin_quantum();
        bank.add_send(0, 0, &send(0.5, None, false));
        g.push_value(&m(json!({ "s": "bus", "n": 2, "bus": 1 })));
        bank.grow(g.keys());
        bank.grow(g.keys());
        assert_eq!(bank.keys, ["1", "2"], "growing twice adds nothing");
        // The existing bus keeps its index and its feeders; the new one reads silence.
        assert_eq!(bank.read(0).0[0][0], 0.5);
        assert!(bank.read(1).1);
        bank.add_send(1, 1, &send(0.25, None, false));
        assert_eq!(bank.read(1).0[1][3], 0.25);
    }

    fn send(ch0: f32, ch1: Option<f32>, silent: bool) -> SendBuf {
        let mut b = SendBuf {
            ch: [[0.0; QUANTUM]; 2],
            channels: if ch1.is_some() { 2 } else { 1 },
            silent,
        };
        b.ch[0] = [ch0; QUANTUM];
        b.ch[1] = [ch1.unwrap_or(f32::NAN); QUANTUM];
        b
    }

    #[test]
    fn the_bank_sums_feeders_in_creation_order() {
        let mut bank = BusBank::of_keys(vec!["1".into(), "2".into()]);
        assert_eq!(bank.index_of("2"), Some(1));
        assert_eq!(bank.index_of("3"), None);
        // No feeder: silent zeros.
        let (out, silent) = bank.read(0);
        assert!(silent && out.iter().flatten().all(|x| *x == 0.0));
        // Feeders added out of creation order (a later-level voice created first still sums
        // first): float addition is not associative, so the order shows.
        bank.begin_quantum();
        bank.add_send(0, 2, &send(1e8, Some(1e8), false));
        bank.add_send(0, 9, &send(1.0, None, false));
        bank.add_send(0, 5, &send(-1e8, Some(-1e8), false));
        let (out, silent) = bank.read(0);
        assert!(!silent);
        // seq order 2, 5, 9: (1e8 - 1e8) + 1 = 1; the add order would give (1e8 + 1) - 1e8 = 0.
        assert_eq!(out[0][0], 1.0);
        assert_eq!(
            out[1][0], 1.0,
            "the mono feeder up-mixes into both channels"
        );
        // A silent feeder is skipped (its stale samples never land); the cached sum is stable.
        bank.begin_quantum();
        bank.add_send(1, 1, &send(f32::NAN, None, true));
        bank.add_send(1, 3, &send(0.25, Some(-0.5), false));
        let (out, silent) = bank.read(1);
        assert!(!silent);
        assert_eq!((out[0][7], out[1][7]), (0.25, -0.5));
        assert_eq!(bank.read(1), (out, silent));
        // Only silent feeders: silent; -0 is kept from a copied feeder.
        bank.begin_quantum();
        bank.add_send(1, 1, &send(-0.0, None, false));
        let (out, silent) = bank.read(1);
        assert!(!silent && out[0][0].is_sign_negative());
        bank.begin_quantum();
        bank.add_send(1, 1, &send(1.0, None, true));
        assert!(bank.read(1).1);
        // begin_quantum clears the feeders; an unknown index reads silence and ignores sends.
        bank.begin_quantum();
        assert!(bank.read(0).1 && bank.read(1).1);
        bank.add_send(7, 0, &send(1.0, None, false));
        assert!(bank.read(7).1);
    }
}

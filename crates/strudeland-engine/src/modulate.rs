//! The `lfo`, `env` and `bmod` modulators (superdough's `modulators.mjs` and the modulator loop
//! of `superdough()`, superdough.mjs:1006-1051, with the `modulator-fxi-copy` and `bmod-gate`
//! patches): their configs from the hap value, target resolution over the voice's [`NodeTable`]
//! (`getControlData`, `getNodeParam`, `getTargetParamsForControl`), `currentValue`, depth and
//! range, the processors they build (`getLfo`, `getEnvelope`, the patched bus-modulator graph),
//! and the connections they make: [`ModSet`] renders each modulator once per quantum and hands
//! its output to the params it is connected to through [`ModView`], in connection order after
//! each param's own internal inputs.
//!
//! What superdough throws on is a `hap` error that stops the loop (the modulators
//! connected before it stay). What it logs and skips (`errorLogger`: no control data, no node
//! under the key) connects nothing and is reported as superdough's error log (kind `other`, the
//! render page's text). What the engine cannot render (a `frequency` target that falls back to
//! the `detune` of a noise source or a noise FM modulator) is [`Resolution::Unsupported`]: the
//! job is refused by its signature (`<type>:<control>`, [`unsupported_targets`]).

use crate::QUANTUM;
use crate::bus::{BusBank, bus_key};
use crate::chain::{
    FxKey, Node, NodeList, NodeName, NodeTable, ParamRef, ParamSlot, SourceKind, StageKind,
};
use crate::coerce::{js_to_number_full, js_to_string, parse_js_number};
use crate::envelope::{self, EnvProps, Envelope, get_envelope};
use crate::job::{Value, ValueMap};
use crate::js::number_to_string;
use crate::lfo::{self, Lfo, LfoProps, ShapeInput, get_lfo};
use crate::osc::ConstantSource;
use crate::param::{Param, ParamInputs};
use crate::params::VoiceParams;
use crate::voice::{GainNode, VoiceError, hap_error, idl_float};

/// The modulator types, in the order the loop runs them inside an FX entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModType {
    Lfo,
    Env,
    Bmod,
}

impl ModType {
    pub const ALL: [ModType; 3] = [ModType::Lfo, ModType::Env, ModType::Bmod];

    /// The hap value key (`lfo`, `env`, `bmod`).
    pub fn key(self) -> &'static str {
        match self {
            ModType::Lfo => "lfo",
            ModType::Env => "env",
            ModType::Bmod => "bmod",
        }
    }
}

/// `CONTROL_TARGETS` (superdoughdata.mjs:8-146): control -> (node key, param name).
const TARGETS: &[(&str, &str, &str)] = &[
    ("stretch", "stretch", "pitchFactor"),
    ("gain", "gain", "gain"),
    ("postgain", "post", "gain"),
    ("pan", "pan", "pan"),
    ("tremolo", "tremolo", "frequency"),
    ("tremolosync", "tremolo", "frequency"),
    ("tremolodepth", "tremolo_gain", "gain"),
    ("tremoloskew", "tremolo", "skew"),
    ("tremolophase", "tremolo", "phase"),
    ("tremoloshape", "tremolo", "shape"),
    ("lfo", "lfo", "frequency"),
    ("lfo_rate", "lfo", "frequency"),
    ("lfo_sync", "lfo", "frequency"),
    ("lfo_depth", "lfo", "depth"),
    ("lfo_depthabs", "lfo", "depth"),
    ("lfo_skew", "lfo", "skew"),
    ("lfo_curve", "lfo", "curve"),
    ("lfo_dcoffset", "lfo", "dcoffset"),
    ("env", "env", "depth"),
    ("env_attack", "env", "attack"),
    ("env_decay", "env", "decay"),
    ("env_sustain", "env", "sustain"),
    ("env_release", "env", "release"),
    ("bmod", "bmod", "depth"),
    ("bmod_depth", "bmod", "depth"),
    ("bmod_depthabs", "bmod", "depth"),
    ("cutoff", "lpf", "frequency"),
    ("resonance", "lpf", "Q"),
    ("lprate", "lpf_lfo", "rate"),
    ("lpsync", "lpf_lfo", "sync"),
    ("lpdepth", "lpf_lfo", "depth"),
    ("lpdepthfrequency", "lpf_lfo", "depth"),
    ("lpshape", "lpf_lfo", "shape"),
    ("lpdc", "lpf_lfo", "dcoffset"),
    ("lpskew", "lpf_lfo", "skew"),
    ("hcutoff", "hpf", "frequency"),
    ("hresonance", "hpf", "Q"),
    ("hprate", "hpf_lfo", "rate"),
    ("hpsync", "hpf_lfo", "sync"),
    ("hpdepth", "hpf_lfo", "depth"),
    ("hpdepthfrequency", "hpf_lfo", "depth"),
    ("hpshape", "hpf_lfo", "shape"),
    ("hpdc", "hpf_lfo", "dcoffset"),
    ("hpskew", "hpf_lfo", "skew"),
    ("bandf", "bpf", "frequency"),
    ("bandq", "bpf", "Q"),
    ("bprate", "bpf_lfo", "rate"),
    ("bpsync", "bpf_lfo", "sync"),
    ("bpdepth", "bpf_lfo", "depth"),
    ("bpdepthfrequency", "bpf_lfo", "depth"),
    ("bpshape", "bpf_lfo", "shape"),
    ("bpdc", "bpf_lfo", "dcoffset"),
    ("bpskew", "bpf_lfo", "skew"),
    ("vowel", "vowel", "frequency"),
    ("coarse", "coarse", "coarse"),
    ("crush", "crush", "crush"),
    ("shape", "shape", "shape"),
    ("shapevol", "shape", "postgain"),
    ("distort", "distort", "distort"),
    ("distortvol", "distort", "postgain"),
    ("distorttype", "distort", "distort"),
    ("compressor", "compressor", "threshold"),
    ("compressorRatio", "compressor", "ratio"),
    ("compressorKnee", "compressor", "knee"),
    ("compressorAttack", "compressor", "attack"),
    ("compressorRelease", "compressor", "release"),
    ("phaserrate", "phaser_lfo", "frequency"),
    ("phasersweep", "phaser_lfo", "depth"),
    ("phasercenter", "phaser", "frequency"),
    ("phaserdepth", "phaser", "Q"),
    ("delay", "delay_mix", "gain"),
    ("delaytime", "delay", "delayTime"),
    ("delayfeedback", "delay", "feedback"),
    ("delaysync", "delay", "delayTime"),
    ("dry", "dry", "gain"),
    ("room", "room_mix", "gain"),
    ("djf", "djf", "value"),
    ("busgain", "bus", "gain"),
    ("s", "source", "frequency"),
    ("detune", "source", "freqspread"),
    ("wt", "source", "position"),
    ("warp", "source", "warp"),
    ("freq", "source", "frequency"),
    ("note", "source", "frequency"),
    ("wtdc", "wt_lfo", "dc"),
    ("wtskew", "wt_lfo", "skew"),
    ("wtrate", "wt_lfo", "frequency"),
    ("wtsync", "wt_lfo", "frequency"),
    ("wtdepth", "wt_lfo", "depth"),
    ("warpdc", "warp_lfo", "dc"),
    ("warpskew", "warp_lfo", "skew"),
    ("warprate", "warp_lfo", "frequency"),
    ("warpsync", "warp_lfo", "frequency"),
    ("warpdepth", "warp_lfo", "depth"),
    ("fmi", "fm_1_gain", "gain"),
    ("fmi2", "fm_2_gain", "gain"),
    ("fmi3", "fm_3_gain", "gain"),
    ("fmi4", "fm_4_gain", "gain"),
    ("fmi5", "fm_5_gain", "gain"),
    ("fmi6", "fm_6_gain", "gain"),
    ("fmi7", "fm_7_gain", "gain"),
    ("fmi8", "fm_8_gain", "gain"),
    ("fmh", "fm_1", "frequency"),
    ("fmh2", "fm_2", "frequency"),
    ("fmh3", "fm_3", "frequency"),
    ("fmh4", "fm_4", "frequency"),
    ("fmh5", "fm_5", "frequency"),
    ("fmh6", "fm_6", "frequency"),
    ("fmh7", "fm_7", "frequency"),
    ("fmh8", "fm_8", "frequency"),
    ("pw", "source", "pulsewidth"),
    ("pwrate", "pw_lfo", "frequency"),
    ("pwsweep", "pw_lfo", "depth"),
    ("vib", "vib", "frequency"),
    ("vibmod", "vib_gain", "gain"),
    ("byteBeatStartTime", "source", "byteBeatStartTime"),
    ("spread", "source", "panspread"),
    ("transient", "transient", "attack"),
];

/// A control's target: the node key and the param name (`targetInfo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlData {
    pub node: &'static str,
    pub param: &'static str,
}

fn target(key: &str) -> Option<ControlData> {
    TARGETS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, node, param)| ControlData { node, param })
}

/// `getControlData(control, subControl)`: `TARGETS[<head>_<subControl>] ??
/// TARGETS[<head>]` with `head = control.split('_')[0]` and `sub` the template text of
/// `subControl` (`undefined` when absent).
pub fn get_control_data(control: &str, sub: &str) -> Option<ControlData> {
    let head = control.split('_').next().unwrap_or("");
    target(&format!("{head}_{sub}")).or_else(|| target(head))
}

/// `getRangeForParam(paramName, currentValue)`: `{ 20 - cv, 24000 - cv }` for a `frequency`
/// param (the data table's name) with `cv >= 30`, else both `undefined`.
pub fn get_range_for_param(param: &str, cv: f64) -> Option<(f64, f64)> {
    (param == "frequency" && cv >= 30.0).then_some((20.0 - cv, 24000.0 - cv))
}

const OSC: &[&str] = &["frequency", "detune"];
const NOISE: &[&str] = &["detune", "playbackRate"];
const CONSTANT: &[&str] = &["offset"];
const BIQUAD: &[&str] = &["frequency", "Q", "gain", "detune"];
const LADDER: &[&str] = &["frequency", "q", "drive"];
const GAIN: &[&str] = &["gain"];
const PAN: &[&str] = &["pan"];
const COARSE: &[&str] = &["coarse"];
const CRUSH: &[&str] = &["crush"];
const SHAPE: &[&str] = &["shape", "postgain"];
const DISTORT: &[&str] = &["distort", "postgain"];
const COMPRESSOR: &[&str] = &["threshold", "ratio", "knee", "attack", "release"];
/// `lfo-processor`'s descriptors, in [`lfo::BEGIN`] .. [`lfo::MAX`] order.
const LFO: &[&str] = &[
    "begin",
    "time",
    "end",
    "frequency",
    "skew",
    "depth",
    "phaseoffset",
    "shape",
    "curve",
    "dcoffset",
    "min",
    "max",
];
/// `envelope-processor`'s descriptors, in [`envelope::BEGIN`] .. [`envelope::RETRIGGER`] order.
const ENV: &[&str] = &[
    "begin",
    "end",
    "attack",
    "decay",
    "sustain",
    "release",
    "attackCurve",
    "decayCurve",
    "releaseCurve",
    "depth",
    "min",
    "max",
    "retrigger",
];
const SUPERSAW: &[&str] = &[
    "begin",
    "end",
    "frequency",
    "panspread",
    "freqspread",
    "detune",
    "voices",
];
const PULSE: &[&str] = &["begin", "end", "frequency", "detune", "pulsewidth"];
const DELAY: &[&str] = &["delayTime", "feedback"];
const DJF: &[&str] = &["value"];
const NONE: &[&str] = &[];

/// The params a node exposes, in slot order.
fn params_of(table: &NodeTable, node: Node) -> &'static [&'static str] {
    match node {
        Node::Stage(id) => match table.stage(id) {
            Some(StageKind::Gain | StageKind::Release | StageKind::Tremolo) => GAIN,
            Some(StageKind::Biquad | StageKind::Phaser) => BIQUAD,
            Some(StageKind::Ladder) => LADDER,
            Some(StageKind::Panner) => PAN,
            Some(StageKind::Coarse) => COARSE,
            Some(StageKind::Crush) => CRUSH,
            Some(StageKind::Shape) => SHAPE,
            Some(StageKind::Distort) => DISTORT,
            Some(StageKind::Compressor) => COMPRESSOR,
            Some(StageKind::LocalDelay) => DELAY,
            Some(StageKind::Transient | StageKind::Vowel | StageKind::LocalReverb) | None => NONE,
        },
        Node::StageWet(_) => GAIN,
        Node::VowelFilter(..) => BIQUAD,
        Node::FmAmount(_)
        | Node::Detached { .. }
        | Node::VibratoGain
        | Node::Bus
        | Node::DelaySend
        | Node::RoomSend => GAIN,
        Node::Source => match table.source() {
            SourceKind::Osc => OSC,
            SourceKind::Noise => NOISE,
            SourceKind::Constant => CONSTANT,
            SourceKind::Supersaw => SUPERSAW,
            SourceKind::Pulse => PULSE,
            SourceKind::Bus => GAIN,
        },
        Node::Vibrato => OSC,
        Node::FmOsc(pos) => {
            if table.fm_is_noise(pos) {
                NOISE
            } else {
                OSC
            }
        }
        Node::StageLfo(_) | Node::PulseLfo | Node::Lfo(_) => LFO,
        Node::Env(_) => ENV,
        Node::OrbitDelay => DELAY,
        Node::OrbitDjf => DJF,
        Node::OrbitReverb => NONE,
    }
}

/// `getNodeParam(node, name)`: the node's own param by exact name; for `frequency` on a node
/// without one, its `detune`, else its `playbackRate`. The flag is whether that fallback was
/// taken.
pub fn node_param(table: &NodeTable, node: Node, name: &str) -> Option<(ParamRef, bool)> {
    let names = params_of(table, node);
    let find = |n: &str| {
        names.iter().position(|p| *p == n).map(|k| ParamRef {
            node,
            slot: ParamSlot(k as u8),
        })
    };
    if let Some(r) = find(name) {
        return Some((r, false));
    }
    if name == "frequency" {
        return find("detune")
            .or_else(|| find("playbackRate"))
            .map(|r| (r, true));
    }
    None
}

/// The outcome of resolving a modulator's target.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Superdough logs the message (its `errorLogger` text) and connects nothing.
    Skip(Option<String>),
    /// The engine cannot render it (the `detune` fallback of a noise source).
    Unsupported,
    /// The params to connect (never empty) and the data table's param name.
    Targets {
        param: &'static str,
        refs: Vec<ParamRef>,
    },
}

/// `errorLogger(e, 'superdough')` as the render page captures it.
fn superdough_log(message: &str) -> String {
    format!("[superdough] error: {message}")
}

fn type_error(message: &str) -> VoiceError {
    hap_error(format!("TypeError: {message}"))
}

/// The FX entry a modulator's `fxi` names (`nodeTracker[fxi]`), `None` when there is none.
fn entry_of(table: &NodeTable, fxi: &Value) -> Option<FxKey> {
    let text = js_to_string(fxi);
    if text == "main" {
        return Some(FxKey::Main);
    }
    let n: usize = text.parse().ok()?;
    let key = FxKey::Index(n);
    (n.to_string() == text && table.has_entry(key)).then_some(key)
}

/// `getTargetParamsForControl(control, nodeTracker[fxi], subControl)` and the first reads of
/// its result: a non-string `control` throws (`control.split`); a
/// control without data is a logged skip; an `fxi` naming no FX entry throws; a node key the
/// entry lacks is a logged skip; a bare node throws (`forEach`); an element without the param
/// throws (`targetParams[0].value`, or `connect(undefined)` later); a first element reached
/// through the noise `detune` fallback is unsupported.
pub fn resolve(
    table: &NodeTable,
    control: Option<&Value>,
    sub_control: Option<&Value>,
    fxi: &Value,
) -> Result<Resolution, VoiceError> {
    let control = match control {
        Some(Value::Str(c)) => c.as_str(),
        None => {
            return Err(type_error(
                "Cannot read properties of undefined (reading 'split')",
            ));
        }
        Some(_) => return Err(type_error("control.split is not a function")),
    };
    let sub = sub_control.map_or_else(|| "undefined".to_string(), js_to_string);
    let Some(info) = get_control_data(control, &sub) else {
        return Ok(Resolution::Skip(Some(superdough_log(&format!(
            "Could not find control data for target '{control}'. It may not be modulatable."
        )))));
    };
    let Some(key) = entry_of(table, fxi) else {
        return Err(type_error(&format!(
            "Cannot read properties of undefined (reading '{}')",
            info.node
        )));
    };
    let node_key = if table.get(key, info.node).is_some() {
        info.node
    } else {
        control
    };
    let Some(list) = table.get(key, node_key) else {
        return Ok(Resolution::Skip(Some(superdough_log(&format!(
            "Could not connect to target '{node_key}' \u{2014} it does not exist. Available \
             targets: {}",
            table.keys(key).join(", ")
        )))));
    };
    let NodeList::Array(elements) = list else {
        return Err(type_error("targetNodes.forEach is not a function"));
    };
    if elements.is_empty() {
        return Ok(Resolution::Skip(None));
    }
    let mut refs = Vec::with_capacity(elements.len());
    let mut fallback = false;
    for (k, e) in elements.iter().enumerate() {
        match e.and_then(|n| node_param(table, n, info.param)) {
            Some((r, fb)) => {
                if k == 0 {
                    fallback = fb;
                }
                refs.push(r);
            }
            None if k == 0 => {
                return Err(type_error(&format!(
                    "Cannot read properties of undefined (reading 'value'): '{control}' has no \
                     param '{}'",
                    info.param
                )));
            }
            None => {
                return Err(type_error(&format!(
                    "Failed to execute 'connect' on 'AudioNode': '{control}' element {k} has no \
                     param '{}'",
                    info.param
                )));
            }
        }
    }
    let noise = |n: Option<Node>| match n {
        Some(Node::Source) => table.source() == SourceKind::Noise,
        Some(Node::FmOsc(pos)) => table.fm_is_noise(pos),
        _ => false,
    };
    if fallback && noise(elements[0]) {
        return Ok(Resolution::Unsupported);
    }
    Ok(Resolution::Targets {
        param: info.param,
        refs,
    })
}

// ---- configs

/// The sub-keys of an `lfo` config that `connectLFO` and `getLfo` read or overwrite (any other
/// key reaches `parameters.get(key).value =` and throws), sorted.
pub const LFO_KEYS: &[&str] = &[
    "begin",
    "control",
    "cps",
    "curve",
    "cycle",
    "dcoffset",
    "depth",
    "depthabs",
    "end",
    "frequency",
    "fxi",
    "max",
    "min",
    "phaseoffset",
    "rate",
    "retrig",
    "shape",
    "skew",
    "subControl",
    "sync",
    "time",
];

/// The sub-keys of an `env` config that `connectEnvelope` and the `envelope-processor` read or
/// overwrite (any other key throws), sorted.
pub const ENV_KEYS: &[&str] = &[
    "acurve",
    "attack",
    "attackCurve",
    "begin",
    "control",
    "dcurve",
    "decay",
    "decayCurve",
    "depth",
    "depthabs",
    "end",
    "fxi",
    "max",
    "min",
    "rcurve",
    "release",
    "releaseCurve",
    "retrigger",
    "subControl",
    "sustain",
];

/// The sub-keys `connectBusModulator` reads (it ignores every other key), sorted.
pub const BMOD_KEYS: &[&str] = &[
    "bus",
    "control",
    "dc",
    "depth",
    "depthabs",
    "fxi",
    "subControl",
];

/// The first key of `config` a modulator of type `ty` cannot take (`None` for `bmod`).
fn bad_key(ty: ModType, config: &ValueMap) -> Option<&str> {
    let keys = match ty {
        ModType::Lfo => LFO_KEYS,
        ModType::Env => ENV_KEYS,
        ModType::Bmod => return None,
    };
    config
        .keys()
        .map(String::as_str)
        .find(|k| keys.binary_search(k).is_err())
}

/// One modulator config of the loop.
#[derive(Debug, Clone, Copy)]
pub struct Config<'a> {
    /// The FX entry whose value carries it (its `fxi` default, `params.fxi ?? key`).
    pub entry: FxKey,
    pub ty: ModType,
    /// The member of `__ids` (a number or a string).
    pub id: &'a Value,
    pub config: &'a ValueMap,
}

/// The configs of one modulator value (`{ __ids: Set, [id]: config }`) of the FX entry `entry`
/// in `__ids` order; `None` when the value is not of that shape (not an object, no `$set`
/// `__ids`, an id without an object config: the key is in the unsupported signature).
pub fn configs_of(entry: FxKey, ty: ModType, value: &Value) -> Option<Vec<Config<'_>>> {
    let Value::Obj(m) = value else {
        return None;
    };
    let Some(Value::Set(ids)) = m.get("__ids") else {
        return None;
    };
    ids.iter()
        .map(|id| match m.get(&js_to_string(id)) {
            Some(Value::Obj(config)) => Some(Config {
                entry,
                ty,
                id,
                config,
            }),
            _ => None,
        })
        .collect()
}

/// The loop's configs of one FX entry `entry` (its `lfo`, `env`, `bmod` values in `map`): every
/// `lfo`, then every `env`, then every `bmod` (`None`: a modulator value of the wrong shape).
fn entry_configs(entry: FxKey, map: &ValueMap) -> Option<Vec<Config<'_>>> {
    let mut out = Vec::new();
    for ty in ModType::ALL {
        if let Some(v) = map.get(ty.key()) {
            out.extend(configs_of(entry, ty, v)?);
        }
    }
    Some(out)
}

/// The loop's configs of a whole voice in superdough's order (`FX.forEach`): each chain entry's,
/// then the main entry's (`None`: a modulator value of the wrong shape in any entry).
fn loop_configs(params: &VoiceParams) -> Option<Vec<Config<'_>>> {
    let mut out = Vec::new();
    for (key, e) in params.entries() {
        out.extend(entry_configs(key, &e.modulators)?);
    }
    Some(out)
}

/// Whether `value` (an `lfo`, `env` or `bmod` value) has the shape the loop can iterate.
pub fn shape_ok(ty: ModType, value: &Value) -> bool {
    configs_of(FxKey::Main, ty, value).is_some()
}

/// The effective `fxi` of a config of the entry `key` (`params.fxi ?? key`: the string `'main'`,
/// or a chain entry's number index from `FX.forEach`).
fn fxi_of(config: &ValueMap, key: FxKey) -> Value {
    match config.get("fxi") {
        Some(v) if !matches!(v, Value::Null) => v.clone(),
        _ => match key {
            FxKey::Main => Value::Str(key.text()),
            FxKey::Index(n) => Value::Num(n as f64),
        },
    }
}

/// The `control` of a config: an `lfo` defaults to `'lfo'` (`control = 'lfo'`), the others have
/// no default.
fn control_of(ty: ModType, config: &ValueMap) -> Option<Value> {
    match config.get("control") {
        Some(v) => Some(v.clone()),
        None if ty == ModType::Lfo => Some(Value::Str("lfo".into())),
        None => None,
    }
}

/// The `<type>:<control>` labels (`FX.<type>:<control>` for a chain entry's) of one value's
/// modulators whose target the engine cannot render (unsupported signature keys), resolved on
/// [`NodeTable::plan`]'s table in the loop's order as the build does: a throwing config (the
/// resolution, or a key its processor lacks) ends the loop (the render reports it), a skip
/// connects nothing, a resolved `lfo`/`env` registers its node for later configs. A value whose
/// controls the build rejects, a sound it does not build, or a modulator value of the wrong shape
/// (its own signature) has no label.
pub fn unsupported_targets(map: &ValueMap, cps: f64) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(params) = crate::params::parse_voice_params(map, 0.0, cps) else {
        return out;
    };
    let Some(configs) = loop_configs(&params) else {
        return out;
    };
    if configs.is_empty() {
        return out;
    }
    let s = crate::voice::sound_name(params.s.as_ref());
    let Some(kind) = SourceKind::of(&s) else {
        return out;
    };
    let mut table = NodeTable::plan(&params, kind);
    for (k, c) in configs.iter().enumerate() {
        let control = control_of(c.ty, c.config);
        let fxi = fxi_of(c.config, c.entry);
        let sub = c.config.get("subControl");
        match resolve(&table, control.as_ref(), sub, &fxi) {
            Err(_) => break,
            Ok(Resolution::Skip(_)) => {}
            Ok(Resolution::Unsupported) => {
                if bad_key(c.ty, c.config).is_some() {
                    break;
                }
                let text = control.as_ref().map_or_else(String::new, js_to_string);
                let prefix = if c.entry == FxKey::Main { "" } else { "FX." };
                let label = format!("{prefix}{}:{text}", c.ty.key());
                if !out.contains(&label) {
                    out.push(label);
                }
            }
            Ok(Resolution::Targets { .. }) => {
                if bad_key(c.ty, c.config).is_some() {
                    break;
                }
                register(&mut table, c, k);
            }
        }
    }
    out
}

/// The `CONTROL_TARGETS` keys a modulator can resolve to an implemented param on some voice,
/// sorted: every `TARGETS` row resolved against the probe voices' node tables. The others skip
/// or throw wherever they are used. What `controls --json` prints as `modulatorTargets`.
/// The probe voices can miss a node some real voice has; `modulatable_targets_are_todays` is
/// the safety net for a row the probe skips.
pub(crate) fn modulatable_targets() -> Vec<&'static str> {
    let tables = probe_tables();
    let mut out = std::collections::BTreeSet::new();
    for (key, _, _) in TARGETS {
        if tables
            .iter()
            .any(|t| row_outcomes(t, key).contains(&Outcome::Targets))
        {
            out.insert(*key);
        }
    }
    out.into_iter().collect()
}

/// How one `TARGETS` row resolves on one node table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Targets,
    Skip,
    Unsupported,
    /// `resolve` returned an error (the build throws).
    Throws,
}

/// Row `key` resolved on `table` for `fxi` `"main"` and `0`: the control is `<head>_0` for the
/// heads `lfo`, `env` and `bmod` (`head` otherwise), with the text after the first `_` as its
/// `subControl`.
fn row_outcomes(table: &NodeTable, key: &str) -> [Outcome; 2] {
    let (head, sub) = match key.split_once('_') {
        Some((h, s)) => (h, Some(s)),
        None => (key, None),
    };
    let control = if matches!(head, "lfo" | "env" | "bmod") {
        format!("{head}_0")
    } else {
        head.to_string()
    };
    let control = Value::Str(control);
    let sub = sub.map(|s| Value::Str(s.to_string()));
    [Value::Str("main".into()), Value::Num(0.0)].map(|fxi| {
        match resolve(table, Some(&control), sub.as_ref(), &fxi) {
            Ok(Resolution::Targets { .. }) => Outcome::Targets,
            Ok(Resolution::Skip(_)) => Outcome::Skip,
            Ok(Resolution::Unsupported) => Outcome::Unsupported,
            Err(_) => Outcome::Throws,
        }
    })
}

/// The probe voices' node tables: for the first `SourceKind::NAMES` sound of each kind, (a) the
/// FX map with that `s` plus `djf`, `vib`, `vibmod`, `pwsweep` and `fmi` through `fmi8`, and (b)
/// `{ s, FX: [FX map + dry] }`, each with `lfo_0` and `env_0` registered on the main entry. A map
/// that fails to deserialize or parse is skipped (`modulatable_targets_are_todays` sees the loss).
fn probe_tables() -> Vec<NodeTable> {
    let fx = serde_json::json!({
        "cutoff": 800, "lprate": 2, "hcutoff": 300, "hpdepth": 0.5, "bandf": 900,
        "bpdepth": 0.5, "vowel": "a", "coarse": 2, "crush": 4, "shape": 0.3, "distort": 0.4,
        "tremolo": 2, "compressor": -10, "pan": 0.3, "phaserrate": 2, "transient": 0.5,
        "delay": 0.5, "delaytime": 0.2, "delayfeedback": 0.3, "room": 0.4
    });
    let mut kinds: Vec<SourceKind> = Vec::new();
    let mut tables = Vec::new();
    for (s, kind) in SourceKind::NAMES {
        if kinds.contains(kind) {
            continue;
        }
        kinds.push(*kind);
        let mut a = fx.clone();
        a["s"] = serde_json::json!(s);
        for (k, v) in [
            ("djf", 0.3),
            ("vib", 2.0),
            ("vibmod", 0.5),
            ("pwsweep", 0.3),
        ] {
            a[k] = serde_json::json!(v);
        }
        for n in 1..=8 {
            let k = if n == 1 {
                "fmi".to_string()
            } else {
                format!("fmi{n}")
            };
            a[k] = serde_json::json!(1);
        }
        let mut entry = fx.clone();
        entry["dry"] = serde_json::json!(0.5);
        let b = serde_json::json!({ "s": s, "FX": [entry] });
        for v in [a, b] {
            let Ok(m) = serde_json::from_value::<ValueMap>(v) else {
                continue;
            };
            let Ok(params) = crate::params::parse_voice_params(&m, 0.25, 2.0) else {
                continue;
            };
            let mut table = NodeTable::plan(&params, *kind);
            table.set(
                FxKey::Main,
                NodeName::Mod("lfo_0".into()),
                NodeList::Array(vec![Some(Node::Lfo(0))]),
            );
            table.set(
                FxKey::Main,
                NodeName::Mod("env_0".into()),
                NodeList::Array(vec![Some(Node::Env(1))]),
            );
            tables.push(table);
        }
    }
    tables
}

/// `nodeTracker.main['<type>_<id>'] = [node]` for an `lfo` or `env` modulator at position `index`.
fn register(table: &mut NodeTable, c: &Config<'_>, index: usize) {
    let id = js_to_string(c.id);
    match c.ty {
        ModType::Lfo => table.set(
            FxKey::Main,
            NodeName::Mod(format!("lfo_{id}")),
            NodeList::Array(vec![Some(Node::Lfo(index))]),
        ),
        ModType::Env => table.set(
            FxKey::Main,
            NodeName::Mod(format!("env_{id}")),
            NodeList::Array(vec![Some(Node::Env(index))]),
        ),
        ModType::Bmod => {}
    }
}

// ---- the processors

/// The patched bus modulator (`bmod-gate`): `bus (stereo) + dc (mono) -> shifted gainNode(1) ->
/// depthGain -> gate gainNode(0)` (open from `begin`, closed from `end`), down-mixed into each
/// target param.
#[derive(Debug, Clone)]
pub struct Bmod {
    /// The bus's index in the job's [`BusBank`].
    pub bus: usize,
    pub dc: ConstantSource,
    pub shifted: GainNode,
    pub depth: GainNode,
    pub gate: GainNode,
    /// The gate's close frame (`setValueAtTime(0, end)`), rounded up to a quantum: the first
    /// quantum it outputs only zeros.
    end_quantum: u64,
}

impl Bmod {
    /// One quantum: the stereo output of the gate (both channels) and its silence hint.
    fn render(&mut self, q: u64, buses: &mut BusBank) -> ([[f32; QUANTUM]; 2], bool) {
        let mut dc = [0.0f32; QUANTUM];
        let playing = self.dc.process(q, &mut dc);
        // `ConstantSourceHandler` zeroes (and so marks silent) its bus for an offset of 0.
        let dc_silent = !playing || self.dc.offset() == 0.0;
        let (bus, bus_silent) = buses.read(self.bus);
        // `shifted`'s input: two channels (the bus is a 2-channel explicit sum), `dc` connected
        // first and up-mixed, then the bus.
        let mut ch = [[0.0f32; QUANTUM]; 2];
        let mut silent = [true; 2];
        let dc_in = [dc];
        crate::orbit::sum_stereo(
            &mut ch,
            &mut silent,
            crate::orbit::Input {
                channels: &dc_in,
                silent: dc_silent,
            },
        );
        crate::orbit::sum_stereo(
            &mut ch,
            &mut silent,
            crate::orbit::Input {
                channels: &bus,
                silent: bus_silent,
            },
        );
        let s = silent[0] && silent[1];
        let s = self.shifted.process(q, &mut ch, s, &ParamInputs::NONE);
        let s = self.depth.process(q, &mut ch, s, &ParamInputs::NONE);
        let s = self.gate.process(q, &mut ch, s, &ParamInputs::NONE);
        (ch, s)
    }
}

/// A modulator's processor.
#[derive(Debug, Clone)]
pub enum ModNode {
    Lfo(Box<Lfo>),
    Env(Box<Envelope>),
    Bmod(Box<Bmod>),
}

/// One modulator of a voice.
#[derive(Debug, Clone)]
pub struct Modulator {
    pub node: ModNode,
}

impl Modulator {
    /// The first quantum start from which the modulator outputs nothing more (its processor is
    /// gone, or its gate closed).
    pub fn end_quantum(&self) -> u64 {
        match &self.node {
            ModNode::Lfo(l) => l.end_quantum(),
            ModNode::Env(e) => e.end_quantum(),
            ModNode::Bmod(b) => b.end_quantum,
        }
    }

    /// The modulator's own param at `slot` (an `lfo`'s or an `env`'s), for `currentValue`.
    fn param(&self, slot: ParamSlot) -> Option<&Param> {
        match &self.node {
            ModNode::Lfo(l) => l.params.get(usize::from(slot.0)),
            ModNode::Env(e) => e.params.get(usize::from(slot.0)),
            ModNode::Bmod(_) => None,
        }
    }
}

/// One modulator's output of the quantum: one buffer (an `lfo`, an `env`) or the two halves of
/// a down-mixed stereo signal (a `bmod`), each summed into the param in turn.
#[derive(Debug, Clone)]
pub struct ModOut {
    bufs: [[f32; QUANTUM]; 2],
    n: usize,
    silent: bool,
}

impl ModOut {
    const SILENT: ModOut = ModOut {
        bufs: [[0.0; QUANTUM]; 2],
        n: 1,
        silent: true,
    };
}

/// The connections of a voice's modulators and their outputs of the quantum: what each param
/// receives from them.
#[derive(Debug, Clone, Copy)]
pub struct ModView<'a> {
    conns: &'a [(usize, ParamRef)],
    out: &'a [ModOut],
}

impl ModView<'_> {
    /// Whether any modulator is connected to `slot` of `node`.
    pub fn connected(&self, node: Node, slot: ParamSlot) -> bool {
        self.conns
            .iter()
            .any(|(_, r)| r.node == node && r.slot == slot)
    }

    /// Appends the outputs of the modulators connected to `slot` of `node` to `bufs`, in
    /// connection order, silent ones left out.
    pub fn extend(&self, node: Node, slot: ParamSlot, bufs: &mut Vec<[f32; QUANTUM]>) {
        for (m, r) in self.conns {
            if r.node != node || r.slot != slot {
                continue;
            }
            if let Some(o) = self.out.get(*m)
                && !o.silent
            {
                bufs.extend_from_slice(&o.bufs[..o.n]);
            }
        }
    }

    /// Adds the modulator connections of `slot` of `node` to `inputs` (after its own).
    pub fn add(&self, node: Node, slot: ParamSlot, inputs: &mut ParamInputs) {
        if self.connected(node, slot) {
            inputs.connected = true;
            self.extend(node, slot, &mut inputs.bufs);
        }
    }

    /// The inputs of slot `slot` of `node` (none but the modulators').
    pub fn inputs(&self, node: Node, slot: ParamSlot) -> ParamInputs {
        let mut p = ParamInputs::NONE;
        self.add(node, slot, &mut p);
        p
    }

    /// The inputs of the twelve params of an `lfo` node.
    pub fn lfo_inputs(&self, node: Node) -> lfo::LfoInputs {
        let mut inputs = lfo::NO_INPUTS;
        if self.conns.iter().any(|(_, r)| r.node == node) {
            for (k, i) in inputs.iter_mut().enumerate() {
                self.add(node, ParamSlot(k as u8), i);
            }
        }
        inputs
    }

    /// The inputs of the thirteen params of an `env` node.
    fn env_inputs(&self, node: Node) -> envelope::EnvInputs {
        let mut inputs = envelope::NO_INPUTS;
        if self.conns.iter().any(|(_, r)| r.node == node) {
            for (k, i) in inputs.iter_mut().enumerate() {
                self.add(node, ParamSlot(k as u8), i);
            }
        }
        inputs
    }

    /// The outputs of the modulators connected to orbit-held params (the orbit delay's
    /// `delayTime` and `feedback`, the djf's value), in
    /// connection order, one entry per summand (`None` for a silent output: the param stays
    /// connected).
    pub fn orbit_inputs(&self, out: &mut Vec<(OrbitParam, Option<[f32; QUANTUM]>)>) {
        for (m, r) in self.conns {
            let param = match (r.node, r.slot.0) {
                (Node::OrbitDelay, 0) => OrbitParam::DelayTime,
                (Node::OrbitDelay, _) => OrbitParam::Feedback,
                (Node::OrbitDjf, _) => OrbitParam::DjfValue,
                _ => continue,
            };
            match self.out.get(*m) {
                Some(o) if !o.silent => out.extend(o.bufs[..o.n].iter().map(|b| (param, Some(*b)))),
                _ => out.push((param, None)),
            }
        }
    }
}

/// An orbit param a voice's modulator can feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbitParam {
    /// The orbit delay's `delayTime`.
    DelayTime,
    /// The orbit delay's feedback gain (`feedback`).
    Feedback,
    /// The orbit `djf` worklet's `value`.
    DjfValue,
}

/// A voice's modulators in creation order, their connections in connection order and their
/// outputs of the current quantum.
#[derive(Debug, Clone, Default)]
pub struct ModSet {
    pub mods: Vec<Modulator>,
    pub conns: Vec<(usize, ParamRef)>,
    out: Vec<ModOut>,
}

impl ModSet {
    /// Whether the voice has no modulator.
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    /// The view the params of the voice read this quantum.
    pub fn view(&self) -> ModView<'_> {
        ModView {
            conns: &self.conns,
            out: &self.out,
        }
    }

    /// Renders every modulator for the quantum at `q`, in descending creation order (a
    /// modulator only drives earlier ones, so every driver is rendered before its target).
    /// Processor errors are pushed to `errors`.
    pub fn evaluate(&mut self, q: u64, buses: &mut BusBank, errors: &mut Vec<String>) {
        if self.mods.is_empty() {
            return;
        }
        self.out.resize(self.mods.len(), ModOut::SILENT);
        for m in (0..self.mods.len()).rev() {
            let view = ModView {
                conns: &self.conns,
                out: &self.out,
            };
            let out = match &mut self.mods[m].node {
                ModNode::Lfo(l) => {
                    let inputs = view.lfo_inputs(Node::Lfo(m));
                    let mut buf = [0.0f32; QUANTUM];
                    let silent = l.render(q, &inputs, &mut buf);
                    if let Some(e) = l.take_error() {
                        errors.push(e.message());
                    }
                    ModOut {
                        bufs: [buf, [0.0; QUANTUM]],
                        n: 1,
                        silent,
                    }
                }
                ModNode::Env(e) => {
                    let inputs = view.env_inputs(Node::Env(m));
                    let mut buf = [0.0f32; QUANTUM];
                    let silent = e.render(q, &inputs, &mut buf);
                    ModOut {
                        bufs: [buf, [0.0; QUANTUM]],
                        n: 1,
                        silent,
                    }
                }
                ModNode::Bmod(b) => {
                    let (ch, silent) = b.render(q, buses);
                    ModOut {
                        bufs: crate::chain::downmix_terms(&ch),
                        n: 2,
                        silent,
                    }
                }
            };
            self.out[m] = out;
        }
    }

    /// Whether a modulator connected to an orbit-held param has not ended at the
    /// quantum starting at `q`.
    pub fn holds_orbit(&self, q: u64) -> bool {
        self.conns.iter().any(|(m, r)| {
            matches!(r.node, Node::OrbitDelay | Node::OrbitDjf)
                && self.mods.get(*m).is_some_and(|x| q < x.end_quantum())
        })
    }
}

// ---- the loop

/// What the loop builds its processors from (superdough's `t`, `endWithRelease`, `cps`, `cycle`
/// and the scheduling frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModTimes {
    pub begin: f64,
    pub end: f64,
    pub cps: f64,
    pub cycle: f64,
    /// Elapsed layout seconds at the onset for fixed-Hz modulation.
    pub phase_sec: f64,
    pub now: u64,
}

/// What the loop produced: the modulators, the error that stopped it (a `hap` error) and the
/// superdough logs of the configs it skipped.
#[derive(Debug, Clone, Default)]
pub struct LoopResult {
    pub set: ModSet,
    pub error: Option<VoiceError>,
    pub logs: Vec<String>,
}

/// The modulator loop of one voice, after its whole chain is built and connected (superdough.mjs
/// :1006-1051): for each FX entry in order (the chain entries, then main), every `lfo` config,
/// then every `env`, then every `bmod`, each with `fxi = config.fxi ?? key` (the entry's index,
/// or `'main'`; the `modulator-fxi-copy` patch). Every connected `lfo`/`env` registers as
/// `nodes.main['<type>_<id>']`, whichever entry carries it. `current_value` reads a resolved param's intrinsic value
/// (`targetParams[0].value`) for the params outside the modulators. A throw ends the loop: the
/// modulators before it stay connected and registered in `table`.
pub fn run_loop(
    params: &VoiceParams,
    table: &mut NodeTable,
    current_value: &dyn Fn(ParamRef) -> Option<f32>,
    buses: &BusBank,
    times: ModTimes,
) -> LoopResult {
    let mut r = LoopResult::default();
    let Some(configs) = loop_configs(params) else {
        r.error = Some(hap_error(
            "modulator value is not a { __ids, [id]: config } object".into(),
        ));
        return r;
    };
    for c in &configs {
        match connect(c, table, current_value, buses, times, &mut r.set) {
            Ok(Some(log)) => {
                if !r.logs.contains(&log) {
                    r.logs.push(log);
                }
            }
            Ok(None) => {}
            Err(e) => {
                r.error = Some(e);
                break;
            }
        }
    }
    r
}

/// The `currentValue` of `refs[0]` (`targetParams[0].value`, 0 read as 1).
fn current(
    r: ParamRef,
    set: &ModSet,
    current_value: &dyn Fn(ParamRef) -> Option<f32>,
) -> Result<f64, VoiceError> {
    let v = match r.node {
        Node::Lfo(m) | Node::Env(m) => set
            .mods
            .get(m)
            .and_then(|x| x.param(r.slot))
            .map(Param::value),
        _ => current_value(r),
    };
    let v = v.ok_or_else(|| hap_error("modulator target has no value".into()))?;
    Ok(if v == 0.0 { 1.0 } else { f64::from(v) })
}

/// `depthabs != null ? depthabs : depth * currentValue` (`depth = 1` when absent): the value and,
/// when it is a string (`depthabs` given as one), its text for `getLfo`'s `max` concatenation.
fn depth_value(config: &ValueMap, cv: f64) -> (f64, Option<String>) {
    match config.get("depthabs") {
        Some(Value::Null) | None => {
            let depth = config.get("depth").map_or(1.0, js_to_number_full);
            (depth * cv, None)
        }
        Some(Value::Str(s)) => (parse_js_number(s), Some(s.clone())),
        Some(v) => (js_to_number_full(v), None),
    }
}

/// A present config key as a number (`.value = v`).
fn num(config: &ValueMap, key: &str) -> Option<f64> {
    config.get(key).map(js_to_number_full)
}

/// Connects one config; `Ok(Some(log))` for a logged skip.
fn connect(
    c: &Config<'_>,
    table: &mut NodeTable,
    current_value: &dyn Fn(ParamRef) -> Option<f32>,
    buses: &BusBank,
    times: ModTimes,
    set: &mut ModSet,
) -> Result<Option<String>, VoiceError> {
    let control = control_of(c.ty, c.config);
    let fxi = fxi_of(c.config, c.entry);
    let resolution = resolve(table, control.as_ref(), c.config.get("subControl"), &fxi)?;
    let (param, refs) = match resolution {
        Resolution::Skip(log) => return Ok(log),
        // `unsupported_signatures` refuses such a job before it renders.
        Resolution::Unsupported => {
            return Err(hap_error("modulator target is a noise detune".into()));
        }
        Resolution::Targets { param, refs } => (param, refs),
    };
    let cv = current(refs[0], set, current_value)?;
    let range = get_range_for_param(param, cv);
    let index = set.mods.len();
    let node = match c.ty {
        ModType::Lfo => ModNode::Lfo(Box::new(build_lfo(c.config, cv, range, times)?)),
        ModType::Env => ModNode::Env(Box::new(build_env(c.config, cv, range, times)?)),
        ModType::Bmod => ModNode::Bmod(Box::new(build_bmod(c.config, cv, range, buses, times)?)),
    };
    set.mods.push(Modulator { node });
    register(table, c, index);
    set.conns.extend(refs.into_iter().map(|r| (index, r)));
    Ok(None)
}

/// The error of a key the processor lacks.
fn key_error(ty: ModType, key: &str) -> VoiceError {
    type_error(&format!(
        "Cannot set properties of undefined (setting 'value'): {} has no param '{key}'",
        ty.key()
    ))
}

/// `connectLFO`'s `getLfo` call (`modParams`): the config's `shape`, `dcoffset`, `skew`, `curve`
/// and `phaseoffset`, `begin: t`, `end: endWithRelease`, `frequency: sync !== undefined ? sync *
/// cps : rate` (`rate = 1`), the depth and the range. Retrigger starts at zero. Otherwise,
/// fixed rates use elapsed time and cycle-synced rates use musical time.
fn build_lfo(
    config: &ValueMap,
    cv: f64,
    range: Option<(f64, f64)>,
    times: ModTimes,
) -> Result<Lfo, VoiceError> {
    if let Some(k) = bad_key(ModType::Lfo, config) {
        return Err(key_error(ModType::Lfo, k));
    }
    let (depth, depth_text) = depth_value(config, cv);
    let frequency = match config.get("sync") {
        Some(sync) => js_to_number_full(sync) * times.cps,
        None => num(config, "rate").unwrap_or(1.0),
    };
    let retrig = num(config, "retrig").unwrap_or(0.0);
    let time = if retrig > 0.5 {
        0.0
    } else if config.contains_key("sync") {
        times.cycle / times.cps
    } else {
        times.phase_sec
    };
    let dcoffset = num(config, "dcoffset");
    let (min, max) = match range {
        Some((min, max)) => (Some(min), Some(max)),
        // `max ?? dcoffset * depth + depth`: a string depth concatenates.
        None => match depth_text {
            Some(text) => {
                let product = dcoffset.unwrap_or(-0.5) * depth;
                let max = parse_js_number(&format!("{}{text}", number_to_string(product)));
                (None, Some(max))
            }
            None => (None, None),
        },
    };
    let props = LfoProps {
        begin: Some(times.begin),
        end: Some(times.end),
        time: Some(time),
        depth: Some(depth),
        dcoffset,
        frequency: Some(frequency),
        skew: num(config, "skew"),
        phaseoffset: num(config, "phaseoffset"),
        curve: num(config, "curve"),
        shape: config.get("shape").map(ShapeInput::from_value),
        min,
        max,
    };
    Ok(get_lfo(&props, times.now)?)
}

/// `connectEnvelope`'s `getEnvelope` call: the config's `attack`, `decay`, `sustain`, `release`
/// and `retrigger`, `begin: t`, `end: endWithRelease`, the depth, the range (only when defined)
/// and `attackCurve: acurve`, `decayCurve: dcurve`, `releaseCurve: rcurve`.
fn build_env(
    config: &ValueMap,
    cv: f64,
    range: Option<(f64, f64)>,
    times: ModTimes,
) -> Result<Envelope, VoiceError> {
    if let Some(k) = bad_key(ModType::Env, config) {
        return Err(key_error(ModType::Env, k));
    }
    let (depth, _) = depth_value(config, cv);
    let props = EnvProps {
        begin: Some(times.begin),
        end: Some(times.end),
        depth: Some(depth),
        min: range.map(|r| r.0),
        max: range.map(|r| r.1),
        attack_curve: num(config, "acurve"),
        decay_curve: num(config, "dcurve"),
        release_curve: num(config, "rcurve"),
        attack: num(config, "attack"),
        decay: num(config, "decay"),
        sustain: num(config, "sustain"),
        release: num(config, "release"),
        retrigger: num(config, "retrigger"),
    };
    Ok(get_envelope(&props, times.now)?)
}

/// `gainNode(v)`: `createGain()`, then `.value = v` (a WebIDL float; also `setValueAtTime(v,
/// now)`).
fn gain_node(v: f64, now: u64) -> Result<GainNode, VoiceError> {
    let mut g = GainNode::new(Param::gain(1.0));
    g.gain.set_value(idl_float(v, "gain")?, now)?;
    Ok(g)
}

/// `connectBusModulator` as `bmod-gate` patches it: the bus (`getBus(bus)`), `dc =
/// ConstantSourceNode({ offset: dc ?? 0 })` started at `begin`, `shifted = gainNode(1)`,
/// `depthGain = gainNode(sign(d) * |d| / 0.3)`, the dead clamp branch's `gainNode(1 / max)`
/// (only its throw), then the gate `gainNode(0)` with `setValueAtTime(1, begin)` and
/// `setValueAtTime(0, end)`.
fn build_bmod(
    config: &ValueMap,
    cv: f64,
    range: Option<(f64, f64)>,
    buses: &BusBank,
    times: ModTimes,
) -> Result<Bmod, VoiceError> {
    let now = times.now;
    let key = bus_key(config.get("bus"));
    let bus = buses
        .index_of(&key)
        .ok_or_else(|| hap_error(format!("bus {key} is not in the job's bus graph")))?;
    let offset = match config.get("dc") {
        None | Some(Value::Null) => 0.0,
        Some(v) => js_to_number_full(v),
    };
    let mut dc = ConstantSource::new(idl_float(offset, "offset")?);
    dc.start(times.begin, now)?;
    let shifted = gain_node(1.0, now)?;
    // `Math.sign(d) * Math.abs(d)` is `d` itself (NaN, +-0 and the infinities included).
    let (d, _) = depth_value(config, cv);
    let depth = gain_node(d / 0.3, now)?;
    if let Some((_, max)) = range {
        idl_float(1.0 / max, "gain")?;
    }
    let mut gate = gain_node(0.0, now)?;
    gate.gain.set_value_at_time(1.0, times.begin, now)?;
    gate.gain.set_value_at_time(0.0, times.end, now)?;
    let end_quantum = crate::render::round_up_128(crate::render::frame_for_param_event(times.end));
    Ok(Bmod {
        bus,
        dc,
        shifted,
        depth,
        gate,
        end_quantum,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chain::StageId;
    use crate::job::value_map as map;
    use crate::job::{ErrorKind, OrbitSpec};
    use crate::noise::NoiseCache;
    use crate::orbit::{Orbit, OrbitInputs};
    use crate::params::parse_voice_params;
    use crate::periodic::WaveTables;
    use crate::voice::{StartInfo, Voice, VoiceOutput};
    use serde_json::json;

    use crate::QUANTUM_U64 as Q;

    /// A modulator value holding `configs` under the ids 0, 1, ...
    fn ids(configs: &[serde_json::Value]) -> serde_json::Value {
        let n: Vec<usize> = (0..configs.len()).collect();
        let mut m = json!({ "__ids": { "$set": n } });
        for (i, c) in configs.iter().enumerate() {
            m[i.to_string()] = c.clone();
        }
        m
    }

    fn orbit() -> Orbit {
        Orbit::new(
            &OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1],
            },
            2,
        )
    }

    fn buses() -> BusBank {
        BusBank::of_keys(vec!["1".into(), "2".into(), "undefined".into()])
    }

    fn start() -> StartInfo {
        StartInfo {
            t: 0.1,
            hap_duration: 0.25,
            now_frame: 0,
            cycle: 0.5,
            phase_sec: 0.25,
            cps: 2.0,
        }
    }

    /// Builds `v` at t = 0.1 (a 0.25 s hap at cycle 0.5, cps 2, scheduled at frame 0) on `o`.
    fn build_on(v: serde_json::Value, o: &mut Orbit) -> (Voice, Option<VoiceError>) {
        let p = parse_voice_params(&map(v), 0.25, 2.0).unwrap();
        let mut tables = WaveTables::new();
        let mut noise = NoiseCache::new(0);
        Voice::build(&p, 0, start(), &mut tables, &mut noise, o, &buses()).unwrap()
    }

    fn build(v: serde_json::Value) -> (Voice, Option<VoiceError>) {
        build_on(v, &mut orbit())
    }

    /// A voice whose modulator loop succeeded.
    fn ok(v: serde_json::Value) -> Voice {
        let (voice, e) = build(v.clone());
        assert!(e.is_none(), "{v}: {e:?}");
        voice
    }

    /// The hap error of a voice whose modulator loop threw.
    fn throws(v: serde_json::Value) -> (Voice, VoiceError) {
        let (voice, e) = build(v.clone());
        let e = e.unwrap_or_else(|| panic!("{v}: no throw"));
        assert_eq!(e.kind, ErrorKind::Hap, "{v}");
        (voice, e)
    }

    fn lfo_of(v: &Voice, k: usize) -> &Lfo {
        match &v.mods().mods[k].node {
            ModNode::Lfo(l) => l,
            other => panic!("modulator {k} is {other:?}"),
        }
    }

    fn lfo_param(v: &Voice, k: usize, p: usize) -> f32 {
        lfo_of(v, k).params[p].value()
    }

    fn table(v: serde_json::Value) -> NodeTable {
        let m = map(v);
        let p = parse_voice_params(&m, 0.25, 2.0).unwrap();
        let s = crate::voice::sound_name(p.s.as_ref());
        NodeTable::plan(&p, SourceKind::of(&s).unwrap())
    }

    fn data(node: &'static str, param: &'static str) -> Option<ControlData> {
        Some(ControlData { node, param })
    }

    #[test]
    fn get_control_data_rows() {
        assert_eq!(get_control_data("lfo_0", "rate"), data("lfo", "frequency"));
        assert_eq!(get_control_data("lfo_0", "shape"), data("lfo", "frequency"));
        assert_eq!(
            get_control_data("lfo_0", "undefined"),
            data("lfo", "frequency")
        );
        assert_eq!(get_control_data("lfo_0", "depthabs"), data("lfo", "depth"));
        assert_eq!(get_control_data("env_3", "undefined"), data("env", "depth"));
        assert_eq!(get_control_data("env_1", "release"), data("env", "release"));
        assert_eq!(get_control_data("bmod", "depth"), data("bmod", "depth"));
        assert_eq!(
            get_control_data("fmi2", "undefined"),
            data("fm_2_gain", "gain")
        );
        assert_eq!(get_control_data("nope", "undefined"), None);
        assert_eq!(get_control_data("", "undefined"), None);
        assert_eq!(
            get_control_data("tremolophase", "undefined"),
            data("tremolo", "phase")
        );
        // The head before the first `_` is looked up: `cutoff_x` is the cutoff.
        assert_eq!(get_control_data("cutoff_x", "y"), data("lpf", "frequency"));
        // Every row of the table is reachable by its own name.
        for (k, node, param) in TARGETS {
            if !k.contains('_') {
                assert_eq!(get_control_data(k, "undefined"), data(node, param), "{k}");
            }
        }
    }

    #[test]
    fn get_range_for_param_rows() {
        assert_eq!(get_range_for_param("frequency", 29.99), None);
        assert_eq!(
            get_range_for_param("frequency", 30.0),
            Some((-10.0, 23970.0))
        );
        assert_eq!(
            get_range_for_param("frequency", 24000.0),
            Some((-23980.0, 0.0))
        );
        assert_eq!(get_range_for_param("frequency", -500.0), None);
        assert_eq!(get_range_for_param("frequency", f64::NAN), None);
        assert_eq!(get_range_for_param("gain", 1000.0), None);
        assert_eq!(get_range_for_param("Q", 100.0), None);
    }

    #[test]
    fn current_value_is_the_intrinsic_value() {
        let gain = |g: serde_json::Value| {
            ok(json!({ "s": "sine", "gain": g, "lfo": ids(&[json!({ "control": "gain" })]) }))
        };
        // A fresh gain 0.8 reads the f32 0.8; depth 1 * cv; getLfo's min/max from it.
        let v = gain(json!(0.8));
        let cv = f64::from(0.8f32);
        assert_eq!(
            lfo_param(&v, 0, lfo::DEPTH).to_bits(),
            (cv as f32).to_bits()
        );
        assert_eq!(lfo_param(&v, 0, lfo::MIN), (-0.5 * cv) as f32);
        assert_eq!(lfo_param(&v, 0, lfo::MAX), (-0.5 * cv + cv) as f32);
        // 0 and -0 read 1; a negative value stays.
        for g in [json!(0), json!({ "$num": "-0" })] {
            assert_eq!(lfo_param(&gain(g.clone()), 0, lfo::DEPTH), 1.0, "{g}");
        }
        assert_eq!(lfo_param(&gain(json!(-0.5)), 0, lfo::DEPTH), -0.5);
        // The biquad's frequency is clamped to Nyquist: cutoff 30000 reads 22050, so the range is
        // { 20 - 22050, 24000 - 22050 }.
        let v = ok(json!({ "s": "sawtooth", "cutoff": 30000,
            "lfo": ids(&[json!({ "control": "cutoff" })]) }));
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 22050.0);
        assert_eq!(lfo_param(&v, 0, lfo::MIN), -22030.0);
        assert_eq!(lfo_param(&v, 0, lfo::MAX), 1950.0);
        // `pan` reads 2 * pan - 1.
        let v = ok(json!({ "s": "sine", "pan": 0.75, "lfo": ids(&[json!({ "control": "pan" })]) }));
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 0.5);
        // A filter envelope on the cutoff does not move it.
        let v = ok(json!({
            "s": "sawtooth", "cutoff": 500, "lpenv": 2,
            "lfo": ids(&[json!({ "control": "cutoff" })])
        }));
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 500.0);
    }

    #[test]
    fn current_value_of_an_orbit_delay_is_the_last_rendered_quantum() {
        let mut o = orbit();
        let fb = json!({ "s": "sine", "delay": 0.5, "delayfeedback": 0.5 });
        let mut a = fb.clone();
        a["delaytime"] = json!(0.02);
        build_on(a, &mut o);
        // A second hap scheduled at the same pause schedules setValueAtTime(0.03, t) and still
        // reads the 0.02 that `.value =` set.
        let mut b = fb.clone();
        b["delaytime"] = json!(0.03);
        b["lfo"] = ids(&[json!({ "control": "delaytime" })]);
        let (v, _) = build_on(b.clone(), &mut o);
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 0.02f32);
        // After rendering past the event the intrinsic value is the timeline's last frame.
        for k in 0..60 {
            o.begin();
            o.process_returns(k * Q, 0, &[], &[], &OrbitInputs::default());
        }
        let (v, _) = build_on(b, &mut o);
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 0.03f32);
        assert_eq!(
            v.mods().conns[0].1,
            ParamRef {
                node: Node::OrbitDelay,
                slot: ParamSlot(0)
            }
        );
    }

    #[test]
    fn depth_value_rows() {
        let lfo = |c: serde_json::Value| {
            let mut c = c;
            c["control"] = json!("gain");
            json!({ "s": "sine", "gain": 0.5, "lfo": ids(&[c]) })
        };
        let depth = |c: serde_json::Value| lfo_param(&ok(lfo(c)), 0, lfo::DEPTH);
        assert_eq!(
            depth(json!({ "depth": 2, "depthabs": 3 })),
            3.0,
            "depthabs wins"
        );
        assert_eq!(
            depth(json!({ "depth": 2, "depthabs": null })),
            1.0,
            "null falls back"
        );
        assert_eq!(depth(json!({})), 0.5, "depth absent is 1");
        assert_eq!(depth(json!({ "depth": null })), 0.0, "depth null is 0");
        assert_eq!(depth(json!({ "depth": "0.5" })), 0.25);
        assert_eq!(
            depth(json!({ "depth": [2] })),
            1.0,
            "an array through its string"
        );
        // A string depthabs concatenates in getLfo's default max (`-1.5 + "3"`).
        let v = ok(lfo(json!({ "depthabs": "3" })));
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 3.0);
        assert_eq!(lfo_param(&v, 0, lfo::MIN), -1.5);
        assert_eq!(lfo_param(&v, 0, lfo::MAX), -1.53);
        for bad in [
            json!({ "$num": "NaN" }),
            json!(1e40),
            json!({ "$num": "Infinity" }),
        ] {
            let (v, e) = throws(lfo(json!({ "depthabs": bad })));
            assert!(e.detail.contains("TypeError"), "{}", e.detail);
            assert!(v.mods().is_empty());
        }
    }

    #[test]
    fn lfo_props_from_the_config() {
        // rate, sync (* cps 2), retrig (time 0) and the config's shape keys; begin t, end
        // endWithRelease (0.1 + 0.25 + 0.01).
        let v = ok(json!({ "s": "sine", "gain": 0.5, "lfo": ids(&[
            json!({ "control": "gain", "rate": 3, "skew": 0.2, "curve": 2, "phaseoffset": 0.1,
                    "dcoffset": 0, "shape": "square" }),
            json!({ "control": "gain", "sync": 1.5, "retrig": 1 }),
            json!({ "control": "gain", "sync": null, "rate": 7 }),
            json!({ "control": "gain", "frequency": 9, "time": 9, "min": 9, "max": 9, "begin": 9,
                    "end": 9, "cps": 9, "cycle": 9 })
        ]) }));
        let p = |k, i| lfo_param(&v, k, i);
        assert_eq!((p(0, lfo::FREQUENCY), p(0, lfo::TIME)), (3.0, 0.25));
        assert_eq!((p(0, lfo::SKEW), p(0, lfo::CURVE)), (0.2, 2.0));
        assert_eq!((p(0, lfo::PHASEOFFSET), p(0, lfo::SHAPE)), (0.1, 4.0));
        assert_eq!(
            (p(0, lfo::DCOFFSET), p(0, lfo::MIN), p(0, lfo::MAX)),
            (0.0, 0.0, 0.5)
        );
        assert_eq!((p(1, lfo::FREQUENCY), p(1, lfo::TIME)), (3.0, 0.0));
        assert_eq!(
            p(2, lfo::FREQUENCY),
            0.0,
            "sync null is present: null * cps"
        );
        // Overwritten keys: frequency rate 1, time cycle / cps, the range default, begin, end.
        assert_eq!((p(3, lfo::FREQUENCY), p(3, lfo::TIME)), (1.0, 0.25));
        assert_eq!((p(3, lfo::MIN), p(3, lfo::MAX)), (-0.25, 0.25));
        assert_eq!(p(3, lfo::BEGIN), 0.1f32);
        assert_eq!(p(3, lfo::END), (0.1f64 + 0.25 + 0.01) as f32);
        assert_eq!(v.mods().mods.len(), 4);
    }

    #[test]
    fn env_props_from_the_config() {
        let v = ok(json!({ "s": "sawtooth", "cutoff": 300, "env": ids(&[json!({
            "control": "cutoff", "attack": 0.05, "decay": 0.1, "sustain": 0.3, "depthabs": 2000,
            "acurve": 0.5, "rcurve": null, "retrigger": 0
        })]) }));
        let ModNode::Env(e) = &v.mods().mods[0].node else {
            panic!("an env");
        };
        let p = |k: usize| e.params[k].value();
        assert_eq!((p(envelope::ATTACK), p(envelope::DECAY)), (0.05, 0.1));
        assert_eq!((p(envelope::SUSTAIN), p(envelope::DEPTH)), (0.3, 2000.0));
        assert_eq!((p(envelope::MIN), p(envelope::MAX)), (-280.0, 23700.0));
        assert_eq!(
            (p(envelope::ATTACK_CURVE), p(envelope::RELEASE_CURVE)),
            (0.5, 0.0)
        );
        assert_eq!(p(envelope::DECAY_CURVE), 0.0, "absent keeps the default");
        assert_eq!((p(envelope::RETRIGGER), p(envelope::BEGIN)), (0.0, 0.1));
        // Without a range (a gain target), min and max keep the processor's -1e9 and 1e9.
        let v = ok(json!({ "s": "sine", "env": ids(&[json!({ "control": "gain" })]) }));
        let ModNode::Env(e) = &v.mods().mods[0].node else {
            panic!("an env");
        };
        assert_eq!(
            (
                e.params[envelope::MIN].value(),
                e.params[envelope::MAX].value()
            ),
            (-1e9, 1e9)
        );
    }

    #[test]
    fn every_throw_site() {
        let one = |c: serde_json::Value| ids(&[c]);
        let cases = [
            json!({ "s": "sine", "lfo": one(json!({ "control": 5 })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": null })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": ["gain"] })) }),
            json!({ "s": "sine", "env": one(json!({ "attack": 0.1 })) }),
            json!({ "s": "sine", "bmod": one(json!({ "bus": 1 })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "attack": 1 })) }),
            json!({ "s": "sine", "env": one(json!({ "control": "gain", "time": 1 })) }),
            json!({ "s": "sine", "env": one(json!({ "control": "gain", "cps": 1 })) }),
            json!({ "s": "sine", "env": one(json!({ "control": "gain", "rate": 1 })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "fxi": 2 })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "fxi": "x" })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "fxi": true })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "fxi": "00" })) }),
            json!({ "s": "sine", "tremolo": 4, "lfo": one(json!({ "control": "tremolophase" })) }),
            json!({ "s": "saw", "cutoff": 500, "lprate": 2, "lfo": one(json!({ "control": "lprate" })) }),
            json!({ "s": "saw", "cutoff": 500, "lprate": 2, "lfo": one(json!({ "control": "lpsync" })) }),
            json!({ "s": "saw", "cutoff": 500, "lfo": one(json!({ "control": "lprate" })) }),
            json!({ "s": "saw", "cutoff": 500, "lfo": one(json!({ "control": "lpdepth" })) }),
            json!({ "s": "saw", "hcutoff": 500, "env": one(json!({ "control": "hpshape" })) }),
            json!({ "s": "saw", "cutoff": 500, "ftype": "ladder", "lfo": one(json!({ "control": "resonance" })) }),
            json!({ "s": "saw", "transient": 0.5, "lfo": one(json!({ "control": "transient" })) }),
            json!({ "s": "one", "lfo": one(json!({ "control": "note" })) }),
            json!({ "s": "bus", "n": 1, "lfo": one(json!({ "control": "note" })) }),
            json!({ "s": "saw", "lfo": one(json!({ "control": "pw" })) }),
            json!({ "s": "pulse", "lfo": one(json!({ "control": "detune" })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "spread" })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "wt" })) }),
            json!({ "s": "pulse", "lfo": one(json!({ "control": "pwrate" })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "depthabs": { "$num": "NaN" } })) }),
            json!({ "s": "sine", "lfo": one(json!({ "control": "gain", "skew": { "$num": "Infinity" } })) }),
            json!({ "s": "sine", "env": one(json!({ "control": "gain", "attack": {} })) }),
            json!({ "s": "sine", "bmod": one(json!({ "control": "gain", "bus": 1, "dc": { "$num": "NaN" } })) }),
            json!({ "s": "pulse", "freq": 24000, "bmod": one(json!({ "control": "note", "bus": 1 })) }),
        ];
        for v in cases {
            let (voice, e) = throws(v.clone());
            assert!(e.detail.starts_with("TypeError"), "{v}: {}", e.detail);
            assert!(voice.mods().is_empty(), "{v}: nothing connected");
            assert!(
                voice.take_logs_ref().is_empty(),
                "{v}: a throw is not a log"
            );
        }
        // A throw keeps the earlier modulators, connected and registered, and builds no later one.
        let (v, _) = throws(json!({ "s": "sine", "lfo": ids(&[
            json!({ "control": "gain" }), json!({ "control": 5 }), json!({ "control": "gain" })
        ]), "env": ids(&[json!({ "control": "gain" })]) }));
        assert_eq!(v.mods().mods.len(), 1);
        assert_eq!(v.mods().conns.len(), 1);
        assert!(v.node_table().get(FxKey::Main, "lfo_0").is_some());
        assert!(v.node_table().get(FxKey::Main, "lfo_2").is_none());
        // A bmod on a pulse below 24000 builds (the clamp's throw is only 1 / 0).
        ok(
            json!({ "s": "pulse", "freq": 23999, "bmod": ids(&[json!({ "control": "note", "bus": 1 })]) }),
        );
    }

    #[test]
    fn skips_connect_nothing_and_are_superdoughs_logs() {
        let one = |c: &str| ids(&[json!({ "control": c })]);
        let cases = [
            (json!({ "s": "sine", "lfo": one("bmod") }), "bmod"),
            (json!({ "s": "sine", "dry": 0.5, "lfo": one("dry") }), "dry"),
            // Without the data table's node the key is the control text itself.
            (
                json!({ "s": "sine", "bus": 1, "lfo": one("busgain") }),
                "busgain",
            ),
            (json!({ "s": "sine", "lfo": one("pan") }), "pan"),
            (json!({ "s": "sine", "lfo": one("cutoff") }), "cutoff"),
            (json!({ "s": "sine", "lfo": one("wtrate") }), "wtrate"),
            (json!({ "s": "sine", "lfo": one("stretch") }), "stretch"),
            (json!({ "s": "sine", "lfo": one("fmi") }), "fmi"),
            (json!({ "s": "sine", "vib": 0, "lfo": one("vib") }), "vib"),
            (json!({ "s": "sine", "lfo": one("lfo_3") }), "lfo_3"),
            (json!({ "s": "sine", "lfo": ids(&[json!({})]) }), "lfo"),
            (json!({ "s": "white", "lfo": one("tremolo") }), "tremolo"),
        ];
        for (v, key) in cases {
            let (mut voice, e) = build(v.clone());
            assert!(e.is_none(), "{v}: {e:?}");
            assert!(voice.mods().is_empty(), "{v}");
            let logs = voice.take_logs();
            assert_eq!(logs.len(), 1, "{v}");
            assert!(
                logs[0].starts_with(&format!(
                    "[superdough] error: Could not connect to target '{key}' \u{2014} it does not \
                     exist. Available targets: "
                )),
                "{v}: {}",
                logs[0]
            );
        }
        let (mut v, _) = build(json!({ "s": "sine", "dry": 0.5, "lfo": one("dry") }));
        assert_eq!(
            v.take_logs(),
            [
                "[superdough] error: Could not connect to target 'dry' \u{2014} it does not exist. \
              Available targets: source, gain, post"
            ]
        );
        // An unknown control has no data; the same log twice is reported once.
        let (mut v, e) = build(json!({ "s": "sine", "lfo": ids(&[
            json!({ "control": "nope" }), json!({ "control": "nope" })
        ]) }));
        assert!(e.is_none());
        assert_eq!(
            v.take_logs(),
            [
                "[superdough] error: Could not find control data for target 'nope'. It may not be \
              modulatable."
            ]
        );
        assert!(v.take_logs().is_empty(), "taken once");
    }

    #[test]
    fn fx_chain_entries_are_addressed_by_fxi() {
        let one = |c: serde_json::Value| ids(&[c]);
        // An entry's own lfo defaults to fxi = the entry's index (modulator-fxi-copy): it
        // targets the entry's lowpass, not main's (main has none), with currentValue 500.
        let v = ok(json!({
            "s": "sawtooth",
            "FX": [{ "cutoff": 500, "lfo": one(json!({ "control": "cutoff" })) }]
        }));
        let lpf = match v.node_table().get(FxKey::Index(0), "lpf") {
            Some(NodeList::Array(e)) => e[0].unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            v.mods().conns,
            [(
                0,
                ParamRef {
                    node: lpf,
                    slot: ParamSlot(0)
                }
            )]
        );
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 500.0);
        // `lfo_0` registers in main whichever entry carries it.
        assert!(v.node_table().get(FxKey::Main, "lfo_0").is_some());
        assert!(v.node_table().get(FxKey::Index(0), "lfo_0").is_none());
        // A numeric fxi from the main value reaches the entry.
        let v = ok(json!({
            "s": "sawtooth", "FX": [{ "cutoff": 500 }],
            "lfo": one(json!({ "control": "cutoff", "fxi": 0 }))
        }));
        assert_eq!(
            v.mods().conns,
            [(
                0,
                ParamRef {
                    node: lpf,
                    slot: ParamSlot(0)
                }
            )]
        );
        // ...as does its string; "main" from an entry reaches main's own filter.
        let v = ok(json!({
            "s": "sawtooth", "cutoff": 900,
            "FX": [{ "cutoff": 500, "lfo": one(json!({ "control": "cutoff", "fxi": "main" })) }]
        }));
        assert_eq!(lfo_param(&v, 0, lfo::DEPTH), 900.0);
        let v = ok(json!({
            "s": "sawtooth", "FX": [{ "cutoff": 500 }],
            "lfo": one(json!({ "control": "cutoff", "fxi": "0" }))
        }));
        assert_eq!(v.mods().conns.len(), 1);
        // An fxi naming no entry throws (two entries: 2 is past the end).
        let (_, e) = throws(json!({
            "s": "sawtooth", "FX": [{ "cutoff": 500 }, { "cutoff": 600 }],
            "lfo": one(json!({ "control": "cutoff", "fxi": 2 }))
        }));
        assert!(e.detail.contains("reading 'lpf'"), "{}", e.detail);
    }

    #[test]
    fn note_from_a_chain_entry_skips() {
        // `note` from fxi 0: the entry has no `source` (nor `note`): superdough's log, nothing
        // connected.
        let (mut v, e) = build(json!({
            "s": "sawtooth",
            "FX": [{ "cutoff": 500, "lfo": ids(&[json!({ "control": "note" })]) }]
        }));
        assert!(e.is_none(), "{e:?}");
        assert!(v.mods().is_empty());
        assert_eq!(
            v.take_logs(),
            [
                "[superdough] error: Could not connect to target 'note' \u{2014} it does not exist. \
              Available targets: gain, lpf, lpf_lfo"
            ]
        );
        // The same from main reaches the source.
        let v = ok(json!({
            "s": "sawtooth", "FX": [{ "cutoff": 500 }],
            "lfo": ids(&[json!({ "control": "note" })])
        }));
        assert_eq!(v.mods().conns[0].1.node, Node::Source);
        // A chain entry's postgain is not its own: a skip.
        let (mut v, _) = build(json!({
            "s": "sawtooth", "FX": [{ "lfo": ids(&[json!({ "control": "postgain" })]) }]
        }));
        assert!(v.mods().is_empty());
        assert_eq!(v.take_logs().len(), 1);
    }

    #[test]
    fn local_delay_and_reverb_params_are_targets() {
        let v = ok(json!({
            "s": "sawtooth",
            "FX": [{
                "delay": 0.25, "delaytime": 0.1, "delayfeedback": 0.4, "room": 0.3, "roomsize": 0.1,
                "lfo": ids(&[
                    json!({ "control": "delaytime" }),
                    json!({ "control": "delayfeedback" }),
                    json!({ "control": "delay" }),
                    json!({ "control": "room" }),
                ])
            }]
        }));
        let node = |name: &str| match v.node_table().get(FxKey::Index(0), name) {
            Some(NodeList::Array(e)) => e[0].unwrap(),
            other => panic!("{name}: {other:?}"),
        };
        let (delay, delay_mix, room_mix) = (node("delay"), node("delay_mix"), node("room_mix"));
        assert!(matches!(delay, Node::Stage(_)));
        assert!(matches!(delay_mix, Node::StageWet(_)));
        let refs: Vec<ParamRef> = v.mods().conns.iter().map(|(_, r)| *r).collect();
        assert_eq!(
            refs,
            [
                ParamRef {
                    node: delay,
                    slot: ParamSlot(0)
                },
                ParamRef {
                    node: delay,
                    slot: ParamSlot(1)
                },
                ParamRef {
                    node: delay_mix,
                    slot: ParamSlot(0)
                },
                ParamRef {
                    node: room_mix,
                    slot: ParamSlot(0)
                },
            ]
        );
        // currentValue: the f32 intrinsic values (depth 1 * cv; no range off `frequency`).
        let depths: Vec<f32> = (0..4).map(|k| lfo_param(&v, k, lfo::DEPTH)).collect();
        assert_eq!(depths, [0.1f32, 0.4, 0.25, 0.3]);
        // The convolver itself has no addressable param: `room` resolves to room_mix only.
        assert_eq!(
            get_control_data("room", "undefined"),
            data("room_mix", "gain")
        );
        let t = table(json!({ "s": "sine", "FX": [{ "room": 0.3 }] }));
        let NodeList::Array(e) = t.get(FxKey::Index(0), "room").unwrap() else {
            panic!()
        };
        assert!(node_param(&t, e[0].unwrap(), "gain").is_none());
    }

    #[test]
    fn modulators_of_earlier_entries() {
        // Entry 0's lfo runs first and registers `lfo_0` in main; main's env reaches it, but an
        // entry's config cannot reach a main modulator that runs later.
        let v = ok(json!({
            "s": "sawtooth", "cutoff": 800,
            "FX": [{ "cutoff": 500, "lfo": ids(&[json!({ "control": "cutoff" })]) }],
            "env": { "__ids": { "$set": ["e"] }, "e": { "control": "lfo_0", "subControl": "rate", "depthabs": 2 } }
        }));
        assert_eq!(v.mods().mods.len(), 2);
        assert_eq!(
            v.mods().conns[1].1,
            ParamRef {
                node: Node::Lfo(0),
                slot: ParamSlot(lfo::FREQUENCY as u8)
            }
        );
        let (mut v, e) = build(json!({
            "s": "sawtooth", "cutoff": 800,
            "FX": [{ "cutoff": 500, "env": { "__ids": { "$set": [0] }, "0": { "control": "lfo_7", "fxi": "main" } } }],
            "lfo": { "__ids": { "$set": [7] }, "7": { "control": "cutoff" } }
        }));
        assert!(e.is_none());
        assert_eq!(
            v.mods().mods.len(),
            1,
            "the env skipped: lfo_7 is not registered yet"
        );
        assert_eq!(v.take_logs().len(), 1);
    }

    #[test]
    fn modulators_of_modulators() {
        // `lfo_0` is found in main once registered; `rate` maps to its frequency.
        let v = ok(json!({ "s": "saw", "cutoff": 500, "lfo": ids(&[
            json!({ "control": "cutoff" }), json!({ "control": "lfo_0", "subControl": "rate", "depthabs": 3 })
        ]) }));
        assert_eq!(v.mods().mods.len(), 2);
        assert_eq!(
            v.mods().conns[1],
            (
                1,
                ParamRef {
                    node: Node::Lfo(0),
                    slot: ParamSlot(lfo::FREQUENCY as u8)
                }
            )
        );
        // currentValue: the f32 rate 1 of lfo 0; no range (the data name is frequency, but cv < 30).
        assert_eq!(lfo_param(&v, 1, lfo::MIN), -1.5);
        assert_eq!(
            v.node_table().keys(FxKey::Main),
            ["source", "gain", "lpf", "lpf_lfo", "post", "lfo_0", "lfo_1"]
        );
        // Only earlier configs: a forward reference skips; a skipped config is not registered; a
        // config cannot target itself.
        for (configs, n) in [
            (
                vec![json!({ "control": "lfo_1" }), json!({ "control": "gain" })],
                1,
            ),
            (
                vec![json!({ "control": "nope" }), json!({ "control": "lfo_0" })],
                0,
            ),
            (vec![json!({ "control": "lfo_0" })], 0),
        ] {
            let v = ok(json!({ "s": "sine", "lfo": ids(&configs) }));
            assert_eq!(v.mods().mods.len(), n, "{configs:?}");
        }
        // Duplicate textual ids: two iterations of one config, the second registration wins.
        let v = ok(json!({ "s": "sine", "lfo": {
            "__ids": { "$set": [0, "0"] }, "0": { "control": "gain" }
        }, "env": ids(&[json!({ "control": "lfo_0" })]) }));
        assert_eq!(v.mods().mods.len(), 3);
        assert_eq!(
            v.node_table().get(FxKey::Main, "lfo_0"),
            Some(&NodeList::Array(vec![Some(Node::Lfo(1))]))
        );
        assert_eq!(v.mods().conns[2].1.node, Node::Lfo(1));
        // An env can target an lfo, an lfo cannot reach an env (envs register after the lfos).
        let v = ok(json!({ "s": "sine",
            "lfo": ids(&[json!({ "control": "env_0" })]),
            "env": ids(&[json!({ "control": "gain" })]) }));
        assert_eq!(v.mods().mods.len(), 1);
        assert!(matches!(v.mods().mods[0].node, ModNode::Env(_)));
    }

    #[test]
    fn last_writer_fm_gains() {
        let t = table(json!({ "s": "sine", "fmi": 1 }));
        let one = |n: Node| Some(NodeList::Array(vec![Some(n)]));
        assert_eq!(t.get(FxKey::Main, "fm_1").cloned(), one(Node::FmOsc(0)));
        assert_eq!(
            t.get(FxKey::Main, "fm_1_gain").cloned(),
            one(Node::FmAmount(0))
        );
        assert_eq!(
            t.keys(FxKey::Main),
            ["source", "fm_1", "fm_1_gain", "gain", "post"]
        );
        // fmi2 (2 to 1) after fmi: fm_2_gain is its connected gain, fm_1_gain the detached
        // target-side gain that overwrites fmi's.
        let t = table(json!({ "s": "sine", "fmi": 1, "fmi2": 0.5 }));
        assert_eq!(
            t.get(FxKey::Main, "fm_2_gain").cloned(),
            one(Node::FmAmount(1))
        );
        assert_eq!(
            t.get(FxKey::Main, "fm_1_gain").cloned(),
            one(Node::Detached { value: 0.5 })
        );
        // A noise target: no connection, both gains detached.
        let t = table(json!({ "s": "sine", "fmi2": 2, "fmwave": "white" }));
        assert_eq!(
            t.get(FxKey::Main, "fm_2_gain").cloned(),
            one(Node::Detached { value: 2.0 })
        );
        assert_eq!(
            t.get(FxKey::Main, "fm_1_gain").cloned(),
            one(Node::Detached { value: 2.0 })
        );
        assert!(!t.fm_is_noise(0) && t.fm_is_noise(1));
        // fmi11 registers fm_1_gain twice: the detached target side wins.
        let t = table(json!({ "s": "sine", "fmi11": 1 }));
        assert_eq!(
            t.get(FxKey::Main, "fm_1_gain").cloned(),
            one(Node::Detached { value: 1.0 })
        );
        // The vibrato before the FM on an oscillator, after it on a worklet synth.
        let t = table(json!({ "s": "sine", "fmi": 1, "vib": 4 }));
        assert_eq!(
            t.keys(FxKey::Main),
            [
                "source",
                "vib",
                "vib_gain",
                "fm_1",
                "fm_1_gain",
                "gain",
                "post"
            ]
        );
        let t = table(json!({ "s": "pulse", "fmi": 1, "vib": 4 }));
        assert_eq!(
            t.keys(FxKey::Main),
            [
                "source",
                "pw_lfo",
                "fm_1",
                "fm_1_gain",
                "vib",
                "vib_gain",
                "gain",
                "post"
            ]
        );
        // A modulation of a detached gain is accepted and changes nothing audible.
        let base = json!({ "s": "sine", "fmi": 1, "fmi2": 1 });
        let mut modulated = base.clone();
        modulated["lfo"] = ids(&[json!({ "control": "fmi", "depthabs": 50 })]);
        let render = |v: serde_json::Value| {
            let (mut voice, e) = build(v);
            assert!(e.is_none());
            let mut out = Vec::new();
            let mut bank = buses();
            for k in 0..60 {
                let mut o = VoiceOutput::new();
                voice.process(k * Q, &mut o, &mut bank);
                out.extend(o.out[0].iter().map(|x| x.to_bits()));
            }
            out
        };
        let (v, _) = build(modulated.clone());
        assert_eq!(v.mods().conns[0].1.node, Node::Detached { value: 1.0 });
        assert_eq!(render(base), render(modulated));
    }

    #[test]
    fn noise_targets_are_unsupported() {
        let r = |v: serde_json::Value, c: &str| {
            resolve(
                &table(v),
                Some(&Value::Str(c.into())),
                None,
                &Value::Str("main".into()),
            )
        };
        assert_eq!(
            r(json!({ "s": "white" }), "note"),
            Ok(Resolution::Unsupported)
        );
        assert_eq!(
            r(json!({ "s": "sine", "fmi": 1, "fmwave": "pink" }), "fmh"),
            Ok(Resolution::Unsupported)
        );
        assert!(matches!(
            r(json!({ "s": "sine", "fmi": 1, "fmwave": "pink" }), "fmi"),
            Ok(Resolution::Targets { .. })
        ));
        // The detune of a noise source is not reachable by another control.
        assert!(r(json!({ "s": "white" }), "detune").is_err());
        assert!(matches!(
            r(json!({ "s": "white" }), "gain"),
            Ok(Resolution::Targets { .. })
        ));
    }

    #[test]
    fn resolve_targets_every_element() {
        let t = table(json!({ "s": "saw", "cutoff": 800, "ftype": "24db", "vowel": "a" }));
        let main = Value::Str("main".into());
        let Ok(Resolution::Targets { param, refs }) =
            resolve(&t, Some(&Value::Str("cutoff".into())), None, &main)
        else {
            panic!("targets");
        };
        assert_eq!(param, "frequency");
        let stages: Vec<Node> = refs.iter().map(|r| r.node).collect();
        assert_eq!(stages, [Node::Stage(StageId(3)), Node::Stage(StageId(4))]);
        let Ok(Resolution::Targets { refs, .. }) =
            resolve(&t, Some(&Value::Str("vowel".into())), None, &main)
        else {
            panic!("targets");
        };
        assert_eq!(refs.len(), 5);
        assert!(refs.iter().all(|r| r.slot == ParamSlot::BIQUAD_FREQUENCY));
        // fxi `main` given as a string, and the default for a missing fxi.
        assert!(resolve(&t, Some(&Value::Str("gain".into())), None, &main).is_ok());
        assert_eq!(fxi_of(&map(json!({})), FxKey::Main), main);
        assert_eq!(fxi_of(&map(json!({ "fxi": null })), FxKey::Main), main);
        assert_eq!(
            fxi_of(&map(json!({ "fxi": 0 })), FxKey::Main),
            Value::Num(0.0)
        );
    }

    #[test]
    fn processing_runs_drivers_before_their_targets() {
        // m2 targets m1's rate, m1 targets m0's rate, m0 the gain: rendered 2, 1, 0.
        let v = ok(json!({ "s": "sine", "gain": 0.5, "lfo": ids(&[
            json!({ "control": "gain", "rate": 3 }),
            json!({ "control": "lfo_0", "subControl": "rate", "depthabs": 2 }),
            json!({ "control": "lfo_1", "subControl": "rate", "depthabs": 1 }),
        ]) }));
        let mut set = v.mods().clone();
        let lfo = |k: usize| match &set.mods[k].node {
            ModNode::Lfo(l) => (**l).clone(),
            _ => unreachable!(),
        };
        let (mut l0, mut l1, mut l2) = (lfo(0), lfo(1), lfo(2));
        let mut bank = buses();
        let mut errors = Vec::new();
        for k in 0..40 {
            let q = k * Q;
            set.evaluate(q, &mut bank, &mut errors);
            let mut b2 = [0.0f32; QUANTUM];
            let s2 = l2.render(q, &lfo::NO_INPUTS, &mut b2);
            let mut i1 = lfo::NO_INPUTS;
            i1[lfo::FREQUENCY].connected = true;
            if !s2 {
                i1[lfo::FREQUENCY].bufs.push(b2);
            }
            let mut b1 = [0.0f32; QUANTUM];
            let s1 = l1.render(q, &i1, &mut b1);
            let mut i0 = lfo::NO_INPUTS;
            i0[lfo::FREQUENCY].connected = true;
            if !s1 {
                i0[lfo::FREQUENCY].bufs.push(b1);
            }
            let mut b0 = [0.0f32; QUANTUM];
            l0.render(q, &i0, &mut b0);
            for (m, want) in [(0, b0), (1, b1), (2, b2)] {
                let got: Vec<u32> = set.out[m].bufs[0].iter().map(|x| x.to_bits()).collect();
                let want: Vec<u32> = want.iter().map(|x| x.to_bits()).collect();
                assert_eq!(got, want, "modulator {m}, quantum {k}");
            }
        }
        assert!(errors.is_empty());
    }

    #[test]
    fn connections_sum_in_connection_order_and_connect_silent_params() {
        let v = ok(json!({ "s": "sine", "lfo": ids(&[
            json!({ "control": "gain", "rate": 3 }), json!({ "control": "gain", "rate": 5 })
        ]) }));
        let gain = (Node::Stage(StageId(2)), ParamSlot::GAIN_GAIN);
        let mut set = v.mods().clone();
        let mut bank = buses();
        set.evaluate(40 * Q, &mut bank, &mut Vec::new());
        let i = set.view().inputs(gain.0, gain.1);
        assert_eq!(i.bufs.len(), 2);
        assert_eq!(i.bufs[0], set.out[0].bufs[0]);
        assert_eq!(i.bufs[1], set.out[1].bufs[0]);
        assert_ne!(i.bufs[0], i.bufs[1]);
        // Other params are not connected.
        assert!(!set.view().connected(gain.0, ParamSlot(1)));
        assert!(!set.view().connected(Node::Stage(StageId(1)), gain.1));
        // Once the lfos are gone (end 0.36 s) their outputs are silent: the gain stays
        // connected (the sample-accurate path), with no buffer.
        for k in 41..140 {
            set.evaluate(k * Q, &mut bank, &mut Vec::new());
        }
        let i = set.view().inputs(gain.0, gain.1);
        assert!(i.connected && i.bufs.is_empty());
    }

    #[test]
    fn the_patched_bmod_graph() {
        let v = ok(json!({ "s": "saw", "cutoff": 600,
            "bmod": ids(&[json!({ "bus": 2, "dc": 0.5, "control": "cutoff" })]) }));
        let ModNode::Bmod(b) = &v.mods().mods[0].node else {
            panic!("a bmod");
        };
        assert_eq!(b.bus, buses().index_of("2").unwrap());
        assert_eq!(b.dc.offset(), 0.5);
        assert_eq!(b.depth.gain.value(), (600.0f64 / 0.3) as f32);
        assert_eq!(b.shifted.gain.value(), 1.0);
        // The gate: 0, open at begin, closed at endWithRelease.
        let ev: Vec<(f32, f64)> = b
            .gate
            .gain
            .events()
            .iter()
            .map(|e| (e.value, e.time))
            .collect();
        assert_eq!(ev, [(0.0, 0.0), (1.0, 0.1), (0.0, 0.1 + 0.25 + 0.01)]);
        assert_eq!(
            v.mods().mods[0].end_quantum(),
            crate::render::round_up_128(crate::render::frame_for_param_event(0.36))
        );
        // A bmod registers nothing; its output is the down-mix of the gated sum, silent before
        // the dc starts and while the gate is shut.
        assert!(
            v.node_table()
                .keys(FxKey::Main)
                .iter()
                .all(|k| !k.starts_with("bmod"))
        );
        let mut set = v.mods().clone();
        let mut bank = buses();
        set.evaluate(0, &mut bank, &mut Vec::new());
        assert!(set.out[0].silent);
        set.evaluate(4480, &mut bank, &mut Vec::new());
        let o = &set.out[0];
        assert!(!o.silent && o.n == 2);
        // dc 0.5 through depth 2000 (the bus is silent): each half is 0.5 * 1000.
        assert_eq!(o.bufs[0][127], 0.5 * (0.5 * (600.0f64 / 0.3) as f32));
        assert_eq!(o.bufs[0], o.bufs[1]);
        // dc absent (or null) is 0: a silent source.
        let v = ok(json!({ "s": "saw", "cutoff": 600,
            "bmod": ids(&[json!({ "bus": 2, "dc": null, "control": "cutoff" })]) }));
        let mut set = v.mods().clone();
        set.evaluate(4480, &mut bank, &mut Vec::new());
        assert!(set.out[0].silent);
        // A missing bus is the text `undefined`.
        ok(json!({ "s": "sine", "bmod": ids(&[json!({ "control": "gain" })]) }));
    }

    #[test]
    fn orbit_held_modulators_keep_the_voice_alive() {
        let base = json!({ "s": "sine", "delay": 0.5, "delaytime": 0.1, "delayfeedback": 0.5,
                           "release": 0.01, "FXrelease": 0.5 });
        let free_at = |v: serde_json::Value| {
            let (mut voice, e) = build(v);
            assert!(e.is_none());
            let mut bank = buses();
            let mut q = 0;
            loop {
                let mut o = VoiceOutput::new();
                if !voice.process(q, &mut o, &mut bank) {
                    return (q, voice);
                }
                q += Q;
            }
        };
        let (plain, _) = free_at(base.clone());
        let mut m = base.clone();
        m["lfo"] = ids(&[json!({ "control": "delaytime", "depthabs": 0.01 })]);
        let (held, v) = free_at(m);
        let end = v.mods().mods[0].end_quantum();
        assert!(plain < end, "{plain} {end}");
        assert_eq!(held, end, "freed in the first quantum the lfo is gone");
        // Each quantum the voice hands the lfo's output to the orbit, in connection order.
        let (mut voice, _) = build(json!({ "s": "sine", "delay": 0.5, "delaytime": 0.1,
            "delayfeedback": 0.5, "djf": 0.3, "lfo": ids(&[
                json!({ "control": "delayfeedback", "depthabs": 0.1 }),
                json!({ "control": "djf", "depthabs": 0.1 }),
                json!({ "control": "delaytime", "depthabs": 0.01 }),
            ]) }));
        let mut o = VoiceOutput::new();
        voice.process(0, &mut o, &mut buses());
        let params: Vec<OrbitParam> = o.orbit_inputs.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            params,
            [
                OrbitParam::Feedback,
                OrbitParam::DjfValue,
                OrbitParam::DelayTime
            ]
        );
        // A live lfo outputs zeros before its begin (not a silent bus).
        assert!(
            o.orbit_inputs
                .iter()
                .all(|(_, b)| b.is_some_and(|b| b.iter().all(|x| *x == 0.0))),
            "zeros before begin"
        );
        voice.process(40 * Q, &mut o, &mut buses());
        assert!(
            o.orbit_inputs
                .iter()
                .all(|(_, b)| b.is_some_and(|b| b.iter().any(|x| *x != 0.0)))
        );
    }

    #[test]
    fn the_orbit_sums_its_voices_in_voice_then_connection_order() {
        let (a, b, c) = ([1.0f32; QUANTUM], [2.0f32; QUANTUM], [3.0f32; QUANTUM]);
        let mut o1 = VoiceOutput::new();
        o1.orbit_inputs = vec![(OrbitParam::DelayTime, Some(a))];
        let mut o2 = VoiceOutput::new();
        o2.orbit_inputs = vec![
            (OrbitParam::DelayTime, Some(b)),
            (OrbitParam::DjfValue, None),
            (OrbitParam::DelayTime, Some(c)),
        ];
        let mut o3 = VoiceOutput::new();
        o3.orbit_inputs = vec![(OrbitParam::Feedback, Some(c))];
        let outputs = [o1, o2, o3];
        let mut all = Vec::new();
        OrbitInputs::fill_all(&mut all, 2, &outputs, &[0, 0, 1]);
        let i = &all[0];
        assert_eq!(i.delay_time.bufs, [a, b, c]);
        assert!(i.delay_time.connected && i.djf.connected && i.djf.bufs.is_empty());
        assert!(!i.feedback.connected, "the third voice is on another orbit");
        assert!(all[1].feedback.connected);
        // Once the voices are freed nothing is connected (the reused inputs are cleared).
        OrbitInputs::fill_all(&mut all, 2, &[], &[]);
        let none = &all[0];
        assert!(!none.delay_time.connected && !none.feedback.connected && !none.djf.connected);
        assert!(none.delay_time.bufs.is_empty() && !all[1].feedback.connected);
    }

    #[test]
    fn config_shapes() {
        let v = |j: serde_json::Value| -> Value { serde_json::from_value(j).unwrap() };
        assert!(shape_ok(ModType::Lfo, &v(ids(&[json!({})]))));
        assert!(shape_ok(
            ModType::Lfo,
            &v(json!({ "__ids": { "$set": [] } }))
        ));
        assert!(!shape_ok(ModType::Lfo, &v(json!(3))));
        assert!(!shape_ok(ModType::Lfo, &v(json!({ "0": {} }))));
        assert!(!shape_ok(
            ModType::Env,
            &v(json!({ "__ids": [0], "0": {} }))
        ));
        assert!(!shape_ok(
            ModType::Bmod,
            &v(json!({ "__ids": { "$set": [0] }, "0": 1 }))
        ));
        assert!(!shape_ok(
            ModType::Bmod,
            &v(json!({ "__ids": { "$set": [1] }, "0": {} }))
        ));
        // String ids are looked up as themselves, number ids through String(id).
        let m = v(json!({ "__ids": { "$set": ["a", 1.5] }, "a": {}, "1.5": {} }));
        assert_eq!(configs_of(FxKey::Main, ModType::Lfo, &m).unwrap().len(), 2);
        // The key lists are sorted (bad_key's binary search).
        for keys in [LFO_KEYS, ENV_KEYS, BMOD_KEYS] {
            assert!(keys.windows(2).all(|w| w[0] < w[1]));
        }
        assert_eq!(
            bad_key(ModType::Lfo, &map(json!({ "rate": 1, "zz": 1 }))),
            Some("zz")
        );
        assert_eq!(bad_key(ModType::Bmod, &map(json!({ "zz": 1 }))), None);
    }

    #[test]
    fn unsupported_targets_labels() {
        let labels = |v: serde_json::Value| unsupported_targets(&map(v), 0.5);
        assert_eq!(
            labels(json!({ "s": "white", "lfo": ids(&[json!({ "control": "note" })]) })),
            ["lfo:note"]
        );
        // Registered nodes count for later configs; a skip and a supported target give nothing.
        assert!(labels(json!({ "s": "white", "lfo": ids(&[
            json!({ "control": "gain" }), json!({ "control": "lfo_0" }), json!({ "control": "nope" })
        ]) }))
        .is_empty());
        // A throw ends the walk.
        assert!(
            labels(json!({ "s": "white", "lfo": ids(&[
            json!({ "control": "fmh", "fxi": "q" }), json!({ "control": "note" })
        ]) }))
            .is_empty()
        );
        // A value of the wrong shape, controls the build rejects, a sound it does not build.
        assert!(labels(json!({ "s": "white", "lfo": 3 })).is_empty());
        assert!(
            labels(json!({ "s": "kick", "lfo": ids(&[json!({ "control": "note" })]) })).is_empty()
        );
        assert!(labels(json!({ "s": "white" })).is_empty());
    }

    /// Today's modulator targets (the dump's `modulatorTargets`). The pin lives here because
    /// `controls.md` classifies keys and lists no targets; it catches a probe that shrinks (a
    /// `NodeTable::plan` change that stops exposing a node on the probe voices) or grows.
    /// Changing it is a deliberate edit, in the same diff as the dump change it causes.
    pub(crate) const TODAY: [&str; 91] = [
        "bandf",
        "bandq",
        "bpdc",
        "bpdepth",
        "bpdepthfrequency",
        "bpshape",
        "bpskew",
        "coarse",
        "compressor",
        "compressorAttack",
        "compressorKnee",
        "compressorRatio",
        "compressorRelease",
        "crush",
        "cutoff",
        "delay",
        "delayfeedback",
        "delaysync",
        "delaytime",
        "detune",
        "distort",
        "distorttype",
        "distortvol",
        "djf",
        "env",
        "env_attack",
        "env_decay",
        "env_release",
        "env_sustain",
        "fmh",
        "fmh2",
        "fmh3",
        "fmh4",
        "fmh5",
        "fmh6",
        "fmh7",
        "fmh8",
        "fmi",
        "fmi2",
        "fmi3",
        "fmi4",
        "fmi5",
        "fmi6",
        "fmi7",
        "fmi8",
        "freq",
        "gain",
        "hcutoff",
        "hpdc",
        "hpdepth",
        "hpdepthfrequency",
        "hpshape",
        "hpskew",
        "hresonance",
        "lfo",
        "lfo_curve",
        "lfo_dcoffset",
        "lfo_depth",
        "lfo_depthabs",
        "lfo_rate",
        "lfo_skew",
        "lfo_sync",
        "lpdc",
        "lpdepth",
        "lpdepthfrequency",
        "lpshape",
        "lpskew",
        "note",
        "pan",
        "phasercenter",
        "phaserdepth",
        "phaserrate",
        "phasersweep",
        "postgain",
        "pw",
        "pwrate",
        "pwsweep",
        "resonance",
        "room",
        "s",
        "shape",
        "shapevol",
        "spread",
        "tremolo",
        "tremolodepth",
        "tremoloshape",
        "tremoloskew",
        "tremolosync",
        "vib",
        "vibmod",
        "vowel",
    ];

    #[test]
    fn every_planned_name_is_a_target_node() {
        use crate::chain::tests::STATIC_NAMES;
        let mut planned = std::collections::BTreeSet::new();
        for table in probe_tables() {
            for key in [FxKey::Main, FxKey::Index(0)] {
                for name in table.keys(key) {
                    if name != "lfo_0" && name != "env_0" {
                        planned.insert(name.to_string());
                    }
                }
            }
        }
        // `room` is the orbit's convolver on `Main` and the local reverb on a chain entry: no
        // param of it is a control's target. `TARGETS` reaches the reverb's level through
        // `room_mix`.
        for name in &planned {
            if name == "room" {
                continue;
            }
            assert!(
                TARGETS.iter().any(|(_, node, _)| node == name),
                "{name} is planned but no TARGETS row names it"
            );
        }
        // The probe reaches every name the plan can register.
        let mut every: Vec<String> = STATIC_NAMES
            .iter()
            .map(|n| n.as_str().to_string())
            .collect();
        for n in 1..=8u8 {
            every.push(NodeName::Fm(n).as_str().to_string());
            every.push(NodeName::FmGain(n).as_str().to_string());
        }
        for name in &every {
            assert!(planned.contains(name), "the probe tables never plan {name}");
        }
    }

    #[test]
    fn modulatable_targets_are_todays() {
        assert_eq!(modulatable_targets(), TODAY);
    }

    #[test]
    fn modulatable_targets_are_known_controls() {
        let targets = modulatable_targets();
        assert!(targets.windows(2).all(|w| w[0] < w[1]), "targets sorted");
        // Every target is a control superdough's table knows and resolves somewhere: the
        // skips and throws are not listed.
        for t in &targets {
            assert!(get_control_data(t, "undefined").is_some(), "{t}");
        }
        for t in [
            "bmod",
            "dry",
            "busgain",
            "stretch",
            "wtrate",
            "tremolophase",
            "lprate",
            "lpsync",
            "wt",
            "transient",
        ] {
            assert!(!targets.contains(&t), "{t}");
        }
    }

    /// A row left out of the targets only skips or throws on every probe table: one that
    /// resolved to `Unsupported` and never to `Targets` would be a target the engine refuses,
    /// hidden by the dump.
    #[test]
    fn every_target_row_is_derived_or_never_unsupported() {
        let targets = modulatable_targets();
        let tables = probe_tables();
        for (key, _, _) in TARGETS {
            if targets.contains(key) {
                continue;
            }
            for table in &tables {
                for o in row_outcomes(table, key) {
                    assert!(
                        matches!(o, Outcome::Skip | Outcome::Throws),
                        "{key} is left out but resolves to {o:?}"
                    );
                }
            }
        }
    }
}

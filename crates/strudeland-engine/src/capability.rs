//! What the engine plays: the control allowlist (audited in `controls.md` in the crate root; a
//! test keeps the two in step), the sounds, the FX-chain and modulator keys, the only
//! unsupported detection (`unsupported_signatures`, behind the binary's `signatures`
//! subcommand, the render refusal and the live warning) and the `controls --json` dump.

use std::collections::BTreeSet;

use crate::chain::SourceKind;
use crate::coerce::{js_to_string, js_truthy};
use crate::job::{EngineJob, Value, ValueMap, bar_of};
use crate::params::{SILENCE, is_silence, is_silence_str, sound_of};

/// Every key the engine accepts: the `implemented` and `ignored` rows of `controls.md`, sorted
/// by byte order.
pub const ALLOWED_KEYS: &[&str] = &[
    "FX",
    "FXrelease",
    "accelerate",
    "activeLabel",
    "amp",
    "analyze",
    "anchor",
    "attack",
    "bandf",
    "bandq",
    "bank",
    "begin",
    "binshift",
    "bmod",
    "bpattack",
    "bpdc",
    "bpdecay",
    "bpdepth",
    "bpdepthfrequency",
    "bpenv",
    "bprate",
    "bprelease",
    "bpshape",
    "bpskew",
    "bpsustain",
    "bpsync",
    "bus",
    "busgain",
    "byteBeatExpression",
    "byteBeatStartTime",
    "ccn",
    "ccv",
    "channel",
    "channels",
    "chord",
    "chorus",
    "clip",
    "coarse",
    "color",
    "colour",
    "comb",
    "compressor",
    "compressorAttack",
    "compressorKnee",
    "compressorRatio",
    "compressorRelease",
    "cps",
    "crush",
    "ctlNum",
    "ctranspose",
    "curve",
    "cut",
    "cutoff",
    "decay",
    "degree",
    "delay",
    "delayfeedback",
    "delaysync",
    "delaytime",
    "deltaSlide",
    "density",
    "detune",
    "dictionary",
    "distort",
    "distorttype",
    "distortvol",
    "djf",
    "drive",
    "dry",
    "duckattack",
    "duckdepth",
    "duckonset",
    "duckorbit",
    "duration",
    "end",
    "enhance",
    "env",
    "expression",
    "fadeInTime",
    "fadeTime",
    "fanchor",
    "fft",
    "fit",
    "fmattack",
    "fmattack2",
    "fmattack3",
    "fmattack4",
    "fmattack5",
    "fmattack6",
    "fmattack7",
    "fmattack8",
    "fmdecay",
    "fmdecay2",
    "fmdecay3",
    "fmdecay4",
    "fmdecay5",
    "fmdecay6",
    "fmdecay7",
    "fmdecay8",
    "fmenv",
    "fmenv2",
    "fmenv3",
    "fmenv4",
    "fmenv5",
    "fmenv6",
    "fmenv7",
    "fmenv8",
    "fmh",
    "fmh2",
    "fmh3",
    "fmh4",
    "fmh5",
    "fmh6",
    "fmh7",
    "fmh8",
    "fmi",
    "fmi00",
    "fmi01",
    "fmi02",
    "fmi03",
    "fmi04",
    "fmi05",
    "fmi06",
    "fmi07",
    "fmi08",
    "fmi10",
    "fmi11",
    "fmi12",
    "fmi13",
    "fmi14",
    "fmi15",
    "fmi16",
    "fmi17",
    "fmi18",
    "fmi2",
    "fmi20",
    "fmi21",
    "fmi22",
    "fmi23",
    "fmi24",
    "fmi25",
    "fmi26",
    "fmi27",
    "fmi28",
    "fmi3",
    "fmi30",
    "fmi31",
    "fmi32",
    "fmi33",
    "fmi34",
    "fmi35",
    "fmi36",
    "fmi37",
    "fmi38",
    "fmi4",
    "fmi40",
    "fmi41",
    "fmi42",
    "fmi43",
    "fmi44",
    "fmi45",
    "fmi46",
    "fmi47",
    "fmi48",
    "fmi5",
    "fmi50",
    "fmi51",
    "fmi52",
    "fmi53",
    "fmi54",
    "fmi55",
    "fmi56",
    "fmi57",
    "fmi58",
    "fmi6",
    "fmi60",
    "fmi61",
    "fmi62",
    "fmi63",
    "fmi64",
    "fmi65",
    "fmi66",
    "fmi67",
    "fmi68",
    "fmi7",
    "fmi70",
    "fmi71",
    "fmi72",
    "fmi73",
    "fmi74",
    "fmi75",
    "fmi76",
    "fmi77",
    "fmi78",
    "fmi8",
    "fmi80",
    "fmi81",
    "fmi82",
    "fmi83",
    "fmi84",
    "fmi85",
    "fmi86",
    "fmi87",
    "fmi88",
    "fmrelease",
    "fmrelease2",
    "fmrelease3",
    "fmrelease4",
    "fmrelease5",
    "fmrelease6",
    "fmrelease7",
    "fmrelease8",
    "fmsustain",
    "fmsustain2",
    "fmsustain3",
    "fmsustain4",
    "fmsustain5",
    "fmsustain6",
    "fmsustain7",
    "fmsustain8",
    "fmwave",
    "fmwave2",
    "fmwave3",
    "fmwave4",
    "fmwave5",
    "fmwave6",
    "fmwave7",
    "fmwave8",
    "frameRate",
    "frames",
    "freeze",
    "freq",
    "fshift",
    "fshiftnote",
    "fshiftphase",
    "ftype",
    "gain",
    "gate",
    "harmonic",
    "hbrick",
    "hcutoff",
    "hold",
    "hours",
    "hpattack",
    "hpdc",
    "hpdecay",
    "hpdepth",
    "hpdepthfrequency",
    "hpenv",
    "hprate",
    "hprelease",
    "hpshape",
    "hpskew",
    "hpsustain",
    "hpsync",
    "hresonance",
    "i",
    "imag",
    "kcutoff",
    "krush",
    "label",
    "lbrick",
    "leslie",
    "lfo",
    "lock",
    "loop",
    "loopBegin",
    "loopEnd",
    "lpattack",
    "lpdc",
    "lpdecay",
    "lpdepth",
    "lpdepthfrequency",
    "lpenv",
    "lprate",
    "lprelease",
    "lpshape",
    "lpskew",
    "lpsustain",
    "lpsync",
    "lrate",
    "lsize",
    "markcss",
    "midibend",
    "midichan",
    "midicmd",
    "midimap",
    "midiport",
    "miditouch",
    "minutes",
    "mode",
    "mtranspose",
    "n",
    "noise",
    "note",
    "nrpnn",
    "nrpv",
    "nudge",
    "octave",
    "octaveR",
    "octaves",
    "octer",
    "octersub",
    "octersubsub",
    "offset",
    "orbit",
    "oschost",
    "oscport",
    "overgain",
    "overshape",
    "pan",
    "panchor",
    "panorient",
    "panspan",
    "pansplay",
    "panwidth",
    "partials",
    "pattack",
    "pcurve",
    "pdecay",
    "penv",
    "phasercenter",
    "phaserdepth",
    "phaserrate",
    "phasersweep",
    "phases",
    "pitchJump",
    "pitchJumpTime",
    "polyTouch",
    "postgain",
    "prelease",
    "progNum",
    "psustain",
    "pw",
    "pwrate",
    "pwsweep",
    "real",
    "release",
    "resonance",
    "ring",
    "ringdf",
    "ringf",
    "room",
    "roomdim",
    "roomfade",
    "roomlp",
    "roomsize",
    "s",
    "scram",
    "seconds",
    "semitone",
    "shape",
    "shapevol",
    "size",
    "slice",
    "slide",
    "smear",
    "songPtr",
    "speed",
    "splice",
    "spread",
    "squiz",
    "stepsPerOctave",
    "sustain",
    "sustainpedal",
    "sysexdata",
    "sysexid",
    "transient",
    "transsustain",
    "tremolo",
    "tremolodepth",
    "tremolophase",
    "tremoloshape",
    "tremoloskew",
    "tremolosync",
    "triode",
    "tsdelay",
    "uid",
    "unison",
    "unit",
    "val",
    "velocity",
    "vib",
    "vibmod",
    "voice",
    "vowel",
    "warp",
    "warpattack",
    "warpdc",
    "warpdecay",
    "warpdepth",
    "warpenv",
    "warpmode",
    "warprate",
    "warprelease",
    "warpshape",
    "warpskew",
    "warpsustain",
    "warpsync",
    "waveloss",
    "wt",
    "wtattack",
    "wtdc",
    "wtdecay",
    "wtdepth",
    "wtenv",
    "wtphaserand",
    "wtrate",
    "wtrelease",
    "wtshape",
    "wtskew",
    "wtsustain",
    "wtsync",
    "xsdelay",
    "zcrush",
    "zdelay",
    "zmod",
    "znoise",
    "zrand",
    "zzfx",
];

/// Every `s` the engine accepts, sorted by byte order: the sounds it builds
/// (`SourceKind::NAMES`) and the silence markers (`SILENCE`).
pub fn allowed_sounds() -> Vec<&'static str> {
    let mut sounds: Vec<&'static str> = SourceKind::NAMES
        .iter()
        .map(|(name, _)| *name)
        .chain(SILENCE.iter().copied())
        .collect();
    sounds.sort_unstable();
    sounds
}

/// Keys the engine implements inside an `FX` chain entry (a subset of [`ALLOWED_KEYS`]), sorted:
/// the keys superdough reads per entry (superdough.mjs:612-938; research `modulation-routing.md`
/// 4.2) without [`CHAIN_UNSUPPORTED_KEYS`], the lowpass, highpass and bandpass families (their
/// cutoff, Q, envelope and LFO keys, and the maps' `fanchor`, `ftype`, `drive`), and the
/// modulators `lfo`, `env`, `bmod`.
pub const CHAIN_KEYS: &[&str] = &[
    "bandf",
    "bandq",
    "bmod",
    "bpattack",
    "bpdc",
    "bpdecay",
    "bpdepth",
    "bpdepthfrequency",
    "bpenv",
    "bprate",
    "bprelease",
    "bpshape",
    "bpskew",
    "bpsustain",
    "bpsync",
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
    "distort",
    "distorttype",
    "distortvol",
    "drive",
    "dry",
    "env",
    "fanchor",
    "ftype",
    "gain",
    "hcutoff",
    "hpattack",
    "hpdc",
    "hpdecay",
    "hpdepth",
    "hpdepthfrequency",
    "hpenv",
    "hprate",
    "hprelease",
    "hpshape",
    "hpskew",
    "hpsustain",
    "hpsync",
    "hresonance",
    "i",
    "lfo",
    "lpattack",
    "lpdc",
    "lpdecay",
    "lpdepth",
    "lpdepthfrequency",
    "lpenv",
    "lprate",
    "lprelease",
    "lpshape",
    "lpskew",
    "lpsustain",
    "lpsync",
    "pan",
    "phasercenter",
    "phaserdepth",
    "phaserrate",
    "phasersweep",
    "resonance",
    "room",
    "roomdim",
    "roomfade",
    "roomlp",
    "roomsize",
    "shape",
    "shapevol",
    "transient",
    "transsustain",
    "tremolo",
    "tremolodepth",
    "tremolophase",
    "tremoloshape",
    "tremoloskew",
    "tremolosync",
    "velocity",
    "vowel",
];

/// Keys superdough reads per `FX` entry that the engine refuses there (`FX.<key>`), sorted.
pub const CHAIN_UNSUPPORTED_KEYS: &[&str] = &[
    "ir",
    "irbegin",
    "irspeed",
    "stretch",
    "workletInputs",
    "workletSrc",
];

/// The valid config sub-keys of each modulator type (`bmod`, `env`, `lfo`), each list sorted: the
/// keys its connect function and processor read or overwrite (any other `lfo` or `env` key
/// throws; `bmod` ignores the others).
pub const MODULATOR_KEYS: &[(&str, &[&str])] = &[
    ("bmod", crate::modulate::BMOD_KEYS),
    ("env", crate::modulate::ENV_KEYS),
    ("lfo", crate::modulate::LFO_KEYS),
];

// ---- unsupported detection

/// A sound and the controls the engine cannot play, first seen in layout bar `first_bar`
/// (serialized as `{ s, keys, firstBar }`, the TS `UnsupportedSignature`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedSignature {
    pub s: String,
    pub keys: Vec<String>,
    pub first_bar: i64,
}

/// The unsupported part of one control map: `None` for a silence marker or a fully supported
/// map, else `(s, sorted unique bad keys)` where `s` is the defaulted, bank-prefixed sound.
pub fn signature_of(map: &ValueMap, cps: f64) -> Option<(String, Vec<String>)> {
    signature_with(map, false, cps)
}

/// [`signature_of`] for a value whose bus-cycle status is known (the live engine passes
/// [`crate::bus::BusGraph::cyclic_readers`] of its growing graph): `bus-cycle` is among the keys
/// when `bus_cycle`.
pub fn signature_of_with_bus(
    map: &ValueMap,
    bus_cycle: bool,
    cps: f64,
) -> Option<(String, Vec<String>)> {
    signature_with(map, bus_cycle, cps)
}

/// [`signature_of`], with `bus-cycle` among the keys when `bus_cycle` (the value reads a bus on
/// a cycle of the job's bus graph, [`crate::bus::BusGraph::cyclic_readers`]). A modulator value (`lfo`,
/// `env`, `bmod`) of the wrong shape is its key; a modulator target the engine cannot render is
/// `<type>:<control>` (`modulate::unsupported_targets`).
fn signature_with(map: &ValueMap, bus_cycle: bool, cps: f64) -> Option<(String, Vec<String>)> {
    let s0 = sound_of(map);
    if is_silence(&s0) {
        return None;
    }
    let s = match map.get("bank") {
        Some(bank) if js_truthy(bank) && js_truthy(&s0) => {
            format!("{}_{}", js_to_string(bank), js_to_string(&s0))
        }
        _ => js_to_string(&s0),
    };
    let mut bad: BTreeSet<String> = BTreeSet::new();
    if bus_cycle {
        bad.insert("bus-cycle".into());
    }
    for (k, v) in map {
        if k == "FX" {
            // Rule 4: the entries are checked by their own rules, not by the marker test.
            fx_signature(v, &mut bad);
            continue;
        }
        if ALLOWED_KEYS.binary_search(&k.as_str()).is_err() || v.contains_unsupported() {
            bad.insert(k.clone());
        }
        value_rules(k, v, "", &mut bad);
    }
    bad.extend(crate::modulate::unsupported_targets(map, cps));
    // The silence half keeps a non-string `s` whose JS string is a marker (`["-"]`) accepted;
    // `is_silence` sees only strings.
    let sound_ok = SourceKind::of(&s).is_some() || is_silence_str(&s);
    if sound_ok && bad.is_empty() {
        return None;
    }
    Some((s, bad.into_iter().collect()))
}

/// The value rules of one key `k` (top level, or `FX.` inside an entry: `prefix`): rule 2a (JS
/// coerces a non-scalar `distorttype`, `[3] % 9 === 3`; the engine's `getDistortionAlgorithm`
/// models numbers, strings, booleans and null only) and rule 4's modulator shape (a value the loop
/// cannot iterate). No value of an allowed key is unsupported on its own.
fn value_rules(k: &str, v: &Value, prefix: &str, bad: &mut BTreeSet<String>) {
    if k == "distorttype" && matches!(v, Value::Arr(_) | Value::Obj(_) | Value::Set(_)) {
        bad.insert(format!("{prefix}{k}"));
    }
    if let Some(ty) = crate::modulate::ModType::ALL.iter().find(|t| t.key() == k)
        && !crate::modulate::shape_ok(*ty, v)
    {
        bad.insert(format!("{prefix}{k}"));
    }
}

/// Rule 4 for the hap's `FX` value: `FX` when it is not an array or an entry is not a plain
/// object; per entry only the keys superdough reads there: a key of [`CHAIN_UNSUPPORTED_KEYS`] is
/// `FX.<key>`, a key of [`CHAIN_KEYS`] holding a `$unsupported` marker is `FX.<key>`, and the
/// value rules apply with the `FX.` prefix. Any other key of an entry is ignored, a marker
/// included. The labels carry no entry index.
fn fx_signature(v: &Value, bad: &mut BTreeSet<String>) {
    let Value::Arr(entries) = v else {
        bad.insert("FX".into());
        return;
    };
    for e in entries {
        let Value::Obj(m) = e else {
            bad.insert("FX".into());
            continue;
        };
        for (k, v) in m {
            if CHAIN_UNSUPPORTED_KEYS.binary_search(&k.as_str()).is_ok() {
                bad.insert(format!("FX.{k}"));
                continue;
            }
            if CHAIN_KEYS.binary_search(&k.as_str()).is_err() {
                continue;
            }
            if v.contains_unsupported() {
                bad.insert(format!("FX.{k}"));
            }
            value_rules(k, v, "FX.", bad);
        }
    }
}

/// Distinct unsupported `{ s, keys }` signatures over every hap of the job in order, each with
/// the bar (`floor(cycle)`) of its first hap. A value that reads a bus on a cycle of the job's
/// bus graph lists `bus-cycle`. The only unsupported detection: `render` reports these, and
/// `strudeland-engine signatures` prints them for the harness.
pub fn unsupported_signatures(job: &EngineJob) -> Vec<UnsupportedSignature> {
    unsupported_signatures_in(job, &crate::bus::BusGraph::from_job(job))
}

/// [`unsupported_signatures`] with the job's bus graph already built (`graph` must be
/// `BusGraph::from_job(job)`), so the renderer reuses the graph it plays.
pub(crate) fn unsupported_signatures_in(
    job: &EngineJob,
    graph: &crate::bus::BusGraph,
) -> Vec<UnsupportedSignature> {
    let mut out: Vec<UnsupportedSignature> = Vec::new();
    let cyclic = graph.cyclic_readers();
    // Synced controls can change support when the same map plays at another rate.
    let mut last_cps = vec![None; job.values.len()];
    for hap in job.pauses.iter().flat_map(|p| &p.haps) {
        let Some(map) = job.values.get(hap.v as usize) else {
            continue;
        };
        let cps = hap.cps.unwrap_or(job.cps);
        if last_cps[hap.v as usize].replace(cps) == Some(cps) {
            continue;
        }
        let bus_cycle = cyclic.get(hap.v as usize).copied().unwrap_or(false);
        let Some((s, keys)) = signature_with(map, bus_cycle, cps) else {
            continue;
        };
        if !out.iter().any(|x| x.s == s && x.keys == keys) {
            out.push(UnsupportedSignature {
                s,
                keys,
                first_bar: bar_of(hap.cycle),
            });
        }
    }
    out
}

/// What `strudeland-engine controls --json` prints (documentation only; nothing reads it): one
/// compact line, lists sorted, fields `schema`, `keys`, `sounds`, `unsupportedValues`,
/// `chainKeys`, `modulatorKeys`, `modulatorTargets` in that order. `unsupportedValues` is always
/// `{}`: no value of an allowed key is unsupported on its own.
pub fn controls_json() -> String {
    let to_json = |list: &[&str]| serde_json::to_string(list).unwrap_or_default();
    let modulator_keys = MODULATOR_KEYS
        .iter()
        .map(|(t, keys)| format!("{}:{}", serde_json::Value::from(*t), to_json(keys)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"schema\":2,\"keys\":{},\"sounds\":{},\"unsupportedValues\":{{}},\"chainKeys\":{},\
         \"modulatorKeys\":{{{}}},\"modulatorTargets\":{}}}",
        to_json(ALLOWED_KEYS),
        to_json(&allowed_sounds()),
        to_json(CHAIN_KEYS),
        modulator_keys,
        to_json(&crate::modulate::modulatable_targets()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::value_map as map;
    use crate::params::{
        BANDPASS_KEYS, BANDPASS_LFO_KEYS, FM_MODULATORS, HIGHPASS_KEYS, HIGHPASS_LFO_KEYS,
        LOWPASS_KEYS, LOWPASS_LFO_KEYS, fm_control_name, fm_suffix, parse_entry_params,
    };
    use serde_json::json;

    #[test]
    fn a_non_scalar_distorttype_is_unsupported() {
        let sig = |v: serde_json::Value| signature_of(&map(v), 0.5);
        for bad in [json!([3]), json!({ "a": 1 }), json!({ "$set": [1] })] {
            assert_eq!(
                sig(json!({ "s": "sawtooth", "distorttype": bad })),
                Some(("sawtooth".into(), vec!["distorttype".into()])),
                "{bad}"
            );
        }
        for good in [
            json!(3),
            json!("fold"),
            json!(null),
            json!(true),
            json!(-1),
            json!(1.5),
        ] {
            assert_eq!(
                sig(json!({ "s": "sawtooth", "distorttype": good })),
                None,
                "{good}"
            );
        }
        // Rule 2a is for distorttype only: other array values are left to their keys.
        assert_eq!(sig(json!({ "s": "sine", "crush": [3] })), None);
    }

    fn job_with(values: serde_json::Value, cycles: &[f64]) -> EngineJob {
        let n = values.as_array().unwrap().len();
        let haps: Vec<_> = cycles
            .iter()
            .enumerate()
            .map(|(i, c)| json!({ "t": 0, "duration": 1, "cycle": c, "v": i % n, "event": i }))
            .collect();
        let j = json!({
            "schema": 2, "sampleRate": 44100, "frames": 44100, "seed": 0, "cps": 1,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": [{ "suspendSec": 0, "haps": haps }],
            "values": values
        });
        EngineJob::from_json(&j.to_string()).unwrap()
    }

    #[test]
    fn unsupported_signature_rules() {
        let sig = |v: serde_json::Value| signature_of(&map(v), 0.5);
        assert_eq!(sig(json!({ "s": "sine", "note": 60, "gain": 0.5 })), None);
        assert_eq!(sig(json!({})), None, "default triangle");
        assert_eq!(
            sig(json!({ "s": "~", "ir": "x" })),
            None,
            "silence markers are skipped"
        );
        assert_eq!(
            sig(json!({ "s": "bytebeat" })),
            Some(("bytebeat".into(), vec![]))
        );
        assert_eq!(
            sig(json!({ "s": "sine", "stretch": 0.5, "irspeed": 1, "ir": "a" })),
            Some((
                "sine".into(),
                vec!["ir".into(), "irspeed".into(), "stretch".into()]
            ))
        );
        assert_eq!(
            sig(json!({ "bank": "tr909", "s": "bd" })),
            Some(("tr909_bd".into(), vec![]))
        );
        assert_eq!(
            sig(json!({ "bank": "tr909" })),
            Some(("tr909_triangle".into(), vec![]))
        );
        assert_eq!(
            sig(json!({ "s": "sine", "ftype": "ladder" })),
            None,
            "the ladder filter is implemented"
        );
        assert_eq!(sig(json!({ "s": "sine", "djf": 0.2, "drive": 3 })), None);
        assert_eq!(
            sig(json!({ "s": "sine", "ftype": 1 })),
            None,
            "numeric 1 is a plain biquad"
        );
        assert_eq!(sig(json!({ "s": "sine", "ftype": "24db" })), None);
        assert_eq!(
            sig(json!({ "s": "sine", "note": { "$unsupported": "function" } })),
            Some(("sine".into(), vec!["note".into()]))
        );
        assert_eq!(
            sig(json!({ "s": "sine", "partials": [1, { "$unsupported": "Foo" }] })),
            Some(("sine".into(), vec!["partials".into()]))
        );
        assert_eq!(sig(json!({ "s": 3 })), Some(("3".into(), vec![])));
        assert_eq!(
            sig(json!({ "s": "sine", "size": 2, "begin": 0.5, "cps": 1 })),
            None,
            "ignored keys"
        );
        // A `Set` is not a reason to refuse by itself; a marker inside one is.
        assert_eq!(sig(json!({ "s": "sine", "note": { "$set": [60] } })), None);
        assert_eq!(
            sig(
                json!({ "s": "sine", "note": [{ "$set": [] }], "gain": { "$set": [{ "$set": [] }] } })
            ),
            None
        );
        assert_eq!(
            sig(json!({ "s": "sine", "note": { "$set": [1, { "$unsupported": "function" }] } })),
            Some(("sine".into(), vec!["note".into()]))
        );
        assert_eq!(
            sig(json!({ "s": { "$set": [] } })),
            Some(("[object Set]".into(), vec![])),
            "a set sound is named as JS names it"
        );
    }

    #[test]
    fn signatures_are_distinct_with_first_bar() {
        let job = job_with(
            json!([{ "s": "sine" }, { "s": "sine", "ir": 1 }, { "s": "bytebeat" }]),
            &[0.0, 2.5, 3.0, 4.0, 5.0, 6.0],
        );
        let sigs = unsupported_signatures(&job);
        assert_eq!(
            sigs,
            vec![
                UnsupportedSignature {
                    s: "sine".into(),
                    keys: vec!["ir".into()],
                    first_bar: 2
                },
                UnsupportedSignature {
                    s: "bytebeat".into(),
                    keys: vec![],
                    first_bar: 3
                },
            ]
        );
        assert_eq!(
            serde_json::to_string(&sigs).unwrap(),
            r#"[{"s":"sine","keys":["ir"],"firstBar":2},{"s":"bytebeat","keys":[],"firstBar":3}]"#,
            "serialized as the TS UnsupportedSignature"
        );
        // A negative cycle floors to a negative bar.
        let neg = unsupported_signatures(&job_with(json!([{ "s": "bytebeat" }]), &[-0.5]));
        assert_eq!(neg[0].first_bar, -1);
        assert!(unsupported_signatures(&job_with(json!([{ "s": "sine" }]), &[0.0])).is_empty());
    }

    #[test]
    fn allowlists_are_sorted_and_unique() {
        let sounds = allowed_sounds();
        for list in [
            ALLOWED_KEYS,
            sounds.as_slice(),
            CHAIN_KEYS,
            CHAIN_UNSUPPORTED_KEYS,
        ] {
            assert!(
                list.windows(2).all(|w| w[0] < w[1]),
                "sorted by byte order and unique"
            );
        }
        for s in SILENCE {
            assert!(sounds.contains(s));
        }
    }

    #[test]
    fn chain_and_modulator_tables() {
        for k in CHAIN_KEYS {
            assert!(ALLOWED_KEYS.contains(k), "chain key {k} is allowed");
            assert!(
                !CHAIN_UNSUPPORTED_KEYS.contains(k),
                "chain key {k} is not refused in a chain"
            );
        }
        for k in CHAIN_UNSUPPORTED_KEYS {
            assert!(!ALLOWED_KEYS.contains(k), "{k} is refused at top level too");
        }
        let types: Vec<&str> = MODULATOR_KEYS.iter().map(|(t, _)| *t).collect();
        assert_eq!(types, ["bmod", "env", "lfo"]);
        for (t, keys) in MODULATOR_KEYS {
            assert!(keys.windows(2).all(|w| w[0] < w[1]), "{t} keys sorted");
        }
        // The chain keys: sorted, every filter family's keys (cutoff, Q, envelope, LFO), the
        // filter maps' shared keys and the modulators; nothing superdough reads only from the hap.
        assert!(
            CHAIN_KEYS.windows(2).all(|w| w[0] < w[1]),
            "chain keys sorted"
        );
        let families = [
            (LOWPASS_KEYS, LOWPASS_LFO_KEYS),
            (HIGHPASS_KEYS, HIGHPASS_LFO_KEYS),
            (BANDPASS_KEYS, BANDPASS_LFO_KEYS),
        ];
        for (keys, lfo_keys) in families {
            for k in keys.iter().chain(lfo_keys.iter()) {
                assert!(CHAIN_KEYS.contains(k), "{k} is a chain key");
            }
        }
        for k in [
            "fanchor", "ftype", "drive", "lfo", "env", "bmod", "dry", "room", "delay",
        ] {
            assert!(CHAIN_KEYS.contains(&k), "{k} is a chain key");
        }
        for k in [
            "s",
            "note",
            "orbit",
            "postgain",
            "attack",
            "release",
            "bus",
            "busgain",
            "djf",
            "duckorbit",
            "FX",
            "FXrelease",
        ] {
            assert!(!CHAIN_KEYS.contains(&k), "{k} is read from the hap only");
        }
        let keys = |t: &str| MODULATOR_KEYS.iter().find(|(x, _)| *x == t).unwrap().1;
        assert!(keys("lfo").contains(&"rate") && !keys("lfo").contains(&"attack"));
        assert!(keys("env").contains(&"attack") && !keys("env").contains(&"cps"));
        assert!(keys("bmod").contains(&"bus") && keys("bmod").contains(&"dc"));
    }

    struct AuditRow {
        key: String,
        class: String,
        module: String,
        status: String,
        owner: String,
    }

    /// Whether `owner` is `#<n>`: a `#` then one or more ASCII digits.
    fn is_task_ref(owner: &str) -> bool {
        owner
            .strip_prefix('#')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }

    fn audit_rows() -> Vec<AuditRow> {
        let text = include_str!("../controls.md");
        let mut lines = text.lines().skip_while(|l| !l.starts_with("| key "));
        assert_eq!(
            lines.next(),
            Some("| key | read sites | class | engine module | status | owner |")
        );
        assert_eq!(lines.next(), Some("|---|---|---|---|---|---|"));
        lines
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
                assert_eq!(cells.len(), 6, "{l}");
                AuditRow {
                    key: cells[0].into(),
                    class: cells[2].into(),
                    module: cells[3].into(),
                    status: cells[4].into(),
                    owner: cells[5].into(),
                }
            })
            .collect()
    }

    /// The `## Sounds the engine refuses` table: `(sound, owner)` rows.
    fn refused_sound_rows() -> Vec<(String, String)> {
        let text = include_str!("../controls.md");
        let mut lines = text
            .lines()
            .skip_while(|l| *l != "## Sounds the engine refuses")
            .skip(1)
            .skip_while(|l| l.is_empty());
        assert_eq!(lines.next(), Some("| sound | owner |"));
        assert_eq!(lines.next(), Some("|---|---|"));
        lines
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
                assert_eq!(cells.len(), 2, "{l}");
                (cells[0].to_string(), cells[1].to_string())
            })
            .collect()
    }

    #[test]
    fn refused_sounds_are_not_allowed() {
        let rows = refused_sound_rows();
        let allowed = allowed_sounds();
        assert!(rows.len() >= 6, "{} rows", rows.len());
        for (sound, owner) in &rows {
            assert!(is_task_ref(owner), "{sound}: owner {owner}");
            if sound.contains(' ') {
                continue; // descriptive
            }
            if let Some(prefix) = sound.strip_suffix('*') {
                assert!(
                    !allowed.iter().any(|s| s.starts_with(prefix)),
                    "an allowed sound starts with {prefix}"
                );
            } else {
                assert!(!allowed.contains(&sound.as_str()), "{sound} is allowed");
            }
        }
        for want in ["bytebeat", "sbd", "wt_*", "z_*", "zzfx"] {
            assert!(rows.iter().any(|(s, _)| s == want), "{want} is listed");
        }
        assert!(is_task_ref("#11") && is_task_ref("#0"));
        for bad in ["", "#", "11", "#1a", "# 1", "-"] {
            assert!(!is_task_ref(bad), "{bad:?}");
        }
    }

    /// Every key is implemented, ignored, or unsupported with a later task named as its owner:
    /// no `unsupported` row (and no refused sound) is owned by the effects task, `#11`, and only
    /// the sample and user-DSP keys stay unsupported.
    #[test]
    fn only_sample_and_user_dsp_keys_stay_unsupported() {
        const EFFECTS_TASK: &str = "#11";
        let rows = audit_rows();
        let unsupported: Vec<&AuditRow> =
            rows.iter().filter(|r| r.class == "unsupported").collect();
        assert!(!unsupported.is_empty(), "some keys stay unsupported");
        for r in &unsupported {
            assert!(is_task_ref(&r.owner), "{}: owner {}", r.key, r.owner);
            assert_ne!(
                r.owner, EFFECTS_TASK,
                "{} is still owned by {EFFECTS_TASK}",
                r.key
            );
        }
        for (sound, owner) in refused_sound_rows() {
            assert_ne!(
                owner, EFFECTS_TASK,
                "{sound} is still owned by {EFFECTS_TASK}"
            );
        }
        // The keys left unsupported, each with its owner.
        for (key, owner) in [
            ("ir", "#18"),
            ("irbegin", "#18"),
            ("irspeed", "#18"),
            ("stretch", "#18"),
            ("source", "#55"),
            ("workletInputs", "#55"),
            ("workletSrc", "#55"),
        ] {
            let r = unsupported.iter().find(|r| r.key == key);
            assert_eq!(r.map(|r| r.owner.as_str()), Some(owner), "{key}");
        }
        assert_eq!(
            unsupported.len(),
            7,
            "only the sample and user-DSP keys stay unsupported"
        );
    }

    #[test]
    fn bus_signatures() {
        // The bus sound and its keys are allowed; a bus reader without a cycle is supported.
        assert!(allowed_sounds().contains(&"bus"));
        for k in ["bus", "busgain", "n"] {
            assert!(ALLOWED_KEYS.binary_search(&k).is_ok(), "{k}");
        }
        assert_eq!(
            signature_of(&map(json!({ "s": "bus", "n": 1, "bus": 2 })), 0.5),
            None
        );
        // `bus-cycle` sorts among the other keys of the value.
        assert_eq!(
            signature_with(
                &map(json!({ "s": "bus", "stretch": 1, "ir": "x" })),
                true,
                0.5
            ),
            Some((
                "bus".to_string(),
                vec![
                    "bus-cycle".to_string(),
                    "ir".to_string(),
                    "stretch".to_string()
                ]
            ))
        );
        assert_eq!(
            signature_with(&map(json!({ "s": "bus", "bank": "x" })), true, 0.5),
            Some(("x_bus".to_string(), vec!["bus-cycle".to_string()]))
        );
    }

    #[test]
    fn the_audit_table_matches_the_allowlist() {
        let rows = audit_rows();
        assert!(rows.len() > 400, "every audited key has a row");
        assert!(
            rows.windows(2).all(|w| w[0].key < w[1].key),
            "rows sorted by key, unique"
        );
        let allowed: Vec<&str> = rows
            .iter()
            .filter(|r| r.class == "implemented" || r.class == "ignored")
            .map(|r| r.key.as_str())
            .collect();
        assert_eq!(
            allowed, ALLOWED_KEYS,
            "ALLOWED_KEYS is exactly the implemented and ignored rows"
        );
        for r in &rows {
            match r.class.as_str() {
                "implemented" => {
                    assert_ne!(r.module, "-", "{}", r.key);
                    assert_eq!(r.status, "implemented", "{}", r.key);
                    assert_eq!(r.owner, "-", "{}", r.key);
                }
                "ignored" => {
                    assert_eq!(
                        (r.module.as_str(), r.status.as_str(), r.owner.as_str()),
                        ("-", "-", "-"),
                        "{}",
                        r.key
                    );
                }
                "unsupported" => {
                    assert_eq!(
                        (r.module.as_str(), r.status.as_str()),
                        ("-", "-"),
                        "{}",
                        r.key
                    );
                    assert!(is_task_ref(&r.owner), "{}: owner {}", r.key, r.owner);
                }
                other => panic!("bad class {other} for {}", r.key),
            }
        }
        // The noise mix, crackle density and every control `applyFM` reads are implemented, and
        // no other `fm` key is.
        let mut fm_keys = vec!["noise".to_string(), "density".to_string()];
        for i in 1..=FM_MODULATORS {
            for j in 0..=FM_MODULATORS {
                fm_keys.push(fm_control_name(i, j));
            }
            let sfx = fm_suffix(i);
            for k in [
                "fmh",
                "fmwave",
                "fmattack",
                "fmdecay",
                "fmsustain",
                "fmrelease",
                "fmenv",
            ] {
                fm_keys.push(format!("{k}{sfx}"));
            }
        }
        assert_eq!(fm_keys.len(), 2 + 72 + 56);
        for key in &fm_keys {
            let r = rows.iter().find(|r| &r.key == key).unwrap();
            assert_eq!(r.class, "implemented", "{key}");
        }
        for r in rows
            .iter()
            .filter(|r| r.key.starts_with("fm") && r.class == "implemented")
        {
            assert!(fm_keys.contains(&r.key), "{} is not read by applyFM", r.key);
        }
        // Permanently unsupported in the effects task, with their owners.
        for (key, owner) in [
            ("ir", "#18"),
            ("irspeed", "#18"),
            ("irbegin", "#18"),
            ("stretch", "#18"),
            ("source", "#55"),
            ("workletSrc", "#55"),
            ("workletInputs", "#55"),
        ] {
            let r = rows.iter().find(|r| r.key == key).unwrap();
            assert_eq!(
                (r.class.as_str(), r.owner.as_str()),
                ("unsupported", owner),
                "{key}"
            );
        }
    }

    /// `CHAIN_KEYS` both ways: an allowed key is a chain key exactly when some probe value
    /// changes what an FX entry reads (`parse_entry_params`), on a base that switches every
    /// family on and on the same base without `delaytime`.
    #[test]
    fn chain_keys_are_what_an_entry_reads() {
        let base = json!({
            "cutoff": 800, "hcutoff": 300, "bandf": 900, "lpenv": 1, "hpenv": 1, "bpenv": 1,
            "lprate": 1, "hprate": 1, "bprate": 1, "compressor": -10, "phaserrate": 2,
            "phaserdepth": 0.5, "tremolo": 2, "delay": 0.5, "delaytime": 0.2,
            "delayfeedback": 0.3, "room": 0.4
        });
        let mut no_delaytime = base.clone();
        no_delaytime.as_object_mut().unwrap().remove("delaytime");
        let bases = [base, no_delaytime];
        let probes = [json!(0.37), json!("ladder"), json!(2), json!(null)];
        let read = |k: &str| {
            bases.iter().any(|b| {
                let mut without = b.clone();
                without.as_object_mut().unwrap().remove(k);
                let plain = parse_entry_params(&map(without.clone()), 1.0);
                probes.iter().any(|x| {
                    let mut with = without.clone();
                    with[k] = x.clone();
                    parse_entry_params(&map(with), 1.0) != plain
                })
            })
        };
        for k in ALLOWED_KEYS {
            assert_eq!(
                read(k),
                CHAIN_KEYS.contains(k) && *k != "i",
                "{k}: read by an entry vs listed in CHAIN_KEYS"
            );
        }
        // Superdough reads `i` per entry (superdough.mjs:628); the engine ignores it
        // (`controls.md`: `ignored`).
        assert!(CHAIN_KEYS.contains(&"i") && !read("i"));
    }

    /// One job holding every value's hap at time 0 in one pause, shaped like the equality
    /// guard's extra jobs (44100 Hz, 22050 frames, seed 0, cps 1, duration 0.25).
    fn sound_job(values: serde_json::Value) -> EngineJob {
        let n = values.as_array().unwrap().len();
        let haps: Vec<_> = (0..n)
            .map(|j| json!({ "t": 0, "duration": 0.25, "cycle": 0, "v": j, "event": j }))
            .collect();
        let j = json!({
            "schema": 2, "sampleRate": 44100, "frames": 22050, "seed": 0, "cps": 1,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": [{ "suspendSec": 0, "haps": haps }],
            "values": values
        });
        EngineJob::from_json(&j.to_string()).unwrap()
    }

    /// (errors, every sample finite, peak) of a job's render.
    fn sound_render(values: serde_json::Value) -> (Vec<crate::RenderError>, bool, f32) {
        let out = crate::render(&sound_job(values)).unwrap();
        let samples = || out.channels.iter().flatten();
        let finite = samples().all(|x| x.is_finite());
        let peak = samples().fold(0.0f32, |m, x| m.max(x.abs()));
        (out.errors, finite, peak)
    }

    #[test]
    fn every_allowed_sound_renders() {
        for s in allowed_sounds() {
            if SILENCE.contains(&s) {
                let (errors, finite, peak) = sound_render(json!([{ "s": s }]));
                assert!(errors.is_empty(), "{s}: {errors:?}");
                assert!(finite && peak == 0.0, "{s} renders all zeros, peak {peak}");
                continue;
            }
            let values = if s == "bus" {
                // `bus` plays what a writer sends it; `bus` alone is silent by design.
                json!([{ "s": "sine", "bus": 1, "dry": 0 }, { "s": "bus", "n": 1 }])
            } else {
                // `crackle` is sparse: a hit has probability density * 0.01 = 0.0002 per sample
                // (`DEFAULT_DENSITY`, noise.rs). Seed 0 gives 8 nonzero samples over these frames
                // (seed 5 gives none), so the job's seed is part of this test.
                json!([{ "s": s }])
            };
            let (errors, finite, peak) = sound_render(values);
            assert!(errors.is_empty(), "{s}: {errors:?}");
            assert!(finite, "{s}: every sample finite");
            assert!(peak > 0.0, "{s} sounds");
        }
        // The writer alone is silent (`dry` 0), so the pair's peak comes through the bus.
        let (errors, finite, peak) = sound_render(json!([{ "s": "sine", "bus": 1, "dry": 0 }]));
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            finite && peak == 0.0,
            "the bus writer alone is silent, peak {peak}"
        );
    }

    #[test]
    fn a_silence_string_in_an_array_stays_accepted() {
        for v in [
            json!({ "s": ["-"] }),
            json!({ "s": ["~"] }),
            json!({ "s": ["_"] }),
            json!({ "s": ["sine"] }),
        ] {
            assert_eq!(signature_of(&map(v.clone()), 0.5), None, "{v}");
        }
    }

    /// A job with one pause per value: hap `j` plays value `j` at cycle `j + 0.5` (bar `j`).
    /// `tests/bin.rs` builds the same job for the binary.
    fn table_job(values: &serde_json::Value) -> EngineJob {
        let n = values.as_array().unwrap().len();
        let pauses: Vec<_> = (0..n)
            .map(|j| {
                let c = j as f64 + 0.5;
                json!({
                    "suspendSec": if j == 0 { 0.0 } else { j as f64 - 0.05 },
                    "haps": [{ "t": c, "duration": 0.25, "cycle": c, "v": j, "event": j }]
                })
            })
            .collect();
        let j = json!({
            "schema": 2, "sampleRate": 44100, "frames": 44100 * n.max(1), "seed": 0, "cps": 1,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": pauses,
            "values": values
        });
        EngineJob::from_json(&j.to_string()).unwrap()
    }

    #[test]
    fn shared_signature_table() {
        let text = include_str!("../tests/data/unsupported-signatures.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        assert!(cases.len() >= 30, "{} cases", cases.len());
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let got =
                serde_json::to_value(unsupported_signatures(&table_job(&case["values"]))).unwrap();
            assert_eq!(got, case["expected"], "case {name}");
        }
    }

    #[test]
    fn controls_json_shape() {
        let text = controls_json();
        assert!(!text.contains('\n'));
        assert!(text.starts_with("{\"schema\":2,\"keys\":["));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schema"], 2);
        assert_eq!(v["keys"].as_array().unwrap().len(), ALLOWED_KEYS.len());
        assert_eq!(
            v["sounds"],
            json!([
                "-", "_", "brown", "bus", "crackle", "one", "pink", "pulse", "saw", "sawtooth",
                "sin", "sine", "sqr", "square", "supersaw", "tri", "triangle", "user", "white",
                "~",
            ])
        );
        assert_eq!(v["unsupportedValues"], json!({}));
        assert_eq!(v["chainKeys"], json!(CHAIN_KEYS));
        for (t, keys) in MODULATOR_KEYS {
            assert_eq!(v["modulatorKeys"][t], json!(keys), "{t}");
        }
        assert_eq!(
            v["modulatorTargets"],
            json!(crate::modulate::tests::TODAY.as_slice())
        );
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 7);
        let order = [
            "schema",
            "keys",
            "sounds",
            "unsupportedValues",
            "chainKeys",
            "modulatorKeys",
            "modulatorTargets",
        ];
        let pos = |k: &str| text.find(&format!("\"{k}\":")).unwrap();
        assert!(
            order.windows(2).all(|w| pos(w[0]) < pos(w[1])),
            "fields in order: {text}"
        );
        assert!(
            text.find("\"bmod\":").unwrap() < text.find("\"env\":").unwrap()
                && text.find("\"env\":").unwrap() < text.find("\"lfo\":").unwrap()
        );
    }
}

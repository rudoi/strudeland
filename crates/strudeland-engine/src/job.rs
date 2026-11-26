//! The engine job (`EngineJob`, schema 2): what the render page plans for one chunk (TS
//! `src/lib/render/engine-job.ts`), its decoded control values, validation and the render errors.
//! Validation mirrors `validateEngineJob` rule for rule; shape and type violations are serde
//! errors, the rest are collected by [`EngineJob::validate`].

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::render::frame_for_pause;

/// The only job schema this engine reads.
pub const ENGINE_JOB_SCHEMA: u32 = 2;
/// Largest `frames` a job may ask for (2^31).
pub const MAX_FRAMES: u64 = 1 << 31;
/// Largest `channels.count`: a WAV's 16-bit channel field, and a bound on what the render
/// allocates per channel.
pub const MAX_CHANNELS: u32 = 65535;
/// Cap on `RenderError::message` (the same cap as the harness's `classifyError`).
pub const MAX_MESSAGE: usize = 300;

/// A job: one chunk's pauses, haps and interned control values.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineJob {
    pub schema: u32,
    pub sample_rate: u32,
    pub frames: u64,
    pub seed: i32,
    pub cps: f64,
    pub channels: ChannelSpec,
    /// Orbit creation order.
    pub orbits: Vec<OrbitSpec>,
    pub pauses: Vec<Pause>,
    /// Interned control maps (`_` keys removed by the planner).
    pub values: Vec<ValueMap>,
    /// Where a reference job came from; read and ignored.
    #[serde(default)]
    pub provenance: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelSpec {
    pub count: u32,
    pub multi_channel_orbits: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrbitSpec {
    /// The numeric orbit (`orbit_number` of the haps routed to it).
    pub orbit: f64,
    /// 0-based output channels.
    pub channels: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pause {
    /// The context time the bar-by-bar loop suspends at before scheduling `haps` (0 first).
    pub suspend_sec: f64,
    /// In superdough call order.
    pub haps: Vec<HapRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HapRef {
    /// Seconds from frame 0 of the chunk (before the late clamp).
    pub t: f64,
    /// Hap duration in seconds.
    pub duration: f64,
    /// Absolute layout cycle of the onset.
    pub cycle: f64,
    /// Local measure rate. Older jobs use the job-wide CPS.
    #[serde(default, deserialize_with = "optional_number")]
    pub cps: Option<f64>,
    /// Elapsed layout seconds for fixed-Hz modulation at this onset.
    #[serde(default, deserialize_with = "optional_number")]
    pub phase_sec: Option<f64>,
    /// Index into `EngineJob::values`.
    pub v: u32,
    /// Index of the hap's event on the TS side; read and ignored.
    pub event: u32,
}

fn optional_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    f64::deserialize(deserializer).map(Some)
}

/// A control map with sorted keys.
pub type ValueMap = BTreeMap<String, Value>;

/// A [`ValueMap`] from a JSON object (tests only).
#[cfg(test)]
pub(crate) fn value_map(v: serde_json::Value) -> ValueMap {
    serde_json::from_value(v).unwrap()
}

/// A decoded control value. `{"$num": ...}` wrappers are decoded to `Num`, `{"$unsupported":
/// type}` markers to `Unsupported`, `{"$set": [...]}` wrappers (a JS `Set`, members in insertion
/// order) to `Set`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
    Unsupported(String),
    Set(Vec<Value>),
}

impl Value {
    /// Decodes a JSON value, resolving the `$num`, `$unsupported` and `$set` wrappers at any
    /// depth (checked in that order, as `wrapperProblem` does in TS).
    pub fn from_json(v: serde_json::Value) -> Result<Value, String> {
        use serde_json::Value as J;
        Ok(match v {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(b),
            J::Number(n) => Value::Num(n.as_f64().ok_or_else(|| format!("bad number {n}"))?),
            J::String(s) => Value::Str(s),
            J::Array(xs) => Value::Arr(
                xs.into_iter()
                    .map(Value::from_json)
                    .collect::<Result<_, _>>()?,
            ),
            J::Object(m) => {
                if let Some(tag) = m.get("$num") {
                    if m.len() != 1 {
                        return Err("$num with other keys".into());
                    }
                    return match tag.as_str() {
                        Some("NaN") => Ok(Value::Num(f64::NAN)),
                        Some("Infinity") => Ok(Value::Num(f64::INFINITY)),
                        Some("-Infinity") => Ok(Value::Num(f64::NEG_INFINITY)),
                        Some("-0") => Ok(Value::Num(-0.0)),
                        _ => Err(format!("bad $num {tag}")),
                    };
                }
                if let Some(tag) = m.get("$unsupported") {
                    if m.len() != 1 {
                        return Err("$unsupported with other keys".into());
                    }
                    return match tag.as_str() {
                        Some(t) => Ok(Value::Unsupported(t.to_string())),
                        None => Err("bad $unsupported".into()),
                    };
                }
                if let Some(members) = m.get("$set") {
                    if m.len() != 1 {
                        return Err("$set with other keys".into());
                    }
                    let Some(members) = members.as_array() else {
                        return Err("bad $set".into());
                    };
                    return Ok(Value::Set(
                        members
                            .iter()
                            .cloned()
                            .map(Value::from_json)
                            .collect::<Result<_, _>>()?,
                    ));
                }
                let mut out = BTreeMap::new();
                for (k, x) in m {
                    out.insert(k, Value::from_json(x)?);
                }
                Value::Obj(out)
            }
        })
    }

    /// Whether this value is or contains an `Unsupported` marker.
    pub fn contains_unsupported(&self) -> bool {
        match self {
            Value::Unsupported(_) => true,
            Value::Arr(xs) | Value::Set(xs) => xs.iter().any(Value::contains_unsupported),
            Value::Obj(m) => m.values().any(Value::contains_unsupported),
            _ => false,
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let j = serde_json::Value::deserialize(d)?;
        Value::from_json(j).map_err(|e| de::Error::custom(format!("invalid value wrapper: {e}")))
    }
}

/// The orbit a control map routes to: `Number(value.orbit ?? 1)`, 1 when not finite (the same
/// rule as `orbitNumber` in TS).
pub fn orbit_number(map: &ValueMap) -> f64 {
    let n = match map.get("orbit") {
        None | Some(Value::Null) => 1.0,
        Some(v) => crate::coerce::js_to_number(v),
    };
    if n.is_finite() { n } else { 1.0 }
}

/// What a render error is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    Hap,
    Ir,
    Unsupported,
    Other,
}

/// An error found while rendering a valid job; the render still produces audio.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderError {
    pub kind: ErrorKind,
    pub message: String,
    /// `floor(cycle)` of the hap the error belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar: Option<i64>,
}

impl RenderError {
    /// A render error with its message capped at `MAX_MESSAGE` characters.
    pub fn new(kind: ErrorKind, message: impl Into<String>, bar: Option<i64>) -> Self {
        let message: String = message.into();
        let message = if message.chars().count() > MAX_MESSAGE {
            message.chars().take(MAX_MESSAGE).collect()
        } else {
            message
        };
        RenderError { kind, message, bar }
    }
}

/// `floor(cycle)` as the bar of a hap (saturating for huge cycles).
pub fn bar_of(cycle: f64) -> i64 {
    cycle.floor() as i64
}

/// A job that cannot be rendered (binary exit 2). The message lists every violation, one per line.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct JobError {
    pub message: String,
}

impl JobError {
    pub fn new(message: impl Into<String>) -> Self {
        JobError {
            message: message.into(),
        }
    }
}

impl EngineJob {
    /// Parses and validates a job from JSON text.
    pub fn from_json(text: &str) -> Result<EngineJob, JobError> {
        let job: EngineJob =
            serde_json::from_str(text).map_err(|e| JobError::new(format!("job: {e}")))?;
        job.validate()?;
        Ok(job)
    }

    /// Checks every rule serde cannot express, collecting all violations (`validateEngineJob`).
    pub fn validate(&self) -> Result<(), JobError> {
        let mut errors: Vec<String> = Vec::new();
        if self.schema == 1 {
            errors.push("schema 1 job: regenerate it".into());
        } else if self.schema != ENGINE_JOB_SCHEMA {
            errors.push(format!("schema: {}, expected 2", self.schema));
        }
        if self.sample_rate != crate::SAMPLE_RATE {
            errors.push(format!("sampleRate: {}, expected 44100", self.sample_rate));
        }
        let frames_ok = self.frames <= MAX_FRAMES;
        if !frames_ok {
            errors.push(format!(
                "frames: {}, expected an integer in [0, 2^31]",
                self.frames
            ));
        }
        if !(self.cps.is_finite() && self.cps > 0.0) {
            errors.push(format!("cps: {}, expected > 0", self.cps));
        }
        if !(1..=MAX_CHANNELS).contains(&self.channels.count) {
            errors.push(format!(
                "channels.count: {}, expected an integer in [1, 65535]",
                self.channels.count
            ));
        }
        let n_values = self.values.len();
        if self.pauses.is_empty() {
            errors.push("pauses: empty".into());
        }
        let mut prev_sec = f64::NEG_INFINITY;
        let mut prev_frame: Option<u64> = None;
        for (j, p) in self.pauses.iter().enumerate() {
            let at = format!("pauses[{j}]");
            let sec = p.suspend_sec;
            if !sec.is_finite() {
                errors.push(format!("{at}.suspendSec: {sec}, not finite"));
            } else {
                if j == 0 && sec != 0.0 {
                    errors.push(format!("{at}.suspendSec: {sec}, expected 0"));
                }
                let frame = frame_for_pause(sec);
                if j > 0 && sec <= prev_sec {
                    errors.push(format!("{at}.suspendSec: {sec}, not after {prev_sec}"));
                } else if let Some(pf) = prev_frame.filter(|pf| j > 0 && frame <= *pf) {
                    errors.push(format!(
                        "{at}.suspendSec: frame {frame}, not after frame {pf}"
                    ));
                }
                if j > 0 && frames_ok && frame >= self.frames {
                    errors.push(format!(
                        "{at}.suspendSec: frame {frame}, not before frames {}",
                        self.frames
                    ));
                }
                prev_sec = sec;
                prev_frame = Some(frame);
            }
            for (k, h) in p.haps.iter().enumerate() {
                let at = format!("{at}.haps[{k}]");
                if !(h.t.is_finite() && h.t >= 0.0) {
                    errors.push(format!("{at}.t: {}, expected >= 0", h.t));
                }
                if !(h.duration.is_finite() && h.duration >= 0.0) {
                    errors.push(format!("{at}.duration: {}, expected >= 0", h.duration));
                }
                if !h.cycle.is_finite() {
                    errors.push(format!("{at}.cycle: {}, not finite", h.cycle));
                }
                if let Some(cps) = h.cps
                    && !(cps.is_finite() && cps > 0.0)
                {
                    errors.push(format!("{at}.cps: {cps}, expected > 0"));
                }
                if let Some(sec) = h.phase_sec
                    && !(sec.is_finite() && sec >= 0.0)
                {
                    errors.push(format!("{at}.phaseSec: {sec}, expected >= 0"));
                }
                if (h.v as usize) >= n_values {
                    errors.push(format!(
                        "{at}.v: {}, not an index into values ({n_values})",
                        h.v
                    ));
                }
            }
        }
        for (i, o) in self.orbits.iter().enumerate() {
            if !o.orbit.is_finite() {
                errors.push(format!("orbits[{i}].orbit: {}, not finite", o.orbit));
            }
            if o.channels.is_empty() {
                errors.push(format!("orbits[{i}].channels: expected a non-empty array"));
            } else if o.channels.iter().any(|&c| c < 0) {
                errors.push(format!("orbits[{i}].channels: expected integers >= 0"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(JobError::new(errors.join("\n")))
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorKind::Hap => "hap",
            ErrorKind::Ir => "ir",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Other => "other",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn minimal() -> serde_json::Value {
        json!({
            "schema": 2, "sampleRate": 44100, "frames": 176400, "seed": 0, "cps": 0.5,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": [
                { "suspendSec": 0, "haps": [{ "t": 0, "duration": 1, "cycle": 0, "v": 0, "event": 0 }] },
                { "suspendSec": 1.95, "haps": [{ "t": 2, "duration": 1, "cycle": 1, "v": 0, "event": 1 }] }
            ],
            "values": [{ "s": "sine", "note": 60 }]
        })
    }

    fn parse(v: serde_json::Value) -> Result<EngineJob, JobError> {
        EngineJob::from_json(&v.to_string())
    }

    fn err(v: serde_json::Value) -> String {
        parse(v).expect_err("job should be rejected").message
    }

    #[test]
    fn orbit_specs_serialize_as_they_parse() {
        let spec = OrbitSpec {
            orbit: 2.5,
            channels: vec![2, 3],
        };
        let text = serde_json::to_string(&spec).unwrap();
        assert_eq!(text, r#"{"orbit":2.5,"channels":[2,3]}"#);
        assert_eq!(serde_json::from_str::<OrbitSpec>(&text).unwrap(), spec);
    }

    #[test]
    fn round_trip_of_a_minimal_job() {
        let job = parse(minimal()).unwrap();
        assert_eq!(job.frames, 176400);
        assert_eq!(job.channels.count, 2);
        assert!(!job.channels.multi_channel_orbits);
        assert_eq!(job.orbits[0].channels, vec![0, 1]);
        assert_eq!(job.pauses.len(), 2);
        assert_eq!(job.pauses[1].haps[0].t, 2.0);
        assert_eq!(job.values[0]["s"], Value::Str("sine".into()));
        assert_eq!(job.values[0]["note"], Value::Num(60.0));
        assert!(job.provenance.is_none());
    }

    #[test]
    fn provenance_is_accepted_and_ignored() {
        let mut j = minimal();
        j["provenance"] = json!({ "schema": 2, "platform": "darwin-arm64" });
        assert!(parse(j).unwrap().provenance.is_some());
    }

    /// The raw text of each top-level member of a JSON array or object (`[`/`{` at `text[0]`),
    /// so entries that `serde_json` cannot parse as a whole (`1e999`) can be passed on verbatim.
    fn raw_members(text: &str) -> Vec<&str> {
        let b = text.trim().as_bytes();
        let text = text.trim();
        let (mut depth, mut in_str, mut esc, mut start) = (0, false, false, 1);
        let mut out = Vec::new();
        for (i, &c) in b.iter().enumerate() {
            if in_str {
                match (esc, c) {
                    (true, _) => esc = false,
                    (false, b'\\') => esc = true,
                    (false, b'"') => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                b'"' => in_str = true,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 && !text[start..i].trim().is_empty() {
                        out.push(text[start..i].trim());
                    }
                }
                b',' if depth == 1 => {
                    out.push(text[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        out
    }

    /// `(key, raw value)` of an object member's text.
    fn member(raw: &str) -> (String, &str) {
        let (k, v) = raw.split_once(':').unwrap();
        (serde_json::from_str(k.trim()).unwrap(), v.trim())
    }

    #[test]
    fn rejects_every_shared_invalid_job() {
        let text = include_str!("../../../tests/helpers/invalid-jobs.json");
        let cases = raw_members(text);
        assert!(
            cases.len() >= 36,
            "the shared corpus has {} entries",
            cases.len()
        );
        for case in cases {
            let fields: BTreeMap<String, &str> =
                raw_members(case).into_iter().map(member).collect();
            let res = EngineJob::from_json(fields["job"]);
            assert!(
                res.is_err(),
                "accepted invalid job {} (rule {})",
                fields["name"],
                fields["rule"]
            );
        }
    }

    #[test]
    fn the_unparsed_1e999_cycle_is_a_serde_error() {
        let text = minimal()
            .to_string()
            .replace("\"cycle\":1", "\"cycle\":1e999");
        assert!(
            EngineJob::from_json(&text)
                .unwrap_err()
                .message
                .starts_with("job:")
        );
    }

    #[test]
    fn numbers_parse_exactly() {
        // JS writes the shortest round-trip decimal; `voiceSeed` hashes `String(cycle)` and the
        // value's numbers, so the job must decode them to the same bits (serde_json's
        // `float_roundtrip`; its default parser can be 1 ulp off on long decimals).
        let texts = [
            "0.3333333333333333",
            "0.30000000000000004",
            "2.2250738585072011e-308",
            "123456.87500000001",
            "9007199254740993",
            "0.1",
            "1.7976931348623157e308",
            "5e-324",
            "2013535394798964.2",
            "0.6931471805599453",
            "4.940656458412465e-324",
            "1.0000000000000002",
        ];
        for t in texts {
            let text = minimal()
                .to_string()
                .replace("\"cycle\":1", &format!("\"cycle\":{t}"))
                .replace("\"values\":[{", &format!("\"values\":[{{\"x\":[{t}],"));
            let j = EngineJob::from_json(&text).unwrap_or_else(|e| panic!("{t}: {e:?}"));
            let want: f64 = t.parse().unwrap();
            let cycle = j.pauses.iter().flat_map(|p| &p.haps).last().unwrap().cycle;
            assert_eq!(cycle.to_bits(), want.to_bits(), "cycle {t}");
            let Some(Value::Arr(xs)) = j.values[0].get("x") else {
                panic!("{t}: x missing")
            };
            assert_eq!(xs[0], Value::Num(want), "value {t}");
        }
    }

    #[test]
    fn num_wrappers_decode() {
        let mut j = minimal();
        j["values"] = json!([{
            "a": { "$num": "NaN" }, "b": { "$num": "Infinity" }, "c": { "$num": "-Infinity" },
            "d": { "$num": "-0" }, "e": [1, { "$num": "NaN" }], "f": { "g": { "$num": "-0" } },
            "u": { "$unsupported": "function" }
        }]);
        let job = parse(j).unwrap();
        let v = &job.values[0];
        assert!(matches!(v["a"], Value::Num(x) if x.is_nan()));
        assert_eq!(v["b"], Value::Num(f64::INFINITY));
        assert_eq!(v["c"], Value::Num(f64::NEG_INFINITY));
        assert!(matches!(v["d"], Value::Num(x) if x == 0.0 && x.is_sign_negative()));
        assert!(matches!(&v["e"], Value::Arr(xs) if matches!(xs[1], Value::Num(x) if x.is_nan())));
        assert!(
            matches!(&v["f"], Value::Obj(m) if matches!(m["g"], Value::Num(x) if x.is_sign_negative()))
        );
        assert_eq!(v["u"], Value::Unsupported("function".into()));
        assert!(v["u"].contains_unsupported());
        assert!(!v["a"].contains_unsupported());
    }

    #[test]
    fn a_control_named_num_is_not_a_wrapper() {
        let mut j = minimal();
        j["values"] = json!([{ "$num": 5 }]);
        assert_eq!(parse(j).unwrap().values[0]["$num"], Value::Num(5.0));
    }

    #[test]
    fn malformed_wrappers_are_job_errors() {
        for bad in [
            json!({ "$num": "nan" }),
            json!({ "$num": 1 }),
            json!({ "$num": "NaN", "x": 1 }),
            json!({ "$unsupported": 3 }),
            json!({ "$unsupported": "f", "x": 1 }),
            json!([{ "$num": "zero" }]),
        ] {
            let mut j = minimal();
            j["values"] = json!([{ "k": bad }]);
            assert!(err(j).contains("invalid value wrapper"));
        }
    }

    #[test]
    fn unknown_schema_and_sample_rate() {
        let mut j = minimal();
        j["schema"] = json!(3);
        j["sampleRate"] = json!(48000);
        let m = err(j);
        assert!(m.contains("schema: 3, expected 2"), "{m}");
        assert!(m.contains("sampleRate: 48000"), "{m}");
        assert_eq!(m.lines().count(), 2, "all violations are listed: {m}");
    }

    #[test]
    fn schema_1_is_rejected() {
        let mut j = minimal();
        j["schema"] = json!(1);
        assert_eq!(err(j), "schema 1 job: regenerate it");
        let mut j = minimal();
        j["schema"] = json!(0);
        assert_eq!(err(j), "schema: 0, expected 2");
    }

    #[test]
    fn set_wrappers_decode() {
        let mut j = minimal();
        j["values"] = json!([{
            "empty": { "$set": [] },
            "ids": { "$set": [2, 0, "a"] },
            "nested": { "$set": [1, { "$set": [{ "$num": "NaN" }] }] },
            "inArr": [{ "$set": [null] }],
            "inObj": { "__ids": { "$set": [0] }, "0": { "control": "gain" } },
            "bad": { "$set": [{ "$unsupported": "function" }] },
            "notAWrapper": { "$set2": [] }
        }]);
        let job = parse(j).unwrap();
        let v = &job.values[0];
        assert_eq!(v["empty"], Value::Set(vec![]));
        assert_eq!(
            v["ids"],
            Value::Set(vec![
                Value::Num(2.0),
                Value::Num(0.0),
                Value::Str("a".into())
            ]),
            "insertion order is kept"
        );
        let Value::Set(xs) = &v["nested"] else {
            panic!("nested: {:?}", v["nested"])
        };
        assert_eq!(xs[0], Value::Num(1.0));
        assert!(matches!(&xs[1], Value::Set(ys) if matches!(ys[0], Value::Num(x) if x.is_nan())));
        assert_eq!(v["inArr"], Value::Arr(vec![Value::Set(vec![Value::Null])]));
        assert!(
            matches!(&v["inObj"], Value::Obj(m) if m["__ids"] == Value::Set(vec![Value::Num(0.0)]))
        );
        assert!(matches!(&v["notAWrapper"], Value::Obj(_)));
        assert!(
            v["bad"].contains_unsupported(),
            "a marker inside a set is found"
        );
        assert!(Value::Arr(vec![v["bad"].clone()]).contains_unsupported());
        for k in ["empty", "ids", "nested", "inArr", "inObj"] {
            assert!(!v[k].contains_unsupported(), "{k}");
        }
    }

    #[test]
    fn malformed_set_wrappers_are_job_errors() {
        for (bad, want) in [
            (json!({ "$set": 1 }), "bad $set"),
            (json!({ "$set": {} }), "bad $set"),
            (json!({ "$set": [], "x": 1 }), "$set with other keys"),
            (json!({ "$set": [{ "$num": "x" }] }), "bad $num"),
            (json!([{ "$set": [{ "$set": 1 }] }]), "bad $set"),
            (json!({ "$num": "NaN", "$set": [] }), "$num with other keys"),
            (
                json!({ "$unsupported": "x", "$set": [] }),
                "$unsupported with other keys",
            ),
        ] {
            let mut j = minimal();
            j["values"] = json!([{ "k": bad }]);
            let m = err(j);
            assert!(m.contains("invalid value wrapper"), "{bad}: {m}");
            assert!(m.contains(want), "{bad}: want {want} in {m}");
        }
        let mut j = minimal();
        j["values"] = json!([{ "k": { "$set": ["bad"] } }]);
        assert_eq!(
            parse(j).unwrap().values[0]["k"],
            Value::Set(vec![Value::Str("bad".into())]),
            "a string member is valid"
        );
    }

    #[test]
    fn validates_mapped_hap_timing() {
        let mut j = minimal();
        let h = &mut j["pauses"][1]["haps"][0];
        h["cps"] = json!(4.0 / 3.0);
        h["phaseSec"] = json!(2.0);
        let parsed = parse(j.clone()).unwrap();
        assert_eq!(parsed.pauses[1].haps[0].cps, Some(4.0 / 3.0));
        assert_eq!(parsed.pauses[1].haps[0].phase_sec, Some(2.0));
        j["pauses"][1]["haps"][0]["cps"] = json!(0);
        assert!(err(j.clone()).contains(".cps: 0"));
        j["pauses"][1]["haps"][0]["cps"] = json!(1);
        j["pauses"][1]["haps"][0]["phaseSec"] = json!(-1);
        assert!(err(j.clone()).contains(".phaseSec: -1"));
        j["pauses"][1]["haps"][0]["phaseSec"] = json!(null);
        assert!(parse(j).is_err());
    }

    #[test]
    fn negative_hap_time_and_duration() {
        let mut j = minimal();
        j["pauses"][0]["haps"][0]["t"] = json!(-0.5);
        j["pauses"][0]["haps"][0]["duration"] = json!(-1);
        let m = err(j);
        assert!(m.contains("pauses[0].haps[0].t: -0.5"), "{m}");
        assert!(m.contains("pauses[0].haps[0].duration: -1"), "{m}");
    }

    #[test]
    fn suspend_rules() {
        let mut j = minimal();
        j["pauses"][1]["suspendSec"] = json!(0);
        assert!(err(j).contains("not after 0"));
        // Distinct seconds that quantize to the same frame.
        let mut j = minimal();
        j["pauses"][0]["haps"] = json!([]);
        j["pauses"][1]["suspendSec"] = json!(0.0001);
        j["pauses"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "suspendSec": 0.0002, "haps": [] }));
        assert!(err(j).contains("frame 128, not after frame 128"));
        // A pause frame at or after `frames`.
        let mut j = minimal();
        j["frames"] = json!(1000);
        j["pauses"][1]["suspendSec"] = json!(0.03);
        assert!(err(j).contains("frame 1408, not before frames 1000"));
        let mut j = minimal();
        j["pauses"][0]["suspendSec"] = json!(0.5);
        assert!(err(j).contains("pauses[0].suspendSec: 0.5, expected 0"));
        let mut j = minimal();
        j["pauses"] = json!([]);
        assert!(err(j).contains("pauses: empty"));
    }

    #[test]
    fn frames_cps_channels_orbits_values() {
        let mut j = minimal();
        j["frames"] = json!(MAX_FRAMES + 1);
        j["cps"] = json!(0);
        j["channels"]["count"] = json!(0);
        j["orbits"] = json!([{ "orbit": 1, "channels": [] }, { "orbit": 2, "channels": [0, -1] }]);
        j["pauses"][0]["haps"][0]["v"] = json!(3);
        let m = err(j);
        for want in [
            "frames: 2147483649",
            "cps: 0, expected > 0",
            "channels.count: 0",
            "orbits[0].channels: expected a non-empty array",
            "orbits[1].channels: expected integers >= 0",
            "pauses[0].haps[0].v: 3, not an index into values (1)",
        ] {
            assert!(m.contains(want), "missing `{want}` in {m}");
        }
        let mut j = minimal();
        j["frames"] = json!(MAX_FRAMES);
        j["pauses"][1]["suspendSec"] = json!(1.95);
        assert!(parse(j).is_ok(), "2^31 frames is allowed");
        let mut j = minimal();
        j["channels"]["count"] = json!(MAX_CHANNELS + 1);
        assert!(err(j).contains("channels.count: 65536, expected an integer in [1, 65535]"));
        let mut j = minimal();
        j["channels"]["count"] = json!(MAX_CHANNELS);
        assert!(parse(j).is_ok(), "65535 channels is allowed");
    }

    #[test]
    fn type_errors_are_serde_errors() {
        for (path, bad) in [
            ("seed", json!(0.5)),
            ("seed", json!(2147483648_i64)),
            ("frames", json!(-1)),
            ("cps", json!("0.5")),
            ("orbits", json!([{ "orbit": "a", "channels": [0] }])),
            ("extra", json!(1)),
        ] {
            let mut j = minimal();
            j[path] = bad;
            assert!(err(j).starts_with("job:"), "{path}");
        }
        assert!(EngineJob::from_json("[1, 2]").is_err());
        assert!(EngineJob::from_json("{").is_err());
    }

    #[test]
    fn orbit_number_rules() {
        let m = |v: serde_json::Value| -> ValueMap {
            serde_json::from_value::<BTreeMap<String, Value>>(v).unwrap()
        };
        assert_eq!(orbit_number(&m(json!({}))), 1.0);
        assert_eq!(orbit_number(&m(json!({ "orbit": null }))), 1.0);
        assert_eq!(orbit_number(&m(json!({ "orbit": 3 }))), 3.0);
        assert_eq!(orbit_number(&m(json!({ "orbit": "2" }))), 2.0);
        assert_eq!(orbit_number(&m(json!({ "orbit": "x" }))), 1.0);
        assert_eq!(
            orbit_number(&m(json!({ "orbit": { "$num": "Infinity" } }))),
            1.0
        );
        assert_eq!(orbit_number(&m(json!({ "orbit": true }))), 1.0);
    }

    #[test]
    fn render_error_serialization_and_cap() {
        let e = RenderError::new(ErrorKind::Hap, "x".repeat(400), Some(3));
        assert_eq!(e.message.chars().count(), MAX_MESSAGE);
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["kind"], "hap");
        assert_eq!(j["bar"], 3);
        let e = RenderError::new(ErrorKind::Unsupported, "m", None);
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["kind"], "unsupported");
        assert!(j.get("bar").is_none());
        assert_eq!(ErrorKind::Ir.to_string(), "ir");
        assert_eq!(ErrorKind::Other.to_string(), "other");
        assert_eq!(bar_of(-0.5), -1);
        assert_eq!(bar_of(2.999), 2);
    }
}

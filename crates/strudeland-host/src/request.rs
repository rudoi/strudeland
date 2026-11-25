//! The requests the host plans. [`RenderRequest`] (one job) is a serde mirror of `RenderRequest`
//! in `src/lib/render/contract.ts` without `code` (passed separately), `samples` and `mutation`
//! (the planner does not read them; an unknown field is an error). [`RenderPlanRequest`] (every
//! chunk of a render) mirrors `RenderPlanInput` in `src/lib/render/contract.ts`, plus the
//! host-only `timeoutSec`; the bundle validates its length request.

use std::{fmt, time::Duration};

use serde::{Deserialize, Deserializer, Serialize, de};
use strudeland_project::{InstrumentSnapshot, validate_instrument_snapshot};

/// The largest `toBar + overlapBars` a request may ask for.
pub const MAX_BARS: u64 = 100_000;

/// The only sample rate the planner and the engine use.
pub const SAMPLE_RATE: u32 = 44_100;

/// Segment layout: the song's arrangement (`clip`) or one linear timeline (`linear`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Clip,
    Linear,
}

impl Layout {
    /// The name the bundle takes (`'clip' | 'linear'`).
    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Clip => "clip",
            Layout::Linear => "linear",
        }
    }
}

impl std::str::FromStr for Layout {
    type Err = String;

    /// The name [`as_str`](Layout::as_str) gives.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [Layout::Clip, Layout::Linear]
            .into_iter()
            .find(|l| l.as_str() == s)
            .ok_or_else(|| format!("unknown layout {s:?} (clip or linear)"))
    }
}

/// Render mode: one stereo mix, or a stereo pair per stem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Mix,
    Stems,
}

/// Seconds planned before `fromBar`: `"auto"` (computed from the schedule) or a number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "LookbackRepr", into = "LookbackRepr")]
pub enum LookbackSec {
    Auto,
    Seconds(f64),
}

/// The wire form of [`LookbackSec`]: the word `"auto"` or a number.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(untagged)]
enum LookbackRepr {
    Word(AutoWord),
    Seconds(f64),
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AutoWord {
    Auto,
}

impl From<LookbackRepr> for LookbackSec {
    fn from(r: LookbackRepr) -> Self {
        match r {
            LookbackRepr::Word(AutoWord::Auto) => LookbackSec::Auto,
            LookbackRepr::Seconds(x) => LookbackSec::Seconds(x),
        }
    }
}

impl From<LookbackSec> for LookbackRepr {
    fn from(l: LookbackSec) -> Self {
        match l {
            LookbackSec::Auto => LookbackRepr::Word(AutoWord::Auto),
            LookbackSec::Seconds(x) => LookbackRepr::Seconds(x),
        }
    }
}

/// One chunk's planning request (bars are layout bars, the window is `[from_bar, to_bar)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderRequest {
    pub layout: Layout,
    pub from_bar: u32,
    pub to_bar: u32,
    pub overlap_bars: u32,
    pub lookback_sec: LookbackSec,
    pub mode: Mode,
    pub sample_rate: u32,
}

impl RenderRequest {
    /// Checks what the types do not: `from_bar < to_bar`, `to_bar + overlap_bars <=`
    /// [`MAX_BARS`], a finite non-negative lookback and the sample rate.
    pub fn validate(&self) -> Result<(), String> {
        if self.from_bar >= self.to_bar {
            return Err(format!(
                "fromBar ({}) must be less than toBar ({})",
                self.from_bar, self.to_bar
            ));
        }
        let end = u64::from(self.to_bar) + u64::from(self.overlap_bars);
        if end > MAX_BARS {
            return Err(format!(
                "toBar + overlapBars ({end}) is more than {MAX_BARS}"
            ));
        }
        if let LookbackSec::Seconds(x) = self.lookback_sec
            && !(x.is_finite() && x >= 0.0)
        {
            return Err(format!(
                "lookbackSec ({x}) must be \"auto\" or a finite number >= 0"
            ));
        }
        if self.sample_rate != SAMPLE_RATE {
            return Err(format!(
                "sampleRate ({}) must be {SAMPLE_RATE}",
                self.sample_rate
            ));
        }
        Ok(())
    }

    /// The request as the JSON object the bundle's `planJob` takes.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// The longest planning timeout a request may set, in seconds (one day).
pub const MAX_TIMEOUT_SEC: f64 = 86_400.0;

/// What a render covers. `clip`, `section` and `length` are resolved by the bundle against the
/// song's `meta` (a song without sections needs `length`); `window` is an explicit window and
/// layout, for the harness and the dev binary only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LengthRequest {
    /// The song's clip, its loop section played `loops` times (default `meta.loops`).
    Clip {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loops: Option<u32>,
    },
    /// One section, played once, on the linear timeline.
    Section { name: String },
    /// The first `bars` bars.
    Length { bars: u32 },
    /// `[from_bar, to_bar)` in `layout`.
    Window {
        from_bar: u32,
        to_bar: u32,
        layout: Layout,
    },
}

/// Arrangement and decay policy for an offline asset export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportPreset {
    TaggedLoop,
    IntroLoop,
    FoldedLoop,
    OneShot,
}

/// A whole render's planning request: every chunk's engine job in one host call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderPlanRequest {
    /// Project pins captured for this evaluation.
    #[serde(
        default,
        deserialize_with = "instrument_snapshot",
        skip_serializing_if = "InstrumentSnapshot::is_empty"
    )]
    pub instruments: InstrumentSnapshot,
    pub length: LengthRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ExportPreset>,
    /// Also return the schedule's events (the dev binary's `--info`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_events: Option<bool>,
    /// Seconds the planning may take; host-side only, never sent to the bundle.
    #[serde(default, skip_serializing)]
    pub timeout_sec: Option<f64>,
}

fn instrument_snapshot<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<InstrumentSnapshot, D::Error> {
    struct SnapshotVisitor;
    impl<'de> de::Visitor<'de> for SnapshotVisitor {
        type Value = InstrumentSnapshot;

        fn expecting(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt.write_str("an object of instrument definitions")
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut snapshot = InstrumentSnapshot::new();
            while let Some(id) = map.next_key::<String>()? {
                let definition = map.next_value().map_err(|error| {
                    <A::Error as de::Error>::custom(format_args!("instrument {id:?}: {error}"))
                })?;
                snapshot.insert(id, definition);
            }
            Ok(snapshot)
        }
    }
    deserializer.deserialize_map(SnapshotVisitor)
}

impl RenderPlanRequest {
    /// Checks what the types do not: the instrument snapshot and a finite timeout in
    /// `(0, MAX_TIMEOUT_SEC]`. The length request is the bundle's to check (`validateRenderRequest`
    /// in `job-plan.ts`).
    pub fn validate(&self) -> Result<(), String> {
        validate_instrument_snapshot(&self.instruments).map_err(|e| e.to_string())?;
        if let Some(t) = self.timeout_sec
            && timeout_from_secs(t).is_none()
        {
            return Err(format!(
                "timeoutSec ({t}) must be a number of seconds greater than 0 and at most {MAX_TIMEOUT_SEC}"
            ));
        }
        Ok(())
    }

    /// The planning timeout, `None` when the request sets none or an invalid one ([`validate`]
    /// rejects those).
    ///
    /// [`validate`]: RenderPlanRequest::validate
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout_sec.and_then(timeout_from_secs)
    }

    /// The request as the JSON object the bundle's `planRender` takes (without `timeoutSec`).
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// `secs` as a planning timeout when it is finite and in `(0, MAX_TIMEOUT_SEC]`, else `None`.
pub fn timeout_from_secs(secs: f64) -> Option<Duration> {
    if !(secs.is_finite() && secs > 0.0 && secs <= MAX_TIMEOUT_SEC) {
        return None;
    }
    Duration::try_from_secs_f64(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_round_trip() {
        for l in [Layout::Clip, Layout::Linear] {
            assert_eq!(l.as_str().parse::<Layout>(), Ok(l));
        }
        assert!("Clip".parse::<Layout>().is_err());
        assert!("song".parse::<Layout>().is_err());
    }

    fn req() -> RenderRequest {
        RenderRequest {
            layout: Layout::Linear,
            from_bar: 0,
            to_bar: 4,
            overlap_bars: 0,
            lookback_sec: LookbackSec::Seconds(0.0),
            mode: Mode::Mix,
            sample_rate: 44_100,
        }
    }

    #[test]
    fn a_valid_request_parses_and_validates() {
        let r: RenderRequest = serde_json::from_str(
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
        )
        .unwrap();
        assert_eq!(r, req());
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn auto_and_numeric_lookback_round_trip() {
        for (text, value) in [
            (r#""auto""#, LookbackSec::Auto),
            ("2.5", LookbackSec::Seconds(2.5)),
            ("0", LookbackSec::Seconds(0.0)),
        ] {
            let parsed: LookbackSec = serde_json::from_str(text).unwrap();
            assert_eq!(parsed, value, "{text}");
            let back = serde_json::to_string(&parsed).unwrap();
            assert_eq!(
                serde_json::from_str::<LookbackSec>(&back).unwrap(),
                value,
                "{text}"
            );
        }
        assert_eq!(
            serde_json::to_string(&LookbackSec::Auto).unwrap(),
            r#""auto""#
        );
        assert!(serde_json::from_str::<LookbackSec>(r#""Auto""#).is_err());
        assert!(serde_json::from_str::<LookbackSec>(r#""2""#).is_err());
        assert!(serde_json::from_str::<LookbackSec>("null").is_err());
    }

    #[test]
    fn the_json_is_the_bundle_shape() {
        let mut r = req();
        r.lookback_sec = LookbackSec::Auto;
        r.layout = Layout::Clip;
        r.mode = Mode::Stems;
        assert_eq!(
            r.to_json().unwrap(),
            r#"{"layout":"clip","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":"auto","mode":"stems","sampleRate":44100}"#
        );
        assert_eq!(Layout::Clip.as_str(), "clip");
        assert_eq!(Layout::Linear.as_str(), "linear");
    }

    #[test]
    fn from_bar_must_be_before_to_bar() {
        for (from, to) in [(4, 4), (5, 4), (0, 0)] {
            let r = RenderRequest {
                from_bar: from,
                to_bar: to,
                ..req()
            };
            let e = r.validate().unwrap_err();
            assert!(e.contains("fromBar"), "{from}..{to}: {e}");
        }
    }

    #[test]
    fn the_window_end_is_bounded() {
        let at_limit = RenderRequest {
            to_bar: 99_990,
            overlap_bars: 10,
            ..req()
        };
        assert_eq!(at_limit.validate(), Ok(()));
        for (to, overlap) in [(99_990, 11), (100_001, 0), (u32::MAX, u32::MAX)] {
            let r = RenderRequest {
                to_bar: to,
                overlap_bars: overlap,
                ..req()
            };
            let e = r.validate().unwrap_err();
            assert!(e.contains("toBar + overlapBars"), "{to}+{overlap}: {e}");
        }
    }

    #[test]
    fn lookback_must_be_finite_and_non_negative() {
        for x in [f64::NAN, -1.0, -0.000_1, f64::INFINITY, f64::NEG_INFINITY] {
            let r = RenderRequest {
                lookback_sec: LookbackSec::Seconds(x),
                ..req()
            };
            let e = r.validate().unwrap_err();
            assert!(e.contains("lookbackSec"), "{x}: {e}");
        }
        for x in [0.0, -0.0, 1e9] {
            let r = RenderRequest {
                lookback_sec: LookbackSec::Seconds(x),
                ..req()
            };
            assert_eq!(r.validate(), Ok(()), "{x}");
        }
        let auto = RenderRequest {
            lookback_sec: LookbackSec::Auto,
            ..req()
        };
        assert_eq!(auto.validate(), Ok(()));
    }

    #[test]
    fn only_44100_is_accepted() {
        for rate in [0, 48_000, 44_099, u32::MAX] {
            let r = RenderRequest {
                sample_rate: rate,
                ..req()
            };
            let e = r.validate().unwrap_err();
            assert!(e.contains("sampleRate"), "{rate}: {e}");
        }
    }

    #[test]
    fn malformed_json_is_rejected() {
        let bad = [
            // unknown fields: the planner ignores samples and mutation, so carrying them is an error
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100,"samples":{}}"#,
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100,"code":"x"}"#,
            // missing field
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix"}"#,
            // negative, fractional and out-of-range bars
            r#"{"layout":"linear","fromBar":-1,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
            r#"{"layout":"linear","fromBar":0.5,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
            r#"{"layout":"linear","fromBar":0,"toBar":4294967296,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
            // bad enum values
            r#"{"layout":"song","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mix","sampleRate":44100}"#,
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":0,"mode":"mono","sampleRate":44100}"#,
            r#"{"layout":"linear","fromBar":0,"toBar":4,"overlapBars":0,"lookbackSec":"soon","mode":"mix","sampleRate":44100}"#,
            "[]",
            "",
        ];
        for text in bad {
            assert!(
                serde_json::from_str::<RenderRequest>(text).is_err(),
                "accepted: {text}"
            );
        }
    }

    fn plan_req(length: LengthRequest) -> RenderPlanRequest {
        RenderPlanRequest {
            length,
            instruments: Default::default(),
            preset: None,
            include_events: None,
            timeout_sec: None,
        }
    }

    #[test]
    fn a_valid_render_plan_request_parses_and_validates() {
        let r: RenderPlanRequest = serde_json::from_str(
            r#"{"length":{"kind":"clip","loops":2},"includeEvents":true,"timeoutSec":120}"#,
        )
        .unwrap();
        assert_eq!(
            r,
            RenderPlanRequest {
                include_events: Some(true),
                timeout_sec: Some(120.0),
                ..plan_req(LengthRequest::Clip { loops: Some(2) })
            }
        );
        assert_eq!(r.validate(), Ok(()));
        assert_eq!(r.timeout(), Some(Duration::from_secs(120)));
        let cases = [
            (r#"{"kind":"clip"}"#, LengthRequest::Clip { loops: None }),
            (
                r#"{"kind":"section","name":"drop"}"#,
                LengthRequest::Section {
                    name: "drop".into(),
                },
            ),
            (
                r#"{"kind":"length","bars":8}"#,
                LengthRequest::Length { bars: 8 },
            ),
            (
                r#"{"kind":"window","fromBar":2,"toBar":6,"layout":"clip"}"#,
                LengthRequest::Window {
                    from_bar: 2,
                    to_bar: 6,
                    layout: Layout::Clip,
                },
            ),
        ];
        for (text, length) in cases {
            let parsed: LengthRequest = serde_json::from_str(text).unwrap();
            assert_eq!(parsed, length, "{text}");
            // camelCase on the wire, `loops` omitted when absent.
            assert_eq!(serde_json::to_string(&parsed).unwrap(), text, "{text}");
            assert_eq!(plan_req(length).validate(), Ok(()), "{text}");
        }
    }

    #[test]
    fn the_render_plan_json_is_the_bundle_shape_without_the_timeout() {
        let r = RenderPlanRequest {
            timeout_sec: Some(5.0),
            ..plan_req(LengthRequest::Length { bars: 4 })
        };
        assert_eq!(
            r.to_json().unwrap(),
            r#"{"length":{"kind":"length","bars":4}}"#
        );
        let r = RenderPlanRequest {
            include_events: Some(false),
            ..plan_req(LengthRequest::Clip { loops: Some(3) })
        };
        assert_eq!(
            r.to_json().unwrap(),
            r#"{"length":{"kind":"clip","loops":3},"includeEvents":false}"#
        );
    }

    #[test]
    fn render_plan_carries_only_valid_instrument_ids_to_the_bundle() {
        let definition = serde_json::json!({"schema":1,"parameters":{},"voices":[
            {"name":"tone","chain":[{"control":"s","value":"sine"}]}
        ]});
        let mut request = plan_req(LengthRequest::Length { bars: 1 });
        let parsed = serde_json::from_value(definition.clone()).unwrap();
        request.instruments.insert("lead".into(), parsed);
        assert_eq!(request.validate(), Ok(()));
        let mut wire: serde_json::Value =
            serde_json::from_str(&request.to_json().unwrap()).unwrap();
        assert_eq!(wire["instruments"]["lead"], definition);
        wire["instruments"]["bass"] = definition;
        wire["instruments"]["bass"]["voices"][0]["chain"][0]["value"] =
            serde_json::json!({"param":"missing"});
        let error = serde_json::from_str::<RenderPlanRequest>(&wire.to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("instrument \"bass\""), "{error}");
        assert!(error.contains("unknown parameter \"missing\""), "{error}");
        let entry = request.instruments.remove("lead").unwrap();
        request.instruments.insert("../lead".into(), entry);
        assert!(request.validate().unwrap_err().contains("../lead"));
    }

    #[test]
    fn every_render_plan_validation_error() {
        for t in [
            0.0,
            -0.0,
            -1.0,
            86_400.001,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let r = RenderPlanRequest {
                timeout_sec: Some(t),
                ..plan_req(LengthRequest::Clip { loops: None })
            };
            let e = r.validate().unwrap_err();
            assert!(e.starts_with("timeoutSec ("), "{t}: {e}");
            assert_eq!(r.timeout(), None, "{t}");
        }
        // The length request is the bundle's to check: any value passes here.
        for (length, timeout) in [
            (LengthRequest::Clip { loops: Some(0) }, Some(86_400.0)),
            (LengthRequest::Clip { loops: Some(1001) }, Some(0.001)),
            (LengthRequest::Length { bars: 0 }, None),
            (LengthRequest::Length { bars: 100_001 }, None),
            (
                LengthRequest::Window {
                    from_bar: 5,
                    to_bar: 4,
                    layout: Layout::Linear,
                },
                None,
            ),
            (
                LengthRequest::Section {
                    name: String::new(),
                },
                None,
            ),
        ] {
            let r = RenderPlanRequest {
                timeout_sec: timeout,
                ..plan_req(length)
            };
            assert_eq!(r.validate(), Ok(()), "{r:?}");
        }
    }

    #[test]
    fn a_render_plan_request_with_mode_or_sample_rate_is_rejected() {
        for text in [
            r#"{"length":{"kind":"clip"},"mode":"mix"}"#,
            r#"{"length":{"kind":"clip"},"sampleRate":44100}"#,
            r#"{"length":{"kind":"clip"},"mode":"mix","sampleRate":44100}"#,
        ] {
            let e = serde_json::from_str::<RenderPlanRequest>(text)
                .unwrap_err()
                .to_string();
            assert!(e.contains("unknown field"), "{text}: {e}");
        }
    }

    #[test]
    fn the_timeout_converts_exactly() {
        assert_eq!(timeout_from_secs(0.5), Some(Duration::from_millis(500)));
        assert_eq!(
            timeout_from_secs(86_400.0),
            Some(Duration::from_secs(86_400))
        );
        assert_eq!(timeout_from_secs(1e-9), Some(Duration::from_nanos(1)));
        for t in [0.0, -0.5, 86_400.5, f64::NAN, f64::INFINITY, f64::MAX] {
            assert_eq!(timeout_from_secs(t), None, "{t}");
        }
    }

    #[test]
    fn malformed_render_plan_json_is_rejected() {
        let bad = [
            // unknown fields, on the request and inside `length`
            r#"{"length":{"kind":"clip"},"code":"x"}"#,
            r#"{"length":{"kind":"clip","bars":4}}"#,
            r#"{"length":{"kind":"length","bars":4,"loops":2}}"#,
            r#"{"length":{"kind":"section","name":"a","loops":2}}"#,
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"clip","overlapBars":0}}"#,
            // snake_case is not the wire spelling
            r#"{"length":{"kind":"window","from_bar":0,"to_bar":4,"layout":"clip"}}"#,
            r#"{"length":{"kind":"clip"},"timeout_sec":5}"#,
            // unknown or missing kind, missing fields
            r#"{"length":{"kind":"song"}}"#,
            r#"{"length":{"bars":4}}"#,
            r#"{"length":{"kind":"length"}}"#,
            r#"{"length":{"kind":"section"}}"#,
            r#"{}"#,
            // wrong types and ranges
            r#"{"length":{"kind":"length","bars":-1}}"#,
            r#"{"length":{"kind":"length","bars":2.5}}"#,
            r#"{"length":{"kind":"clip","loops":"2"}}"#,
            r#"{"length":{"kind":"window","fromBar":0,"toBar":4,"layout":"song"}}"#,
            r#"{"length":{"kind":"clip"},"includeEvents":1}"#,
            r#"{"length":{"kind":"clip"},"timeoutSec":"5"}"#,
            r#"{"length":"clip"}"#,
            "[]",
            "",
        ];
        for text in bad {
            assert!(
                serde_json::from_str::<RenderPlanRequest>(text).is_err(),
                "accepted: {text}"
            );
        }
    }
}

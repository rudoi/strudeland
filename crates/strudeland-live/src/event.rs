//! Events the player sends to the app (forwarded over a Tauri `Channel`).

use serde::Serialize;

use crate::master::LevelFrame;

/// A `u64` as a decimal string (JS numbers lose precision past 2^53).
pub mod u64_string {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// The engine frame (44.1 kHz) heard at host time `host_ns`.
    Anchor {
        frame: u64,
        #[serde(with = "u64_string")]
        host_ns: u64,
    },
    /// An unsupported value was skipped (once per signature per player).
    Warning {
        signature: String,
        keys: Vec<String>,
        first_cycle: f64,
    },
    Stats(Stats),
    /// Mono, decimated to 1024 samples, about 30 a second.
    Scope {
        samples: Vec<f32>,
    },
    Error {
        message: String,
    },
    /// The master's levels over every sample rendered since the previous frame (before the
    /// limiter), about 30 a second, anchored or not.
    Levels(LevelFrame),
}

/// Microseconds the engine thread spent per batch over the last second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BatchDecodeUs {
    pub p50: u64,
    pub max: u64,
}

/// Cumulative counters, plus the two windowed fields over the last second.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub underruns: u64,
    pub late_haps: u64,
    pub haps: u64,
    pub batches: u64,
    pub skipped_haps: u64,
    pub render_headroom_ms: f64,
    pub batch_decode_us: BatchDecodeUs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn to_json(e: &Event) -> Value {
        serde_json::to_value(e).unwrap()
    }

    #[test]
    fn anchor_has_a_string_host_time() {
        let e = Event::Anchor {
            frame: 4410,
            host_ns: u64::MAX,
        };
        assert_eq!(
            to_json(&e),
            json!({"kind": "anchor", "frame": 4410, "hostNs": "18446744073709551615"})
        );
    }

    #[test]
    fn warning_fields_are_camel_case() {
        let e = Event::Warning {
            signature: "s=z_noise".into(),
            keys: vec!["s".into(), "gain".into()],
            first_cycle: 2.5,
        };
        assert_eq!(
            to_json(&e),
            json!({"kind": "warning", "signature": "s=z_noise", "keys": ["s", "gain"], "firstCycle": 2.5})
        );
    }

    #[test]
    fn stats_flatten_next_to_the_tag() {
        let e = Event::Stats(Stats {
            underruns: 1,
            late_haps: 2,
            haps: 3,
            batches: 4,
            skipped_haps: 5,
            render_headroom_ms: 6.5,
            batch_decode_us: BatchDecodeUs { p50: 7, max: 8 },
        });
        assert_eq!(
            to_json(&e),
            json!({
                "kind": "stats", "underruns": 1, "lateHaps": 2, "haps": 3, "batches": 4,
                "skippedHaps": 5, "renderHeadroomMs": 6.5, "batchDecodeUs": {"p50": 7, "max": 8}
            })
        );
    }

    #[test]
    fn stats_default_is_all_zero() {
        let v = serde_json::to_value(Stats::default()).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 7);
        assert_eq!(v["batchDecodeUs"], json!({"p50": 0, "max": 0}));
        assert_eq!(v["renderHeadroomMs"], json!(0.0));
    }

    #[test]
    fn scope_and_error() {
        assert_eq!(
            to_json(&Event::Scope {
                samples: vec![0.5, -0.25]
            }),
            json!({"kind": "scope", "samples": [0.5, -0.25]})
        );
        assert_eq!(
            to_json(&Event::Error {
                message: "boom".into()
            }),
            json!({"kind": "error", "message": "boom"})
        );
    }

    #[test]
    fn levels_flatten_next_to_the_tag() {
        let e = Event::Levels(LevelFrame {
            peak: [0.5, 1.25],
            rms: [0.25, 0.125],
            overs: [0, 3],
            limit_db: -2.5,
        });
        assert_eq!(
            to_json(&e),
            json!({
                "kind": "levels", "peak": [0.5, 1.25], "rms": [0.25, 0.125], "overs": [0, 3],
                "limitDb": -2.5
            })
        );
        let silent = serde_json::to_string(&Event::Levels(LevelFrame::default())).unwrap();
        assert_eq!(
            silent,
            r#"{"kind":"levels","peak":[0.0,0.0],"rms":[0.0,0.0],"overs":[0,0],"limitDb":0.0}"#
        );
    }
}

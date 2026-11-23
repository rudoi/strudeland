//! The `playback_schedule` payload: values new to the session and the haps that use them.

use serde::{Deserialize, Serialize};
use strudeland_engine::OrbitSpec;

/// One flush of the JS backend: `values` apply before `haps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub values: Vec<ValueEntry>,
    pub haps: Vec<HapEntry>,
    pub cps: f64,
}

/// `[index, encodedControlMap, orbitSpec | null]`; always three elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueEntry(pub u32, pub serde_json::Value, pub Option<OrbitSpec>);

/// Engine seconds, duration seconds, musical cycle, value index and optional elapsed layout seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HapEntry(
    pub f64,
    pub f64,
    pub f64,
    pub u32,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub Option<f64>,
);

impl Batch {
    /// Shape checks only. A hap naming an unknown value index is the engine's error, not this one.
    pub fn validate(&self) -> Result<(), String> {
        if !self.cps.is_finite() || self.cps <= 0.0 {
            return Err(format!("cps must be finite and above 0, got {}", self.cps));
        }
        let mut seen = std::collections::HashSet::new();
        for ValueEntry(index, _, orbit) in &self.values {
            if !seen.insert(*index) {
                return Err(format!("value index {index} appears twice"));
            }
            if let Some(o) = orbit
                && !o.orbit.is_finite()
            {
                return Err(format!("value {index}: orbit is not finite"));
            }
        }
        for (i, HapEntry(t, duration, cycle, _, phase_sec)) in self.haps.iter().enumerate() {
            if !t.is_finite() || !duration.is_finite() || !cycle.is_finite() {
                return Err(format!("hap {i}: t, duration and cycle must be finite"));
            }
            if phase_sec.is_some_and(|sec| !sec.is_finite()) {
                return Err(format!("hap {i}: phase seconds must be finite"));
            }
            if *duration < 0.0 {
                return Err(format!("hap {i}: duration is negative"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
        "values": [
            [0, {"s": "sine", "gain": {"$num": "0.5"}}, {"orbit": 1, "channels": [0, 1]}],
            [1, {"s": "saw"}, null]
        ],
        "haps": [[0.1, 0.25, 0, 0], [0.35, 0.25, 0.5, 1]],
        "cps": 0.5
    }"#;

    fn parse(text: &str) -> Result<Batch, serde_json::Error> {
        serde_json::from_str(text)
    }

    #[test]
    fn elapsed_layout_time_is_optional_on_the_wire() {
        for values in [
            serde_json::json!([0.1, 0.25, 1.0, 0]),
            serde_json::json!([0.1, 0.25, 1.0, 0, 2.0]),
        ] {
            let hap: HapEntry = serde_json::from_value(values.clone()).unwrap();
            assert_eq!(serde_json::to_value(hap).unwrap(), values);
        }
        let mut batch = parse(EXAMPLE).unwrap();
        batch.haps[0].4 = Some(f64::NAN);
        assert!(batch.validate().is_err());
    }

    #[test]
    fn parses_the_map_example() {
        let b = parse(EXAMPLE).unwrap();
        assert_eq!(b.values.len(), 2);
        assert_eq!(b.values[0].0, 0);
        assert_eq!(
            b.values[0].2,
            Some(OrbitSpec {
                orbit: 1.0,
                channels: vec![0, 1]
            })
        );
        assert_eq!(b.values[1].2, None);
        assert_eq!(b.haps[1], HapEntry(0.35, 0.25, 0.5, 1, None));
        assert_eq!(b.cps, 0.5);
        assert!(b.validate().is_ok());
    }

    #[test]
    fn round_trips() {
        let b = parse(EXAMPLE).unwrap();
        let text = serde_json::to_string(&b).unwrap();
        assert_eq!(parse(&text).unwrap(), b);
    }

    #[test]
    fn a_two_element_value_entry_fails_to_parse() {
        let text = r#"{"values": [[0, {"s": "sine"}]], "haps": [], "cps": 1}"#;
        assert!(parse(text).is_err());
    }

    #[test]
    fn a_four_element_hap_entry_is_required() {
        let text = r#"{"values": [], "haps": [[0, 1, 0]], "cps": 1}"#;
        assert!(parse(text).is_err());
    }

    fn base() -> Batch {
        parse(EXAMPLE).unwrap()
    }

    #[test]
    fn validate_rejects_non_finite_times() {
        let mut b = base();
        b.haps[0].0 = f64::NAN;
        assert!(b.validate().is_err());
        let mut b = base();
        b.haps[0].2 = f64::INFINITY;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_negative_duration() {
        let mut b = base();
        b.haps[0].1 = -0.1;
        assert!(b.validate().unwrap_err().contains("negative"));
    }

    #[test]
    fn validate_rejects_a_bad_cps() {
        for cps in [0.0, -1.0, f64::NAN] {
            let mut b = base();
            b.cps = cps;
            assert!(b.validate().is_err(), "cps {cps}");
        }
    }

    #[test]
    fn validate_rejects_a_duplicate_index() {
        let mut b = base();
        b.values[1].0 = 0;
        assert!(b.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn validate_accepts_an_unknown_hap_index() {
        let mut b = base();
        b.haps[0].3 = 99;
        assert!(b.validate().is_ok());
    }
}

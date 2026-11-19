//! `orbit_pair` and the stems routing against the table shared with the TS tests
//! (`tests/helpers/orbit-pairs.json`, also read by `tests/render-engine-job.test.ts`).

use serde_json::Value;
use strudeland_engine::orbit::{orbit_pair, route};

const TABLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/helpers/orbit-pairs.json"
));

fn entries() -> Vec<(f64, f64, f64)> {
    let v: Value = serde_json::from_str(TABLE).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let n = |k: &str| e[k].as_f64().unwrap();
            (n("orbit"), n("pairs"), n("pair"))
        })
        .collect()
}

#[test]
fn the_table_covers_wrapping_and_non_positive_orbits() {
    let e = entries();
    assert!(e.len() >= 10);
    assert!(e.iter().any(|(o, p, _)| *o > *p), "an orbit past the pairs");
    assert!(e.iter().any(|(o, _, _)| *o <= 0.0));
    assert!(e.iter().any(|(_, p, _)| *p == 16.0));
}

#[test]
fn orbit_pair_matches_the_shared_table() {
    for (orbit, pairs, pair) in entries() {
        assert_eq!(
            orbit_pair(orbit, pairs),
            pair,
            "orbit {orbit} pairs {pairs}"
        );
    }
}

#[test]
fn stems_channels_route_to_the_pair() {
    // `orbitChannels` with multiChannelOrbits: orbit k > 0 -> [2k-2, 2k-1], else [0, 1]; the
    // merger connection `% count` then lands on the pair `orbit_pair` names.
    for (orbit, pairs, pair) in entries() {
        let k = orbit as i64;
        let channels = if orbit > 0.0 {
            vec![2 * k - 2, 2 * k - 1]
        } else {
            vec![0, 1]
        };
        let p = pair as usize;
        assert_eq!(
            route(&channels, 2 * pairs as usize),
            vec![(0, 2 * p), (1, 2 * p + 1)],
            "orbit {orbit} pairs {pairs}"
        );
    }
}

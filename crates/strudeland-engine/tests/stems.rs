use serde_json::{Value, json};
use strudeland_engine::{
    EngineJob, Rendered, StemGrouping, StemId, StemLayout, render, render_with_stems,
};

fn job(values: Value, events: &[(usize, f64, f64)], orbits: Value) -> EngineJob {
    let haps: Vec<_> = events.iter().enumerate().map(|(i, &(v, t, duration))| {
        json!({"v":v, "t":t, "duration":duration, "cycle":t, "event":i})
    }).collect();
    EngineJob::from_json(
        &json!({
            "schema": 2, "sampleRate": 44100, "frames": 8979, "seed": 245, "cps": 1,
            "channels": {"count":2, "multiChannelOrbits":false}, "orbits": orbits,
            "pauses":[{"suspendSec":0,"haps":haps}], "values":values
        })
        .to_string(),
    )
    .unwrap()
}

fn stereo(orbits: &[i32]) -> Value {
    json!(
        orbits
            .iter()
            .map(|o| json!({"orbit":o,"channels":[0,1]}))
            .collect::<Vec<_>>()
    )
}

/// Verify the bit pattern of the reference (including signed zero) and the precision of a DAW
/// summing these f32 stems with a double accumulator. The rounding bound scales with signal and
/// summand count, with only one f32 subnormal as the absolute floor.
fn capture(job: &EngineJob, layout: &StemLayout) -> Rendered {
    let ordinary = render(job).unwrap();
    let result = render_with_stems(job, layout).unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.errors, ordinary.errors);
    assert_eq!(result.channels.len(), 2 + 2 * layout.ids().len());
    for (ch, reference) in ordinary.channels.iter().enumerate() {
        for (i, &x) in reference.iter().enumerate() {
            assert_eq!(x.to_bits(), result.channels[ch][i].to_bits());
            let mut sum = 0.0;
            let mut magnitude = f64::from(x).abs();
            for pair in result.channels[2..].chunks_exact(2) {
                let sample = f64::from(pair[ch][i]);
                sum += sample;
                magnitude += sample.abs();
            }
            let tolerance = (layout.ids().len() + 1) as f64 * f64::from(f32::EPSILON) * magnitude
                + f64::from(f32::from_bits(1));
            assert!(
                (sum - f64::from(x)).abs() <= tolerance,
                "frame {i}, channel {ch}: {sum} vs {x}, tolerance {tolerance}"
            );
        }
    }
    result
}

fn pair<'a>(result: &'a Rendered, layout: &StemLayout, id: StemId) -> &'a [Vec<f32>] {
    let ch = 2 + 2 * layout.ids().iter().position(|s| s == &id).unwrap();
    &result.channels[ch..ch + 2]
}

#[test]
fn layout_spans_scheduled_values_defaults_and_distinct_user_labels() {
    let first = job(
        json!([
            {"label":"unlabeled"}, {"s":"sine","label":""}, {"s":"_","label":"rest","orbit":4},
            {"s":"saw","label":"unused"}
        ]),
        &[(0, 0.0, 0.03), (1, 0.05, 0.03), (2, 0.0, 0.1)],
        stereo(&[1, 4]),
    );
    let later = job(
        json!([{"s":"one","label":"FX","orbit":91}]),
        &[(0, 0.0, 0.02)],
        stereo(&[91]),
    );
    let jobs = [first, later];
    let layout = StemLayout::for_jobs(&jobs, StemGrouping::Voice);
    assert_eq!(
        layout.ids(),
        &[
            StemId::Voice("FX".into()),
            StemId::Voice("unlabeled".into()),
            StemId::Unlabeled,
            StemId::Effects("1".into()),
            StemId::Effects("91".into())
        ]
    );
    let first = capture(&jobs[0], &layout);
    assert!(
        pair(&first, &layout, StemId::Voice("FX".into()))
            .iter()
            .flatten()
            .all(|&x| x == 0.0)
    );
    for id in [StemId::Voice("unlabeled".into()), StemId::Unlabeled] {
        assert!(
            pair(&first, &layout, id)
                .iter()
                .flatten()
                .any(|&x| x != 0.0)
        );
    }
    let layout = StemLayout::for_jobs(&jobs, StemGrouping::Instrument);
    assert!(
        layout
            .ids()
            .contains(&StemId::Instrument("triangle".into()))
    );
    let bank = job(
        json!([{"bank":"808","s":"bd"}]),
        &[(0, 0.0, 0.03)],
        stereo(&[1]),
    );
    assert!(
        StemLayout::for_jobs(&[bank], StemGrouping::Instrument)
            .ids()
            .contains(&StemId::Instrument("808_bd".into()))
    );
}

#[test]
fn direct_groups_follow_real_orbits_and_explicit_merger_connections() {
    let j = job(
        json!([
            {"s":"one","label":"A","orbit":0,"gain":0.25,"pan":0.2},
            {"s":"one","label":"A","orbit":103,"gain":0.5,"pan":0.8},
            {"s":"one","label":"B","gain":0.125},
        ]),
        &[
            (0, 0.0, 0.13),
            (1, 0.02, 0.08),
            (2, 0.04, 0.12),
            (0, 0.06, 0.03),
        ],
        json!([{"orbit":0,"channels":[1,0]},{"orbit":103,"channels":[1,1]},{"orbit":1,"channels":[0,1]}]),
    );
    let orbit_layout = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Orbit);
    let orbits = capture(&j, &orbit_layout);
    let one = pair(&orbits, &orbit_layout, StemId::Orbit("1".into()));
    let zero = pair(&orbits, &orbit_layout, StemId::Orbit("0".into()));
    let high = pair(&orbits, &orbit_layout, StemId::Orbit("103".into()));
    assert!(high[0].iter().all(|&x| x == 0.0));
    assert!(high[1].iter().any(|&x| x > 0.1));
    let labels = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Voice);
    let voices = capture(&j, &labels);
    assert_eq!(pair(&voices, &labels, StemId::Voice("B".into())), one);
    let a = pair(&voices, &labels, StemId::Voice("A".into()));
    for ch in 0..2 {
        for (i, &x) in a[ch].iter().enumerate() {
            assert!((x - zero[ch][i] - high[ch][i]).abs() < 2e-7);
        }
    }
    let instruments = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Instrument);
    let output = capture(&j, &instruments);
    let only = pair(&output, &instruments, StemId::Instrument("one".into()));
    assert_eq!(only, &output.channels[..2]);
}

#[test]
fn direct_uses_actual_duck_gain_and_shared_returns_follow_the_orbit() {
    let shared = json!({"s":"one","label":"part","orbit":6,"gain":0.4,"room":0.55,"roomsize":0.05,
        "roomlp":0,"delay":0.6,"delaytime":0.02,"delayfeedback":0.6,"djf":0.15});
    let direct = json!({"s":"one","label":"part","orbit":6,"gain":0.4});
    let marker = json!({"s":"~","label":"marker","duckorbit":6,"duckdepth":0.81,"duckattack":0.02});
    let events = [(0, 0.0, 0.07), (1, 0.02, 0.02)];
    let reference = job(json!([direct, marker]), &events, stereo(&[6, 1]));
    let ducked = render(&reference).unwrap();
    let plain = job(json!([direct]), &events[..1], stereo(&[6]));
    assert!(ducked.channels[0][900] < 0.6 * render(&plain).unwrap().channels[0][900]);
    let j = job(json!([shared, marker]), &events, stereo(&[6, 1]));
    let layout = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Voice);
    assert_eq!(layout.ids().len(), 2);
    let rendered = capture(&j, &layout);
    assert_eq!(
        pair(&rendered, &layout, StemId::Voice("part".into())),
        ducked.channels
    );
    let fx = pair(&rendered, &layout, StemId::Effects("6".into()));
    assert!(fx[0][4500..].iter().any(|x| x.abs() > 0.001));

    let mut mapped = j;
    mapped.orbits[0].channels = vec![1, 1];
    let rendered = capture(&mapped, &layout);
    let fx = pair(&rendered, &layout, StemId::Effects("6".into()));
    assert!(fx[0].iter().all(|&x| x == 0.0));
    assert!(fx[1][4500..].iter().any(|x| x.abs() > 0.001));
}

#[test]
fn bus_output_belongs_to_the_reader_and_silence_has_no_stems() {
    let j = job(
        json!([
            {"s":"one","label":"feed","bus":7,"dry":0},
            {"s":"bus","label":"read","n":7,"orbit":2}
        ]),
        &[(0, 0.0, 0.08), (1, 0.0, 0.09)],
        stereo(&[1, 2]),
    );
    let layout = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Voice);
    let result = capture(&j, &layout);
    assert!(
        pair(&result, &layout, StemId::Voice("feed".into()))
            .iter()
            .flatten()
            .all(|&x| x == 0.0)
    );
    let reader = pair(&result, &layout, StemId::Voice("read".into()));
    assert_eq!(reader, &result.channels[..2]);
    assert!(reader[0].iter().any(|x| x.abs() > 0.05));

    let silent = job(json!([{"s":"-"}]), &[(0, 0.0, 0.1)], stereo(&[0, 1]));
    let empty = StemLayout::for_jobs(std::slice::from_ref(&silent), StemGrouping::Orbit);
    assert!(empty.ids().is_empty());
    capture(&silent, &empty);
    assert!(render_with_stems(&j, &empty).is_err());
}

#[test]
fn effects_retain_merger_roundoff_exposed_by_cross_orbit_cancellation() {
    let j = job(
        json!([
            {"s":"one","orbit":1,"label":"loud","gain":1},
            {"s":"one","orbit":2,"label":"quiet","gain":1e-10},
            {"s":"one","orbit":3,"label":"loud","gain":-1}
        ]),
        &[(0, 0.0, 0.1), (1, 0.0, 0.1), (2, 0.0, 0.1)],
        stereo(&[1, 2, 3]),
    );
    let layout = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Voice);
    let r = capture(&j, &layout);
    let quiet = pair(&r, &layout, StemId::Voice("quiet".into()))[0][500];
    assert!(quiet > 0.0);
    assert_eq!(
        pair(&r, &layout, StemId::Effects("2".into()))[0][500],
        -quiet
    );
    assert_eq!(r.channels[0][500], 0.0);
}

#[test]
fn capture_refuses_an_overflowing_mix_even_when_each_orbit_is_finite() {
    let j = job(
        json!([
            {"s":"one","orbit":1,"sustain":1,"gain":3e38,"postgain":3},
            {"s":"one","orbit":2,"sustain":1,"gain":3e38,"postgain":3}
        ]),
        &[(0, 0.0, 0.05), (1, 0.0, 0.05)],
        stereo(&[1, 2]),
    );
    let layout = StemLayout::for_jobs(std::slice::from_ref(&j), StemGrouping::Orbit);
    assert!(
        render_with_stems(&j, &layout)
            .unwrap_err()
            .to_string()
            .contains("non-finite stem reference mix")
    );
}

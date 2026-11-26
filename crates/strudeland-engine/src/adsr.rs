//! Superdough's envelope helpers (helpers.mjs): `getADSRValues` (the default rules) and
//! `getParamADSR` (the event list it schedules on a param), plus util.mjs's `nanFallback`.
//! The arithmetic is JS `f64`; values reach the param through WebIDL `float` conversion.

use crate::coerce::{js_max, js_min};
use crate::param::{Param, ParamError, idl_float};

/// The `curve` argument (`'linear'` or `'exponential'`; `getParamADSR`'s default and any other
/// value, such as the `undefined` from a `pcurve` outside {0, 1}, behave as exponential).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Exponential,
}

/// `getADSRValues`'s own defaults when the caller passes none.
pub const DEFAULT_ADSR: [f64; 4] = [0.001, 0.001, 1.0, 0.01];

/// util.mjs `nanFallback(value, fallback)` on a number: NaN becomes `fallback`.
pub fn nan_fallback(x: f64, fallback: f64) -> f64 {
    if x.is_nan() { fallback } else { x }
}

/// `getADSRValues(params, curve, defaultValues)` (helpers.mjs:182-193). `None` is a missing or
/// `null` value (the JS tests `== null` and uses `??`). All four missing: `defaults`. Else the
/// sustain is `s` when given, 1 when `attack` is given without `decay` or neither is, else 0.001;
/// the result is `[max(a ?? 0, 0.001), max(d ?? 0, 0.001), min(sustain, 1), max(r ?? 0, 0.01)]`
/// with JS `Math.max`/`Math.min` (NaN stays NaN; no lower clamp on the sustain). The curve does not
/// change anything (`envmin` is 0.001 for both); it is kept for the call sites' symmetry.
pub fn get_adsr_values(params: [Option<f64>; 4], _curve: Curve, defaults: [f64; 4]) -> [f64; 4] {
    let [a, d, s, r] = params;
    if a.is_none() && d.is_none() && s.is_none() && r.is_none() {
        return defaults;
    }
    let sustain = match s {
        Some(s) => s,
        None if d.is_none() => 1.0,
        None => 0.001,
    };
    [
        js_max(a.unwrap_or(0.0), 0.001),
        js_max(d.unwrap_or(0.0), 0.001),
        js_min(sustain, 1.0),
        js_max(r.unwrap_or(0.0), 0.01),
    ]
}

/// helpers.mjs `getSlope`.
fn slope(y1: f64, y2: f64, x1: f64, x2: f64) -> f64 {
    let denom = x2 - x1;
    if denom == 0.0 {
        0.0
    } else {
        (y2 - y1) / (x2 - x1)
    }
}

/// `getParamADSR(param, a, d, s, r, min, max, begin, end, curve)` (helpers.mjs:55-114), called at
/// context frame `now`:
///
/// 1. `setValueAtTime(min, begin)`;
/// 2. `a > dur`: ramp to the attack line's value at `dur`, at `end`; else `a + d > dur`: ramp to
///    `max` at `begin + a`, then to the decay line's value at `dur`, at `end`; else ramp to `max`
///    at `begin + a`, to the sustain value at `begin + a + d`, and `setValueAtTime(sustain, end)`;
/// 3. ramp to `min` at `end + r`.
///
/// `a`, `d`, `s`, `r` go through `nanFallback` (0). Exponential curves map a `min`/`max` of 0 to
/// 0.001 and a computed ramp target of 0 or NaN to 0.001; the target values are linear
/// interpolations either way. The first call that throws stops the list (the events before it
/// stay, as in JS) and its error is returned.
#[allow(clippy::too_many_arguments)]
pub fn get_param_adsr(
    param: &mut Param,
    attack: f64,
    decay: f64,
    sustain: f64,
    release: f64,
    min: f64,
    max: f64,
    begin: f64,
    end: f64,
    curve: Curve,
    now: u64,
) -> Result<(), ParamError> {
    let a = nan_fallback(attack, 0.0);
    let d = nan_fallback(decay, 0.0);
    let s = nan_fallback(sustain, 0.0);
    let r = nan_fallback(release, 0.0);
    let exp = curve == Curve::Exponential;
    let (min, max) = if exp {
        (
            if min == 0.0 { 0.001 } else { min },
            if max == 0.0 { 0.001 } else { max },
        )
    } else {
        (min, max)
    };
    let range = max - min;
    let sustain_val = min + s * range;
    let duration = end - begin;
    let env_val_at_time = |time: f64| {
        let val = if a > time {
            time * slope(min, max, 0.0, a) + min
        } else {
            (time - a) * slope(max, sustain_val, 0.0, d) + max
        };
        if exp && (val == 0.0 || val.is_nan()) {
            0.001
        } else {
            val
        }
    };
    let ramp = |p: &mut Param, v: f64, t: f64| match curve {
        Curve::Linear => p.linear_ramp_to(idl_float(v, "linearRampToValueAtTime")?, t, now),
        Curve::Exponential => {
            p.exponential_ramp_to(idl_float(v, "exponentialRampToValueAtTime")?, t, now)
        }
    };
    param.set_value_at_time(idl_float(min, "setValueAtTime")?, begin, now)?;
    if a > duration {
        ramp(param, env_val_at_time(duration), end)?;
    } else if a + d > duration {
        ramp(param, env_val_at_time(a), begin + a)?;
        ramp(param, env_val_at_time(duration), end)?;
    } else {
        ramp(param, env_val_at_time(a), begin + a)?;
        ramp(param, env_val_at_time(a + d), begin + a + d)?;
        param.set_value_at_time(idl_float(sustain_val, "setValueAtTime")?, end, now)?;
    }
    ramp(param, min, end + r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::{RampKind, Rate};
    use crate::test_vectors::{load, num};
    use serde_json::Value as J;

    use crate::SAMPLE_RATE_F64 as SR;

    fn opt(v: &J) -> Option<f64> {
        if v.is_null() { None } else { Some(num(v)) }
    }

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    fn param() -> Param {
        Param::new(1.0, -f32::MAX, f32::MAX, Rate::A)
    }

    fn events(p: &Param) -> Vec<(f64, f32, RampKind)> {
        p.events()
            .iter()
            .map(|e| (e.time, e.value, e.kind))
            .collect()
    }

    #[test]
    fn adsr_values_match_the_js_vectors() {
        let v = load("adsr.json");
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        for c in cases {
            let p: Vec<Option<f64>> = c["params"].as_array().unwrap().iter().map(opt).collect();
            let curve = match c["curve"].as_str().unwrap() {
                "linear" => Curve::Linear,
                _ => Curve::Exponential,
            };
            let defaults = if c["defaults"].is_null() {
                DEFAULT_ADSR
            } else {
                let d: Vec<f64> = c["defaults"].as_array().unwrap().iter().map(num).collect();
                [d[0], d[1], d[2], d[3]]
            };
            let got = get_adsr_values([p[0], p[1], p[2], p[3]], curve, defaults);
            let want: Vec<f64> = c["result"].as_array().unwrap().iter().map(num).collect();
            for k in 0..4 {
                assert!(
                    same(got[k], want[k]),
                    "{c}: [{k}] {} vs {}",
                    got[k],
                    want[k]
                );
            }
        }
    }

    #[test]
    fn param_adsr_matches_the_js_vectors() {
        let v = load("adsr.json");
        let cases = v["paramAdsr"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for c in cases {
            let adsr: Vec<f64> = c["adsr"].as_array().unwrap().iter().map(num).collect();
            let curve = match c["curve"].as_str().unwrap() {
                "linear" => Curve::Linear,
                _ => Curve::Exponential,
            };
            let mut p = param();
            get_param_adsr(
                &mut p,
                adsr[0],
                adsr[1],
                adsr[2],
                adsr[3],
                num(&c["min"]),
                num(&c["max"]),
                num(&c["begin"]),
                num(&c["end"]),
                curve,
                0,
            )
            .unwrap();
            // The JS calls, inserted with Blink's rule (after every event at or before the time).
            let mut want: Vec<(f64, f32, RampKind)> = Vec::new();
            for call in c["calls"].as_array().unwrap() {
                let kind = match call["method"].as_str().unwrap() {
                    "setValueAtTime" => RampKind::SetValue,
                    "linearRampToValueAtTime" => RampKind::Linear,
                    _ => RampKind::Exponential,
                };
                let (value, time) = (num(&call["value"]) as f32, num(&call["time"]));
                let at = want.partition_point(|e| e.0 <= time);
                want.insert(at, (time, value, kind));
            }
            assert_eq!(events(&p), want, "{c}");
        }
    }

    #[test]
    fn adsr_value_rules() {
        let d = [0.001, 0.05, 0.6, 0.01];
        let l = Curve::Linear;
        assert_eq!(get_adsr_values([None; 4], l, d), d);
        assert_eq!(get_adsr_values([None; 4], l, DEFAULT_ADSR), DEFAULT_ADSR);
        assert_eq!(
            get_adsr_values([Some(0.1), None, None, None], l, d),
            [0.1, 0.001, 1.0, 0.01]
        );
        assert_eq!(
            get_adsr_values([None, Some(0.2), None, None], l, d),
            [0.001, 0.2, 0.001, 0.01]
        );
        assert_eq!(
            get_adsr_values([None, None, Some(0.3), None], l, d),
            [0.001, 0.001, 0.3, 0.01]
        );
        assert_eq!(
            get_adsr_values([Some(0.1), Some(0.2), None, None], l, d),
            [0.1, 0.2, 0.001, 0.01]
        );
        assert_eq!(
            get_adsr_values([None, None, None, Some(0.5)], l, d),
            [0.001, 0.001, 1.0, 0.5]
        );
        assert_eq!(
            get_adsr_values([None, None, Some(2.0), Some(0.0)], l, d),
            [0.001, 0.001, 1.0, 0.01]
        );
        assert_eq!(
            get_adsr_values([Some(-1.0), None, Some(-0.5), None], Curve::Exponential, d),
            [0.001, 0.001, -0.5, 0.01],
            "no lower clamp on the sustain"
        );
        // Math.max/Math.min keep NaN (getParamADSR's nanFallback handles it later).
        let nan = get_adsr_values([Some(f64::NAN), None, Some(f64::NAN), Some(f64::NAN)], l, d);
        assert!(nan[0].is_nan() && nan[2].is_nan() && nan[3].is_nan());
        assert_eq!(nan[1], 0.001);
    }

    #[test]
    fn nan_fallback_replaces_only_nan() {
        assert_eq!(nan_fallback(f64::NAN, 0.0), 0.0);
        assert_eq!(nan_fallback(f64::NAN, 1.0), 1.0);
        assert_eq!(nan_fallback(0.5, 1.0), 0.5);
        assert_eq!(nan_fallback(f64::INFINITY, 0.0), f64::INFINITY);
        assert!(nan_fallback(-0.0, 1.0).is_sign_negative());
    }

    #[test]
    fn the_three_shapes() {
        use RampKind::*;
        let run = |a, d, s, r, end, curve| {
            let mut p = param();
            get_param_adsr(&mut p, a, d, s, r, 0.0, 1.0, 0.0, end, curve, 0).unwrap();
            events(&p)
        };
        // Normal.
        assert_eq!(
            run(0.1, 0.2, 0.5, 0.3, 1.0, Curve::Linear),
            vec![
                (0.0, 0.0, SetValue),
                (0.1, 1.0, Linear),
                (0.1 + 0.2, 0.5, Linear),
                (1.0, 0.5, SetValue),
                (1.3, 0.0, Linear)
            ]
        );
        // a + d > dur: the decay line's value at dur (0.5 - 0.2 = 0.3 into a 1 -> 0 decay of 0.6).
        let e = run(0.2, 0.6, 0.0, 0.1, 0.5, Curve::Linear);
        assert_eq!(e.len(), 4);
        assert_eq!(e[1], (0.2, 1.0, Linear));
        assert_eq!(e[2], (0.5, 0.5, Linear));
        assert_eq!(e[3], (0.6, 0.0, Linear));
        // a > dur: the attack line's value at dur.
        let e = run(1.0, 0.1, 0.5, 0.1, 0.25, Curve::Linear);
        assert_eq!(
            e,
            vec![
                (0.0, 0.0, SetValue),
                (0.25, 0.25, Linear),
                (0.35, 0.0, Linear)
            ]
        );
        // Exponential: min 0 -> 0.001, targets of 0 -> 0.001.
        let e = run(0.1, 0.2, 0.0, 0.3, 1.0, Curve::Exponential);
        assert_eq!(e[0], (0.0, 0.001, SetValue));
        assert_eq!(e[2].2, Exponential);
        assert_eq!(e[2].1, 0.001);
        assert_eq!(e[4], (1.3, 0.001, Exponential));
    }

    #[test]
    fn exponential_zero_targets_from_the_lines_become_0_001() {
        // min -1, max 1, sustain 0.5: the sustain value is 0 -> 0.001 on the exponential curve.
        let mut p = param();
        get_param_adsr(
            &mut p,
            0.1,
            0.1,
            0.5,
            0.1,
            -1.0,
            1.0,
            0.0,
            1.0,
            Curve::Exponential,
            0,
        )
        .unwrap();
        assert_eq!(p.events()[2].value, 0.001);
        // -0 is falsy too.
        let mut p = param();
        get_param_adsr(
            &mut p,
            0.1,
            0.1,
            0.0,
            0.1,
            -0.0,
            -0.0,
            0.0,
            1.0,
            Curve::Exponential,
            0,
        )
        .unwrap();
        assert_eq!(p.events()[0].value, 0.001);
    }

    #[test]
    fn error_paths_stop_the_list() {
        // A sustain value that overflows f32: the setValueAtTime(sustain) throws; the events
        // before it stay.
        let mut p = param();
        let e = get_param_adsr(
            &mut p,
            0.1,
            0.1,
            -1e300,
            0.1,
            0.0,
            1.0,
            0.0,
            1.0,
            Curve::Linear,
            0,
        )
        .unwrap_err();
        assert!(matches!(e, ParamError::NonFiniteValue { .. }), "{e}");
        assert!(
            e.to_string()
                .starts_with("TypeError: Failed to execute 'linearRampToValueAtTime'")
        );
        assert_eq!(p.events().len(), 2);
        // A negative begin: RangeError on the first call, nothing inserted.
        let mut p = param();
        let e = get_param_adsr(
            &mut p,
            0.1,
            0.1,
            0.5,
            0.1,
            0.0,
            1.0,
            -1.0,
            1.0,
            Curve::Linear,
            0,
        )
        .unwrap_err();
        assert_eq!(
            e,
            ParamError::NegativeTime {
                method: "setValueAtTime",
                time: -1.0
            }
        );
        assert!(p.events().is_empty());
        // An infinite release: the last ramp's time is not finite.
        let mut p = param();
        let e = get_param_adsr(
            &mut p,
            0.1,
            0.1,
            0.5,
            f64::INFINITY,
            0.0,
            1.0,
            0.0,
            1.0,
            Curve::Exponential,
            0,
        )
        .unwrap_err();
        assert_eq!(
            e,
            ParamError::NonFiniteTime {
                method: "exponentialRampToValueAtTime"
            }
        );
        assert_eq!(p.events().len(), 4);
    }

    #[test]
    fn times_before_now_are_clamped() {
        // Scheduled at frame 4410 (0.1 s) with begin 0: every earlier time moves to 0.1 s.
        let mut p = param();
        get_param_adsr(
            &mut p,
            0.01,
            0.01,
            0.5,
            0.01,
            0.0,
            1.0,
            0.0,
            0.5,
            Curve::Linear,
            4410,
        )
        .unwrap();
        let times: Vec<f64> = p.events().iter().map(|e| e.time).collect();
        assert_eq!(
            times,
            vec![4410.0 / SR, 4410.0 / SR, 4410.0 / SR, 0.5, 0.51]
        );
    }
}

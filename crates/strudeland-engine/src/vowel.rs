//! superdough's vowel filter (`VowelNode` in vowel.mjs, built by `ac.createVowelFilter` in the
//! FX loop when `vowel !== undefined`): the input fanned out to five bandpass biquads, each
//! followed by its formant gain, summed into a makeup gain of 8. The formant table is vowel.mjs's
//! `vowelFormant`, letters and aliases. The first five formants and filter topology
//! descend from WebDirt's Graph.js at 70e97c158a1ece0be4a0f4ad0da5a2f6ebe32530
//! (Jamie Beverley and David Ogborn, GPLv3; see ../../../LICENSES/GPL-3.0.txt).
//! Strudel adapted the filter on 2022-09-15 and added the remaining rows and aliases
//! on 2024-01-13. Strudeland ported the Strudel filter and table to Rust on 2026-11-12.
//!
//! Chromium sums the five formant outputs at the makeup gain's input in a hash-set order that
//! changes between runs (the last bit of the sum varies); the engine sums them in index order,
//! and the `vowel-formants` reference is a measured-floor case.

use crate::QUANTUM;
use crate::biquad::{Biquad, BiquadInputs, FilterKind};
use crate::chain::VoiceBus;
use crate::job::Value;
use crate::param::{Param, ParamInputs};
use crate::voice::GainNode;

/// One letter's formants: frequencies (Hz), gains and linear Qs.
pub type Formant = ([f64; 5], [f64; 5], [f64; 5]);

/// vowel.mjs's `vowelFormant`, in the file's order: `(letter, freqs, gains, qs)`.
#[allow(clippy::type_complexity)] // the shape the design names
pub const FORMANTS: [(&str, [f64; 5], [f64; 5], [f64; 5]); 15] = [
    (
        "a",
        [660.0, 1120.0, 2750.0, 3000.0, 3350.0],
        [1.0, 0.5012, 0.0708, 0.0631, 0.0126],
        [80.0, 90.0, 120.0, 130.0, 140.0],
    ),
    (
        "e",
        [440.0, 1800.0, 2700.0, 3000.0, 3300.0],
        [1.0, 0.1995, 0.1259, 0.1, 0.1],
        [70.0, 80.0, 100.0, 120.0, 120.0],
    ),
    (
        "i",
        [270.0, 1850.0, 2900.0, 3350.0, 3590.0],
        [1.0, 0.0631, 0.0631, 0.0158, 0.0158],
        [40.0, 90.0, 100.0, 120.0, 120.0],
    ),
    (
        "o",
        [430.0, 820.0, 2700.0, 3000.0, 3300.0],
        [1.0, 0.3162, 0.0501, 0.0794, 0.01995],
        [40.0, 80.0, 100.0, 120.0, 120.0],
    ),
    (
        "u",
        [370.0, 630.0, 2750.0, 3000.0, 3400.0],
        [1.0, 0.1, 0.0708, 0.0316, 0.01995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "ae",
        [650.0, 1515.0, 2400.0, 3000.0, 3350.0],
        [1.0, 0.5, 0.1008, 0.0631, 0.0126],
        [80.0, 90.0, 120.0, 130.0, 140.0],
    ),
    (
        "aa",
        [560.0, 900.0, 2570.0, 3000.0, 3300.0],
        [1.0, 0.5, 0.0708, 0.0631, 0.0126],
        [80.0, 90.0, 120.0, 130.0, 140.0],
    ),
    (
        "oe",
        [500.0, 1430.0, 2300.0, 3000.0, 3300.0],
        [1.0, 0.2, 0.0708, 0.0316, 0.01995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "ue",
        [250.0, 1750.0, 2150.0, 3200.0, 3300.0],
        [1.0, 0.1, 0.0708, 0.0316, 0.01995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "y",
        [400.0, 1460.0, 2400.0, 3000.0, 3300.0],
        [1.0, 0.2, 0.0708, 0.0316, 0.02995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "uh",
        [600.0, 1250.0, 2100.0, 3100.0, 3500.0],
        [1.0, 0.3, 0.0608, 0.0316, 0.01995],
        [40.0, 70.0, 100.0, 120.0, 130.0],
    ),
    (
        "un",
        [500.0, 1240.0, 2280.0, 3000.0, 3500.0],
        [1.0, 0.1, 0.1708, 0.0216, 0.02995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "en",
        [600.0, 1480.0, 2450.0, 3200.0, 3300.0],
        [1.0, 0.15, 0.0708, 0.0316, 0.02995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "an",
        [700.0, 1050.0, 2500.0, 3000.0, 3300.0],
        [1.0, 0.1, 0.0708, 0.0316, 0.02995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
    (
        "on",
        [500.0, 1080.0, 2350.0, 3000.0, 3300.0],
        [1.0, 0.1, 0.0708, 0.0316, 0.02995],
        [40.0, 60.0, 100.0, 120.0, 120.0],
    ),
];

/// vowel.mjs's getters: `(alias, letter)`.
pub const ALIASES: [(&str, &str); 7] = [
    ("æ", "ae"),
    ("ø", "oe"),
    ("ɑ", "aa"),
    ("å", "aa"),
    ("ö", "oe"),
    ("ü", "ue"),
    ("ı", "y"),
];

/// The makeup gain (`makeupGain.gain.value = 8`).
const MAKEUP: f32 = 8.0;

/// `vowelFormant[letter]`: a table letter or an alias; anything else (another string, `null`, a
/// number, ...) throws `vowel: unknown vowel <String(letter)>`.
pub fn formant(letter: &Value) -> Result<Formant, String> {
    if let Value::Str(s) = letter {
        let name = ALIASES
            .iter()
            .find(|(a, _)| a == s)
            .map_or(s.as_str(), |(_, l)| l);
        if let Some((_, f, g, q)) = FORMANTS.iter().find(|(l, ..)| *l == name) {
            return Ok((*f, *g, *q));
        }
    }
    let text = crate::coerce::js_to_string(letter);
    Err(format!("vowel: unknown vowel {text}"))
}

/// One formant path: its bandpass biquad and its gain.
#[derive(Debug, Clone)]
pub struct Path {
    pub filter: Biquad,
    pub gain: GainNode,
}

/// The vowel stage: five formant paths summed into the makeup gain.
#[derive(Debug, Clone)]
pub struct Vowel {
    pub paths: Vec<Path>,
    pub makeup: GainNode,
}

impl Vowel {
    /// `new VowelNode(ac, letter)` at frame `now`: for each formant in order, the gain's `.value =
    /// gains[i]`, then the bandpass's `Q.value = qs[i]` and `frequency.value = freqs[i]` (each a
    /// `setValueAtTime` at `now`), then the makeup gain's `.value = 8`. An unknown letter throws
    /// before any of them.
    pub fn new(letter: &Value, now: u64) -> Result<Vowel, String> {
        let (freqs, gains, qs) = formant(letter)?;
        let mut paths = Vec::with_capacity(5);
        for i in 0..5 {
            let mut gain = GainNode::new(Param::gain(1.0));
            // Table values are finite: these never throw.
            let _ = gain.gain.set_value(gains[i] as f32, now);
            let mut filter = Biquad::new(FilterKind::Bandpass);
            let _ = filter.q.set_value(qs[i] as f32, now);
            let _ = filter.frequency.set_value(freqs[i] as f32, now);
            paths.push(Path { filter, gain });
        }
        let mut makeup = GainNode::new(Param::gain(1.0));
        let _ = makeup.gain.set_value(MAKEUP, now);
        Ok(Vowel { paths, makeup })
    }

    /// One quantum in place: each path filters and scales a copy of the input, the five outputs
    /// are summed in index order (silent only when all five are), then the makeup gain.
    #[cfg(test)]
    pub fn process(&mut self, q: u64, bus: &mut VoiceBus) {
        self.process_with(q, bus, &[const { ParamInputs::NONE }; 5]);
    }

    /// [`Vowel::process`] with the connections into each formant's `frequency`.
    pub fn process_with(&mut self, q: u64, bus: &mut VoiceBus, formants: &[ParamInputs; 5]) {
        let n = bus.channels;
        let input_silent = bus.silent;
        let mut sum = [[0.0f32; QUANTUM]; 2];
        let mut all_silent = true;
        // The `VowelNode` itself, `GainNode(1)` with no event: a silent input gives zeros.
        let fan_out = if input_silent {
            [[0.0f32; QUANTUM]; 2]
        } else {
            bus.ch
        };
        for (i, p) in self.paths.iter_mut().enumerate() {
            let mut c = fan_out;
            let fs = if formants[i].connected {
                let inputs = BiquadInputs {
                    frequency: &formants[i],
                    ..BiquadInputs::NONE
                };
                p.filter.process(q, &mut c[..n], input_silent, &inputs)
            } else {
                p.filter
                    .process(q, &mut c[..n], input_silent, &BiquadInputs::NONE)
            };
            let gs = p.gain.process(q, &mut c[..n], fs, &ParamInputs::NONE);
            all_silent &= gs;
            if i == 0 {
                sum = c;
            } else {
                for (s, x) in sum[..n].iter_mut().zip(c[..n].iter()) {
                    for (a, b) in s.iter_mut().zip(x.iter()) {
                        *a += *b;
                    }
                }
            }
        }
        bus.silent = self
            .makeup
            .process(q, &mut sum[..n], all_silent, &ParamInputs::NONE);
        bus.ch = sum;
    }

    /// Whether the five biquads propagate silence from the quantum after `q` (with a silent
    /// input, the stage outputs only silence from then on).
    pub fn is_quiet_after(&self, q: u64) -> bool {
        let next = q + QUANTUM as u64;
        self.paths.iter().all(|p| p.filter.propagates_silence(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::RampKind;

    const SOURCE: &str = include_str!("../../../vendor/strudel/packages/superdough/vowel.mjs");

    /// The numbers between `[` and `]` after `key: ` in `line`.
    fn list(line: &str, key: &str) -> [f64; 5] {
        let start = line.find(&format!(" {key}: [")).unwrap() + key.len() + 4;
        let end = start + line[start..].find(']').unwrap();
        let v: Vec<f64> = line[start..end]
            .split(',')
            .map(|x| x.trim().parse().unwrap())
            .collect();
        v.try_into().unwrap()
    }

    #[test]
    fn table_matches_vowel_mjs() {
        let mut rows = Vec::new();
        let mut aliases = Vec::new();
        let mut lines = SOURCE.lines();
        while let Some(line) = lines.next() {
            let t = line.trim();
            if let Some((name, rest)) = t.split_once(": { freqs:") {
                let _ = rest;
                rows.push((
                    name.to_string(),
                    list(t, "freqs"),
                    list(t, "gains"),
                    list(t, "qs"),
                ));
            } else if let Some(alias) = t.strip_prefix("get ").and_then(|r| r.strip_suffix("() {"))
            {
                let body = lines.next().unwrap().trim();
                let target = body
                    .strip_prefix("return this.")
                    .and_then(|r| r.strip_suffix(';'))
                    .unwrap();
                aliases.push((alias.to_string(), target.to_string()));
            }
        }
        assert_eq!(rows.len(), 15);
        for (row, want) in rows.iter().zip(FORMANTS.iter()) {
            assert_eq!(row.0, want.0);
            assert_eq!(
                (row.1, row.2, row.3),
                (want.1, want.2, want.3),
                "{}",
                want.0
            );
        }
        let want: Vec<(String, String)> = ALIASES
            .iter()
            .map(|(a, l)| (a.to_string(), l.to_string()))
            .collect();
        assert_eq!(aliases, want);
    }

    #[test]
    fn unknown_letter_message() {
        let msg = |v: Value| formant(&v).unwrap_err();
        assert_eq!(msg(Value::Str("zz".into())), "vowel: unknown vowel zz");
        assert_eq!(msg(Value::Null), "vowel: unknown vowel null");
        assert_eq!(msg(Value::Num(1.0)), "vowel: unknown vowel 1");
        assert_eq!(msg(Value::Num(0.5)), "vowel: unknown vowel 0.5");
        assert_eq!(msg(Value::Bool(true)), "vowel: unknown vowel true");
        assert_eq!(
            msg(Value::Str("constructor".into())),
            "vowel: unknown vowel constructor"
        );
        assert_eq!(msg(Value::Str("A".into())), "vowel: unknown vowel A");
        assert!(Vowel::new(&Value::Str("x".into()), 0).is_err());
    }

    #[test]
    fn aliases_resolve() {
        for (alias, letter) in ALIASES {
            assert_eq!(
                formant(&Value::Str(alias.into())),
                formant(&Value::Str(letter.into())),
                "{alias}"
            );
        }
        assert_eq!(formant(&Value::Str("a".into())).unwrap().0[0], 660.0);
    }

    #[test]
    fn graph_event_order() {
        let now = 1280;
        let v = Vowel::new(&Value::Str("o".into()), now).unwrap();
        let (f, g, q) = formant(&Value::Str("o".into())).unwrap();
        let one = |p: &Param, value: f32| {
            let e = p.events();
            assert_eq!(e.len(), 1);
            assert_eq!(e[0].kind, RampKind::SetValue);
            assert_eq!(e[0].time, now as f64 / 44100.0);
            assert_eq!(e[0].value, value);
        };
        assert_eq!(v.paths.len(), 5);
        for (i, p) in v.paths.iter().enumerate() {
            one(&p.gain.gain, g[i] as f32);
            one(&p.filter.q, q[i] as f32);
            one(&p.filter.frequency, f[i] as f32);
            assert_eq!(p.filter.kind(), FilterKind::Bandpass);
            assert!(p.filter.gain.events().is_empty());
        }
        one(&v.makeup.gain, 8.0);
    }

    fn saw(k: u64) -> [f32; QUANTUM] {
        std::array::from_fn(|f| (((k as usize * QUANTUM + f) % 100) as f32 / 50.0) - 1.0)
    }

    #[test]
    fn five_path_sum_order() {
        let mut v = Vowel::new(&Value::Str("a".into()), 0).unwrap();
        let mut paths = v.paths.clone();
        let mut makeup = v.makeup.clone();
        for k in 0..6 {
            let mut bus = VoiceBus::mono();
            bus.ch[0] = saw(k);
            bus.silent = false;
            v.process(k * 128, &mut bus);
            let mut sum = [0.0f32; QUANTUM];
            for (i, p) in paths.iter_mut().enumerate() {
                let mut c = [saw(k)];
                let s = p
                    .filter
                    .process(k * 128, &mut c, false, &BiquadInputs::NONE);
                p.gain.process(k * 128, &mut c, s, &ParamInputs::NONE);
                for f in 0..QUANTUM {
                    sum[f] = if i == 0 { c[0][f] } else { sum[f] + c[0][f] };
                }
            }
            let mut m = [sum];
            makeup.process(k * 128, &mut m, false, &ParamInputs::NONE);
            assert_eq!(bus.ch[0], m[0], "quantum {k}");
            assert!(!bus.silent);
        }
    }

    #[test]
    fn silent_input_silent_output() {
        // A silent input (whatever its stale samples) gives zeros; the bus turns silent once the
        // biquads' tails (computed at their first update) have passed.
        let mut v = Vowel::new(&Value::Str("i".into()), 0).unwrap();
        let mut k = 0;
        loop {
            let mut bus = VoiceBus::mono();
            bus.ch[0] = [0.5; QUANTUM];
            v.process(k * 128, &mut bus);
            assert!(bus.ch[0].iter().all(|x| *x == 0.0), "quantum {k}");
            if bus.silent {
                break;
            }
            assert!(k < 2000);
            k += 1;
        }
        assert!(
            k > 0,
            "a fresh biquad has an infinite tail until its first update"
        );
        // After a sound, the rings keep the stage sounding until every biquad's tail passed.
        let mut bus = VoiceBus::mono();
        bus.ch[0] = saw(1);
        bus.silent = false;
        v.process(128, &mut bus);
        assert!(!v.is_quiet_after(128));
        let mut k = 2;
        loop {
            let mut bus = VoiceBus::mono();
            v.process(k * 128, &mut bus);
            if v.is_quiet_after(k * 128) {
                let mut after = VoiceBus::mono();
                v.process((k + 1) * 128, &mut after);
                assert!(after.silent, "quiet after {k}");
                break;
            }
            assert!(k < 2000);
            k += 1;
        }
        // A stereo input keeps two channels.
        let mut v = Vowel::new(&Value::Str("e".into()), 0).unwrap();
        let mut bus = VoiceBus::mono();
        bus.channels = 2;
        bus.ch = [saw(0), saw(0)];
        bus.silent = false;
        v.process(0, &mut bus);
        assert_eq!(bus.channels, 2);
        assert_eq!(bus.ch[0], bus.ch[1]);
    }
}

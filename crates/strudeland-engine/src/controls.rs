//! Control parsing (`VoiceParams`), superdough's coercions and defaults that sit outside a node,
//! the final control allowlist and unsupported detection. The audit behind the allowlist is
//! `controls.md` in the crate root; a test keeps the two in step.

use std::collections::BTreeSet;

use crate::chain::FxKey;
use crate::compressor::CompressorParams;
use crate::job::{EngineJob, Value, ValueMap, bar_of};
use crate::js::number_to_string;

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

/// Every `s` the engine accepts (sources, their aliases and the silence markers), sorted.
pub const ALLOWED_SOUNDS: &[&str] = &[
    "-", "_", "brown", "bus", "crackle", "one", "pink", "pulse", "saw", "sawtooth", "sin", "sine",
    "sqr", "square", "supersaw", "tri", "triangle", "user", "white", "~",
];

/// The superdough silence markers.
pub const SILENCE: &[&str] = &["-", "~", "_"];

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

/// Modulator target controls (`CONTROL_TARGETS` keys) that can resolve to an implemented param,
/// sorted. The others skip (`bmod`, `dry`, `busgain`, `stretch`, the `wt`/`warp` LFOs) or throw
/// (`tremolophase`, the filter LFOs' `rate`/`sync`, `wt`, `warp`, `byteBeatStartTime`,
/// `transient`) wherever they are used.
pub const MODULATOR_TARGETS: &[&str] = &[
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

/// An error in a hap's controls (reported as a `hap` render error).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ControlError {
    #[error("not a note: \"{0}\"")]
    NotANote(String),
}

// ---- JS coercions

/// JS `Number(x)` for a decoded value; arrays, objects and sets are NaN (`Unsupported` too: the
/// engine refuses such jobs before reading them).
pub fn js_to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Num(x) => *x,
        Value::Str(s) => parse_js_number(s),
        Value::Arr(_) | Value::Obj(_) | Value::Set(_) | Value::Unsupported(_) => f64::NAN,
    }
}

/// JS `Number(x)` with arrays and objects through their string (`Number([]) = 0`,
/// `Number([5]) = 5`, an object NaN); everything else as [`js_to_number`].
pub fn js_to_number_full(v: &Value) -> f64 {
    match v {
        Value::Arr(_) | Value::Obj(_) => parse_js_number(&js_to_string(v)),
        _ => js_to_number(v),
    }
}

/// JS truthiness.
pub fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Num(x) => *x != 0.0 && !x.is_nan(),
        Value::Str(s) => !s.is_empty(),
        Value::Arr(_) | Value::Obj(_) | Value::Set(_) | Value::Unsupported(_) => true,
    }
}

/// JS `String(x)`.
pub fn js_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(x) => number_to_string(*x),
        Value::Str(s) => s.clone(),
        Value::Arr(xs) => xs
            .iter()
            .map(|x| {
                if matches!(x, Value::Null) {
                    String::new()
                } else {
                    js_to_string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Obj(_) | Value::Unsupported(_) => "[object Object]".into(),
        Value::Set(_) => "[object Set]".into(),
    }
}

/// JS `Math.max` of two numbers (NaN wins, unlike `f64::max`).
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// JS `Math.min` of two numbers (NaN wins).
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn radix_integer(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return f64::NAN;
    }
    match u128::from_str_radix(digits, radix) {
        Ok(n) => n as f64,
        Err(_) => digits.chars().fold(0.0, |acc, c| {
            acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
        }),
    }
}

/// Whether `s` is an ECMAScript `StrUnsignedDecimalLiteral` without `Infinity`: digits with an
/// optional fraction (`1.`, `.5`, `1.5`) and an optional exponent.
fn is_decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let f = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - f;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let e = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e {
            return false;
        }
    }
    i == b.len()
}

/// ECMAScript `StringToNumber`: JS whitespace trimmed, `""` is 0, `0x`/`0o`/`0b` integers,
/// `Infinity` with an optional sign, signed decimal literals; anything else is NaN.
pub fn parse_js_number(s: &str) -> f64 {
    let t = s.trim_matches(is_js_whitespace);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return radix_integer(rest, radix);
        }
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_decimal_literal(body) {
        return f64::NAN;
    }
    body.parse::<f64>().map(|x| sign * x).unwrap_or(f64::NAN)
}

// ---- notes and frequency

/// `noteToMidi` (`superdough/util.mjs:23`): `^([a-gA-G])([#bsf]*)(-?[0-9]*)$`, default octave 3,
/// `(oct + 1) * 12 + chroma + accidentals`.
pub fn note_to_midi(note: &str) -> Result<f64, ControlError> {
    let bad = || ControlError::NotANote(note.to_string());
    let mut chars = note.chars();
    let pc = chars.next().ok_or_else(bad)?;
    let chroma = match pc.to_ascii_lowercase() {
        'c' => 0.0,
        'd' => 2.0,
        'e' => 4.0,
        'f' => 5.0,
        'g' => 7.0,
        'a' => 9.0,
        'b' => 11.0,
        _ => return Err(bad()),
    };
    let rest = chars.as_str();
    let acc_len = rest
        .find(|c| !matches!(c, '#' | 'b' | 's' | 'f'))
        .unwrap_or(rest.len());
    let (acc, oct) = rest.split_at(acc_len);
    let digits = oct.strip_prefix('-').unwrap_or(oct);
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let offset: f64 = acc
        .chars()
        .map(|c| if matches!(c, '#' | 's') { 1.0 } else { -1.0 })
        .sum();
    // `oct ? Number(oct) : undefined`: "" takes the default; "-" is NaN.
    let octave = if oct.is_empty() {
        3.0
    } else {
        parse_js_number(oct)
    };
    Ok((octave + 1.0) * 12.0 + chroma + offset)
}

/// `getFrequencyFromValue` (helpers.mjs:610-622): `note || 36`, string notes through
/// [`note_to_midi`], `freq` from the note when falsy, then `* 2^octave`; `Number(freq)`.
pub fn frequency_from_value(p: &VoiceParams) -> Result<f64, ControlError> {
    let default_note = Value::Num(36.0);
    let note = match &p.note {
        Some(n) if js_truthy(n) => n,
        _ => &default_note,
    };
    let note_num: Option<f64> = match note {
        Value::Str(s) => Some(note_to_midi(s)?),
        Value::Num(x) => Some(*x),
        _ => None,
    };
    let freq = match (&p.freq, note_num) {
        (Some(f), _) if js_truthy(f) => js_to_number(f),
        (_, Some(n)) => libm::pow(2.0, (n - 69.0) / 12.0) * 440.0,
        (Some(f), None) => js_to_number(f),
        (None, None) => f64::NAN,
    };
    let octave = p.octave.unwrap_or(0.0);
    Ok(freq * libm::pow(2.0, octave))
}

// ---- VoiceParams

/// The controls one voice reads, typed per superdough read site. Numeric fields hold JS
/// `Number(x)` (`Null` is 0) with `None` for a missing key; the use site applies the defaults as
/// the JS destructuring does. The keys superdough reads per FX entry are in [`EntryParams`]: one
/// per `value.FX` entry ([`VoiceParams::fx`]) and the hap value's own ([`VoiceParams::main`]);
/// the fields here are the ones superdough reads only from the hap value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceParams {
    /// Raw `s` (the sound after the bank rewrite; `"triangle"` when absent).
    pub s: Option<Value>,
    /// Raw `note` (number or note name; truthiness matters).
    pub note: Option<Value>,
    /// Raw `freq` (truthiness matters).
    pub freq: Option<Value>,
    pub octave: Option<f64>,
    /// Raw `partials ?? n` (synth.mjs:507): a partial count or a list of magnitudes.
    pub partials: Option<Value>,
    /// Raw `phases` (a list of partial phases in turns, read as `phases?.[n] ?? 0`).
    pub phases: Option<Value>,
    /// `Number(vib)` (vibrato rate in Hz; `None` when absent: `undefined > 0` is false).
    pub vib: Option<f64>,
    /// `Number(vibmod)` (vibrato depth in semitones; `None` when absent, default 0.5).
    pub vibmod: Option<f64>,
    /// Whether the pitch envelope is active: `pattack ?? pdecay ?? psustain ?? prelease ?? penv`
    /// is not `undefined` (a `null` `penv` counts).
    pub pitch_env: bool,
    /// `pattack`, `pdecay`, `psustain`, `prelease`: `None` also for `null` (`getADSRValues`).
    pub pattack: Option<f64>,
    pub pdecay: Option<f64>,
    pub psustain: Option<f64>,
    pub prelease: Option<f64>,
    /// `Number(penv)` (`None` when absent; `nanFallback(penv, 1)` at the use site).
    pub penv: Option<f64>,
    /// Raw `pcurve` (indexes `['linear', 'exponential']`).
    pub pcurve: Option<Value>,
    /// `Number(panchor)`, `None` for a missing or `null` value (`panchor ?? psustain`).
    pub panchor: Option<f64>,
    /// `attack`, `decay`, `sustain`, `release`: `None` also for `null` (`getADSRValues` tests
    /// `== null` and uses `??`).
    pub attack: Option<f64>,
    pub decay: Option<f64>,
    pub sustain: Option<f64>,
    pub release: Option<f64>,
    /// `getFilterType(value.ftype) === '24db'`: every filter of every FX entry is two cascaded
    /// biquads (the doubling reads the hap value, superdough.mjs:681).
    pub ftype_24db: bool,
    /// `Number(postgain)`, default 1.
    pub postgain: f64,
    /// The hap duration in seconds (superdough's `value.duration = hapDuration`).
    pub duration: f64,
    /// `Number(noise)` when `noise` is truthy (`if (noise)` builds the noise mix).
    pub noise: Option<f64>,
    /// `Number(density)`, `None` when absent (`getNoiseOscillator`'s default 0.02 applies; a
    /// `null` density is 0).
    pub density: Option<f64>,
    /// The FM matrix (`applyFM`).
    pub fm: FmParams,
    /// `Math.max(release, FXrelease ?? 0)` with `release` destructured at the top of superdough
    /// (default 0.01; `null` is 0): `endWithRelease = end + full_release`.
    pub full_release: f64,
    /// The raw `release` superdough destructures (superdough.mjs:502): `0.01` when absent, `null`
    /// as 0 (`Math.max`, `>` and `+` coerce it), else `Number(release)`.
    pub release_raw: f64,
    /// `Number(FXrelease)`, `None` when absent or `null` (`FXrelease ?? 0` is 0 and `null > x` is
    /// false for every release that matters).
    pub fx_release: Option<f64>,
    /// `Number(djf)` when `djf != null` (the orbit's `djf` worklet value).
    pub djf: Option<f64>,
    /// The supersaw's `unison` (`let { unison = 5 } = value`: a `null` is 0).
    pub unison: f64,
    /// The supersaw's `spread` (default 0.6; a `null` is 0).
    pub spread: f64,
    /// The supersaw's `freqspread`: `Number(detune ?? n ?? 0.18)` (semitones).
    pub freqspread: f64,
    /// The pulse's `pulsewidth`: `Number(pw)`, default 0.5 (`let { pw: pulsewidth = 0.5 }`; a
    /// `null` is 0).
    pub pw: f64,
    /// The pulse's width modulation rate: `Number(pwrate)`, 1 when `pwrate == null` (synth.mjs
    /// 301-312).
    pub pwrate: f64,
    /// The pulse's width modulation depth: `Number(pwsweep)`; when `pwsweep == null`, 0.3 if
    /// `pwrate != null`, else 0. The `pw_lfo` exists when it is `!= 0` (a NaN one too).
    pub pwsweep: f64,
    /// The bus the voice sends `post` to: `bus_key(bus)` when `bus != null` (superdough.mjs:980).
    pub bus: Option<String>,
    /// `Number(busgain)`, default 1 (`applyGainCurve` is the identity; a `null` is 0).
    pub busgain: f64,
    /// The bus the `bus` sound reads: `bus_key(n ?? 0)` (synth.mjs:385).
    pub bus_sound_key: String,
    /// The `value.FX` entries in order (superdough's `FX = [...FX, value]` without the last):
    /// their keys are `FxKey::Index(n)`. Only plain-object entries of an array are read (any other
    /// shape is the `FX` signature, refused before rendering).
    pub fx: Vec<EntryParams>,
    /// The hap value's own entry, the last of the loop (`FxKey::Main`).
    pub main: EntryParams,
}

impl VoiceParams {
    /// The FX entries in superdough's loop order with their keys: the chain entries, then main.
    pub fn entries(&self) -> impl Iterator<Item = (FxKey, &EntryParams)> {
        self.fx
            .iter()
            .enumerate()
            .map(|(n, e)| (FxKey::Index(n), e))
            .chain(std::iter::once((FxKey::Main, &self.main)))
    }
}

/// What superdough reads per FX entry (superdough.mjs:612-938 and the modulator loop): the
/// destructured keys with their `getDefaultValue` defaults, the filter maps, the effects, and the
/// local delay and reverb keys (read for a chain entry only; the main entry's delay and reverb
/// keys are the hap value's, read for the orbit sends). Keys an entry carries outside this set are
/// ignored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntryParams {
    pub gain: Option<f64>,
    pub velocity: Option<f64>,
    /// The lowpass (`cutoff`, `resonance`, `lp*`), present when `cutoff` is defined.
    pub lowpass: Option<FilterParams>,
    /// The highpass (`hcutoff`, `hresonance`, `hp*`), present when `hcutoff` is defined.
    pub highpass: Option<FilterParams>,
    /// The bandpass (`bandf`, `bandq`, `bp*`), present when `bandf` is defined.
    pub bandpass: Option<FilterParams>,
    /// `nanFallback(fanchor, 0)` (the filter maps' `anchor`, shared by the entry's three filter
    /// envelopes).
    pub fanchor: f64,
    /// `model === 'ladder'` (the entry's raw `ftype` is the string `'ladder'`, the filter maps'
    /// `model`): every filter of the entry is one ladder worklet.
    pub ftype_ladder: bool,
    /// `Number(drive)`, default 0.69 (`createFilter`'s `drive = 0.69`; a `null` drive is 0): the
    /// ladder's `drive`.
    pub drive: f64,
    /// `Number(pan)` when `pan` is defined (a `null` pan is 0).
    pub pan: Option<f64>,
    /// `Number(dry)` when `dry != null`: the main entry's `Gain(dry)` to the orbit, a chain
    /// entry's local `dry` gains (`gainNode(fx.dry ?? 1)`).
    pub dry: Option<f64>,
    /// `Number(delay)`, the delay level (default 0; `applyGainCurve` is the identity): the main
    /// entry's orbit send, a chain entry's local wet gain.
    pub delay: f64,
    /// `Number(delaytime ?? delaysync / cps)` (`delaysync` default 3/16 cycles).
    pub delaytime: f64,
    /// Whether `delaytime ?? delaysync / cps` is a JS number. `getDelay`'s guard
    /// `delayTime.value !== delaytime` compares strictly, so a string or boolean `delaytime`
    /// (converted by WebIDL where it is used) always schedules.
    pub delaytime_is_number: bool,
    /// `Number(delayfeedback)` (default 0.5), before it is clamped.
    pub delayfeedback: f64,
    /// `Number(room)`, the reverb level (absent: NaN, so no reverb).
    pub room: f64,
    /// Raw `roomsize`, `roomfade`, `roomlp`, `roomdim` (`None` when absent: `getReverb`'s
    /// `hasChanged` and `generate`'s defaults test `undefined`; `null` and strings are kept).
    pub roomsize: Option<Value>,
    pub roomfade: Option<Value>,
    pub roomlp: Option<Value>,
    pub roomdim: Option<Value>,
    /// The compressor (superdough.mjs:853-864), when `compressor !== undefined`: `getCompressor`'s
    /// values with its `??` defaults.
    pub compressor: Option<CompressorParams>,
    /// Raw `vowel`, when `vowel !== undefined` (a `null` reaches the lookup and throws).
    pub vowel: Option<Value>,
    /// The phaser (superdough.mjs:873-886), when `phaserrate` is defined and `phaserdepth > 0`.
    pub phaser: Option<PhaserParams>,
    /// `Number(coarse)`, `crush`, `shape`, `distort` when defined (`!== undefined`: a `null`
    /// builds the worklet with `.value = 0`).
    pub coarse: Option<f64>,
    pub crush: Option<f64>,
    pub shape: Option<f64>,
    /// `Number(shapevol)`, default 1 (`applyGainCurve` is the identity): the shape's `postgain`.
    pub shapevol: f64,
    pub distort: Option<f64>,
    /// `Number(distortvol)`, default 1: the distortion's `postgain`.
    pub distortvol: f64,
    /// Raw `distorttype`, the distortion's `processorOptions.algorithm` (`None`: the default 0).
    pub distorttype: Option<Value>,
    /// `Number(transient)` when defined (`!== undefined`): the transient's `attack` option.
    pub transient: Option<f64>,
    /// `Number(transsustain)`, `None` when absent (the processor's default 0 applies).
    pub transsustain: Option<f64>,
    /// The tremolo (superdough.mjs:820-851), when `tremolo` (after the `tremolosync` rewrite) is
    /// defined.
    pub tremolo: Option<TremoloParams>,
    /// The entry's `lfo`, `env` and `bmod` values as given (the modulator loop reads them).
    pub modulators: ValueMap,
}

/// One hap's tremolo, as superdough's FX loop reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct TremoloParams {
    /// The LFO's frequency: `cps * Number(tremolosync)` when `tremolosync != null`, else
    /// `Number(tremolo)` (a `null` is 0).
    pub frequency: f64,
    /// `Number(tremolodepth)`, default 1 (`applyGainCurve` is the identity; a `null` is 0, which
    /// is what both `1 - null` and `.value = null` give).
    pub depth: f64,
    /// `tremoloskew ?? (tremoloshape != null ? 0.5 : 1)`.
    pub skew: f64,
    /// Raw `tremoloshape` (`None` when absent: `getLfo`'s default 0).
    pub shape: Option<Value>,
    /// `Number(tremolophase)` (`None` when absent: `getLfo`'s default 0).
    pub phase: Option<f64>,
}

/// One hap's phaser, as `getPhaser(t, endWithRelease, phaserrate, phaserdepth, phasercenter,
/// phasersweep)` reads it: the JS parameter defaults (`frequency = 1`, `centerFrequency = 1000`,
/// `sweep = 2000`) apply to an absent control; a `null` one is treated as absent too (a documented
/// difference: JS passes `null` through a parameter default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaserParams {
    /// `Number(phaserrate)`, 1 for `null`.
    pub rate: f64,
    /// `phaserdepth ?? 0.75` (> 0, else no phaser).
    pub depth: f64,
    /// `Number(phasercenter)`, default 1000.
    pub center: f64,
    /// `Number(phasersweep)`, default 2000.
    pub sweep: f64,
}

fn phaser_params(map: &ValueMap) -> Option<PhaserParams> {
    map.get("phaserrate")?;
    let depth = num_nullish(map, "phaserdepth").unwrap_or(0.75);
    // `phaserdepth > 0` (false for NaN).
    if depth.is_nan() || depth <= 0.0 {
        return None;
    }
    Some(PhaserParams {
        rate: num_nullish(map, "phaserrate").unwrap_or(1.0),
        depth,
        center: num_nullish(map, "phasercenter").unwrap_or(1000.0),
        sweep: num_nullish(map, "phasersweep").unwrap_or(2000.0),
    })
}

/// `getCompressor(ac, compressor, compressorRatio, compressorKnee, compressorAttack,
/// compressorRelease)` when `compressor !== undefined`, with its `??` defaults (`-3`, 10, 10,
/// 0.005, 0.05; a `null` takes the default).
fn compressor_params(map: &ValueMap) -> Option<CompressorParams> {
    map.get("compressor")?;
    Some(CompressorParams {
        threshold: num_nullish(map, "compressor").unwrap_or(-3.0),
        ratio: num_nullish(map, "compressorRatio").unwrap_or(10.0),
        knee: num_nullish(map, "compressorKnee").unwrap_or(10.0),
        attack: num_nullish(map, "compressorAttack").unwrap_or(0.005),
        release: num_nullish(map, "compressorRelease").unwrap_or(0.05),
    })
}

/// The number of FM modulators (`applyFM` loops `i = 1..=8`).
pub const FM_MODULATORS: usize = 8;

/// One entry of the FM matrix whose amount is truthy: modulator `source` (1..=8) into `target`
/// (0 is the carrier's frequency, else modulator `target`'s).
#[derive(Debug, Clone, PartialEq)]
pub struct FmEntry {
    pub source: usize,
    pub target: usize,
    /// `Number(value[control])` (`gainNode(amt)` converts it to a float).
    pub amount: f64,
}

/// One FM modulator's controls, suffix `""` for modulator 1, else its number.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FmModParams {
    /// `Number(fmh ?? 1)` (the harmonicity ratio to the carrier's frequency).
    pub ratio: f64,
    /// Raw `fmwave` (`None` for absent or `null`: `?? 'sine'`).
    pub wave: Option<Value>,
    /// `fmattack`, `fmdecay`, `fmsustain`, `fmrelease`: `None` also for `null`.
    pub adsr: [Option<f64>; 4],
    /// Whether any of the four envelope controls is defined (`!== undefined`; `null` counts).
    pub has_envelope: bool,
    /// `(fmenv ?? 'exp') === 'exp'`: exponential ramps, else linear.
    pub exponential: bool,
}

/// `applyFM`'s inputs: the active matrix entries in loop order and the eight modulators'
/// controls (index 0 is modulator 1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FmParams {
    pub entries: Vec<FmEntry>,
    pub mods: [FmModParams; FM_MODULATORS],
}

/// The control of matrix entry `(i, j)`: `fmi` for (1, 0), `fmi{i}` when `i == j + 1`, else
/// `fmi{i}{j}`.
pub fn fm_control_name(i: usize, j: usize) -> String {
    if i == j + 1 {
        if i == 1 {
            "fmi".to_string()
        } else {
            format!("fmi{i}")
        }
    } else {
        format!("fmi{i}{j}")
    }
}

/// The suffix of modulator `idx`'s controls: `""` for 1, else the number.
pub fn fm_suffix(idx: usize) -> String {
    if idx == 1 {
        String::new()
    } else {
        idx.to_string()
    }
}

fn fm_params(map: &ValueMap) -> FmParams {
    let mut entries = Vec::new();
    for i in 1..=FM_MODULATORS {
        for j in 0..=FM_MODULATORS {
            if let Some(v) = map.get(&fm_control_name(i, j))
                && js_truthy(v)
            {
                entries.push(FmEntry {
                    source: i,
                    target: j,
                    amount: js_to_number(v),
                });
            }
        }
    }
    let mods = std::array::from_fn(|k| {
        let sfx = fm_suffix(k + 1);
        let keys = ["attack", "decay", "sustain", "release"].map(|s| format!("fm{s}{sfx}"));
        FmModParams {
            ratio: match map.get(&format!("fmh{sfx}")) {
                None | Some(Value::Null) => 1.0,
                Some(v) => js_to_number(v),
            },
            wave: match map.get(&format!("fmwave{sfx}")) {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.clone()),
            },
            adsr: [
                num_nullish(map, &keys[0]),
                num_nullish(map, &keys[1]),
                num_nullish(map, &keys[2]),
                num_nullish(map, &keys[3]),
            ],
            has_envelope: keys.iter().any(|k| map.contains_key(k)),
            exponential: match map.get(&format!("fmenv{sfx}")) {
                None | Some(Value::Null) => true,
                Some(Value::Str(s)) => s == "exp",
                Some(_) => false,
            },
        }
    });
    FmParams { entries, mods }
}

/// One filter of `createFilter` (helpers.mjs:234-305) as superdough's maps rename it.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterParams {
    /// `Number(frequency)` (the cutoff control).
    pub frequency: f64,
    /// `Number(q)`, 1 when the Q control is undefined (`q = 1`; a `null` Q is 0).
    pub q: f64,
    /// `[attack, decay, sustain, release]`, `None` also for `null` (`getADSRValues`).
    pub adsr: [Option<f64>; 4],
    /// `nanFallback(env, 1)` (octaves).
    pub env: f64,
    /// Whether any of the four envelope controls or `env` is defined (`!== undefined`; `null`
    /// counts).
    pub has_envelope: bool,
    /// The filter LFO's controls (`rate`, `sync`, `depth`, `depthfrequency`, `shape`, `dcoffset`,
    /// `skew` of the filter's map), raw (`None` when absent).
    pub lfo: FilterLfoParams,
}

/// The raw filter LFO controls of one filter (`lprate`, `lpsync`, `lpdepth`, `lpdepthfrequency`,
/// `lpshape`, `lpdc`, `lpskew` and the `hp`/`bp` ones); `createFilter` reads them as JS values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterLfoParams {
    pub rate: Option<Value>,
    pub sync: Option<Value>,
    pub depth: Option<Value>,
    pub depthfrequency: Option<Value>,
    pub shape: Option<Value>,
    pub dcoffset: Option<Value>,
    pub skew: Option<Value>,
}

/// The keys of one filter: frequency, Q, attack, decay, sustain, release, env.
type FilterKeys = [&'static str; 7];

const LOWPASS_KEYS: FilterKeys = [
    "cutoff",
    "resonance",
    "lpattack",
    "lpdecay",
    "lpsustain",
    "lprelease",
    "lpenv",
];
const HIGHPASS_KEYS: FilterKeys = [
    "hcutoff",
    "hresonance",
    "hpattack",
    "hpdecay",
    "hpsustain",
    "hprelease",
    "hpenv",
];
const BANDPASS_KEYS: FilterKeys = [
    "bandf",
    "bandq",
    "bpattack",
    "bpdecay",
    "bpsustain",
    "bprelease",
    "bpenv",
];

/// `nanFallback(Number(v), fallback)` for an optional control (absent is `undefined`: NaN).
fn number_or(map: &ValueMap, key: &str, fallback: f64) -> f64 {
    crate::adsr::nan_fallback(num(map, key).unwrap_or(f64::NAN), fallback)
}

/// The filter LFO keys of one filter: rate, sync, depth, depthfrequency, shape, dc, skew.
type FilterLfoKeys = [&'static str; 7];

const LOWPASS_LFO_KEYS: FilterLfoKeys = [
    "lprate",
    "lpsync",
    "lpdepth",
    "lpdepthfrequency",
    "lpshape",
    "lpdc",
    "lpskew",
];
const HIGHPASS_LFO_KEYS: FilterLfoKeys = [
    "hprate",
    "hpsync",
    "hpdepth",
    "hpdepthfrequency",
    "hpshape",
    "hpdc",
    "hpskew",
];
const BANDPASS_LFO_KEYS: FilterLfoKeys = [
    "bprate",
    "bpsync",
    "bpdepth",
    "bpdepthfrequency",
    "bpshape",
    "bpdc",
    "bpskew",
];

fn filter_lfo_params(map: &ValueMap, keys: &FilterLfoKeys) -> FilterLfoParams {
    let raw = |k: &str| map.get(k).cloned();
    let [rate, sync, depth, depthfrequency, shape, dcoffset, skew] = *keys;
    FilterLfoParams {
        rate: raw(rate),
        sync: raw(sync),
        depth: raw(depth),
        depthfrequency: raw(depthfrequency),
        shape: raw(shape),
        dcoffset: raw(dcoffset),
        skew: raw(skew),
    }
}

fn filter_params(
    map: &ValueMap,
    keys: &FilterKeys,
    lfo_keys: &FilterLfoKeys,
) -> Option<FilterParams> {
    let [frequency, q, a, d, s, r, env] = *keys;
    let frequency = num(map, frequency)?;
    Some(FilterParams {
        frequency,
        q: num(map, q).unwrap_or(1.0),
        adsr: [
            num_nullish(map, a),
            num_nullish(map, d),
            num_nullish(map, s),
            num_nullish(map, r),
        ],
        env: number_or(map, env, 1.0),
        has_envelope: [a, d, s, r, env].iter().any(|k| map.contains_key(*k)),
        lfo: filter_lfo_params(map, lfo_keys),
    })
}

/// The tremolo of superdough's FX loop: `tremolo = fx.tremolo`, replaced by `cps * tremolosync`
/// when `tremolosync != null`; built when the result is not `undefined`.
fn tremolo_params(map: &ValueMap, cps: f64) -> Option<TremoloParams> {
    let frequency = match num_nullish(map, "tremolosync") {
        Some(sync) => cps * sync,
        None => num(map, "tremolo")?,
    };
    let shape_given = !matches!(map.get("tremoloshape"), None | Some(Value::Null));
    Some(TremoloParams {
        frequency,
        depth: num(map, "tremolodepth").unwrap_or(1.0),
        skew: num_nullish(map, "tremoloskew").unwrap_or(if shape_given { 0.5 } else { 1.0 }),
        shape: map.get("tremoloshape").cloned(),
        phase: num(map, "tremolophase"),
    })
}

/// `getFilterType(ftype) === '24db'` (superdough.mjs:375-379): `ftype ?? 0`; a number `n`
/// indexes `['12db', 'ladder', '24db']` at `floor(((n % 3) + 3) % 3)`; any other value is kept.
pub fn is_24db(ftype: Option<&Value>) -> bool {
    match ftype {
        None | Some(Value::Null) => false,
        Some(Value::Num(n)) => ((n % 3.0) + 3.0) % 3.0 >= 2.0,
        Some(Value::Str(s)) => s == "24db",
        Some(_) => false,
    }
}

fn num(map: &ValueMap, key: &str) -> Option<f64> {
    map.get(key).map(js_to_number)
}

fn num_nullish(map: &ValueMap, key: &str) -> Option<f64> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => Some(js_to_number(v)),
    }
}

/// Reads the controls of one (mutated) control map. `cps` gives the default delay time
/// (`delaysync / cps`). The `FX` entries are read when `FX` is an array (an entry that is not a
/// plain object is skipped: such a value has the `FX` signature and is never rendered).
pub fn parse_voice_params(
    map: &ValueMap,
    hap_duration: f64,
    cps: f64,
) -> Result<VoiceParams, ControlError> {
    // Destructuring defaults apply to `undefined` only; `null` is `Number(null) = 0`.
    let or_default = |key: &str, default: f64| num(map, key).unwrap_or(default);
    let release_raw = or_default("release", 0.01);
    let fx_release = num_nullish(map, "FXrelease");
    let fx = match map.get("FX") {
        Some(Value::Arr(entries)) => entries
            .iter()
            .filter_map(|e| match e {
                Value::Obj(m) => Some(parse_entry_params(m, cps)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(VoiceParams {
        s: map.get("s").cloned(),
        note: map.get("note").cloned(),
        freq: map.get("freq").cloned(),
        octave: num(map, "octave"),
        partials: match map.get("partials") {
            None | Some(Value::Null) => map.get("n").cloned(),
            Some(v) => Some(v.clone()),
        },
        phases: map.get("phases").cloned(),
        vib: num(map, "vib"),
        vibmod: num(map, "vibmod"),
        pitch_env: {
            let present = |k: &str| !matches!(map.get(k), None | Some(Value::Null));
            ["pattack", "pdecay", "psustain", "prelease"]
                .iter()
                .any(|k| present(k))
                || map.contains_key("penv")
        },
        pattack: num_nullish(map, "pattack"),
        pdecay: num_nullish(map, "pdecay"),
        psustain: num_nullish(map, "psustain"),
        prelease: num_nullish(map, "prelease"),
        penv: num(map, "penv"),
        pcurve: map.get("pcurve").cloned(),
        panchor: num_nullish(map, "panchor"),
        attack: num_nullish(map, "attack"),
        decay: num_nullish(map, "decay"),
        sustain: num_nullish(map, "sustain"),
        release: num_nullish(map, "release"),
        ftype_24db: is_24db(map.get("ftype")),
        postgain: num(map, "postgain").unwrap_or(1.0),
        duration: hap_duration,
        noise: map.get("noise").filter(|v| js_truthy(v)).map(js_to_number),
        density: num(map, "density"),
        fm: fm_params(map),
        full_release: js_max(release_raw, fx_release.unwrap_or(0.0)),
        release_raw,
        fx_release,
        djf: num_nullish(map, "djf"),
        unison: or_default("unison", 5.0),
        spread: or_default("spread", 0.6),
        freqspread: num_nullish(map, "detune")
            .or_else(|| num_nullish(map, "n"))
            .unwrap_or(0.18),
        pw: or_default("pw", 0.5),
        pwrate: num_nullish(map, "pwrate").unwrap_or(1.0),
        pwsweep: num_nullish(map, "pwsweep").unwrap_or(
            if matches!(map.get("pwrate"), None | Some(Value::Null)) {
                0.0
            } else {
                0.3
            },
        ),
        bus: crate::bus::send_key(map),
        busgain: or_default("busgain", 1.0),
        bus_sound_key: crate::bus::sound_read_key(map),
        fx,
        main: parse_entry_params(map, cps),
    })
}

/// The keys superdough reads per FX entry (superdough.mjs:612-938), from one entry's map (a
/// `value.FX` element, or the hap value itself for the main entry), with the destructuring
/// defaults of the FX loop (`getDefaultValue`: `delay` 0, `delayfeedback` 0.5, `delaysync` 3/16,
/// `shapevol` and `distortvol` 1, ...; they apply to `undefined` only). `cps` gives the default
/// delay time (`delaysync / cps`).
pub fn parse_entry_params(map: &ValueMap, cps: f64) -> EntryParams {
    let or_default = |key: &str, default: f64| num(map, key).unwrap_or(default);
    EntryParams {
        gain: num(map, "gain"),
        velocity: num(map, "velocity"),
        lowpass: filter_params(map, &LOWPASS_KEYS, &LOWPASS_LFO_KEYS),
        highpass: filter_params(map, &HIGHPASS_KEYS, &HIGHPASS_LFO_KEYS),
        bandpass: filter_params(map, &BANDPASS_KEYS, &BANDPASS_LFO_KEYS),
        fanchor: number_or(map, "fanchor", 0.0),
        ftype_ladder: matches!(map.get("ftype"), Some(Value::Str(s)) if s == "ladder"),
        drive: or_default("drive", 0.69),
        pan: num(map, "pan"),
        dry: num_nullish(map, "dry"),
        delay: or_default("delay", 0.0),
        delaytime: match map.get("delaytime") {
            None | Some(Value::Null) => or_default("delaysync", 3.0 / 16.0) / cps,
            Some(v) => js_to_number(v),
        },
        delaytime_is_number: matches!(
            map.get("delaytime"),
            None | Some(Value::Null) | Some(Value::Num(_))
        ),
        delayfeedback: or_default("delayfeedback", 0.5),
        // `room > 0` and `gainNode(room)` convert an array or object through its string.
        room: map.get("room").map_or(f64::NAN, js_to_number_full),
        roomsize: map.get("roomsize").cloned(),
        roomfade: map.get("roomfade").cloned(),
        roomlp: map.get("roomlp").cloned(),
        roomdim: map.get("roomdim").cloned(),
        compressor: compressor_params(map),
        vowel: map.get("vowel").cloned(),
        phaser: phaser_params(map),
        coarse: num(map, "coarse"),
        crush: num(map, "crush"),
        shape: num(map, "shape"),
        shapevol: or_default("shapevol", 1.0),
        distort: num(map, "distort"),
        distortvol: or_default("distortvol", 1.0),
        distorttype: map.get("distorttype").cloned(),
        transient: num(map, "transient"),
        transsustain: num(map, "transsustain"),
        tremolo: tremolo_params(map, cps),
        modulators: ["lfo", "env", "bmod"]
            .iter()
            .filter_map(|k| map.get(*k).map(|v| ((*k).to_string(), v.clone())))
            .collect(),
    }
}

/// Property names an array inherits (`Array.prototype` and `Object.prototype`): indexing
/// `['linear', 'exponential']` with one gives a defined value that is not `'exponential'`.
const ARRAY_PROPERTIES: &[&str] = &[
    "length",
    "constructor",
    "at",
    "concat",
    "copyWithin",
    "entries",
    "every",
    "fill",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "map",
    "pop",
    "push",
    "reduce",
    "reduceRight",
    "reverse",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "toLocaleString",
    "toReversed",
    "toSorted",
    "toSpliced",
    "toString",
    "unshift",
    "values",
    "with",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "valueOf",
    "__proto__",
];

/// The pitch envelope's curve: `getParamADSR(.., curves[pcurve ?? 0])` with `curves =
/// ['linear', 'exponential']`. Index `"1"` is exponential, `"0"` linear; any other key gives
/// `undefined` (getParamADSR's default, exponential) except inherited array properties, which
/// are defined values other than `'exponential'` (linear ramps).
pub fn pitch_curve(pcurve: Option<&Value>) -> crate::adsr::Curve {
    use crate::adsr::Curve;
    let key = match pcurve {
        None | Some(Value::Null) => "0".to_string(),
        Some(v) => js_to_string(v),
    };
    match key.as_str() {
        "1" => Curve::Exponential,
        "0" => Curve::Linear,
        k if ARRAY_PROPERTIES.contains(&k) => Curve::Linear,
        _ => Curve::Exponential,
    }
}

/// An FX entry's gain stage, `gain_curve(nanFallback(gain, 1)) * gain_curve(velocity)` with
/// superdough's defaults (gain 0.8, velocity 1; the gain curve is the identity), in f64.
pub fn gain_stage(p: &EntryParams) -> f64 {
    let gain = match p.gain {
        None => 0.8,
        Some(g) if g.is_nan() => 1.0,
        Some(g) => g,
    };
    gain * p.velocity.unwrap_or(1.0)
}

// ---- the superdough mutation

/// The sound of a control map: `s` with the `'triangle'` default.
pub fn sound_of(map: &ValueMap) -> Value {
    map.get("s")
        .cloned()
        .unwrap_or_else(|| Value::Str("triangle".into()))
}

fn is_silence(s: &Value) -> bool {
    matches!(s, Value::Str(x) if SILENCE.contains(&x.as_str()))
}

/// Superdough's mutation of a hap value before `voiceSeed` (superdough.mjs:483-566):
/// `duration := hapDuration`; a silence marker returns `None`; with a truthy `bank` and a
/// truthy `s`, `s := String(bank) + '_' + String(s)` is written into the map.
pub fn prepare_value(map: &ValueMap, hap_duration: f64) -> Option<ValueMap> {
    let s = sound_of(map);
    if is_silence(&s) {
        return None;
    }
    let mut out = map.clone();
    out.insert("duration".into(), Value::Num(hap_duration));
    if let Some(bank) = map.get("bank")
        && js_truthy(bank)
        && js_truthy(&s)
    {
        out.insert(
            "s".into(),
            Value::Str(format!("{}_{}", js_to_string(bank), js_to_string(&s))),
        );
    }
    Some(out)
}

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
    let sound_ok = ALLOWED_SOUNDS.binary_search(&s.as_str()).is_ok();
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
    // A value map gives the same signature for every hap that plays it: only its first hap counts.
    let mut seen = vec![false; job.values.len()];
    for hap in job.pauses.iter().flat_map(|p| &p.haps) {
        let Some(map) = job.values.get(hap.v as usize) else {
            continue;
        };
        if std::mem::replace(&mut seen[hap.v as usize], true) {
            continue;
        }
        let bus_cycle = cyclic.get(hap.v as usize).copied().unwrap_or(false);
        let Some((s, keys)) = signature_with(map, bus_cycle, job.cps) else {
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
        to_json(ALLOWED_SOUNDS),
        to_json(CHAIN_KEYS),
        modulator_keys,
        to_json(MODULATOR_TARGETS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::EngineJob;
    use serde_json::json;

    fn map(v: serde_json::Value) -> ValueMap {
        serde_json::from_value(v).unwrap()
    }

    fn params(v: serde_json::Value) -> VoiceParams {
        parse_voice_params(&map(v), 0.5, 0.5).unwrap()
    }

    #[test]
    fn entry_params_defaults_and_the_main_entry() {
        // Every default of an empty entry (superdough's FX loop destructuring and the maps).
        let e = parse_entry_params(&map(json!({})), 0.5);
        assert_eq!((e.gain, e.velocity), (None, None));
        assert_eq!(gain_stage(&e), 0.8);
        assert_eq!(
            (e.lowpass.clone(), e.highpass.clone(), e.bandpass.clone()),
            (None, None, None)
        );
        assert_eq!((e.fanchor, e.ftype_ladder, e.drive), (0.0, false, 0.69));
        assert_eq!((e.pan, e.dry), (None, None));
        assert_eq!((e.delay, e.delayfeedback), (0.0, 0.5));
        assert_eq!(e.delaytime, 0.375, "delaysync 3/16 / cps 0.5");
        assert!(e.delaytime_is_number);
        assert!(e.room.is_nan(), "absent room: room > 0 is false");
        assert_eq!(
            (&e.roomsize, &e.roomfade, &e.roomlp, &e.roomdim),
            (&None, &None, &None, &None)
        );
        assert_eq!(
            (e.shapevol, e.distortvol, e.distorttype.clone()),
            (1.0, 1.0, None)
        );
        assert_eq!(
            (e.coarse, e.crush, e.shape, e.distort),
            (None, None, None, None)
        );
        assert_eq!((e.transient, e.transsustain), (None, None));
        assert!(e.tremolo.is_none() && e.compressor.is_none() && e.phaser.is_none());
        assert!(e.vowel.is_none() && e.modulators.is_empty());
        // The main entry is the hap value read the same way.
        let v = json!({
            "s": "sawtooth", "gain": 0.5, "velocity": 0.7, "cutoff": 800, "fanchor": 0.2,
            "ftype": "ladder", "drive": 2, "pan": 0.3, "dry": 0.4, "delay": 0.6,
            "delaytime": "0.2", "delayfeedback": 0.3, "room": 0.2, "roomsize": 3, "shape": 0.5,
            "tremolo": 2, "lfo": { "__ids": { "$set": [0] }, "0": { "control": "gain" } }
        });
        let p = params(v.clone());
        assert_eq!(p.main, parse_entry_params(&map(v), 0.5));
        assert!(p.fx.is_empty());
        assert!(!p.main.delaytime_is_number);
        assert_eq!(p.main.modulators.len(), 1);
    }

    #[test]
    fn fx_entries_are_read_in_order() {
        let p = params(json!({
            "s": "sine", "gain": 0.3, "ftype": "24db",
            "FX": [{ "cutoff": 500 }, 3, null, { "gain": 1, "delay": 0.5, "s": "bytebeat" }]
        }));
        // The non-object entries are skipped (the value has the `FX` signature).
        assert_eq!(p.fx.len(), 2);
        assert_eq!(p.fx[0].lowpass.as_ref().map(|f| f.frequency), Some(500.0));
        assert_eq!(
            (p.fx[0].gain, p.fx[1].gain, p.main.gain),
            (None, Some(1.0), Some(0.3))
        );
        assert_eq!(p.fx[1].delay, 0.5);
        assert!(p.ftype_24db, "the doubling is the hap's");
        let keys: Vec<FxKey> = p.entries().map(|(k, _)| k).collect();
        assert_eq!(keys, [FxKey::Index(0), FxKey::Index(1), FxKey::Main]);
        assert!(params(json!({ "FX": 3 })).fx.is_empty());
        assert!(params(json!({ "FX": [] })).fx.is_empty());
        // The filter maps' `anchor` and `drive` and the ladder `model` are the entry's own.
        let p = params(json!({
            "fanchor": 0.5, "drive": 3, "ftype": "ladder",
            "FX": [{ "fanchor": 0.25, "cutoff": 500 }]
        }));
        assert_eq!((p.fx[0].fanchor, p.main.fanchor), (0.25, 0.5));
        assert_eq!((p.fx[0].drive, p.main.drive), (0.69, 3.0));
        assert_eq!((p.fx[0].ftype_ladder, p.main.ftype_ladder), (false, true));
        // An entry's own FX is not read.
        assert_eq!(
            params(json!({ "FX": [{ "FX": [{ "gain": 2 }] }] })).fx[0].gain,
            None
        );
    }

    #[test]
    fn room_reads_arrays_through_their_string() {
        assert_eq!(js_to_number_full(&Value::Arr(vec![Value::Num(5.0)])), 5.0);
        assert_eq!(params(json!({ "room": [0.5] })).main.room, 0.5);
        assert_eq!(params(json!({ "room": "0.25" })).main.room, 0.25);
        assert_eq!(params(json!({ "room": null })).main.room, 0.0);
        assert!(params(json!({ "room": [1, 2] })).main.room.is_nan());
        assert!(params(json!({ "room": {} })).main.room.is_nan());
    }

    #[test]
    fn worklet_controls_and_their_defaults() {
        let p = params(json!({}));
        assert_eq!(
            (p.main.coarse, p.main.crush, p.main.shape, p.main.distort),
            (None, None, None, None)
        );
        assert_eq!((p.main.shapevol, p.main.distortvol), (1.0, 1.0));
        assert_eq!(p.main.distorttype, None);
        assert_eq!((p.main.transient, p.main.transsustain), (None, None));
        assert_eq!(p.full_release, 0.01, "release defaults to 0.01");
        let p = params(json!({
            "coarse": null, "crush": "4", "shape": true, "shapevol": null, "distort": 2,
            "distortvol": "0.4", "distorttype": "fold", "transient": null, "transsustain": -0.8,
            "release": null
        }));
        assert_eq!(p.main.coarse, Some(0.0), "null is defined: Number(null)");
        assert_eq!(p.main.crush, Some(4.0));
        assert_eq!(p.main.shape, Some(1.0));
        assert_eq!(
            p.main.shapevol, 0.0,
            "the default applies to undefined only"
        );
        assert_eq!((p.main.distort, p.main.distortvol), (Some(2.0), 0.4));
        assert_eq!(p.main.distorttype, Some(Value::Str("fold".into())));
        assert_eq!(
            (p.main.transient, p.main.transsustain),
            (Some(0.0), Some(-0.8))
        );
        assert_eq!(p.full_release, 0.0, "Math.max(null, 0)");
        // Math.max(release, 0): negative releases do not shorten endWithRelease; NaN stays.
        assert_eq!(params(json!({ "release": -1 })).full_release, 0.0);
        assert_eq!(params(json!({ "release": "0.3" })).full_release, 0.3);
        assert!(params(json!({ "release": "x" })).full_release.is_nan());
    }

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

    #[test]
    fn send_controls_and_their_defaults() {
        let p = params(json!({}));
        assert_eq!((p.main.delay, p.main.delayfeedback), (0.0, 0.5));
        assert_eq!(p.main.delaytime, 0.375, "delaysync 3/16 / cps 0.5");
        assert!(p.main.room.is_nan(), "no room: room > 0 is false");
        let p =
            params(json!({ "delay": "0.5", "delaytime": 0.2, "delayfeedback": 0.7, "room": 0.3 }));
        assert_eq!(
            (
                p.main.delay,
                p.main.delaytime,
                p.main.delayfeedback,
                p.main.room
            ),
            (0.5, 0.2, 0.7, 0.3)
        );
        // `delaytime ?? delaysync / cps`; null delaysync is 0, null levels are Number(null) = 0.
        let m = map(json!({ "delaysync": 0.25 }));
        assert_eq!(
            parse_voice_params(&m, 0.5, 2.0).unwrap().main.delaytime,
            0.125
        );
        // The guard compares the raw `delaytime` strictly: only numbers can equal `.value`.
        assert!(params(json!({})).main.delaytime_is_number);
        assert!(
            params(json!({ "delaytime": 0.25 }))
                .main
                .delaytime_is_number
        );
        assert!(
            params(json!({ "delaytime": null, "delaysync": "0.5" }))
                .main
                .delaytime_is_number
        );
        let p = params(json!({ "delaytime": "0.25" }));
        assert_eq!(
            (p.main.delaytime, p.main.delaytime_is_number),
            (0.25, false)
        );
        assert!(
            !params(json!({ "delaytime": true }))
                .main
                .delaytime_is_number
        );
        let p = params(
            json!({ "delaytime": null, "delaysync": null, "delay": null, "delayfeedback": null, "room": null }),
        );
        assert_eq!(
            (
                p.main.delay,
                p.main.delaytime,
                p.main.delayfeedback,
                p.main.room
            ),
            (0.0, 0.0, 0.0, 0.0)
        );
        let p = params(json!({ "delaytime": "x" }));
        assert!(p.main.delaytime.is_nan());
    }

    #[test]
    fn room_settings_stay_raw() {
        let p = params(json!({ "room": 0.5 }));
        assert_eq!(
            (
                &p.main.roomsize,
                &p.main.roomfade,
                &p.main.roomlp,
                &p.main.roomdim
            ),
            (&None, &None, &None, &None),
            "absent is undefined"
        );
        let p = params(
            json!({ "roomsize": 4, "roomfade": null, "roomlp": "800", "roomdim": [1, 2], "size": 9 }),
        );
        assert_eq!(p.main.roomsize, Some(Value::Num(4.0)));
        assert_eq!(p.main.roomfade, Some(Value::Null));
        assert_eq!(p.main.roomlp, Some(Value::Str("800".into())));
        assert_eq!(
            p.main.roomdim,
            Some(Value::Arr(vec![Value::Num(1.0), Value::Num(2.0)]))
        );
    }

    /// `getFrequencyFromValue` (helpers.mjs) vs `frequency.json`: within 1 ulp (JSC's `Math.pow`
    /// may differ from V8's in the last bit), errors by message.
    #[test]
    fn frequency_matches_the_js_vectors() {
        use crate::test_vectors::{decode, load, num, within_ulps};
        let v = load("frequency.json");
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 30);
        let mut errors = 0;
        for c in cases {
            let Value::Obj(m) = decode(&c["value"]) else {
                panic!("{c}: value is not an object");
            };
            let got = frequency_from_value(&parse_voice_params(&m, 0.5, 0.5).unwrap());
            match c.get("error") {
                Some(e) => {
                    errors += 1;
                    assert_eq!(got.unwrap_err().to_string(), e.as_str().unwrap(), "{c}");
                }
                None => {
                    let want = num(&c["freq"]);
                    let got = got.unwrap();
                    assert!(within_ulps(got, want, 1), "{c}: {got} vs {want}");
                }
            }
        }
        assert!(errors >= 1);
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn js_number_coercion() {
        assert_eq!(parse_js_number(""), 0.0);
        assert_eq!(parse_js_number("  \t\n"), 0.0);
        assert_eq!(parse_js_number(" 1.5 "), 1.5);
        assert_eq!(parse_js_number("\u{a0}2\u{feff}"), 2.0);
        assert_eq!(parse_js_number("-3e2"), -300.0);
        assert_eq!(parse_js_number("+.5"), 0.5);
        assert_eq!(parse_js_number("1."), 1.0);
        assert_eq!(parse_js_number("1E-2"), 0.01);
        assert_eq!(parse_js_number("0x1F"), 31.0);
        assert_eq!(parse_js_number("0o17"), 15.0);
        assert_eq!(parse_js_number("0B101"), 5.0);
        assert_eq!(parse_js_number("Infinity"), f64::INFINITY);
        assert_eq!(parse_js_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(parse_js_number("+Infinity"), f64::INFINITY);
        assert_eq!(
            parse_js_number("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            2f64.powi(136)
        );
        for bad in [
            ".", "e5", "1e", "1e+", "-0x10", "0x", "0xg", "inf", "NaN", "infinity", "1_000", "1 2",
            "--1", "abc", "0b2", "1.2.3",
        ] {
            assert!(parse_js_number(bad).is_nan(), "{bad}");
        }
        assert!(parse_js_number("-0").is_sign_negative());
    }

    #[test]
    fn value_coercions() {
        assert_eq!(js_to_number(&Value::Null), 0.0);
        assert_eq!(js_to_number(&Value::Bool(true)), 1.0);
        assert_eq!(js_to_number(&Value::Str("7".into())), 7.0);
        assert!(js_to_number(&Value::Arr(vec![])).is_nan());
        assert!(js_to_number(&Value::Obj(Default::default())).is_nan());
        assert!(!js_truthy(&Value::Num(f64::NAN)));
        assert!(!js_truthy(&Value::Num(-0.0)));
        assert!(!js_truthy(&Value::Str(String::new())));
        assert!(js_truthy(&Value::Str("0".into())));
        assert!(js_truthy(&Value::Arr(vec![])));
        assert_eq!(js_to_string(&Value::Num(1e21)), "1e+21");
        assert_eq!(js_to_string(&Value::Bool(false)), "false");
        assert_eq!(js_to_string(&Value::Null), "null");
        let arr = Value::Arr(vec![
            Value::Num(1.0),
            Value::Null,
            Value::Arr(vec![Value::Str("a".into())]),
        ]);
        assert_eq!(js_to_string(&arr), "1,,a");
        assert_eq!(
            js_to_string(&Value::Obj(Default::default())),
            "[object Object]"
        );
        assert!(js_max(f64::NAN, 1.0).is_nan());
        assert!(js_min(1.0, f64::NAN).is_nan());
        assert_eq!(js_max(0.0, 0.001), 0.001);
        assert_eq!(js_min(2.0, 1.0), 1.0);
    }

    #[test]
    fn note_to_midi_forms() {
        let m = |s: &str| note_to_midi(s).unwrap();
        assert_eq!(m("c3"), 48.0);
        assert_eq!(m("a4"), 69.0);
        assert_eq!(m("C#4"), 61.0);
        assert_eq!(m("eb3"), 51.0);
        assert_eq!(m("bb"), 58.0);
        assert_eq!(m("fs2"), 42.0);
        assert_eq!(m("gf2"), 42.0);
        assert_eq!(m("c##4"), 62.0);
        assert_eq!(m("c-1"), 0.0);
        assert_eq!(m("c"), 48.0);
        assert_eq!(m("A007"), 105.0);
        assert!(m("c-").is_nan());
        for bad in ["", "h3", "c3x", "3c", "c 3", "c--1", "cb#x"] {
            assert_eq!(
                note_to_midi(bad),
                Err(ControlError::NotANote(bad.into())),
                "{bad}"
            );
        }
        assert_eq!(
            ControlError::NotANote("x".into()).to_string(),
            "not a note: \"x\""
        );
    }

    #[test]
    fn frequency_rules() {
        let f = |v: serde_json::Value| frequency_from_value(&params(v)).unwrap();
        assert!(close(f(json!({})), 65.40639132514966), "note || 36");
        assert!(close(f(json!({ "note": 0 })), 65.40639132514966));
        assert!(close(f(json!({ "note": "" })), 65.40639132514966));
        assert!(close(f(json!({ "note": null })), 65.40639132514966));
        assert!(close(
            f(json!({ "note": { "$num": "NaN" } })),
            65.40639132514966
        ));
        assert_eq!(f(json!({ "note": 69 })), 440.0);
        assert_eq!(f(json!({ "note": "a4" })), 440.0);
        assert_eq!(f(json!({ "note": 69, "octave": 1 })), 880.0);
        assert_eq!(f(json!({ "freq": 100, "note": 69 })), 100.0);
        assert_eq!(f(json!({ "freq": 100, "octave": -1 })), 50.0);
        assert_eq!(f(json!({ "freq": "300" })), 300.0);
        assert_eq!(
            f(json!({ "freq": 0, "note": 69 })),
            440.0,
            "falsy freq comes from the note"
        );
        assert_eq!(f(json!({ "note": 69, "octave": null })), 440.0);
        assert!(
            f(json!({ "note": true })).is_nan(),
            "a non-string, non-number note"
        );
        assert!(f(json!({ "freq": "abc" })).is_nan());
        assert!(f(json!({ "note": 69, "octave": "x" })).is_nan());
        assert_eq!(
            frequency_from_value(&params(json!({ "note": "q" }))),
            Err(ControlError::NotANote("q".into()))
        );
    }

    #[test]
    fn voice_params_and_defaults() {
        let p = params(json!({ "attack": null, "decay": "0.2", "sustain": true, "gain": null }));
        assert_eq!(p.attack, None);
        assert_eq!(p.decay, Some(0.2));
        assert_eq!(p.sustain, Some(1.0));
        assert_eq!(p.release, None);
        assert_eq!(p.main.gain, Some(0.0));
        assert_eq!(p.duration, 0.5);
        assert_eq!(
            gain_stage(&p.main),
            0.0,
            "null gain is 0 (nanFallback keeps it)"
        );
        assert_eq!(gain_stage(&params(json!({})).main), 0.8);
        assert_eq!(
            gain_stage(&params(json!({ "gain": "abc" })).main),
            1.0,
            "NaN gain falls back to 1"
        );
        assert_eq!(
            gain_stage(&params(json!({ "gain": 0.5, "velocity": 0.5 })).main),
            0.25
        );
        assert!(
            gain_stage(&params(json!({ "velocity": "x" })).main).is_nan(),
            "velocity has no fallback"
        );
        assert_eq!(gain_stage(&params(json!({ "velocity": null })).main), 0.0);
    }

    #[test]
    fn ladder_drive_and_djf_params() {
        // `model === 'ladder'` only for the string 'ladder'; never 24 dB at once.
        let p = params(json!({}));
        assert!(!p.main.ftype_ladder);
        assert_eq!(p.main.drive, 0.69);
        assert_eq!(p.djf, None);
        let p = params(json!({ "ftype": "ladder" }));
        assert!(p.main.ftype_ladder && !p.ftype_24db);
        for f in [
            json!(1),
            json!("Ladder"),
            json!(["ladder"]),
            json!(null),
            json!("24db"),
        ] {
            assert!(!params(json!({ "ftype": f })).main.ftype_ladder, "{f}");
        }
        // drive: default 0.69 for undefined only; null is 0; strings convert.
        assert_eq!(params(json!({ "drive": null })).main.drive, 0.0);
        assert_eq!(params(json!({ "drive": "3" })).main.drive, 3.0);
        assert!(params(json!({ "drive": "x" })).main.drive.is_nan());
        // djf: `djf != null`.
        assert_eq!(params(json!({ "djf": null })).djf, None);
        assert_eq!(params(json!({ "djf": 0.2 })).djf, Some(0.2));
        assert_eq!(params(json!({ "djf": "0.8" })).djf, Some(0.8));
        assert_eq!(params(json!({ "djf": true })).djf, Some(1.0));
        assert!(params(json!({ "djf": "x" })).djf.unwrap().is_nan());
    }

    #[test]
    fn filter_params() {
        let p = params(json!({}));
        assert_eq!(
            (
                p.main.lowpass.clone(),
                p.main.highpass.clone(),
                p.main.bandpass.clone()
            ),
            (None, None, None)
        );
        assert_eq!(p.main.fanchor, 0.0);
        assert!(!p.ftype_24db);
        let p =
            params(json!({ "cutoff": 800, "resonance": 5, "lpattack": 0.1, "lpsustain": null }));
        let lp = p.main.lowpass.unwrap();
        assert_eq!(
            lp,
            FilterParams {
                frequency: 800.0,
                q: 5.0,
                adsr: [Some(0.1), None, None, None],
                env: 1.0,
                has_envelope: true,
                lfo: FilterLfoParams::default(),
            }
        );
        // Q undefined -> 1, null -> 0; frequency null -> 0 (still a filter).
        let p = params(json!({ "hcutoff": null, "hresonance": null, "bandf": "250" }));
        let hp = p.main.highpass.unwrap();
        assert_eq!((hp.frequency, hp.q, hp.has_envelope), (0.0, 0.0, false));
        let bp = p.main.bandpass.unwrap();
        assert_eq!((bp.frequency, bp.q), (250.0, 1.0));
        // env: nanFallback(env, 1); null is 0; a null env still enables the envelope.
        let p = params(
            json!({ "cutoff": 1, "lpenv": null, "hcutoff": 1, "hpenv": "x",
                               "bandf": 1, "bpenv": -2 }),
        );
        let (lp, hp, bp) = (
            p.main.lowpass.unwrap(),
            p.main.highpass.unwrap(),
            p.main.bandpass.unwrap(),
        );
        assert_eq!((lp.env, lp.has_envelope), (0.0, true));
        assert_eq!((hp.env, hp.has_envelope), (1.0, true));
        assert_eq!((bp.env, bp.has_envelope), (-2.0, true));
        // Each filter reads its own keys only.
        let p = params(json!({ "cutoff": 1, "hpattack": 0.3, "bpdecay": 0.2 }));
        assert!(!p.main.lowpass.unwrap().has_envelope);
        assert!(p.main.highpass.is_none() && p.main.bandpass.is_none());
        // fanchor: nanFallback(.., 0).
        assert_eq!(params(json!({ "fanchor": 0.25 })).main.fanchor, 0.25);
        assert_eq!(params(json!({ "fanchor": "x" })).main.fanchor, 0.0);
        assert_eq!(params(json!({ "fanchor": null })).main.fanchor, 0.0);
    }

    #[test]
    fn filter_types() {
        assert!(!is_24db(None));
        assert!(!is_24db(Some(&Value::Null)));
        for (n, want) in [
            (0.0, false),
            (1.0, false),
            (2.0, true),
            (2.99, true),
            (3.0, false),
            (5.0, true),
            (-1.0, true),
            (-0.5, true),
            (-2.0, false),
            (f64::NAN, false),
            (f64::INFINITY, false),
        ] {
            assert_eq!(is_24db(Some(&Value::Num(n))), want, "{n}");
        }
        assert!(is_24db(Some(&Value::Str("24db".into()))));
        assert!(!is_24db(Some(&Value::Str("12db".into()))));
        assert!(!is_24db(Some(&Value::Str("2".into()))));
        assert!(!is_24db(Some(&Value::Bool(true))));
        assert!(params(json!({ "ftype": "24db" })).ftype_24db);
    }

    #[test]
    fn pan_and_level_params() {
        let p = params(json!({}));
        assert_eq!((p.main.pan, p.postgain, p.main.dry), (None, 1.0, None));
        let p = params(json!({ "pan": null, "postgain": null, "dry": null }));
        assert_eq!((p.main.pan, p.postgain, p.main.dry), (Some(0.0), 0.0, None));
        let p = params(json!({ "pan": "0.25", "postgain": 2, "dry": 0 }));
        assert_eq!(
            (p.main.pan, p.postgain, p.main.dry),
            (Some(0.25), 2.0, Some(0.0))
        );
        assert!(params(json!({ "dry": "x" })).main.dry.unwrap().is_nan());
    }

    #[test]
    fn oscillator_params() {
        // `partials ?? n`: a nullish partials falls back to n.
        assert_eq!(params(json!({ "n": 3 })).partials, Some(Value::Num(3.0)));
        assert_eq!(
            params(json!({ "partials": null, "n": 3 })).partials,
            Some(Value::Num(3.0))
        );
        assert_eq!(
            params(json!({ "partials": [1, 0.5], "n": 3 })).partials,
            Some(Value::Arr(vec![Value::Num(1.0), Value::Num(0.5)]))
        );
        assert_eq!(
            params(json!({ "partials": 0, "n": 3 })).partials,
            Some(Value::Num(0.0))
        );
        assert_eq!(params(json!({})).partials, None);
        assert_eq!(
            params(json!({ "phases": [0.5] })).phases,
            Some(Value::Arr(vec![Value::Num(0.5)]))
        );
        // vib and vibmod: Number(), null is 0, absent None.
        let p = params(json!({ "vib": "5", "vibmod": null }));
        assert_eq!((p.vib, p.vibmod), (Some(5.0), Some(0.0)));
        let p = params(json!({}));
        assert_eq!((p.vib, p.vibmod), (None, None));
        // Pitch envelope activation: any p-control that is not nullish, or penv present.
        assert!(!params(json!({})).pitch_env);
        assert!(!params(json!({ "pattack": null, "prelease": null })).pitch_env);
        assert!(params(json!({ "penv": null })).pitch_env);
        assert!(params(json!({ "pdecay": 0 })).pitch_env);
        assert!(params(json!({ "psustain": "x" })).pitch_env);
        assert!(!params(json!({ "pcurve": 1, "panchor": 0 })).pitch_env);
        let p = params(json!({ "pattack": null, "pdecay": "0.1", "penv": "x", "panchor": null }));
        assert_eq!((p.pattack, p.pdecay, p.panchor), (None, Some(0.1), None));
        assert!(p.penv.unwrap().is_nan());
        assert_eq!(params(json!({ "panchor": "0.25" })).panchor, Some(0.25));
    }

    #[test]
    fn pitch_curve_indexes_linear_exponential() {
        use crate::adsr::Curve::*;
        assert_eq!(pitch_curve(None), Linear);
        assert_eq!(pitch_curve(Some(&Value::Null)), Linear);
        assert_eq!(pitch_curve(Some(&Value::Num(0.0))), Linear);
        assert_eq!(pitch_curve(Some(&Value::Num(-0.0))), Linear);
        assert_eq!(pitch_curve(Some(&Value::Str("0".into()))), Linear);
        assert_eq!(pitch_curve(Some(&Value::Num(1.0))), Exponential);
        assert_eq!(pitch_curve(Some(&Value::Str("1".into()))), Exponential);
        assert_eq!(
            pitch_curve(Some(&Value::Arr(vec![Value::Num(1.0)]))),
            Exponential
        );
        // Outside {0, 1}: undefined, getParamADSR's default (exponential).
        assert_eq!(pitch_curve(Some(&Value::Num(2.0))), Exponential);
        assert_eq!(pitch_curve(Some(&Value::Num(0.5))), Exponential);
        assert_eq!(pitch_curve(Some(&Value::Bool(true))), Exponential);
        assert_eq!(pitch_curve(Some(&Value::Str("01".into()))), Exponential);
        // Inherited array properties are defined values other than 'exponential'.
        assert_eq!(pitch_curve(Some(&Value::Str("length".into()))), Linear);
        assert_eq!(pitch_curve(Some(&Value::Str("map".into()))), Linear);
    }

    #[test]
    fn superdough_mutation() {
        assert_eq!(prepare_value(&map(json!({ "s": "~" })), 1.0), None);
        assert_eq!(
            prepare_value(&map(json!({ "s": "-", "bank": "x" })), 1.0),
            None
        );
        let v = prepare_value(&map(json!({ "s": 5 })), 1.0).unwrap();
        assert_eq!(v["s"], Value::Num(5.0), "a non-string s is never silence");
        let v = prepare_value(&map(json!({ "note": 1, "duration": 9 })), 0.25).unwrap();
        assert_eq!(v["duration"], Value::Num(0.25));
        assert!(!v.contains_key("s"), "s is not written without a bank");
        let v = prepare_value(&map(json!({ "bank": "tr909" })), 1.0).unwrap();
        assert_eq!(v["s"], Value::Str("tr909_triangle".into()));
        let v = prepare_value(&map(json!({ "bank": 7, "s": "bd" })), 1.0).unwrap();
        assert_eq!(v["s"], Value::Str("7_bd".into()));
        let v = prepare_value(&map(json!({ "bank": "", "s": "sine" })), 1.0).unwrap();
        assert_eq!(v["s"], Value::Str("sine".into()));
        let v = prepare_value(&map(json!({ "bank": "b", "s": "" })), 1.0).unwrap();
        assert_eq!(
            v["s"],
            Value::Str(String::new()),
            "a falsy s keeps its value"
        );
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
        for list in [
            ALLOWED_KEYS,
            ALLOWED_SOUNDS,
            CHAIN_KEYS,
            CHAIN_UNSUPPORTED_KEYS,
            MODULATOR_TARGETS,
        ] {
            assert!(
                list.windows(2).all(|w| w[0] < w[1]),
                "sorted by byte order and unique"
            );
        }
        for s in SILENCE {
            assert!(ALLOWED_SOUNDS.contains(s));
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
        assert_eq!(CHAIN_KEYS.len(), 87);
        assert!(
            MODULATOR_TARGETS.windows(2).all(|w| w[0] < w[1]),
            "targets sorted"
        );
        // Every target is a control superdough's table knows and resolves somewhere: the
        // skips and throws are not listed.
        for t in MODULATOR_TARGETS {
            assert!(
                crate::modulate::get_control_data(t, "undefined").is_some(),
                "{t}"
            );
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
            assert!(!MODULATOR_TARGETS.contains(&t), "{t}");
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
        assert!(rows.len() >= 6, "{} rows", rows.len());
        for (sound, owner) in &rows {
            assert!(is_task_ref(owner), "{sound}: owner {owner}");
            if sound.contains(' ') {
                continue; // descriptive
            }
            if let Some(prefix) = sound.strip_suffix('*') {
                assert!(
                    !ALLOWED_SOUNDS.iter().any(|s| s.starts_with(prefix)),
                    "an allowed sound starts with {prefix}"
                );
            } else {
                assert!(
                    !ALLOWED_SOUNDS.contains(&sound.as_str()),
                    "{sound} is allowed"
                );
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
    fn fm_control_names_follow_the_matrix_rule() {
        assert_eq!(fm_control_name(1, 0), "fmi");
        assert_eq!(fm_control_name(2, 1), "fmi2");
        assert_eq!(fm_control_name(8, 7), "fmi8");
        assert_eq!(fm_control_name(2, 0), "fmi20");
        assert_eq!(fm_control_name(1, 3), "fmi13");
        assert_eq!(fm_control_name(1, 1), "fmi11");
        assert_eq!(fm_control_name(3, 1), "fmi31");
        assert_eq!(fm_suffix(1), "");
        assert_eq!(fm_suffix(5), "5");
        // Every name applyFM reads is allowed.
        for i in 1..=FM_MODULATORS {
            for j in 0..=FM_MODULATORS {
                let k = fm_control_name(i, j);
                assert!(ALLOWED_KEYS.contains(&k.as_str()), "{k}");
            }
        }
    }

    #[test]
    fn fm_entries_are_truthy_amounts_in_matrix_order() {
        let p = params(json!({
            "fmi2": 2, "fmi": "1.5", "fmi13": 0, "fmi31": null, "fmi20": "", "fmi44": true,
            "fmi55": "abc", "fmi10": 3
        }));
        let e: Vec<(usize, usize, f64)> =
            p.fm.entries
                .iter()
                .map(|e| (e.source, e.target, e.amount))
                .collect();
        assert_eq!(e[..3], [(1, 0, 1.5), (2, 1, 2.0), (4, 4, 1.0)]);
        assert_eq!((e[3].0, e[3].1), (5, 5));
        assert!(
            e[3].2.is_nan(),
            "a truthy non-number amount is NaN (gain.value throws)"
        );
        assert_eq!(e.len(), 4, "fmi10 is not a name applyFM reads");
        assert!(params(json!({})).fm.entries.is_empty());
    }

    #[test]
    fn fm_modulator_controls() {
        let p = params(json!({
            "fmh": null, "fmh2": "3", "fmwave": "white", "fmwave3": null,
            "fmattack": null, "fmdecay2": 0.2, "fmenv2": "lin", "fmenv3": null, "fmenv4": 0,
            "fmenv5": "exp"
        }));
        let m = &p.fm.mods;
        assert_eq!(m[0].ratio, 1.0, "fmh ?? 1");
        assert_eq!(m[1].ratio, 3.0);
        assert_eq!(m[2].ratio, 1.0);
        assert_eq!(m[0].wave, Some(Value::Str("white".into())));
        assert_eq!(m[2].wave, None);
        assert!(m[0].has_envelope, "a null control counts as defined");
        assert_eq!(m[0].adsr, [None; 4]);
        assert!(m[1].has_envelope);
        assert_eq!(m[1].adsr, [None, Some(0.2), None, None]);
        assert!(!m[2].has_envelope);
        assert!(m[0].exponential, "fmenv defaults to 'exp'");
        assert!(!m[1].exponential);
        assert!(m[2].exponential, "null ?? 'exp'");
        assert!(!m[3].exponential, "0 is not 'exp'");
        assert!(m[4].exponential);
    }

    #[test]
    fn bus_controls() {
        let p = params(json!({}));
        assert_eq!(
            (p.bus, p.busgain, p.bus_sound_key.as_str()),
            (None, 1.0, "0")
        );
        assert_eq!(params(json!({ "bus": null })).bus, None);
        assert_eq!(params(json!({ "bus": 0 })).bus.as_deref(), Some("0"));
        assert_eq!(params(json!({ "bus": "x" })).bus.as_deref(), Some("x"));
        assert_eq!(params(json!({ "bus": 2.5 })).bus.as_deref(), Some("2.5"));
        assert_eq!(params(json!({ "busgain": null })).busgain, 0.0);
        assert_eq!(params(json!({ "busgain": "0.25" })).busgain, 0.25);
        assert_eq!(params(json!({ "n": 3 })).bus_sound_key, "3");
        assert_eq!(params(json!({ "n": null })).bus_sound_key, "0");
        // The bus sound and its keys are allowed; a bus reader without a cycle is supported.
        assert!(ALLOWED_SOUNDS.binary_search(&"bus").is_ok());
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
    fn noise_and_density() {
        assert_eq!(params(json!({})).noise, None);
        assert_eq!(params(json!({ "noise": 0 })).noise, None);
        assert_eq!(params(json!({ "noise": null })).noise, None);
        assert_eq!(params(json!({ "noise": "" })).noise, None);
        assert_eq!(params(json!({ "noise": 0.4 })).noise, Some(0.4));
        assert_eq!(params(json!({ "noise": "0" })).noise, Some(0.0));
        assert_eq!(params(json!({ "noise": true })).noise, Some(1.0));
        assert_eq!(params(json!({})).density, None);
        assert_eq!(params(json!({ "density": null })).density, Some(0.0));
        assert_eq!(params(json!({ "density": "2" })).density, Some(2.0));
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
        assert_eq!(v["sounds"].as_array().unwrap().len(), ALLOWED_SOUNDS.len());
        assert_eq!(v["unsupportedValues"], json!({}));
        assert_eq!(v["chainKeys"], json!(CHAIN_KEYS));
        for (t, keys) in MODULATOR_KEYS {
            assert_eq!(v["modulatorKeys"][t], json!(keys), "{t}");
        }
        assert_eq!(v["modulatorTargets"], json!(MODULATOR_TARGETS));
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

    #[test]
    fn tremolo_filter_lfo_and_pulse_width_controls() {
        // No tremolo key: none; tremolosync (cps 0.5) wins over tremolo.
        assert_eq!(params(json!({ "tremolodepth": 2 })).main.tremolo, None);
        let t = params(json!({ "tremolo": 3, "tremolosync": 8 }))
            .main
            .tremolo
            .unwrap();
        assert_eq!(t.frequency, 4.0);
        assert_eq!(
            params(json!({ "tremolosync": 8 }))
                .main
                .tremolo
                .unwrap()
                .frequency,
            4.0
        );
        let t = params(json!({ "tremolo": 3, "tremolosync": null }))
            .main
            .tremolo
            .unwrap();
        assert_eq!(t.frequency, 3.0);
        // Defaults: depth 1, skew 1 without a shape, no phase; null shape is no shape.
        let t = params(json!({ "tremolo": 3 })).main.tremolo.unwrap();
        assert_eq!(
            t,
            TremoloParams {
                frequency: 3.0,
                depth: 1.0,
                skew: 1.0,
                shape: None,
                phase: None
            }
        );
        let t = params(json!({ "tremolo": 3, "tremoloshape": null }))
            .main
            .tremolo
            .unwrap();
        assert_eq!((t.skew, t.shape), (1.0, Some(Value::Null)));
        let t = params(json!({ "tremolo": null, "tremoloshape": 2, "tremolodepth": null }))
            .main
            .tremolo
            .unwrap();
        assert_eq!((t.frequency, t.depth, t.skew), (0.0, 0.0, 0.5));
        let t = params(json!({ "tremolo": 3, "tremoloskew": 0.1, "tremolophase": "0.5" }))
            .main
            .tremolo
            .unwrap();
        assert_eq!((t.skew, t.phase), (0.1, Some(0.5)));
        // The filter LFO controls are kept raw per filter.
        let p = params(json!({
            "cutoff": 800, "lprate": 3, "lpdc": null, "hcutoff": 200, "hpsync": "2",
            "bandf": 900, "bpshape": "saw", "bpdepthfrequency": 100, "lpskew": 0.2
        }));
        let lp = p.main.lowpass.unwrap().lfo;
        assert_eq!(lp.rate, Some(Value::Num(3.0)));
        assert_eq!(lp.dcoffset, Some(Value::Null));
        assert_eq!(lp.skew, Some(Value::Num(0.2)));
        assert_eq!(lp.sync, None);
        assert_eq!(
            p.main.highpass.unwrap().lfo.sync,
            Some(Value::Str("2".into()))
        );
        let bp = p.main.bandpass.unwrap().lfo;
        assert_eq!(bp.shape, Some(Value::Str("saw".into())));
        assert_eq!(bp.depthfrequency, Some(Value::Num(100.0)));
        assert_eq!(bp.depth, None);
        // pwrate/pwsweep defaults (synth.mjs 301-312).
        let pw = |v| {
            let p = params(v);
            (p.pwrate, p.pwsweep)
        };
        assert_eq!(pw(json!({})), (1.0, 0.0));
        assert_eq!(pw(json!({ "pwrate": 4 })), (4.0, 0.3));
        assert_eq!(pw(json!({ "pwsweep": 0.5 })), (1.0, 0.5));
        assert_eq!(pw(json!({ "pwrate": null, "pwsweep": null })), (1.0, 0.0));
        assert_eq!(pw(json!({ "pwrate": 2, "pwsweep": 0 })), (2.0, 0.0));
        assert!(pw(json!({ "pwsweep": "x" })).1.is_nan());
    }
}

//! Bit-exact ports of the JavaScript the engine must agree with: `String(number)` and superdough's
//! seeding (`superdough/rng.mjs`: `hashSeed`, `mulberry32`, `makeRandom`, `randomFromSeed`,
//! `stable`, `valueHash`, `voiceSeed`). Every function here is checked exactly against the JS
//! vectors in `tests/vectors/js-*.json` (`bun run engine:vectors`).
//!
//! Offline the audio seed is always a number (`state.seed ?? 0`), so the `Math.random` fallbacks
//! of `rng.mjs` are not ported.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::job::{Value, ValueMap};

/// JS `Number.prototype.toString()` (radix 10), ECMAScript `Number::toString` steps 5 to 12:
/// the shortest round-trip digits, plain notation for decimal exponents in `(-6, 21]`, else
/// `d.ddde+X` / `d.ddde-X`. Both zeros are `"0"`.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if x < 0.0 {
        return format!("-{}", number_to_string(-x));
    }
    let (digits, n) = shortest_digits(x);
    let k = digits.len() as i64;
    if k <= n && n <= 21 {
        let mut s = digits;
        s.extend(std::iter::repeat_n('0', (n - k) as usize));
        s
    } else if 0 < n && n <= 21 {
        let (a, b) = digits.split_at(n as usize);
        format!("{a}.{b}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let sign = if n > 0 { '+' } else { '-' };
        let e = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{e}")
        } else {
            let (a, b) = digits.split_at(1);
            format!("{a}.{b}e{sign}{e}")
        }
    }
}

/// Splits Rust's `{:e}` text ("d.ddde<exp>") into its digits and the exponent.
fn split_exp(text: &str) -> (String, i64) {
    let (mantissa, exp) = text.split_once('e').unwrap_or((text, "0"));
    let digits = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exp.parse::<i64>().unwrap_or(0))
}

/// ECMAScript's `k` digits and decimal point position `n` (value = 0.digits * 10^n) of a finite
/// positive `x`: the fewest digits that round-trip, the closest to `x` among those, and on an
/// exact tie the even one. Rust's shortest formatting gives the first two but breaks ties
/// upward (2013535394798964.25 prints as ...964.3; JS gives ...964.2), so a tie, which needs
/// the exact expansion of `x` to be those digits plus a final 5, is checked and settled here.
fn shortest_digits(x: f64) -> (String, i64) {
    let (digits, exp) = split_exp(&format!("{x:e}"));
    let k = digits.len();
    // k + 1 significant digits ending in 5 is necessary for a tie (exact, so no rounding).
    let (longer, longer_exp) = split_exp(&format!("{x:.k$e}"));
    if longer.ends_with('5') {
        // At most 767 significant digits: this precision prints the exact expansion.
        let (exact, _) = split_exp(&format!("{x:.800e}"));
        if exact.trim_end_matches('0') == longer {
            let down = &longer[..k];
            let even = down
                .bytes()
                .last()
                .is_some_and(|d| (d - b'0').is_multiple_of(2));
            let parses = format!("{down}e{}", longer_exp - (k as i64 - 1))
                .parse::<f64>()
                .is_ok_and(|y| y == x);
            if even && parses && down != digits {
                return (down.to_string(), longer_exp + 1);
            }
        }
    }
    (digits, exp + 1)
}

/// One argument of `hashSeed(...parts)`: hashed as `typeof p + ':' + String(p)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HashPart<'a> {
    Number(f64),
    Str(&'a str),
    /// Any other value: its `typeof` and its `String(p)` text (e.g. `("boolean", "true")`,
    /// `("object", "null")`, `("object", "1,2")`).
    Typed(&'a str, &'a str),
}

const FNV_OFFSET: u32 = 2166136261;
const FNV_PRIME: u32 = 16777619;

fn fnv_units(mut h: u32, s: &str) -> u32 {
    for c in s.encode_utf16() {
        h ^= u32::from(c);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// `hashSeed(...parts)` (rng.mjs): 32-bit FNV-1a over the UTF-16 code units of the parts, each
/// written `number:<String(n)>`, `string:<s>` or `<typeof p>:<String(p)>`, joined with U+0000.
pub fn hash_seed(parts: &[HashPart]) -> u32 {
    let mut h = FNV_OFFSET;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h = fnv_units(h, "\u{0}");
        }
        h = match p {
            HashPart::Number(x) => fnv_units(fnv_units(h, "number:"), &number_to_string(*x)),
            HashPart::Str(s) => fnv_units(fnv_units(h, "string:"), s),
            HashPart::Typed(t, s) => fnv_units(fnv_units(fnv_units(h, t), ":"), s),
        };
    }
    h
}

/// `mulberry32(seed)` (rng.mjs): a generator of floats in `[0, 1)`, each a multiple of 2^-32.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mulberry32 {
    a: u32,
}

impl Mulberry32 {
    /// `mulberry32(seed)`; JS takes `seed >>> 0`, so callers pass the `u32` bit pattern.
    pub fn new(seed: u32) -> Self {
        Mulberry32 { a: seed }
    }

    /// The next output, exactly as the JS closure computes it.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let mut t = self.a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4294967296.0
    }
}

/// `makeRandom(...keyParts)` with the audio seed `seed`: `mulberry32(hashSeed(seed, ...keyParts))`.
pub fn make_random(seed: i32, key_parts: &[HashPart]) -> Mulberry32 {
    let mut parts = Vec::with_capacity(key_parts.len() + 1);
    parts.push(HashPart::Number(f64::from(seed)));
    parts.extend_from_slice(key_parts);
    Mulberry32::new(hash_seed(&parts))
}

/// `randomFromSeed(seed)` for a numeric seed (a voice seed): `mulberry32(seed)`.
pub fn random_from_seed(seed: u32) -> Mulberry32 {
    Mulberry32::new(seed)
}

/// JS default `Array.prototype.sort()` order for strings: by UTF-16 code units.
fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// A JS "array index": a canonical decimal integer from 0 to 2^32 - 2.
fn array_index(k: &str) -> Option<u32> {
    let canonical = !k.is_empty()
        && k.bytes().all(|b| b.is_ascii_digit())
        && (k == "0" || !k.starts_with('0'))
        && k.len() <= 10;
    if !canonical {
        return None;
    }
    k.parse::<u64>()
        .ok()
        .filter(|n| *n <= 4_294_967_294)
        .map(|n| n as u32)
}

/// The key order `JSON.stringify` enumerates after `Object.fromEntries(Object.keys(v).sort()...)`:
/// array-index keys first in ascending numeric order, then the rest in UTF-16 order.
fn js_object_key_cmp(a: &str, b: &str) -> Ordering {
    match (array_index(a), array_index(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => utf16_cmp(a, b),
    }
}

/// `JSON.stringify` of a string (well-formed: Rust strings hold no lone surrogates).
fn json_quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify` of a number: non-finite values are `null`, `-0` is `0`.
fn json_number(x: f64) -> String {
    if x.is_finite() {
        number_to_string(x)
    } else {
        "null".into()
    }
}

/// `JSON.stringify(sortKeys(v))`. `Unsupported` stands for a value `JSON.stringify` skips (a
/// function or `undefined`): omitted as an object member, `null` as an array element. The engine
/// refuses jobs with `Unsupported` members before hashing, so this only keeps it total. A `Set`
/// has no own keys, so `sortKeys` turns it into an empty object: `{}`.
fn json_sorted(v: &Value, out: &mut String) {
    match v {
        Value::Null | Value::Unsupported(_) => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(x) => out.push_str(&json_number(*x)),
        Value::Str(s) => json_quote(s, out),
        Value::Set(_) => out.push_str("{}"),
        Value::Arr(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_sorted(x, out);
            }
            out.push(']');
        }
        Value::Obj(m) => {
            let mut keys: Vec<&String> = m
                .iter()
                .filter(|(_, v)| !matches!(v, Value::Unsupported(_)))
                .map(|(k, _)| k)
                .collect();
            keys.sort_by(|a, b| js_object_key_cmp(a, b));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_quote(k, out);
                out.push(':');
                json_sorted(&m[*k], out);
            }
            out.push('}');
        }
    }
}

/// `stable(v)` (rng.mjs): `JSON.stringify` with recursively sorted keys for arrays and objects,
/// `String(v)` for scalars. A `Set` is an object: `{}`.
pub fn stable(v: &Value) -> String {
    match v {
        Value::Arr(_) | Value::Obj(_) | Value::Set(_) => {
            let mut out = String::new();
            json_sorted(v, &mut out);
            out
        }
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(x) => number_to_string(*x),
        Value::Str(s) => s.clone(),
        Value::Unsupported(_) => "undefined".into(),
    }
}

/// `valueHash(value)` (rng.mjs): the keys not starting with `_` (and not `undefined`/functions,
/// which a job encodes as `Unsupported`), sorted by UTF-16 code units, each hashed as the two
/// string parts `key` and `stable(value)`.
pub fn value_hash(map: &ValueMap) -> u32 {
    let mut keys: Vec<&String> = map
        .iter()
        .filter(|(k, v)| !k.starts_with('_') && !matches!(v, Value::Unsupported(_)))
        .map(|(k, _)| k)
        .collect();
    keys.sort_by(|a, b| utf16_cmp(a, b));
    let stables: Vec<String> = keys.iter().map(|k| stable(&map[*k])).collect();
    let parts: Vec<HashPart> = keys
        .iter()
        .zip(&stables)
        .flat_map(|(k, s)| [HashPart::Str(k), HashPart::Str(s)])
        .collect();
    hash_seed(&parts)
}

/// `voiceSeed`'s module state (`lastCycle` and the `occurrences` map) for one render; a fresh
/// one is what `setAudioSeed` leaves after a seed change.
#[derive(Debug, Clone, Default)]
pub struct Occurrence {
    last_cycle: Option<f64>,
    counts: HashMap<u32, u64>,
}

impl Occurrence {
    pub fn new() -> Self {
        Self::default()
    }
}

/// `voiceSeed(cycle, value)` (rng.mjs) with the audio seed `seed`. `map` is the value after
/// superdough's mutation (`params::prepare_value`). The occurrence counter resets whenever
/// `cycle` differs from the previous call's (JS `!==`: NaN always differs, `-0` equals `0`), so
/// it is call-order dependent.
pub fn voice_seed(cycle: f64, map: &ValueMap, seed: i32, occ: &mut Occurrence) -> u32 {
    if occ.last_cycle.is_none_or(|last| last != cycle) {
        occ.counts.clear();
        occ.last_cycle = Some(cycle);
    }
    let vh = value_hash(map);
    let count = occ.counts.entry(vh).or_insert(0);
    let occurrence = *count;
    *count += 1;
    hash_seed(&[
        HashPart::Number(f64::from(seed)),
        HashPart::Number(cycle),
        HashPart::Number(f64::from(vh)),
        HashPart::Number(occurrence as f64),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::prepare_value;
    use crate::test_vectors::{decode, load as vectors, num};
    use serde_json::Value as J;

    fn cases(v: &J) -> &Vec<J> {
        v["cases"].as_array().expect("cases")
    }

    fn bits_of(v: &J) -> f64 {
        let s = v.as_str().expect("bits are a hex string");
        f64::from_bits(u64::from_str_radix(s, 16).unwrap())
    }

    fn map_of(v: &J) -> ValueMap {
        match decode(v) {
            Value::Obj(m) => m,
            other => panic!("not a map: {other:?}"),
        }
    }

    fn s(x: f64) -> String {
        number_to_string(x)
    }

    // ---- number_to_string

    #[test]
    fn specials_and_zeros() {
        assert_eq!(s(f64::NAN), "NaN");
        assert_eq!(s(-f64::NAN), "NaN");
        assert_eq!(s(f64::INFINITY), "Infinity");
        assert_eq!(s(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(s(0.0), "0");
        assert_eq!(s(-0.0), "0");
    }

    #[test]
    fn integers_and_decimals() {
        assert_eq!(s(1.0), "1");
        assert_eq!(s(-42.0), "-42");
        assert_eq!(s(440.0), "440");
        assert_eq!(s(0.1), "0.1");
        assert_eq!(s(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(s(123.456), "123.456");
        assert_eq!(s(1.5e-3), "0.0015");
        assert_eq!(s(2.0 / 3.0), "0.6666666666666666");
        assert_eq!(s(1e20), "100000000000000000000");
        assert_eq!(s(123456789012345680000.0), "123456789012345680000");
        assert_eq!(s(9007199254740993.0), "9007199254740992");
        assert_eq!(s(4294967295.0), "4294967295");
    }

    #[test]
    fn exact_ties_round_to_the_even_digit() {
        // 2013535394798964.25 exactly: both ...964.2 and ...964.3 round-trip and are equally
        // close; ECMAScript takes the even one (Rust's `{:e}` alone gives ...964.3).
        let x = f64::from_bits(0x431c_9d32_f7cc_75d1);
        assert_eq!(format!("{x:e}"), "2.0135353947989643e15");
        assert_eq!(s(x), "2013535394798964.2");
        assert_eq!(s(-x), "-2013535394798964.2");
        // 2^50 + 1.75: ...1.7 and ...1.8 tie and 8 is even, which is also the upward one.
        let y = 1125899906842624.0 + 1.75;
        assert_eq!(s(y), "1125899906842625.8");
        assert_eq!(s(1125899906842624.0 + 3.25), "1125899906842627.2");
        // Short exact values are not ties.
        assert_eq!(s(0.5), "0.5");
        assert_eq!(s(2.5), "2.5");
        assert_eq!(s(9007199254740994.0), "9007199254740994");
    }

    #[test]
    fn exponent_boundaries() {
        assert_eq!(s(1e21), "1e+21");
        assert_eq!(s(999999999999999900000.0), "999999999999999900000");
        assert_eq!(s(1.5e21), "1.5e+21");
        assert_eq!(s(0.000001), "0.000001");
        assert_eq!(s(0.0000012), "0.0000012");
        assert_eq!(s(9.99e-7), "9.99e-7");
        assert_eq!(s(1e-7), "1e-7");
        assert_eq!(s(1.25e-7), "1.25e-7");
        assert_eq!(s(f64::MAX), "1.7976931348623157e+308");
        assert_eq!(s(f64::MIN_POSITIVE), "2.2250738585072014e-308");
        assert_eq!(s(5e-324), "5e-324");
        assert_eq!(s(-1e-7), "-1e-7");
        assert_eq!(s(-1e21), "-1e+21");
    }

    /// A small xorshift generator so the property test is deterministic without extra crates.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn number_to_string_round_trips() {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let mut checked = 0;
        for i in 0..200_000 {
            let x = match i % 3 {
                0 => f64::from_bits(rng.next()),
                1 => (rng.next() % 1_000_000_000) as f64 / 10f64.powi((rng.next() % 12) as i32),
                _ => {
                    (rng.next() as f64 / u64::MAX as f64)
                        * 10f64.powi((rng.next() % 40) as i32 - 20)
                }
            };
            if !x.is_finite() {
                continue;
            }
            let text = s(x);
            let back: f64 = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(
                back.to_bits(),
                if x == 0.0 { 0 } else { x.to_bits() },
                "{x:e} -> {text}"
            );
            // JS's shape: no '+' outside an exponent, at most one '.', no leading zeros in integers.
            assert!(!text.contains("e+0") && !text.contains("e-0"), "{text}");
            checked += 1;
        }
        assert!(checked > 190_000);
    }

    #[test]
    fn number_to_string_vectors() {
        let v = vectors("js-number-to-string.json");
        assert!(cases(&v).len() > 100);
        for c in cases(&v) {
            let x = bits_of(&c["bits"]);
            let from_value = num(&c["value"]);
            assert!(
                from_value.to_bits() == x.to_bits() || (x.is_nan() && from_value.is_nan()),
                "value and bits disagree: {c}"
            );
            assert_eq!(s(x), c["string"].as_str().unwrap(), "{c}");
        }
    }

    // ---- hash_seed

    #[test]
    fn hash_seed_basics() {
        assert_eq!(hash_seed(&[]), FNV_OFFSET, "the empty string");
        // "string:" then nothing: the empty string part is not the empty list.
        assert_ne!(hash_seed(&[HashPart::Str("")]), FNV_OFFSET);
        assert_ne!(
            hash_seed(&[HashPart::Str("")]),
            hash_seed(&[HashPart::Str(""), HashPart::Str("")]),
            "the separator is hashed"
        );
        assert_ne!(
            hash_seed(&[HashPart::Number(1.0)]),
            hash_seed(&[HashPart::Str("1")]),
            "the type prefix is hashed"
        );
        assert_eq!(
            hash_seed(&[HashPart::Number(-0.0)]),
            hash_seed(&[HashPart::Number(0.0)]),
            "String(-0) is 0"
        );
        assert_eq!(
            hash_seed(&[HashPart::Number(f64::NAN)]),
            hash_seed(&[HashPart::Number(-f64::NAN)])
        );
    }

    #[test]
    fn typed_parts_hash_their_typeof_and_text() {
        // `typeof p + ':' + String(p)` for any value: the typed form of a number or a string is
        // the same hash as the dedicated variants.
        assert_eq!(
            hash_seed(&[
                HashPart::Typed("number", "1"),
                HashPart::Typed("string", "x")
            ]),
            hash_seed(&[HashPart::Number(1.0), HashPart::Str("x")])
        );
        assert_ne!(
            hash_seed(&[HashPart::Typed("boolean", "true")]),
            hash_seed(&[HashPart::Str("true")])
        );
        assert_ne!(
            hash_seed(&[HashPart::Typed("object", "null")]),
            hash_seed(&[HashPart::Typed("object", "")])
        );
    }

    #[test]
    fn hash_seed_uses_utf16_code_units() {
        // U+1F600 is the surrogate pair D83D DE00: two FNV steps, not four UTF-8 bytes.
        let mut h = FNV_OFFSET;
        for c in "string:".encode_utf16().chain([0xd83d, 0xde00]) {
            h ^= u32::from(c);
            h = h.wrapping_mul(FNV_PRIME);
        }
        assert_eq!(hash_seed(&[HashPart::Str("\u{1F600}")]), h);
        let mut bytes = FNV_OFFSET;
        for b in "string:\u{1F600}".bytes() {
            bytes ^= u32::from(b);
            bytes = bytes.wrapping_mul(FNV_PRIME);
        }
        assert_ne!(h, bytes);
        // A NUL inside a string is the same code unit as the separator.
        assert_eq!(
            hash_seed(&[HashPart::Str("a\u{0}string:b")]),
            hash_seed(&[HashPart::Str("a"), HashPart::Str("b")])
        );
    }

    #[test]
    fn hash_seed_vectors() {
        let v = vectors("js-hash-seed.json");
        assert!(cases(&v).len() > 10);
        for c in cases(&v) {
            let texts: Vec<String> = c["parts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["type"] == "string")
                .map(|p| p["value"].as_str().unwrap().to_string())
                .collect();
            let mut ti = texts.iter();
            let parts: Vec<HashPart> = c["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| match p["type"].as_str().unwrap() {
                    "number" => HashPart::Number(bits_of(&p["bits"])),
                    "string" => HashPart::Str(ti.next().unwrap()),
                    t => panic!("bad part type {t}"),
                })
                .collect();
            assert_eq!(
                u64::from(hash_seed(&parts)),
                c["hash"].as_u64().unwrap(),
                "{c}"
            );
        }
    }

    // ---- Mulberry32

    #[test]
    fn mulberry32_first_outputs() {
        // mulberry32(0): a = 0x6d2b79f5 on the first call.
        let mut m = Mulberry32::new(0);
        let first = m.next();
        assert!((0.0..1.0).contains(&first));
        assert_eq!((first * 4294967296.0).fract(), 0.0, "a multiple of 2^-32");
        assert_eq!(Mulberry32::new(0).next(), first, "deterministic");
        assert_ne!(m.next(), first);
        // Seed wraparound: a = 2^32 - 1 + 0x6d2b79f5 wraps without panicking.
        let mut top = Mulberry32::new(u32::MAX);
        for _ in 0..1000 {
            let x = top.next();
            assert!((0.0..1.0).contains(&x));
        }
        assert_eq!(random_from_seed(7), Mulberry32::new(7));
        assert_eq!(
            make_random(3, &[HashPart::Str("noise")]),
            Mulberry32::new(hash_seed(&[HashPart::Number(3.0), HashPart::Str("noise")]))
        );
        assert_eq!(
            make_random(-1, &[]),
            Mulberry32::new(hash_seed(&[HashPart::Number(-1.0)]))
        );
    }

    #[test]
    fn mulberry32_vectors() {
        let v = vectors("js-mulberry32.json");
        let seeds: Vec<u64> = cases(&v)
            .iter()
            .map(|c| c["seed"].as_u64().unwrap())
            .collect();
        assert!(
            seeds.contains(&0) && seeds.contains(&2147483647),
            "{seeds:?}"
        );
        for c in cases(&v) {
            let mut m = Mulberry32::new(c["seed"].as_u64().unwrap() as u32);
            let outs = c["outputs"].as_array().unwrap();
            assert_eq!(outs.len(), 64);
            for (i, o) in outs.iter().enumerate() {
                let want = num(o);
                assert_eq!(
                    m.next().to_bits(),
                    want.to_bits(),
                    "seed {} output {i}",
                    c["seed"]
                );
            }
        }
        let made = v["makeRandom"].as_array().unwrap();
        assert!(!made.is_empty());
        for c in made {
            let texts: Vec<String> = c["keyParts"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect();
            let mut ti = texts.iter();
            let parts: Vec<HashPart> = c["keyParts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| match p {
                    J::String(_) => HashPart::Str(ti.next().unwrap()),
                    n => HashPart::Number(num(n)),
                })
                .collect();
            let seed = c["seed"].as_i64().unwrap() as i32;
            let mut m = make_random(seed, &parts);
            for (i, o) in c["outputs"].as_array().unwrap().iter().enumerate() {
                assert_eq!(m.next().to_bits(), num(o).to_bits(), "{c} output {i}");
            }
        }
    }

    // ---- stable and value_hash

    fn obj(v: J) -> Value {
        decode(&v)
    }

    #[test]
    fn stable_scalars_are_string_conversions() {
        assert_eq!(stable(&Value::Null), "null");
        assert_eq!(stable(&Value::Bool(true)), "true");
        assert_eq!(stable(&Value::Num(-0.0)), "0");
        assert_eq!(stable(&Value::Num(f64::NAN)), "NaN");
        assert_eq!(stable(&Value::Num(1e21)), "1e+21");
        assert_eq!(stable(&Value::Str("a\"b".into())), "a\"b", "not quoted");
    }

    #[test]
    fn stable_sorts_keys_like_js_objects() {
        let v = obj(
            serde_json::json!({ "b": 1, "a": [3, { "z": null, "y": true }], "10": 1, "2": 2, "02": 0, "-1": 0 }),
        );
        assert_eq!(
            stable(&v),
            r#"{"2":2,"10":1,"-1":0,"02":0,"a":[3,{"y":true,"z":null}],"b":1}"#
        );
        // 2^32 - 2 is the largest array index; 2^32 - 1 sorts as a string.
        let v = obj(serde_json::json!({ "4294967295": 0, "4294967294": 0, "a": 0, "1": 0 }));
        assert_eq!(stable(&v), r#"{"1":0,"4294967294":0,"4294967295":0,"a":0}"#);
        // UTF-16 order: U+FF61 (one unit) sorts after U+1F600 (surrogates D83D DE00).
        let v = obj(serde_json::json!({ "\u{FF61}": 0, "\u{1F600}": 1 }));
        assert_eq!(stable(&v), "{\"\u{1F600}\":1,\"\u{FF61}\":0}");
        assert_eq!(stable(&obj(serde_json::json!({}))), "{}");
        assert_eq!(stable(&obj(serde_json::json!([]))), "[]");
    }

    #[test]
    fn stable_numbers_and_escapes_follow_json_stringify() {
        let v = Value::Arr(vec![
            Value::Num(f64::NAN),
            Value::Num(f64::INFINITY),
            Value::Num(f64::NEG_INFINITY),
            Value::Num(-0.0),
            Value::Num(0.1),
            Value::Num(1e-7),
            Value::Str("q\"\\\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}\u{2028}\u{2029}\u{1F600}".into()),
            Value::Unsupported("function".into()),
        ]);
        assert_eq!(
            stable(&v),
            "[null,null,null,0,0.1,1e-7,\"q\\\"\\\\\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}\u{2028}\u{2029}\u{1F600}\",null]"
        );
        let mut m = ValueMap::new();
        m.insert("f".into(), Value::Unsupported("function".into()));
        m.insert("a".into(), Value::Num(1.0));
        assert_eq!(stable(&Value::Obj(m)), r#"{"a":1}"#, "skipped members");
    }

    #[test]
    fn stable_of_a_set_is_an_empty_object() {
        assert_eq!(stable(&Value::Set(vec![])), "{}");
        assert_eq!(stable(&obj(serde_json::json!({ "$set": [1, "a"] }))), "{}");
        assert_eq!(
            stable(&obj(serde_json::json!({ "b": { "$set": [0, 1] }, "a": 1 }))),
            r#"{"a":1,"b":{}}"#,
            "nested in an object"
        );
        assert_eq!(
            stable(&obj(serde_json::json!([{ "$set": [3] }, 1]))),
            "[{},1]",
            "nested in an array"
        );
        assert_eq!(
            stable(&obj(serde_json::json!({ "$set": [{ "$set": [1] }] }))),
            "{}"
        );
    }

    #[test]
    fn value_hash_of_a_set_equals_the_empty_object() {
        let with_set = map_of(&serde_json::json!({
            "s": "sine",
            "lfo": { "__ids": { "$set": [0, 1] }, "0": { "control": "gain" } }
        }));
        let with_obj = map_of(&serde_json::json!({
            "s": "sine",
            "lfo": { "__ids": {}, "0": { "control": "gain" } }
        }));
        assert_eq!(value_hash(&with_set), value_hash(&with_obj));
        let top = map_of(&serde_json::json!({ "s": "sine", "ids": { "$set": [2] } }));
        assert_eq!(
            value_hash(&top),
            hash_seed(&[
                HashPart::Str("ids"),
                HashPart::Str("{}"),
                HashPart::Str("s"),
                HashPart::Str("sine")
            ]),
            "a top-level set is hashed, not skipped"
        );
    }

    #[test]
    fn stable_vectors() {
        let v = vectors("js-stable.json");
        assert!(cases(&v).len() > 10);
        for c in cases(&v) {
            assert_eq!(
                stable(&decode(&c["value"])),
                c["string"].as_str().unwrap(),
                "{c}"
            );
        }
    }

    #[test]
    fn value_hash_skips_underscore_keys_and_sorts_by_utf16() {
        let m = map_of(&serde_json::json!({ "s": "sine", "note": 60 }));
        let mut with_private = m.clone();
        with_private.insert("_voiceSeed".into(), Value::Num(5.0));
        with_private.insert("f".into(), Value::Unsupported("function".into()));
        assert_eq!(value_hash(&m), value_hash(&with_private));
        assert_eq!(
            value_hash(&m),
            hash_seed(&[
                HashPart::Str("note"),
                HashPart::Str("60"),
                HashPart::Str("s"),
                HashPart::Str("sine")
            ])
        );
        // Top-level keys sort as strings ("10" before "2"), unlike nested objects.
        let m = map_of(&serde_json::json!({ "2": 1, "10": [1, 2] }));
        assert_eq!(
            value_hash(&m),
            hash_seed(&[
                HashPart::Str("10"),
                HashPart::Str("[1,2]"),
                HashPart::Str("2"),
                HashPart::Str("1")
            ])
        );
        assert_eq!(value_hash(&ValueMap::new()), FNV_OFFSET);
    }

    // ---- voice_seed

    fn seed_of(cycle: f64, m: &ValueMap, seed: i32, occ: &mut Occurrence, n: u64) -> u32 {
        let vh = value_hash(m);
        let got = voice_seed(cycle, m, seed, occ);
        assert_eq!(
            got,
            hash_seed(&[
                HashPart::Number(f64::from(seed)),
                HashPart::Number(cycle),
                HashPart::Number(f64::from(vh)),
                HashPart::Number(n as f64)
            ]),
            "occurrence {n} at cycle {cycle}"
        );
        got
    }

    #[test]
    fn voice_seed_counts_identical_values_within_a_cycle() {
        let a = map_of(&serde_json::json!({ "s": "sine", "duration": 0.5 }));
        let b = map_of(&serde_json::json!({ "s": "saw", "duration": 0.5 }));
        let mut occ = Occurrence::new();
        let s0 = seed_of(0.0, &a, 0, &mut occ, 0);
        let s1 = seed_of(0.0, &a, 0, &mut occ, 1);
        seed_of(0.0, &b, 0, &mut occ, 0);
        seed_of(0.0, &a, 0, &mut occ, 2);
        assert_ne!(s0, s1);
        // A new cycle resets, and so does coming back to the old one.
        seed_of(1.0, &a, 0, &mut occ, 0);
        seed_of(1.0, &a, 0, &mut occ, 1);
        seed_of(0.0, &a, 0, &mut occ, 0);
        // Interleaved cycles reset on every call.
        seed_of(0.5, &a, 0, &mut occ, 0);
        seed_of(0.0, &a, 0, &mut occ, 0);
        seed_of(0.5, &a, 0, &mut occ, 0);
        // -0 equals 0 (no reset) and hashes as "0".
        let mut occ = Occurrence::new();
        let z = seed_of(0.0, &a, 0, &mut occ, 0);
        let nz = seed_of(-0.0, &a, 0, &mut occ, 1);
        assert_ne!(z, nz);
        // NaN never equals the previous cycle.
        let mut occ = Occurrence::new();
        seed_of(f64::NAN, &a, 0, &mut occ, 0);
        seed_of(f64::NAN, &a, 0, &mut occ, 0);
        // The audio seed changes the result.
        let mut o1 = Occurrence::new();
        let mut o2 = Occurrence::new();
        assert_ne!(
            voice_seed(0.0, &a, 0, &mut o1),
            voice_seed(0.0, &a, 1, &mut o2)
        );
        let mut o3 = Occurrence::new();
        assert_eq!(voice_seed(0.0, &a, i32::MIN, &mut o3), {
            let mut o = Occurrence::new();
            voice_seed(0.0, &a, i32::MIN, &mut o)
        });
    }

    #[test]
    fn voice_seed_vectors() {
        let v = vectors("js-voice-seed.json");
        let seqs = cases(&v);
        assert!(seqs.len() >= 5);
        let mut skipped = 0;
        for seq in seqs {
            let name = seq["name"].as_str().unwrap();
            let seed = seq["seed"].as_i64().unwrap() as i32;
            let mut occ = Occurrence::new();
            for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
                let value = map_of(&step["value"]);
                let prepared = prepare_value(&value, num(&step["duration"]));
                match (&prepared, step["seed"].as_u64()) {
                    (None, None) => skipped += 1,
                    (Some(m), Some(want)) => {
                        let got = voice_seed(num(&step["cycle"]), m, seed, &mut occ);
                        assert_eq!(u64::from(got), want, "{name} step {i}: {step}");
                    }
                    (p, w) => panic!("{name} step {i}: prepared {p:?}, expected {w:?}"),
                }
            }
        }
        assert!(skipped > 0, "the vectors include silence markers");
    }
}

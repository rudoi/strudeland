//! superdough's JS coercions as the engine needs them: `Number(x)`, `String(x)`, truthiness,
//! `Math.max`/`min` and ECMAScript `StringToNumber`. Unlike `js`, these have no JS vectors;
//! their unit tests pin them.

use crate::job::Value;
use crate::js::number_to_string;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

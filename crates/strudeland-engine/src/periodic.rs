// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/periodic_wave_handler.cc, modules/webaudio/periodic_wave.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! `PeriodicWave` band-limited tables (`PeriodicWaveHandler`): 36 tables of 4096 samples at
//! 44.1 kHz, one per third of an octave, each keeping the partials below Nyquist for its pitch
//! range; the built-in `sine`, `square`, `sawtooth` and `triangle` waves (`GenerateBasicWaveform`)
//! and custom waves (`createPeriodicWave(real, imag)`), normalized by the peak of the
//! fullest table. Table selection per frequency has a scalar form (`WaveDataForFundamentalFrequency`)
//! and the four-lane NEON form the a-rate oscillator uses (multiplications by reciprocals instead
//! of divisions, so its table factors can differ in the last bit).
//!
//! Also superdough's `waveformN(partials, phases, type)` (synth.mjs:459-502), which builds the
//! `real`/`imag` arrays of a custom wave from the `partials` and `phases` controls.

use std::collections::HashMap;
use std::rc::Rc;

use crate::controls::js_to_number;
use crate::fft::FftFrame;
use crate::job::Value;

/// `PeriodicWaveSize()` at 44.1 kHz.
pub const TABLE_SIZE: usize = 4096;
/// `NumberOfRanges()`: `(unsigned)(0.5 + 3 * log2f(4096))`.
pub const NUM_RANGES: usize = 36;
/// `MaxNumberOfPartials()`.
pub const MAX_PARTIALS: usize = TABLE_SIZE / 2;
/// `RateScale()`: `PeriodicWaveSize() / sample_rate` in float.
pub const RATE_SCALE: f32 = 4096.0f32 / 44100.0f32;
/// `lowest_fundamental_frequency_`: `nyquist / MaxNumberOfPartials()` in float.
pub const LOWEST_FUNDAMENTAL: f32 = 22050.0f32 / 2048.0f32;
/// `cents_per_range_` (1200 / 3).
const CENTS_PER_RANGE: f32 = 400.0;
/// `kPiFloat`.
const PI_F32: f32 = std::f32::consts::PI;
#[cfg(test)]
const MASK: usize = TABLE_SIZE - 1;

/// The oscillator types with a built-in table, and custom waves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaveKind {
    Sine,
    Square,
    Sawtooth,
    Triangle,
    Custom,
}

impl WaveKind {
    /// The `OscillatorType` string of a built-in wave (`None` for anything else).
    pub fn builtin(name: &str) -> Option<WaveKind> {
        match name {
            "sine" => Some(WaveKind::Sine),
            "square" => Some(WaveKind::Square),
            "sawtooth" => Some(WaveKind::Sawtooth),
            "triangle" => Some(WaveKind::Triangle),
            _ => None,
        }
    }
}

/// Why a periodic wave cannot be built (superdough throws; the voice is silent).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WaveError {
    /// `createPeriodicWave` with fewer than two coefficients (an `IndexSizeError`).
    #[error(
        "IndexSizeError: Failed to execute 'createPeriodicWave' on 'BaseAudioContext': The length of the real array provided ({0}) is less than the minimum bound (2)."
    )]
    TooShort(usize),
    /// A coefficient that is NaN or infinite as a float (WebIDL `sequence<float>`, a TypeError).
    #[error(
        "TypeError: Failed to execute 'createPeriodicWave' on 'BaseAudioContext': The provided float value is non-finite."
    )]
    NonFinite,
    /// `new Float32Array(n)` with a negative or too large length.
    #[error("RangeError: Invalid typed array length: {0}")]
    InvalidLength(String),
    /// Coefficients so large that the tables overflow the float range. Chromium would render
    /// NaN; the engine refuses the wave instead (a documented deviation).
    #[error("the periodic wave overflows the float range")]
    Overflow,
}

/// The 36 band-limited tables of one wave.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodicWave {
    kind: WaveKind,
    tables: Vec<Vec<f32>>,
}

/// `NumberOfPartialsForRange(range)`: `2^(-range * 400 / 1200) * 2048` partials, truncated.
pub fn partials_for_range(range: usize) -> usize {
    let cents_to_cull = range as f32 * CENTS_PER_RANGE;
    let culling_scale = 2.0f64.powf(f64::from(-cents_to_cull / 1200.0)) as f32;
    (culling_scale * MAX_PARTIALS as f32) as usize
}

/// The scalar table selection: `(lower, higher, factor)` where `lower` is the table with fewer
/// partials (`range + 1`), `higher` the one with more and `factor` blends from higher to lower.
fn select_scalar(frequency: f32) -> (usize, usize, f32) {
    let f = frequency.abs();
    let ratio = if f > 0.0 { f / LOWEST_FUNDAMENTAL } else { 0.5 };
    let cents = ratio.log2() * 1200.0;
    let pitch_range = 1.0 + cents / CENTS_PER_RANGE;
    // `std::clamp(pitch_range, 0, 35)` (a NaN passes through).
    let top = (NUM_RANGES - 1) as f32;
    let pitch_range = if pitch_range < 0.0 {
        0.0
    } else if top < pitch_range {
        top
    } else {
        pitch_range
    };
    let index1 = pitch_range as u32 as usize;
    let index2 = if index1 < NUM_RANGES - 1 {
        index1 + 1
    } else {
        index1
    };
    (index2, index1, pitch_range - index1 as f32)
}

/// AArch64 `FMAX`/`FMIN` (`vmaxq_f32`/`vminq_f32`): NaN if either operand is NaN.
fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}

fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.min(b)
    }
}

/// The NEON four-lane table selection (`periodic_wave_handler.cc`, `CPU_ARM_NEON` branch).
fn select_neon(frequency: [f32; 4]) -> ([usize; 4], [usize; 4], [f32; 4]) {
    let inv_lowest = 1.0f32 / LOWEST_FUNDAMENTAL;
    let inv_cents = 1.0f32 / CENTS_PER_RANGE;
    let top = (NUM_RANGES - 1) as f32;
    let mut lower = [0; 4];
    let mut higher = [0; 4];
    let mut factor = [0.0; 4];
    for m in 0..4 {
        let f = frequency[m].abs();
        let ratio = if f > 0.0 { f * inv_lowest } else { 0.5 };
        let cents = ratio.log2() * 1200.0;
        let pitch_range = 1.0 + cents * inv_cents;
        let pitch_range = fmin(fmax(pitch_range, 0.0), top);
        // `vcvtq_u32_f32`: truncation, saturating, NaN to 0.
        let index1 = pitch_range as u32;
        let index2 = (index1 + 1).min(NUM_RANGES as u32 - 1);
        higher[m] = index1 as usize;
        lower[m] = index2 as usize;
        factor[m] = pitch_range - index1 as f32;
    }
    (lower, higher, factor)
}

impl PeriodicWave {
    /// `GenerateBasicWaveform(shape)`: the built-in table of `kind` (`None` for `Custom`).
    pub fn basic(kind: WaveKind) -> Option<PeriodicWave> {
        if kind == WaveKind::Custom {
            return None;
        }
        let mut real = vec![0.0f32; MAX_PARTIALS];
        let mut imag = vec![0.0f32; MAX_PARTIALS];
        for n in 1..MAX_PARTIALS {
            let pi_factor = 2.0f32 / (n as f32 * PI_F32);
            let b = match kind {
                WaveKind::Sine => {
                    if n == 1 {
                        1.0
                    } else {
                        0.0
                    }
                }
                WaveKind::Square => {
                    if n & 1 == 1 {
                        2.0 * pi_factor
                    } else {
                        0.0
                    }
                }
                WaveKind::Sawtooth => pi_factor * if n & 1 == 1 { 1.0 } else { -1.0 },
                WaveKind::Triangle => {
                    if n & 1 == 1 {
                        2.0 * (pi_factor * pi_factor)
                            * if (n - 1) >> 1 & 1 == 1 { -1.0 } else { 1.0 }
                    } else {
                        0.0
                    }
                }
                WaveKind::Custom => unreachable!(),
            };
            real[n] = 0.0;
            imag[n] = b;
        }
        let tables = create_band_limited_tables(&real, &imag, false);
        Some(PeriodicWave { kind, tables })
    }

    /// `createPeriodicWave(real, imag)` with default normalization. The arrays are the JS
    /// `Float32Array`s; only their first 2048 entries shape the wave.
    pub fn custom(real: &[f32], imag: &[f32]) -> Result<PeriodicWave, WaveError> {
        PeriodicWave::custom_with(real, imag, false)
    }

    /// `createPeriodicWave(real, imag, { disableNormalization })`.
    pub fn custom_with(
        real: &[f32],
        imag: &[f32],
        disable_normalization: bool,
    ) -> Result<PeriodicWave, WaveError> {
        if real.len() < 2 {
            return Err(WaveError::TooShort(real.len()));
        }
        if imag.len() != real.len() {
            return Err(WaveError::TooShort(imag.len().min(real.len())));
        }
        if real.iter().chain(imag).any(|x| !x.is_finite()) {
            return Err(WaveError::NonFinite);
        }
        // The coefficients are scaled by the table size before the inverse FFT; one that
        // overflows there (or a table that overflows) would make Chromium render NaN.
        let scale = TABLE_SIZE as f32;
        let n = real.len().min(MAX_PARTIALS);
        if real[..n]
            .iter()
            .chain(&imag[..n])
            .any(|x| !(x * scale).is_finite())
        {
            return Err(WaveError::Overflow);
        }
        let tables = create_band_limited_tables(real, imag, disable_normalization);
        if tables.iter().flatten().any(|x| !x.is_finite()) {
            return Err(WaveError::Overflow);
        }
        Ok(PeriodicWave {
            kind: WaveKind::Custom,
            tables,
        })
    }

    #[cfg(test)]
    pub fn kind(&self) -> WaveKind {
        self.kind
    }

    /// Table `range` (0 has every partial, 35 the fewest).
    #[cfg(test)]
    pub fn table(&self, range: usize) -> &[f32] {
        &self.tables[range]
    }

    /// The scalar `WaveDataForFundamentalFrequency`: `(lower, higher, factor)`.
    pub fn wave_data(&self, frequency: f32) -> (&[f32], &[f32], f32) {
        let (lo, hi, factor) = select_scalar(frequency);
        (&self.tables[lo], &self.tables[hi], factor)
    }

    /// The four-lane NEON `WaveDataForFundamentalFrequency`.
    pub fn wave_data4(&self, frequency: [f32; 4]) -> ([&[f32]; 4], [&[f32]; 4], [f32; 4]) {
        let (lo, hi, factor) = select_neon(frequency);
        let t = |i: usize| self.tables[i].as_slice();
        (
            [t(lo[0]), t(lo[1]), t(lo[2]), t(lo[3])],
            [t(hi[0]), t(hi[1]), t(hi[2]), t(hi[3])],
            factor,
        )
    }
}

/// `CreateBandLimitedTables(real, imag, disable_normalization)`.
fn create_band_limited_tables(
    real: &[f32],
    imag: &[f32],
    disable_normalization: bool,
) -> Vec<Vec<f32>> {
    let mut normalization_scale = 0.5f32;
    let components = real.len().min(MAX_PARTIALS);
    let mut frame = FftFrame::new(TABLE_SIZE).expect("the table size is even");
    let mut tables = Vec::with_capacity(NUM_RANGES);
    for range in 0..NUM_RANGES {
        let scale = TABLE_SIZE as f32;
        for i in 0..components {
            frame.real[i] = real[i] * scale;
            frame.imag[i] = imag[i] * -scale;
        }
        let partials = partials_for_range(range);
        for i in components.min(partials + 1)..MAX_PARTIALS {
            frame.real[i] = 0.0;
            frame.imag[i] = 0.0;
        }
        frame.real[0] = 0.0;
        frame.imag[0] = 0.0;
        let mut table = vec![0.0f32; TABLE_SIZE];
        frame.do_inverse_fft(&mut table);
        if !disable_normalization && range == 0 {
            // `Vmaxmgv`: the largest magnitude.
            let max = table.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            if max != 0.0 {
                normalization_scale = 1.0 / max;
            }
        }
        for x in table.iter_mut() {
            *x *= normalization_scale;
        }
        tables.push(table);
    }
    tables
}

/// `table[i & mask]` helper for the oscillator.
#[cfg(test)]
pub fn at(table: &[f32], i: u32) -> f32 {
    table[i as usize & MASK]
}

// ---- the tables of one run

/// How many custom waves the cache keeps (each is 36 x 4096 floats); it is emptied when full.
const CUSTOM_CACHE_LIMIT: usize = 64;

/// The tables of one engine run: the four built-ins, built on first use, and custom waves by
/// their coefficients.
#[derive(Debug, Default)]
pub struct WaveTables {
    builtins: HashMap<WaveKind, Rc<PeriodicWave>>,
    custom: HashMap<(Vec<u32>, Vec<u32>), Rc<PeriodicWave>>,
}

impl WaveTables {
    pub fn new() -> WaveTables {
        WaveTables::default()
    }

    /// The built-in wave of `kind` (`Context()->GetPeriodicWave(type)`); `None` for `Custom`.
    pub fn builtin(&mut self, kind: WaveKind) -> Option<Rc<PeriodicWave>> {
        if kind == WaveKind::Custom {
            return None;
        }
        Some(
            self.builtins
                .entry(kind)
                .or_insert_with(|| Rc::new(PeriodicWave::basic(kind).expect("a built-in kind")))
                .clone(),
        )
    }

    /// `createPeriodicWave(real, imag)`, cached by the coefficients that shape the tables (the
    /// first 2048 of each array; the rest only matter for the finiteness check).
    pub fn custom(&mut self, real: &[f32], imag: &[f32]) -> Result<Rc<PeriodicWave>, WaveError> {
        if real.len() < 2 {
            return Err(WaveError::TooShort(real.len()));
        }
        if real.iter().chain(imag).any(|x| !x.is_finite()) {
            return Err(WaveError::NonFinite);
        }
        let n = real.len().min(MAX_PARTIALS);
        let key = (
            real[..n].iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            imag[..n.min(imag.len())]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
        );
        if let Some(w) = self.custom.get(&key) {
            return Ok(w.clone());
        }
        let wave = Rc::new(PeriodicWave::custom(real, imag)?);
        if self.custom.len() >= CUSTOM_CACHE_LIMIT {
            self.custom.clear();
        }
        self.custom.insert(key, wave.clone());
        Ok(wave)
    }

    /// The number of cached custom waves (tests).
    #[cfg(test)]
    pub fn custom_len(&self) -> usize {
        self.custom.len()
    }
}

// ---- superdough's waveformN

/// The wave types `waveformN` knows (`terms`).
fn terms(kind: &str, n: f64) -> Option<(f64, f64)> {
    match kind {
        "sawtooth" => Some((0.0, -1.0 / n)),
        "square" => Some((0.0, if n % 2.0 == 0.0 { 0.0 } else { 1.0 / n })),
        "triangle" => Some((if n % 2.0 == 0.0 { 0.0 } else { 1.0 / (n * n) }, 0.0)),
        "user" => Some((0.0, 1.0)),
        _ => None,
    }
}

/// JS `x?.[n]` for a decoded control value: array elements, string characters (UTF-16 code
/// units), object members by `String(n)`; `None` for `undefined`.
fn index_js(v: &Value, n: usize) -> Option<Value> {
    match v {
        Value::Arr(xs) => xs.get(n).cloned(),
        Value::Str(s) => {
            let units: Vec<u16> = s.encode_utf16().collect();
            units
                .get(n)
                .map(|u| Value::Str(String::from_utf16_lossy(&[*u])))
        }
        Value::Obj(m) => m.get(&n.to_string()).cloned(),
        _ => None,
    }
}

/// JS `ToIndex(x)` of `new Float32Array(x)` for a non-object `x`.
fn to_index(x: &Value) -> Result<f64, WaveError> {
    let n = js_to_number(x);
    let n = if n.is_nan() { 0.0 } else { n.trunc() };
    if !(0.0..=9_007_199_254_740_991.0).contains(&n) {
        return Err(WaveError::InvalidLength(crate::js::number_to_string(
            js_to_number(x),
        )));
    }
    Ok(n)
}

/// Whether `waveformN` is used: superdough's `!partials || partials?.length === 0 || s ===
/// 'sine'` is false. `partials` is `value.partials ?? value.n`.
pub fn uses_partials(partials: Option<&Value>) -> bool {
    match partials {
        None => false,
        Some(v) => {
            crate::controls::js_truthy(v)
                && !matches!(v, Value::Arr(xs) if xs.is_empty())
                && !matches!(v, Value::Str(s) if s.is_empty())
        }
    }
}

/// `waveformN(partials, phases, type)` (synth.mjs:459-502): the `real` and `imag` arrays of the
/// custom wave, as `Float32Array`s of length `len + 1` (index 0, the DC term, stays 0). Only the
/// first [`MAX_PARTIALS`] entries are returned (Blink ignores the rest); the remaining entries
/// are still checked for the non-finite values `createPeriodicWave` would reject.
pub fn waveform_n(
    partials: &Value,
    phases: Option<&Value>,
    kind: &str,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    // `isList = typeof partials === 'object'`: arrays and objects (an object has no length).
    let (len, list): (f64, Option<&Value>) = match partials {
        Value::Arr(xs) => (xs.len() as f64, Some(partials)),
        // A `Set` has `size`, not `length`: NaN like an object.
        Value::Obj(_) | Value::Set(_) | Value::Unsupported(_) => (f64::NAN, Some(partials)),
        other => (to_index(other).map_err(|e| e.to_string())?, None),
    };
    if terms(kind, 1.0).is_none() {
        return Err(format!("Error: unknown wave type {kind}"));
    }
    // `new Float32Array(len + 1)`; an object's `undefined` length gives 0 entries.
    let size = if len.is_nan() { 0.0 } else { len + 1.0 };
    let kept = (size as usize).min(MAX_PARTIALS);
    let mut real = vec![0.0f32; kept];
    let mut imag = vec![0.0f32; kept];
    // Entries past the phases list (and, for a numeric count, past MAX_PARTIALS) are 1 rotated by
    // 0: always finite, so the loop can stop early.
    let phase_len = match phases {
        Some(Value::Arr(xs)) => xs.len(),
        Some(Value::Str(s)) => s.encode_utf16().count(),
        Some(Value::Obj(m)) => m.len().max(MAX_PARTIALS),
        _ => 0,
    };
    let count = if len.is_nan() { 0 } else { len as usize };
    let last = match list {
        Some(_) => count,
        None => count.min(MAX_PARTIALS.max(phase_len)),
    };
    for n in 0..last {
        let mag = match list {
            Some(v) => index_js(v, n).map_or(f64::NAN, |x| js_to_number(&x)),
            None => 1.0,
        };
        let (r, i) = terms(kind, (n + 1) as f64).expect("checked above");
        let mut re = r * mag;
        let mut im = i * mag;
        let phase = phases.and_then(|p| index_js(p, n));
        let phase = match phase {
            None | Some(Value::Null) => Value::Num(0.0),
            Some(p) => p,
        };
        if !matches!(phase, Value::Num(x) if x == 0.0) {
            // `PI2 * phase` coerces the phase with `ToNumber`.
            let p = js_to_number(&phase);
            let c = libm::cos(2.0 * std::f64::consts::PI * p);
            let s = libm::sin(2.0 * std::f64::consts::PI * p);
            let (r0, i0) = (re, im);
            re = c * r0 - s * i0;
            im = s * r0 + c * i0;
        }
        let (re32, im32) = (re as f32, im as f32);
        if !re32.is_finite() || !im32.is_finite() {
            return Err(WaveError::NonFinite.to_string());
        }
        if n + 1 < kept {
            real[n + 1] = re32;
            imag[n + 1] = im32;
        }
    }
    if kept < 2 {
        return Err(WaveError::TooShort(kept).to_string());
    }
    Ok((real, imag))
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::test_vectors::{decode, load, num};

    #[test]
    fn constants_match_blink_at_44100() {
        assert_eq!(TABLE_SIZE, 4096);
        assert_eq!(
            (0.5 + 3.0 * (TABLE_SIZE as f32).log2()) as usize,
            NUM_RANGES
        );
        assert_eq!(NUM_RANGES, 36);
        assert_eq!(RATE_SCALE, 4096.0f32 / 44100.0f32);
        assert_eq!(LOWEST_FUNDAMENTAL, (0.5f64 * 44100.0) as f32 / 2048.0);
        assert!((LOWEST_FUNDAMENTAL - 10.766602).abs() < 1e-5);
    }

    #[test]
    fn partials_per_range() {
        assert_eq!(partials_for_range(0), 2048);
        assert_eq!(partials_for_range(1), 1625); // 2^(-1/3) * 2048 = 1625.5
        assert_eq!(partials_for_range(3), 1024);
        assert_eq!(partials_for_range(6), 512);
        assert_eq!(partials_for_range(33), 1);
        assert_eq!(partials_for_range(35), 0);
        for r in 1..NUM_RANGES {
            assert!(partials_for_range(r) <= partials_for_range(r - 1));
        }
    }

    #[test]
    fn builtin_sine_is_normalized_to_peak_one() {
        let w = PeriodicWave::basic(WaveKind::Sine).unwrap();
        let t = w.table(0);
        let peak = t.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert_eq!(peak, 1.0);
        // A sine starting at 0 with a positive slope; quarter period is the peak.
        assert!(t[0].abs() < 1e-6);
        assert!((t[1024] - 1.0).abs() < 1e-6);
        assert!((t[3072] + 1.0).abs() < 1e-6);
        for (i, x) in t.iter().enumerate() {
            let want = (2.0 * std::f64::consts::PI * i as f64 / 4096.0).sin();
            assert!((f64::from(*x) - want).abs() < 1e-6, "{i}");
        }
        // Ranges up to 33 keep the fundamental (the sine has no other partial); 34 and 35 keep
        // no partial at all.
        assert_eq!(w.table(33), w.table(0));
        assert!(w.table(34).iter().all(|x| *x == 0.0));
        assert!(w.table(35).iter().all(|x| *x == 0.0));
    }

    #[test]
    fn builtin_shapes_and_normalization_scale() {
        for kind in [WaveKind::Square, WaveKind::Sawtooth, WaveKind::Triangle] {
            let w = PeriodicWave::basic(kind).unwrap();
            let peak0 = w.table(0).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            // `x * (1 / peak)` is 1 within a float rounding.
            assert!((peak0 - 1.0).abs() <= f32::EPSILON, "{kind:?} {peak0}");
            // The scale of range 0 applies to every range, so the band-limited ranges peak
            // differently (Gibbs ripple of fewer partials).
            let peak20 = w.table(20).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak20 > 0.5 && peak20 < 1.2, "{kind:?} {peak20}");
            assert_eq!(w.kind(), kind);
        }
        // Positive slope at time 0 for all of them; square is near +1 at a quarter period,
        // triangle peaks there, sawtooth is near +1 just before half.
        let sq = PeriodicWave::basic(WaveKind::Square).unwrap();
        assert!(sq.table(0)[1024] > 0.8);
        assert!(sq.table(0)[3072] < -0.8);
        let tri = PeriodicWave::basic(WaveKind::Triangle).unwrap();
        assert!((tri.table(0)[1024] - 1.0).abs() < 1e-3);
        let saw = PeriodicWave::basic(WaveKind::Sawtooth).unwrap();
        assert!(saw.table(0)[2000] > 0.8);
        assert!(saw.table(0)[2100] < -0.8);
        assert!(PeriodicWave::basic(WaveKind::Custom).is_none());
    }

    #[test]
    fn disabled_normalization_scales_by_one_half() {
        let real = [0.0, 0.0];
        let imag = [0.0, 1.0];
        let a = PeriodicWave::custom_with(&real, &imag, true).unwrap();
        let peak = a.table(0).iter().fold(0.0f32, |m, x| m.max(x.abs()));
        // The inverse FFT of `imag[1] * -fftSize` gives 2 * b * sin; times 0.5.
        assert!((peak - 1.0).abs() < 1e-6, "{peak}");
        let b = PeriodicWave::custom_with(&real, &[0.0, 3.0], true).unwrap();
        let peak = b.table(0).iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak - 3.0).abs() < 1e-5, "{peak}");
        let c = PeriodicWave::custom(&real, &[0.0, 3.0]).unwrap();
        let peak = c.table(0).iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert_eq!(peak, 1.0);
    }

    #[test]
    fn custom_sine_equals_the_builtin() {
        let w = PeriodicWave::custom(&[0.0, 0.0], &[0.0, 1.0]).unwrap();
        let s = PeriodicWave::basic(WaveKind::Sine).unwrap();
        for r in 0..NUM_RANGES {
            assert_eq!(w.table(r), s.table(r), "range {r}");
        }
        assert_eq!(w.kind(), WaveKind::Custom);
    }

    #[test]
    fn custom_errors() {
        assert_eq!(
            PeriodicWave::custom(&[0.0], &[0.0]),
            Err(WaveError::TooShort(1))
        );
        assert_eq!(
            PeriodicWave::custom(&[0.0, f32::NAN], &[0.0, 1.0]),
            Err(WaveError::NonFinite)
        );
        assert_eq!(
            PeriodicWave::custom(&[0.0, 0.0], &[0.0, f32::INFINITY]),
            Err(WaveError::NonFinite)
        );
        assert_eq!(
            PeriodicWave::custom(&[0.0, 0.0], &[0.0, 1e38]),
            Err(WaveError::Overflow)
        );
        assert!(
            WaveError::TooShort(1)
                .to_string()
                .starts_with("IndexSizeError")
        );
        assert!(WaveError::NonFinite.to_string().starts_with("TypeError"));
    }

    #[test]
    fn components_above_the_range_limit_are_culled() {
        // Partial 1500 survives in range 0 (2048 partials) and range 1 (1625), not in range 2.
        let mut imag = vec![0.0f32; 1501];
        imag[1500] = 1.0;
        let real = vec![0.0f32; 1501];
        let w = PeriodicWave::custom(&real, &imag).unwrap();
        assert!(w.table(0).iter().any(|x| *x != 0.0));
        assert!(w.table(1).iter().any(|x| *x != 0.0));
        assert!(w.table(2).iter().all(|x| *x == 0.0));
        // Coefficients past 2048 are ignored.
        let mut long = vec![0.0f32; 5000];
        long[1] = 1.0;
        let mut long_real = vec![0.0f32; 5000];
        long_real[4000] = 7.0;
        let a = PeriodicWave::custom(&long_real, &long).unwrap();
        let b = PeriodicWave::custom(&[0.0, 0.0], &[0.0, 1.0]).unwrap();
        assert_eq!(a.table(0), b.table(0));
        // DC is always cleared.
        let dc = PeriodicWave::custom(&[5.0, 0.0], &[0.0, 1.0]).unwrap();
        assert_eq!(dc.table(0), b.table(0));
    }

    #[test]
    fn table_selection_edges() {
        // Frequency 0 uses ratio 0.5: pitch range 1 - 3 = -2, clamped to 0.
        assert_eq!(select_scalar(0.0), (1, 0, 0.0));
        assert_eq!(select_neon([0.0; 4]).2, [0.0; 4]);
        // The lowest fundamental is pitch range exactly 1.
        assert_eq!(select_scalar(LOWEST_FUNDAMENTAL), (2, 1, 0.0));
        // Very high frequencies clamp to 35, whose two tables are the same.
        assert_eq!(select_scalar(1e30), (35, 35, 0.0));
        assert_eq!(select_scalar(22050.0).1, 34);
        let (lo, hi, f) = select_neon([22050.0, 1e30, -1e30, f32::INFINITY]);
        assert_eq!(hi, [34, 35, 35, 35]);
        assert_eq!(lo, [35, 35, 35, 35]);
        assert_eq!(f[1], 0.0);
        // Negative frequencies alias to positive ones.
        assert_eq!(select_scalar(-440.0), select_scalar(440.0));
        // NaN: the scalar ratio is 0.5 (NaN > 0 is false); NEON likewise.
        assert_eq!(select_scalar(f32::NAN), (1, 0, 0.0));
        assert_eq!(select_neon([f32::NAN; 4]).1, [0; 4]);
        // Between ranges the factor is the fractional part; scalar and NEON agree closely.
        for f in [55.0f32, 440.0, 1000.0, 4186.0, 12345.6] {
            let (lo, hi, fa) = select_scalar(f);
            let (lo4, hi4, fa4) = select_neon([f; 4]);
            assert_eq!((lo, hi), (lo4[0], hi4[0]), "{f}");
            assert_eq!(lo, hi + 1);
            assert!(
                (fa - fa4[0]).abs() < 1e-5 && (0.0..1.0).contains(&fa),
                "{f}"
            );
        }
        // 440 Hz: 1 + log2(440 / 10.7666) * 3 = 17.06...
        let (_, hi, fa) = select_scalar(440.0);
        assert_eq!(hi, 17);
        assert!((fa - 0.0587).abs() < 1e-3, "{fa}");
    }

    #[test]
    fn wave_data_returns_the_selected_tables() {
        let w = PeriodicWave::basic(WaveKind::Sawtooth).unwrap();
        let (lo, hi, f) = w.wave_data(440.0);
        assert_eq!(lo, w.table(18));
        assert_eq!(hi, w.table(17));
        assert!(f > 0.0);
        let (lo4, hi4, _) = w.wave_data4([440.0, 0.0, 22050.0, 55.0]);
        assert_eq!(lo4[0], w.table(18));
        assert_eq!(hi4[1], w.table(0));
        assert_eq!(lo4[2], w.table(35));
        assert_eq!(at(w.table(0), 4096 + 5), w.table(0)[5]);
    }

    #[test]
    fn wave_tables_cache() {
        let mut t = WaveTables::new();
        let a = t.builtin(WaveKind::Sine).unwrap();
        let b = t.builtin(WaveKind::Sine).unwrap();
        assert!(Rc::ptr_eq(&a, &b));
        assert!(t.builtin(WaveKind::Custom).is_none());
        let c = t.custom(&[0.0, 0.0, 0.0], &[0.0, 1.0, 0.5]).unwrap();
        let d = t.custom(&[0.0, 0.0, 0.0], &[0.0, 1.0, 0.5]).unwrap();
        assert!(Rc::ptr_eq(&c, &d));
        assert_eq!(t.custom_len(), 1);
        let e = t.custom(&[0.0, 0.0, 0.0], &[0.0, 1.0, 0.25]).unwrap();
        assert!(!Rc::ptr_eq(&c, &e));
        assert_eq!(t.custom_len(), 2);
        assert_eq!(
            t.custom(&[0.0], &[0.0]).unwrap_err(),
            WaveError::TooShort(1)
        );
        assert_eq!(
            t.custom(&[0.0, f32::NAN], &[0.0, 0.0]).unwrap_err(),
            WaveError::NonFinite
        );
        for k in 0..CUSTOM_CACHE_LIMIT + 3 {
            t.custom(&[0.0, 0.0], &[0.0, 1.0 + k as f32]).unwrap();
        }
        assert!(t.custom_len() <= CUSTOM_CACHE_LIMIT);
    }

    fn arr(xs: &[f64]) -> Value {
        Value::Arr(xs.iter().map(|x| Value::Num(*x)).collect())
    }

    #[test]
    fn waveform_n_terms() {
        let (re, im) = waveform_n(&Value::Num(4.0), None, "sawtooth").unwrap();
        assert_eq!(re, vec![0.0; 5]);
        assert_eq!(im, vec![0.0, -1.0, -0.5, -1.0 / 3.0, -0.25]);
        let (_, im) = waveform_n(&Value::Num(4.0), None, "square").unwrap();
        assert_eq!(im, vec![0.0, 1.0, 0.0, 1.0 / 3.0, 0.0]);
        let (re, im) = waveform_n(&Value::Num(3.0), None, "triangle").unwrap();
        assert_eq!(re, vec![0.0, 1.0, 0.0, 1.0 / 9.0]);
        assert_eq!(im, vec![0.0; 4]);
        let (_, im) = waveform_n(&arr(&[1.0, 0.5]), None, "user").unwrap();
        assert_eq!(im, vec![0.0, 1.0, 0.5]);
        // Magnitudes scale the terms; strings and null coerce like JS.
        let mags = Value::Arr(vec![Value::Str("2".into()), Value::Null, Value::Num(0.5)]);
        let (_, im) = waveform_n(&mags, None, "sawtooth").unwrap();
        assert_eq!(im, vec![0.0, -2.0, -0.0, -0.5 / 3.0_f32]);
    }

    #[test]
    fn waveform_n_phases_rotate() {
        let (re, im) = waveform_n(&Value::Num(2.0), Some(&arr(&[0.25, 0.5])), "user").unwrap();
        // (0, 1) rotated by a quarter turn is (-1, 0); by half a turn (0, -1).
        assert_eq!(re[1], -1.0);
        assert!(im[1].abs() < 1e-16);
        assert!(re[2].abs() < 1e-15);
        assert_eq!(im[2], -1.0);
        // A phase of 0 (or a missing / null one) does not rotate.
        let (_, im) = waveform_n(
            &Value::Num(2.0),
            Some(&Value::Arr(vec![Value::Null])),
            "user",
        )
        .unwrap();
        assert_eq!(im, vec![0.0, 1.0, 1.0]);
        // A numeric `phases` has no elements.
        let (_, im) = waveform_n(&Value::Num(1.0), Some(&Value::Num(0.25)), "user").unwrap();
        assert_eq!(im, vec![0.0, 1.0]);
        // A string phases indexes characters: "5" -> 5 turns (identity up to rounding).
        let (re, _) = waveform_n(&Value::Num(1.0), Some(&Value::Str("5".into())), "user").unwrap();
        assert!(re[1].abs() < 1e-14);
    }

    #[test]
    fn waveform_n_lengths_and_errors() {
        // A fractional count truncates; a string count parses; true is 1.
        assert_eq!(
            waveform_n(&Value::Num(2.7), None, "user").unwrap().0.len(),
            3
        );
        assert_eq!(
            waveform_n(&Value::Str("3".into()), None, "user")
                .unwrap()
                .0
                .len(),
            4
        );
        assert_eq!(
            waveform_n(&Value::Bool(true), None, "user")
                .unwrap()
                .0
                .len(),
            2
        );
        // Length 0 (a non-numeric string, an object) leaves real/imag of length 1.
        assert!(
            waveform_n(&Value::Str("abc".into()), None, "user")
                .unwrap_err()
                .starts_with("IndexSizeError")
        );
        assert!(
            waveform_n(&Value::Obj(Default::default()), None, "user")
                .unwrap_err()
                .starts_with("IndexSizeError")
        );
        assert_eq!(
            waveform_n(&Value::Num(-1.0), None, "user").unwrap_err(),
            "RangeError: Invalid typed array length: -1"
        );
        assert_eq!(
            waveform_n(&Value::Num(1.0), None, "nope").unwrap_err(),
            "Error: unknown wave type nope"
        );
        // Non-finite products (Infinity * 0, NaN magnitudes) are rejected by createPeriodicWave.
        let bad = arr(&[1.0, f64::INFINITY]);
        assert!(
            waveform_n(&bad, None, "triangle")
                .unwrap_err()
                .starts_with("TypeError")
        );
        let nested = Value::Arr(vec![Value::Arr(vec![])]);
        assert!(
            waveform_n(&nested, None, "user")
                .unwrap_err()
                .starts_with("TypeError")
        );
        // 1e39 overflows a float.
        assert!(
            waveform_n(&arr(&[1e39]), None, "user")
                .unwrap_err()
                .starts_with("TypeError")
        );
        // A huge count only builds the first 2048 entries.
        let (re, im) = waveform_n(&Value::Num(1e12), None, "sawtooth").unwrap();
        assert_eq!((re.len(), im.len()), (MAX_PARTIALS, MAX_PARTIALS));
        assert_eq!(im[2047], -1.0 / 2047.0);
        // A bad phase far past 2048 is still found.
        let mut phases = vec![Value::Num(0.0); 3000];
        phases[2999] = Value::Str("x".into());
        assert!(
            waveform_n(&Value::Num(5000.0), Some(&Value::Arr(phases)), "user")
                .unwrap_err()
                .starts_with("TypeError")
        );
    }

    #[test]
    fn uses_partials_rules() {
        assert!(!uses_partials(None));
        assert!(!uses_partials(Some(&Value::Num(0.0))));
        assert!(!uses_partials(Some(&Value::Num(f64::NAN))));
        assert!(!uses_partials(Some(&Value::Null)));
        assert!(!uses_partials(Some(&Value::Str(String::new()))));
        assert!(!uses_partials(Some(&Value::Arr(vec![]))));
        assert!(uses_partials(Some(&Value::Num(3.0))));
        assert!(uses_partials(Some(&Value::Num(-1.0))));
        assert!(uses_partials(Some(&Value::Str("abc".into()))));
        assert!(uses_partials(Some(&arr(&[0.0]))));
    }

    #[test]
    fn waveform_n_matches_the_js_vectors() {
        let file = load("waveform-n.json");
        let cases = file["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let partials = decode(&case["partials"]);
            let phases = case.get("phases").map(decode);
            let kind = case["type"].as_str().unwrap();
            let got = waveform_n(&partials, phases.as_ref(), kind);
            match case.get("error") {
                // The error class (Bun's JavaScriptCore words its messages differently from V8;
                // `TypeError` marks the non-finite coefficients `createPeriodicWave` rejects,
                // which the stub context used for the vectors does not check itself).
                Some(e) => {
                    let msg = got.expect_err(&format!("{case}"));
                    let class = e.as_str().unwrap();
                    assert!(msg.starts_with(class), "{case}: {msg}");
                }
                None => {
                    let (re, im) = got.unwrap_or_else(|e| panic!("{case}: {e}"));
                    for (name, got, want) in
                        [("real", &re, &case["real"]), ("imag", &im, &case["imag"])]
                    {
                        let want: Vec<f64> = want.as_array().unwrap().iter().map(num).collect();
                        assert_eq!(got.len(), want.len().min(MAX_PARTIALS), "{case} {name}");
                        for (k, (a, b)) in got.iter().zip(&want).enumerate() {
                            assert!(
                                within_ulps32(*a, *b as f32, 1),
                                "{case} {name}[{k}]: {a} vs {b}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Floats within `n` units in the last place (equal values, including both zeros, pass).
    fn within_ulps32(a: f32, b: f32, n: u32) -> bool {
        if a == b {
            return true;
        }
        if a.is_nan() || b.is_nan() || a.is_sign_negative() != b.is_sign_negative() {
            return false;
        }
        a.to_bits().abs_diff(b.to_bits()) <= n
    }
}

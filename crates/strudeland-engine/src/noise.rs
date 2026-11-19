// Port of Blink (Chromium 156.0.8078.4) modules/webaudio/audio_buffer_source_handler.cc.
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.

//! Superdough's noise buffers (`noise.mjs`: white, pink, brown, crackle) and the looping
//! `AudioBufferSourceNode` that plays them.
//!
//! The buffers are generated bit for bit as `getNoiseBuffer` does: the stores go into a
//! `Float32Array`, so each output rounds to `f32` and the brown recurrence reads the rounded value
//! back; pink rounds twice (the sum, then `*= 0.11` on the stored value). White, pink and brown
//! are cached for the run (they depend only on the audio seed, the type and the sample rate);
//! crackle is regenerated for every voice from the voice seed.
//!
//! Playback follows `AudioBufferSourceHandler::RenderFromBuffer` at playback rate 1 with loop on
//! over the whole buffer: the source starts on `TimeToSampleFrame(start, RoundUp)` and, when the
//! true start time lies before that frame (a negative `start_frame_offset`), the read index skips
//! the fraction (`virtual_read_index += |offset|`), which leaves it fractional, so every later
//! frame takes the linear-interpolation path (`(1 - f) * s[i] + f * s[i + 1]` in double); an
//! integral read index takes the copying fast path. Both wrap at the buffer end.

use std::rc::Rc;

use crate::QUANTUM;
use crate::js::{HashPart, make_random, random_from_seed};
use crate::osc::{Schedule, ScheduleError};

/// Frames of every noise buffer: `2 * sampleRate`.
pub const NOISE_FRAMES: usize = 2 * crate::SAMPLE_RATE as usize;

/// The loop start frame (`loopStart` is never set, so the loop is the whole buffer).
const LOOP_START: f64 = 0.0;

/// `getNoiseOscillator`'s `density` default (crackle hit probability `density * 0.01`).
pub const DEFAULT_DENSITY: f64 = 0.02;

/// The density FM noise modulators pass (`getNoiseBuffer(type, 2, voiceSeed)`).
pub const FM_DENSITY: f64 = 2.0;

/// The noise types (`noises` in helpers.mjs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseKind {
    White,
    Pink,
    Brown,
    Crackle,
}

impl NoiseKind {
    /// The kind a sound or `fmwave` name selects (exact names, as `noises.includes` compares).
    pub fn from_name(name: &str) -> Option<NoiseKind> {
        match name {
            "white" => Some(NoiseKind::White),
            "pink" => Some(NoiseKind::Pink),
            "brown" => Some(NoiseKind::Brown),
            "crackle" => Some(NoiseKind::Crackle),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            NoiseKind::White => "white",
            NoiseKind::Pink => "pink",
            NoiseKind::Brown => "brown",
            NoiseKind::Crackle => "crackle",
        }
    }
}

/// `getNoiseBuffer(type, density, voiceSeed)`'s generation loop (noise.mjs:11-56) for audio seed
/// `audio_seed`: white/pink/brown draw from `makeRandom('noise', type, 44100)`, crackle from
/// `randomFromSeed(voiceSeed)` with hit probability `density * 0.01` (a second draw only on a
/// hit). `density` is the JS `Number` of the control (NaN never hits).
pub fn noise_buffer(kind: NoiseKind, audio_seed: i32, density: f64, voice_seed: u32) -> Vec<f32> {
    let mut random = match kind {
        NoiseKind::Crackle => random_from_seed(voice_seed),
        _ => make_random(
            audio_seed,
            &[
                HashPart::Str("noise"),
                HashPart::Str(kind.name()),
                HashPart::Number(f64::from(crate::SAMPLE_RATE)),
            ],
        ),
    };
    let mut output = vec![0.0f32; NOISE_FRAMES];
    match kind {
        NoiseKind::White => {
            for o in output.iter_mut() {
                *o = (random.next() * 2.0 - 1.0) as f32;
            }
        }
        NoiseKind::Brown => {
            let mut last_out = 0.0f64;
            for o in output.iter_mut() {
                let white = random.next() * 2.0 - 1.0;
                *o = ((last_out + 0.02 * white) / 1.02) as f32;
                last_out = f64::from(*o);
            }
        }
        NoiseKind::Pink => {
            let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
                (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for o in output.iter_mut() {
                let white = random.next() * 2.0 - 1.0;
                b0 = 0.99886 * b0 + white * 0.0555179;
                b1 = 0.99332 * b1 + white * 0.0750759;
                b2 = 0.969 * b2 + white * 0.153852;
                b3 = 0.8665 * b3 + white * 0.3104856;
                b4 = 0.55 * b4 + white * 0.5329522;
                b5 = -0.7616 * b5 - white * 0.016898;
                *o = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362) as f32;
                *o = (f64::from(*o) * 0.11) as f32;
                b6 = white * 0.115926;
            }
        }
        NoiseKind::Crackle => {
            let probability = density * 0.01;
            for o in output.iter_mut() {
                *o = if random.next() < probability {
                    (random.next() * 2.0 - 1.0) as f32
                } else {
                    0.0
                };
            }
        }
    }
    output
}

/// `noiseCache` (noise.mjs): the white, pink and brown buffers of the run, built on first use.
/// The audio seed is fixed per run, so the cache is never cleared.
#[derive(Debug, Clone)]
pub struct NoiseCache {
    audio_seed: i32,
    white: Option<Rc<[f32]>>,
    pink: Option<Rc<[f32]>>,
    brown: Option<Rc<[f32]>>,
}

impl NoiseCache {
    pub fn new(audio_seed: i32) -> NoiseCache {
        NoiseCache {
            audio_seed,
            white: None,
            pink: None,
            brown: None,
        }
    }

    /// `getNoiseBuffer(type, density, voiceSeed)`: the cached buffer for white/pink/brown (the
    /// density and voice seed are ignored), a fresh buffer for crackle.
    pub fn get(&mut self, kind: NoiseKind, density: f64, voice_seed: u32) -> Rc<[f32]> {
        let slot = match kind {
            NoiseKind::White => &mut self.white,
            NoiseKind::Pink => &mut self.pink,
            NoiseKind::Brown => &mut self.brown,
            NoiseKind::Crackle => {
                return noise_buffer(kind, self.audio_seed, density, voice_seed).into();
            }
        };
        slot.get_or_insert_with(|| noise_buffer(kind, self.audio_seed, density, voice_seed).into())
            .clone()
    }

    /// Whether the buffer of `kind` has been built (crackle is never cached).
    #[cfg(test)]
    pub fn is_cached(&self, kind: NoiseKind) -> bool {
        match kind {
            NoiseKind::White => self.white.is_some(),
            NoiseKind::Pink => self.pink.is_some(),
            NoiseKind::Brown => self.brown.is_some(),
            NoiseKind::Crackle => false,
        }
    }
}

/// A looping `AudioBufferSourceNode` over a whole mono noise buffer at playback rate 1 (detune
/// 0, loop points unset), as `getNoiseOscillator` and the FM noise modulators build it.
#[derive(Debug, Clone)]
pub struct NoiseSource {
    buffer: Rc<[f32]>,
    schedule: Schedule,
    /// `virtual_read_index_`: the read position in the buffer (fractional after a sub-frame
    /// start).
    virtual_read_index: f64,
}

impl NoiseSource {
    /// A source over `buffer` (`o.buffer = ...; o.loop = true`). `buffer` must not be empty.
    pub fn new(buffer: Rc<[f32]>) -> NoiseSource {
        NoiseSource {
            buffer,
            schedule: Schedule::new(),
            virtual_read_index: 0.0,
        }
    }

    /// `start(when)` at context frame `now` (`start()` is `start(0)`).
    pub fn start(&mut self, when: f64, now: u64) -> Result<(), ScheduleError> {
        self.schedule.start(when, now)
    }

    /// `stop(when)`.
    pub fn stop(&mut self, when: f64) -> Result<(), ScheduleError> {
        self.schedule.stop(when)
    }

    #[cfg(test)]
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    #[cfg(test)]
    pub fn buffer(&self) -> &[f32] {
        &self.buffer
    }

    /// The read position after the last processed quantum.
    #[cfg(test)]
    pub fn read_index(&self) -> f64 {
        self.virtual_read_index
    }

    /// `AudioBufferSourceHandler::Process` for the quantum at `quantum_start` into `out`
    /// (overwritten; zero where the source is silent). Returns whether the output bus carries
    /// signal (the source plays in this quantum).
    pub fn process(&mut self, quantum_start: u64, out: &mut [f32; QUANTUM]) -> bool {
        out.fill(0.0);
        let Some(p) = self.schedule.update(quantum_start) else {
            return false;
        };
        self.render_from_buffer(
            &mut out[p.offset..p.offset + p.frames],
            p.start_frame_offset,
        );
        true
    }

    /// `RenderFromBuffer` at rate 1, looping over `[0, length)`.
    fn render_from_buffer(&mut self, dst: &mut [f32], start_time_offset: f64) {
        let length = self.buffer.len() as f64;
        let mut vri = self.virtual_read_index;
        // Forward loop clamping (never reached: the read index wraps as it is advanced).
        if vri >= length {
            vri = 0.0f64.min(length - 1.0);
            self.virtual_read_index = vri;
        }
        // Forward start-time adjustment: skip the part of the first frame before the start.
        if start_time_offset < 0.0 {
            vri += (start_time_offset * 1.0).abs();
        }
        self.virtual_read_index = if vri == vri.floor() {
            self.fast_path(dst, vri)
        } else {
            self.interpolated_path(dst, vri)
        };
    }

    /// `ProcessFastPath`: copies whole frames, wrapping at the buffer end.
    fn fast_path(&self, dst: &mut [f32], vri: f64) -> f64 {
        let end = self.buffer.len();
        let mut read = vri as usize;
        let mut write = 0;
        let mut remaining = dst.len();
        while remaining > 0 {
            let this_time = end.saturating_sub(read).min(remaining);
            dst[write..write + this_time].copy_from_slice(&self.buffer[read..read + this_time]);
            write += this_time;
            read += this_time;
            remaining -= this_time;
            if read >= end {
                // `end - delta + fmod(overflow, delta)` with no overflow at rate 1.
                read = 0;
            }
        }
        read as f64
    }

    /// `ProcessInterpolatedPath` at rate 1 with loop on: linear interpolation in double, the
    /// sample after the last one being the first.
    fn interpolated_path(&self, dst: &mut [f32], mut vri: f64) -> f64 {
        let end = self.buffer.len() as f64;
        let max_index = self.buffer.len() - 1;
        for d in dst.iter_mut() {
            let read = vri as usize;
            let factor = vri - read as f64;
            let mut read2 = read + 1;
            if (read as f64) < end && read2 as f64 >= end {
                let wrapped = vri + 1.0 - end;
                read2 = if wrapped >= 0.0 { wrapped as usize } else { 0 };
            } else if read2 >= self.buffer.len() {
                read2 = read;
            }
            let read = read.min(max_index);
            let read2 = read2.min(max_index);
            let sample = if read == read2 && read >= 1 {
                let s1 = f64::from(self.buffer[read - 1]);
                let s2 = f64::from(self.buffer[read]);
                s2 + (s2 - s1) * factor
            } else {
                let s1 = f64::from(self.buffer[read]);
                let s2 = f64::from(self.buffer[read2]);
                (1.0 - factor) * s1 + factor * s2
            };
            *d = sample as f32;
            vri += 1.0;
            if vri >= end {
                // `virtual_end_frame - virtual_delta_frames + fmod(overflow, delta)`, the loop
                // spanning the whole buffer.
                let delta = end - LOOP_START;
                let overflow = vri - end;
                vri = end - delta + overflow % delta;
            }
        }
        vri
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::render::frame_for_buffer_start;
    use crate::test_vectors::load;
    use serde_json::Value as J;

    /// FNV-1a (32-bit) over the buffer's little-endian `f32` bytes, as the vectors hash it.
    fn fnv1a(buffer: &[f32]) -> u32 {
        let mut h: u32 = 2166136261;
        for x in buffer {
            for b in x.to_le_bytes() {
                h ^= u32::from(b);
                h = h.wrapping_mul(16777619);
            }
        }
        h
    }

    fn bits(v: &J) -> Vec<u32> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as u32)
            .collect()
    }

    fn check_vectors(file: &str, kind: NoiseKind) {
        let v = load(file);
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 2, "{file}: at least two cases");
        for c in cases {
            let seed = c["seed"].as_i64().unwrap() as i32;
            let voice_seed = c["voiceSeed"].as_u64().unwrap_or(0) as u32;
            let density = c["density"].as_f64().unwrap_or(DEFAULT_DENSITY);
            let buf = noise_buffer(kind, seed, density, voice_seed);
            assert_eq!(buf.len() as u64, c["length"].as_u64().unwrap());
            let head: Vec<u32> = buf[..1024].iter().map(|x| x.to_bits()).collect();
            let tail: Vec<u32> = buf[NOISE_FRAMES - 1024..]
                .iter()
                .map(|x| x.to_bits())
                .collect();
            assert_eq!(head, bits(&c["head"]), "{file} {c:?} head");
            assert_eq!(tail, bits(&c["tail"]), "{file} seed {seed} tail");
            assert_eq!(
                u64::from(fnv1a(&buf)),
                c["fnv1a"].as_u64().unwrap(),
                "{file} seed {seed} hash"
            );
        }
    }

    #[test]
    fn white_matches_the_js_vectors() {
        check_vectors("noise-white.json", NoiseKind::White);
    }

    #[test]
    fn pink_matches_the_js_vectors() {
        check_vectors("noise-pink.json", NoiseKind::Pink);
    }

    #[test]
    fn brown_matches_the_js_vectors() {
        check_vectors("noise-brown.json", NoiseKind::Brown);
    }

    #[test]
    fn crackle_matches_the_js_vectors() {
        check_vectors("noise-crackle.json", NoiseKind::Crackle);
    }

    #[test]
    fn kinds_parse_by_exact_name() {
        for k in [
            NoiseKind::White,
            NoiseKind::Pink,
            NoiseKind::Brown,
            NoiseKind::Crackle,
        ] {
            assert_eq!(NoiseKind::from_name(k.name()), Some(k));
        }
        assert_eq!(NoiseKind::from_name("White"), None);
        assert_eq!(NoiseKind::from_name("noise"), None);
        assert_eq!(NoiseKind::from_name(""), None);
    }

    #[test]
    fn buffers_are_two_seconds_and_bounded() {
        for k in [NoiseKind::White, NoiseKind::Pink, NoiseKind::Brown] {
            let b = noise_buffer(k, 0, DEFAULT_DENSITY, 0);
            assert_eq!(b.len(), 88200);
            assert!(b.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{k:?}");
            assert!(b.iter().any(|x| *x != 0.0));
        }
    }

    #[test]
    fn the_seed_changes_the_cached_kinds_only_through_the_audio_seed() {
        let a = noise_buffer(NoiseKind::White, 0, 0.02, 1);
        let b = noise_buffer(NoiseKind::White, 0, 5.0, 99);
        let c = noise_buffer(NoiseKind::White, 1, 0.02, 1);
        assert_eq!(a, b, "white ignores density and the voice seed");
        assert_ne!(a, c);
        let d = noise_buffer(NoiseKind::Crackle, 0, 2.0, 7);
        let e = noise_buffer(NoiseKind::Crackle, 12345, 2.0, 7);
        let f = noise_buffer(NoiseKind::Crackle, 0, 2.0, 8);
        assert_eq!(d, e, "crackle ignores the audio seed");
        assert_ne!(d, f);
    }

    #[test]
    fn crackle_density_sets_the_hit_rate() {
        let hits = |density: f64| {
            noise_buffer(NoiseKind::Crackle, 0, density, 42)
                .iter()
                .filter(|x| **x != 0.0)
                .count()
        };
        assert_eq!(hits(0.0), 0);
        assert_eq!(hits(-5.0), 0);
        assert_eq!(hits(f64::NAN), 0);
        // probability 0.0002 over 88200 draws: about 18 hits.
        let low = hits(0.02);
        assert!((5..=40).contains(&low), "{low}");
        let mid = hits(2.0);
        assert!((1500..=2000).contains(&mid), "{mid}");
        // At 100 or more every first draw (< 1) hits.
        assert_eq!(hits(100.0), 88200 - zeros_from_second_draw(100.0));
        assert_eq!(
            hits(f64::INFINITY),
            88200 - zeros_from_second_draw(f64::INFINITY)
        );
    }

    /// Hits whose second draw is exactly 0.5 store 0 (`0.5 * 2 - 1`).
    fn zeros_from_second_draw(density: f64) -> usize {
        let mut r = random_from_seed(42);
        let p = density * 0.01;
        let mut n = 0;
        for _ in 0..NOISE_FRAMES {
            if r.next() < p && r.next() == 0.5 {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn two_crackle_buffers_of_one_voice_are_identical() {
        let a = noise_buffer(NoiseKind::Crackle, 0, 2.0, 1234);
        let b = noise_buffer(NoiseKind::Crackle, 0, 2.0, 1234);
        assert_eq!(a, b);
        // A different density changes the threshold, so the hit pattern differs.
        let c = noise_buffer(NoiseKind::Crackle, 0, 0.02, 1234);
        assert_ne!(a, c);
    }

    #[test]
    fn the_cache_shares_buffers_and_never_caches_crackle() {
        let mut cache = NoiseCache::new(3);
        assert!(!cache.is_cached(NoiseKind::Pink));
        let a = cache.get(NoiseKind::Pink, 0.02, 1);
        assert!(cache.is_cached(NoiseKind::Pink));
        let b = cache.get(NoiseKind::Pink, 2.0, 2);
        assert!(Rc::ptr_eq(&a, &b));
        assert_eq!(&*a, &noise_buffer(NoiseKind::Pink, 3, 0.0, 0)[..]);
        let c1 = cache.get(NoiseKind::Crackle, 2.0, 5);
        let c2 = cache.get(NoiseKind::Crackle, 2.0, 5);
        assert!(!Rc::ptr_eq(&c1, &c2));
        assert_eq!(c1, c2);
        assert!(!cache.is_cached(NoiseKind::Crackle));
        assert!(!cache.is_cached(NoiseKind::White));
        assert!(!cache.is_cached(NoiseKind::Brown));
    }

    fn ramp(n: usize) -> Rc<[f32]> {
        (0..n).map(|i| i as f32).collect::<Vec<_>>().into()
    }

    fn run(src: &mut NoiseSource, quanta: u64) -> Vec<f32> {
        let mut out = Vec::new();
        for q in 0..quanta {
            let mut buf = [7.0; QUANTUM];
            src.process(q * QUANTUM as u64, &mut buf);
            out.extend_from_slice(&buf);
        }
        out
    }

    fn close(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() < 1e-9
    }

    #[test]
    fn an_on_frame_start_copies_the_buffer() {
        // 0.25 s is exactly frame 11025: no offset, the integer fast path.
        assert_eq!(0.25 * 44100.0, 11025.0);
        let mut s = NoiseSource::new(ramp(1000));
        s.start(0.25, 0).unwrap();
        let mut out = [7.0; QUANTUM];
        assert!(!s.process(10880, &mut out));
        assert!(out.iter().all(|x| *x == 0.0));
        assert!(s.process(11008, &mut out));
        assert!(out[..17].iter().all(|x| *x == 0.0));
        for i in 17..128 {
            assert_eq!(out[i], (i - 17) as f32);
        }
        assert_eq!(s.read_index(), 111.0);
    }

    #[test]
    fn a_fractional_start_skips_and_interpolates() {
        let mut s = NoiseSource::new(ramp(1000));
        // t * sr = 100.25: the source starts on frame 101 at buffer position 0.75.
        let t = 100.25 / 44100.0;
        assert_eq!(frame_for_buffer_start(t), 101);
        s.start(t, 0).unwrap();
        let out = run(&mut s, 2);
        assert!(out[..101].iter().all(|x| *x == 0.0));
        for i in 101..256 {
            let want = (i - 101) as f64 + (101.0 - t * 44100.0);
            assert!(
                (f64::from(out[i]) - want).abs() < 1e-4,
                "{i}: {} {want}",
                out[i]
            );
        }
        assert_ne!(s.read_index(), s.read_index().floor());
    }

    #[test]
    fn a_start_rounded_down_by_the_1024th_frame_takes_the_fast_path() {
        // 100 + 1/4096 of a frame: TimeToSampleFrame rounds it to 100, the offset is positive,
        // nothing is skipped.
        let t = (100.0 + 1.0 / 4096.0) / 44100.0;
        assert_eq!(frame_for_buffer_start(t), 100);
        let mut s = NoiseSource::new(ramp(1000));
        s.start(t, 0).unwrap();
        let out = run(&mut s, 1);
        assert_eq!(out[99], 0.0);
        assert_eq!(out[100], 0.0);
        assert_eq!(out[101], 1.0);
        assert_eq!(s.read_index(), 28.0);
        // Half a 1024th rounds up (half away from zero): frame 101, skipping almost a frame.
        let t = (100.0 + 1.0 / 2048.0) / 44100.0;
        assert_eq!(frame_for_buffer_start(t), 101);
        let mut s = NoiseSource::new(ramp(1000));
        s.start(t, 0).unwrap();
        let out = run(&mut s, 1);
        assert_eq!(out[100], 0.0);
        assert!(
            (out[101] - (1.0 - 1.0 / 2048.0)).abs() < 1e-6,
            "{}",
            out[101]
        );
    }

    #[test]
    fn the_fast_path_loops_at_the_buffer_end() {
        let mut s = NoiseSource::new(ramp(100));
        s.start(0.0, 0).unwrap();
        let out = run(&mut s, 2);
        for (i, x) in out.iter().enumerate() {
            assert_eq!(*x, (i % 100) as f32, "{i}");
        }
        assert_eq!(s.read_index(), 56.0);
    }

    #[test]
    fn the_interpolated_path_loops_through_the_first_sample() {
        let buf: Rc<[f32]> = vec![1.0f32, 2.0, 3.0, 4.0].into();
        let mut s = NoiseSource::new(buf);
        // Position 0.5 at frame 1.
        s.start(0.5 / 44100.0, 0).unwrap();
        let mut out = [0.0; QUANTUM];
        assert!(s.process(0, &mut out));
        assert_eq!(out[0], 0.0);
        // 0.5 -> 1.5, 1.5 -> 2.5, 2.5 -> 3.5, the last sample blends with the first (4 -> 1).
        assert_eq!(&out[1..6], &[1.5, 2.5, 3.5, 2.5, 1.5]);
        assert!(s.read_index() > 0.0 && s.read_index() < 4.0);
    }

    #[test]
    fn a_one_frame_buffer_loops_on_itself() {
        let buf: Rc<[f32]> = vec![0.25f32].into();
        let mut s = NoiseSource::new(buf.clone());
        s.start(0.0, 0).unwrap();
        let out = run(&mut s, 1);
        assert!(out.iter().all(|x| *x == 0.25));
        let mut s = NoiseSource::new(buf);
        s.start(0.3 / 44100.0, 0).unwrap();
        let out = run(&mut s, 1);
        assert_eq!(out[0], 0.0);
        assert!(out[1..].iter().all(|x| *x == 0.25), "{:?}", &out[..4]);
    }

    #[test]
    fn stop_ends_the_source_and_silence_is_reported() {
        let mut s = NoiseSource::new(ramp(1000));
        s.start(10.5 / 44100.0, 0).unwrap();
        s.stop(140.0 / 44100.0).unwrap();
        let mut out = [0.0; QUANTUM];
        assert!(s.process(0, &mut out));
        assert_eq!(out[10], 0.0);
        assert!(close(out[11], 0.5));
        assert!(s.process(128, &mut out));
        assert!(close(out[11], 128.5));
        assert!(out[12..].iter().all(|x| *x == 0.0));
        assert!(!s.process(256, &mut out));
        assert!(out.iter().all(|x| *x == 0.0));
        assert!(s.schedule().finished());
    }

    #[test]
    fn before_its_start_the_source_is_silent() {
        let mut s = NoiseSource::new(ramp(1000));
        s.start(1.0, 0).unwrap();
        let mut out = [1.0; QUANTUM];
        assert!(!s.process(0, &mut out));
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(s.read_index(), 0.0);
    }

    #[test]
    fn start_at_zero_starts_at_the_scheduling_frame() {
        let mut s = NoiseSource::new(ramp(1000));
        s.start(0.0, 256).unwrap();
        assert_eq!(s.schedule().start_frame(), 256);
        let out = run(&mut s, 3);
        assert!(out[..256].iter().all(|x| *x == 0.0));
        // 256 / 44100 * 44100 is within an ulp of 256: whatever skip that leaves is invisible.
        assert!(close(out[256], 0.0));
        assert!(close(out[300], 44.0));
    }

    #[test]
    fn schedule_errors_are_reported() {
        let mut s = NoiseSource::new(ramp(10));
        assert!(s.stop(1.0).is_err());
        assert!(s.start(f64::NAN, 0).is_err());
        assert!(s.start(-1.0, 0).is_err());
        s.start(0.0, 0).unwrap();
        assert!(s.start(0.0, 0).is_err());
        assert!(s.stop(f64::INFINITY).is_err());
    }
}

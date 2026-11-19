//! What a render covers and how its output is finished: the length request, the worker count, the
//! planning timeout, loudness and the encoder.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::Serialize;
use strudeland_host::LengthRequest;

use crate::RenderError;

/// The default integrated-loudness target, in LUFS: EBU R128's, so dynamic songs reach it by gain
/// alone instead of through the limiter.
pub const DEFAULT_TARGET_LUFS: f64 = -23.0;
/// The default true-peak ceiling, in dBTP.
pub const DEFAULT_TRUE_PEAK_DB: f64 = -1.0;
/// The most worker threads the default picks (`min(available_parallelism, DEFAULT_MAX_JOBS)`).
pub const DEFAULT_MAX_JOBS: usize = 4;

/// Everything [`render`](crate::render) and [`render_audio`](crate::render_audio) take besides the
/// song.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub length: LengthRequest,
    /// Worker threads. `None`: `min(available_parallelism, 4)`. `Some(0)` is a usage error
    /// (`--jobs`). A value above the chunk count is clamped to it (no error).
    pub jobs: Option<usize>,
    /// `None`: no planning timeout. The CLI passes `Some(120 s)` by default.
    pub plan_timeout: Option<Duration>,
    /// Ask the planner for the schedule events (dev binary only).
    pub include_events: bool,
    pub output: OutputOptions,
}

/// How a joined buffer is finished: loudness, then the encoder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputOptions {
    pub loudness: Loudness,
    pub encoder: EncoderOptions,
}

/// Loudness setting: the engine output untouched, or a static gain to `target_lufs` and a
/// true-peak limiter at `true_peak_db`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Loudness {
    Off,
    On { target_lufs: f64, true_peak_db: f64 },
}

impl Default for Loudness {
    fn default() -> Self {
        Loudness::On {
            target_lufs: DEFAULT_TARGET_LUFS,
            true_peak_db: DEFAULT_TRUE_PEAK_DB,
        }
    }
}

/// `--loudness on|off` (the `strudeland render` command and the harness binary).
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnOff {
    On,
    Off,
}

/// An output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Wav,
    Mp3,
    Ogg,
    Opus,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Wav, Format::Mp3, Format::Ogg, Format::Opus];

    /// The file extension (and the flag text).
    pub fn ext(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Mp3 => "mp3",
            Format::Ogg => "ogg",
            Format::Opus => "opus",
        }
    }

    /// The format of a file extension, ignoring case (`"WAV"` is [`Format::Wav`]).
    pub fn from_extension(ext: &str) -> Option<Format> {
        Format::ALL
            .into_iter()
            .find(|f| f.ext().eq_ignore_ascii_case(ext))
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.ext())
    }
}

/// `--format` takes the lowercase names.
impl clap::ValueEnum for Format {
    fn value_variants<'a>() -> &'a [Self] {
        &Format::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.ext()))
    }
}

impl FromStr for Format {
    type Err = String;

    /// The lowercase name (`wav`, `mp3`, `ogg`, `opus`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Format::ALL
            .into_iter()
            .find(|f| f.ext() == s)
            .ok_or_else(|| format!("unknown format {s:?} (wav, mp3, ogg or opus)"))
    }
}

/// WAV sample format (flag text `16`, `24`, `32f`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    B16,
    B24,
    F32,
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::B16, BitDepth::B24, BitDepth::F32];

    /// The flag text.
    pub fn as_str(self) -> &'static str {
        match self {
            BitDepth::B16 => "16",
            BitDepth::B24 => "24",
            BitDepth::F32 => "32f",
        }
    }
}

/// `--bit-depth` takes the flag texts.
impl clap::ValueEnum for BitDepth {
    fn value_variants<'a>() -> &'a [Self] {
        &BitDepth::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

impl FromStr for BitDepth {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        BitDepth::ALL
            .into_iter()
            .find(|b| b.as_str() == s)
            .ok_or_else(|| format!("unknown bit depth {s:?} (16, 24 or 32f)"))
    }
}

/// MP3 rate control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp3Mode {
    Cbr,
    Vbr,
}

impl Mp3Mode {
    pub const ALL: [Mp3Mode; 2] = [Mp3Mode::Cbr, Mp3Mode::Vbr];

    /// The flag text.
    pub fn as_str(self) -> &'static str {
        match self {
            Mp3Mode::Cbr => "cbr",
            Mp3Mode::Vbr => "vbr",
        }
    }
}

/// `--mp3-mode` takes the flag texts.
impl clap::ValueEnum for Mp3Mode {
    fn value_variants<'a>() -> &'a [Self] {
        &Mp3Mode::ALL
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

impl FromStr for Mp3Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Mp3Mode::ALL
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| format!("unknown mp3 mode {s:?} (cbr or vbr)"))
    }
}

/// A format with its encoder settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderOptions {
    Wav {
        bit_depth: BitDepth,
    },
    /// `vbr_quality` 0..=9 (ignored for CBR).
    Mp3 {
        mode: Mp3Mode,
        vbr_quality: u8,
    },
    /// -1..=10, vorbis quality `q/10`.
    Ogg {
        quality: i32,
    },
    /// 6..=510.
    Opus {
        kbps: u32,
    },
}

impl EncoderOptions {
    /// The format these settings encode.
    pub fn format(&self) -> Format {
        match self {
            EncoderOptions::Wav { .. } => Format::Wav,
            EncoderOptions::Mp3 { .. } => Format::Mp3,
            EncoderOptions::Ogg { .. } => Format::Ogg,
            EncoderOptions::Opus { .. } => Format::Opus,
        }
    }

    /// The default settings of `format`: WAV 24-bit, MP3 CBR 320, Ogg quality 6, Opus 160 kbps.
    pub fn default_for(format: Format) -> EncoderOptions {
        match format {
            Format::Wav => EncoderOptions::Wav {
                bit_depth: BitDepth::B24,
            },
            Format::Mp3 => EncoderOptions::Mp3 {
                mode: Mp3Mode::Cbr,
                vbr_quality: 0,
            },
            Format::Ogg => EncoderOptions::Ogg { quality: 6 },
            Format::Opus => EncoderOptions::Opus { kbps: 160 },
        }
    }

    /// The settings for `format` from the encoder flags, on [`default_for`](Self::default_for).
    /// A flag that does not apply to the format, or a quality out of range, is a usage error
    /// naming the flag.
    pub fn from_flags(
        format: Format,
        bit_depth: Option<BitDepth>,
        mp3_mode: Option<Mp3Mode>,
        quality: Option<i32>,
    ) -> Result<EncoderOptions, RenderError> {
        if bit_depth.is_some() && format != Format::Wav {
            return Err(usage("--bit-depth", "--bit-depth only applies to wav"));
        }
        if mp3_mode.is_some() && format != Format::Mp3 {
            return Err(usage("--mp3-mode", "--mp3-mode only applies to mp3"));
        }
        let mut options = EncoderOptions::default_for(format);
        match &mut options {
            EncoderOptions::Wav { bit_depth: b } => {
                if quality.is_some() {
                    return Err(usage("--quality", "--quality does not apply to wav"));
                }
                *b = bit_depth.unwrap_or(*b);
            }
            EncoderOptions::Mp3 { mode, vbr_quality } => {
                *mode = mp3_mode.unwrap_or(*mode);
                match (*mode, quality) {
                    (Mp3Mode::Cbr, Some(_)) => {
                        return Err(usage("--quality", "--quality needs --mp3-mode vbr"));
                    }
                    (Mp3Mode::Cbr, None) | (Mp3Mode::Vbr, None) => {}
                    (Mp3Mode::Vbr, Some(q)) => *vbr_quality = in_range(q, 0, 9, "mp3 vbr")? as u8,
                }
            }
            EncoderOptions::Ogg { quality: oq } => {
                if let Some(q) = quality {
                    *oq = in_range(q, -1, 10, "ogg")?;
                }
            }
            EncoderOptions::Opus { kbps } => {
                if let Some(q) = quality {
                    *kbps = in_range(q, 6, 510, "opus")? as u32;
                }
            }
        }
        Ok(options)
    }
}

fn usage(flag: &'static str, message: impl Into<String>) -> RenderError {
    RenderError::Usage {
        flag,
        message: message.into(),
    }
}

/// `q` if it is in `lo..=hi`, else the `--quality` usage error naming the range and `what`.
fn in_range(q: i32, lo: i32, hi: i32, what: &str) -> Result<i32, RenderError> {
    if (lo..=hi).contains(&q) {
        Ok(q)
    } else {
        Err(usage(
            "--quality",
            format!("--quality must be an integer from {lo} to {hi} for {what}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_of(r: Result<EncoderOptions, RenderError>) -> (&'static str, String) {
        match r {
            Err(RenderError::Usage { flag, message }) => (flag, message),
            other => panic!("not a usage error: {other:?}"),
        }
    }

    #[test]
    fn encode_options_wav() {
        let f = Format::Wav;
        assert_eq!(
            EncoderOptions::from_flags(f, None, None, None).unwrap(),
            EncoderOptions::Wav {
                bit_depth: BitDepth::B24
            }
        );
        for b in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
            assert_eq!(
                EncoderOptions::from_flags(f, Some(b), None, None).unwrap(),
                EncoderOptions::Wav { bit_depth: b }
            );
        }
        assert_eq!(
            usage_of(EncoderOptions::from_flags(
                f,
                None,
                Some(Mp3Mode::Cbr),
                None
            )),
            ("--mp3-mode", "--mp3-mode only applies to mp3".into())
        );
        assert_eq!(
            usage_of(EncoderOptions::from_flags(f, None, None, Some(6))),
            ("--quality", "--quality does not apply to wav".into())
        );
    }

    #[test]
    fn encode_options_mp3() {
        let f = Format::Mp3;
        let cbr = EncoderOptions::Mp3 {
            mode: Mp3Mode::Cbr,
            vbr_quality: 0,
        };
        assert_eq!(
            EncoderOptions::from_flags(f, None, None, None).unwrap(),
            cbr
        );
        assert_eq!(
            EncoderOptions::from_flags(f, None, Some(Mp3Mode::Cbr), None).unwrap(),
            cbr
        );
        for mode in [None, Some(Mp3Mode::Cbr)] {
            assert_eq!(
                usage_of(EncoderOptions::from_flags(f, None, mode, Some(0))),
                ("--quality", "--quality needs --mp3-mode vbr".into()),
                "{mode:?}"
            );
        }
        assert_eq!(
            EncoderOptions::from_flags(f, None, Some(Mp3Mode::Vbr), None).unwrap(),
            EncoderOptions::Mp3 {
                mode: Mp3Mode::Vbr,
                vbr_quality: 0
            }
        );
        for q in 0..=9 {
            assert_eq!(
                EncoderOptions::from_flags(f, None, Some(Mp3Mode::Vbr), Some(q)).unwrap(),
                EncoderOptions::Mp3 {
                    mode: Mp3Mode::Vbr,
                    vbr_quality: q as u8
                }
            );
        }
        for q in [-1, 10, i32::MIN, i32::MAX] {
            assert_eq!(
                usage_of(EncoderOptions::from_flags(
                    f,
                    None,
                    Some(Mp3Mode::Vbr),
                    Some(q)
                )),
                (
                    "--quality",
                    "--quality must be an integer from 0 to 9 for mp3 vbr".into()
                ),
                "{q}"
            );
        }
        assert_eq!(
            usage_of(EncoderOptions::from_flags(
                f,
                Some(BitDepth::B16),
                None,
                None
            )),
            ("--bit-depth", "--bit-depth only applies to wav".into())
        );
    }

    #[test]
    fn encode_options_ogg_and_opus() {
        let cases = [
            (Format::Ogg, -1, 10, 6, "ogg"),
            (Format::Opus, 6, 510, 160, "opus"),
        ];
        let make = |f: Format, q: i32| match f {
            Format::Ogg => EncoderOptions::Ogg { quality: q },
            _ => EncoderOptions::Opus { kbps: q as u32 },
        };
        for (f, lo, hi, default, name) in cases {
            assert_eq!(
                EncoderOptions::from_flags(f, None, None, None).unwrap(),
                make(f, default)
            );
            for q in [lo, lo + 1, hi - 1, hi] {
                assert_eq!(
                    EncoderOptions::from_flags(f, None, None, Some(q)).unwrap(),
                    make(f, q),
                    "{name} {q}"
                );
            }
            for q in [lo - 1, hi + 1, i32::MIN, i32::MAX] {
                assert_eq!(
                    usage_of(EncoderOptions::from_flags(f, None, None, Some(q))),
                    (
                        "--quality",
                        format!("--quality must be an integer from {lo} to {hi} for {name}")
                    ),
                    "{name} {q}"
                );
            }
            assert_eq!(
                usage_of(EncoderOptions::from_flags(
                    f,
                    Some(BitDepth::F32),
                    None,
                    None
                )),
                ("--bit-depth", "--bit-depth only applies to wav".into())
            );
            for m in [Mp3Mode::Cbr, Mp3Mode::Vbr] {
                assert_eq!(
                    usage_of(EncoderOptions::from_flags(f, None, Some(m), None)),
                    ("--mp3-mode", "--mp3-mode only applies to mp3".into())
                );
            }
        }
    }

    #[test]
    fn format_names_and_extensions() {
        for (f, ext) in [
            (Format::Wav, "wav"),
            (Format::Mp3, "mp3"),
            (Format::Ogg, "ogg"),
            (Format::Opus, "opus"),
        ] {
            assert_eq!(f.ext(), ext);
            assert_eq!(f.to_string(), ext);
            assert_eq!(ext.parse::<Format>(), Ok(f), "{ext}");
            assert_eq!(Format::from_extension(ext), Some(f), "{ext}");
            assert_eq!(
                Format::from_extension(&ext.to_uppercase()),
                Some(f),
                "{ext} upper"
            );
            assert!(ext.to_uppercase().parse::<Format>().is_err(), "{ext} upper");
            assert_eq!(EncoderOptions::default_for(f).format(), f);
            assert_eq!(
                serde_json::to_string(&f).unwrap(),
                format!("\"{ext}\""),
                "{ext}"
            );
        }
        assert_eq!(Format::from_extension("flac"), None);
        assert_eq!(Format::from_extension(""), None);
        assert!("flac".parse::<Format>().is_err());
    }

    #[test]
    fn flag_texts_parse() {
        assert_eq!("16".parse::<BitDepth>(), Ok(BitDepth::B16));
        assert_eq!("24".parse::<BitDepth>(), Ok(BitDepth::B24));
        assert_eq!("32f".parse::<BitDepth>(), Ok(BitDepth::F32));
        assert!("32".parse::<BitDepth>().is_err());
        assert_eq!("cbr".parse::<Mp3Mode>(), Ok(Mp3Mode::Cbr));
        assert_eq!("vbr".parse::<Mp3Mode>(), Ok(Mp3Mode::Vbr));
        assert!("abr".parse::<Mp3Mode>().is_err());
    }

    #[test]
    fn defaults() {
        assert_eq!(
            Loudness::default(),
            Loudness::On {
                target_lufs: -23.0,
                true_peak_db: -1.0
            }
        );
        assert_eq!(
            EncoderOptions::default_for(Format::Wav),
            EncoderOptions::Wav {
                bit_depth: BitDepth::B24
            }
        );
        assert_eq!(
            EncoderOptions::default_for(Format::Mp3),
            EncoderOptions::Mp3 {
                mode: Mp3Mode::Cbr,
                vbr_quality: 0
            }
        );
        assert_eq!(
            EncoderOptions::default_for(Format::Ogg),
            EncoderOptions::Ogg { quality: 6 }
        );
        assert_eq!(
            EncoderOptions::default_for(Format::Opus),
            EncoderOptions::Opus { kbps: 160 }
        );
    }
}

//! Encoders: WAV, MP3, Ogg Vorbis and Opus, the verification decoders and the codec seam the
//! finishing loop ([`crate::finish`]) encodes and verifies through.

pub mod mp3;
pub mod ogg;
pub mod opus;
pub mod verify;
pub mod wav;

use std::ops::Range;

use crate::{EncoderOptions, Format, LoopPoints, RenderError};

pub use mp3::Mp3Flush;

/// One encode of a buffer: no leveling, no verification.
pub struct EncodeRequest<'a> {
    /// The channels, full scale +-1.0.
    pub channels: &'a [Vec<f32>],
    /// The input sample rate.
    pub sample_rate: u32,
    /// The format and its settings.
    pub encoder: &'a EncoderOptions,
    /// MP3 only: how the stream is finalized.
    pub mp3_flush: Mp3Flush,
    /// The loop in decoded frames (exclusive end): input-rate frames, 48 kHz for Opus.
    pub loop_range: Option<Range<u64>>,
}

impl<'a> EncodeRequest<'a> {
    /// A plain encode of `channels`: the no-gap MP3 flush and no loop.
    pub fn plain(
        channels: &'a [Vec<f32>],
        sample_rate: u32,
        encoder: &'a EncoderOptions,
    ) -> EncodeRequest<'a> {
        EncodeRequest {
            channels,
            sample_rate,
            encoder,
            mp3_flush: Mp3Flush::NoGap,
            loop_range: None,
        }
    }
}

/// Encodes `request.channels` once with `request.encoder`, with loop metadata where requested.
/// No leveling, no verification.
pub fn encode_once(request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError> {
    let channels = request.channels;
    let sample_rate = request.sample_rate;
    let range = request.loop_range.clone();
    match *request.encoder {
        EncoderOptions::Wav { bit_depth } => {
            wav::encode_wav_with_loop(channels, sample_rate, bit_depth, range)
        }
        EncoderOptions::Mp3 { mode, vbr_quality } => mp3::encode_mp3(
            channels,
            sample_rate,
            mode,
            vbr_quality,
            request.mp3_flush,
            range,
        ),
        EncoderOptions::Ogg { quality } => {
            ogg::encode_ogg_with_loop(channels, sample_rate, quality, range)
        }
        EncoderOptions::Opus { kbps } => {
            opus::encode_opus_with_loop(channels, sample_rate, kbps, range)
        }
    }
}

/// The decoded loop range and its reported points for a loop of `range` input frames: the
/// identity, or each end resampled for Opus (which always decodes at 48 kHz).
pub(crate) fn loop_points(
    format: Format,
    range: &Range<usize>,
    sample_rate: u32,
) -> (Range<u64>, LoopPoints) {
    let (range, rate) = if format == Format::Opus {
        (
            opus::resampled_len(range.start, sample_rate)
                ..opus::resampled_len(range.end, sample_rate),
            opus::OPUS_RATE,
        )
    } else {
        (range.clone(), sample_rate)
    };
    let points = LoopPoints {
        start_frame: range.start as u64,
        length_frames: range.len() as u64,
        sample_rate: rate,
    };
    (range.start as u64..range.end as u64, points)
}

/// Encode passes at most: the first encode and two corrections.
pub(crate) const MAX_ENCODES: u32 = 3;

/// The margin added to a lossy overshoot when the limiter ceiling or the shared gain is lowered,
/// in dB.
pub const CORRECTION_MARGIN_DB: f64 = 0.1;

/// The encoder and the verification decoder, behind a seam so tests can force an overshoot.
pub(crate) trait Codec {
    fn encode(&self, request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError>;
    /// The decoded file's true peak in dBTP.
    fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError>;
}

/// The real encoders and decoders.
pub(crate) struct Real;

impl Codec for Real {
    fn encode(&self, request: &EncodeRequest<'_>) -> Result<Vec<u8>, RenderError> {
        encode_once(request)
    }

    fn decoded_true_peak_db(&self, format: Format, bytes: &[u8]) -> Result<f64, RenderError> {
        verify::decoded_true_peak_db(format, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_points_resample_each_end_for_opus() {
        assert_eq!(
            loop_points(Format::Opus, &(147..294), 44_100),
            (
                160..320,
                LoopPoints {
                    start_frame: 160,
                    length_frames: 160,
                    sample_rate: 48_000,
                }
            )
        );
        assert_eq!(loop_points(Format::Opus, &(0..1), 44_100).0, 0..2);
        for format in [Format::Wav, Format::Ogg, Format::Mp3] {
            assert_eq!(
                loop_points(format, &(147..294), 44_100),
                (
                    147..294,
                    LoopPoints {
                        start_frame: 147,
                        length_frames: 147,
                        sample_rate: 44_100,
                    }
                ),
                "{format:?}"
            );
        }
    }
}

//! Ogg Vorbis with libvorbis (aoTuV): quality VBR with a fixed stream serial, so the same input
//! gives the same bytes.

use std::num::{NonZeroU8, NonZeroU32};

use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

use crate::RenderError;

/// The Ogg stream serial (`"STRD"`).
pub const STREAM_SERIAL: i32 = 0x5354_5244;

/// Frames per block handed to the encoder.
const BLOCK: usize = 4096;

fn err(what: impl std::fmt::Display) -> RenderError {
    RenderError::Encode(format!("ogg: {what}"))
}

/// Encodes a stereo buffer (two equal-length channels) at `sample_rate` with `--quality q`
/// (-1 to 10, vorbis quality `q / 10`).
pub fn encode_ogg(
    channels: &[Vec<f32>],
    sample_rate: u32,
    quality: i32,
) -> Result<Vec<u8>, RenderError> {
    let [left, right] = channels else {
        return Err(err(format!(
            "{} channels (Ogg Vorbis takes two)",
            channels.len()
        )));
    };
    if left.len() != right.len() {
        return Err(err("channel length mismatch"));
    }
    if !(-1..=10).contains(&quality) {
        return Err(err(format!("quality {quality} is not -1 to 10")));
    }
    let rate = NonZeroU32::new(sample_rate).ok_or_else(|| err("sample rate 0"))?;
    let stereo = NonZeroU8::new(2).ok_or_else(|| err("channel count 0"))?;
    let mut builder =
        VorbisEncoderBuilder::new_with_serial(rate, stereo, Vec::new(), STREAM_SERIAL);
    builder
        .bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: quality as f32 / 10.0,
        })
        .comment_tag("ENCODER", "strudeland")
        .map_err(err)?;
    let mut encoder = builder.build().map_err(err)?;
    // Never hand the encoder an empty block: zero frames is libvorbis's end-of-stream signal.
    for (l, r) in left.chunks(BLOCK).zip(right.chunks(BLOCK)) {
        encoder.encode_audio_block([l, r]).map_err(err)?;
    }
    encoder.finish().map_err(err)
}

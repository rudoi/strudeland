//! MP3 with LAME: CBR 320 kbps or VBR V0 to V9, joint stereo, algorithm quality 2, with the LAME
//! tag written over the reserved first frame. The output keeps the input sample rate.

use mp3lame_encoder::{
    Bitrate, Builder, DualPcm, FlushNoGap, Mode, Quality, VbrMode, max_required_buffer_size,
};

use std::num::NonZeroU32;
use std::ops::Range;

use crate::{Mp3Mode, RenderError};

/// Frames per `encode` call.
const BLOCK: usize = 44_100;

fn err(what: impl std::fmt::Display) -> RenderError {
    RenderError::Encode(format!("mp3: {what}"))
}

/// The LAME VBR quality `V<q>`, `q` in 0..=9.
fn vbr_quality(q: u8) -> Result<Quality, RenderError> {
    Ok(match q {
        0 => Quality::Best,
        1 => Quality::SecondBest,
        2 => Quality::NearBest,
        3 => Quality::VeryNice,
        4 => Quality::Nice,
        5 => Quality::Good,
        6 => Quality::Decent,
        7 => Quality::Ok,
        8 => Quality::SecondWorst,
        9 => Quality::Worst,
        _ => return Err(err(format!("VBR quality {q} is not 0 to 9"))),
    })
}

/// Encodes a stereo buffer (two equal-length channels, full scale +-1.0) at `sample_rate`.
pub fn encode_mp3(
    channels: &[Vec<f32>],
    sample_rate: u32,
    mode: Mp3Mode,
    quality: u8,
) -> Result<Vec<u8>, RenderError> {
    encode_mp3_with_loop(channels, sample_rate, mode, quality, None)
}

/// Encodes MP3 with an optional loop in decoded frames (exclusive end), after gapless trimming.
pub fn encode_mp3_with_loop(
    channels: &[Vec<f32>],
    sample_rate: u32,
    mode: Mp3Mode,
    quality: u8,
    loop_range: Option<Range<u64>>,
) -> Result<Vec<u8>, RenderError> {
    let [left, right] = channels else {
        return Err(err(format!("{} channels (MP3 takes two)", channels.len())));
    };
    if left.len() != right.len() {
        return Err(err("channel length mismatch"));
    }
    let mut out = match loop_range {
        Some(range) if range.is_empty() || range.end > left.len() as u64 => {
            return Err(err("loop range is outside the audio"));
        }
        Some(range) => loop_id3_tag(range),
        None => Vec::new(),
    };
    let audio_offset = out.len();
    let builder = Builder::new()
        .ok_or_else(|| RenderError::Encode("LAME could not allocate an encoder".into()))?
        .with_num_channels(2)
        .map_err(err)?
        .with_sample_rate(sample_rate)
        .map_err(err)?
        // LAME would resample low VBR qualities to an MPEG-2 rate; keep the input rate.
        .with_output_sample_rate(NonZeroU32::new(sample_rate))
        .map_err(err)?
        .with_mode(Mode::JointStereo)
        .map_err(err)?
        .with_quality(Quality::NearBest)
        .map_err(err)?
        .with_to_write_vbr_tag(true)
        .map_err(err)?;
    let builder = match mode {
        Mp3Mode::Cbr => builder.with_brate(Bitrate::Kbps320).map_err(err)?,
        Mp3Mode::Vbr => builder
            .with_vbr_mode(VbrMode::Mtrh)
            .map_err(err)?
            .with_vbr_quality(vbr_quality(quality)?)
            .map_err(err)?,
    };
    let mut encoder = builder.build().map_err(err)?;

    for (l, r) in left.chunks(BLOCK).zip(right.chunks(BLOCK)) {
        out.reserve(max_required_buffer_size(l.len()));
        encoder
            .encode_to_vec(DualPcm { left: l, right: r }, &mut out)
            .map_err(err)?;
    }
    out.reserve(max_required_buffer_size(0));
    encoder.flush_to_vec::<FlushNoGap>(&mut out).map_err(err)?;

    // LAME reserved the first frame for the tag (a frame header, then zeros); the real tag replaces it in place.
    let size = encoder.lame_tag_size();
    if size > 0 {
        let mut tag = Vec::with_capacity(size);
        encoder
            .lame_tag_encode_to_vec(&mut tag)
            .ok_or_else(|| err("LAME could not write its tag"))?;
        let at = audio_offset + encoder.id3v2_tag_size();
        let reserved = out
            .get_mut(at..at.saturating_add(tag.len()))
            .ok_or_else(|| err("the output is shorter than the LAME tag"))?;
        reserved.copy_from_slice(&tag);
    }
    Ok(out)
}

/// A leading ID3v2.4 tag with two user-defined Latin-1 text frames.
fn loop_id3_tag(range: Range<u64>) -> Vec<u8> {
    let mut tag = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    for (name, value) in [
        ("LOOPSTART", range.start.to_string()),
        ("LOOPLENGTH", (range.end - range.start).to_string()),
    ] {
        tag.extend_from_slice(b"TXXX");
        tag.extend_from_slice(&synchsafe(2 + name.len() + value.len()));
        tag.extend_from_slice(&[0, 0, 0]); // frame flags, text encoding
        tag.extend_from_slice(name.as_bytes());
        tag.push(0);
        tag.extend_from_slice(value.as_bytes());
    }
    let size = synchsafe(tag.len() - 10);
    tag[6..10].copy_from_slice(&size);
    tag
}

/// ID3v2.4 encodes sizes as big-endian groups of seven bits.
fn synchsafe(size: usize) -> [u8; 4] {
    [21, 14, 7, 0].map(|shift| ((size >> shift) & 0x7f) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_mp3_vbr_quality_is_the_discriminant() {
        for q in 0..=9u8 {
            assert_eq!(vbr_quality(q).unwrap() as u8, q);
        }
        assert!(matches!(vbr_quality(10), Err(RenderError::Encode(_))));
    }
}

//! Opus in Ogg (RFC 7845): the buffer is resampled to 48 kHz, encoded with libopus in 20 ms
//! frames and muxed by hand with a fixed stream serial, so the same input gives the same bytes.

use std::ops::Range;

use ::ogg::writing::{PacketWriteEndInfo, PacketWriter};
use ::opus::{Application, Bitrate, Channels, Encoder};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler};

use crate::RenderError;

/// The Ogg stream serial (`"STRD"`).
pub const STREAM_SERIAL: u32 = 0x5354_5244;
/// Opus's sample rate.
pub const OPUS_RATE: u32 = 48_000;
/// Frames of one 20 ms packet at 48 kHz.
pub const FRAME: usize = 960;
/// The largest packet libopus writes for one frame is 1275 bytes; this leaves room.
const MAX_PACKET: usize = 4000;
/// The vendor string of the `OpusTags` header.
const VENDOR: &[u8] = b"strudeland";

fn err(what: impl std::fmt::Display) -> RenderError {
    RenderError::Encode(format!("opus: {what}"))
}

/// Frames of `frames` input frames at `sample_rate` resampled to 48 kHz:
/// `ceil(frames * 48000 / sample_rate)` (for 44.1 kHz, `ceil(frames * 160 / 147)`).
pub fn resampled_len(frames: usize, sample_rate: u32) -> usize {
    let num = frames as u128 * u128::from(OPUS_RATE);
    let den = u128::from(sample_rate.max(1));
    num.div_ceil(den) as usize
}

/// Resamples two equal-length channels from `sample_rate` to 48 kHz with rubato's FFT resampler
/// (the startup delay trimmed) and returns [`resampled_len`] interleaved stereo frames.
pub fn resample_48k(channels: &[Vec<f32>], sample_rate: u32) -> Result<Vec<f32>, RenderError> {
    let frames = channels.first().map_or(0, Vec::len);
    let m = resampled_len(frames, sample_rate);
    if frames == 0 {
        return Ok(Vec::new());
    }
    let mut resampler = Fft::<f32>::new(
        sample_rate as usize,
        OPUS_RATE as usize,
        1024,
        2,
        FixedSync::Input,
    )
    .map_err(err)?;
    let input = SequentialSliceOfVecs::new(channels, 2, frames).map_err(err)?;
    let mut data = resampler
        .process_all(&input, frames, None)
        .map_err(err)?
        .take_data();
    // rubato rounds `ratio * frames` in f64, which can give one frame past the exact length
    // (at multiples of 147 frames); the exact length is kept.
    if data.len() < 2 * m {
        return Err(err(format!(
            "the resampler returned {} frames, expected {m}",
            data.len() / 2
        )));
    }
    data.truncate(2 * m);
    Ok(data)
}

/// The `OpusHead` identification header (RFC 7845 5.1): version 1, two channels, the pre-skip,
/// the input sample rate, gain 0, mapping family 0.
pub fn opus_head(pre_skip: u16, input_rate: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(19);
    h.extend_from_slice(b"OpusHead");
    h.push(1);
    h.push(2);
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&input_rate.to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes());
    h.push(0);
    h
}

/// The `OpusTags` comment header (RFC 7845 5.2): the vendor string and no comments.
pub fn opus_tags() -> Vec<u8> {
    opus_tags_with_loop(None)
}

fn opus_tags_with_loop(loop_range: Option<Range<u64>>) -> Vec<u8> {
    let mut t = Vec::with_capacity(8 + 4 + VENDOR.len() + 4);
    t.extend_from_slice(b"OpusTags");
    t.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    t.extend_from_slice(VENDOR);
    let count = if loop_range.is_some() { 2u32 } else { 0 };
    t.extend_from_slice(&count.to_le_bytes());
    if let Some(range) = loop_range {
        for comment in [
            format!("LOOPSTART={}", range.start),
            format!("LOOPLENGTH={}", range.end - range.start),
        ] {
            t.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            t.extend_from_slice(comment.as_bytes());
        }
    }
    t
}

/// Encodes a stereo buffer (two equal-length channels) at `sample_rate` as Ogg Opus at `kbps`
/// (6 to 510). The decoder outputs exactly [`resampled_len`] frames after the pre-skip.
pub fn encode_opus(
    channels: &[Vec<f32>],
    sample_rate: u32,
    kbps: u32,
) -> Result<Vec<u8>, RenderError> {
    encode_opus_with_loop(channels, sample_rate, kbps, None)
}

/// Encodes Ogg Opus with an optional loop in decoded 48 kHz frames (exclusive end).
pub fn encode_opus_with_loop(
    channels: &[Vec<f32>],
    sample_rate: u32,
    kbps: u32,
    loop_range: Option<Range<u64>>,
) -> Result<Vec<u8>, RenderError> {
    let [left, right] = channels else {
        return Err(err(format!("{} channels (Opus takes two)", channels.len())));
    };
    if left.len() != right.len() {
        return Err(err("channel length mismatch"));
    }
    if sample_rate == 0 {
        return Err(err("sample rate 0"));
    }
    if !(6..=510).contains(&kbps) {
        return Err(err(format!("bitrate {kbps} kbps is not 6 to 510")));
    }
    let frames = resampled_len(left.len(), sample_rate) as u64;
    if loop_range
        .as_ref()
        .is_some_and(|range| range.is_empty() || range.end > frames)
    {
        return Err(err("loop range is outside the audio"));
    }
    let mut pcm = resample_48k(channels, sample_rate)?;
    let m = pcm.len() / 2;

    let mut encoder = Encoder::new(OPUS_RATE, Channels::Stereo, Application::Audio).map_err(err)?;
    encoder
        .set_bitrate(Bitrate::Bits(kbps as i32 * 1000))
        .map_err(err)?;
    encoder.set_vbr(true).map_err(err)?;
    let lookahead = encoder.get_lookahead().map_err(err)?;
    let pre_skip = u16::try_from(lookahead)
        .map_err(|_| err(format!("encoder lookahead {lookahead} out of range")))?;

    // Packets cover the pre-skip and the signal; the last one is padded with zeros.
    let total = m as u64 + u64::from(pre_skip);
    let packets = total.div_ceil(FRAME as u64).max(1) as usize;
    pcm.resize(packets * FRAME * 2, 0.0);

    let mut writer = PacketWriter::new(Vec::new());
    writer
        .write_packet(
            opus_head(pre_skip, sample_rate),
            STREAM_SERIAL,
            PacketWriteEndInfo::EndPage,
            0,
        )
        .map_err(err)?;
    writer
        .write_packet(
            opus_tags_with_loop(loop_range),
            STREAM_SERIAL,
            PacketWriteEndInfo::EndPage,
            0,
        )
        .map_err(err)?;
    let mut packet = vec![0u8; MAX_PACKET];
    for (k, frame) in pcm.chunks_exact(FRAME * 2).enumerate() {
        let len = encoder.encode_float(frame, &mut packet).map_err(err)?;
        let data = packet
            .get(..len)
            .ok_or_else(|| err(format!("packet of {len} bytes")))?
            .to_vec();
        let (info, granule) = if k + 1 == packets {
            // The end trim: the decoder stops at the pre-skip plus the signal.
            (PacketWriteEndInfo::EndStream, total)
        } else {
            (PacketWriteEndInfo::NormalPacket, (FRAME * (k + 1)) as u64)
        };
        writer
            .write_packet(data, STREAM_SERIAL, info, granule)
            .map_err(err)?;
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_opus_resampled_len_is_exact() {
        assert_eq!(resampled_len(0, 44_100), 0);
        assert_eq!(resampled_len(1, 44_100), 2);
        assert_eq!(resampled_len(147, 44_100), 160);
        assert_eq!(resampled_len(148, 44_100), 162);
        assert_eq!(resampled_len(44_100, 44_100), 48_000);
        assert_eq!(resampled_len(10, 48_000), 10);
    }

    #[test]
    fn encode_opus_headers() {
        let h = opus_head(312, 44_100);
        assert_eq!(
            h,
            [
                b'O', b'p', b'u', b's', b'H', b'e', b'a', b'd', 1, 2, 0x38, 0x01, 0x44, 0xac, 0, 0,
                0, 0, 0
            ]
        );
        let t = opus_tags();
        assert_eq!(&t[..8], b"OpusTags");
        assert_eq!(&t[8..12], &10u32.to_le_bytes());
        assert_eq!(&t[12..22], b"strudeland");
        assert_eq!(&t[22..], &[0, 0, 0, 0]);
    }
}

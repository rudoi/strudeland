//! The verification decoders: an encoded lossy file back to float samples, so its true peak can be
//! measured over the whole decoded stream (encoder delay and padding included, no alignment).
//! MP3 and Ogg Vorbis decode with symphonia, Opus with libopus over the Ogg packets.
//! [`decoded_true_peak_db`] streams the packets through a true-peak meter and keeps no audio;
//! [`decode`] keeps it (for tests and tools).

use std::io::Cursor;

use ::ogg::reading::PacketReader;
use ::opus::{Channels, Decoder};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::level::TruePeakMeter;
use crate::{Format, RenderError};

/// Decoded audio: planar channels at `sample_rate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
}

fn err(what: impl std::fmt::Display) -> RenderError {
    RenderError::Encode(format!("verify: {what}"))
}

/// Decodes an MP3, Ogg Vorbis or Ogg Opus file. WAV is not decoded here (it is lossless).
pub fn decode(format: Format, bytes: &[u8]) -> Result<Decoded, RenderError> {
    match format {
        Format::Mp3 | Format::Ogg => {
            let mut channels: Vec<Vec<f32>> = Vec::new();
            let mut sample_rate = 0;
            visit_symphonia(bytes, &mut |count, rate, interleaved| {
                if channels.is_empty() {
                    channels = vec![Vec::new(); count];
                }
                sample_rate = rate;
                append(&mut channels, interleaved);
            })?;
            Ok(Decoded {
                channels,
                sample_rate,
            })
        }
        Format::Opus => decode_opus(bytes).map(|(d, _, _)| d),
        Format::Wav => Err(err("wav is not decoded")),
    }
}

/// The true peak in dBTP of the whole decoded stream (`true_peak_db` of [`decode`]'s channels),
/// measured packet by packet without keeping the audio.
pub fn decoded_true_peak_db(format: Format, bytes: &[u8]) -> Result<f64, RenderError> {
    let mut meter: Option<TruePeakMeter> = None;
    let mut sink = |count: usize, _rate: u32, interleaved: &[f32]| {
        // A packet without channels adds nothing (and does not fix the channel count).
        if count == 0 {
            return;
        }
        meter
            .get_or_insert_with(|| TruePeakMeter::new(count))
            .push_interleaved(interleaved);
    };
    match format {
        Format::Mp3 | Format::Ogg => visit_symphonia(bytes, &mut sink)?,
        Format::Opus => {
            visit_opus(bytes, &mut sink)?;
        }
        Format::Wav => return Err(err("wav is not decoded")),
    }
    // No packet: no samples, as two empty channels measure.
    let linear = meter.map_or(0.0, TruePeakMeter::linear);
    Ok(20.0 * linear.log10())
}

/// Appends interleaved samples to planar channels.
fn append(channels: &mut [Vec<f32>], interleaved: &[f32]) {
    let n = channels.len().max(1);
    for (i, s) in interleaved.iter().enumerate() {
        if let Some(c) = channels.get_mut(i % n) {
            c.push(*s);
        }
    }
}

/// Decodes an MP3 or Ogg Vorbis file, calling `sink(channels, sample_rate, interleaved)` per
/// packet. Errors when the channel count changes or nothing decodes.
fn visit_symphonia(
    bytes: &[u8],
    sink: &mut dyn FnMut(usize, u32, &[f32]),
) -> Result<(), RenderError> {
    let mss = MediaSourceStream::new(
        Box::new(Cursor::new(bytes)),
        MediaSourceStreamOptions::default(),
    );
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(err)?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| err("no audio track"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| err("no audio codec parameters"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(err)?;

    // The channel count of the first packet that has channels.
    let mut channels = 0;
    let mut scratch: Vec<f32> = Vec::new();
    while let Some(packet) = format.next_packet().map_err(err)? {
        if packet.track_id != track_id {
            continue;
        }
        let buf = decoder.decode(&packet).map_err(err)?;
        let spec = buf.spec();
        let rate = spec.rate();
        let count = spec.channels().count();
        if channels == 0 {
            channels = count;
        } else if channels != count {
            return Err(err("the channel count changed mid-stream"));
        }
        buf.copy_to_vec_interleaved(&mut scratch);
        sink(count, rate, &scratch);
    }
    if channels == 0 {
        return Err(err("no audio decoded"));
    }
    Ok(())
}

/// The largest Opus packet is 120 ms: 5760 frames at 48 kHz.
const MAX_OPUS_FRAMES: usize = 5760;

/// Decodes an Ogg Opus file as a player does (RFC 7845 4.2 and 4.4): the pre-skip is dropped and
/// the output ends at the last page's granule position.
pub fn decode_opus_trimmed(bytes: &[u8]) -> Result<Decoded, RenderError> {
    let (mut decoded, pre_skip, end) = decode_opus(bytes)?;
    let keep = end
        .checked_sub(u64::from(pre_skip))
        .ok_or_else(|| err("the final granule is before the pre-skip"))?;
    for c in &mut decoded.channels {
        let skip = usize::from(pre_skip).min(c.len());
        c.drain(..skip);
        if (c.len() as u64) < keep {
            return Err(err("the stream is shorter than its final granule"));
        }
        c.truncate(keep as usize);
    }
    Ok(decoded)
}

/// Every packet decoded and appended (the pre-skip and the end padding kept), with the pre-skip
/// and the last page's granule position.
fn decode_opus(bytes: &[u8]) -> Result<(Decoded, u16, u64), RenderError> {
    let mut channels = vec![Vec::new(), Vec::new()];
    let (pre_skip, end) = visit_opus(bytes, &mut |_, _, decoded| append(&mut channels, decoded))?;
    Ok((
        Decoded {
            channels,
            sample_rate: 48_000,
        },
        pre_skip,
        end,
    ))
}

/// Decodes every packet of an Ogg Opus file, calling `sink(2, 48000, interleaved)` per packet;
/// returns the pre-skip and the last page's granule position.
fn visit_opus(
    bytes: &[u8],
    sink: &mut dyn FnMut(usize, u32, &[f32]),
) -> Result<(u16, u64), RenderError> {
    let mut reader = PacketReader::new(Cursor::new(bytes));
    let head = reader
        .read_packet()
        .map_err(err)?
        .ok_or_else(|| err("no OpusHead"))?;
    if !head.data.starts_with(b"OpusHead") || head.data.get(9) != Some(&2) {
        return Err(err("not a stereo Ogg Opus stream"));
    }
    let pre_skip = match (head.data.get(10), head.data.get(11)) {
        (Some(lo), Some(hi)) => u16::from_le_bytes([*lo, *hi]),
        _ => return Err(err("short OpusHead")),
    };
    let tags = reader
        .read_packet()
        .map_err(err)?
        .ok_or_else(|| err("no OpusTags"))?;
    if !tags.data.starts_with(b"OpusTags") {
        return Err(err("no OpusTags"));
    }
    let mut decoder = Decoder::new(48_000, Channels::Stereo).map_err(err)?;
    let mut out = vec![0f32; MAX_OPUS_FRAMES * 2];
    let mut end = 0;
    while let Some(packet) = reader.read_packet().map_err(err)? {
        end = packet.absgp_page();
        let frames = decoder
            .decode_float(&packet.data, &mut out, false)
            .map_err(err)?;
        let decoded = out
            .get(..frames * 2)
            .ok_or_else(|| err(format!("decoder reported {frames} frames")))?;
        sink(2, 48_000, decoded);
    }
    Ok((pre_skip, end))
}

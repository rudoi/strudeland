//! 32-bit float WAV writing and reading (pure: any `Write`, any byte slice). One or two channels
//! use the plain float format (tag 3, the 44-byte header `src/lib/render/wav.ts` writes); more
//! channels use `WAVE_FORMAT_EXTENSIBLE` with the IEEE-float sub-format.

use std::io::{self, Write};

const TAG_FLOAT: u16 = 3;
const TAG_EXTENSIBLE: u16 = 0xfffe;
/// The KSDATAFORMAT_SUBTYPE_IEEE_FLOAT GUID as stored in the file.
const IEEE_FLOAT_GUID: [u8; 16] = [
    0x03, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
];

/// Why a WAV cannot be written or read.
#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("no channels")]
    NoChannels,
    #[error("channel length mismatch")]
    LengthMismatch,
    #[error("data larger than 4 GiB")]
    TooLarge,
    #[error("too many channels: {0}")]
    TooManyChannels(usize),
    #[error("not a float WAV: {0}")]
    Format(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Bytes of the data chunk for `channels` x `frames` f32 samples, if it fits a RIFF file.
pub fn data_size(channels: usize, frames: usize) -> Option<u32> {
    sample_data_size(channels, frames, 4)
}

/// Bytes of the data chunk for `channels` x `frames` samples of `bytes` each, if it fits a RIFF
/// file. Computed from the lengths only.
pub fn sample_data_size(channels: usize, frames: usize, bytes: usize) -> Option<u32> {
    let size = (channels as u64)
        .checked_mul(frames as u64)?
        .checked_mul(bytes as u64)?;
    // The RIFF size field counts the header after the first 8 bytes (at most 72 bytes of it).
    if size.checked_add(72)? > u64::from(u32::MAX) {
        None
    } else {
        u32::try_from(size).ok()
    }
}

/// Writes interleaved little-endian f32 samples at `sample_rate`.
pub fn write_f32<W: Write>(
    w: &mut W,
    channels: &[Vec<f32>],
    sample_rate: u32,
) -> Result<(), WavError> {
    let first = channels.first().ok_or(WavError::NoChannels)?;
    let frames = first.len();
    if channels.iter().any(|c| c.len() != frames) {
        return Err(WavError::LengthMismatch);
    }
    let nch =
        u16::try_from(channels.len()).map_err(|_| WavError::TooManyChannels(channels.len()))?;
    let data = data_size(channels.len(), frames).ok_or(WavError::TooLarge)?;
    let block_align = nch * 4;
    let extensible = nch > 2;
    let fmt_size: u32 = if extensible { 40 } else { 16 };
    let mut h = Vec::with_capacity(68);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(4 + 8 + fmt_size + 8 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&fmt_size.to_le_bytes());
    h.extend_from_slice(
        &(if extensible {
            TAG_EXTENSIBLE
        } else {
            TAG_FLOAT
        })
        .to_le_bytes(),
    );
    h.extend_from_slice(&nch.to_le_bytes());
    h.extend_from_slice(&sample_rate.to_le_bytes());
    h.extend_from_slice(&(sample_rate.wrapping_mul(u32::from(block_align))).to_le_bytes());
    h.extend_from_slice(&block_align.to_le_bytes());
    h.extend_from_slice(&32u16.to_le_bytes());
    if extensible {
        h.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        h.extend_from_slice(&32u16.to_le_bytes()); // valid bits
        h.extend_from_slice(&0u32.to_le_bytes()); // channel mask: no speaker positions
        h.extend_from_slice(&IEEE_FLOAT_GUID);
    }
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    w.write_all(&h)?;
    let mut buf = Vec::with_capacity(4096 * channels.len() * 4);
    for i in 0..frames {
        for c in channels {
            buf.extend_from_slice(&c[i].to_le_bytes());
        }
        if buf.len() >= 1 << 16 {
            w.write_all(&buf)?;
            buf.clear();
        }
    }
    w.write_all(&buf)?;
    Ok(())
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A decoded float WAV.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedWav {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    /// The format tag in the `fmt ` chunk (3 or 0xFFFE).
    pub tag: u16,
}

/// Reads a 32-bit float WAV (plain or extensible).
pub fn read_f32(bytes: &[u8]) -> Result<DecodedWav, WavError> {
    let bad = |m: &str| WavError::Format(m.to_string());
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(bad("not RIFF/WAVE"));
    }
    let mut off = 12;
    let mut fmt: Option<(u16, u16, u32)> = None;
    while let (Some(id), Some(size)) = (bytes.get(off..off + 4), u32_at(bytes, off + 4)) {
        let body = off + 8;
        let size = size as usize;
        if id == b"fmt " {
            let tag = u16_at(bytes, body).ok_or_else(|| bad("short fmt"))?;
            let nch = u16_at(bytes, body + 2).ok_or_else(|| bad("short fmt"))?;
            let rate = u32_at(bytes, body + 4).ok_or_else(|| bad("short fmt"))?;
            let bits = u16_at(bytes, body + 14).ok_or_else(|| bad("short fmt"))?;
            let float = match tag {
                TAG_FLOAT => true,
                TAG_EXTENSIBLE => bytes.get(body + 24..body + 40) == Some(&IEEE_FLOAT_GUID[..]),
                _ => false,
            };
            if !float || bits != 32 || nch == 0 {
                return Err(bad(&format!("tag {tag}, {bits} bits, {nch} channels")));
            }
            fmt = Some((tag, nch, rate));
        } else if id == b"data" {
            let (tag, nch, sample_rate) = fmt.ok_or_else(|| bad("data before fmt"))?;
            let data = bytes
                .get(body..body + size)
                .ok_or_else(|| bad("short data"))?;
            let nch = usize::from(nch);
            let frames = data.len() / (nch * 4);
            let mut channels = vec![Vec::with_capacity(frames); nch];
            for (i, s) in data.chunks_exact(4).take(frames * nch).enumerate() {
                channels[i % nch].push(f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
            }
            return Ok(DecodedWav {
                channels,
                sample_rate,
                tag,
            });
        }
        off = body + size + (size % 2);
    }
    Err(bad("missing data chunk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(n: usize, frames: usize) -> Vec<Vec<f32>> {
        (0..n)
            .map(|c| {
                (0..frames)
                    .map(|i| (c * 1000 + i) as f32 * 0.5 - 3.0)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn stereo_round_trip_with_the_plain_header() {
        let ch = channels(2, 10);
        let mut out = Vec::new();
        write_f32(&mut out, &ch, 44100).unwrap();
        assert_eq!(out.len(), 44 + 2 * 10 * 4);
        assert_eq!(u16_at(&out, 20), Some(3));
        assert_eq!(u32_at(&out, 4), Some(36 + 80));
        let d = read_f32(&out).unwrap();
        assert_eq!(d.channels, ch);
        assert_eq!((d.sample_rate, d.tag), (44100, 3));
    }

    #[test]
    fn six_channels_use_extensible() {
        let ch = channels(6, 7);
        let mut out = Vec::new();
        write_f32(&mut out, &ch, 44100).unwrap();
        assert_eq!(out.len(), 68 + 6 * 7 * 4);
        assert_eq!(u16_at(&out, 20), Some(0xfffe));
        assert_eq!(u32_at(&out, 16), Some(40));
        let d = read_f32(&out).unwrap();
        assert_eq!(d.channels, ch);
        assert_eq!(d.tag, 0xfffe);
    }

    #[test]
    fn special_values_are_preserved_and_errors_reported() {
        let ch = vec![vec![f32::NAN, f32::INFINITY, -0.0, 2.5]];
        let mut out = Vec::new();
        write_f32(&mut out, &ch, 44100).unwrap();
        let d = read_f32(&out).unwrap();
        assert!(d.channels[0][0].is_nan());
        assert_eq!(d.channels[0][1], f32::INFINITY);
        assert!(d.channels[0][2].is_sign_negative());
        assert!(matches!(
            write_f32(&mut Vec::new(), &[], 44100),
            Err(WavError::NoChannels)
        ));
        assert!(matches!(
            write_f32(&mut Vec::new(), &[vec![0.0], vec![]], 44100),
            Err(WavError::LengthMismatch)
        ));
        assert!(read_f32(b"nope").is_err());
        assert!(read_f32(&out[..30]).is_err());
        assert_eq!(data_size(2, 10), Some(80));
        assert_eq!(data_size(32, 1 << 31), None);
        assert_eq!(sample_data_size(2, 10, 3), Some(60));
        assert_eq!(sample_data_size(2, 0, 2), Some(0));
        // The largest data chunk: u32::MAX - 72 bytes.
        let max = u64::from(u32::MAX) - 72;
        assert_eq!(sample_data_size(1, max as usize, 1), Some(max as u32));
        assert_eq!(sample_data_size(1, max as usize + 1, 1), None);
        assert_eq!(sample_data_size(2, usize::MAX, 3), None);
        let empty = vec![Vec::new(); 2];
        let mut out = Vec::new();
        write_f32(&mut out, &empty, 44100).unwrap();
        assert_eq!(read_f32(&out).unwrap().channels, empty);
    }
}

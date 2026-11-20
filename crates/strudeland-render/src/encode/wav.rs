//! The WAV writer: 16 and 24-bit PCM with deterministic TPDF dither, and 32-bit float through the
//! engine's writer.

use std::ops::Range;

use strudeland_engine::Mulberry32;
use strudeland_engine::wav::sample_data_size;

use crate::{BitDepth, RenderError};

/// The dither generators' base seed (`"STRD"`); channel `c` uses `DITHER_SEED + c`.
pub const DITHER_SEED: u32 = 0x5354_5244;

/// `WAVE_FORMAT_PCM`.
const TAG_PCM: u16 = 1;
/// Nine sampler fields and six fields for the single loop, all 32-bit.
const SMPL_DATA_SIZE: u32 = 60;

/// Encodes `channels` (equal lengths) at `sample_rate` as a WAV of `bit_depth`. 16 and 24-bit
/// take one or two channels and are TPDF-dithered with fixed seeds, so the bytes depend only on
/// the input; `32f` writes the samples verbatim.
pub fn encode_wav(
    channels: &[Vec<f32>],
    sample_rate: u32,
    bit_depth: BitDepth,
) -> Result<Vec<u8>, RenderError> {
    let bytes = match bit_depth {
        BitDepth::F32 => {
            let mut out = Vec::new();
            strudeland_engine::wav::write_f32(&mut out, channels, sample_rate)
                .map_err(|e| RenderError::Encode(format!("wav: {e}")))?;
            return Ok(out);
        }
        BitDepth::B16 => 2usize,
        BitDepth::B24 => 3usize,
    };
    let first = channels
        .first()
        .ok_or_else(|| RenderError::Encode("wav: no channels".into()))?;
    if channels.len() > 2 {
        return Err(RenderError::Encode(format!(
            "wav: {} channels (PCM takes one or two)",
            channels.len()
        )));
    }
    let frames = first.len();
    if channels.iter().any(|c| c.len() != frames) {
        return Err(RenderError::Encode("wav: channel length mismatch".into()));
    }
    let data = sample_data_size(channels.len(), frames, bytes)
        .ok_or_else(|| RenderError::Encode("wav: data larger than 4 GiB".into()))?;
    let nch = channels.len() as u16; // 1 or 2
    let block_align = nch * bytes as u16;
    let bits = 8 * bytes as u16;

    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&TAG_PCM.to_le_bytes());
    out.extend_from_slice(&nch.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(
        &sample_rate
            .wrapping_mul(u32::from(block_align))
            .to_le_bytes(),
    );
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());

    let scale: f64 = if bytes == 2 { 32768.0 } else { 8_388_608.0 };
    let mut rngs: Vec<Mulberry32> = (0..channels.len())
        .map(|c| Mulberry32::new(DITHER_SEED.wrapping_add(c as u32)))
        .collect();
    for i in 0..frames {
        for (channel, rng) in channels.iter().zip(rngs.iter_mut()) {
            let x = channel.get(i).copied().unwrap_or(0.0);
            let q = quantize(x, scale, rng);
            out.extend(q.to_le_bytes().into_iter().take(bytes));
        }
    }
    Ok(out)
}

/// Encodes WAV with an optional forward loop in decoded frames (exclusive end).
pub fn encode_wav_with_loop(
    channels: &[Vec<f32>],
    sample_rate: u32,
    bit_depth: BitDepth,
    loop_range: Option<Range<u64>>,
) -> Result<Vec<u8>, RenderError> {
    let Some(range) = loop_range else {
        return encode_wav(channels, sample_rate, bit_depth);
    };
    let frames = channels.first().map_or(0, |c| c.len() as u64);
    if range.is_empty() || range.end > frames || range.end - 1 > u64::from(u32::MAX) {
        return Err(RenderError::Encode(
            "wav: loop range is outside the audio".into(),
        ));
    }
    let mut out = encode_wav(channels, sample_rate, bit_depth)?;
    // The legacy writer ends after the sample payload. Pad an odd-sized data chunk before smpl.
    let pad = out.len() % 2;
    let extra = pad + 8 + SMPL_DATA_SIZE as usize;
    let riff_size = u32::try_from((out.len() - 8) as u64 + extra as u64).map_err(|_| {
        RenderError::Encode("wav: RIFF with loop metadata is larger than 4 GiB".into())
    })?;
    out.reserve_exact(extra);
    out.get_mut(4..8)
        .ok_or_else(|| RenderError::Encode("wav: missing RIFF size".into()))?
        .copy_from_slice(&riff_size.to_le_bytes());
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(b"smpl");
    out.extend_from_slice(&SMPL_DATA_SIZE.to_le_bytes());
    let period_ns = 1_000_000_000u32.checked_div(sample_rate).unwrap_or(0);
    for field in [
        0, // manufacturer
        0, // product
        period_ns,
        60, // MIDI unity note (middle C)
        0,  // MIDI pitch fraction
        0,  // SMPTE format
        0,  // SMPTE offset
        1,  // loop count
        0,  // sampler data size
        0,  // cue point ID
        0,  // forward loop
        range.start as u32,
        (range.end - 1) as u32, // inclusive end
        0,                      // loop point fraction
        0,                      // repeat indefinitely
    ] {
        out.extend_from_slice(&field.to_le_bytes());
    }
    Ok(out)
}

/// One TPDF-dithered sample: `round(x * scale + u1 - u2)` (ties away from zero), clamped to
/// `[-scale, scale - 1]`. A NaN becomes 0.
pub fn quantize(x: f32, scale: f64, rng: &mut Mulberry32) -> i32 {
    let u1 = rng.next();
    let u2 = rng.next();
    let v = f64::from(x) * scale + (u1 - u2);
    v.round().clamp(-scale, scale - 1.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_quantize_clamps_and_maps_nan_to_zero() {
        let mut r = Mulberry32::new(DITHER_SEED);
        for _ in 0..1000 {
            assert_eq!(quantize(1.0, 32768.0, &mut r), 32767);
            // -1.0 is a code; the dither moves it up by one a quarter of the time.
            assert!((-32768..=-32767).contains(&quantize(-1.0, 32768.0, &mut r)));
            assert_eq!(quantize(-1.01, 32768.0, &mut r), -32768);
            assert_eq!(quantize(4.0, 8_388_608.0, &mut r), 8_388_607);
            assert_eq!(quantize(-4.0, 8_388_608.0, &mut r), -8_388_608);
            assert_eq!(quantize(f32::INFINITY, 32768.0, &mut r), 32767);
            assert_eq!(quantize(f32::NEG_INFINITY, 32768.0, &mut r), -32768);
            assert_eq!(quantize(f32::NAN, 32768.0, &mut r), 0);
        }
    }
}

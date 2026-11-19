//! The WAV writer: 16 and 24-bit PCM with deterministic TPDF dither, and 32-bit float through the
//! engine's writer.

use strudeland_engine::Mulberry32;
use strudeland_engine::wav::sample_data_size;

use crate::{BitDepth, RenderError};

/// The dither generators' base seed (`"STRD"`); channel `c` uses `DITHER_SEED + c`.
pub const DITHER_SEED: u32 = 0x5354_5244;

/// `WAVE_FORMAT_PCM`.
const TAG_PCM: u16 = 1;

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

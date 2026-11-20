//! The encoders: WAV, MP3, Ogg Vorbis and Opus, on synthetic buffers.

use std::io::Cursor;

use ogg::reading::PacketReader;
use strudeland_render::encode::mp3::encode_mp3_with_loop;
use strudeland_render::encode::ogg::encode_ogg_with_loop;
use strudeland_render::encode::opus::{encode_opus_with_loop, resample_48k, resampled_len};
use strudeland_render::encode::verify::{decode, decode_opus_trimmed, decoded_true_peak_db};
use strudeland_render::encode::wav::{encode_wav, encode_wav_with_loop};
use strudeland_render::encode::{CORRECTION_MARGIN_DB, encode_once};
use strudeland_render::level::true_peak_db;
use strudeland_render::{BitDepth, EncoderOptions, Format, Mp3Mode, RenderError};

const SR: u32 = 44_100;

/// A stereo sine: left at `hz`, right at `1.5 * hz`, amplitude `amp`.
fn sine(frames: usize, hz: f64, amp: f64) -> Vec<Vec<f32>> {
    let ch = |f: f64| {
        (0..frames)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * f * i as f64 / f64::from(SR)).sin()) as f32
            })
            .collect::<Vec<f32>>()
    };
    vec![ch(hz), ch(hz * 1.5)]
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A test-local PCM reader: interleaved integer samples of a 16 or 24-bit WAV.
fn read_pcm(b: &[u8]) -> (u16, u16, Vec<i32>) {
    let channels = u16_at(b, 22);
    let bits = u16_at(b, 34);
    let data = u32_at(b, 40) as usize;
    let bytes = usize::from(bits / 8);
    let samples = b[44..44 + data]
        .chunks_exact(bytes)
        .map(|s| match bytes {
            2 => i32::from(i16::from_le_bytes([s[0], s[1]])),
            3 => i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8,
            _ => unreachable!(),
        })
        .collect();
    (channels, bits, samples)
}

fn wav(channels: &[Vec<f32>], depth: BitDepth) -> Vec<u8> {
    encode_wav(channels, SR, depth).unwrap()
}

#[test]
fn encode_wav_pcm_headers() {
    let input = sine(1000, 440.0, 0.5);
    for (depth, bytes) in [(BitDepth::B16, 2u16), (BitDepth::B24, 3)] {
        let b = wav(&input, depth);
        let data = 1000 * 2 * u32::from(bytes);
        assert_eq!(b.len(), 44 + data as usize, "{depth:?}");
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32_at(&b, 4), 36 + data);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u32_at(&b, 16), 16, "fmt size");
        assert_eq!(u16_at(&b, 20), 1, "tag");
        assert_eq!(u16_at(&b, 22), 2, "channels");
        assert_eq!(u32_at(&b, 24), SR, "rate");
        assert_eq!(u32_at(&b, 28), SR * 2 * u32::from(bytes), "byte rate");
        assert_eq!(u16_at(&b, 32), 2 * bytes, "block align");
        assert_eq!(u16_at(&b, 34), 8 * bytes, "bits");
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32_at(&b, 40), data, "data size");
    }
    // Mono.
    let b = wav(&input[..1], BitDepth::B16);
    assert_eq!(u16_at(&b, 22), 1);
    assert_eq!(u16_at(&b, 32), 2);
    assert_eq!(u32_at(&b, 40), 2000);
}

#[test]
fn encode_wav_32f_is_the_engine_writer() {
    let mut input = sine(777, 1000.0, 0.9);
    input[0][3] = 1.75; // over unity, verbatim
    let b = wav(&input, BitDepth::F32);
    let mut expected = Vec::new();
    strudeland_engine::wav::write_f32(&mut expected, &input, SR).unwrap();
    assert_eq!(b, expected);
    assert_eq!(u16_at(&b, 20), 3, "float tag");
    assert_eq!(u16_at(&b, 34), 32, "bits");
    assert_eq!(u32_at(&b, 40), 777 * 2 * 4, "data size");
}

#[test]
fn encode_wav_smpl_keeps_pcm_and_uses_an_inclusive_end() {
    let input = sine(17, 440.0, 0.5);
    for depth in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
        // Mono 24-bit has an odd data size and needs a pad byte before the next chunk.
        let input = &input[..1];
        let plain = wav(input, depth);
        let tagged = encode_wav_with_loop(input, SR, depth, Some(3..17)).unwrap();
        assert_eq!(encode_wav_with_loop(input, SR, depth, None).unwrap(), plain);
        assert_eq!(u32_at(&tagged, 4) as usize + 8, tagged.len());
        assert_eq!(&tagged[8..plain.len()], &plain[8..]);
        let at = plain.len().next_multiple_of(2);
        assert!(tagged[plain.len()..at].iter().all(|&b| b == 0));
        assert_eq!(&tagged[at..at + 4], b"smpl");
        assert_eq!(u32_at(&tagged, at + 4), 60);
        assert_eq!(tagged.len(), at + 8 + 60);
        let smpl = &tagged[at + 8..];
        assert_eq!(u32_at(smpl, 28), 1, "one loop");
        assert_eq!(u32_at(smpl, 40), 0, "forward");
        assert_eq!(u32_at(smpl, 44), 3);
        assert_eq!(u32_at(smpl, 48), 16, "inclusive end");
        assert_eq!(u32_at(smpl, 56), 0, "repeat indefinitely");
    }
}

#[test]
fn encode_wav_24_bit_decodes_within_two_lsb() {
    let input = sine(20_000, 997.0, 0.8);
    let (channels, bits, samples) = read_pcm(&wav(&input, BitDepth::B24));
    assert_eq!((channels, bits), (2, 24));
    assert_eq!(samples.len(), 40_000);
    for (i, s) in samples.iter().enumerate() {
        let x = f64::from(input[i % 2][i / 2]) * 8_388_608.0;
        assert!((f64::from(*s) - x).abs() <= 2.0, "sample {i}: {s} vs {x}");
    }
}

#[test]
fn encode_wav_16_bit_dither_statistics() {
    // Silence: the dither alone, round(u1 - u2), is -1, 0 or 1; 0 with probability 3/4.
    let n = 200_000;
    let silence = vec![vec![0.0f32; n]; 2];
    let (_, _, samples) = read_pcm(&wav(&silence, BitDepth::B16));
    assert!(samples.iter().all(|s| (-1..=1).contains(s)));
    for c in 0..2 {
        let ch: Vec<i32> = samples.iter().skip(c).step_by(2).copied().collect();
        let zeros = ch.iter().filter(|s| **s == 0).count() as f64 / n as f64;
        let plus = ch.iter().filter(|s| **s == 1).count() as f64 / n as f64;
        let minus = ch.iter().filter(|s| **s == -1).count() as f64 / n as f64;
        assert!((zeros - 0.75).abs() < 0.005, "channel {c} zeros {zeros}");
        assert!((plus - 0.125).abs() < 0.005, "channel {c} +1 {plus}");
        assert!((minus - 0.125).abs() < 0.005, "channel {c} -1 {minus}");
    }
    // The channels use different generators.
    let left: Vec<i32> = samples.iter().step_by(2).copied().collect();
    let right: Vec<i32> = samples.iter().skip(1).step_by(2).copied().collect();
    assert_ne!(left, right);

    // A DC level between two codes: the error is unbiased and its variance is 1/4 LSB^2
    // (1/12 rounding plus 1/6 triangular dither).
    let dc = 0.3 / 32768.0 + 100.0 / 32768.0;
    let input = vec![vec![dc as f32; n]; 2];
    let (_, _, samples) = read_pcm(&wav(&input, BitDepth::B16));
    let x = f64::from(dc as f32) * 32768.0;
    let errs: Vec<f64> = samples.iter().map(|s| f64::from(*s) - x).collect();
    let mean = errs.iter().sum::<f64>() / errs.len() as f64;
    let var = errs.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / errs.len() as f64;
    assert!(mean.abs() < 0.01, "mean {mean}");
    assert!(var > 0.15 && var < 0.35, "variance {var}");
    assert!(errs.iter().all(|e| e.abs() < 1.5));
}

#[test]
fn encode_wav_is_byte_reproducible() {
    let input = sine(30_000, 220.0, 0.7);
    for depth in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
        assert_eq!(wav(&input, depth), wav(&input, depth), "{depth:?}");
    }
}

#[test]
fn encode_wav_clamps_at_full_scale() {
    // Over-unity input clamps to the extreme codes; +1.0 is one past the top code so it always
    // clamps, -1.0 is the bottom code and the dither can lift it by one.
    let input = vec![vec![1.0f32, 2.0, -2.0, -1.01, 1.01, -1.0]; 2];
    let (_, _, s16) = read_pcm(&wav(&input, BitDepth::B16));
    let left: Vec<i32> = s16.iter().step_by(2).copied().collect();
    assert_eq!(&left[..5], &[32767, 32767, -32768, -32768, 32767]);
    assert!((-32768..=-32767).contains(&left[5]), "{}", left[5]);
    let (_, _, s24) = read_pcm(&wav(&input, BitDepth::B24));
    let left: Vec<i32> = s24.iter().step_by(2).copied().collect();
    assert_eq!(
        &left[..5],
        &[8_388_607, 8_388_607, -8_388_608, -8_388_608, 8_388_607]
    );
    assert!((-8_388_608..=-8_388_607).contains(&left[5]), "{}", left[5]);
}

#[test]
fn encode_wav_empty_input_and_bad_shapes() {
    for depth in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
        let b = wav(&[vec![], vec![]], depth);
        assert_eq!(b.len(), 44, "{depth:?}");
        assert_eq!(u32_at(&b, 40), 0);
        let one = wav(&[vec![0.25], vec![-0.25]], depth);
        assert_eq!(u32_at(&one, 40) as usize, one.len() - 44);
    }
    for depth in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
        for bad in [vec![], vec![vec![0.0f32; 3], vec![0.0; 4]]] {
            assert!(
                matches!(encode_wav(&bad, SR, depth), Err(RenderError::Encode(_))),
                "{depth:?} {bad:?}"
            );
        }
    }
    let three = vec![vec![0.0f32; 4]; 3];
    for depth in [BitDepth::B16, BitDepth::B24] {
        let e = encode_wav(&three, SR, depth).unwrap_err();
        assert_eq!(
            e.to_string(),
            "encode failed: wav: 3 channels (PCM takes one or two)"
        );
    }
}

#[test]
fn encode_once_dispatches_wav() {
    let input = sine(500, 300.0, 0.5);
    for depth in [BitDepth::B16, BitDepth::B24, BitDepth::F32] {
        assert_eq!(
            encode_once(&input, SR, &EncoderOptions::Wav { bit_depth: depth }).unwrap(),
            wav(&input, depth)
        );
    }
}

fn mp3(input: &[Vec<f32>], mode: Mp3Mode, q: u8) -> Vec<u8> {
    encode_once(
        input,
        SR,
        &EncoderOptions::Mp3 {
            mode,
            vbr_quality: q,
        },
    )
    .unwrap()
}

fn ogg(input: &[Vec<f32>], quality: i32) -> Vec<u8> {
    encode_once(input, SR, &EncoderOptions::Ogg { quality }).unwrap()
}

#[test]
fn encode_mp3_lame_tag_overwrites_the_first_frame() {
    let input = sine(SR as usize * 3, 440.0, 0.5);
    for (mode, q, tag) in [
        (Mp3Mode::Cbr, 0, b"Info"),
        (Mp3Mode::Vbr, 0, b"Xing"),
        (Mp3Mode::Vbr, 9, b"Xing"),
    ] {
        let b = mp3(&input, mode, q);
        // An MPEG-1 Layer III frame sync, then the tag after the 32-byte stereo side info.
        assert_eq!(b[0], 0xff, "{mode:?} {q}");
        assert_eq!(b[1] & 0xfe, 0xfa, "{mode:?} {q}");
        assert_eq!((b[2] >> 2) & 3, 0, "{mode:?} {q}: 44100 Hz, not resampled");
        assert_eq!(&b[36..40], tag, "{mode:?} {q}");
        assert_eq!(&b[0x9c..0xa0], b"LAME", "{mode:?} {q}: the LAME extension");
    }
}

#[test]
fn encode_mp3_id3_loop_tags_keep_the_lame_frame_and_decoded_audio() {
    let input = sine(100_003, 440.0, 0.5);
    for mode in [Mp3Mode::Cbr, Mp3Mode::Vbr] {
        let tagged = encode_mp3_with_loop(&input, SR, mode, 6, Some(147..100_003)).unwrap();
        let plain = mp3(&input, mode, 6);
        let read_size = |bytes: &[u8]| {
            bytes.iter().fold(0usize, |size, &b| {
                assert!(b < 128);
                size * 128 + usize::from(b)
            })
        };
        assert_eq!(&tagged[..6], b"ID3\x04\x00\x00");
        let end = 10 + read_size(&tagged[6..10]);
        let mut at = 10;
        for content in [
            b"\x00LOOPSTART\x00147".as_slice(),
            b"\x00LOOPLENGTH\x0099856",
        ] {
            assert_eq!(&tagged[at..at + 4], b"TXXX");
            let size = read_size(&tagged[at + 4..at + 8]);
            assert_eq!(&tagged[at + 8..at + 10], &[0, 0]);
            at += 10;
            assert_eq!(&tagged[at..at + size], content);
            at += size;
        }
        assert_eq!(at, end);
        assert_eq!(&tagged[end..], plain);
        assert_eq!(
            decode(Format::Mp3, &tagged).unwrap(),
            decode(Format::Mp3, &plain).unwrap()
        );
    }
}

/// Reads a Vorbis-style comment packet with a codec-specific magic prefix.
fn ogg_comments(bytes: &[u8], prefix: &[u8]) -> Vec<String> {
    let mut reader = PacketReader::new(Cursor::new(bytes));
    reader.read_packet().unwrap().unwrap(); // identification header
    let comments = reader.read_packet().unwrap().unwrap().data;
    assert!(comments.starts_with(prefix));
    let vendor_at = prefix.len() + 4;
    let vendor_len = u32_at(&comments, prefix.len()) as usize;
    let mut at = vendor_at + vendor_len;
    let count = u32_at(&comments, at);
    at += 4;
    (0..count)
        .map(|_| {
            let size = u32_at(&comments, at) as usize;
            at += 4;
            let value = std::str::from_utf8(&comments[at..at + size]).unwrap();
            at += size;
            value.to_owned()
        })
        .collect()
}

#[test]
fn encode_ogg_loop_comments_use_the_pcm_sample_rate() {
    let input = sine(10_003, 440.0, 0.5);
    let tagged = encode_ogg_with_loop(&input, SR, 6, Some(148..10_003)).unwrap();
    assert_eq!(
        ogg_comments(&tagged, b"\x03vorbis"),
        ["ENCODER=strudeland", "LOOPSTART=148", "LOOPLENGTH=9855"]
    );
    assert_eq!(
        decode(Format::Ogg, &tagged).unwrap(),
        decode(Format::Ogg, &ogg(&input, 6)).unwrap()
    );
}

#[test]
fn encode_opus_loop_comments_use_decoded_frames_after_pre_skip() {
    let input = sine(10_003, 440.0, 0.5);
    // ceil(148 * 160 / 147) is 162 and ceil(10003 * 160 / 147) is 10888.
    // Resampling the endpoints independently yields length 10726.
    let tagged = encode_opus_with_loop(&input, SR, 160, Some(162..10_888)).unwrap();
    assert_eq!(
        ogg_comments(&tagged, b"OpusTags"),
        ["LOOPSTART=162", "LOOPLENGTH=10726"]
    );
    let decoded = decode_opus_trimmed(&tagged).unwrap();
    assert_eq!(decoded.sample_rate, 48_000);
    assert_eq!(decoded.channels[0].len(), 10_888);
    assert_eq!(decoded, decode_opus_trimmed(&opus(&input, 160)).unwrap());
}

#[test]
fn encode_mp3_cbr_320_size() {
    let n = SR as usize * 10;
    let b = mp3(&sine(n, 440.0, 0.5), Mp3Mode::Cbr, 0);
    let expected = n as f64 / f64::from(SR) * 40_000.0;
    let ratio = b.len() as f64 / expected;
    assert!(
        (ratio - 1.0).abs() < 0.02,
        "{} bytes, expected {expected}",
        b.len()
    );
}

#[test]
fn encode_mp3_is_byte_reproducible_and_vbr_quality_matters() {
    let input = sine(SR as usize * 2, 330.0, 0.6);
    for (mode, q) in [(Mp3Mode::Cbr, 0), (Mp3Mode::Vbr, 0), (Mp3Mode::Vbr, 9)] {
        assert_eq!(mp3(&input, mode, q), mp3(&input, mode, q), "{mode:?} {q}");
    }
    assert!(mp3(&input, Mp3Mode::Vbr, 9).len() < mp3(&input, Mp3Mode::Vbr, 0).len());
}

#[test]
fn encode_mp3_empty_and_one_frame() {
    for n in [0, 1] {
        let input = vec![vec![0.25f32; n]; 2];
        for mode in [Mp3Mode::Cbr, Mp3Mode::Vbr] {
            let b = mp3(&input, mode, 0);
            assert!(b.len() >= 40, "{n} {mode:?}: {} bytes", b.len());
            assert_eq!(b[0], 0xff, "{n} {mode:?}");
        }
    }
}

#[test]
fn encode_mp3_and_ogg_reject_bad_shapes() {
    let bad: [Vec<Vec<f32>>; 3] = [vec![], vec![vec![0.0; 4]], vec![vec![0.0; 4], vec![0.0; 5]]];
    for input in &bad {
        for opts in [
            EncoderOptions::Mp3 {
                mode: Mp3Mode::Cbr,
                vbr_quality: 0,
            },
            EncoderOptions::Ogg { quality: 6 },
        ] {
            assert!(
                matches!(encode_once(input, SR, &opts), Err(RenderError::Encode(_))),
                "{opts:?} {input:?}"
            );
        }
    }
    let input = sine(100, 440.0, 0.5);
    for q in [-2, 11] {
        assert!(matches!(
            encode_once(&input, SR, &EncoderOptions::Ogg { quality: q }),
            Err(RenderError::Encode(_))
        ));
    }
    assert!(matches!(
        encode_once(
            &input,
            SR,
            &EncoderOptions::Mp3 {
                mode: Mp3Mode::Vbr,
                vbr_quality: 10
            }
        ),
        Err(RenderError::Encode(_))
    ));
    assert!(matches!(
        encode_once(&input, 0, &EncoderOptions::Ogg { quality: 6 }),
        Err(RenderError::Encode(_))
    ));
}

#[test]
fn encode_ogg_headers_reproducibility_and_quality() {
    let input = sine(SR as usize * 3, 440.0, 0.5);
    let b = ogg(&input, 6);
    assert_eq!(&b[0..4], b"OggS");
    // The first page's serial is the fixed one.
    assert_eq!(u32_at(&b, 14), 0x5354_5244);
    assert_eq!(&b[29..35], b"vorbis");
    assert!(
        b.windows(b"ENCODER=strudeland".len())
            .any(|w| w == b"ENCODER=strudeland"),
        "the comment tag"
    );
    assert_eq!(b, ogg(&input, 6));
    // Noise makes the bitrate depend on the quality.
    let mut seed = 1u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (f64::from(seed) / 4_294_967_296.0 - 0.5) as f32
    };
    let noise: Vec<Vec<f32>> = (0..2)
        .map(|_| (0..SR as usize * 2).map(|_| noise()).collect())
        .collect();
    assert!(ogg(&noise, -1).len() < ogg(&noise, 10).len());
}

#[test]
fn encode_ogg_empty_and_one_frame() {
    for n in [0, 1] {
        let b = ogg(&vec![vec![0.25f32; n]; 2], 6);
        assert_eq!(&b[0..4], b"OggS", "{n}");
        assert_eq!(b, ogg(&vec![vec![0.25f32; n]; 2], 6), "{n}");
    }
}

fn opus(input: &[Vec<f32>], kbps: u32) -> Vec<u8> {
    encode_once(input, SR, &EncoderOptions::Opus { kbps }).unwrap()
}

/// Every packet of an Ogg stream with its page's granule position and end-of-stream flag.
fn ogg_packets(b: &[u8]) -> Vec<(Vec<u8>, u64, bool, u32)> {
    let mut reader = ogg::reading::PacketReader::new(std::io::Cursor::new(b.to_vec()));
    let mut out = Vec::new();
    while let Some(p) = reader.read_packet().unwrap() {
        let (absgp, last, serial) = (p.absgp_page(), p.last_in_stream(), p.stream_serial());
        out.push((p.data, absgp, last, serial));
    }
    out
}

fn lookahead() -> u16 {
    let mut e =
        opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio).unwrap();
    e.set_bitrate(opus::Bitrate::Bits(160_000)).unwrap();
    e.get_lookahead().unwrap() as u16
}

#[test]
fn encode_opus_resample_length() {
    // Exact for every length, including multiples of 147 and a prime.
    for n in [1usize, 146, 147, 294, 1000, 44_100, 100_003] {
        let input = sine(n, 440.0, 0.5);
        let out = resample_48k(&input, SR).unwrap();
        let expected = (n * 160).div_ceil(147);
        assert_eq!(out.len(), 2 * expected, "{n}");
        assert_eq!(resampled_len(n, SR), expected, "{n}");
    }
    assert!(resample_48k(&[vec![], vec![]], SR).unwrap().is_empty());
    // The level survives the resample (a 1 kHz sine at 0.5).
    let out = resample_48k(&sine(44_100, 1000.0, 0.5), SR).unwrap();
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let peak = left[4800..43_200].iter().fold(0f32, |a, s| a.max(s.abs()));
    assert!((peak - 0.5).abs() < 0.005, "peak {peak}");
}

#[test]
fn encode_opus_ogg_structure_and_granules() {
    let pre_skip = lookahead();
    for n in [0usize, 1, 959, 44_100, 100_003] {
        let b = opus(&sine(n, 440.0, 0.5), 160);
        let packets = ogg_packets(&b);
        let m = resampled_len(n, SR) as u64;
        let total = m + u64::from(pre_skip);
        let f = total.div_ceil(960).max(1) as usize;
        assert_eq!(packets.len(), 2 + f, "{n}: header packets plus audio");
        let (head, head_gp, _, serial) = &packets[0];
        assert_eq!(*serial, 0x5354_5244, "{n}");
        assert_eq!(*head_gp, 0, "{n}");
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!(head.len(), 19);
        assert_eq!(head[8], 1, "version");
        assert_eq!(head[9], 2, "channels");
        assert_eq!(u16_at(head, 10), pre_skip, "{n}: pre-skip is the lookahead");
        assert_eq!(u32_at(head, 12), SR, "input rate");
        assert_eq!(u16_at(head, 16), 0, "gain");
        assert_eq!(head[18], 0, "mapping family");
        assert_eq!(&packets[1].0[..8], b"OpusTags");
        assert_eq!(packets[1].1, 0);
        let (_, last_gp, last, _) = packets.last().unwrap();
        assert!(*last, "{n}: end of stream");
        assert_eq!(
            *last_gp, total,
            "{n}: final granule is pre-skip plus the 48 kHz length"
        );
        assert!(packets.iter().take(packets.len() - 1).all(|p| !p.2), "{n}");
        // Granules never decrease.
        let gps: Vec<u64> = packets.iter().map(|p| p.1).collect();
        assert!(gps.windows(2).all(|w| w[0] <= w[1]), "{n}: {gps:?}");
    }
    // The pages' headers: capture pattern, the first page flags begin-of-stream.
    let b = opus(&sine(1000, 440.0, 0.5), 160);
    assert_eq!(&b[..4], b"OggS");
    assert_eq!(b[5], 0x02);
    assert_eq!(u32_at(&b, 14), 0x5354_5244);
}

#[test]
fn encode_opus_is_byte_reproducible_and_bitrate_matters() {
    let input = sine(SR as usize * 2, 440.0, 0.5);
    for kbps in [6, 160, 510] {
        assert_eq!(opus(&input, kbps), opus(&input, kbps), "{kbps}");
    }
    assert!(opus(&input, 32).len() < opus(&input, 256).len());
}

#[test]
fn encode_opus_rejects_bad_input() {
    let input = sine(100, 440.0, 0.5);
    for kbps in [0, 5, 511, u32::MAX] {
        assert!(
            matches!(
                encode_once(&input, SR, &EncoderOptions::Opus { kbps }),
                Err(RenderError::Encode(_))
            ),
            "{kbps}"
        );
    }
    let bad: [Vec<Vec<f32>>; 3] = [vec![], vec![vec![0.0; 4]], vec![vec![0.0; 4], vec![0.0; 5]]];
    for b in &bad {
        assert!(matches!(
            encode_once(b, SR, &EncoderOptions::Opus { kbps: 160 }),
            Err(RenderError::Encode(_))
        ));
    }
    assert!(matches!(
        encode_once(&input, 0, &EncoderOptions::Opus { kbps: 160 }),
        Err(RenderError::Encode(_))
    ));
}

fn rms_db(channels: &[Vec<f32>]) -> f64 {
    let (sum, n) = channels.iter().flatten().fold((0f64, 0usize), |(s, n), x| {
        (s + f64::from(*x).powi(2), n + 1)
    });
    10.0 * (sum / n as f64).log10()
}

#[test]
fn encode_decode_mp3_round_trip() {
    let n = SR as usize * 3;
    let input = sine(n, 440.0, 0.5);
    for (mode, q) in [(Mp3Mode::Cbr, 0), (Mp3Mode::Vbr, 0), (Mp3Mode::Vbr, 9)] {
        let d = decode(Format::Mp3, &mp3(&input, mode, q)).unwrap();
        assert_eq!(d.sample_rate, SR, "{mode:?} {q}");
        assert_eq!(d.channels.len(), 2, "{mode:?} {q}");
        let len = d.channels[0].len();
        assert_eq!(d.channels[1].len(), len);
        assert!(
            len + 2304 >= n && len <= n + 4608,
            "{mode:?} {q}: {len} frames for {n}"
        );
        let (a, b) = (rms_db(&input), rms_db(&d.channels));
        assert!((a - b).abs() < 0.25, "{mode:?} {q}: rms {b} dB vs {a} dB");
    }
}

#[test]
fn encode_decode_ogg_round_trip() {
    let n = SR as usize * 3;
    let input = sine(n, 440.0, 0.5);
    for q in [-1, 6, 10] {
        let d = decode(Format::Ogg, &ogg(&input, q)).unwrap();
        assert_eq!(d.sample_rate, SR, "{q}");
        assert_eq!(d.channels.len(), 2, "{q}");
        let len = d.channels[0].len();
        assert_eq!(d.channels[1].len(), len);
        assert!(len.abs_diff(n) <= 1152, "{q}: {len} frames for {n}");
        let (a, b) = (rms_db(&input), rms_db(&d.channels));
        assert!((a - b).abs() < 0.25, "{q}: rms {b} dB vs {a} dB");
    }
}

#[test]
fn encode_decode_opus_round_trip() {
    let pre_skip = usize::from(lookahead());
    for n in [1usize, 959, SR as usize * 3, 100_003] {
        let input = sine(n, 440.0, 0.5);
        let b = opus(&input, 160);
        let m = resampled_len(n, SR);
        let f = (m + pre_skip).div_ceil(960);
        let raw = decode(Format::Opus, &b).unwrap();
        assert_eq!(raw.sample_rate, 48_000);
        assert_eq!(raw.channels.len(), 2);
        assert_eq!(raw.channels[0].len(), f * 960, "{n}: every packet decoded");
        assert_eq!(raw.channels[1].len(), f * 960, "{n}");
        let trimmed = decode_opus_trimmed(&b).unwrap();
        assert_eq!(
            trimmed.channels[0].len(),
            m,
            "{n}: the length after the pre-skip"
        );
        assert_eq!(trimmed.channels[1].len(), m, "{n}");
        assert_eq!(
            &trimmed.channels[0][..],
            &raw.channels[0][pre_skip..pre_skip + m]
        );
        let exact = n as f64 * 48_000.0 / 44_100.0;
        assert!((m as f64 - exact).abs() <= 960.0, "{n}");
        if n >= SR as usize {
            let (a, b) = (rms_db(&input), rms_db(&trimmed.channels));
            assert!((a - b).abs() < 0.25, "{n}: rms {b} dB vs {a} dB");
        }
    }
}

#[test]
fn encode_decode_keeps_the_peak_of_an_impulse_train() {
    // Full-scale impulses every 100 ms, alternating sign; the true peak is measured over the
    // whole decoded stream, with no alignment.
    let n = SR as usize * 3;
    let mut ch = vec![0f32; n];
    for (k, i) in (2205..n).step_by(4410).enumerate() {
        ch[i] = if k % 2 == 0 { 1.0 } else { -1.0 };
    }
    let input = vec![ch.clone(), ch];
    let reference = true_peak_db(&input);
    for (format, bytes) in [
        (Format::Mp3, mp3(&input, Mp3Mode::Cbr, 0)),
        (Format::Ogg, ogg(&input, 6)),
        (Format::Opus, opus(&input, 160)),
    ] {
        let d = decode(format, &bytes).unwrap();
        let peak = true_peak_db(&d.channels);
        assert!(
            (peak - reference).abs() <= 1.0,
            "{format}: decoded true peak {peak:.2} dBTP vs {reference:.2}"
        );
        // Streamed packet by packet, the same value bit for bit.
        let streamed = decoded_true_peak_db(format, &bytes).unwrap();
        assert_eq!(streamed.to_bits(), peak.to_bits(), "{format}");
    }
}

#[test]
fn encode_decode_errors() {
    assert!(matches!(
        decode(Format::Wav, b"RIFF"),
        Err(RenderError::Encode(_))
    ));
    assert!(matches!(
        decoded_true_peak_db(Format::Wav, b"RIFF"),
        Err(RenderError::Encode(_))
    ));
    let garbage: Vec<u8> = (0..5000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for format in [Format::Mp3, Format::Ogg, Format::Opus] {
        for bytes in [&[][..], &garbage[..], b"OggS"] {
            match decode(format, bytes) {
                Err(RenderError::Encode(m)) => assert!(m.starts_with("verify: "), "{format}: {m}"),
                other => panic!("{format}: {other:?}"),
            }
            match decoded_true_peak_db(format, bytes) {
                Err(RenderError::Encode(m)) => assert!(m.starts_with("verify: "), "{format}: {m}"),
                other => panic!("{format}: {other:?}"),
            }
        }
    }
    // A Vorbis stream is not an Opus stream.
    let o = ogg(&sine(4410, 440.0, 0.5), 6);
    assert!(matches!(
        decode(Format::Opus, &o),
        Err(RenderError::Encode(_))
    ));
    assert!(matches!(
        decode_opus_trimmed(&o),
        Err(RenderError::Encode(_))
    ));
}

/// Dense seeded noise (an LCG), `seconds` long at amplitude 0.5.
fn noise(seconds: usize) -> Vec<Vec<f32>> {
    let mut state: u32 = 0x5eed_0001;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.5
    };
    let frames = seconds * SR as usize;
    let l: Vec<f32> = (0..frames).map(|_| next()).collect();
    let r: Vec<f32> = (0..frames).map(|_| next()).collect();
    vec![l, r]
}

fn encode_loop(
    input: &[Vec<f32>],
    loudness: strudeland_render::Loudness,
    encoder: EncoderOptions,
) -> (
    strudeland_render::encode::Encoded,
    Vec<strudeland_render::Progress>,
    Vec<String>,
) {
    let events = std::cell::RefCell::new(Vec::new());
    let mut warnings = Vec::new();
    let e = strudeland_render::encode::encode(
        input,
        SR,
        &strudeland_render::OutputOptions { loudness, encoder },
        &|p| events.borrow_mut().push(p),
        &mut warnings,
    )
    .unwrap();
    (e, events.into_inner(), warnings)
}

#[test]
fn encode_loop_corrects_a_lossy_overshoot_within_two_passes() {
    use strudeland_render::Loudness;
    use strudeland_render::level::level;
    let input = noise(3);
    for (target, encoder) in [
        (-5.0, EncoderOptions::Ogg { quality: 6 }),
        (-3.0, EncoderOptions::Opus { kbps: 160 }),
        (
            -3.0,
            EncoderOptions::Mp3 {
                mode: Mp3Mode::Cbr,
                vbr_quality: 0,
            },
        ),
    ] {
        let loudness = Loudness::On {
            target_lufs: target,
            true_peak_db: -1.0,
        };
        // Leveled to the ceiling, the first encode decodes above it.
        let first = level(&input, SR, &loudness, 0.0, &mut Vec::new()).unwrap();
        assert!(first.summary.output_true_peak_db <= -1.0, "{encoder:?}");
        let bytes = encode_once(&first.channels, SR, &encoder).unwrap();
        let dp = true_peak_db(&decode(encoder.format(), &bytes).unwrap().channels);
        assert!(dp > -1.0, "{encoder:?}: the first pass decodes at {dp}");

        let (e, events, warnings) = encode_loop(&input, loudness, encoder);
        let n = e.loudness.encodes;
        assert!((2..=3).contains(&n), "{encoder:?}: {n} encodes");
        assert_eq!(
            events,
            (1..=n)
                .map(|pass| strudeland_render::Progress::Encoded { pass })
                .collect::<Vec<_>>(),
            "{encoder:?}"
        );
        assert!(warnings.is_empty(), "{encoder:?}: {warnings:?}");
        let decoded = e.loudness.decoded_true_peak_db.unwrap();
        assert!(decoded <= -1.0, "{encoder:?}: decoded at {decoded}");
        // The correction lowered the limiter ceiling and kept the gain.
        assert_eq!(e.loudness.gain_db, first.summary.gain_db, "{encoder:?}");
        assert!(
            e.loudness.limiter_ceiling_dbtp.unwrap() < -1.0 - CORRECTION_MARGIN_DB,
            "{encoder:?}"
        );
        // The returned bytes are the last pass's: they decode to the reported peak.
        let again = true_peak_db(&decode(encoder.format(), &e.bytes).unwrap().channels);
        assert_eq!(again, decoded, "{encoder:?}");
    }
}

#[test]
fn encode_loop_reports_but_never_corrects_with_loudness_off() {
    use strudeland_render::Loudness;
    // Loud enough that the decoded file is over -1 dBTP.
    let input: Vec<Vec<f32>> = noise(2)
        .into_iter()
        .map(|c| c.into_iter().map(|x| x * 1.8).collect())
        .collect();
    for encoder in [
        EncoderOptions::Ogg { quality: 6 },
        EncoderOptions::Mp3 {
            mode: Mp3Mode::Cbr,
            vbr_quality: 0,
        },
    ] {
        let (e, events, warnings) = encode_loop(&input, Loudness::Off, encoder);
        assert_eq!(e.loudness.encodes, 1, "{encoder:?}");
        assert_eq!(
            events,
            vec![strudeland_render::Progress::Encoded { pass: 1 }]
        );
        assert_eq!(e.bytes, encode_once(&input, SR, &encoder).unwrap());
        let decoded = e.loudness.decoded_true_peak_db.unwrap();
        assert!(decoded > -1.0, "{encoder:?}: {decoded}");
        assert_eq!(
            decoded,
            true_peak_db(&decode(encoder.format(), &e.bytes).unwrap().channels)
        );
        assert_eq!(e.loudness.gain_db, 0.0);
        assert!(
            warnings.iter().all(|w| !w.starts_with("lossy encode")),
            "{warnings:?}"
        );
    }
}

//! Versioned browser operations and offline audition of a captured definition.
use std::time::Duration;

use strudeland_project::{InstrumentDefinition, InstrumentEntry, InstrumentLibrary, Project};
use strudeland_render::{
    BitDepth, EncoderOptions, LengthRequest, Loudness, OutputOptions, RenderOptions,
    encode::wav::encode_wav, render_audio,
};
use tauri::State;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::command_error::CommandError;
use crate::project::{ProjectState, with_project};

#[tauri::command]
pub async fn instrument_library_entries() -> Result<Vec<InstrumentEntry>, CommandError> {
    InstrumentLibrary::open_default()
        .and_then(|library| library.entries())
        .map_err(CommandError::other)
}

#[tauri::command]
pub async fn project_instrument_entries(
    state: State<'_, ProjectState>,
    root: String,
) -> Result<Vec<InstrumentEntry>, CommandError> {
    with_project(&state, &root, Project::instrument_entries)
}

fn capture(
    state: &ProjectState,
    root: Option<&str>,
    id: &str,
    fingerprint: &str,
) -> Result<InstrumentDefinition, CommandError> {
    match root {
        Some(root) => with_project(state, root, |p| p.instrument_definition(id, fingerprint)),
        None => InstrumentLibrary::open_default()
            .and_then(|lib| lib.definition(id, fingerprint))
            .map_err(CommandError::other),
    }
}

#[tauri::command]
pub async fn instrument_definition(
    state: State<'_, ProjectState>,
    root: Option<String>,
    id: String,
    fingerprint: String,
) -> Result<InstrumentDefinition, CommandError> {
    capture(&state, root.as_deref(), &id, &fingerprint)
}

#[tauri::command]
pub async fn project_write_instrument_pin(
    state: State<'_, ProjectState>,
    root: String,
    id: String,
    library_fingerprint: String,
    pin_fingerprint: Option<String>,
) -> Result<InstrumentEntry, CommandError> {
    with_project(&state, &root, |project| {
        project.write_instrument_pin(
            &InstrumentLibrary::open_default()?,
            &id,
            &library_fingerprint,
            pin_fingerprint.as_deref(),
        )
    })
}

static PREVIEW: Semaphore = Semaphore::const_new(1);

fn reserve_preview() -> Result<SemaphorePermit<'static>, CommandError> {
    PREVIEW
        .try_acquire()
        .map_err(|_| CommandError::busy("instrument preview is busy"))
}

#[tauri::command]
pub async fn instrument_preview(
    state: State<'_, ProjectState>,
    root: Option<String>,
    id: String,
    fingerprint: String,
) -> Result<tauri::ipc::Response, CommandError> {
    let permit = reserve_preview()?;
    let definition = capture(&state, root.as_deref(), &id, &fingerprint)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Keep the render slot until the worker exits, including when its caller disconnects.
        let _permit = permit;
        render_preview(id, definition)
            .map(tauri::ipc::Response::new)
            .map_err(CommandError::other)
    })
    .await
    .map_err(CommandError::other)?
}

// At 120 BPM one 4/4 bar lasts two seconds. Limit the wet reverb's impulse response
// before rendering: its allocation depends on roomsize independently of the output length.
const PHRASE: &str = r#"
const meta = { title: 'Instrument preview', bpm: 120, meter: '4/4' };
const [preset] = Object.values(instruments);
note("c4 e4 g4 c5").apply(preset()).fmap(value => {
    if (value.room > 0 && Number(value.roomsize ?? value.size ?? 2) > 4) {
        throw new Error('instrument preview supports roomsize up to 4 seconds');
    }
    return value;
})
"#;
const PREVIEW_PEAK: f32 = 0.35;
const BIT_DEPTH: BitDepth = BitDepth::B16;

fn render_preview(id: String, definition: InstrumentDefinition) -> Result<Vec<u8>, String> {
    let options = RenderOptions {
        instruments: [(id, definition)].into(),
        length: LengthRequest::Length { bars: 1 },
        jobs: Some(1),
        plan_timeout: Some(Duration::from_secs(5)),
        include_events: false,
        output: OutputOptions {
            loudness: Loudness::Off,
            encoder: EncoderOptions::Wav {
                bit_depth: BIT_DEPTH,
            },
        },
    };
    let mut audio = render_audio(PHRASE, &options, &|_| {}).map_err(|e| e.to_string())?;
    if !audio.summary.warnings.is_empty() {
        return Err(audio.summary.warnings.join("\n"));
    }
    let rate = audio.summary.sample_rate;
    let fade_frames = (rate / 100) as usize;
    let mut peak = 0.0_f32;
    for &sample in audio.channels.iter().flatten() {
        if !sample.is_finite() {
            return Err("instrument preview produced a non-finite sample".into());
        }
        peak = peak.max(sample.abs());
    }
    let gain = PREVIEW_PEAK / peak.max(PREVIEW_PEAK);
    for channel in &mut audio.channels {
        for (from_end, sample) in channel.iter_mut().rev().enumerate() {
            let fade = from_end.min(fade_frames) as f32 / fade_frames as f32;
            *sample *= gain * fade;
        }
    }
    encode_wav(&audio.channels, rate, BIT_DEPTH).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn voice(name: &str, sound: &str, pan: Value) -> Value {
        json!({"name": name, "chain": [
            {"control":"s", "value": sound}, {"control":"pan", "value": pan}, {"control":"gain", "value": 4}
        ]})
    }

    fn preset(voices: &[Value]) -> InstrumentDefinition {
        serde_json::from_value(json!({
            "schema":1, "parameters":{"position":0}, "voices":voices
        }))
        .unwrap()
    }

    fn preview_samples(voices: &[Value]) -> Vec<[i16; 2]> {
        let wav = render_preview("test".into(), preset(voices)).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 2);
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        assert_eq!(rate, 44_100);
        assert_eq!(wav.len(), 44 + 2 * rate as usize * 4);
        let frames = wav[44..]
            .chunks_exact(4)
            .map(|f| {
                [
                    i16::from_le_bytes([f[0], f[1]]),
                    i16::from_le_bytes([f[2], f[3]]),
                ]
            })
            .collect::<Vec<_>>();
        let limit = PREVIEW_PEAK * 32768.0 + 2.0; // PCM quantization and dither
        assert!(
            frames
                .iter()
                .flatten()
                .all(|v| f32::from(*v).abs() <= limit)
        );
        assert!(frames.last().unwrap().iter().all(|v| v.abs() <= 1));
        frames
    }

    #[test]
    fn a_second_preview_fails_immediately_while_the_worker_has_the_slot() {
        let active = reserve_preview().unwrap();
        assert_eq!(
            reserve_preview().unwrap_err(),
            CommandError::busy("instrument preview is busy")
        );
        drop(active);
        assert!(reserve_preview().is_ok());
    }

    #[test]
    fn default_parameter_places_the_preset_on_the_left_and_kit_adds_the_right_voice() {
        let left = voice("left", "sine", json!({"param":"position"}));
        let preset = preview_samples(std::slice::from_ref(&left));
        assert!(preset.iter().any(|[l, _]| l.abs() > 1000));
        assert!(preset.iter().all(|[_, r]| r.abs() <= 1));
        let kit = preview_samples(&[left, voice("right", "sawtooth", json!(1))]);
        for channel in 0..2 {
            assert!(kit.iter().any(|f| f[channel].abs() > 1000));
        }
    }

    #[test]
    fn preview_names_the_unsupported_sound_and_limits_reverb_work() {
        let unsupported = preset(&[voice("lead", "missing_sample", json!(0.5))]);
        let error = render_preview("test".into(), unsupported).unwrap_err();
        assert!(error.contains("missing_sample"), "{error}");
        let mut wet = voice("lead", "sine", json!(0.5));
        wet["chain"].as_array_mut().unwrap().extend([
            json!({"control": "room", "value": 1}),
            json!({"control": "roomsize", "value": 1000}),
        ]);
        let error = render_preview("test".into(), preset(&[wet])).unwrap_err();
        assert!(error.contains("roomsize"), "{error}");
    }

    #[test]
    fn pinned_capture_rejects_a_changed_project_and_needs_no_global_source() {
        let tmp = tempfile::tempdir().unwrap();
        let project = Project::create(&tmp.path().join("song"), "Song").unwrap();
        let library = InstrumentLibrary::at(tmp.path().join("library"));
        let expected = preset(&[voice("lead", "sine", json!(0.5))]);
        let entry = library
            .create("lead", &serde_json::to_string(&expected).unwrap())
            .unwrap();
        project.pin_instrument(&library, &entry.id).unwrap();
        std::fs::remove_dir_all(library.path()).unwrap();

        let state = ProjectState::default();
        let root = state.open_unwatched(project);
        assert_eq!(
            capture(&state, Some(&root), &entry.id, &entry.fingerprint).unwrap(),
            expected
        );
        assert!(capture(&state, Some(&root), &entry.id, "stale").is_err());
        let next = Project::create(&tmp.path().join("next"), "Next").unwrap();
        state.open_unwatched(next);
        assert_eq!(
            capture(&state, Some(&root), &entry.id, &entry.fingerprint).unwrap_err(),
            CommandError::project_changed()
        );
    }
}

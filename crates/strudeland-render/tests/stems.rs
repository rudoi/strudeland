mod common;

use std::fs;
use std::path::Path;

use common::{Recorder, options, window};
use serde_json::Value;
use strudeland_engine::wav::{DecodedWav, read_f32};
use strudeland_host::Layout;
use strudeland_render::{
    Progress, RenderError, RenderOptions, StemGrouping, render_audio, render_stems,
    resolve_project_stem_output, resolve_stem_output,
};

fn wav(path: &Path) -> DecodedWav {
    read_f32(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn same_window_in_every_file_across_chunks_and_workers() {
    let code = r#"
const meta = { bpm: 137, meter: '1/4' };
const lead = s('sine').freq(310).orbit(1).label('lead').clip(0.4).room(0.2).roomsize(0.3);
const bass = s('sine').freq(130).orbit(49).label('bass').clip(0.5).delay(0.3).delaytime(0.1).delayfeedback(0.1);
stack(lead, bass)
"#;
    let temp = tempfile::tempdir().unwrap();
    let mut opts = options(window(1, 20, Layout::Linear));
    opts.jobs = Some(1);
    let reference = render_audio(code, &opts, &|_| {}).unwrap();
    assert!(reference.summary.chunks > 1);
    let run = |opts: &RenderOptions, name| {
        let target = resolve_stem_output(
            Some(&temp.path().join(name)),
            Path::new("song.js"),
            &opts.length,
        )
        .unwrap();
        render_stems(code, opts, StemGrouping::Voice, &target, &|_| {}).unwrap()
    };
    let one = run(&opts, "one");
    opts.jobs = Some(2);
    let two = run(&opts, "two");
    assert_eq!(two.summary.jobs, 2);
    assert_eq!(one.files.len(), 6);
    for (a, b) in one.files.iter().zip(&two.files) {
        assert_eq!(a.file_name(), b.file_name());
        assert_eq!(fs::read(a).unwrap(), fs::read(b).unwrap());
    }
    let mix = wav(&one.files[0]);
    assert_eq!(mix.channels, reference.channels);
    let stems: Vec<_> = one.files[1..one.files.len() - 1]
        .iter()
        .map(|p| wav(p))
        .collect();
    for stem in &stems {
        assert_eq!(stem.sample_rate, mix.sample_rate);
        assert_eq!(stem.tag, 3);
        assert_eq!(stem.channels.len(), 2);
        assert!(
            stem.channels
                .iter()
                .all(|ch| ch.len() == one.summary.frames as usize)
        );
    }
    for (ch, mix) in mix.channels.iter().enumerate() {
        for (frame, &reference) in mix.iter().enumerate() {
            let values = || stems.iter().map(|s| f64::from(s.channels[ch][frame]));
            let sum: f64 = values().sum();
            let magnitude: f64 = values().map(f64::abs).sum();
            assert!(
                (sum - f64::from(reference)).abs() <= 8.0 * f64::from(f32::EPSILON) * magnitude
            );
        }
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(one.files.last().unwrap()).unwrap()).unwrap();
    assert_eq!(manifest["frames"], one.summary.frames);
    assert_eq!(manifest["window"]["fromBar"], 1);
    assert_eq!(manifest["window"]["toBar"], 20);
    assert_eq!(manifest["mix"], "mix.wav");
    assert_eq!(manifest["stems"][0]["kind"], "voice");
    assert_eq!(manifest["stems"][0]["id"], "bass");
    assert_eq!(manifest["stems"][3]["kind"], "effects");
}

#[test]
fn a_competing_manifest_leaves_published_files_and_reports_the_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("stems");
    let opts = options(window(0, 1, Layout::Linear));
    let target = resolve_stem_output(Some(&directory), Path::new("song.js"), &opts.length).unwrap();
    let manifest = directory.join("stems.json");
    let recorder = Recorder::new();
    let result = render_stems("s('sine')", &opts, StemGrouping::Orbit, &target, &|event| {
        if let Progress::Written { path } = &event
            && path.ends_with("mix.wav")
        {
            fs::write(&manifest, b"competing file").unwrap();
        }
        recorder.record(event);
    });
    let error = result.expect_err("a competing manifest must prevent completion");
    assert!(
        matches!(&error, RenderError::Io { path, source }
            if path == &manifest && source.kind() == std::io::ErrorKind::AlreadyExists),
        "{error:?}"
    );
    assert_eq!(fs::read(&manifest).unwrap(), b"competing file");
    let written: Vec<_> = recorder
        .events()
        .into_iter()
        .filter_map(|p| match p {
            Progress::Written { path } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(
        written,
        [directory.join("mix.wav"), directory.join("stem-001.wav")]
    );
    assert!(target.check_available().is_err());
}

#[cfg(unix)]
#[test]
fn replacing_the_reserved_directory_cannot_complete_an_incomplete_set() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("stems");
    let moved = tmp.path().join("moved");
    let opts = options(window(0, 1, Layout::Linear));
    let target = resolve_stem_output(Some(&directory), Path::new("song.js"), &opts.length).unwrap();
    let recorder = Recorder::new();
    let result = render_stems("s('sine')", &opts, StemGrouping::Orbit, &target, &|event| {
        if let Progress::Written { path } = &event
            && path.ends_with("mix.wav")
        {
            fs::rename(&directory, &moved).unwrap();
            fs::create_dir(&directory).unwrap();
        }
        recorder.record(event);
    });
    assert!(matches!(result, Err(RenderError::Io { path, source })
        if path == directory && source.to_string().contains("reserved render directory changed")));
    assert!(moved.join("mix.wav").is_file());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    assert_eq!(
        recorder
            .events()
            .iter()
            .filter(|event| matches!(event, Progress::Written { .. }))
            .count(),
        1
    );
}

#[test]
fn project_path_is_previewable_and_silence_produces_only_a_mix() {
    let tmp = tempfile::tempdir().unwrap();
    let project = strudeland_project::Project::create(&tmp.path().join("project"), "test").unwrap();
    let song = project.root().join("songs/unsaved.js");
    let opts = options(window(0, 1, Layout::Linear));
    let target = resolve_project_stem_output(&project, &song, &opts.length).unwrap();
    assert_eq!(
        target.path(),
        project.root().join("renders/unsaved-bars0-1-stems")
    );
    target.check_available().unwrap();
    assert!(!target.path().exists());
    let output = render_stems("s('~')", &opts, StemGrouping::Orbit, &target, &|_| {}).unwrap();
    assert_eq!(output.files.len(), 2);
    assert!(
        wav(&output.files[0])
            .channels
            .iter()
            .flatten()
            .all(|&x| x == 0.0)
    );
    let manifest: Value = serde_json::from_slice(&fs::read(&output.files[1]).unwrap()).unwrap();
    assert_eq!(manifest["stems"].as_array().unwrap().len(), 0);
}

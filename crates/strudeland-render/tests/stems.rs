mod common;

use std::fs;
use std::path::Path;

use common::{Recorder, options, window};
use serde_json::Value;
use strudeland_engine::wav::{DecodedWav, read_f32};
use strudeland_host::Layout;
use strudeland_render::export;
use strudeland_render::{
    Progress, RenderError, RenderOptions, StemExport, StemGrouping, render_audio,
};

fn request(opts: &RenderOptions, grouping: StemGrouping, song: &Path) -> export::Request {
    export::Request {
        song: song.into(),
        length: opts.length.clone(),
        kind: export::Kind::Stems { grouping },
        jobs: opts.jobs,
        plan_timeout: opts.plan_timeout,
    }
}

/// Prepares and runs a stem set into `dir`, forwarding pipeline progress.
fn stems(
    code: &str,
    opts: &RenderOptions,
    grouping: StemGrouping,
    dir: &Path,
    progress: &dyn Fn(Progress),
) -> Result<StemExport, RenderError> {
    let prepared = export::prepare(
        request(opts, grouping, Path::new("song.js")),
        export::Destination::Explicit(dir.into()),
    )
    .map_err(RenderError::from)?;
    match prepared.run(
        &mut export::RealPipeline,
        code,
        export::ReplacePolicy::Always,
        None,
        opts.instruments.clone(),
        &|e| {
            if let export::Event::Pipeline(p) = e {
                progress(p)
            }
        },
    ) {
        Ok(export::Finished::Stems { export, .. }) => Ok(export),
        Ok(_) => panic!("expected a stem set"),
        Err(e) => Err(e.source.into()),
    }
}

fn wav(path: &Path) -> DecodedWav {
    read_f32(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn stems_share_the_mixed_clip_grid_across_a_range_wrap() {
    let song = include_str!("../../../tests/render/mixed-meter-loop.js");
    // Clip bars 10 and 11 straddle the wrap from the last E bar to F1.
    let opts = options(window(10, 14, Layout::Clip));
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("stems");
    let result = stems(song, &opts, StemGrouping::Orbit, &dir, &|_| {}).unwrap();
    // E, F1, F1, F1 = 4/4 + 4/4 + 4/4 + 3/8 = 13.5 quarter notes.
    assert_eq!(result.summary.frames, 297_675);
    let mix = render_audio(song, &opts, &|_| {}).unwrap();
    assert_eq!(wav(&result.files[0]).channels, mix.channels);
    for file in result
        .files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "wav"))
    {
        assert!(wav(file).channels.iter().all(|ch| ch.len() == 297_675));
    }
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
        stems(
            code,
            opts,
            StemGrouping::Voice,
            &temp.path().join(name),
            &|_| {},
        )
        .unwrap()
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
    let manifest = directory.join("stems.json");
    let recorder = Recorder::new();
    let result = stems(
        "s('sine')",
        &opts,
        StemGrouping::Orbit,
        &directory,
        &|event| {
            if let Progress::Written { path } = &event
                && path.ends_with("mix.wav")
            {
                fs::write(&manifest, b"competing file").unwrap();
            }
            recorder.record(event);
        },
    );
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
    assert!(
        export::prepare_previewed(
            request(&opts, StemGrouping::Orbit, Path::new("song.js")),
            export::Destination::Explicit(directory.clone())
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn replacing_the_reserved_directory_cannot_complete_an_incomplete_set() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("stems");
    let moved = tmp.path().join("moved");
    let opts = options(window(0, 1, Layout::Linear));
    let recorder = Recorder::new();
    let result = stems(
        "s('sine')",
        &opts,
        StemGrouping::Orbit,
        &directory,
        &|event| {
            if let Progress::Written { path } = &event
                && path.ends_with("mix.wav")
            {
                fs::rename(&directory, &moved).unwrap();
                fs::create_dir(&directory).unwrap();
            }
            recorder.record(event);
        },
    );
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
    let (prepared, preview) = export::prepare_previewed(
        request(&opts, StemGrouping::Orbit, &song),
        export::Destination::project(project.clone()),
    )
    .unwrap();
    let directory = project.root().join("renders/unsaved-bars0-1-stems");
    assert!(
        matches!(&preview, export::Preview::Stems { directory: d } if d == &directory),
        "{preview:?}"
    );
    assert!(!directory.exists());
    let Ok(export::Finished::Stems { export: output, .. }) = prepared.run(
        &mut export::RealPipeline,
        "s('~')",
        export::ReplacePolicy::Always,
        None,
        opts.instruments.clone(),
        &|_| {},
    ) else {
        panic!("expected a stem set");
    };
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

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::time::Duration;

use strudeland_project::Project;

use super::*;
use crate::testing::{FakePipeline, output};
use crate::{Format, LengthRequest, RenderError};

const CLIP: LengthRequest = LengthRequest::Clip { loops: None };

fn mix(song: &Path, preset: Option<ExportPreset>) -> Request {
    Request {
        song: song.to_owned(),
        length: CLIP,
        kind: Kind::Mix {
            preset,
            output: OutputOptions {
                loudness: Loudness::default(),
                encoder: EncoderOptions::default_for(Format::Wav),
            },
        },
        jobs: None,
        plan_timeout: None,
    }
}

fn stems(song: &Path, length: LengthRequest) -> Request {
    Request {
        song: song.to_owned(),
        length,
        kind: Kind::Stems {
            grouping: StemGrouping::Orbit,
        },
        jobs: None,
        plan_timeout: None,
    }
}

fn file(role: ExportRole, bytes: &[u8], warnings: &[&str]) -> ExportFile {
    ExportFile {
        role,
        output: output(bytes, warnings),
        loop_points: None,
    }
}

fn intro_loop() -> Vec<ExportFile> {
    vec![
        file(ExportRole::Intro, b"intro", &[]),
        file(ExportRole::Loop, b"loop", &[]),
    ]
}

fn fake_render(bytes: &[u8]) -> FakePipeline {
    FakePipeline {
        render: Some(Ok(output(bytes, &[]))),
        real_publish: true,
        ..FakePipeline::default()
    }
}

fn fake_export(files: Vec<ExportFile>) -> FakePipeline {
    FakePipeline {
        export: Some(Ok(files)),
        real_publish: true,
        ..FakePipeline::default()
    }
}

fn project(dir: &Path) -> (Project, PathBuf) {
    let project = Project::create(&dir.join("p"), "p").unwrap();
    let song = project.root().join("songs/main.js");
    (project, song)
}

fn failed(result: Result<Finished, RunError>) -> Error {
    match result {
        Err(e) if e.written.is_empty() => e.source,
        other => panic!("not a failure before any publish: {other:?}"),
    }
}

fn partial(result: Result<Finished, RunError>) -> RunError {
    match result {
        Err(e) if !e.written.is_empty() => e,
        other => panic!("not a partial write: {other:?}"),
    }
}

fn mix_files(result: Result<Finished, RunError>) -> (Vec<Published>, Vec<String>) {
    match result {
        Ok(Finished::Mix {
            files, warnings, ..
        }) => (files, warnings),
        other => panic!("not a mix: {other:?}"),
    }
}

const SONG: &str = "const meta = { bpm: 240, meter: '1/4' }; note('a4').s('sine')";

#[test]
fn prepare_never_stats_the_explicit_file() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("song.js");
    let out = dir.path().join("out.wav");
    fs::create_dir(&out).unwrap();
    let prepared = prepare(mix(&song, None), Destination::Explicit(out.clone())).unwrap();
    let mut fake = fake_render(b"new");
    let result = prepared.run(
        &mut fake,
        "code",
        ReplacePolicy::Always,
        None,
        Default::default(),
        &|_| {},
    );
    assert_eq!(fake.seen.len(), 1);
    assert!(matches!(failed(result), Error::Project { .. }));
    assert!(fake.published.is_empty());
    assert!(out.is_dir());

    #[cfg(unix)]
    {
        let target = dir.path().join("target.wav");
        fs::write(&target, b"target").unwrap();
        let link = dir.path().join("link.wav");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let prepared = prepare(mix(&song, None), Destination::Explicit(link.clone())).unwrap();
        prepared
            .run(
                &mut fake_render(b"new"),
                "code",
                ReplacePolicy::Always,
                None,
                Default::default(),
                &|_| {},
            )
            .unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_file());
        assert_eq!(fs::read(&link).unwrap(), b"new");
        assert_eq!(fs::read(&target).unwrap(), b"target");
    }
}

#[test]
fn always_replaces_and_renders_is_created_on_publish() {
    let dir = tempfile::tempdir().unwrap();
    let (project, song) = project(dir.path());
    let renders = project.root().join("renders");
    let path = renders.join("main.wav");
    fs::write(&path, b"old").unwrap();
    let run = |bytes: &[u8]| {
        prepare(
            mix(&song, None),
            Destination::from_out(None, &song).unwrap(),
        )
        .unwrap()
        .run(
            &mut fake_render(bytes),
            "code",
            ReplacePolicy::Always,
            None,
            Default::default(),
            &|_| {},
        )
    };
    let (files, _) = mix_files(run(b"new"));
    assert_eq!(files[0].path, path);
    assert_eq!(fs::read(&path).unwrap(), b"new");

    fs::remove_dir_all(&renders).unwrap();
    let prepared = prepare(
        mix(&song, None),
        Destination::from_out(None, &song).unwrap(),
    )
    .unwrap();
    assert!(!renders.exists());
    drop(prepared);
    mix_files(run(b"again"));
    assert_eq!(fs::read(&path).unwrap(), b"again");
}

#[test]
fn create_new_needs_approval_before_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let (project, song) = project(dir.path());
    let path = project.root().join("renders/main.wav");
    fs::write(&path, b"old").unwrap();
    let previewed =
        || prepare_previewed(mix(&song, None), Destination::project(project.clone())).unwrap();
    let (prepared, preview) = previewed();
    assert_eq!(
        preview,
        Preview::Mix(vec![PreviewFile {
            role: ExportRole::Single,
            path: path.clone(),
            exists: true
        }])
    );
    let mut fake = fake_render(b"new");
    let result = prepared.run(
        &mut fake,
        "code",
        ReplacePolicy::CreateNew { approved: false },
        Some(&preview),
        Default::default(),
        &|_| {},
    );
    assert!(matches!(failed(result), Error::ReplacementNotApproved));
    assert!(fake.seen.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"old");

    let (prepared, _) = previewed();
    let result = prepared.run(
        &mut fake,
        "code",
        ReplacePolicy::CreateNew { approved: true },
        None,
        Default::default(),
        &|_| {},
    );
    assert!(matches!(failed(result), Error::PreviewRequired));

    for other in [
        Preview::Mix(vec![PreviewFile {
            role: ExportRole::Loop,
            path: path.clone(),
            exists: false,
        }]),
        Preview::Mix(vec![PreviewFile {
            role: ExportRole::Single,
            path: path.with_file_name("other.wav"),
            exists: false,
        }]),
        Preview::Stems {
            directory: path.clone(),
        },
    ] {
        let (prepared, _) = previewed();
        let result = prepared.run(
            &mut fake,
            "code",
            ReplacePolicy::CreateNew { approved: true },
            Some(&other),
            Default::default(),
            &|_| {},
        );
        assert!(matches!(failed(result), Error::RolesMismatch), "{other:?}");
    }
    assert!(fake.seen.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"old");
}

#[test]
fn create_new_keeps_a_competing_file() {
    let dir = tempfile::tempdir().unwrap();
    let (project, song) = project(dir.path());
    let folder = dir.path().join("out");
    fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    for destination in [
        Destination::ChosenFolder(folder.clone()),
        Destination::project(project.clone()),
    ] {
        let label = format!("{destination:?}");
        let (prepared, preview) =
            prepare_previewed(mix(&song, Some(ExportPreset::IntroLoop)), destination).unwrap();
        let first = prepared.targets()[0].path.clone();
        let second = prepared.targets()[1].path.clone();
        let result = prepared.run(
            &mut fake_export(intro_loop()),
            "code",
            ReplacePolicy::CreateNew { approved: true },
            Some(&preview),
            Default::default(),
            &|event| {
                if let Event::Written { file, .. } = event
                    && file.role == ExportRole::Intro
                {
                    fs::write(&second, b"competing").unwrap();
                }
            },
        );
        let p = partial(result);
        assert_eq!(
            p.written,
            [WrittenPath {
                path: first.clone(),
                absolute: first.clone()
            }],
            "{label}"
        );
        assert_eq!(p.failed.as_deref(), Some(second.as_path()), "{label}");
        assert!(matches!(p.source, Error::Project { .. }), "{label}");
        assert_eq!(fs::read(&first).unwrap(), b"intro");
        assert_eq!(fs::read(&second).unwrap(), b"competing");
    }
}

#[test]
fn a_moved_chosen_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let folder = dir.path().join("out");
    fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let moved = dir.path().join("moved");
    let (prepared, preview) = prepare_previewed(
        mix(&song, Some(ExportPreset::IntroLoop)),
        Destination::ChosenFolder(folder.clone()),
    )
    .unwrap();
    let result = prepared.run(
        &mut fake_export(intro_loop()),
        "code",
        ReplacePolicy::CreateNew { approved: true },
        Some(&preview),
        Default::default(),
        &|event| {
            if let Event::Written { .. } = event {
                fs::rename(&folder, &moved).unwrap();
            }
        },
    );
    let p = partial(result);
    assert_eq!(p.written.len(), 1);
    assert!(
        matches!(&p.source, Error::FolderChanged { path } if *path == folder),
        "{:?}",
        p.source
    );
    assert!(moved.join("main-intro.wav").is_file());
    assert!(!moved.join("main-loop.wav").exists());
}

#[test]
fn a_first_file_failure_is_not_partial() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let out = dir.path().join("pair.wav");
    for (before, partial_write) in [(0, false), (1, true)] {
        let prepared = prepare(
            mix(&song, Some(ExportPreset::IntroLoop)),
            Destination::Explicit(out.clone()),
        )
        .unwrap();
        let mut fake = FakePipeline {
            export: Some(Ok(intro_loop())),
            publish_error: Some((before, Error::Render(RenderError::Render("disk".into())))),
            ..FakePipeline::default()
        };
        let result = prepared.run(
            &mut fake,
            "code",
            ReplacePolicy::Always,
            None,
            Default::default(),
            &|_| {},
        );
        if partial_write {
            let p = partial(result);
            assert_eq!(p.written.len(), 1);
            assert_eq!(p.failed, Some(dir.path().join("pair-loop.wav")));
        } else {
            assert!(matches!(failed(result), Error::Render(_)));
        }
        assert_eq!(fake.published.len(), before);
    }
    // real_publish is off: nothing touched the disk.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn an_approved_file_that_vanished_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (project, song) = project(dir.path());
    let path = project.root().join("renders/main.wav");
    fs::write(&path, b"old").unwrap();
    let (prepared, preview) =
        prepare_previewed(mix(&song, None), Destination::project(project.clone())).unwrap();
    fs::remove_file(&path).unwrap();
    let (files, _) = mix_files(prepared.run(
        &mut fake_render(b"new"),
        "code",
        ReplacePolicy::CreateNew { approved: true },
        Some(&preview),
        Default::default(),
        &|_| {},
    ));
    assert_eq!(files[0].path, path);
    assert_eq!(fs::read(&path).unwrap(), b"new");
}

#[test]
fn events_are_ordered_and_written_is_synchronous() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let first = dir.path().join("pair-intro.wav");
    let second = dir.path().join("pair-loop.wav");
    let prepared = prepare(
        mix(&song, Some(ExportPreset::IntroLoop)),
        Destination::Explicit(dir.path().join("pair.wav")),
    )
    .unwrap();
    let mut fake = fake_export(intro_loop());
    fake.events = vec![
        Progress::Planned { chunks: 1 },
        Progress::ChunkRendered { index: 0, of: 1 },
        Progress::Encoded { pass: 1 },
    ];
    let order = RefCell::new(Vec::new());
    let on_disk = RefCell::new(Vec::new());
    let (files, _) = mix_files(prepared.run(
        &mut fake,
        "code",
        ReplacePolicy::Always,
        None,
        Default::default(),
        &|event| {
            order.borrow_mut().push(match &event {
                Event::Pipeline(p) => format!("{p:?}"),
                Event::Publishing => "Publishing".into(),
                Event::Written { file, .. } => {
                    on_disk
                        .borrow_mut()
                        .push((first.is_file(), second.exists()));
                    format!("Written {:?}", file.role)
                }
            });
        },
    ));
    assert_eq!(
        order.into_inner(),
        [
            "Planned { chunks: 1 }",
            "ChunkRendered { index: 0, of: 1 }",
            "Encoded { pass: 1 }",
            "Publishing",
            "Written Intro",
            "Written Loop",
        ]
    );
    assert_eq!(on_disk.into_inner(), [(true, false), (true, true)]);
    assert_eq!(fake.preset, Some(ExportPreset::IntroLoop));
    assert_eq!(fake.published, [first.clone(), second.clone()]);
    assert_eq!(
        files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        [first, second]
    );
}

#[test]
fn warnings_are_new_per_file_and_deduped_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let run = |preset, files: Vec<ExportFile>| {
        let prepared = prepare(
            mix(&song, preset),
            Destination::Explicit(dir.path().join("w.wav")),
        )
        .unwrap();
        let mut fake = FakePipeline::default();
        match preset {
            None => {
                fake.render = files.into_iter().next().map(|f| Ok(f.output));
            }
            Some(_) => fake.export = Some(Ok(files)),
        }
        let new = RefCell::new(Vec::new());
        let (_, warnings) = mix_files(prepared.run(
            &mut fake,
            "code",
            ReplacePolicy::Always,
            None,
            Default::default(),
            &|event| {
                if let Event::Written { new_warnings, .. } = event {
                    new.borrow_mut().push(new_warnings.to_vec());
                }
            },
        ));
        (new.into_inner(), warnings)
    };
    let (new, all) = run(
        Some(ExportPreset::IntroLoop),
        vec![
            file(ExportRole::Intro, b"i", &["a", "b"]),
            file(ExportRole::Loop, b"l", &["b", "c"]),
        ],
    );
    assert_eq!(new, [vec!["a", "b"], vec!["c"]]);
    assert_eq!(all, ["a", "b", "c"]);
    let (new, all) = run(None, vec![file(ExportRole::Single, b"s", &["a", "a"])]);
    assert_eq!(new, [vec!["a"]]);
    assert_eq!(all, ["a"]);
}

#[test]
fn render_options_pass_through() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let custom = OutputOptions {
        loudness: Loudness::On {
            target_lufs: -16.0,
            true_peak_db: -2.0,
        },
        encoder: EncoderOptions::default_for(Format::Mp3),
    };
    let instruments: InstrumentSnapshot = [(
        "lead".to_string(),
        serde_json::from_value(serde_json::json!({
            "schema": 1, "parameters": {}, "voices": [
                {"name": "lead", "chain": [{"control": "s", "value": "sawtooth"}]}
            ]
        }))
        .unwrap(),
    )]
    .into();
    let length = LengthRequest::Length { bars: 4 };
    let request = Request {
        song: song.clone(),
        length: length.clone(),
        kind: Kind::Mix {
            preset: None,
            output: custom,
        },
        jobs: Some(3),
        plan_timeout: Some(Duration::from_secs_f64(7.5)),
    };
    let mut fake = FakePipeline {
        render: Some(Ok(output(b"x", &[]))),
        ..FakePipeline::default()
    };
    prepare(request, Destination::Explicit(dir.path().join("o.mp3")))
        .unwrap()
        .run(
            &mut fake,
            "the code",
            ReplacePolicy::Always,
            None,
            instruments.clone(),
            &|_| {},
        )
        .unwrap();
    assert_eq!(
        fake.seen,
        [(
            "the code".to_string(),
            RenderOptions {
                instruments: instruments.clone(),
                length: length.clone(),
                jobs: Some(3),
                plan_timeout: Some(Duration::from_secs_f64(7.5)),
                include_events: false,
                output: custom,
            }
        )]
    );

    let set = dir.path().join("set");
    let mut request = stems(&song, length.clone());
    request.jobs = Some(2);
    let mut fake = FakePipeline {
        stems: Some(Err(RenderError::Render("stop".into()))),
        ..FakePipeline::default()
    };
    let prepared = prepare(request, Destination::Explicit(set.clone())).unwrap();
    assert_eq!(prepared.stem_directory(), Some(set.as_path()));
    assert!(prepared.targets().is_empty());
    assert_eq!(prepared.preset(), None);
    let _ = prepared.run(
        &mut fake,
        "stems",
        ReplacePolicy::Always,
        None,
        instruments.clone(),
        &|_| {},
    );
    assert_eq!(
        fake.seen,
        [(
            "stems".to_string(),
            RenderOptions {
                instruments,
                length,
                jobs: Some(2),
                plan_timeout: None,
                include_events: false,
                output: OutputOptions {
                    loudness: Loudness::Off,
                    encoder: EncoderOptions::Wav {
                        bit_depth: BitDepth::F32
                    },
                },
            }
        )]
    );
    assert_eq!(fake.stem_targets, [set]);
}

#[test]
fn a_role_mismatch_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let prepared = prepare(
        mix(&song, Some(ExportPreset::IntroLoop)),
        Destination::Explicit(dir.path().join("pair.wav")),
    )
    .unwrap();
    let mut fake = fake_export(vec![file(ExportRole::Single, b"s", &[])]);
    let events = RefCell::new(0);
    let result = prepared.run(
        &mut fake,
        "code",
        ReplacePolicy::Always,
        None,
        Default::default(),
        &|event| {
            if !matches!(event, Event::Pipeline(_)) {
                *events.borrow_mut() += 1;
            }
        },
    );
    assert!(matches!(failed(result), Error::RolesMismatch));
    assert!(fake.published.is_empty());
    assert_eq!(events.into_inner(), 0);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn stem_sets_never_replace_and_publish_the_manifest_last() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let length = LengthRequest::Length { bars: 1 };
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep.txt"), b"keep").unwrap();
    let destination = || Destination::Explicit(existing.clone());
    assert!(prepare_previewed(stems(&song, length.clone()), destination()).is_err());
    let result = prepare(stems(&song, length.clone()), destination())
        .unwrap()
        .run(
            &mut RealPipeline,
            SONG,
            ReplacePolicy::Always,
            None,
            Default::default(),
            &|_| {},
        );
    assert!(matches!(
        failed(result),
        Error::Render(RenderError::Io { ref source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists
    ),);
    let names: Vec<_> = fs::read_dir(&existing)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["keep.txt"]);
    assert_eq!(fs::read(existing.join("keep.txt")).unwrap(), b"keep");

    let fresh = dir.path().join("fresh");
    let (prepared, preview) = prepare_previewed(
        stems(&song, length.clone()),
        Destination::Explicit(fresh.clone()),
    )
    .unwrap();
    assert_eq!(
        preview,
        Preview::Stems {
            directory: fresh.clone()
        }
    );
    let mix_events = RefCell::new(0);
    let finished = prepared
        .run(
            &mut RealPipeline,
            SONG,
            ReplacePolicy::CreateNew { approved: false },
            None,
            Default::default(),
            &|event| {
                if !matches!(event, Event::Pipeline(_)) {
                    *mix_events.borrow_mut() += 1;
                }
            },
        )
        .unwrap();
    let Finished::Stems { grouping, export } = finished else {
        panic!("not a stem set");
    };
    assert_eq!(grouping, StemGrouping::Orbit);
    assert!(export.files.last().unwrap().ends_with("stems.json"));
    assert!(export.files.first().unwrap().ends_with("mix.wav"));
    assert_eq!(mix_events.into_inner(), 0);
}

#[test]
fn stem_failures_report_written_paths() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("main.js");
    let set = dir.path().join("set");
    let mix_path = set.join("mix.wav");
    for events in [
        vec![Progress::Written {
            path: mix_path.clone(),
        }],
        vec![],
    ] {
        let wrote = !events.is_empty();
        let mut fake = FakePipeline {
            events,
            stems: Some(Err(RenderError::Io {
                path: set.join("stem-001.wav"),
                source: std::io::Error::other("full"),
            })),
            ..FakePipeline::default()
        };
        let result = prepare(
            stems(&song, LengthRequest::Length { bars: 1 }),
            Destination::Explicit(set.clone()),
        )
        .unwrap()
        .run(
            &mut fake,
            "code",
            ReplacePolicy::Always,
            None,
            Default::default(),
            &|_| {},
        );
        if wrote {
            let p = partial(result);
            assert_eq!(p.failed, None);
            assert_eq!(
                p.written,
                [WrittenPath {
                    path: mix_path.clone(),
                    absolute: mix_path.clone()
                }]
            );
            assert!(matches!(p.source, Error::Render(RenderError::Io { .. })));
        } else {
            assert!(matches!(
                failed(result),
                Error::Render(RenderError::Io { .. })
            ));
        }
    }
}

#[test]
fn a_missing_fake_result_is_an_error() {
    let mut fake = FakePipeline::default();
    let opts = RenderOptions {
        instruments: Default::default(),
        length: CLIP,
        jobs: None,
        plan_timeout: None,
        include_events: false,
        output: OutputOptions {
            loudness: Loudness::Off,
            encoder: EncoderOptions::default_for(Format::Wav),
        },
    };
    let e = fake.render("x", &opts, &|_| {}).unwrap_err();
    assert_eq!(e.to_string(), "render failed: fake pipeline: no result");
}

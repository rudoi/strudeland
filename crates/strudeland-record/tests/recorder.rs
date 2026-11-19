//! The recorder end to end with the fake source: control thread, writer, results, events.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

use strudeland_record::testing::{FakeHandle, FakeSource, Limits};
use strudeland_record::{
    Anchor, ArmConfig, ArmedInfo, Ended, Event, FaultKind, RecordError, Recorder, START_TIMEOUT,
    TakeResult,
};

const RATE: u32 = 1_000;
const WAIT: Duration = Duration::from_secs(5);

fn info(channel: u16) -> ArmedInfo {
    ArmedInfo {
        device_id: "fake:mic".to_string(),
        device_name: "fake mic".to_string(),
        channel,
        channels: 2,
        sample_rate: RATE,
        bluetooth: false,
    }
}

struct Rig {
    rec: Recorder,
    events: Receiver<Event>,
    handles: Receiver<FakeHandle>,
    dir: tempfile::TempDir,
}

fn rig_with(limits: Limits) -> Rig {
    let (handle_tx, handles) = mpsc::channel();
    let factory = Box::new(move |cfg: &ArmConfig| {
        let (source, handle) = FakeSource::new(info(cfg.channel));
        handle_tx.send(handle).unwrap();
        Ok(Box::new(source) as Box<dyn strudeland_record::Source>)
    });
    let (rec, events) = Recorder::with_limits(factory, limits);
    Rig {
        rec,
        events,
        handles,
        dir: tempfile::tempdir().unwrap(),
    }
}

fn rig() -> Rig {
    rig_with(Limits::default())
}

impl Rig {
    fn arm(&self) -> FakeHandle {
        self.rec.arm(ArmConfig::default()).unwrap();
        self.handles.recv_timeout(WAIT).unwrap()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Starts a take at `path`, delivering `first` at `host_ns` once the take runs.
    fn start(
        &self,
        handle: &FakeHandle,
        path: &Path,
        first: &[f32],
        host_ns: impl Into<Option<u64>> + Copy,
    ) -> Result<(), RecordError> {
        let rec = &self.rec;
        std::thread::scope(|s| {
            let started = s.spawn(move || rec.start(path));
            while !started.is_finished() {
                if handle.is_recording() {
                    assert!(handle.push(first, host_ns));
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            started.join().unwrap()
        })
    }

    /// The next event that is not a level.
    fn next(&self) -> Event {
        loop {
            match self.events.recv_timeout(WAIT).expect("an event") {
                Event::Level(_) => continue,
                other => return other,
            }
        }
    }

    fn no_more_events(&self) {
        std::thread::sleep(Duration::from_millis(50));
        let rest: Vec<Event> = self
            .events
            .try_iter()
            .filter(|e| !matches!(e, Event::Level(_)))
            .collect();
        assert!(rest.is_empty(), "unexpected events: {rest:?}");
    }
}

fn samples(path: &Path) -> (hound::WavSpec, Vec<f32>) {
    let reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    (spec, reader.into_samples().map(Result::unwrap).collect())
}

fn ramp(n: usize, offset: usize) -> Vec<f32> {
    (offset..offset + n)
        .map(|i| (i % 1_000) as f32 / 1_000.0)
        .collect()
}

fn take_ended(event: Event) -> TakeResult {
    match event {
        Event::TakeEnded(r) => r,
        other => panic!("expected TakeEnded, got {other:?}"),
    }
}

#[test]
fn the_recorder_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Recorder>();
}

#[test]
fn the_start_timeout_is_two_seconds() {
    assert_eq!(START_TIMEOUT, Duration::from_secs(2));
    assert_eq!(Limits::default().start_timeout, START_TIMEOUT);
    assert_eq!(Limits::default().max_frames, None);
}

#[test]
fn a_take_writes_mono_float_samples_and_anchors() {
    let rig = rig();
    let handle = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(i) if i == info(0)));
    let path = rig.path("take.wav");
    let audio = ramp(3_000, 0);
    rig.start(&handle, &path, &audio[..100], 1_000_000).unwrap();
    assert!(matches!(rig.next(), Event::TakeStarted));
    for (k, block) in audio[100..].chunks(100).enumerate() {
        let host_ns = 1_000_000 + (k as u64 + 1) * 100_000_000;
        assert!(handle.push(block, host_ns));
    }
    let result = rig.rec.stop().unwrap();
    assert_eq!(
        result,
        TakeResult {
            path: path.clone(),
            frames: 3_000,
            sample_rate: RATE,
            channel: 0,
            anchors: vec![
                Anchor {
                    frame: 0,
                    host_ns: 1_000_000
                },
                Anchor {
                    frame: 1_000,
                    host_ns: 1_001_000_000
                },
                Anchor {
                    frame: 2_000,
                    host_ns: 2_001_000_000
                },
            ],
            xruns: 0,
            ended: Ended::User,
        }
    );
    assert_eq!(take_ended(rig.next()), result);
    let (spec, got) = samples(&path);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.sample_rate, RATE);
    assert_eq!(got, audio);
    // Still armed: a second take works, and the stored result is not returned again.
    assert!(handle.is_open());
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    let second = rig.path("second.wav");
    rig.start(&handle, &second, &[0.5; 10], 9).unwrap();
    let result = rig.rec.stop().unwrap();
    assert_eq!(result.frames, 10);
    assert_eq!(
        result.anchors,
        vec![Anchor {
            frame: 0,
            host_ns: 9
        }]
    );
}

#[test]
fn start_refuses_an_existing_file() {
    let rig = rig();
    let handle = rig.arm();
    let path = rig.path("exists.wav");
    std::fs::write(&path, b"keep").unwrap();
    let err = rig.start(&handle, &path, &[0.0], 0).unwrap_err();
    let RecordError::Io(io) = err else {
        panic!("expected Io, got {err:?}");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    assert!(!handle.is_recording());
    rig.start(&handle, &rig.path("other.wav"), &[0.0], 0)
        .unwrap();
}

#[test]
fn start_and_stop_before_arm_are_not_armed() {
    let rig = rig();
    let path = rig.path("x.wav");
    assert!(matches!(rig.rec.start(&path), Err(RecordError::NotArmed)));
    assert!(!path.exists());
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    rig.rec.disarm();
    rig.no_more_events();
}

#[test]
fn double_start_and_arm_while_recording_are_already_recording() {
    let rig = rig();
    let handle = rig.arm();
    rig.start(&handle, &rig.path("a.wav"), &[0.0], 0).unwrap();
    let second = rig.path("b.wav");
    assert!(matches!(
        rig.rec.start(&second),
        Err(RecordError::AlreadyRecording)
    ));
    assert!(!second.exists());
    assert!(matches!(
        rig.rec.arm(ArmConfig::default()),
        Err(RecordError::AlreadyRecording)
    ));
    assert!(matches!(
        rig.rec.arm(ArmConfig {
            device_id: None,
            channel: 1
        }),
        Err(RecordError::AlreadyRecording)
    ));
    assert!(rig.handles.try_recv().is_err());
}

#[test]
fn arming_twice_is_a_no_op_and_another_config_rearms() {
    let rig = rig();
    let first = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(_)));
    assert_eq!(rig.rec.arm(ArmConfig::default()).unwrap(), info(0));
    assert!(rig.handles.try_recv().is_err(), "the factory ran again");
    assert!(first.is_open());
    rig.no_more_events();

    let other = ArmConfig {
        device_id: None,
        channel: 1,
    };
    assert_eq!(rig.rec.arm(other).unwrap(), info(1));
    let second = rig.handles.recv_timeout(WAIT).unwrap();
    assert!(matches!(rig.next(), Event::Disarmed));
    assert!(matches!(rig.next(), Event::Armed(i) if i == info(1)));
    assert!(!first.is_open());
    assert!(second.is_open());
}

#[test]
fn a_failing_factory_is_the_arm_error() {
    let (rec, events) =
        Recorder::with_source_factory(Box::new(|_: &ArmConfig| Err(RecordError::PermissionDenied)));
    assert!(matches!(
        rec.arm(ArmConfig::default()),
        Err(RecordError::PermissionDenied)
    ));
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        rec.start(&dir.path().join("x.wav")),
        Err(RecordError::NotArmed)
    ));
    drop(rec);
    assert_eq!(events.try_iter().count(), 0);
}

#[test]
fn no_samples_is_a_start_timeout_and_removes_the_file() {
    let rig = rig_with(Limits {
        start_timeout: Duration::from_millis(100),
        ..Limits::default()
    });
    let handle = rig.arm();
    let path = rig.path("silent.wav");
    assert!(matches!(
        rig.rec.start(&path),
        Err(RecordError::StartTimeout)
    ));
    assert!(!path.exists());
    assert!(!handle.is_recording());
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    // Still armed and the same path is free again.
    rig.start(&handle, &path, &[0.25; 4], 1).unwrap();
    assert_eq!(rig.rec.stop().unwrap().frames, 4);
}

#[test]
fn the_cap_ends_the_take_at_exactly_max_frames() {
    let rig = rig_with(Limits {
        max_frames: Some(1_000),
        ..Limits::default()
    });
    let handle = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(_)));
    let path = rig.path("long.wav");
    let audio = ramp(1_550, 0);
    rig.start(&handle, &path, &audio[..50], 0).unwrap();
    assert!(matches!(rig.next(), Event::TakeStarted));
    for block in audio[50..].chunks(100) {
        handle.push(block, 0);
    }
    let ended = take_ended(rig.next());
    assert_eq!(ended.ended, Ended::Cap);
    assert_eq!(ended.frames, 1_000);
    assert!(!handle.is_recording());
    assert!(handle.is_open(), "the stream stays open after the cap");
    assert_eq!(rig.rec.stop().unwrap(), ended);
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    assert_eq!(samples(&path).1, audio[..1_000]);
    rig.no_more_events();
}

#[test]
fn device_loss_while_recording_finalizes_then_fails() {
    let rig = rig();
    let handle = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(_)));
    let path = rig.path("lost.wav");
    let audio = ramp(300, 0);
    rig.start(&handle, &path, &audio[..100], 5).unwrap();
    assert!(matches!(rig.next(), Event::TakeStarted));
    handle.push(&audio[100..], 6);
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    let ended = take_ended(rig.next());
    assert_eq!(ended.ended, Ended::DeviceLost("unplugged".to_string()));
    assert_eq!(ended.frames, 300);
    assert!(
        matches!(rig.next(), Event::Failed { error: RecordError::DeviceLost(m), .. } if m == "unplugged")
    );
    rig.no_more_events();
    assert_eq!(samples(&path).1, audio);
    assert!(!handle.is_open());
    assert!(matches!(
        rig.rec.start(&rig.path("again.wav")),
        Err(RecordError::NotArmed)
    ));
    assert_eq!(rig.rec.stop().unwrap(), ended);
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
}

#[test]
fn a_fault_while_armed_and_idle_only_fails() {
    let rig = rig();
    let handle = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(_)));
    assert!(handle.fault(FaultKind::Other, "backend broke"));
    assert!(
        matches!(rig.next(), Event::Failed { error: RecordError::Backend(m), .. } if m == "backend broke")
    );
    rig.no_more_events();
    assert!(!handle.is_open());
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    // Arming again builds a new source.
    let again = rig.arm();
    assert!(again.is_open());
}

#[test]
fn xruns_and_dropped_blocks_count_during_the_take_only() {
    let rig = rig();
    let handle = rig.arm();
    assert!(handle.xrun());
    rig.start(&handle, &rig.path("x.wav"), &[0.0; 10], 0)
        .unwrap();
    assert!(handle.xrun());
    assert!(handle.xrun());
    // Larger than the 4 s ring: dropped and counted, without blocking.
    let started = std::time::Instant::now();
    assert!(handle.push(&vec![0.0; 4 * RATE as usize + 1], 1));
    assert!(started.elapsed() < Duration::from_millis(200));
    handle.push(&[0.0; 10], 2);
    let result = rig.rec.stop().unwrap();
    assert_eq!(result.xruns, 3);
    assert_eq!(result.frames, 20);
}

#[test]
fn disarm_mid_take_ends_it_and_does_not_store_it() {
    let rig = rig();
    let handle = rig.arm();
    assert!(matches!(rig.next(), Event::Armed(_)));
    let path = rig.path("d.wav");
    rig.start(&handle, &path, &[0.5; 20], 0).unwrap();
    assert!(matches!(rig.next(), Event::TakeStarted));
    rig.rec.disarm();
    let ended = take_ended(rig.next());
    assert_eq!((ended.frames, ended.ended), (20, Ended::User));
    assert!(matches!(rig.next(), Event::Disarmed));
    assert!(!handle.is_open());
    assert!(matches!(rig.rec.stop(), Err(RecordError::NotArmed)));
    assert_eq!(samples(&path).1, vec![0.5; 20]);
}

#[test]
fn dropping_the_recorder_mid_take_finalizes_the_file() {
    let rig = rig();
    let handle = rig.arm();
    let path = rig.path("quit.wav");
    let audio = ramp(500, 0);
    rig.start(&handle, &path, &audio, 0).unwrap();
    let Rig { rec, events, .. } = rig;
    drop(rec);
    let rest: Vec<Event> = events
        .try_iter()
        .filter(|e| !matches!(e, Event::Level(_)))
        .collect();
    assert!(
        matches!(rest.as_slice(), [Event::Armed(_), Event::TakeStarted, Event::TakeEnded(r), Event::Disarmed] if r.frames == 500)
    );
    assert_eq!(samples(&path).1, audio);
}

#[test]
fn shutdown_finalizes_like_drop() {
    let rig = rig();
    let handle = rig.arm();
    let path = rig.path("s.wav");
    rig.start(&handle, &path, &[0.125; 7], 0).unwrap();
    let Rig { rec, dir, .. } = rig;
    rec.shutdown();
    assert_eq!(samples(&path).1, vec![0.125; 7]);
    drop(dir);
}

#[test]
fn levels_flow_while_armed_without_a_take() {
    let rig = rig();
    let handle = rig.arm();
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        handle.push(&[0.5, -0.5], 0);
        if let Ok(Event::Level(level)) = rig.events.recv_timeout(Duration::from_millis(10)) {
            assert_eq!(level.peak, 0.5);
            assert_eq!(level.rms, 0.5);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no level event");
    }
}

#[test]
fn an_unflushed_copy_is_repaired_by_the_project() {
    let project_dir = tempfile::tempdir().unwrap();
    let root = project_dir.path().join("p");
    let project = strudeland_project::Project::create(&root, "p").unwrap();
    let rig = rig();
    let handle = rig.arm();
    let path = rig.path("live.wav");
    let audio = ramp(1_200, 0);
    rig.start(&handle, &path, &audio[..500], 0).unwrap();
    assert!(handle.wait_drained(WAIT));
    // Past the 1 s flush with 500 frames written, then more that stay in the write buffer.
    std::thread::sleep(Duration::from_millis(1_200));
    handle.push(&audio[500..], 1);
    assert!(handle.wait_drained(WAIT));

    let partial = project.recording_partial_path("take").unwrap();
    std::fs::copy(&path, &partial).unwrap();
    let flushed = hound::WavReader::open(&partial).unwrap().len();
    assert_eq!(flushed, 500, "the copy holds the flushed frames");
    // Tear the last frame: 300 whole frames and half of the next.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&partial)
        .unwrap();
    file.set_len(68 + 300 * 4 + 2).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(60))
        .unwrap();
    drop(file);
    rig.rec.stop().unwrap();

    let report = project.repair_partials().unwrap();
    assert_eq!(report.recovered.len(), 1, "{report:?}");
    assert_eq!(report.recovered[0].stem, "take");
    assert_eq!(report.recovered[0].frames, 300);
    let (spec, got) = samples(&project.recording_wav_path("take"));
    assert_eq!(spec.sample_rate, RATE);
    assert_eq!(got, audio[..300]);
    assert!(!partial.exists());
}

#[test]
fn each_stream_has_its_own_arm_id_and_failed_carries_it() {
    let rig = rig();
    let (first, _) = rig.rec.arm_with_id(ArmConfig::default()).unwrap();
    let handle = rig.handles.recv_timeout(WAIT).unwrap();
    assert!(matches!(rig.next(), Event::Armed(_)));
    // A no-op re-arm keeps the id.
    assert_eq!(rig.rec.arm_with_id(ArmConfig::default()).unwrap().0, first);
    assert!(handle.fault(FaultKind::Other, "gone"));
    assert!(matches!(rig.next(), Event::Failed { arm_id, .. } if arm_id == first));
    let (second, _) = rig.rec.arm_with_id(ArmConfig::default()).unwrap();
    assert_ne!(second, first);
}

#[test]
fn a_take_without_reliable_timing_keeps_all_samples() {
    let rig = rig();
    let handle = rig.arm();
    let path = rig.path("unclocked.wav");
    rig.start(&handle, &path, &[0.5; 10], None).unwrap();
    assert!(handle.push(&[0.25; 10], 100));
    let result = rig.rec.stop().unwrap();
    assert_eq!(result.frames, 20);
    assert!(result.anchors.is_empty());
    assert_eq!(samples(&path).1, [[0.5; 10], [0.25; 10]].concat());
}

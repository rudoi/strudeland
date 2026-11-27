//! The take lifecycle over `FakeSource`, `FakePlatform`, `CollectingSink` and a temp project.

use super::*;
use std::ops::Deref;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;
use strudeland_project::SidecarInput;
use tempfile::TempDir;

use crate::FaultKind;
use crate::testing::{
    FakeHandle, FakePlatform, Limits, RATE, T0, TakesRig, WAIT, block_publication, sidecar_of,
    start_fields,
};

/// [`TakesRig`] over a project `alpha` in a temporary directory.
struct Rig {
    tmp: TempDir,
    core: TakesRig,
}

impl Deref for Rig {
    type Target = TakesRig;

    fn deref(&self) -> &TakesRig {
        &self.core
    }
}

fn rig_with(limits: Limits) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let project = Arc::new(Project::create(&tmp.path().join("alpha"), "alpha").unwrap());
    let config = TakesConfig {
        app_version: "test".to_string(),
    };
    let core = TakesRig::new(tmp.path(), project, limits, config);
    Rig { tmp, core }
}

fn rig() -> Rig {
    rig_with(Limits::default())
}

fn stop_fields(bar1_host_ns: Option<&str>) -> TakeStop {
    TakeStop {
        output: serde_json::from_value(json!({ "baseLatency": 0.005, "outputLatency": 0.02 }))
            .unwrap(),
        tempo: serde_json::from_value(json!({
            "bpm": 90, "meter": "6/8", "cps": 0.5, "beatsPerBar": 3, "tempoOverride": true,
        }))
        .unwrap(),
        backing: serde_json::from_value(json!({
            "song": true, "metronome": true, "countIn": false, "songPath": "songs/main.js",
            "startSection": "verse", "loopSection": null,
        }))
        .unwrap(),
        bar1_host_ns: serde_json::from_value(json!(bar1_host_ns)).unwrap(),
        clock_offset_ns: serde_json::from_value(
            json!({ "start": { "offsetNs": "-12", "rttNs": "40" }, "end": null }),
        )
        .unwrap(),
        output_stamps: serde_json::from_value(json!([[0.5, "1000000123"]])).unwrap(),
    }
}

impl Rig {
    /// Starts a take in the rig's project and delivers its first block (10 frames at `T0`).
    fn start(&self, handle: &FakeHandle, song_stem: Option<&str>) -> Result<String, TakeError> {
        self.start_with(handle, song_stem, start_fields())
    }

    /// [`Rig::start`] with these start fields.
    fn start_with(
        &self,
        handle: &FakeHandle,
        song_stem: Option<&str>,
        fields: TakeStart,
    ) -> Result<String, TakeError> {
        self.start_in(handle, self.resolve(), song_stem, fields)
    }
}

/// The arm id of the state's armed input, if it has one.
fn armed_id(takes: &Takes) -> Option<u64> {
    takes.lock().state.armed().map(|a| a.arm_id)
}

fn record(e: RecordError) -> TakeError {
    TakeError::Record(Arc::new(e))
}

// --- permission ---

#[test]
fn not_determined_asks_then_arms_when_granted() {
    let rig = rig();
    *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
    let status = rig.takes.arm(&rig.app_data(), None, None).unwrap();
    assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
    assert_eq!(status.state, RecState::Armed);
    assert_eq!(status.permission, Permission::Authorized);
    rig.next_handle();
    assert_eq!(rig.sink.states(), vec![TakeEvent::new(RecState::Armed)]);
}

#[test]
fn not_determined_asks_then_refuses_when_denied() {
    let rig = rig();
    *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
    *rig.platform.answer.lock().unwrap() = Permission::Denied;
    let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
    assert_eq!(error, TakeError::PermissionDenied);
    assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
    rig.no_new_handle();
    assert_eq!(
        rig.sink.states(),
        vec![TakeEvent::new(RecState::Idle).error(error)]
    );
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Idle);
}

#[test]
fn denied_and_restricted_open_no_stream() {
    for (permission, expected) in [
        (Permission::Denied, TakeError::PermissionDenied),
        (Permission::Restricted, TakeError::PermissionRestricted),
    ] {
        let rig = rig();
        *rig.platform.permission.lock().unwrap() = permission;
        let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
        assert_eq!(error, expected, "{permission:?}");
        assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 0);
        rig.no_new_handle();
        assert_eq!(
            rig.sink.states(),
            vec![TakeEvent::new(RecState::Idle).error(expected)]
        );
    }
}

#[test]
fn on_a_desktop_without_a_prompt_the_stream_decides_access_and_can_be_retried() {
    let rig = rig();
    rig.platform.has_prompt.store(false, Ordering::Release);
    *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
    assert_eq!(
        rig.takes.status(&rig.app_data()).permission,
        Permission::NotDetermined
    );
    *rig.arm_failure.lock().unwrap() = Some(RecordError::PermissionDenied);
    let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
    assert_eq!(error, TakeError::PermissionDenied);
    assert_eq!(
        rig.takes.status(&rig.app_data()).permission,
        Permission::Denied
    );
    let armed = rig.takes.arm(&rig.app_data(), None, None).unwrap();
    assert_eq!(armed.permission, Permission::Authorized);
    assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 0);
}

// --- permission (threads) ---

#[test]
fn a_second_arm_during_a_pending_request_is_busy() {
    let rig = rig();
    *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
    let (release, gate) = mpsc::channel();
    *rig.platform.gate.lock().unwrap() = Some(gate);
    std::thread::scope(|s| {
        let first = s.spawn(|| rig.takes.arm(&rig.app_data(), None, None));
        let deadline = Instant::now() + WAIT;
        while rig.platform.requests.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let second = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
        assert_eq!(second, TakeError::PermissionPending);
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap().unwrap().state, RecState::Armed);
    });
    assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
    assert!(!rig.takes.requesting.load(Ordering::SeqCst));
}

// --- arm and disarm ---

#[test]
fn arm_without_devices_is_no_device() {
    let rig = rig();
    rig.platform.devices.lock().unwrap().clear();
    let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
    assert_eq!(error, record(RecordError::NoDevice));
    let error = rig
        .takes
        .arm(&rig.app_data(), Some("fake:gone".into()), None)
        .unwrap_err();
    assert_eq!(error, record(RecordError::NoDevice));
    rig.no_new_handle();
}

#[test]
fn arm_with_an_absent_device_id_is_no_device() {
    let rig = rig();
    let error = rig
        .takes
        .arm(&rig.app_data(), Some("fake:gone".into()), None)
        .unwrap_err();
    assert_eq!(error, record(RecordError::NoDevice));
    assert_eq!(error.to_string(), "no input device");
    assert!(!rig.prefs().exists());
}

#[test]
fn a_busy_device_stays_idle_and_reports_the_device_error() {
    let rig = rig();
    *rig.arm_failure.lock().unwrap() = Some(RecordError::DeviceBusy);
    let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
    // The kind (device lost on macOS, busy elsewhere) is the app's error table.
    assert_eq!(error, record(RecordError::DeviceBusy));
    assert_eq!(error.to_string(), "the input device is busy");
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Idle);
    assert!(rig.sink.states().is_empty());
}

#[test]
fn a_channel_out_of_range_is_other() {
    let rig = rig();
    let error = rig
        .takes
        .arm(&rig.app_data(), Some("fake:usb".into()), Some(4))
        .unwrap_err();
    assert_eq!(
        error,
        TakeError::ChannelOutOfRange {
            channel: 4,
            channels: 4
        }
    );
    rig.no_new_handle();
}

#[test]
fn arm_reports_the_armed_input_and_rearm_replaces_it() {
    let rig = rig();
    let status = rig.takes.arm(&rig.app_data(), None, None).unwrap();
    assert_eq!(status.device, Some(FakePlatform::mic()));
    assert_eq!(status.channel, 0);
    let first = rig.next_handle();
    let status = rig
        .takes
        .arm(&rig.app_data(), Some("fake:usb".into()), Some(2))
        .unwrap();
    assert_eq!(status.device, Some(FakePlatform::usb()));
    assert_eq!(status.channel, 2);
    let second = rig.next_handle();
    assert!(!first.is_open());
    assert!(second.is_open());
}

#[test]
fn arm_and_disarm_during_a_take_are_busy() {
    let rig = rig();
    let handle = rig.take();
    assert_eq!(
        rig.takes.arm(&rig.app_data(), None, None).unwrap_err(),
        TakeError::Busy
    );
    assert_eq!(
        rig.takes.disarm(&rig.app_data()).unwrap_err(),
        TakeError::Busy
    );
    assert!(handle.is_recording());
}

#[test]
fn disarm_closes_the_stream_and_emits_idle() {
    let rig = rig();
    let handle = rig.arm();
    let status = rig.takes.disarm(&rig.app_data()).unwrap();
    assert_eq!(status.state, RecState::Idle);
    assert_eq!(status.device, None);
    assert!(!handle.is_open());
    assert_eq!(rig.sink.last(), Some(TakeEvent::new(RecState::Idle)));
    // Disarming again is harmless.
    assert_eq!(
        rig.takes.disarm(&rig.app_data()).unwrap().state,
        RecState::Idle
    );
}

// --- the saved input ---

#[test]
fn a_saved_exclusive_input_keeps_its_identity_and_armed_channel_count() {
    let rig = rig();
    let mut busy = FakePlatform::usb();
    busy.channels = 0;
    busy.sample_rate = 0;
    *rig.platform.devices.lock().unwrap() = vec![FakePlatform::mic(), busy];
    RecordingInput {
        device_id: Some(FakePlatform::usb().id),
        channel: 2,
    }
    .save(&rig.app_data())
    .unwrap();

    // Enumeration could not open the USB endpoint. The stream-open error is authoritative.
    *rig.arm_failure.lock().unwrap() = Some(RecordError::PermissionDenied);
    let error = rig.takes.arm(&rig.app_data(), None, None).unwrap_err();
    assert_eq!(error, TakeError::PermissionDenied);
    let st = rig.takes.arm(&rig.app_data(), None, None).unwrap();
    assert_eq!(st.device, Some(FakePlatform::usb()));
    assert_eq!(st.channel, 2);
    rig.next_handle();

    // With the exclusive stream open the picker still exposes the active channels.
    let list = rig.takes.devices(&rig.app_data()).unwrap();
    assert!(list.devices.contains(&FakePlatform::usb()));
    let st = rig
        .takes
        .arm(&rig.app_data(), Some(FakePlatform::usb().id), Some(3))
        .unwrap();
    assert_eq!(st.channel, 3);
}

#[test]
fn arm_writes_prefs_only_when_it_carried_an_argument() {
    let rig = rig();
    RecordingInput {
        device_id: Some("fake:gone".into()),
        channel: 2,
    }
    .save(&rig.app_data())
    .unwrap();
    let before = std::fs::read_to_string(rig.prefs()).unwrap();
    // The saved device is absent: the default input, and the file is not rewritten.
    let status = rig.takes.arm(&rig.app_data(), None, None).unwrap();
    assert_eq!(status.device.unwrap().id, "fake:mic");
    assert_eq!(status.channel, 0);
    assert_eq!(std::fs::read_to_string(rig.prefs()).unwrap(), before);
    rig.takes.arm(&rig.app_data(), None, Some(0)).unwrap();
    assert_eq!(
        RecordingInput::load(&rig.app_data()).value,
        RecordingInput {
            device_id: Some("fake:mic".into()),
            channel: 0
        }
    );
    rig.takes
        .arm(&rig.app_data(), Some("fake:usb".into()), None)
        .unwrap();
    assert_eq!(
        RecordingInput::load(&rig.app_data()).value,
        RecordingInput {
            device_id: Some("fake:usb".into()),
            channel: 0
        }
    );
}

#[test]
fn a_prefs_write_failure_does_not_fail_the_arm() {
    let rig = rig();
    std::fs::create_dir_all(rig.prefs()).unwrap();
    let status = rig
        .takes
        .arm(&rig.app_data(), Some("fake:usb".into()), Some(1))
        .unwrap();
    assert_eq!(status.state, RecState::Armed);
    assert_eq!(status.channel, 1);
}

#[test]
fn devices_and_idle_status_use_the_saved_choice() {
    let rig = rig();
    let result = rig.takes.devices(&rig.app_data()).unwrap();
    assert_eq!(
        result.devices,
        vec![FakePlatform::mic(), FakePlatform::usb()]
    );
    assert_eq!(
        result.selected,
        Some(Selection {
            device_id: "fake:mic".into(),
            channel: 0
        })
    );
    RecordingInput {
        device_id: Some("fake:usb".into()),
        channel: 2,
    }
    .save(&rig.app_data())
    .unwrap();
    let result = rig.takes.devices(&rig.app_data()).unwrap();
    assert_eq!(
        result.selected,
        Some(Selection {
            device_id: "fake:usb".into(),
            channel: 2
        })
    );
    let idle = rig.takes.status(&rig.app_data());
    assert_eq!(
        idle,
        TakeStatus {
            state: RecState::Idle,
            permission: Permission::Authorized,
            device: None,
            channel: 2,
            stem: None,
            elapsed_seconds: None,
        }
    );
    rig.platform.devices.lock().unwrap().clear();
    let none = rig.takes.devices(&rig.app_data()).unwrap();
    assert!(none.devices.is_empty());
    assert_eq!(none.selected, None);
}

// --- levels (polling) ---

#[test]
fn levels_are_forwarded_while_armed() {
    let rig = rig();
    let handle = rig.arm();
    let deadline = Instant::now() + WAIT;
    while rig.sink.levels() == 0 {
        assert!(Instant::now() < deadline, "no level event");
        handle.push(&[0.5; 10], T0);
        std::thread::sleep(Duration::from_millis(5));
    }
}

// --- start ---

#[test]
fn start_names_the_take_and_reports_recording() {
    let rig = rig();
    let handle = rig.arm();
    assert_eq!(rig.start(&handle, Some("main")).unwrap(), "main-01");
    assert_eq!(
        rig.sink.last(),
        Some(TakeEvent::new(RecState::Recording).stem("main-01"))
    );
    let status = rig.takes.status(&rig.app_data());
    assert_eq!(status.state, RecState::Recording);
    assert_eq!(status.stem.as_deref(), Some("main-01"));
    assert!(status.elapsed_seconds.is_some_and(|s| s >= 0.0));
    assert_eq!(rig.files(), vec![".main-01.wav.partial".to_string()]);
    rig.takes.stop(None).unwrap();
    assert_eq!(rig.start(&handle, Some("main")).unwrap(), "main-02");
    rig.takes.stop(None).unwrap();
    assert_eq!(rig.start(&handle, None).unwrap(), "memo-01");
    rig.takes.stop(None).unwrap();
    // Not a valid stem: the generic name.
    assert_eq!(rig.start(&handle, Some("a/b")).unwrap(), "memo-02");
}

#[test]
fn start_while_recording_is_busy() {
    let rig = rig();
    let handle = rig.take();
    assert_eq!(rig.start(&handle, None).unwrap_err(), TakeError::Busy);
}

#[test]
fn start_unarmed_is_other() {
    let rig = rig();
    let error = rig
        .takes
        .start(rig.resolve(), None, start_fields())
        .unwrap_err();
    assert_eq!(error, TakeError::NotArmed);
    assert!(rig.files().is_empty());
}

#[test]
fn start_with_a_stale_root_touches_nothing() {
    /// The resolver's own error, which `start` returns as it is.
    #[derive(Debug, PartialEq)]
    enum ResolveError {
        Take(TakeError),
        Stale,
    }
    impl From<TakeError> for ResolveError {
        fn from(e: TakeError) -> ResolveError {
            ResolveError::Take(e)
        }
    }
    let rig = rig();
    let handle = rig.arm();
    let error = rig
        .takes
        .start(|| Err(ResolveError::Stale), None, start_fields())
        .unwrap_err();
    assert_eq!(error, ResolveError::Stale);
    assert!(rig.files().is_empty());
    assert!(!handle.is_recording());
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Armed);
}

#[test]
fn a_start_without_audio_times_out_and_leaves_no_partial() {
    let rig = rig_with(Limits {
        max_frames: None,
        start_timeout: Duration::from_millis(100),
    });
    rig.arm();
    let error = rig
        .takes
        .start(rig.resolve(), None, start_fields())
        .unwrap_err();
    assert_eq!(error, TakeError::NoAudio);
    assert!(rig.files().is_empty());
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Armed);
    assert_eq!(rig.sink.last(), Some(TakeEvent::new(RecState::Armed)));
}

#[test]
fn an_invalid_start_tempo_still_saves_the_memo() {
    let rig = rig();
    let handle = rig.arm();
    let mut fields = start_fields();
    // `setcps(0)`.
    fields.tempo.cps = 0.0;
    rig.start_with(&handle, Some("main"), fields).unwrap();
    rig.push(&handle, 10, 1_000);
    let entry = rig.takes.stop(None).unwrap();
    let sidecar = entry.sidecar.unwrap();
    assert_eq!(sidecar.tempo, SidecarTempo::placeholder());
    assert_eq!(sidecar.backing, start_fields().backing);
    assert_eq!(sidecar.timing.bar1_frame, None);

    // Invalid webview fields fall back to the same sanitized sidecar.
    let mut fields = start_fields();
    fields.tempo.cps = 0.0;
    rig.start_with(&handle, Some("main"), fields).unwrap();
    rig.push(&handle, 10, 1_000);
    let mut stop_with = stop_fields(Some("1500000000"));
    stop_with.tempo.cps = 0.0;
    let entry = rig.takes.stop(Some(stop_with)).unwrap();
    assert_eq!(entry.stem, "main-02");
    assert_eq!(entry.sidecar.unwrap().tempo, SidecarTempo::placeholder());
}

// --- stop ---

#[test]
fn stop_writes_the_webview_fields_input_and_anchors() {
    let rig = rig();
    rig.take();
    let entry = rig
        .takes
        .stop(Some(stop_fields(Some("1500000000"))))
        .unwrap();
    assert_eq!(entry.stem, "main-01");
    assert_eq!(entry.frames, Some(1_000));
    assert_eq!(entry.sample_rate, Some(RATE));
    assert!(entry.has_sidecar);
    assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    let sidecar = sidecar_of(&rig, "main-01");
    assert_eq!(entry.sidecar.as_ref(), Some(&sidecar));
    let fields = stop_fields(Some("1500000000"));
    assert_eq!(sidecar.schema, 1);
    assert_eq!(sidecar.app.version, "test");
    assert_eq!(sidecar.created_at.len(), "2026-01-01T00:00:00Z".len());
    assert_eq!(
        sidecar.input,
        SidecarInput {
            device_id: "fake:mic".into(),
            device_name: "fake mic".into(),
            channel: 0,
            channels: 1,
            sample_rate: RATE,
        }
    );
    assert_eq!(sidecar.output, fields.output);
    assert_eq!(sidecar.tempo, fields.tempo);
    assert_eq!(sidecar.backing, fields.backing);
    assert_eq!(sidecar.timing.bar1_host_ns, Some(HostNs(1_500_000_000)));
    // The first anchor is frame 0 at T0; 500 ms later at 1 kHz is frame 500.
    assert_eq!(sidecar.timing.bar1_frame, Some(500.0));
    assert_eq!(sidecar.timing.anchors.first(), Some(&(0, HostNs(T0))));
    assert_eq!(sidecar.timing.clock_offset_ns, fields.clock_offset_ns);
    assert_eq!(sidecar.timing.output_stamps, fields.output_stamps);
    assert_eq!(sidecar.xruns, 0);
    assert!(!sidecar.recovered);
    assert_eq!(
        rig.sink.last(),
        Some(TakeEvent::new(RecState::Armed).stem("main-01"))
    );
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Armed);
}

#[test]
fn stop_without_fields_writes_the_fallback_sidecar() {
    let rig = rig();
    rig.take();
    rig.takes.stop(None).unwrap();
    let sidecar = sidecar_of(&rig, "main-01");
    let start = start_fields();
    assert_eq!(
        sidecar.output,
        SidecarOutput {
            base_latency: 0.0,
            output_latency: None
        }
    );
    assert_eq!(sidecar.tempo, start.tempo);
    assert_eq!(sidecar.backing, start.backing);
    assert_eq!(sidecar.timing.bar1_frame, None);
    assert_eq!(sidecar.timing.bar1_host_ns, None);
    assert_eq!(sidecar.timing.clock_offset_ns.start, None);
    assert_eq!(sidecar.timing.clock_offset_ns.end, None);
    assert!(sidecar.timing.output_stamps.is_empty());
    assert!(!sidecar.timing.anchors.is_empty());
}

#[test]
fn invalid_webview_fields_keep_the_audio_with_the_fallback() {
    let rig = rig();
    rig.take();
    let mut fields = stop_fields(Some("1500000000"));
    fields.tempo.meter = "four".into();
    let entry = rig.takes.stop(Some(fields)).unwrap();
    let sidecar = entry.sidecar.unwrap();
    assert_eq!(sidecar.tempo, start_fields().tempo);
    assert_eq!(sidecar.timing.bar1_host_ns, None);
}

#[test]
fn a_bar1_outside_the_take_is_stored_null() {
    let rig = rig();
    rig.take();
    // 0.5 s before the first frame.
    let entry = rig
        .takes
        .stop(Some(stop_fields(Some("500000000"))))
        .unwrap();
    let timing = entry.sidecar.unwrap().timing;
    assert_eq!(timing.bar1_host_ns, Some(HostNs(500_000_000)));
    assert_eq!(timing.bar1_frame, None);
}

#[test]
fn a_second_stop_is_other() {
    let rig = rig();
    rig.take();
    rig.takes.stop(None).unwrap();
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
    let fresh = self::rig();
    assert_eq!(fresh.takes.stop(None).unwrap_err(), TakeError::NoTake);
}

#[test]
fn stop_finalizes_into_the_project_captured_at_start() {
    let rig = rig();
    rig.take();
    let first = rig.project();
    let beta = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
    let entry = rig.takes.stop(None).unwrap();
    assert_eq!(entry.stem, "main-01");
    assert!(first.recording_wav_path("main-01").is_file());
    assert!(beta.recordings().unwrap().is_empty());
}

// --- publish ---

#[test]
fn a_failed_publish_keeps_the_take_and_a_retry_publishes_it() {
    let rig = rig();
    rig.take();
    let conflict = block_publication(&rig);
    let error = rig
        .takes
        .stop(Some(stop_fields(Some("1500000000"))))
        .unwrap_err();
    assert_eq!(rig.files(), vec![".main-01.wav.partial", "main-01.json"]);
    assert_eq!(std::fs::read(&conflict).unwrap(), b"existing sidecar");
    let event = rig.sink.last().unwrap();
    assert_eq!(
        (event.state, event.stem.as_deref(), event.error.as_ref()),
        (RecState::Armed, Some("main-01"), Some(&error))
    );
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Armed);
    // The conflict still exists, and the take remains available for retry.
    assert!(rig.takes.stop(None).is_err());
    std::fs::remove_file(conflict).unwrap();
    let entry = rig
        .takes
        .stop(Some(stop_fields(Some("1500000000"))))
        .unwrap();
    assert_eq!(entry.stem, "main-01");
    assert_eq!(entry.frames, Some(1_000));
    assert_eq!(entry.sidecar.unwrap().timing.bar1_frame, Some(500.0));
    assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    assert_eq!(
        rig.sink.last(),
        Some(TakeEvent::new(RecState::Armed).stem("main-01"))
    );
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
}

#[test]
fn a_take_that_failed_to_publish_is_published_by_disarm_reload_and_quit() {
    type Settle = fn(&Rig);
    let settles: [(Settle, &str); 3] = [
        (|r| drop(r.takes.disarm(&r.app_data()).unwrap()), "disarm"),
        (|r| r.takes.on_reload(), "reload"),
        (|r| r.takes.shutdown(), "quit"),
    ];
    for (settle, name) in settles {
        let rig = rig();
        rig.take();
        let conflict = block_publication(&rig);
        assert!(rig.takes.stop(None).is_err(), "{name}");
        std::fs::remove_file(conflict).unwrap();
        settle(&rig);
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"], "{name}");
        assert_eq!(sidecar_of(&rig, "main-01").timing.bar1_host_ns, None);
    }
}

// --- endings Rust handles by itself ---

#[test]
fn the_length_cap_finalizes_and_a_later_stop_is_idempotent() {
    let rig = rig_with(Limits {
        max_frames: Some(100),
        start_timeout: Duration::from_secs(2),
    });
    let handle = rig.arm();
    rig.start(&handle, Some("main")).unwrap();
    assert!(handle.push(&[0.5; 200], T0 + 10_000_000));
    let event = rig.sink.wait(|e| e.error.is_some());
    assert_eq!(
        event,
        TakeEvent::new(RecState::Armed)
            .stem("main-01")
            .error(TakeError::LimitReached)
    );
    let entry = rig.project().recordings().unwrap().remove(0);
    assert_eq!(entry.frames, Some(100));
    assert_eq!(entry.sidecar.unwrap().timing.bar1_host_ns, None);
    let status = rig.takes.status(&rig.app_data());
    assert_eq!(status.state, RecState::Armed);
    assert!(handle.is_open());
    // The webview's stop replaces the fallback sidecar.
    let entry = rig
        .takes
        .stop(Some(stop_fields(Some("1050000000"))))
        .unwrap();
    let timing = entry.sidecar.unwrap().timing;
    assert_eq!(timing.bar1_host_ns, Some(HostNs(1_050_000_000)));
    assert_eq!(timing.bar1_frame, Some(50.0));
    assert_eq!(
        rig.sink.last(),
        Some(TakeEvent::new(RecState::Armed).stem("main-01"))
    );
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
}

#[test]
fn a_device_loss_finalizes_and_emits_idle() {
    let rig = rig();
    let handle = rig.take();
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    let event = rig.sink.wait(|e| e.error.is_some());
    assert_eq!(event.state, RecState::Idle);
    assert_eq!(event.stem.as_deref(), Some("main-01"));
    assert_eq!(
        event.error.unwrap(),
        record(RecordError::DeviceLost("unplugged".into()))
    );
    assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    let fallback = sidecar_of(&rig, "main-01");
    assert_eq!(fallback.timing.bar1_frame, None);
    assert_eq!(fallback.timing.bar1_host_ns, None);
    let status = rig.takes.status(&rig.app_data());
    assert_eq!(status.state, RecState::Idle);
    assert_eq!(status.device, None);
    let entry = rig
        .takes
        .stop(Some(stop_fields(Some("1250000000"))))
        .unwrap();
    assert_eq!(entry.sidecar.unwrap().timing.bar1_frame, Some(250.0));
    assert_eq!(
        rig.sink.last(),
        Some(TakeEvent::new(RecState::Idle).stem("main-01"))
    );
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
    // The recorder arms again.
    rig.arm();
}

#[test]
fn a_device_loss_while_armed_emits_idle_without_a_stem() {
    let rig = rig();
    let handle = rig.arm();
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    let event = rig.sink.wait(|e| e.error.is_some());
    assert_eq!(event.state, RecState::Idle);
    assert_eq!(event.stem, None);
    assert!(matches!(
        event.error,
        Some(TakeError::Record(ref e)) if matches!(**e, RecordError::DeviceLost(_))
    ));
}

#[test]
fn a_stale_failed_after_a_rearm_is_ignored() {
    let rig = rig();
    rig.arm();
    let current = armed_id(&rig.takes).unwrap();
    let events = rig.sink.states().len();
    // The `Failed` of an earlier stream, processed after this arm.
    rig.takes.on_event(Event::Failed {
        arm_id: current - 1,
        error: RecordError::DeviceLost("old".into()),
    });
    assert_eq!(rig.sink.states().len(), events);
    let status = rig.takes.status(&rig.app_data());
    assert_eq!(status.state, RecState::Armed);
    assert!(status.device.is_some());
    // This stream's own `Failed` disarms.
    rig.takes.on_event(Event::Failed {
        arm_id: current,
        error: RecordError::DeviceLost("now".into()),
    });
    assert_eq!(rig.sink.last().unwrap().state, RecState::Idle);
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Idle);
}

#[test]
fn a_rearm_after_a_device_loss_stays_armed() {
    let rig = rig();
    let handle = rig.arm();
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    rig.sink.wait(|e| e.error.is_some());
    rig.arm();
    assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Armed);
}

#[test]
fn arm_forgets_an_ended_take() {
    let rig = rig();
    let handle = rig.take();
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    rig.sink.wait(|e| e.error.is_some());
    rig.arm();
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
}

// --- reload and quit ---

#[test]
fn a_reload_finalizes_the_take_and_disarms_silently() {
    let rig = rig();
    let handle = rig.take();
    let events = rig.sink.states().len();
    rig.takes.on_reload();
    assert!(!handle.is_open());
    assert_eq!(rig.sink.states().len(), events, "no event on reload");
    let entry = rig.project().recordings().unwrap().remove(0);
    assert_eq!(entry.stem, "main-01");
    assert_eq!(entry.frames, Some(1_000));
    let timing = entry.sidecar.unwrap().timing;
    assert_eq!(timing.bar1_frame, None);
    assert_eq!(timing.bar1_host_ns, None);
    let status = rig.takes.status(&rig.app_data());
    assert_eq!(status.state, RecState::Idle);
    assert_eq!(rig.takes.stop(None).unwrap_err(), TakeError::NoTake);
    // The recorder arms again afterwards.
    rig.arm();
}

#[test]
fn a_reload_while_idle_is_a_no_op() {
    let rig = rig();
    rig.takes.on_reload();
    assert!(rig.takes.lock().recorder.is_none());
    assert!(rig.sink.states().is_empty());
    let handle = rig.arm();
    let events = rig.sink.states().len();
    rig.takes.on_reload();
    assert!(!handle.is_open(), "an armed stream is closed");
    assert_eq!(rig.sink.states().len(), events);
}

#[test]
fn quit_finalizes_the_take_and_joins() {
    let rig = rig();
    let handle = rig.take();
    let events = rig.sink.states().len();
    rig.takes.shutdown();
    assert!(!handle.is_open());
    assert_eq!(rig.sink.states().len(), events, "no event on quit");
    {
        let inner = rig.takes.lock();
        assert!(inner.recorder.is_none());
        assert!(inner.forwarder.is_none());
    }
    assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    let sidecar = sidecar_of(&rig, "main-01");
    assert_eq!(sidecar.timing.bar1_frame, None);
    assert_eq!(sidecar.timing.clock_offset_ns.start, None);
    // Quitting twice, or idle, is harmless.
    rig.takes.shutdown();
    self::rig().takes.shutdown();
}

// --- repair ---

/// An empty partial last modified a minute ago (the repair discards it).
fn stale_partial(project: &Project, stem: &str) -> PathBuf {
    let path = project
        .root()
        .join(format!("recordings/.{stem}.wav.partial"));
    let file = std::fs::File::create(&path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(60))
        .unwrap();
    path
}

#[test]
fn repair_skips_a_project_with_a_take_in_progress_or_ended() {
    let rig = rig();
    let handle = rig.take();
    let project = rig.project();
    let old = stale_partial(&project, "memo-09");
    assert_eq!(repair_on_open(&rig.takes, &project), RepairOutcome::Skipped);
    assert!(old.exists());
    assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
    rig.sink.wait(|e| e.error.is_some());
    assert_eq!(repair_on_open(&rig.takes, &project), RepairOutcome::Skipped);
    assert!(old.exists());
    rig.takes.stop(None).unwrap();
    let RepairOutcome::Repaired(report) = repair_on_open(&rig.takes, &project) else {
        panic!("not repaired");
    };
    assert_eq!(report.discarded, vec![".memo-09.wav.partial".to_string()]);
    assert!(!old.exists());
}

#[test]
fn repair_runs_in_another_project_during_a_take() {
    let rig = rig();
    rig.take();
    let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
    let old = stale_partial(&other, "memo-01");
    let RepairOutcome::Repaired(report) = repair_on_open(&rig.takes, &other) else {
        panic!("not repaired");
    };
    assert_eq!(report.discarded, vec![".memo-01.wav.partial".to_string()]);
    assert!(!old.exists());
    assert!(rig.takes.lock().state.is_recording(), "the take runs on");
}

#[test]
fn a_repair_error_is_reported_not_raised() {
    let rig = rig();
    let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
    let recordings = other.root().join("recordings");
    std::fs::remove_dir_all(&recordings).unwrap();
    std::fs::write(&recordings, "not a folder").unwrap();
    assert!(matches!(
        repair_on_open(&rig.takes, &other),
        RepairOutcome::Failed(_)
    ));
}

// --- leave ---

#[test]
fn leaving_an_unpublished_ended_take_publishes_it() {
    let rig = rig();
    rig.take();
    let conflict = block_publication(&rig);
    assert!(rig.takes.stop(None).is_err());
    let mut inner = rig.takes.lock();
    assert!(matches!(inner.state, TakeState::Ended { .. }));
    std::fs::remove_file(conflict).unwrap();
    leave(
        &mut inner,
        TakeState::Idle {
            stream_denied: false,
        },
    );
    assert!(matches!(
        inner.state,
        TakeState::Idle {
            stream_denied: false
        }
    ));
    drop(inner);
    assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    assert_eq!(sidecar_of(&rig, "main-01").timing.bar1_host_ns, None);
}

#[test]
fn leaving_a_published_ended_take_writes_nothing() {
    let rig = rig_with(Limits {
        max_frames: Some(100),
        start_timeout: Duration::from_secs(2),
    });
    let handle = rig.arm();
    rig.start(&handle, Some("main")).unwrap();
    assert!(handle.push(&[0.5; 200], T0 + 10_000_000));
    rig.sink.wait(|e| e.error.is_some());
    let sidecar = rig.recordings_dir().join("main-01.json");
    std::fs::write(&sidecar, b"marker").unwrap();
    let mut inner = rig.takes.lock();
    assert!(matches!(inner.state, TakeState::Ended { .. }));
    settle_ended(&mut inner);
    assert!(matches!(inner.state, TakeState::Armed(_)));
    drop(inner);
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"marker");
}

#[test]
fn leaving_a_state_that_is_not_ended_publishes_nothing() {
    let rig = rig();
    rig.arm();
    let mut inner = rig.takes.lock();
    assert!(matches!(inner.state, TakeState::Armed(_)));
    leave(
        &mut inner,
        TakeState::Idle {
            stream_denied: false,
        },
    );
    drop(inner);
    assert!(rig.files().is_empty());
}

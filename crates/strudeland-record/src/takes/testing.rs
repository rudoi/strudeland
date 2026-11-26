//! The fakes for the take lifecycle's tests and the app's: a platform, a collecting sink and a
//! recorder maker over [`FakeSource`]. Reached only through [`crate::testing`].

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Permission, Platform, RecorderMaker, Sink, TakeEvent};
use crate::recorder::Limits;
use crate::source::{FakeHandle, FakeSource};
use crate::{ArmConfig, ArmedInfo, DeviceInfo, Level, RecordError, Recorder, Source};

/// The sample rate of every fake device.
const RATE: u32 = 1_000;
/// How long [`CollectingSink::wait`] waits.
const WAIT: Duration = Duration::from_secs(5);

/// The OS as a test sets it: the permission, the answer to a request, the device list.
pub struct FakePlatform {
    pub has_prompt: AtomicBool,
    pub permission: Mutex<Permission>,
    /// The status after a request.
    pub answer: Mutex<Permission>,
    pub requests: AtomicUsize,
    /// When set, a request waits for a message on it.
    pub gate: Mutex<Option<mpsc::Receiver<()>>>,
    pub devices: Mutex<Vec<DeviceInfo>>,
}

impl FakePlatform {
    /// With a prompt, authorized, listing [`FakePlatform::mic`] and [`FakePlatform::usb`].
    pub fn new() -> FakePlatform {
        FakePlatform {
            has_prompt: AtomicBool::new(true),
            permission: Mutex::new(Permission::Authorized),
            answer: Mutex::new(Permission::Authorized),
            requests: AtomicUsize::new(0),
            gate: Mutex::new(None),
            devices: Mutex::new(vec![FakePlatform::mic(), FakePlatform::usb()]),
        }
    }

    /// A device at the fakes' 1 kHz rate.
    pub fn device(id: &str, name: &str, channels: u16, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.to_string(),
            channels,
            sample_rate: RATE,
            bluetooth: false,
            is_default,
        }
    }

    /// The default input: one channel.
    pub fn mic() -> DeviceInfo {
        FakePlatform::device("fake:mic", "fake mic", 1, true)
    }

    /// Four channels, not the default.
    pub fn usb() -> DeviceInfo {
        FakePlatform::device("fake:usb", "usb interface", 4, false)
    }
}

impl Default for FakePlatform {
    fn default() -> FakePlatform {
        FakePlatform::new()
    }
}

impl Platform for Arc<FakePlatform> {
    fn has_permission_prompt(&self) -> bool {
        self.has_prompt.load(Ordering::Acquire)
    }
    fn permission(&self) -> Permission {
        *self.permission.lock().unwrap()
    }

    fn request_permission(&self) {
        self.requests.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.gate.lock().unwrap().take() {
            gate.recv().unwrap();
        }
        *self.permission.lock().unwrap() = *self.answer.lock().unwrap();
    }

    fn devices(&self) -> Result<Vec<DeviceInfo>, RecordError> {
        Ok(self.devices.lock().unwrap().clone())
    }
}

/// Keeps every state event and counts the levels.
#[derive(Default)]
pub struct CollectingSink {
    states: Mutex<Vec<TakeEvent>>,
    levels: AtomicUsize,
}

impl Sink for CollectingSink {
    fn state(&self, event: TakeEvent) {
        self.states.lock().unwrap().push(event);
    }

    fn level(&self, _: Level) {
        self.levels.fetch_add(1, Ordering::SeqCst);
    }
}

impl CollectingSink {
    pub fn states(&self) -> Vec<TakeEvent> {
        self.states.lock().unwrap().clone()
    }

    pub fn last(&self) -> Option<TakeEvent> {
        self.states.lock().unwrap().last().cloned()
    }

    /// Waits (up to 5 s) for a state event matching `pred` and returns it.
    pub fn wait(&self, pred: impl Fn(&TakeEvent) -> bool) -> TakeEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(e) = self.states().into_iter().find(|e| pred(e)) {
                return e;
            }
            assert!(Instant::now() < deadline, "no matching state event");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The number of level events so far.
    pub fn levels(&self) -> usize {
        self.levels.load(Ordering::SeqCst)
    }
}

/// A recorder maker over [`FakeSource`] with these limits: each arm opens a fake stream on
/// [`FakePlatform::mic`] or [`FakePlatform::usb`] (else `NoDevice`) and sends its handle on the
/// receiver. The next arm fails with the error in `arm_failure` when one is set.
pub fn fake_maker(
    limits: Limits,
    arm_failure: Arc<Mutex<Option<RecordError>>>,
) -> (RecorderMaker, Receiver<FakeHandle>) {
    let (handle_tx, handles) = mpsc::channel();
    let maker: RecorderMaker = Box::new(move || {
        let tx = handle_tx.clone();
        let failure = Arc::clone(&arm_failure);
        let factory = Box::new(move |cfg: &ArmConfig| {
            if let Some(e) = failure.lock().unwrap().take() {
                return Err(e);
            }
            let known = [FakePlatform::mic(), FakePlatform::usb()];
            let id = cfg.device_id.as_deref().unwrap_or("fake:mic");
            let d = known
                .iter()
                .find(|d| d.id == id)
                .ok_or(RecordError::NoDevice)?;
            let (source, handle) = FakeSource::new(ArmedInfo {
                device_id: d.id.clone(),
                device_name: d.name.clone(),
                channel: cfg.channel,
                channels: d.channels,
                sample_rate: d.sample_rate,
                bluetooth: d.bluetooth,
            });
            tx.send(handle).unwrap();
            Ok(Box::new(source) as Box<dyn Source>)
        });
        Recorder::with_limits(factory, limits.clone())
    });
    (maker, handles)
}

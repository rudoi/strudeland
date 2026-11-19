//! Input devices, the cpal source factory and the host clock.

use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait};

use crate::error::RecordError;
use crate::recorder::ArmConfig;
use crate::source::{CpalSource, DEFAULT_CAPTURE_TIME, Source};

/// One input device, with its default input config's channel count and rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `DeviceId::to_string()`, `coreaudio:<uid>` on macOS.
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub bluetooth: bool,
    pub is_default: bool,
}

/// The input devices, default first, then by name. Devices whose id, description or default
/// input config cannot be read are skipped. Opens no stream.
pub fn list_input_devices() -> Result<Vec<DeviceInfo>, RecordError> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|e| RecordError::from_cpal(&e))?;
    let mut out: Vec<DeviceInfo> = devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let description = device.description().ok()?;
            let config = device.default_input_config().ok()?;
            Some(DeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name: description.name().to_string(),
                channels: config.channels(),
                sample_rate: config.sample_rate(),
                bluetooth: description.interface_type() == cpal::InterfaceType::Bluetooth,
            })
        })
        .collect();
    sort_devices(&mut out);
    Ok(out)
}

fn sort_devices(devices: &mut [DeviceInfo]) {
    devices.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// The device for `device_id` (`None`: the default input). An id that does not parse or is
/// not present is `NoDevice`.
pub fn find_input_device(device_id: Option<&str>) -> Result<cpal::Device, RecordError> {
    let id = device_id
        .map(|id| cpal::DeviceId::from_str(id).map_err(|_| RecordError::NoDevice))
        .transpose()?;
    let host = cpal::default_host();
    match id {
        None => host.default_input_device().ok_or(RecordError::NoDevice),
        Some(id) => host.device_by_id(&id).ok_or(RecordError::NoDevice),
    }
}

/// The source factory `Recorder::new` uses.
pub(crate) fn cpal_source(config: &ArmConfig) -> Result<Box<dyn Source>, RecordError> {
    let device = find_input_device(config.device_id.as_deref())?;
    Ok(Box::new(CpalSource::new(
        device,
        config.channel,
        DEFAULT_CAPTURE_TIME,
    )?))
}

/// The mach host time in nanoseconds, the clock of cpal's CoreAudio timestamps and of WebKit's
/// `getOutputTimestamp()`.
#[cfg(target_vendor = "apple")]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = mach2::mach_time::mach_timebase_info::default();
        // SAFETY: `info` is a valid, writable `mach_timebase_info` for the call's duration.
        let status = unsafe { mach2::mach_time::mach_timebase_info(&mut info) };
        if status == 0 && info.denom != 0 {
            (info.numer, info.denom)
        } else {
            (1, 1)
        }
    });
    // SAFETY: `mach_absolute_time` takes no arguments and has no preconditions.
    let ticks = unsafe { mach2::mach_time::mach_absolute_time() };
    (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
}

/// Nanoseconds since a process-wide origin (no mach clock off Apple platforms).
#[cfg(not(target_vendor = "apple"))]
pub fn host_now_ns() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            name: name.into(),
            channels: 1,
            sample_rate: 48_000,
            bluetooth: false,
            is_default,
        }
    }

    #[test]
    fn devices_sort_default_first_then_by_name() {
        let mut devices = vec![
            dev("c:2", "zed", false),
            dev("c:1", "alpha", false),
            dev("c:3", "mic", true),
            dev("c:0", "alpha", false),
        ];
        sort_devices(&mut devices);
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["c:3", "c:0", "c:1", "c:2"]);
    }

    #[test]
    fn host_now_is_monotonic_and_nanoseconds() {
        let a = host_now_ns();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = host_now_ns();
        assert!(b > a, "{a} then {b}");
        // 20 ms of sleep is at least 20e6 ns and, on any loaded machine, well under 2 s.
        assert!(b - a >= 20_000_000, "{}", b - a);
        assert!(b - a < 2_000_000_000, "{}", b - a);
    }

    #[test]
    fn an_unparseable_device_id_is_no_device() {
        assert!(matches!(
            find_input_device(Some("not a device id")),
            Err(RecordError::NoDevice)
        ));
    }

    #[test]
    #[ignore = "needs an input device"]
    fn devices_list_ids_that_parse_back() {
        let devices = list_input_devices().unwrap();
        assert!(!devices.is_empty());
        for d in &devices {
            assert!(cpal::DeviceId::from_str(&d.id).is_ok(), "{}", d.id);
        }
    }
}

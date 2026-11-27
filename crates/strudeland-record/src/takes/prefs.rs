//! The input selection over the saved [`RecordingInput`].

use strudeland_project::RecordingInput;

use crate::DeviceInfo;

/// The saved device when present, else the default input (else the first); the saved channel
/// when the device has it, else 0. `None` without devices.
pub(super) fn resolve_selection<'a>(
    devices: &'a [DeviceInfo],
    input: &RecordingInput,
) -> Option<(&'a DeviceInfo, u16)> {
    let saved = input
        .device_id
        .as_deref()
        .and_then(|id| devices.iter().find(|d| d.id == id));
    let device = saved
        .or_else(|| devices.iter().find(|d| d.is_default))
        .or_else(|| devices.first())?;
    Some((device, saved_channel(device, input)))
}

/// The saved channel when `device` has it (or reports no channels: an exclusive endpoint whose
/// config query was refused), else 0.
pub(super) fn saved_channel(device: &DeviceInfo, input: &RecordingInput) -> u16 {
    if device.channels == 0 || input.channel < device.channels {
        input.channel
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takes::testing::FakePlatform;

    fn device(id: &str, name: &str, channels: u16, is_default: bool) -> DeviceInfo {
        FakePlatform::device(id, name, channels, is_default)
    }

    #[test]
    fn selection_resolution() {
        let devices = [FakePlatform::mic(), FakePlatform::usb()];
        let saved = |id: Option<&str>, channel| RecordingInput {
            device_id: id.map(str::to_string),
            channel,
        };
        let pick = |input: &RecordingInput| {
            resolve_selection(&devices, input).map(|(d, c)| (d.id.clone(), c))
        };
        assert_eq!(
            pick(&RecordingInput::default()),
            Some(("fake:mic".into(), 0))
        );
        assert_eq!(
            pick(&saved(Some("fake:usb"), 3)),
            Some(("fake:usb".into(), 3))
        );
        // Channel clamp.
        assert_eq!(
            pick(&saved(Some("fake:usb"), 4)),
            Some(("fake:usb".into(), 0))
        );
        // Saved device absent: the default input, the channel checked against it.
        assert_eq!(
            pick(&saved(Some("fake:gone"), 3)),
            Some(("fake:mic".into(), 0))
        );
        // No default flagged: the first device.
        let plain = [device("x", "x", 2, false), device("y", "y", 2, false)];
        assert_eq!(
            resolve_selection(&plain, &saved(None, 1)).map(|(d, c)| (d.id.as_str(), c)),
            Some(("x", 1))
        );
        assert_eq!(resolve_selection(&[], &RecordingInput::default()), None);
    }

    #[test]
    fn saved_channel_cases() {
        let saved = |channel| RecordingInput {
            device_id: None,
            channel,
        };
        // An exclusive endpoint reports no channels: any saved channel is kept.
        let exclusive = device("x", "x", 0, false);
        assert_eq!(saved_channel(&exclusive, &saved(7)), 7);
        let usb = FakePlatform::usb();
        assert_eq!(saved_channel(&usb, &saved(3)), 3);
        assert_eq!(saved_channel(&usb, &saved(4)), 0);
        assert_eq!(saved_channel(&usb, &saved(9)), 0);
    }
}

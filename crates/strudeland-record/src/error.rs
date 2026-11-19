//! The crate's error type and its mapping from cpal's.

use cpal::ErrorKind;

/// Every way arming, starting or stopping a take can fail.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// No input device, or the saved id is not present.
    #[error("no input device")]
    NoDevice,
    #[error("microphone access is denied")]
    PermissionDenied,
    #[error("the input device is busy")]
    DeviceBusy,
    #[error("the input device was lost: {0}")]
    DeviceLost(String),
    /// The device's sample format is unsupported, or the channel is out of range.
    #[error("unsupported input format: {0}")]
    UnsupportedFormat(String),
    #[error("not armed")]
    NotArmed,
    #[error("already recording")]
    AlreadyRecording,
    /// No samples written within the start timeout (2 s).
    #[error("the capture did not start")]
    StartTimeout,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("wav: {0}")]
    Wav(String),
    #[error("audio backend: {0}")]
    Backend(String),
}

impl RecordError {
    /// Maps a cpal error onto the crate's kinds.
    pub fn from_cpal(error: &cpal::Error) -> RecordError {
        let message = error.to_string();
        match error.kind() {
            ErrorKind::PermissionDenied => RecordError::PermissionDenied,
            ErrorKind::DeviceNotAvailable
            | ErrorKind::StreamInvalidated
            | ErrorKind::DeviceChanged => RecordError::DeviceLost(message),
            ErrorKind::DeviceBusy => RecordError::DeviceBusy,
            ErrorKind::UnsupportedConfig => RecordError::UnsupportedFormat(message),
            _ => RecordError::Backend(message),
        }
    }
}

impl From<hound::Error> for RecordError {
    fn from(error: hound::Error) -> RecordError {
        match error {
            hound::Error::IoError(e) => RecordError::Io(e),
            other => RecordError::Wav(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(kind: ErrorKind) -> RecordError {
        RecordError::from_cpal(&cpal::Error::from(kind))
    }

    #[test]
    fn cpal_kinds_map_onto_record_errors() {
        assert!(matches!(
            map(ErrorKind::PermissionDenied),
            RecordError::PermissionDenied
        ));
        assert!(matches!(
            map(ErrorKind::DeviceNotAvailable),
            RecordError::DeviceLost(_)
        ));
        assert!(matches!(
            map(ErrorKind::StreamInvalidated),
            RecordError::DeviceLost(_)
        ));
        assert!(matches!(
            map(ErrorKind::DeviceChanged),
            RecordError::DeviceLost(_)
        ));
        assert!(matches!(
            map(ErrorKind::DeviceBusy),
            RecordError::DeviceBusy
        ));
        assert!(matches!(
            map(ErrorKind::UnsupportedConfig),
            RecordError::UnsupportedFormat(_)
        ));
        assert!(matches!(map(ErrorKind::Xrun), RecordError::Backend(_)));
        assert!(matches!(map(ErrorKind::Other), RecordError::Backend(_)));
    }

    #[test]
    fn device_lost_keeps_the_cpal_message() {
        let RecordError::DeviceLost(message) = map(ErrorKind::DeviceNotAvailable) else {
            panic!("not DeviceLost");
        };
        assert_eq!(message, ErrorKind::DeviceNotAvailable.to_string());
    }

    #[test]
    fn hound_io_errors_stay_io() {
        let io = hound::Error::IoError(std::io::Error::other("disk"));
        assert!(matches!(RecordError::from(io), RecordError::Io(_)));
        let wav = RecordError::from(hound::Error::Unsupported);
        assert!(matches!(wav, RecordError::Wav(_)));
    }
}

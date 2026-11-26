//! [`TakeError`]: every way a take verb fails, with the text the user sees.

use std::mem::discriminant;
use std::sync::Arc;

use strudeland_project::ProjectError;

use crate::RecordError;

#[cfg(target_os = "macos")]
const RESTRICTED: &str = "microphone access is restricted on this mac";
#[cfg(not(target_os = "macos"))]
const RESTRICTED: &str = "microphone access is restricted by the system";

/// A take verb's failure, or the error a state event carries. Each `Display` is the message the
/// app's `CommandError` for it carries; the app maps the kinds in one table.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TakeError {
    #[error("a take is in progress")]
    Busy,
    #[error("a microphone access request is pending")]
    PermissionPending,
    #[error("the recorder is not armed")]
    NotArmed,
    #[error("no take in progress")]
    NoTake,
    /// The writer received no samples within the start timeout.
    #[error("no audio arrived from the input")]
    NoAudio,
    /// Access was denied (a `NotDetermined` after a request reads as denied).
    #[error("microphone access denied")]
    PermissionDenied,
    #[error("{}", RESTRICTED)]
    PermissionRestricted,
    #[error("channel {channel} is out of range for {channels} channels")]
    ChannelOutOfRange { channel: u16, channels: u16 },
    /// The take stopped at the recorder's length cap.
    #[error("recording stopped at the length limit")]
    LimitReached,
    /// The writer failed mid-take; the message is the writer's.
    #[error("the take could not be written: {0}")]
    WriteFailed(String),
    /// A recorder error with no domain variant, in its own text. An `Arc`: a failed stop sends
    /// the same error in its event and its result.
    #[error("{0}")]
    Record(Arc<RecordError>),
    /// A memo file error, in its own text.
    #[error("{0}")]
    Memo(Arc<ProjectError>),
}

/// Equal when the variant and the text are: `RecordError` and `ProjectError` hold an
/// `io::Error`, so the derive is not available.
impl PartialEq for TakeError {
    fn eq(&self, other: &TakeError) -> bool {
        discriminant(self) == discriminant(other) && self.to_string() == other.to_string()
    }
}

/// The four recorder errors with a domain meaning get their variant; every other one is
/// [`TakeError::Record`].
impl From<RecordError> for TakeError {
    fn from(e: RecordError) -> TakeError {
        match e {
            RecordError::AlreadyRecording => TakeError::Busy,
            RecordError::NotArmed => TakeError::NotArmed,
            RecordError::StartTimeout => TakeError::NoAudio,
            RecordError::PermissionDenied => TakeError::PermissionDenied,
            other => TakeError::Record(Arc::new(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_error_texts() {
        #[cfg(target_os = "macos")]
        let restricted = "microphone access is restricted on this mac";
        #[cfg(not(target_os = "macos"))]
        let restricted = "microphone access is restricted by the system";
        let cases = [
            (TakeError::Busy, "a take is in progress"),
            (
                TakeError::PermissionPending,
                "a microphone access request is pending",
            ),
            (TakeError::NotArmed, "the recorder is not armed"),
            (TakeError::NoTake, "no take in progress"),
            (TakeError::NoAudio, "no audio arrived from the input"),
            (TakeError::PermissionDenied, "microphone access denied"),
            (TakeError::PermissionRestricted, restricted),
            (
                TakeError::ChannelOutOfRange {
                    channel: 4,
                    channels: 4,
                },
                "channel 4 is out of range for 4 channels",
            ),
            (
                TakeError::LimitReached,
                "recording stopped at the length limit",
            ),
            (
                TakeError::WriteFailed("disk full".into()),
                "the take could not be written: disk full",
            ),
            (
                TakeError::Record(Arc::new(RecordError::Backend("x".into()))),
                "audio backend: x",
            ),
            (
                TakeError::Memo(Arc::new(ProjectError::RecordingNotFound("a".into()))),
                "memo a not found",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text, "{error:?}");
        }
        assert_eq!(
            TakeError::from(RecordError::AlreadyRecording),
            TakeError::Busy
        );
        assert_eq!(TakeError::from(RecordError::NotArmed), TakeError::NotArmed);
        assert_eq!(
            TakeError::from(RecordError::StartTimeout),
            TakeError::NoAudio
        );
        assert_eq!(
            TakeError::from(RecordError::PermissionDenied),
            TakeError::PermissionDenied
        );
        assert_eq!(
            TakeError::from(RecordError::NoDevice),
            TakeError::Record(Arc::new(RecordError::NoDevice))
        );
        // Equality is the variant and the text.
        assert_ne!(
            TakeError::WriteFailed("a".into()),
            TakeError::WriteFailed("b".into())
        );
    }
}

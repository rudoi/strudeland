//! The app's side of the control endpoint: everything between a socket path and a verified
//! stream.
//!
//! - **Unix:** the `{path}.lock` lock (created `0600`, never removed), the stale probe, the
//!   bind, the modes (a missing parent `0700`, the socket `0600`), on Linux the private-parent
//!   step, and accept with the Linux peer check. A rejected peer is dropped silently.
//! - **Windows:** the first-instance pipe, the in-use decision for a pipe another strudeland of
//!   this account owns, and the next pipe instance after each connect.
//!
//! [`bind`] must be called inside a tokio runtime with IO enabled: the Unix listener and the
//! Windows pipe register with it, and panic without one. Every detail text is the one the app
//! has always shown; the caller adds its own `control: …` log lines.

use std::io;
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Endpoint, Stream, bind};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Endpoint, Stream, bind};

/// How long to wait before the next `accept` after a failure that pauses.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

/// What binding the control endpoint gave.
#[derive(Debug)]
pub enum Bind {
    /// This process owns the endpoint.
    Listening(Endpoint),
    /// Another strudeland owns it; the detail names it.
    InUse(String),
    /// The endpoint cannot be used; the detail says why.
    Disabled(String),
}

/// Why [`Endpoint::accept`] returned no stream.
#[derive(Debug)]
pub enum AcceptError {
    /// Unix `accept` or Windows `connect` failed.
    Accept { source: io::Error, closed: bool },
    /// Windows only: the next pipe instance could not be created, so the client that just
    /// connected was dropped. Never returned on Unix.
    NextInstance { source: io::Error, closed: bool },
}

impl AcceptError {
    /// The endpoint accepts no more. Windows: resetting the pipe after the failure failed too.
    /// Always false on Unix.
    pub fn closed(&self) -> bool {
        match self {
            Self::Accept { closed, .. } | Self::NextInstance { closed, .. } => *closed,
        }
    }

    /// How long the caller waits before the next `accept`: 100 ms after a Unix accept failure
    /// and after a next-instance failure, none after a Windows connect failure.
    pub fn pause(&self) -> Option<Duration> {
        match self {
            Self::Accept { .. } => cfg!(unix).then_some(ACCEPT_PAUSE),
            Self::NextInstance { .. } => Some(ACCEPT_PAUSE),
        }
    }
}

/// The server half of a `ping` round trip: reads one request, asserts it is `ping`, answers
/// `"ok"`, then waits for the client to close.
#[cfg(test)]
async fn answer_one_ping(stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let mut stream = BufReader::new(stream);
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "ping");
    stream
        .get_mut()
        .write_all(
            format!(
                "{}\n",
                serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":"ok"})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    // Keep the stream open until the client has received the response.
    assert_eq!(stream.get_mut().read(&mut [0]).await.unwrap(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_error_pauses_and_closes_as_today() {
        let error = || io::Error::other("failed");
        for closed in [false, true] {
            let accept = AcceptError::Accept {
                source: error(),
                closed,
            };
            assert_eq!(accept.closed(), closed);
            let expected = if cfg!(unix) {
                Some(Duration::from_millis(100))
            } else {
                None
            };
            assert_eq!(accept.pause(), expected);

            let next = AcceptError::NextInstance {
                source: error(),
                closed,
            };
            assert_eq!(next.closed(), closed);
            assert_eq!(next.pause(), Some(Duration::from_millis(100)));
        }
    }
}

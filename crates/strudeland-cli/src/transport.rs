//! The real transport: a lazily connected control socket client.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use strudeland_ctl::{CallError, Client, Method, socket_path_with};

use crate::run::Transport;

pub struct SocketTransport {
    socket: Option<PathBuf>,
    timeout: Duration,
    client: Option<Client>,
}

impl SocketTransport {
    /// `socket` is the `--socket` value; `timeout_secs` the `--timeout` value.
    pub fn new(socket: Option<PathBuf>, timeout_secs: u64) -> Self {
        SocketTransport {
            socket,
            timeout: Duration::from_secs(timeout_secs),
            client: None,
        }
    }

    fn connect(&self) -> Result<Client, CallError> {
        let path = socket_path_with(self.socket.as_deref())
            .map_err(|e| CallError::Io(io::Error::other(e.to_string())))?;
        Client::connect(&path, self.timeout)
    }
}

impl Transport for SocketTransport {
    fn call(
        &mut self,
        method: Method,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        if self.client.is_none() {
            self.client = Some(self.connect()?);
        }
        let client = self.client.as_mut().expect("connected above");
        client.call_raw(method.as_str(), params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_failure_is_a_call_error() {
        let dir = tempfile::Builder::new()
            .prefix("t")
            .tempdir_in("/tmp")
            .unwrap();
        let path = dir.path().join("none.sock");
        let mut t = SocketTransport::new(Some(path.clone()), 1);
        match t.call(Method::Ping, serde_json::json!({})) {
            Err(CallError::NotRunning { path: p }) => assert_eq!(p, path),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn too_long_socket_path_is_an_error() {
        let long = PathBuf::from(format!("/{}", "a".repeat(120)));
        let mut t = SocketTransport::new(Some(long), 1);
        assert!(matches!(
            t.call(Method::Ping, serde_json::json!({})),
            Err(CallError::Io(_))
        ));
    }
}

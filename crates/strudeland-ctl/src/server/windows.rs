//! The Windows endpoint: the first-instance pipe, the in-use decision and the next pipe
//! instance after each connect.

use std::path::{Path, PathBuf};

use tokio::net::windows::named_pipe::NamedPipeServer;

use super::{AcceptError, Bind};
use crate::windows_pipe;

/// A connected client. The pipe's DACL admits only this account, so it needs no further check.
pub type Stream = NamedPipeServer;

/// The pipe instance waiting for the next client.
#[derive(Debug)]
pub struct Endpoint {
    pipe: tokio::sync::Mutex<NamedPipeServer>,
    path: PathBuf,
}

/// Creates the first pipe instance at `path`. When that fails, another strudeland of this account
/// owning the pipe makes it in use, anything else disabled; both carry the create error. Must be
/// called inside a tokio runtime with IO enabled.
pub fn bind(path: &Path) -> Bind {
    match windows_pipe::listen(path, true) {
        Ok(pipe) => Bind::Listening(Endpoint {
            pipe: tokio::sync::Mutex::new(pipe),
            path: path.to_path_buf(),
        }),
        Err(e) => {
            let message = format!("cannot listen on {}: {e}", path.display());
            if windows_pipe::owned_existing(path) {
                Bind::InUse(message)
            } else {
                Bind::Disabled(message)
            }
        }
    }
}

impl Endpoint {
    /// Waits for a client, creates the next pipe instance and returns the connected one.
    /// On a failure the waiting instance is reset; `closed` says the reset failed too.
    /// Cancel-safe.
    pub async fn accept(&self) -> Result<Stream, AcceptError> {
        let mut pipe = self.pipe.lock().await;
        if let Err(source) = pipe.connect().await {
            let closed = pipe.disconnect().is_err();
            return Err(AcceptError::Accept { source, closed });
        }
        // Keep at least one instance alive, and ensure a second client can connect.
        let next = match windows_pipe::listen(&self.path, false) {
            Ok(next) => next,
            Err(source) => {
                let closed = pipe.disconnect().is_err();
                return Err(AcceptError::NextInstance { source, closed });
            }
        };
        Ok(std::mem::replace(&mut *pipe, next))
    }

    /// Does nothing: the waiting pipe instance closes when the endpoint drops, and a named pipe
    /// leaves no file behind.
    pub fn close(&self) {}
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::answer_one_ping;
    use crate::{CallError, Client};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn endpoint() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            r"\\.\pipe\strudeland-server-test-{}-{id}",
            std::process::id()
        ))
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_second_client_connects_while_the_first_is_open() {
        let path = endpoint();
        let rt = runtime();
        let endpoint = {
            let _guard = rt.enter();
            let Bind::Listening(endpoint) = bind(&path) else {
                panic!("expected Listening");
            };
            endpoint
        };
        let server = std::thread::spawn(move || {
            rt.block_on(async {
                let a = endpoint.accept().await.unwrap();
                let b = endpoint.accept().await.unwrap();
                answer_one_ping(a).await;
                answer_one_ping(b).await;
            });
        });
        let mut a = Client::connect(&path, Duration::from_secs(1)).unwrap();
        // Only the next instance can take a second client while the first is connected.
        let mut b = Client::connect(&path, Duration::from_secs(1)).unwrap();
        assert_eq!(a.call_raw("ping", json!({})).unwrap(), json!("ok"));
        drop(a);
        assert_eq!(b.call_raw("ping", json!({})).unwrap(), json!("ok"));
        drop(b);
        server.join().unwrap();
    }

    #[test]
    fn a_second_bind_is_in_use_with_the_listen_error() {
        let path = endpoint();
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(_endpoint) = bind(&path) else {
            panic!("expected Listening");
        };
        let e = windows_pipe::listen(&path, true).unwrap_err();
        let Bind::InUse(detail) = bind(&path) else {
            panic!("expected InUse");
        };
        assert_eq!(detail, format!("cannot listen on {}: {e}", path.display()));
    }

    #[test]
    fn an_invalid_endpoint_is_disabled() {
        let path = PathBuf::from(r"\\.\pipe\bad\component");
        let rt = runtime();
        let _guard = rt.enter();
        let e = windows_pipe::listen(&path, true).unwrap_err();
        let Bind::Disabled(detail) = bind(&path) else {
            panic!("expected Disabled");
        };
        assert_eq!(detail, format!("cannot listen on {}: {e}", path.display()));
    }

    #[test]
    fn a_dropped_endpoint_is_not_running() {
        let path = endpoint();
        let rt = runtime();
        let _guard = rt.enter();
        let Bind::Listening(endpoint) = bind(&path) else {
            panic!("expected Listening");
        };
        drop(endpoint);
        assert!(matches!(
            Client::connect(&path, Duration::from_secs(1)),
            Err(CallError::NotRunning { .. })
        ));
    }
}

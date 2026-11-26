//! A blocking client over the Unix socket. One request at a time, answered in order.

#[cfg(windows)]
use crate::windows_pipe::Pipe;
use std::io::{self, BufReader};
#[cfg(unix)]
use std::os::unix::net::UnixStream as Pipe;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::frame::{FrameError, read_message, write_message};
use crate::methods::Rpc;
use crate::protocol::{JSONRPC_VERSION, MAX_LINE_BYTES, Request, Response, RpcError};

/// The largest response line the client reads (16 MiB), so a big `code.get` is never cut.
pub(crate) const RESPONSE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// How much longer than the request timeout the client waits on the socket, so the app's own
/// timeout error normally arrives first.
const GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// Nothing listens at the path (`ENOENT` or `ECONNREFUSED` on connect).
    #[cfg_attr(unix, error("strudeland is not running (no socket at {})", path.display()))]
    #[cfg_attr(windows, error("strudeland is not running (no pipe at {})", path.display()))]
    NotRunning { path: PathBuf },
    /// Any other transport failure, including the app closing the connection early.
    #[error("{0}")]
    Io(io::Error),
    /// The client's own read or write timeout fired.
    #[error("the app did not answer in time")]
    Timeout,
    /// The request line (newline included) is over [`MAX_LINE_BYTES`]; it was not sent.
    #[error("request too large ({bytes} bytes; the app reads at most {MAX_LINE_BYTES})")]
    TooLarge { bytes: usize },
    /// A malformed or mismatched response.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The app answered with an error.
    #[error("{0}")]
    Rpc(RpcError),
}

impl CallError {
    fn from_io(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => CallError::Timeout,
            _ => CallError::Io(e),
        }
    }
}

/// `M`'s typed result from a raw `result`; `CallError::Protocol("invalid {method} result: {e}")`
/// when it does not match. The one place that text is built.
pub fn decode_result<M: Rpc>(result: &serde_json::Value) -> Result<M::Result, CallError> {
    <M::Result as serde::Deserialize>::deserialize(result)
        .map_err(|e| CallError::Protocol(format!("invalid {} result: {e}", M::METHOD.as_str())))
}

/// A connection to the app's control socket.
pub struct Client {
    reader: BufReader<Pipe>,
    next_id: u64,
    timeout: Duration,
    /// Added to `timeout` for the socket's read and write timeouts.
    grace: Duration,
}

impl Client {
    /// Connects to the socket at `path`. `timeout` is the per-request timeout, sent to the app as
    /// `timeout_ms`; the socket waits `timeout + 5 s`.
    pub fn connect(path: &Path, timeout: Duration) -> Result<Client, CallError> {
        #[cfg(target_os = "linux")]
        if let Some(dir) = path.parent() {
            match crate::path::check_private_dir(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Err(CallError::NotRunning {
                        path: path.to_path_buf(),
                    });
                }
                Err(e) => return Err(CallError::Io(e)),
            }
        }
        #[cfg(unix)]
        let stream = Pipe::connect(path);
        #[cfg(windows)]
        let stream = Pipe::connect(path, timeout.saturating_add(GRACE));
        let stream = stream.map_err(|e| match e.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => CallError::NotRunning {
                path: path.to_path_buf(),
            },
            _ => CallError::from_io(e),
        })?;
        #[cfg(target_os = "linux")]
        crate::path::check_peer(&stream).map_err(CallError::Io)?;
        Ok(Client {
            reader: BufReader::new(stream),
            next_id: 1,
            timeout,
            grace: GRACE,
        })
    }

    /// Calls a method with typed params and parses its typed result.
    pub fn call<M: Rpc>(&mut self, params: &M::Params) -> Result<M::Result, CallError> {
        let params = serde_json::to_value(params)
            .map_err(|e| CallError::Protocol(format!("cannot encode params: {e}")))?;
        let result = self.call_raw(M::METHOD.as_str(), params)?;
        decode_result::<M>(&result)
    }

    /// Calls a method by name and returns its raw `result`.
    pub fn call_raw(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            timeout_ms: Some(u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX)),
            ..Request::new(id, method, params)
        };
        let line = serde_json::to_string(&request)
            .map_err(|e| CallError::Protocol(format!("cannot encode request: {e}")))?;
        // The app would answer `request too large` and close without reading it all.
        if line.len() + 1 > MAX_LINE_BYTES {
            return Err(CallError::TooLarge {
                bytes: line.len() + 1,
            });
        }

        let io_timeout = self.timeout.saturating_add(self.grace);
        let stream = self.reader.get_mut();
        #[cfg(unix)]
        {
            stream
                .set_read_timeout(Some(io_timeout))
                .map_err(CallError::Io)?;
            stream
                .set_write_timeout(Some(io_timeout))
                .map_err(CallError::Io)?;
        }
        #[cfg(windows)]
        stream.set_deadline(io_timeout);
        write_message(stream, &line).map_err(CallError::from_io)?;

        let raw = read_message(&mut self.reader, RESPONSE_MAX_BYTES);
        #[cfg(windows)]
        if !matches!(&raw, Ok(Some(_))) {
            self.reader.get_mut().disconnect();
        }
        let line = match raw {
            Ok(Some(line)) => line,
            Ok(None) => {
                return Err(CallError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the app closed the connection",
                )));
            }
            Err(FrameError::Io(e)) => return Err(CallError::from_io(e)),
            Err(FrameError::TooLong) => {
                return Err(CallError::Protocol("response too large".into()));
            }
            Err(FrameError::NotUtf8) => {
                return Err(CallError::Protocol("response is not valid UTF-8".into()));
            }
        };
        let response: Response = serde_json::from_str(&line)
            .map_err(|e| CallError::Protocol(format!("invalid response: {e}")))?;
        if response.jsonrpc != JSONRPC_VERSION {
            return Err(CallError::Protocol(format!(
                "unexpected jsonrpc version {:?}",
                response.jsonrpc
            )));
        }
        // A null id with an error is the app failing the line itself (one request in flight).
        let null_error = response.id.is_none() && response.error.is_some();
        if response.id != Some(id) && !null_error {
            return Err(CallError::Protocol(format!(
                "response id {} does not match request id {id}",
                response
                    .id
                    .map_or_else(|| "null".to_string(), |i| i.to_string())
            )));
        }
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(CallError::Rpc(error)),
            (Some(_), Some(_)) => Err(CallError::Protocol(
                "response has both result and error".into(),
            )),
            (None, None) => Err(CallError::Protocol(
                "response has neither result nor error".into(),
            )),
        }
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;
    use crate::methods::{StateCall, Stop, StopResult};
    use serde_json::json;

    #[test]
    fn decode_result_names_the_method() {
        match decode_result::<StateCall>(&json!({"project": null})) {
            Err(CallError::Protocol(m)) => {
                assert_eq!(m, "invalid state result: missing field `songs`")
            }
            other => panic!("expected a protocol error, got {other:?}"),
        }
        assert_eq!(
            decode_result::<Stop>(&json!({"wasPlaying": true})).unwrap(),
            StopResult { was_playing: true }
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::methods::{Empty, Ping, PingResult, SongParams, StateCall, Stop, StopResult};
    use crate::protocol::codes;
    use serde_json::{Value, json};
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::thread::{self, JoinHandle};

    /// A short-named tempdir, since socket paths are limited to 103 bytes.
    fn short_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        tempfile::Builder::new()
            .prefix("c")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap()
    }

    /// Accepts one connection and answers each request line with `respond(request)`; `None`
    /// closes the connection. Every request read is sent on the returned channel.
    fn serve<F>(path: &Path, mut respond: F) -> (mpsc::Receiver<Value>, JoinHandle<()>)
    where
        F: FnMut(&Value) -> Option<String> + Send + 'static,
    {
        let listener = UnixListener::bind(path).unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            while let Ok(Some(line)) = read_message(&mut reader, RESPONSE_MAX_BYTES) {
                let request: Value = serde_json::from_str(&line).unwrap();
                let reply = respond(&request);
                tx.send(request).unwrap();
                match reply {
                    Some(reply) => {
                        if write_message(&mut writer, &reply).is_err() {
                            return;
                        }
                    }
                    None => return,
                }
            }
        });
        (rx, handle)
    }

    fn ok_line(request: &Value, result: Value) -> String {
        json!({"jsonrpc": "2.0", "id": request["id"], "result": result}).to_string()
    }

    #[test]
    fn call_raw_ok() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (rx, h) = serve(&path, |r| Some(ok_line(r, json!({"wasPlaying": true}))));
        let mut c = Client::connect(&path, Duration::from_secs(2)).unwrap();
        let result = c.call_raw("stop", json!({})).unwrap();
        assert_eq!(result, json!({"wasPlaying": true}));
        let sent = rx.recv().unwrap();
        assert_eq!(
            sent,
            json!({"jsonrpc":"2.0","id":1,"method":"stop","params":{},"timeout_ms":2000})
        );
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn typed_call_ok_and_ids_increase() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (rx, h) = serve(&path, |r| match r["method"].as_str().unwrap() {
            "ping" => Some(ok_line(r, json!({"version": "0.1.0", "attached": true}))),
            "stop" => Some(ok_line(r, json!({"wasPlaying": false}))),
            m => panic!("unexpected {m}"),
        });
        let mut c = Client::connect(&path, Duration::from_secs(7)).unwrap();
        assert_eq!(
            c.call::<Ping>(&Empty {}).unwrap(),
            PingResult {
                version: "0.1.0".into(),
                attached: true
            }
        );
        assert_eq!(
            c.call::<Stop>(&Empty {}).unwrap(),
            StopResult { was_playing: false }
        );
        assert_eq!(
            c.call::<Ping>(&Empty {}).unwrap().version,
            "0.1.0".to_string()
        );
        drop(c);
        h.join().unwrap();
        let ids: Vec<_> = rx.iter().map(|r| r["id"].as_u64().unwrap()).collect();
        assert_eq!(ids, [1, 2, 3]);
    }

    #[test]
    fn timeout_ms_is_timeout_times_1000() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (rx, h) = serve(&path, |r| Some(ok_line(r, json!({}))));
        let mut c = Client::connect(&path, Duration::from_secs(30)).unwrap();
        c.call_raw("state", json!({})).unwrap();
        drop(c);
        h.join().unwrap();
        assert_eq!(rx.recv().unwrap()["timeout_ms"], json!(30_000));
    }

    #[test]
    fn typed_params_are_sent_camel_case_and_omitted() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (rx, h) = serve(&path, |r| {
            Some(ok_line(
                r,
                json!({"ok":true,"song":"songs/main.js","title":"Main","started":true,"audio":"running","error":null}),
            ))
        });
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let out = c
            .call::<crate::methods::EvalCall>(&SongParams { song: None })
            .unwrap();
        assert!(out.ok);
        drop(c);
        h.join().unwrap();
        let sent = rx.recv().unwrap();
        assert_eq!(sent["method"], "eval");
        assert_eq!(sent["params"], json!({}));
    }

    #[test]
    fn rpc_error_is_returned() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |r| {
            Some(
                json!({"jsonrpc":"2.0","id":r["id"],"error":{"code":-32010,"message":"the song changed since you read it"}})
                    .to_string(),
            )
        });
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        match c.call_raw("code.set", json!({"code":"x","expect":"y"})) {
            Err(CallError::Rpc(e)) => {
                assert_eq!(e.code, codes::SONG_CHANGED);
                assert_eq!(e.message, "the song changed since you read it");
            }
            other => panic!("unexpected {other:?}"),
        }
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn server_closing_early_is_io() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |_| None);
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let e = c.call_raw("state", json!({})).unwrap_err();
        assert!(matches!(e, CallError::Io(_)), "{e:?}");
        h.join().unwrap();
    }

    #[test]
    fn read_timeout_is_timeout() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let listener = UnixListener::bind(&path).unwrap();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let h = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            // Hold the connection open without answering until the test is done.
            let _ = done_rx.recv();
        });
        let mut c = Client::connect(&path, Duration::from_millis(150)).unwrap();
        c.grace = Duration::ZERO;
        let e = c.call_raw("state", json!({})).unwrap_err();
        assert!(matches!(e, CallError::Timeout), "{e:?}");
        done_tx.send(()).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn mismatched_id_is_protocol() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |_| {
            Some(json!({"jsonrpc":"2.0","id":99,"result":{}}).to_string())
        });
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let e = c.call_raw("state", json!({})).unwrap_err();
        match e {
            CallError::Protocol(m) => assert!(m.contains("99"), "{m}"),
            other => panic!("unexpected {other:?}"),
        }
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn malformed_responses_are_protocol() {
        let cases: Vec<fn(&Value) -> String> = vec![
            |_| "not json".to_string(),
            |r| json!({"jsonrpc":"2.0","id":r["id"]}).to_string(),
            |r| {
                json!({"jsonrpc":"2.0","id":r["id"],"result":{},"error":{"code":1,"message":"m"}})
                    .to_string()
            },
            |r| json!({"jsonrpc":"1.0","id":r["id"],"result":{}}).to_string(),
            |_| json!({"jsonrpc":"2.0","id":null,"result":{}}).to_string(),
        ];
        for (i, case) in cases.into_iter().enumerate() {
            let dir = short_dir();
            let path = dir.path().join("s");
            let (_rx, h) = serve(&path, move |r| Some(case(r)));
            let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
            let e = c.call_raw("state", json!({})).unwrap_err();
            assert!(matches!(e, CallError::Protocol(_)), "case {i}: {e:?}");
            drop(c);
            h.join().unwrap();
        }
    }

    #[test]
    fn a_null_id_error_answers_the_request() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |_| {
            Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"request too large"}})
                    .to_string(),
            )
        });
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        match c.call_raw("state", json!({})) {
            Err(CallError::Rpc(e)) => assert_eq!(e, RpcError::too_large()),
            other => panic!("unexpected {other:?}"),
        }
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn an_oversize_request_is_refused_before_sending() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (rx, h) = serve(&path, |r| Some(ok_line(r, json!({}))));
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let e = c
            .call_raw("code.set", json!({"code": "x".repeat(MAX_LINE_BYTES)}))
            .unwrap_err();
        assert!(matches!(e, CallError::TooLarge { .. }), "{e:?}");
        assert!(e.to_string().starts_with("request too large ("), "{e}");
        // Nothing was sent; the connection still works.
        c.call_raw("state", json!({})).unwrap();
        drop(c);
        h.join().unwrap();
        let sent: Vec<Value> = rx.iter().collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "state");
    }

    #[test]
    fn null_result_is_a_result() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |r| Some(ok_line(r, Value::Null)));
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        assert_eq!(c.call_raw("state", json!({})).unwrap(), Value::Null);
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn wrong_result_shape_is_protocol() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let (_rx, h) = serve(&path, |r| Some(ok_line(r, json!({"version": 1}))));
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        match c.call::<StateCall>(&Empty {}) {
            Err(CallError::Protocol(m)) => assert!(m.starts_with("invalid state result"), "{m}"),
            other => panic!("unexpected {other:?}"),
        }
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn missing_path_is_not_running() {
        let dir = short_dir();
        let path = dir.path().join("none");
        match Client::connect(&path, Duration::from_secs(1)) {
            Err(CallError::NotRunning { path: p }) => assert_eq!(p, path),
            Err(e) => panic!("unexpected {e:?}"),
            Ok(_) => panic!("connected"),
        }
    }

    #[test]
    fn stale_socket_is_not_running() {
        let dir = short_dir();
        let path = dir.path().join("s");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(matches!(
            Client::connect(&path, Duration::from_secs(1)),
            Err(CallError::NotRunning { .. })
        ));
    }

    #[test]
    fn not_running_message_names_the_path() {
        let e = CallError::NotRunning {
            path: PathBuf::from("/x/ctl.sock"),
        };
        assert_eq!(
            e.to_string(),
            "strudeland is not running (no socket at /x/ctl.sock)"
        );
    }

    #[test]
    fn large_response_is_read_whole() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let code = "x".repeat(2 * crate::MAX_LINE_BYTES);
        let expected = code.clone();
        let (_rx, h) = serve(&path, move |r| Some(ok_line(r, json!({"code": code}))));
        let mut c = Client::connect(&path, Duration::from_secs(5)).unwrap();
        let result = c.call_raw("code.get", json!({})).unwrap();
        assert_eq!(result["code"].as_str().unwrap().len(), expected.len());
        drop(c);
        h.join().unwrap();
    }

    #[test]
    fn non_utf8_response_is_protocol() {
        let dir = short_dir();
        let path = dir.path().join("s");
        let listener = UnixListener::bind(&path).unwrap();
        let h = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_message(&mut reader, RESPONSE_MAX_BYTES).unwrap();
            stream.write_all(b"\xff\n").unwrap();
        });
        let mut c = Client::connect(&path, Duration::from_secs(1)).unwrap();
        let e = c.call_raw("state", json!({})).unwrap_err();
        assert!(matches!(e, CallError::Protocol(_)), "{e:?}");
        h.join().unwrap();
    }
}

//! The JSON-RPC 2.0 envelope: requests, responses, errors and the error codes.

use serde::{Deserialize, Deserializer, Serialize};

/// The only `jsonrpc` value accepted and sent.
pub(crate) const JSONRPC_VERSION: &str = "2.0";

/// The longest request line the app reads, newline included (1 MiB).
pub const MAX_LINE_BYTES: usize = 1_048_576;

/// The error codes of the control protocol. `src/lib/control/protocol.ts` mirrors them; the wire
/// examples' `errors.json` is the cross-check.
pub mod codes {
    macro_rules! codes {
        ($($(#[$doc:meta])* $name:ident = $value:expr;)*) => {
            $($(#[$doc])* pub const $name: i64 = $value;)*
            /// Every code by its Rust name, in declaration order.
            pub const ALL: &[(&str, i64)] = &[$((stringify!($name), $name)),*];
        };
    }

    codes! {
        /// The line is not valid JSON, or is too long.
        PARSE_ERROR = -32700;
        /// The JSON is not a valid request (bad `id`, `jsonrpc`, `method`, or a notification).
        INVALID_REQUEST = -32600;
        /// The method is not one of `Method`.
        METHOD_NOT_FOUND = -32601;
        /// The params do not match the method.
        INVALID_PARAMS = -32602;
        /// An unexpected exception in the webview handler.
        INTERNAL_ERROR = -32603;
        /// The webview did not answer in time.
        TIMEOUT = -32002;
        /// The webview is not attached (or reloaded while the request was pending).
        NOT_READY = -32003;
        /// No project is open.
        NO_PROJECT = -32004;
        /// The `song` param names no song of the open project.
        NO_SUCH_SONG = -32005;
        /// The song could not be loaded (read failed, or deleted on disk).
        SONG_UNLOADABLE = -32006;
        /// `events` before any successful evaluate.
        NOTHING_EVALUATED = -32007;
        /// The webview snapshot failed.
        SCREENSHOT_FAILED = -32008;
        /// `code.set`'s `expect` does not match the buffer.
        SONG_CHANGED = -32010;
        /// `song.delete` or `project.open` would lose unsaved edits and `discard` was not set.
        UNSAVED = -32011;
        /// `song.new` or `song.rename` names a song that already exists.
        SONG_EXISTS = -32012;
        /// `project.open` was given a path that is not a project.
        OPEN_FAILED = -32013;
        /// `project.open` while the app waits on an open dialog or loads a project the user chose.
        BUSY = -32014;
    }
}

/// A request line. Members are JSON-RPC's own, so there is no `rename_all`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    /// Must be `"2.0"`.
    pub jsonrpc: String,
    /// `None` only while rejecting a request; every accepted request has an id.
    pub id: Option<u64>,
    pub method: String,
    /// Absent or `null` means `{}`.
    #[serde(default)]
    pub params: serde_json::Value,
    /// A non-standard transport hint: how long the app waits for the webview, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl Request {
    /// A `"2.0"` request with the given id.
    pub fn new(id: u64, method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: Some(id),
            method: method.to_string(),
            params,
            timeout_ms: None,
        }
    }
}

/// A response line. Exactly one of `result` and `error` is set; `id` is always serialized.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<u64>,
    /// A present `"result": null` deserializes as `Some(Value::Null)`, not `None`.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response.
    pub fn ok(id: Option<u64>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    pub fn err(id: Option<u64>, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Keeps a present member, even `null`, as `Some`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(d).map(Some)
}

/// A JSON-RPC error object. There is no `data` member. Displays as its message.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// `-32700` for a line that is not valid JSON (or not UTF-8): `parse error: {detail}`.
    pub fn parse(detail: impl std::fmt::Display) -> Self {
        Self::new(codes::PARSE_ERROR, format!("parse error: {detail}"))
    }

    /// `-32700` for a line over [`MAX_LINE_BYTES`].
    pub fn too_large() -> Self {
        Self::new(codes::PARSE_ERROR, "request too large")
    }

    /// `-32003`: the webview is not attached.
    pub fn not_ready() -> Self {
        Self::new(codes::NOT_READY, "app not ready")
    }

    /// `-32003`: the webview reloaded while the request was pending.
    pub fn page_reloaded() -> Self {
        Self::new(codes::NOT_READY, "the page reloaded")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn named_errors() {
        let cases = [
            (RpcError::parse("x"), codes::PARSE_ERROR, "parse error: x"),
            (
                RpcError::too_large(),
                codes::PARSE_ERROR,
                "request too large",
            ),
            (RpcError::not_ready(), codes::NOT_READY, "app not ready"),
            (
                RpcError::page_reloaded(),
                codes::NOT_READY,
                "the page reloaded",
            ),
        ];
        for (e, code, message) in cases {
            assert_eq!((e.code, e.message.as_str()), (code, message));
        }
    }

    #[test]
    fn max_line_is_one_mebibyte() {
        assert_eq!(MAX_LINE_BYTES, 1024 * 1024);
    }

    #[test]
    fn request_defaults_params_and_timeout() {
        let r: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#).unwrap();
        assert_eq!(r.params, serde_json::Value::Null);
        assert_eq!(r.timeout_ms, None);
        assert_eq!(r.id, Some(9));
    }

    #[test]
    fn request_reads_timeout_ms_and_ignores_unknown_members() {
        let r: Request = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":4,"method":"code.set","params":{"code":"x"},"timeout_ms":30000,"extra":1}"#,
        )
        .unwrap();
        assert_eq!(r.timeout_ms, Some(30000));
        assert_eq!(r.params, json!({"code": "x"}));
    }

    #[test]
    fn request_serializes_without_absent_timeout() {
        let r = Request::new(1, "ping", json!({}));
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#
        );
        let r = Request {
            timeout_ms: Some(5000),
            ..r
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{},"timeout_ms":5000}"#
        );
    }

    #[test]
    fn response_always_serializes_id() {
        let r = Response::err(None, RpcError::new(codes::PARSE_ERROR, "parse error: x"));
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error: x"}}"#
        );
        let r = Response::ok(Some(7), json!({"wasPlaying": true}));
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"jsonrpc":"2.0","id":7,"result":{"wasPlaying":true}}"#
        );
    }

    #[test]
    fn response_keeps_a_null_result() {
        let r: Response =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":null}"#).unwrap();
        assert_eq!(r.result, Some(serde_json::Value::Null));
        let r: Response = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1}"#).unwrap();
        assert_eq!(r.result, None);
        assert_eq!(r.error, None);
    }

    #[test]
    fn rpc_error_displays_its_message() {
        let e = RpcError::new(codes::SONG_CHANGED, "the song changed since you read it");
        assert_eq!(e.to_string(), "the song changed since you read it");
    }
}

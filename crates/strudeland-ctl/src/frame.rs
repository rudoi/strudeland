//! Line framing: one JSON object per `\n`-terminated line, plus the pure request parser and
//! response encoder the app's async server reuses.

use std::io::{self, BufRead, Write};

use serde_json::Value;

use crate::protocol::{JSONRPC_VERSION, Request, Response, RpcError, codes};

/// Why a line could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The line (newline included) is longer than the cap. The rest of it is left unread, so the
    /// connection should be closed.
    #[error("line too long")]
    TooLong,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("line is not valid UTF-8")]
    NotUtf8,
}

/// One line being read, at most `max` bytes with its newline. The pure core of [`read_message`],
/// shared with the app's async reader: feed it each buffer the reader fills, consume what it says,
/// and call [`finish`](Self::finish) once it reports the line complete.
#[derive(Debug)]
pub struct LineReader {
    line: Vec<u8>,
    max: usize,
    terminated: bool,
}

impl LineReader {
    pub fn new(max: usize) -> Self {
        Self {
            line: Vec::new(),
            max,
            terminated: false,
        }
    }

    /// Takes the reader's buffered bytes (empty at EOF). Returns how many of them to consume and
    /// whether the line is complete. `TooLong` consumes nothing.
    pub fn feed(&mut self, buf: &[u8]) -> Result<(usize, bool), FrameError> {
        if buf.is_empty() {
            return Ok((0, true));
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                if self.line.len() + i + 1 > self.max {
                    return Err(FrameError::TooLong);
                }
                self.line.extend_from_slice(&buf[..i]);
                self.terminated = true;
                Ok((i + 1, true))
            }
            None => {
                let n = buf.len();
                // Without its newline the line is already `line.len() + n + 1` bytes.
                if self.line.len() + n >= self.max {
                    return Err(FrameError::TooLong);
                }
                self.line.extend_from_slice(buf);
                Ok((n, false))
            }
        }
    }

    /// The line without its `\n`; `None` for EOF before any byte.
    pub fn finish(self) -> Result<Option<String>, FrameError> {
        if !self.terminated && self.line.is_empty() {
            return Ok(None);
        }
        String::from_utf8(self.line)
            .map(Some)
            .map_err(|_| FrameError::NotUtf8)
    }
}

/// Reads one line of at most `max` bytes, newline included, and returns it without the `\n`.
/// `Ok(None)` is EOF before any byte. A last line without a newline is returned as is.
pub fn read_message<R: BufRead>(r: &mut R, max: usize) -> Result<Option<String>, FrameError> {
    let mut line = LineReader::new(max);
    loop {
        let buf = match r.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FrameError::Io(e)),
        };
        let (n, done) = line.feed(buf)?;
        r.consume(n);
        if done {
            return line.finish();
        }
    }
}

/// Writes `line` and a `\n`, then flushes.
pub fn write_message<W: Write>(w: &mut W, line: &str) -> io::Result<()> {
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()
}

/// Parses and validates a request line. On failure returns the id (when it could be recovered)
/// and the error to answer with: `-32700` for invalid JSON, `-32600` for a JSON value that is
/// not a valid request. `params` that are absent or `null` become `{}`; anything else is passed
/// through unchecked. The method name is not checked here.
pub fn parse_request(line: &str) -> Result<Request, (Option<u64>, RpcError)> {
    let value: Value = serde_json::from_str(line).map_err(|e| (None, RpcError::parse(e)))?;
    let invalid = |id: Option<u64>, why: &str| {
        (
            id,
            RpcError::new(codes::INVALID_REQUEST, format!("invalid request: {why}")),
        )
    };
    let Value::Object(mut obj) = value else {
        return Err(invalid(None, "expected a JSON object"));
    };
    let id = match obj.get("id") {
        None => {
            return Err(invalid(
                None,
                "missing id (notifications are not supported)",
            ));
        }
        Some(v) => match v.as_u64() {
            Some(id) => id,
            None => return Err(invalid(None, "id must be a non-negative integer")),
        },
    };
    let id = Some(id);
    if obj.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid(id, "jsonrpc must be \"2.0\""));
    }
    let method = match obj.get("method").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Err(invalid(id, "method must be a string")),
    };
    let timeout_ms = match obj.get("timeout_ms") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64() {
            Some(ms) => Some(ms),
            None => return Err(invalid(id, "timeout_ms must be a non-negative integer")),
        },
    };
    let params = match obj.remove("params") {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(p) => p,
    };
    Ok(Request {
        jsonrpc: JSONRPC_VERSION.to_string(),
        id,
        method,
        params,
        timeout_ms,
    })
}

/// Encodes a response as one line of JSON, without the trailing newline.
pub fn encode_response(r: &Response) -> String {
    serde_json::to_string(r).expect("a Response always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufReader, Cursor};

    fn read_all(input: &[u8], max: usize) -> Vec<Result<Option<String>, String>> {
        let mut r = BufReader::with_capacity(4, Cursor::new(input.to_vec()));
        let mut out = Vec::new();
        loop {
            let m = read_message(&mut r, max).map_err(|e| e.to_string());
            let stop = !matches!(m, Ok(Some(_)));
            out.push(m);
            if stop {
                return out;
            }
        }
    }

    #[test]
    fn reads_lines_then_eof() {
        assert_eq!(
            read_all(b"{\"a\":1}\n{\"b\":2}\n", 100),
            vec![
                Ok(Some("{\"a\":1}".into())),
                Ok(Some("{\"b\":2}".into())),
                Ok(None)
            ]
        );
    }

    #[test]
    fn eof_is_none() {
        assert_eq!(read_all(b"", 100), vec![Ok(None)]);
    }

    #[test]
    fn last_line_without_newline_is_returned() {
        assert_eq!(
            read_all(b"abc", 100),
            vec![Ok(Some("abc".into())), Ok(None)]
        );
    }

    #[test]
    fn empty_line_is_an_empty_message() {
        assert_eq!(
            read_all(b"\nx\n", 100),
            vec![Ok(Some(String::new())), Ok(Some("x".into())), Ok(None)]
        );
    }

    #[test]
    fn cap_counts_the_newline() {
        // 9 bytes + newline = 10: fits a cap of 10, not of 9.
        let line = b"123456789\n";
        assert_eq!(read_all(line, 10)[0], Ok(Some("123456789".into())));
        assert_eq!(read_all(line, 9)[0], Err("line too long".into()));
        // Without a newline the line still needs room for one.
        assert_eq!(read_all(b"123456789", 10)[0], Ok(Some("123456789".into())));
        assert_eq!(read_all(b"1234567890", 10)[0], Err("line too long".into()));
    }

    #[test]
    fn line_over_the_real_cap_is_too_long() {
        let mut big = vec![b'x'; crate::MAX_LINE_BYTES];
        big.push(b'\n');
        let mut r = BufReader::new(Cursor::new(big));
        assert!(matches!(
            read_message(&mut r, crate::MAX_LINE_BYTES),
            Err(FrameError::TooLong)
        ));
    }

    #[test]
    fn non_utf8_line_is_rejected() {
        assert_eq!(
            read_all(b"\xff\xfe\n", 100)[0],
            Err("line is not valid UTF-8".into())
        );
    }

    #[test]
    fn write_appends_newline() {
        let mut out = Vec::new();
        write_message(&mut out, r#"{"a":1}"#).unwrap();
        assert_eq!(out, b"{\"a\":1}\n");
    }

    #[test]
    fn round_trip_request_through_frames() {
        let req = Request {
            timeout_ms: Some(30000),
            ..Request::new(4, "code.set", json!({"code": "$: s(\"bd\")\n"}))
        };
        let mut wire = Vec::new();
        write_message(&mut wire, &serde_json::to_string(&req).unwrap()).unwrap();
        let line = read_message(&mut Cursor::new(wire), 1000).unwrap().unwrap();
        assert_eq!(parse_request(&line).unwrap(), req);
    }

    #[test]
    fn round_trip_response_through_frames() {
        let resp = Response::ok(Some(3), json!({"code": "a\nb"}));
        let mut wire = Vec::new();
        write_message(&mut wire, &encode_response(&resp)).unwrap();
        let line = read_message(&mut Cursor::new(wire), 1000).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), resp);
    }

    #[test]
    fn parses_the_example_request() {
        let r = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#).unwrap();
        assert_eq!(r, Request::new(1, "ping", json!({})));
    }

    #[test]
    fn unknown_method_parses() {
        let r = parse_request(r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#).unwrap();
        assert_eq!(r.method, "nope");
        assert_eq!(r.id, Some(9));
    }

    #[test]
    fn absent_or_null_params_become_an_empty_object() {
        let r = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"state"}"#).unwrap();
        assert_eq!(r.params, json!({}));
        let r =
            parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"state","params":null}"#).unwrap();
        assert_eq!(r.params, json!({}));
    }

    #[test]
    fn non_object_params_pass_through() {
        let r = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"state","params":[1]}"#).unwrap();
        assert_eq!(r.params, json!([1]));
    }

    #[test]
    fn timeout_ms_is_read() {
        let r = parse_request(
            r#"{"jsonrpc":"2.0","id":4,"method":"eval","params":{},"timeout_ms":30000}"#,
        )
        .unwrap();
        assert_eq!(r.timeout_ms, Some(30000));
        assert_eq!(r.params, json!({}));
    }

    fn rejected(line: &str) -> (Option<u64>, i64) {
        let (id, e) = parse_request(line).unwrap_err();
        (id, e.code)
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        let (id, e) = parse_request("not json").unwrap_err();
        assert_eq!(id, None);
        assert_eq!(e.code, codes::PARSE_ERROR);
        assert_eq!(e.message, "parse error: expected ident at line 1 column 2");
    }

    #[test]
    fn bad_ids_are_invalid_requests_with_null_id() {
        for line in [
            r#"{"jsonrpc":"2.0","id":"1","method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":-1,"method":"ping"}"#,
        ] {
            assert_eq!(rejected(line), (None, codes::INVALID_REQUEST), "{line}");
        }
    }

    #[test]
    fn notification_is_an_invalid_request() {
        let (id, e) = parse_request(r#"{"jsonrpc":"2.0","method":"ping"}"#).unwrap_err();
        assert_eq!(id, None);
        assert_eq!(e.code, codes::INVALID_REQUEST);
        assert!(e.message.contains("notifications"), "{}", e.message);
    }

    #[test]
    fn non_object_is_an_invalid_request() {
        assert_eq!(rejected("[1,2]"), (None, codes::INVALID_REQUEST));
        assert_eq!(rejected("42"), (None, codes::INVALID_REQUEST));
    }

    #[test]
    fn other_invalid_members_keep_the_id() {
        for line in [
            r#"{"jsonrpc":"1.0","id":3,"method":"ping"}"#,
            r#"{"id":3,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":3}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":7}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"ping","timeout_ms":"5"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"ping","timeout_ms":1.5}"#,
        ] {
            assert_eq!(rejected(line), (Some(3), codes::INVALID_REQUEST), "{line}");
        }
    }

    #[test]
    fn encode_response_has_no_newline_and_omits_the_absent_member() {
        // A single-member result, so the line does not depend on `Value`'s key order.
        let ok = encode_response(&Response::ok(Some(1), json!({"attached":true})));
        assert_eq!(ok, r#"{"jsonrpc":"2.0","id":1,"result":{"attached":true}}"#);
        let err = encode_response(&Response::err(
            Some(9),
            RpcError::new(codes::METHOD_NOT_FOUND, "unknown method 'nope'"),
        ));
        assert_eq!(
            err,
            r#"{"jsonrpc":"2.0","id":9,"error":{"code":-32601,"message":"unknown method 'nope'"}}"#
        );
        assert!(!ok.ends_with('\n') && !err.ends_with('\n'));
    }
}

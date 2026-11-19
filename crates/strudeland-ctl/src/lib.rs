//! The strudeland control socket: JSON-RPC 2.0 envelope types, line framing, error codes, the
//! typed params and results of every method, the socket path and a blocking client.
//!
//! No Tauri and no async runtime, so the app, the CLI and any other client can all share it.

pub mod client;
pub mod frame;
pub mod methods;
pub mod path;
pub mod protocol;

pub use client::{CallError, Client};
pub use frame::{
    FrameError, LineReader, encode_response, parse_request, read_message, write_message,
};
pub use methods::*;
pub use path::{
    APP_IDENTIFIER, PathError, SOCKET_ENV, socket_path, socket_path_from, socket_path_with,
};
pub use protocol::{MAX_LINE_BYTES, Request, Response, RpcError, codes};

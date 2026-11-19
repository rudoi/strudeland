//! Local byte pipes restricted to the current account.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::runtime::{Builder, Runtime};
use tokio::time::{Instant, sleep, timeout_at};
use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const PREFIX: &str = r"\\.\pipe\";

struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this pointer was allocated by a Windows API documented to use LocalFree.
            unsafe { LocalFree(self.0) };
        }
    }
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: sid is owned by the caller and stays valid for the call.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _allocation = Local(text.cast());
    // SAFETY: ConvertSidToStringSidW returned a NUL-terminated UTF-16 SID.
    let len = (0..).find(|&i| unsafe { *text.add(i) } == 0).unwrap();
    // SAFETY: len covers only the initialized UTF-16 string.
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(text, len)
    }))
}

fn user_sid() -> io::Result<String> {
    let mut token = null_mut();
    // SAFETY: the pseudo-process handle is valid and token is a writable out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken on success transfers this owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut len = 0;
    // SAFETY: sizing call with no result buffer.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // Pointer alignment accommodates TOKEN_USER as well as the variable-length SID.
    let mut storage = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
    // SAFETY: storage is aligned and holds at least len writable bytes.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: success initialized a TOKEN_USER whose SID pointer refers into storage.
    sid_string(unsafe { (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid })
}

pub fn default_path() -> io::Result<PathBuf> {
    Ok(PathBuf::from(format!(
        "{PREFIX}{}.{}.ctl",
        crate::APP_IDENTIFIER,
        user_sid()?
    )))
}

pub fn validate_path(path: &Path) -> io::Result<()> {
    let valid = path
        .to_str()
        .and_then(|p| p.strip_prefix(PREFIX))
        .is_some_and(|name| {
            !name.is_empty()
                && name.encode_utf16().count() <= 256 - PREFIX.len()
                && !name
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '\\' | '/'))
        });
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a local \\\\.\\pipe\\NAME endpoint",
        ))
    }
}

/// Creates the first exclusive instance or the next available instance of our server pipe.
pub fn listen(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
    validate_path(path)?;
    let sid = user_sid()?;
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut descriptor = null_mut();
    // SAFETY: input is NUL-terminated and descriptor is a writable out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let _allocation = Local(descriptor);
    let mut security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: both security and its descriptor live for the synchronous CreateNamedPipeW call.
    unsafe {
        ServerOptions::new()
            .reject_remote_clients(true)
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(path, (&raw mut security).cast())
    }
}

fn client_options() -> ClientOptions {
    let mut options = ClientOptions::new();
    // Identification permits inspecting the peer's user without impersonating its privileges.
    options.security_qos_flags(SECURITY_IDENTIFICATION);
    options
}

fn check_owner(pipe: &NamedPipeClient, expected: &str) -> io::Result<()> {
    let mut descriptor = null_mut();
    let mut owner = null_mut();
    // SAFETY: the pipe handle is valid, and only the selected owner fields require out pointers.
    let code = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    let _allocation = Local(descriptor);
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    if owner.is_null() || sid_string(owner)? != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unexpected control pipe owner",
        ));
    }
    Ok(())
}

pub(crate) struct Pipe {
    // Drop the registered handle before its runtime.
    client: Option<NamedPipeClient>,
    runtime: Runtime,
    deadline: Instant,
}

impl Pipe {
    pub(crate) fn connect(path: &Path, limit: Duration) -> io::Result<Self> {
        validate_path(path)?;
        let expected = user_sid()?;
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let deadline = Instant::now() + limit;
        let client = runtime.block_on(async {
            let options = client_options();
            // ClientOptions supplies GENERIC_READ | GENERIC_WRITE. GENERIC_READ includes
            // READ_CONTROL, required to verify the connected pipe's owner.
            loop {
                match options.open(path) {
                    Ok(pipe) => {
                        check_owner(&pipe, &expected)?;
                        return Ok(pipe);
                    }
                    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                        timeout_at(deadline, sleep(Duration::from_millis(20)))
                            .await
                            .map_err(timed_out)?;
                    }
                    Err(e) => return Err(e),
                }
            }
        })?;
        Ok(Self {
            client: Some(client),
            runtime,
            deadline,
        })
    }

    pub(crate) fn disconnect(&mut self) {
        if let Some(client) = self.client.take() {
            // SAFETY: the owned handle is still live. Null requests cancellation of every
            // pending operation on it. Tokio/Mio retain the kernel buffers until completion.
            unsafe { CancelIoEx(client.as_raw_handle(), std::ptr::null()) };
        }
    }

    pub(crate) fn set_deadline(&mut self, timeout: Duration) {
        self.deadline = Instant::now() + timeout;
    }

    fn client(client: &mut Option<NamedPipeClient>) -> io::Result<&mut NamedPipeClient> {
        client
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "control pipe is closed"))
    }

    fn finish(&mut self, result: io::Result<usize>) -> io::Result<usize> {
        if result.is_err() {
            self.disconnect();
        }
        result
    }
}

fn timed_out(_: tokio::time::error::Elapsed) -> io::Error {
    io::ErrorKind::TimedOut.into()
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let client = Self::client(&mut self.client)?;
        let result = self.runtime.block_on(async {
            timeout_at(self.deadline, client.read(buffer))
                .await
                .map_err(timed_out)?
        });
        self.finish(result)
    }
}

impl Write for Pipe {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let client = Self::client(&mut self.client)?;
        let result = self.runtime.block_on(async {
            timeout_at(self.deadline, client.write(buffer))
                .await
                .map_err(timed_out)?
        });
        self.finish(result)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Tokio writes into the pipe directly. FlushFileBuffers would wait for the peer to read.
        Ok(())
    }
}

/// Detects a same-account listener when the first-instance create reports a collision.
/// The caller is already entered into the app runtime.
pub fn owned_existing(path: &Path) -> bool {
    let Ok(expected) = user_sid() else {
        return false;
    };
    let options = client_options();
    for _ in 0..4 {
        match options.open(path) {
            Ok(pipe) => return check_owner(&pipe, &expected).is_ok(),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, Client};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn endpoint() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            "{PREFIX}strudeland-test-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn local_endpoints_only() {
        assert!(validate_path(&default_path().unwrap()).is_ok());
        assert!(validate_path(&endpoint()).is_ok());
        assert!(validate_path(Path::new(r"\\server\pipe\remote")).is_err());
        assert!(validate_path(Path::new(r"\\.\pipe\bad\component")).is_err());
    }

    #[test]
    fn a_stalled_peer_times_out_and_the_partial_pipe_is_discarded() {
        let path = endpoint();
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let _server = {
            let _entered = runtime.enter();
            listen(&path, true).unwrap()
        };
        let mut client = Pipe::connect(&path, Duration::from_secs(1)).unwrap();
        client.set_deadline(Duration::from_millis(100));
        let result = client
            .write_all(&vec![b'x'; 256 * 1024])
            .and_then(|()| client.write_all(b"\n"));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            client.write(b"retry").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn authenticated_local_round_trip() {
        let path = endpoint();
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let server = {
            let _entered = runtime.enter();
            let server = listen(&path, true).unwrap();
            assert!(listen(&path, true).is_err());
            server
        };
        let handle = std::thread::spawn(move || {
            runtime.block_on(async {
                server.connect().await.unwrap();
                let mut server = tokio::io::BufReader::new(server);
                let mut line = String::new();
                tokio::io::AsyncBufReadExt::read_line(&mut server, &mut line)
                    .await
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], "ping");
                server
                    .get_mut()
                    .write_all(
                        format!(
                            "{}\n",
                            json!({"jsonrpc":"2.0","id":request["id"],"result":"ok"})
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                // Keep the server alive until the client has received the response.
                assert_eq!(server.get_mut().read(&mut [0]).await.unwrap(), 0);
            });
        });
        let mut client = Client::connect(&path, Duration::from_secs(1)).unwrap();
        assert_eq!(client.call_raw("ping", json!({})).unwrap(), json!("ok"));
        drop(client);
        handle.join().unwrap();
        assert!(matches!(
            Client::connect(&path, Duration::from_secs(1)),
            Err(CallError::NotRunning { .. })
        ));
    }
}

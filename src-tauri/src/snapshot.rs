//! Native screenshots of the main window webview, written as PNG.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use strudeland_ctl::ScreenshotResult;
use tauri::AppHandle;
use tokio::sync::oneshot;

use crate::control::{SnapshotReceiver, Snapshotter};

/// Snapshots the `main` webview of this app.
pub struct TauriSnapshotter(pub AppHandle);

type Outcome = Result<ScreenshotResult, String>;

/// A sender shared between callbacks that may run more than once (the completion block is `Fn`):
/// the first `send` delivers, later ones are ignored.
#[derive(Clone)]
struct SendOnce(Arc<Mutex<Option<oneshot::Sender<Outcome>>>>);

impl SendOnce {
    fn new() -> (Self, SnapshotReceiver) {
        let (tx, rx) = oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    /// True once nobody waits for the outcome: it was already sent, or the socket side timed out
    /// and dropped the receiver. Late work checks this so a timed-out request writes nothing.
    fn abandoned(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(oneshot::Sender::is_closed)
    }

    fn send(&self, outcome: Outcome) {
        let tx = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(tx) = tx {
            // The socket side may have timed out and dropped the receiver.
            let _ = tx.send(outcome);
        }
    }
}

const EMPTY: &str = "the snapshot was empty (is the window hidden or minimized?)";

fn write_png(
    path: &Path,
    png: &[u8],
    width: isize,
    height: isize,
    publish: impl FnOnce() -> bool,
) -> Outcome {
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(EMPTY.into());
    };
    if png.is_empty() || width == 0 || height == 0 {
        return Err(EMPTY.into());
    }
    let write = || -> io::Result<()> {
        let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        // NamedTempFile reserves a unique sibling and uses private permissions on Unix.
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(png)?;
        if !publish() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "the snapshot was cancelled",
            ));
        }
        temp.persist(path).map_err(|e| e.error)?;
        Ok(())
    };
    write().map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(ScreenshotResult {
        path: path.to_string_lossy().into_owned(),
        width,
        height,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
impl Snapshotter for TauriSnapshotter {
    /// No snapshot is started.
    fn snapshot(&self, _path: PathBuf) -> Result<SnapshotReceiver, String> {
        Err("screenshots are unavailable on this platform".into())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Snapshotter for TauriSnapshotter {
    fn snapshot(&self, path: PathBuf) -> Result<SnapshotReceiver, String> {
        use tauri::Manager;

        let window = self
            .0
            .get_webview_window("main")
            .ok_or_else(|| "no main window".to_string())?;
        let (tx, rx) = SendOnce::new();
        let on_main = tx.clone();
        // Queued to the main thread; this thread (a tokio worker) does not wait for it.
        if let Err(e) = window.with_webview(move |pw| {
            if on_main.abandoned() {
                return;
            }
            #[cfg(target_os = "macos")]
            macos::start(pw.inner(), path, on_main);
            #[cfg(target_os = "linux")]
            linux::start(pw.inner(), path, on_main);
            #[cfg(windows)]
            windows_webview::start(pw.controller(), path, on_main);
        }) {
            tx.send(Err(format!("cannot reach the webview: {e}")));
        }
        Ok(rx)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    use block2::RcBlock;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSData, NSDictionary, NSError};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

    use super::{EMPTY, Outcome, SendOnce, write_png};

    /// Runs on the main thread (inside `with_webview`): starts the snapshot. The completion block
    /// also runs on the main thread, later; it only copies the image bytes and leaves the encode
    /// and the write to a background thread. Nothing here blocks.
    pub(super) fn start(webview: *mut c_void, path: PathBuf, tx: SendOnce) {
        let Some(mtm) = MainThreadMarker::new() else {
            tx.send(Err("the snapshot did not start on the main thread".into()));
            return;
        };
        // SAFETY: on macOS `PlatformWebview::inner()` is the window's live `WKWebView` (wry's
        // subclass of it), valid while this main-thread callback runs. Retaining it keeps it alive
        // until the call below returns; WebKit retains what it needs past that.
        let Some(webview) = (unsafe { Retained::retain(webview.cast::<WKWebView>()) }) else {
            tx.send(Err("the webview is gone".into()));
            return;
        };
        // SAFETY: a plain `new` on the main thread (the marker proves it).
        let config = unsafe { WKSnapshotConfiguration::new(mtm) };
        let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            // SAFETY: WebKit passes either null or a valid object, borrowed for this call.
            let (image, error) = unsafe { (image.as_ref(), error.as_ref()) };
            finish(image, error, path.clone(), tx.clone());
        });
        // SAFETY: called on the main thread with a valid config; WebKit copies the block and
        // calls it once, on the main thread, with the signature `(NSImage?, NSError?)`.
        unsafe {
            webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
        }
    }

    /// Runs on the main thread, in the completion block: copies the snapshot's TIFF bytes (cheap)
    /// and hands the PNG encode and the file write to a background thread, which sends the
    /// outcome. Failures before the hand-off are sent from here.
    fn finish(image: Option<&NSImage>, error: Option<&NSError>, path: PathBuf, tx: SendOnce) {
        let Some(image) = image else {
            tx.send(Err(error
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "no image".into())));
            return;
        };
        // Drain the TIFF temporaries here rather than in the run loop's pool.
        let tiff = autoreleasepool(|_| image.TIFFRepresentation().map(|tiff| tiff.to_vec()));
        let Some(tiff) = tiff.filter(|tiff| !tiff.is_empty()) else {
            tx.send(Err(EMPTY.into()));
            return;
        };
        std::thread::spawn(move || {
            // The request may have timed out while WebKit was drawing; then write nothing.
            if !tx.abandoned() {
                let result = encode(&tiff, &path, &tx);
                tx.send(result);
            }
        });
    }

    /// Runs on a background thread: decodes the TIFF, encodes it as PNG and writes it to `path`.
    /// `NSData` and `NSBitmapImageRep` are not main-thread-only (no `MainThreadMarker` needed),
    /// and every object here is created and dropped on this thread.
    fn encode(tiff: &[u8], path: &Path, tx: &SendOnce) -> Outcome {
        // This thread has no run loop pool; drain the encoding temporaries here.
        autoreleasepool(|_| {
            let tiff = NSData::with_bytes(tiff);
            let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                .ok_or_else(|| "cannot read the snapshot image".to_string())?;
            let properties = NSDictionary::new();
            // SAFETY: an empty dictionary is a valid properties argument.
            let png = unsafe {
                rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
            }
            .ok_or_else(|| "cannot encode the snapshot as PNG".to_string())?;
            // SAFETY: `png` is a fresh result owned only by this thread; nothing mutates it while
            // the slice is borrowed.
            let bytes = unsafe { png.as_bytes_unchecked() };
            write_png(path, bytes, rep.pixelsWide(), rep.pixelsHigh(), || {
                !tx.abandoned()
            })
        })
    }
}

#[cfg(any(target_os = "linux", windows, test))]
fn dimensions(png: &[u8]) -> Option<(isize, isize)> {
    let header = png.get(..24)?;
    if header.get(..16)? != b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width as isize, height as isize))
}

#[cfg(any(target_os = "linux", windows))]
fn finish_png(path: PathBuf, png: Vec<u8>, tx: SendOnce) {
    std::thread::spawn(move || {
        if !tx.abandoned() {
            let result =
                dimensions(&png)
                    .ok_or_else(|| EMPTY.to_string())
                    .and_then(|(width, height)| {
                        write_png(&path, &png, width, height, || !tx.abandoned())
                    });
            tx.send(result);
        }
    });
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{SendOnce, finish_png};
    use std::path::PathBuf;
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebView, WebViewExt, gio};

    pub(super) fn start(webview: WebView, path: PathBuf, tx: SendOnce) {
        webview.snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::empty(),
            None::<&gio::Cancellable>,
            move |result| {
                if tx.abandoned() {
                    return;
                }
                let png = result.map_err(|e| e.to_string()).and_then(|surface| {
                    let mut png = Vec::new();
                    surface.write_to_png(&mut png).map_err(|e| e.to_string())?;
                    Ok(png)
                });
                match png {
                    Ok(png) => finish_png(path, png, tx),
                    Err(e) => tx.send(Err(e)),
                }
            },
        );
    }
}

#[cfg(windows)]
mod windows_webview {
    use super::{SendOnce, finish_png};
    use std::path::PathBuf;
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, ICoreWebView2Controller,
    };
    use windows::Win32::System::Com::{
        IStream, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
    };

    pub(super) fn start(controller: ICoreWebView2Controller, path: PathBuf, tx: SendOnce) {
        let completed = tx.clone();
        // SAFETY: Tauri invokes this on the webview apartment thread. The controller, stream
        // and callback are valid for the call, and the callback retains the stream until done.
        let result = unsafe {
            controller.CoreWebView2().and_then(|webview| {
                let stream = CreateStreamOnHGlobal(Default::default(), true)?;
                let snapshot = stream.clone();
                let callback = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                    if !completed.abandoned() {
                        match result.and_then(|()| bytes(&snapshot)) {
                            Ok(png) => finish_png(path, png, completed),
                            Err(e) => completed.send(Err(e.to_string())),
                        }
                    }
                    Ok(())
                }));
                webview.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &callback,
                )
            })
        };
        if let Err(e) = result {
            tx.send(Err(e.to_string()));
        }
    }

    fn bytes(stream: &IStream) -> windows::core::Result<Vec<u8>> {
        // SAFETY: this is the completed snapshot stream on the creating apartment.
        unsafe { stream.Seek(0, STREAM_SEEK_SET, None)? };
        let mut png = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let mut read = 0;
            // SAFETY: chunk is valid for writes of its length and read is a writable out pointer.
            unsafe {
                stream
                    .Read(
                        chunk.as_mut_ptr().cast(),
                        chunk.len() as u32,
                        Some(&mut read),
                    )
                    .ok()?
            };
            png.extend_from_slice(&chunk[..read as usize]);
            if (read as usize) < chunk.len() {
                return Ok(png);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_are_read_from_the_png_pixel_header() {
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x06\x40\x00\x00\x04\xb0";
        assert_eq!(dimensions(png), Some((1600, 1200)));
        assert_eq!(dimensions(&png[..23]), None);
        assert_eq!(dimensions(&png[1..]), None);
    }

    #[test]
    fn write_png_writes_and_describes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        std::fs::write(&path, "old").unwrap();
        let r = write_png(&path, b"png", 1600, 1200, || true).unwrap();
        assert_eq!(
            r,
            ScreenshotResult {
                path: path.to_string_lossy().into_owned(),
                width: 1600,
                height: 1200,
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"png", "overwritten");
    }

    #[test]
    fn cancelled_snapshot_keeps_the_existing_file_and_removes_only_its_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        std::fs::write(&path, b"old").unwrap();

        assert!(write_png(&path, b"png", 3, 2, || false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_are_private_even_in_a_shared_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        write_png(&path, b"png", 3, 2, || true).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn an_empty_snapshot_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        for (png, w, h) in [
            (&b""[..], 10, 10),
            (&b"png"[..], 0, 10),
            (&b"png"[..], 10, 0),
            (&b"png"[..], -1, 10),
        ] {
            assert_eq!(write_png(&path, png, w, h, || true), Err(EMPTY.to_string()));
        }
        assert!(!path.exists());
    }

    #[test]
    fn a_write_error_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/s.png");
        let err = write_png(&path, b"png", 1, 1, || true).unwrap_err();
        assert!(
            err.starts_with(&format!("cannot write {}: ", path.display())),
            "{err}"
        );
    }

    #[tokio::test]
    async fn send_once_delivers_the_first_outcome_only() {
        let (tx, rx) = SendOnce::new();
        let other = tx.clone();
        tx.send(Err("first".into()));
        other.send(Err("second".into()));
        assert_eq!(rx.await.unwrap(), Err("first".to_string()));
    }

    #[tokio::test]
    async fn dropping_every_send_once_cancels() {
        let (tx, rx) = SendOnce::new();
        drop(tx.clone());
        drop(tx);
        assert!(rx.await.is_err());
    }

    #[test]
    fn send_once_is_abandoned_after_a_send_or_a_dropped_receiver() {
        let (tx, rx) = SendOnce::new();
        assert!(!tx.abandoned());
        tx.send(Err("done".into()));
        assert!(tx.abandoned());
        drop(rx);

        let (tx, rx) = SendOnce::new();
        drop(rx);
        assert!(tx.abandoned());
    }

    #[test]
    fn write_png_leaves_no_temporary_file_and_keeps_the_old_file_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        write_png(&path, b"png", 2, 2, || true).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("s.png")]);

        // Renaming onto a directory fails; the directory and no temp file remain.
        let sub = dir.path().join("taken");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("keep"), "x").unwrap();
        assert!(write_png(&sub, b"png", 2, 2, || true).is_err());
        assert!(sub.join("keep").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn send_once_after_the_receiver_is_gone_is_harmless() {
        let (tx, rx) = SendOnce::new();
        drop(rx);
        tx.send(Err("late".into()));
    }
}

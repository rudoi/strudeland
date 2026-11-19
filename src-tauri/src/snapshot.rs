//! The native screenshot behind [`Snapshotter`]: a WKWebView snapshot of the `main` window's web
//! content, written as a PNG. macOS only; elsewhere `snapshot` fails at once.

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

/// Writes the encoded PNG to `path` (overwriting) and describes it. An empty image is an error and
/// writes nothing.
/// Writes to a temporary file next to `path`, then renames it over `path`: a reader never sees a
/// partial PNG, and a failed write leaves an existing file alone.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_png(path: &Path, png: &[u8], width: isize, height: isize) -> Outcome {
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(EMPTY.into());
    };
    if png.is_empty() || width == 0 || height == 0 {
        return Err(EMPTY.into());
    }
    write_atomic(path, png).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(ScreenshotResult {
        path: path.to_string_lossy().into_owned(),
        width,
        height,
    })
}

#[cfg(not(target_os = "macos"))]
impl Snapshotter for TauriSnapshotter {
    /// No snapshot is started.
    fn snapshot(&self, _path: PathBuf) -> Result<SnapshotReceiver, String> {
        Err("screenshots need macOS".into())
    }
}

#[cfg(target_os = "macos")]
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
        if let Err(e) = window.with_webview(move |pw| macos::start(pw.inner(), path, on_main)) {
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
                tx.send(encode(&tiff, &path));
            }
        });
    }

    /// Runs on a background thread: decodes the TIFF, encodes it as PNG and writes it to `path`.
    /// `NSData` and `NSBitmapImageRep` are not main-thread-only (no `MainThreadMarker` needed),
    /// and every object here is created and dropped on this thread.
    fn encode(tiff: &[u8], path: &Path) -> Outcome {
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
            write_png(path, bytes, rep.pixelsWide(), rep.pixelsHigh())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_png_writes_and_describes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        std::fs::write(&path, "old").unwrap();
        let r = write_png(&path, b"png", 1600, 1200).unwrap();
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
    fn an_empty_snapshot_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.png");
        for (png, w, h) in [
            (&b""[..], 10, 10),
            (&b"png"[..], 0, 10),
            (&b"png"[..], 10, 0),
            (&b"png"[..], -1, 10),
        ] {
            assert_eq!(write_png(&path, png, w, h), Err(EMPTY.to_string()));
        }
        assert!(!path.exists());
    }

    #[test]
    fn a_write_error_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/s.png");
        let err = write_png(&path, b"png", 1, 1).unwrap_err();
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
        write_png(&path, b"png", 2, 2).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("s.png")]);

        // Renaming onto a directory fails; the directory and no temp file remain.
        let sub = dir.path().join("taken");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("keep"), "x").unwrap();
        assert!(write_png(&sub, b"png", 2, 2).is_err());
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

use std::sync::{Mutex, MutexGuard};

#[cfg(target_os = "macos")]
use tauri::{AppHandle, Emitter};
use tauri::{Manager, WebviewWindow};
use tokio::sync::OwnedMutexGuard;

use crate::export_audio::ExportState;

#[cfg(target_os = "macos")]
const REQUEST_EVENT: &str = "app://quit-requested";

#[derive(Default)]
pub struct QuitState(Mutex<QuitRequest>);

#[derive(Default)]
struct QuitRequest {
    #[cfg(any(target_os = "macos", test))]
    next_id: u32,
    ready: bool,
    pending: Option<u32>,
    exit_permit: Option<OwnedMutexGuard<()>>,
}

impl QuitState {
    fn lock(&self) -> MutexGuard<'_, QuitRequest> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn detach(&self) {
        self.set_ready(false);
    }

    fn set_ready(&self, ready: bool) {
        let mut state = self.lock();
        state.pending = None;
        state.ready = ready;
    }

    #[cfg(any(target_os = "macos", test))]
    fn request(&self, send: impl FnOnce(u32) -> tauri::Result<()>) {
        let id = {
            let mut state = self.lock();
            if !state.ready || state.pending.is_some() || state.exit_permit.is_some() {
                return;
            }
            let id = state.next_id;
            state.next_id = id.wrapping_add(1);
            state.pending = Some(id);
            id
        };
        if let Err(error) = send(id) {
            let mut state = self.lock();
            if state.pending == Some(id) {
                state.pending = None;
            }
            eprintln!("could not deliver application Quit request: {error}");
        }
    }

    fn reply(&self, id: u32, approved: bool, exports: &ExportState) -> Result<bool, &'static str> {
        let mut state = self.lock();
        if !state.ready || state.pending != Some(id) {
            return Ok(false);
        }
        state.pending = None;
        if !approved {
            return Ok(false);
        }
        state.allow_exit(exports)?;
        Ok(true)
    }
}

impl QuitRequest {
    fn allow_exit(&mut self, exports: &ExportState) -> Result<(), &'static str> {
        let permit = exports
            .reserve()
            .map_err(|_| "an audio export is still running; try quitting after it finishes")?;
        self.exit_permit = Some(permit);
        Ok(())
    }
}

#[tauri::command]
pub fn app_quit_listen(window: WebviewWindow) -> Result<(), &'static str> {
    if window.label() != "main" {
        return Err("only the main window can handle application Quit");
    }
    window.state::<QuitState>().set_ready(true);
    Ok(())
}

#[tauri::command]
pub fn app_quit_reply(window: WebviewWindow, id: u32, approved: bool) -> Result<(), &'static str> {
    if window.label() != "main" {
        return Err("only the main window can handle application Quit");
    }
    if window
        .state::<QuitState>()
        .reply(id, approved, &window.state::<ExportState>())?
    {
        window.app_handle().exit(0);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn request(app: &AppHandle) {
    let quit = app.state::<QuitState>();
    if app.get_webview_window("main").is_some() {
        quit.request(|id| app.emit_to("main", REQUEST_EVENT, id));
    } else {
        let approved = quit.lock().allow_exit(&app.state::<ExportState>()).is_ok();
        if approved {
            app.exit(0);
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) mod macos {
    use std::sync::OnceLock;

    use objc2::runtime::{AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, ffi, sel};
    use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub fn install(app: &AppHandle) -> Result<(), &'static str> {
        let main = MainThreadMarker::new().ok_or("install Quit handler on the main thread")?;
        let application = NSApplication::sharedApplication(main);
        let delegate = application
            .delegate()
            .ok_or("missing AppKit app delegate")?;
        let object: &AnyObject = delegate.as_ref();
        let class = object.class();
        let selector = sel!(applicationShouldTerminate:);
        if class.name() != c"TaoAppDelegateParent" || class.instance_method(selector).is_some() {
            return Err("unexpected AppKit delegate for application Quit");
        }
        APP.set(app.clone()).map_err(|_| "duplicate Quit handler")?;

        // SAFETY: AppKit expects an NSUInteger reply and passes self, the selector, and the
        // application object. Add only the missing optional delegate method to Tao's class.
        let added = unsafe {
            let callback: extern "C-unwind" fn(_, _, _) -> _ = should_terminate;
            ffi::class_addMethod(
                std::ptr::from_ref(class).cast_mut(),
                selector,
                std::mem::transmute::<_, Imp>(callback),
                c"Q@:@".as_ptr(),
            )
        };
        if !added.as_bool() {
            return Err("could not register AppKit Quit handler");
        }
        Ok(())
    }

    extern "C-unwind" fn should_terminate(
        _: &AnyObject,
        _: Sel,
        _: &NSApplication,
    ) -> NSApplicationTerminateReply {
        if let Some(app) = APP.get() {
            super::request(app);
        }
        // Let the ordinary event loop deliver IPC and export progress. The approved reply
        // exits through Tauri, which runs the app's existing shutdown path.
        NSApplicationTerminateReply::TerminateCancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_cancelled_and_stale_quit_requests() {
        let quit = QuitState::default();
        let exports = ExportState::default();
        let ids = Mutex::new(Vec::new());
        let send = |id| {
            ids.lock().unwrap().push(id);
            Ok(())
        };
        quit.set_ready(true);
        quit.request(send);
        quit.request(send);
        assert_eq!(*ids.lock().unwrap(), [0]);
        assert!(!quit.reply(0, false, &exports).unwrap());
        quit.request(send);
        assert!(!quit.reply(0, true, &exports).unwrap());
        assert_eq!(*ids.lock().unwrap(), [0, 1]);
        quit.detach();
        assert!(!quit.reply(1, true, &exports).unwrap());
        quit.request(send);
        assert_eq!(*ids.lock().unwrap(), [0, 1]);
    }

    #[test]
    fn active_export_blocks_exit_and_approved_exit_reserves_export() {
        let quit = QuitState::default();
        let exports = ExportState::default();
        quit.set_ready(true);
        let job = exports.reserve().unwrap();
        quit.request(|_| Ok(()));
        assert!(quit.reply(0, true, &exports).is_err());
        drop(job);
        quit.request(|_| Ok(()));
        assert!(quit.reply(1, true, &exports).unwrap());
        assert!(exports.reserve().is_err());
        assert!(!quit.reply(1, true, &exports).unwrap());
    }

    #[test]
    fn a_delivery_error_releases_the_pending_request() {
        let quit = QuitState::default();
        quit.set_ready(true);
        quit.request(|_| Err(tauri::Error::WindowNotFound));
        quit.request(|_| Ok(()));
        assert_eq!(quit.lock().pending, Some(1));
    }
}

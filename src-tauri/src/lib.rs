mod app_quit;
#[cfg(target_os = "macos")]
mod cli_install;
mod command_error;
mod commands;
mod control;
mod export_audio;
mod instruments;
mod lifecycle;
mod piano_roll;
mod playback;
mod project;
mod project_menu;
mod recording;
mod snapshot;
mod watcher;

use std::sync::Arc;

use tauri::Manager;
use tauri::webview::PageLoadEvent;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bridge = Arc::new(control::Bridge::new());
    tauri::Builder::default()
        .menu(project_menu::build)
        .on_menu_event(project_menu::dispatch)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(project::ProjectState::default())
        .manage(project::AppDataState::locate())
        .manage(app_quit::QuitState::default())
        .manage(export_audio::ExportState::default())
        .manage(playback::PlaybackState::default())
        .manage(piano_roll::PianoRollState::default())
        .manage(Arc::clone(&bridge))
        // A reload or HMR of the main webview: requests answer `not ready` until it attaches again.
        .on_page_load(|webview, payload| {
            if webview.label() == "main" && matches!(payload.event(), PageLoadEvent::Started) {
                let recorder = webview.try_state::<recording::RecorderState>();
                let session = webview.try_state::<playback::PlaybackState>();
                lifecycle::on_reload(
                    &webview.state::<app_quit::QuitState>(),
                    &webview.state::<Arc<control::Bridge>>(),
                    recorder.as_ref().map(|r| &*r.0),
                    session.as_deref(),
                );
            }
        })
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app_quit::macos::install(app.handle())?;
            #[cfg(target_os = "macos")]
            cli_install::register(app.handle())?;
            let recorder = recording::system_state(app.handle());
            let core = Arc::clone(&recorder.0);
            app.state::<project::ProjectState>()
                .on_open(move |opened| recording::repair_in_background(&core, opened));
            app.manage(recorder);
            control::start(app.handle(), bridge);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_quit::app_quit_listen,
            app_quit::app_quit_reply,
            commands::app_info,
            project_menu::export_audio_set_available,
            export_audio::export_audio_pick_folder,
            export_audio::export_audio_preview,
            export_audio::export_audio_start,
            project::project_create,
            project::project_open,
            project::project_info,
            project::project_instruments,
            instruments::instrument_library_entries,
            instruments::project_instrument_entries,
            instruments::instrument_definition,
            instruments::instrument_preview,
            piano_roll::piano_roll_inspect,
            instruments::project_write_instrument_pin,
            project::project_last,
            project::recent_list,
            project::prefs_get,
            project::prefs_set,
            project::song_list,
            project::song_read,
            project::song_save,
            project::song_create,
            project::song_rename,
            project::song_delete,
            project::project_root_of,
            recording::recording_status,
            recording::recording_devices,
            recording::recording_arm,
            recording::recording_disarm,
            recording::recording_start,
            recording::recording_stop,
            recording::recording_list,
            recording::recording_rename,
            recording::recording_delete,
            recording::recording_read,
            recording::host_now_ns,
            playback::playback_start,
            playback::playback_schedule,
            playback::playback_stop,
            playback::playback_seed,
            playback::playback_limiter,
            control::control_attach,
            control::control_reply,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the strudeland application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let session = app.try_state::<playback::PlaybackState>();
                let recorder = app.try_state::<recording::RecorderState>();
                lifecycle::on_exit(
                    session.as_deref(),
                    recorder.as_ref().map(|r| &*r.0),
                    &app.state::<Arc<control::Bridge>>(),
                );
            }
        });
}

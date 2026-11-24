//! Project folders for strudeland: the on-disk layout, `project.json`, song discovery,
//! path-scoped reads and atomic writes, voice memos (`recordings/`), file templates and the recent projects list.
//!
//! This crate has no Tauri dependency so the app and the CLI can share it.

mod error;
mod instruments;
mod prefs;
mod project;
mod recents;
mod recordings;
mod template;

pub use error::ProjectError;
pub use instruments::{
    INSTRUMENTS_DIR, InstrumentDefinition, InstrumentEntry, InstrumentLibrary, InstrumentSnapshot,
    instrument_dir_entries, instrument_file_candidate, validate_instrument_snapshot,
};
pub use prefs::{PREFS_FILE, Prefs};
pub use project::{
    MANIFEST_FILE, Manifest, Project, ProjectInfo, RenderSet, SCHEMA, SONG_EXT, SONGS_DIR,
    SongEntry, atomic_create_bytes, atomic_write_bytes, is_song_file, is_song_file_name,
    list_songs, render_exists, render_filename, song_rel_path,
};
pub use recents::{MAX_RECENTS, RecentProject, Recents};
pub use recordings::{
    ClockOffsetSample, HostNs, OffsetNs, RECORDINGS_DIR, RecordingEntry, RecoveredRecording,
    RepairReport, RepairSkip, SIDECAR_SCHEMA, Sidecar, SidecarApp, SidecarBacking,
    SidecarClockOffsets, SidecarInput, SidecarOutput, SidecarTempo, SidecarTiming, WavInfo,
    format_rfc3339_utc, is_recording_file_name, read_wav_header, recording_dir_entries,
};
pub use template::{MAIN_SONG, new_song};

// Windows file symlinks require Developer Mode or the symlink privilege. Exercise them when
// available; the unprivileged directory-junction fixture runs on every native Windows build.
#[cfg(test)]
macro_rules! test_symlink {
    ($target:expr, $link:expr $(,)?) => {{
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink($target, $link);
        #[cfg(windows)]
        let result = {
            let target = $target;
            let link = $link;
            let result = if std::path::Path::new(&target).is_dir() {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            };
            if result
                .as_ref()
                .is_err_and(|e| e.raw_os_error() == Some(1314))
            {
                eprintln!("Windows symlink privilege unavailable");
                return;
            }
            result
        };
        result
    }};
}
#[cfg(test)]
pub(crate) use test_symlink;

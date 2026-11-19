//! Project folders for strudeland: the on-disk layout, `project.json`, song discovery,
//! path-scoped reads and atomic writes, voice memos (`recordings/`), file templates and the recent projects list.
//!
//! This crate has no Tauri dependency so the app and the CLI can share it.

mod error;
mod prefs;
mod project;
mod recents;
mod recordings;
mod template;

pub use error::ProjectError;
pub use prefs::{PREFS_FILE, Prefs};
pub use project::{
    MANIFEST_FILE, Manifest, Project, ProjectInfo, SCHEMA, SONG_EXT, SONGS_DIR, SongEntry,
    atomic_write_bytes, is_song_file, is_song_file_name, song_rel_path,
};
pub use recents::{MAX_RECENTS, RecentProject, Recents};
pub use recordings::{
    ClockOffsetSample, HostNs, OffsetNs, RECORDINGS_DIR, RecordingEntry, RecoveredRecording,
    RepairReport, RepairSkip, SIDECAR_SCHEMA, Sidecar, SidecarApp, SidecarBacking,
    SidecarClockOffsets, SidecarInput, SidecarOutput, SidecarTempo, SidecarTiming, WavInfo,
    format_rfc3339_utc, is_recording_file_name, read_wav_header,
};
pub use template::{MAIN_SONG, new_song};

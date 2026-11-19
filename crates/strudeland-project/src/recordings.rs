//! Voice memos in a project's `recordings/`: naming, listing, rename, delete, the sidecar
//! (schema 1: types, read, write, validation) and the repair of `.partial` files a crash left.
//!
//! A memo is `recordings/<stem>.wav` plus an optional `recordings/<stem>.json` sidecar. During a
//! take the audio goes to `recordings/.<stem>.wav.partial`, which is never listed.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ProjectError;
use crate::project::{
    MAX_STEM_LEN, Placed, Project, atomic_create_bytes, atomic_write_bytes, check_dir,
    has_exact_name, is_case_only_rename, is_new_stem, is_valid_stem, link_noclobber,
    move_noreplace, redirects,
};

/// The folder under the project root that holds the memos.
pub const RECORDINGS_DIR: &str = "recordings";
/// The sidecar schema this crate reads and writes.
pub const SIDECAR_SCHEMA: u32 = 1;

const WAV_EXT: &str = ".wav";
const JSON_EXT: &str = ".json";
const PARTIAL_EXT: &str = ".wav.partial";
/// The largest sidecar `read_sidecar` reads.
const MAX_SIDECAR_BYTES: u64 = 16 * 1024 * 1024;
/// At most this many chunks are walked before `data`.
const MAX_CHUNKS: usize = 32;
/// 12 bytes `RIFF`/`WAVE`, a 24-byte `fmt ` chunk and an 8-byte `data` header.
const MIN_WAV_LEN: u64 = 44;
/// A partial modified more recently than this is skipped by the repair as in use.
const IN_USE_WINDOW: Duration = Duration::from_secs(5);
/// The highest numbered default name and recovered name tried.
const MAX_DEFAULT_N: u32 = 9999;
const MAX_RECOVERED_N: u32 = 99;

// --- host-time strings ---

/// Nanoseconds of host time, serialised as a decimal string (JSON numbers lose precision above
/// 2^53). Deserialises only from a string of ASCII digits that fits a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostNs(pub u64);

/// A signed nanosecond offset, serialised as a decimal string with an optional leading `-`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OffsetNs(pub i64);

impl Serialize for HostNs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for HostNs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_digits(&text)
            .and_then(|_| text.parse::<u64>().ok())
            .map(HostNs)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "expected a decimal string of host nanoseconds, got {text:?}"
                ))
            })
    }
}

impl Serialize for OffsetNs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for OffsetNs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let digits = text.strip_prefix('-').unwrap_or(&text);
        parse_digits(digits)
            .and_then(|_| text.parse::<i64>().ok())
            .map(OffsetNs)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "expected a signed decimal string of nanoseconds, got {text:?}"
                ))
            })
    }
}

/// `Some(())` when `text` is one or more ASCII digits.
fn parse_digits(text: &str) -> Option<()> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then_some(())
}

// --- sidecar schema 1 ---

/// The memo sidecar, `recordings/<stem>.json`. camelCase JSON; unknown keys are ignored on read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub schema: u32,
    /// RFC 3339 UTC to the second, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created_at: String,
    pub app: SidecarApp,
    pub input: SidecarInput,
    pub output: SidecarOutput,
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
    pub timing: SidecarTiming,
    pub xruns: u32,
    /// The take was salvaged by the repair: its tempo is a placeholder and it has no timing.
    pub recovered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarApp {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarInput {
    pub device_id: String,
    pub device_name: String,
    /// The 0-based index of the captured channel.
    pub channel: u32,
    /// The device's channel count.
    pub channels: u32,
    /// The WAV's sample rate.
    pub sample_rate: u32,
}

/// From the webview, in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarOutput {
    pub base_latency: f64,
    pub output_latency: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarTempo {
    /// Quarter notes per minute.
    pub bpm: f64,
    /// `N/D`, for example `6/8`.
    pub meter: String,
    pub cps: f64,
    /// Quarter notes per bar (6/8 gives 3).
    pub beats_per_bar: f64,
    /// A `setcps` in the code overrode `meta`.
    pub tempo_override: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarBacking {
    pub song: bool,
    pub metronome: bool,
    pub count_in: bool,
    /// Project-relative, forward slashes (`songs/main.js`).
    pub song_path: Option<String>,
    pub start_section: Option<String>,
    pub loop_section: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarTiming {
    /// The fractional 0-based sample index of bar 1's downbeat.
    pub bar1_frame: Option<f64>,
    /// The host time at which bar 1's downbeat leaves the speakers.
    pub bar1_host_ns: Option<HostNs>,
    pub clock_offset_ns: SidecarClockOffsets,
    /// `[frame, hostNs]` pairs from the capture callback.
    pub anchors: Vec<(u64, HostNs)>,
    /// `[contextTime, hostNs]` pairs from `getOutputTimestamp()`.
    pub output_stamps: Vec<(f64, HostNs)>,
}

/// The webview-to-host clock offsets measured before the backing starts and at stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarClockOffsets {
    pub start: Option<ClockOffsetSample>,
    pub end: Option<ClockOffsetSample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockOffsetSample {
    /// `hostNs - performance.now()` as nanoseconds.
    pub offset_ns: OffsetNs,
    /// The round trip of the picked sample (the error bound is half of it).
    pub rtt_ns: HostNs,
}

impl SidecarTempo {
    /// The tempo a sidecar records when the real one is unknown or invalid: 120 bpm in 4/4.
    pub fn placeholder() -> SidecarTempo {
        SidecarTempo {
            bpm: 120.0,
            meter: "4/4".to_string(),
            cps: 0.5,
            beats_per_bar: 4.0,
            tempo_override: false,
        }
    }

    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("tempo.bpm", self.bpm),
            ("tempo.cps", self.cps),
            ("tempo.beatsPerBar", self.beats_per_bar),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(format!("{name} must be a finite number above 0"));
            }
        }
        if !is_meter(&self.meter) {
            return Err("tempo.meter must be N/D, for example 4/4".to_string());
        }
        Ok(())
    }
}

impl SidecarBacking {
    /// No backing: neither the song nor the metronome played.
    pub fn none() -> SidecarBacking {
        SidecarBacking {
            song: false,
            metronome: false,
            count_in: false,
            song_path: None,
            start_section: None,
            loop_section: None,
        }
    }

    /// The song or the metronome played, so bar 1 has a time.
    fn clocked(&self) -> bool {
        self.song || self.metronome
    }

    fn validate(&self) -> Result<(), String> {
        if self.count_in && !self.clocked() {
            return Err("backing.countIn requires backing.song or backing.metronome".to_string());
        }
        for (name, value) in [
            ("backing.songPath", &self.song_path),
            ("backing.startSection", &self.start_section),
            ("backing.loopSection", &self.loop_section),
        ] {
            if value.as_deref() == Some("") {
                return Err(format!("{name} must be null or a non-empty string"));
            }
        }
        Ok(())
    }
}

impl SidecarTiming {
    /// No timing: no bar 1, clock offsets, anchors or output stamps.
    pub fn none() -> SidecarTiming {
        SidecarTiming {
            bar1_frame: None,
            bar1_host_ns: None,
            clock_offset_ns: SidecarClockOffsets {
                start: None,
                end: None,
            },
            anchors: Vec::new(),
            output_stamps: Vec::new(),
        }
    }

    fn validate(&self, clocked: bool) -> Result<(), String> {
        if let Some(frame) = self.bar1_frame {
            if !non_negative(frame) {
                return Err(
                    "timing.bar1Frame must be null or a finite number at or above 0".to_string(),
                );
            }
            if !clocked {
                return Err(
                    "timing.bar1Frame requires backing.song or backing.metronome".to_string(),
                );
            }
            if self.bar1_host_ns.is_none() {
                return Err("timing.bar1Frame requires timing.bar1HostNs".to_string());
            }
        }
        for pair in self.anchors.windows(2) {
            if pair[1].0 <= pair[0].0 {
                return Err("timing.anchors frames must be strictly increasing".to_string());
            }
            if pair[1].1 < pair[0].1 {
                return Err("timing.anchors hostNs must not decrease".to_string());
            }
        }
        if self.output_stamps.iter().any(|(t, _)| !non_negative(*t)) {
            return Err(
                "timing.outputStamps contextTime must be a finite number at or above 0".to_string(),
            );
        }
        Ok(())
    }
}

impl Sidecar {
    /// Checks the rules beyond the types. The message names the field.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SIDECAR_SCHEMA {
            return Err(format!("schema must be {SIDECAR_SCHEMA}"));
        }
        if self.created_at.is_empty() {
            return Err("createdAt must not be empty".to_string());
        }
        let input = &self.input;
        if input.channels < 1 {
            return Err("input.channels must be at least 1".to_string());
        }
        if input.channel >= input.channels {
            return Err("input.channel must be below input.channels".to_string());
        }
        if input.sample_rate < 1 {
            return Err("input.sampleRate must be at least 1".to_string());
        }
        if !non_negative(self.output.base_latency) {
            return Err("output.baseLatency must be a finite number at or above 0".to_string());
        }
        if self.output.output_latency.is_some_and(|v| !non_negative(v)) {
            return Err(
                "output.outputLatency must be null or a finite number at or above 0".to_string(),
            );
        }
        self.tempo.validate()?;
        self.backing.validate()?;
        self.timing.validate(self.backing.clocked())?;
        Ok(())
    }

    /// Replaces each part that fails [`Sidecar::validate`] with its empty value, so a take whose
    /// webview fields are bad still gets a valid sidecar: an invalid tempo becomes
    /// [`SidecarTempo::placeholder`], an invalid backing [`SidecarBacking::none`], invalid
    /// timing [`SidecarTiming::none`], an invalid output latency 0 and null, and an empty
    /// `createdAt` the Unix epoch. Input and schema are left as they are (the recorder fills
    /// them). Whether anything was replaced.
    pub fn sanitize(&mut self) -> bool {
        let mut changed = false;
        if self.created_at.is_empty() {
            self.created_at = format_rfc3339_utc(0);
            changed = true;
        }
        let output_ok = non_negative(self.output.base_latency)
            && self.output.output_latency.is_none_or(non_negative);
        if !output_ok {
            self.output = SidecarOutput {
                base_latency: 0.0,
                output_latency: None,
            };
            changed = true;
        }
        if self.tempo.validate().is_err() {
            self.tempo = SidecarTempo::placeholder();
            changed = true;
        }
        if self.backing.validate().is_err() {
            self.backing = SidecarBacking::none();
            changed = true;
        }
        if self.timing.validate(self.backing.clocked()).is_err() {
            self.timing = SidecarTiming::none();
            changed = true;
        }
        changed
    }

    /// Pretty-printed JSON with a trailing newline, as the sidecar is written.
    fn to_file_bytes(&self) -> Vec<u8> {
        let mut json =
            serde_json::to_string_pretty(self).expect("a sidecar always serialises to JSON");
        json.push('\n');
        json.into_bytes()
    }
}

fn non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// `^[1-9][0-9]*/[1-9][0-9]*$`.
fn is_meter(meter: &str) -> bool {
    let part = |p: &str| parse_digits(p).is_some() && !p.starts_with('0');
    meter
        .split_once('/')
        .is_some_and(|(n, d)| part(n) && part(d))
}

/// Parses sidecar bytes: `schema` is checked before any other field, so a future schema is
/// reported as unsupported, never as a field error. `rel` prefixes the `InvalidSidecar` text.
fn parse_sidecar(bytes: &[u8], rel: &str) -> Result<Sidecar, ProjectError> {
    let invalid =
        |msg: &dyn std::fmt::Display| ProjectError::InvalidSidecar(format!("{rel}: {msg}"));
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| invalid(&e))?;
    let Some(schema) = value.as_object().and_then(|o| o.get("schema")) else {
        return Err(invalid(&"missing schema"));
    };
    if schema.as_u64() != Some(u64::from(SIDECAR_SCHEMA)) {
        return Err(ProjectError::UnsupportedSidecarSchema {
            found: schema.to_string(),
        });
    }
    let sidecar: Sidecar = serde_json::from_value(value).map_err(|e| invalid(&e))?;
    sidecar.validate().map_err(|e| invalid(&e))?;
    Ok(sidecar)
}

// --- listing ---

/// One memo as [`Project::recordings`] lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEntry {
    pub stem: String,
    /// From the WAV header; `None` when the header could not be read.
    pub frames: Option<u64>,
    pub sample_rate: Option<u32>,
    /// Seconds, `frames / sample_rate`.
    pub duration: Option<f64>,
    /// A regular file `<stem>.json` exists.
    pub has_sidecar: bool,
    /// The parsed and validated sidecar; `None` when missing or invalid.
    pub sidecar: Option<Sidecar>,
    /// Why an existing sidecar did not read; `None` otherwise.
    pub sidecar_error: Option<String>,
    /// Bytes of the WAV.
    pub size: u64,
    /// The WAV's mtime in Unix milliseconds, 0 when unavailable.
    pub modified_ms: u64,
}

/// What [`read_wav_header`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub block_align: u16,
}

/// The chunk layout up to the first data byte.
struct RiffLayout {
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    /// The offset of the first data byte.
    data_start: u64,
    /// The `data` chunk's size field, as written.
    data_size: u32,
}

/// Walks the RIFF chunks from offset 12 up to `data` (at most [`MAX_CHUNKS`]), trusting nothing
/// past the file end. The error is a reason for the repair report.
fn walk_riff(file: &mut File, file_len: u64) -> Result<RiffLayout, &'static str> {
    let mut read = |offset: u64, buf: &mut [u8]| {
        offset
            .checked_add(buf.len() as u64)
            .is_some_and(|end| end <= file_len)
            && file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| file.read_exact(buf))
                .is_ok()
    };
    let mut head = [0u8; 12];
    if !read(0, &mut head) || &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file");
    }
    let mut offset = 12u64;
    let mut fmt: Option<(u16, u32, u16)> = None;
    for _ in 0..MAX_CHUNKS {
        let mut header = [0u8; 8];
        if !read(offset, &mut header) {
            return Err("no data chunk");
        }
        let size = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes"));
        match &header[0..4] {
            b"fmt " if size >= 16 => {
                let mut body = [0u8; 16];
                if !read(offset + 8, &mut body) {
                    return Err("truncated fmt chunk");
                }
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().expect("4 bytes"));
                let block_align = u16::from_le_bytes([body[12], body[13]]);
                fmt = Some((channels, rate, block_align));
            }
            b"data" => {
                let Some((channels, sample_rate, block_align)) = fmt else {
                    return Err("no fmt chunk before data");
                };
                return Ok(RiffLayout {
                    channels,
                    sample_rate,
                    block_align,
                    data_start: offset + 8,
                    data_size: size,
                });
            }
            _ => {}
        }
        offset += 8 + u64::from(size) + u64::from(size & 1);
    }
    Err("too many chunks before data")
}

/// Reads a WAV's frame count and format from its first chunks, never the audio. The data size is
/// clamped to the file (a placeholder, stale or overrunning size is fine) and a torn last frame is
/// dropped. `None` for a non-WAV, a truncated or corrupt header, or a zero channel count, rate or
/// block align. The format tag is not checked.
pub fn read_wav_header(path: &Path) -> Option<WavInfo> {
    let mut file = File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let layout = walk_riff(&mut file, file_len).ok()?;
    if layout.channels == 0 || layout.sample_rate == 0 || layout.block_align == 0 {
        return None;
    }
    let avail = file_len - layout.data_start;
    let bytes = u64::from(layout.data_size).min(avail);
    Some(WavInfo {
        frames: bytes / u64::from(layout.block_align),
        sample_rate: layout.sample_rate,
        channels: layout.channels,
        block_align: layout.block_align,
    })
}

/// True for exactly `<valid stem>.wav` or `<valid stem>.json` (lowercase suffix). Dotfiles and
/// `.partial` files are false, since a stem never starts with `.`.
pub fn is_recording_file_name(name: &str) -> bool {
    name.strip_suffix(WAV_EXT)
        .or_else(|| name.strip_suffix(JSON_EXT))
        .is_some_and(is_valid_stem)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time in seconds.
pub fn format_rfc3339_utc(unix_secs: u64) -> String {
    let days = unix_secs / 86_400;
    let secs = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

// --- repair ---

/// What [`Project::repair_partials`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    pub recovered: Vec<RecoveredRecording>,
    /// Partial file names deleted because they held no audio.
    pub discarded: Vec<String>,
    /// Partials left as they are (retried on the next open).
    pub skipped: Vec<RepairSkip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredRecording {
    pub stem: String,
    pub frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairSkip {
    /// The partial's file name.
    pub file: String,
    pub reason: String,
}

/// Undoes placed moves, best effort: where the old name still exists the new one is a second
/// link and is removed, otherwise the new name is renamed back.
fn undo_moves(moves: &[(PathBuf, PathBuf)]) {
    for (from, to) in moves.iter().rev() {
        if fs::symlink_metadata(from).is_ok() {
            let _ = fs::remove_file(to);
        } else {
            let _ = move_noreplace(to, from);
        }
    }
}

/// Whether nothing (of any kind) exists at `path`.
fn is_free(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
}

/// `base` shortened from the end (trailing spaces trimmed) so that `base + suffix` fits a stem.
fn fit_stem(base: &str, suffix: &str) -> String {
    let keep = MAX_STEM_LEN.saturating_sub(suffix.len()).min(base.len());
    // Stems are ASCII, so any byte index is a char boundary.
    format!("{}{suffix}", base[..keep].trim_end_matches(' '))
}

fn validate_new_stem(stem: &str) -> Result<(), ProjectError> {
    if is_new_stem(stem) {
        Ok(())
    } else {
        Err(ProjectError::InvalidRecordingName(stem.to_string()))
    }
}

fn validate_stem(stem: &str) -> Result<(), ProjectError> {
    if is_valid_stem(stem) {
        Ok(())
    } else {
        Err(ProjectError::InvalidRecordingName(stem.to_string()))
    }
}

fn wav_name(stem: &str) -> String {
    format!("{stem}{WAV_EXT}")
}

fn json_name(stem: &str) -> String {
    format!("{stem}{JSON_EXT}")
}

fn partial_name(stem: &str) -> String {
    format!(".{stem}{PARTIAL_EXT}")
}

/// What is at a memo file's path. A symlink is refused as outside the project.
enum FileKind {
    Missing,
    File(fs::Metadata),
    Other,
}

fn file_kind(path: &Path, name: &str) -> Result<FileKind, ProjectError> {
    let kind = listed_file_kind(path, name)?;
    if matches!(kind, FileKind::File(_)) && !has_exact_name(path)? {
        return Ok(FileKind::Missing);
    }
    Ok(kind)
}

/// The caller already has the exact entry spelling from read_dir, or checks it after this call.
fn listed_file_kind(path: &Path, name: &str) -> Result<FileKind, ProjectError> {
    match fs::symlink_metadata(path) {
        Ok(m) if redirects(&m) => Err(ProjectError::PathOutsideProject(format!(
            "{RECORDINGS_DIR}/{name}"
        ))),
        Ok(m) if m.is_file() => Ok(FileKind::File(m)),
        Ok(_) => Ok(FileKind::Other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(FileKind::Missing),
        Err(e) => Err(ProjectError::io(path, e)),
    }
}

/// Reads and parses the sidecar at `path` (a regular file); `rel` names it in errors.
fn read_sidecar_file(path: &Path, rel: &str) -> Result<Sidecar, ProjectError> {
    let file = File::open(path).map_err(|e| ProjectError::io(path, e))?;
    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ProjectError::io(path, e))?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(ProjectError::InvalidSidecar(format!(
            "{rel}: file too large"
        )));
    }
    parse_sidecar(&bytes, rel)
}

fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Opens the recordings folder under a canonical project root, refusing directory links and
/// junctions. A project without a recordings folder has no entries.
pub fn recording_dir_entries(root: &Path) -> Result<Option<fs::ReadDir>, ProjectError> {
    let dir = root.join(RECORDINGS_DIR);
    match check_dir(&dir, RECORDINGS_DIR, "") {
        Ok(()) => fs::read_dir(&dir)
            .map(Some)
            .map_err(|e| ProjectError::io(&dir, e)),
        Err(ProjectError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

impl Project {
    fn recordings_dir(&self) -> PathBuf {
        self.root().join(RECORDINGS_DIR)
    }

    /// Runs the directory check on an existing `recordings/`; `false` when it is missing.
    /// `name` is the file the error message names.
    fn check_recordings_dir(&self, name: &str) -> Result<bool, ProjectError> {
        let dir = self.recordings_dir();
        match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(ProjectError::io(&dir, e)),
            Ok(_) => check_dir(&dir, RECORDINGS_DIR, name).map(|()| true),
        }
    }

    /// `recordings/<stem>.wav` (no checks).
    pub fn recording_wav_path(&self, stem: &str) -> PathBuf {
        self.recordings_dir().join(wav_name(stem))
    }

    /// `recordings/<stem>.json` (no checks).
    pub fn recording_json_path(&self, stem: &str) -> PathBuf {
        self.recordings_dir().join(json_name(stem))
    }

    fn recording_partial_file(&self, stem: &str) -> PathBuf {
        self.recordings_dir().join(partial_name(stem))
    }

    /// The first free `<base>-NN` name (`-01` to `-99`, then `-100` and up, never truncated; the
    /// base is shortened to keep the name within 64 bytes). `base` is `song_stem` when it is a
    /// valid stem, else `memo`. A name is free when its WAV, sidecar and partial are all absent
    /// (a symlink counts as taken). Creates nothing.
    pub fn default_recording_stem(&self, song_stem: Option<&str>) -> Result<String, ProjectError> {
        let base = song_stem.filter(|s| is_valid_stem(s)).unwrap_or("memo");
        let exists = self.check_recordings_dir(base)?;
        for n in 1..=MAX_DEFAULT_N {
            let candidate = fit_stem(base, &format!("-{n:02}"));
            if !exists || self.name_is_free(&candidate) {
                return Ok(candidate);
            }
        }
        Err(ProjectError::RecordingExists(base.to_string()))
    }

    fn name_is_free(&self, stem: &str) -> bool {
        is_free(&self.recording_wav_path(stem))
            && is_free(&self.recording_json_path(stem))
            && is_free(&self.recording_partial_file(stem))
    }

    /// Every memo: one entry per regular file `recordings/<stem>.wav` with a valid stem (lowercase
    /// suffix; symlinks and directories skipped), sorted byte-wise by stem. Partials and sidecars
    /// without a WAV are not listed. Each entry's sidecar is read once; a bad sidecar never fails
    /// the listing. A missing `recordings/` gives an empty vec.
    pub fn recordings(&self) -> Result<Vec<RecordingEntry>, ProjectError> {
        let Some(entries) = recording_dir_entries(self.root())? else {
            return Ok(Vec::new());
        };
        let dir = self.recordings_dir();
        let mut wavs = Vec::new();
        let mut sidecars = HashSet::new();
        for entry in entries {
            let entry = entry.map_err(|e| ProjectError::io(&dir, e))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.ends_with(JSON_EXT) {
                sidecars.insert(name);
                continue;
            }
            let Some(stem) = name.strip_suffix(WAV_EXT) else {
                continue;
            };
            if !is_valid_stem(stem) {
                continue;
            }
            // Not followed through a symlink, so a symlinked WAV is skipped.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() || redirects(&meta) {
                continue;
            }
            wavs.push((stem.to_owned(), entry.path(), meta));
        }
        let mut out: Vec<_> = wavs
            .into_iter()
            .map(|(stem, path, meta)| self.entry(&stem, &path, &meta, Some(&sidecars)))
            .collect();
        out.sort_by(|a, b| a.stem.as_bytes().cmp(b.stem.as_bytes()));
        Ok(out)
    }

    fn entry(
        &self,
        stem: &str,
        wav: &Path,
        meta: &fs::Metadata,
        listed_sidecars: Option<&HashSet<String>>,
    ) -> RecordingEntry {
        let header = read_wav_header(wav);
        let (has_sidecar, read) = self.read_sidecar_found(stem, listed_sidecars);
        let (sidecar, sidecar_error) = match read {
            Ok(sidecar) => (sidecar, None),
            Err(e) => (None, Some(e.to_string())),
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        RecordingEntry {
            stem: stem.to_string(),
            frames: header.map(|h| h.frames),
            sample_rate: header.map(|h| h.sample_rate),
            duration: header.map(|h| h.frames as f64 / f64::from(h.sample_rate)),
            has_sidecar,
            sidecar,
            sidecar_error,
            size: meta.len(),
            modified_ms,
        }
    }

    /// The listed entry of one memo, as [`Project::recordings`] gives it, without reading the
    /// other memos. `RecordingNotFound` when `<stem>.wav` is not a regular file
    /// (`PathOutsideProject` for a symlink).
    pub fn recording_entry(&self, stem: &str) -> Result<RecordingEntry, ProjectError> {
        validate_stem(stem)?;
        let meta = self.require_wav(stem)?;
        Ok(self.entry(stem, &self.recording_wav_path(stem), &meta, None))
    }

    /// The whole WAV of a memo (behind the app's `recording_read`).
    pub fn read_recording_bytes(&self, stem: &str) -> Result<Vec<u8>, ProjectError> {
        validate_stem(stem)?;
        let path = self.recording_wav_path(stem);
        self.require_wav(stem)?;
        fs::read(&path).map_err(|e| ProjectError::io(&path, e))
    }

    /// Validates the stem, checks `recordings/` and requires `<stem>.wav` to be a regular file
    /// (`RecordingNotFound` otherwise, `PathOutsideProject` for a symlink).
    fn require_wav(&self, stem: &str) -> Result<fs::Metadata, ProjectError> {
        let name = wav_name(stem);
        if !self.check_recordings_dir(&name)? {
            return Err(ProjectError::RecordingNotFound(stem.to_string()));
        }
        match file_kind(&self.recording_wav_path(stem), &name)? {
            FileKind::File(meta) => Ok(meta),
            _ => Err(ProjectError::RecordingNotFound(stem.to_string())),
        }
    }

    /// The absolute `recordings/.<stem>.wav.partial` a take writes to, creating `recordings/`
    /// when it is missing. `RecordingExists` when the WAV, the sidecar or the partial already
    /// exists (any kind of entry). Creates no file.
    pub fn recording_partial_path(&self, stem: &str) -> Result<PathBuf, ProjectError> {
        validate_new_stem(stem)?;
        let dir = self.recordings_dir();
        let partial = partial_name(stem);
        if !self.check_recordings_dir(&partial)? {
            fs::create_dir_all(&dir).map_err(|e| ProjectError::io(&dir, e))?;
            check_dir(&dir, RECORDINGS_DIR, &partial)?;
        }
        if !self.name_is_free(stem) {
            return Err(ProjectError::RecordingExists(stem.to_string()));
        }
        Ok(self.recording_partial_file(stem))
    }

    /// Publishes a finished take: validates `sidecar`, moves the partial to `<stem>.wav` without
    /// replacing anything, removes the partial, then writes `<stem>.json`. The audio is never
    /// lost: if the sidecar write fails the WAV stays listed without one and the error is
    /// returned. Returns the WAV path.
    pub fn finish_recording(&self, stem: &str, sidecar: &Sidecar) -> Result<PathBuf, ProjectError> {
        self.finish_recording_with(
            stem,
            sidecar,
            |from, to| fs::hard_link(from, to),
            atomic_create_bytes,
        )
    }

    fn finish_recording_with(
        &self,
        stem: &str,
        sidecar: &Sidecar,
        link: impl Fn(&Path, &Path) -> io::Result<()>,
        write: impl Fn(&Path, &[u8]) -> Result<(), ProjectError>,
    ) -> Result<PathBuf, ProjectError> {
        validate_stem(stem)?;
        let json = json_name(stem);
        sidecar
            .validate()
            .map_err(|e| ProjectError::InvalidSidecar(format!("{RECORDINGS_DIR}/{json}: {e}")))?;
        let partial_file = partial_name(stem);
        if !self.check_recordings_dir(&partial_file)? {
            return Err(ProjectError::RecordingNotFound(stem.to_string()));
        }
        let partial = self.recording_partial_file(stem);
        let FileKind::File(_) = file_kind(&partial, &partial_file)? else {
            return Err(ProjectError::RecordingNotFound(stem.to_string()));
        };
        let json_path = self.recording_json_path(stem);
        if !is_free(&json_path) {
            return Err(ProjectError::RecordingExists(stem.to_string()));
        }
        let wav = self.recording_wav_path(stem);
        if link_noclobber(&partial, &wav, &link, || {
            ProjectError::RecordingExists(stem.to_string())
        })? == Placed::Linked
            && let Err(e) = fs::remove_file(&partial)
        {
            // Leave one name: the partial still holds the audio.
            let _ = fs::remove_file(&wav);
            return Err(ProjectError::io(&partial, e));
        }
        write(&json_path, &sidecar.to_file_bytes())?;
        Ok(wav)
    }

    /// Reads and validates `<stem>.json`. `Ok(None)` when it does not exist; a symlink is
    /// `PathOutsideProject`; more than 16 MiB is `InvalidSidecar`.
    pub fn read_sidecar(&self, stem: &str) -> Result<Option<Sidecar>, ProjectError> {
        self.read_sidecar_found(stem, None).1
    }

    /// [`Project::read_sidecar`], with whether `<stem>.json` is a regular file (`true` also
    /// when reading or parsing it failed).
    fn read_sidecar_found(
        &self,
        stem: &str,
        listed: Option<&HashSet<String>>,
    ) -> (bool, Result<Option<Sidecar>, ProjectError>) {
        let found = || -> Result<Option<(PathBuf, String)>, ProjectError> {
            validate_stem(stem)?;
            let name = json_name(stem);
            let rel = format!("{RECORDINGS_DIR}/{name}");
            if !self.check_recordings_dir(&name)? {
                return Ok(None);
            }
            let path = self.recording_json_path(stem);
            let kind = match listed {
                Some(names) if !names.contains(&name) => return Ok(None),
                Some(_) => listed_file_kind(&path, &name)?,
                None => file_kind(&path, &name)?,
            };
            match kind {
                FileKind::Missing => Ok(None),
                FileKind::Other => Err(ProjectError::InvalidSidecar(format!("{rel}: not a file"))),
                FileKind::File(_) => Ok(Some((path, rel))),
            }
        };
        match found() {
            Ok(Some((path, rel))) => (true, read_sidecar_file(&path, &rel).map(Some)),
            Ok(None) => (false, Ok(None)),
            Err(e) => (false, Err(e)),
        }
    }

    /// Validates `sidecar` and writes it atomically as `<stem>.json`, replacing an existing one.
    /// The memo's WAV must exist.
    pub fn write_sidecar(&self, stem: &str, sidecar: &Sidecar) -> Result<(), ProjectError> {
        validate_stem(stem)?;
        let name = json_name(stem);
        sidecar
            .validate()
            .map_err(|e| ProjectError::InvalidSidecar(format!("{RECORDINGS_DIR}/{name}: {e}")))?;
        self.require_wav(stem)?;
        let path = self.recording_json_path(stem);
        match file_kind(&path, &name)? {
            FileKind::File(_) => atomic_write_bytes(&path, &sidecar.to_file_bytes()),
            FileKind::Missing => atomic_create_bytes(&path, &sidecar.to_file_bytes()),
            FileKind::Other => Err(ProjectError::io(
                &path,
                io::Error::new(io::ErrorKind::IsADirectory, "not a file"),
            )),
        }
    }

    /// Renames a memo (its WAV and sidecar) without ever replacing an existing one (nor the
    /// partial of a take in progress under `new`), and returns `new`. If a step fails the moves made
    /// so far are undone, so one name is left.
    pub fn rename_recording(&self, old: &str, new: &str) -> Result<String, ProjectError> {
        self.rename_recording_with(
            old,
            new,
            |from, to| fs::hard_link(from, to),
            |path| fs::remove_file(path),
        )
    }

    fn rename_recording_with(
        &self,
        old: &str,
        new: &str,
        link: impl Fn(&Path, &Path) -> io::Result<()>,
        remove: impl Fn(&Path) -> io::Result<()>,
    ) -> Result<String, ProjectError> {
        validate_stem(old)?;
        validate_new_stem(new)?;
        let old_wav_name = wav_name(old);
        if !self.check_recordings_dir(&old_wav_name)? {
            return Err(ProjectError::RecordingNotFound(old.to_string()));
        }
        let old_wav = self.recording_wav_path(old);
        let FileKind::File(_) = file_kind(&old_wav, &old_wav_name)? else {
            return Err(ProjectError::RecordingNotFound(old.to_string()));
        };
        if new == old {
            return Err(ProjectError::RecordingExists(new.to_string()));
        }
        let old_json = self.recording_json_path(old);
        let has_json = match file_kind(&old_json, &json_name(old))? {
            FileKind::File(_) => true,
            FileKind::Missing => false,
            FileKind::Other => {
                let e = io::Error::new(io::ErrorKind::IsADirectory, "not a file");
                return Err(ProjectError::io(&old_json, e));
            }
        };
        let new_wav = self.recording_wav_path(new);
        let new_json = self.recording_json_path(new);
        let mut pairs = vec![(old_wav.clone(), new_wav.clone())];
        if has_json {
            pairs.push((old_json.clone(), new_json.clone()));
        }

        // A case-only rename on a case-insensitive volume: the new name is the same file.
        if is_case_only_rename(&old_wav, &new_wav).map_err(|e| ProjectError::io(&old_wav, e))? {
            if !is_free(&self.recording_partial_file(new))
                || (!is_free(&new_json)
                    && (!has_json
                        || !is_case_only_rename(&old_json, &new_json)
                            .map_err(|e| ProjectError::io(&old_json, e))?))
            {
                return Err(ProjectError::RecordingExists(new.to_string()));
            }
            let mut done = Vec::new();
            for (from, to) in &pairs {
                if let Err(e) = fs::rename(from, to) {
                    for (from, to) in done.iter().rev() {
                        let _ = fs::rename(to, from);
                    }
                    return Err(ProjectError::io(from, e));
                }
                done.push((from.clone(), to.clone()));
            }
            return Ok(new.to_string());
        }

        // A take in progress under `new` holds its partial: renaming onto it would stop that take
        // from publishing.
        if !self.name_is_free(new) {
            return Err(ProjectError::RecordingExists(new.to_string()));
        }
        let mut placed: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut linked: Vec<PathBuf> = Vec::new();
        for (from, to) in &pairs {
            match link_noclobber(from, to, &link, || {
                ProjectError::RecordingExists(new.to_string())
            }) {
                Ok(kind) => {
                    placed.push((from.clone(), to.clone()));
                    if kind == Placed::Linked {
                        linked.push(from.clone());
                    }
                }
                Err(e) => {
                    undo_moves(&placed);
                    return Err(e);
                }
            }
        }
        // The sidecar's old name goes first, then the WAV's.
        for from in linked.iter().rev() {
            if let Err(e) = remove(from) {
                undo_moves(&placed);
                return Err(ProjectError::io(from, e));
            }
        }
        Ok(new.to_string())
    }

    /// Deletes a memo permanently: the sidecar first (a missing one is fine), then the WAV.
    pub fn delete_recording(&self, stem: &str) -> Result<(), ProjectError> {
        validate_stem(stem)?;
        self.require_wav(stem)?;
        let json = self.recording_json_path(stem);
        if let FileKind::File(_) = file_kind(&json, &json_name(stem))? {
            match fs::remove_file(&json) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(ProjectError::io(&json, e)),
            }
        }
        let wav = self.recording_wav_path(stem);
        fs::remove_file(&wav).map_err(|e| ProjectError::io(&wav, e))
    }

    /// Salvages the `.partial` files a crash left in `recordings/`: fixes their RIFF sizes, moves
    /// each to a free memo name and writes a placeholder sidecar with `recovered: true`. Partials
    /// with no audio are deleted; unparseable ones and ones modified in the last 5 seconds are
    /// left alone. Only the directory check and listing `recordings/` return an error; per-file
    /// problems go into the report. Never call it while a take runs in this project.
    pub fn repair_partials(&self) -> Result<RepairReport, ProjectError> {
        let mut report = RepairReport::default();
        if !self.check_recordings_dir("")? {
            return Ok(report);
        }
        let dir = self.recordings_dir();
        let entries = fs::read_dir(&dir).map_err(|e| ProjectError::io(&dir, e))?;
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| ProjectError::io(&dir, e))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Some(stem) = name
                .strip_prefix('.')
                .and_then(|n| n.strip_suffix(PARTIAL_EXT))
            else {
                continue;
            };
            if !is_valid_stem(stem) {
                continue;
            }
            if fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_file() && !redirects(&m)) {
                candidates.push((name.clone(), stem.to_string()));
            }
        }
        candidates.sort();
        let now = SystemTime::now();
        for (file, stem) in candidates {
            match self.repair_one(&file, &stem, now) {
                Ok(Outcome::Recovered(r)) => report.recovered.push(r),
                Ok(Outcome::Discarded) => report.discarded.push(file),
                Err(reason) => report.skipped.push(RepairSkip { file, reason }),
            }
        }
        Ok(report)
    }

    fn repair_one(&self, file: &str, stem: &str, now: SystemTime) -> Result<Outcome, String> {
        let path = self.recordings_dir().join(file);
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age < IN_USE_WINDOW)
        {
            return Err("in use".to_string());
        }
        let created_at = format_rfc3339_utc(mtime_secs(&meta));
        let file_len = meta.len();
        if file_len < MIN_WAV_LEN {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            return Ok(Outcome::Discarded);
        }
        let mut handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let (frames, sample_rate) = fix_partial_sizes(&mut handle, file_len)?;
        if frames == 0 {
            drop(handle);
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            return Ok(Outcome::Discarded);
        }
        drop(handle);

        let target = self
            .recovered_stem(stem)
            .ok_or_else(|| "no free name".to_string())?;
        let wav = self.recording_wav_path(&target);
        let link = |from: &Path, to: &Path| fs::hard_link(from, to);
        let placed = link_noclobber(&path, &wav, &link, || {
            ProjectError::RecordingExists(target.clone())
        })
        .map_err(|e| e.to_string())?;
        if placed == Placed::Linked
            && let Err(e) = fs::remove_file(&path)
        {
            let _ = fs::remove_file(&wav);
            return Err(e.to_string());
        }
        let sidecar = recovered_sidecar(created_at, sample_rate);
        atomic_create_bytes(&self.recording_json_path(&target), &sidecar.to_file_bytes())
            .map_err(|e| format!("recovered as {target}.wav, but the sidecar failed: {e}"))?;
        Ok(Outcome::Recovered(RecoveredRecording {
            stem: target,
            frames,
        }))
    }

    /// `stem` when its WAV and sidecar are absent, else the first free `<stem>-recovered`,
    /// `<stem>-recovered-2` … `-99`.
    fn recovered_stem(&self, stem: &str) -> Option<String> {
        let free =
            |s: &str| is_free(&self.recording_wav_path(s)) && is_free(&self.recording_json_path(s));
        if is_new_stem(stem) && free(stem) {
            return Some(stem.to_string());
        }
        (1..=MAX_RECOVERED_N)
            .map(|n| {
                let suffix = if n == 1 {
                    "-recovered".to_string()
                } else {
                    format!("-recovered-{n}")
                };
                fit_stem(stem, &suffix)
            })
            .find(|s| free(s))
    }
}

/// Fixes a partial's RIFF and `data` sizes in place from its length: drops a torn last frame,
/// clamps to the u32 limit, pads an odd data chunk and syncs. Returns the whole frames and the
/// sample rate; with zero frames it changes nothing. Idempotent: a second call on a fixed file
/// writes the same bytes.
fn fix_partial_sizes(handle: &mut File, file_len: u64) -> Result<(u64, u32), String> {
    let layout = walk_riff(handle, file_len).map_err(str::to_string)?;
    if layout.block_align == 0 {
        return Err("block align 0".to_string());
    }
    if layout.sample_rate == 0 {
        return Err("sample rate 0".to_string());
    }
    let align = u64::from(layout.block_align);
    let mut avail = file_len - layout.data_start;
    // An odd data chunk the repair already padded: the pad byte is not audio.
    if layout.data_size & 1 == 1 && avail == u64::from(layout.data_size) + 1 {
        avail -= 1;
    }
    let max_bytes = u64::from(u32::MAX) / align * align;
    let data_bytes = (avail / align * align).min(max_bytes);
    let frames = data_bytes / align;
    if frames == 0 {
        return Ok((0, layout.sample_rate));
    }
    let pad = data_bytes & 1;
    let end = layout.data_start + data_bytes;
    let mut fix = || -> io::Result<()> {
        handle.set_len(end)?;
        if pad == 1 {
            handle.seek(SeekFrom::Start(end))?;
            handle.write_all(&[0])?;
        }
        let data_size = u32::try_from(data_bytes).expect("clamped to u32");
        handle.seek(SeekFrom::Start(layout.data_start - 4))?;
        handle.write_all(&data_size.to_le_bytes())?;
        let riff = u32::try_from(end + pad - 8).unwrap_or(u32::MAX);
        handle.seek(SeekFrom::Start(4))?;
        handle.write_all(&riff.to_le_bytes())?;
        handle.sync_all()
    };
    fix().map_err(|e| e.to_string())?;
    Ok((frames, layout.sample_rate))
}

enum Outcome {
    Recovered(RecoveredRecording),
    Discarded,
}

/// The placeholder sidecar of a recovered take: the real tempo and timing are unknown.
fn recovered_sidecar(created_at: String, sample_rate: u32) -> Sidecar {
    Sidecar {
        schema: SIDECAR_SCHEMA,
        created_at,
        app: SidecarApp {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        input: SidecarInput {
            device_id: String::new(),
            device_name: String::new(),
            channel: 0,
            channels: 1,
            sample_rate,
        },
        output: SidecarOutput {
            base_latency: 0.0,
            output_latency: None,
        },
        tempo: SidecarTempo::placeholder(),
        backing: SidecarBacking::none(),
        timing: SidecarTiming::none(),
        xruns: 0,
        recovered: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_symlink;
    use tempfile::TempDir;

    fn new_project() -> (TempDir, Project) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = Project::create(&tmp.path().join("p"), "p").expect("create");
        (tmp, project)
    }

    fn rec(project: &Project) -> PathBuf {
        project.root().join(RECORDINGS_DIR)
    }

    fn put(project: &Project, name: &str, bytes: &[u8]) -> PathBuf {
        let path = rec(project).join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn names(project: &Project) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(rec(project))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n != ".gitkeep")
            .collect();
        names.sort();
        names
    }

    fn stems(project: &Project) -> Vec<String> {
        project
            .recordings()
            .unwrap()
            .into_iter()
            .map(|e| e.stem)
            .collect()
    }

    /// A WAV: `fmt_len` 16 (plain) or 40 (extensible), an optional `LIST` chunk before `data`,
    /// `data_size` written as given (`None`: the real length) and `data` bytes.
    fn wav(
        fmt_len: u32,
        list: bool,
        channels: u16,
        rate: u32,
        block_align: u16,
        data_size: Option<u32>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&fmt_len.to_le_bytes());
        let mut fmt = vec![0u8; fmt_len as usize];
        fmt[0..2].copy_from_slice(&(if fmt_len == 40 { 0xFFFEu16 } else { 3 }).to_le_bytes());
        fmt[2..4].copy_from_slice(&channels.to_le_bytes());
        fmt[4..8].copy_from_slice(&rate.to_le_bytes());
        fmt[12..14].copy_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&fmt);
        if list {
            out.extend_from_slice(b"LIST");
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(&[1, 2, 3, 0]); // odd size plus its pad byte
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_size.unwrap_or(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Mono f32 at 48 kHz, plain `fmt `, with `frames` frames and a correct size.
    fn mono(frames: usize) -> Vec<u8> {
        wav(16, false, 1, 48_000, 4, None, &vec![0u8; frames * 4])
    }

    fn sidecar() -> Sidecar {
        Sidecar {
            schema: 1,
            created_at: "2026-11-13T09:30:00Z".to_string(),
            app: SidecarApp {
                version: "0.1.0".to_string(),
            },
            input: SidecarInput {
                device_id: "BuiltInMicrophoneDevice".to_string(),
                device_name: "MacBook Pro Microphone".to_string(),
                channel: 0,
                channels: 1,
                sample_rate: 48_000,
            },
            output: SidecarOutput {
                base_latency: 0.01,
                output_latency: Some(0.02),
            },
            tempo: SidecarTempo {
                bpm: 120.0,
                meter: "4/4".to_string(),
                cps: 0.5,
                beats_per_bar: 4.0,
                tempo_override: false,
            },
            backing: SidecarBacking {
                song: true,
                metronome: true,
                count_in: true,
                song_path: Some("songs/main.js".to_string()),
                start_section: Some("verse".to_string()),
                loop_section: None,
            },
            timing: SidecarTiming {
                bar1_frame: Some(96_000.0),
                bar1_host_ns: Some(HostNs(9_007_199_254_740_993)),
                clock_offset_ns: SidecarClockOffsets {
                    start: Some(ClockOffsetSample {
                        offset_ns: OffsetNs(-12),
                        rtt_ns: HostNs(900_000),
                    }),
                    end: None,
                },
                anchors: vec![(0, HostNs(1_000)), (48_000, HostNs(2_000))],
                output_stamps: vec![(1.5, HostNs(1_500))],
            },
            xruns: 0,
            recovered: false,
        }
    }

    fn sidecar_json(s: &Sidecar) -> serde_json::Value {
        serde_json::to_value(s).unwrap()
    }

    fn set_old(path: &Path) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    }

    /// A partial old enough for the repair.
    fn put_partial(project: &Project, stem: &str, bytes: &[u8]) -> PathBuf {
        let path = put(project, &partial_name(stem), bytes);
        set_old(&path);
        path
    }

    fn no_links(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no hard links"))
    }

    // --- naming ---

    #[test]
    fn default_stem_uses_memo_or_the_song() {
        let (_t, p) = new_project();
        assert_eq!(p.default_recording_stem(None).unwrap(), "memo-01");
        assert_eq!(p.default_recording_stem(Some("main")).unwrap(), "main-01");
        assert_eq!(
            p.default_recording_stem(Some("bad.name")).unwrap(),
            "memo-01"
        );
        assert_eq!(p.default_recording_stem(Some(" x")).unwrap(), "memo-01");
        assert_eq!(names(&p), Vec::<String>::new(), "creates nothing");
    }

    #[test]
    fn default_stem_with_no_recordings_dir() {
        let (_t, p) = new_project();
        fs::remove_dir_all(rec(&p)).unwrap();
        assert_eq!(p.default_recording_stem(None).unwrap(), "memo-01");
        assert!(!rec(&p).exists());
    }

    #[test]
    fn default_stem_skips_names_taken_by_any_file() {
        let (_t, p) = new_project();
        put(&p, "memo-01.wav", b"");
        put(&p, "memo-02.json", b"");
        put(&p, ".memo-03.wav.partial", b"");
        test_symlink!(rec(&p).join("memo-01.wav"), rec(&p).join("memo-04.wav")).unwrap();
        assert_eq!(p.default_recording_stem(None).unwrap(), "memo-05");
    }

    #[test]
    fn default_stem_goes_past_99() {
        let (_t, p) = new_project();
        for n in 1..=99 {
            put(&p, &format!("memo-{n:02}.wav"), b"");
        }
        assert_eq!(p.default_recording_stem(None).unwrap(), "memo-100");
    }

    #[test]
    fn default_stem_shortens_a_long_song_stem() {
        let (_t, p) = new_project();
        let long = "a".repeat(64);
        assert_eq!(
            p.default_recording_stem(Some(&long)).unwrap(),
            format!("{}-01", "a".repeat(61))
        );
        let spaced = format!("{}   b", "a".repeat(60));
        assert_eq!(spaced.len(), 64);
        assert_eq!(
            p.default_recording_stem(Some(&spaced)).unwrap(),
            format!("{}-01", "a".repeat(60))
        );
        assert_eq!(fit_stem(&long, "-100"), format!("{}-100", "a".repeat(60)));
        assert_eq!(fit_stem("memo", "-100"), "memo-100");
    }

    #[test]
    fn partial_path_rejects_invalid_and_taken_names() {
        let (_t, p) = new_project();
        for bad in ["", " a", "a ", "a.b", "a/b", ".a", &"a".repeat(65)] {
            assert!(
                matches!(p.recording_partial_path(bad), Err(ProjectError::InvalidRecordingName(ref s)) if s == bad),
                "{bad:?}"
            );
        }
        put(&p, "w.wav", b"");
        put(&p, "j.json", b"");
        put(&p, ".q.wav.partial", b"");
        fs::create_dir(rec(&p).join("d.wav")).unwrap();
        for taken in ["w", "j", "q", "d"] {
            assert!(
                matches!(p.recording_partial_path(taken), Err(ProjectError::RecordingExists(ref s)) if s == taken),
                "{taken}"
            );
        }
    }

    #[test]
    fn partial_path_creates_the_folder_but_no_file() {
        let (_t, p) = new_project();
        fs::remove_dir_all(rec(&p)).unwrap();
        let path = p.recording_partial_path("take").unwrap();
        assert_eq!(path, p.root().join("recordings/.take.wav.partial"));
        assert!(rec(&p).is_dir());
        assert!(!path.exists());
    }

    #[test]
    fn invalid_recording_name_display() {
        assert_eq!(
            ProjectError::InvalidRecordingName("a.b".into()).to_string(),
            "invalid memo name \"a.b\" (use letters, digits, space, - and _; 1 to 64 characters)"
        );
        assert_eq!(
            ProjectError::RecordingExists("m".into()).to_string(),
            "memo m already exists"
        );
        assert_eq!(
            ProjectError::RecordingNotFound("m".into()).to_string(),
            "memo m not found"
        );
    }

    // --- listing ---

    #[test]
    fn listing_rules() {
        let (_t, p) = new_project();
        put(&p, "b.wav", &mono(10));
        put(&p, "a.wav", &mono(10));
        put(&p, "Z.wav", &mono(10));
        put(&p, ".c.wav.partial", &mono(10));
        put(&p, "d.json", b"{}");
        put(&p, "e.WAV", &mono(10));
        put(&p, "f .wav", &mono(10));
        put(&p, "g.h.wav", &mono(10));
        put(&p, "notes.txt", b"x");
        fs::create_dir(rec(&p).join("dir.wav")).unwrap();
        test_symlink!(rec(&p).join("a.wav"), rec(&p).join("link.wav")).unwrap();
        assert_eq!(stems(&p), ["Z", "a", "b"]);
    }

    #[test]
    fn listing_without_recordings_dir_is_empty() {
        let (_t, p) = new_project();
        fs::remove_dir_all(rec(&p)).unwrap();
        assert_eq!(p.recordings().unwrap(), vec![]);
    }

    #[test]
    fn listing_reads_header_and_sidecars() {
        let (_t, p) = new_project();
        put(&p, "good.wav", &mono(24_000));
        p.write_sidecar("good", &sidecar()).unwrap();
        put(&p, "none.wav", &mono(10));
        put(&p, "bad.wav", &mono(10));
        put(&p, "bad.json", b"{ nope");
        put(&p, "future.wav", &mono(10));
        put(&p, "future.json", br#"{"schema": 2}"#);
        put(&p, "junk.wav", b"not a wav at all, really");
        let list = p.recordings().unwrap();
        let get = |s: &str| list.iter().find(|e| e.stem == s).unwrap().clone();

        let good = get("good");
        assert_eq!(good.frames, Some(24_000));
        assert_eq!(good.sample_rate, Some(48_000));
        assert_eq!(good.duration, Some(0.5));
        assert!(good.has_sidecar);
        assert_eq!(good.sidecar, Some(sidecar()));
        assert_eq!(good.sidecar_error, None);
        assert_eq!(good.size, 44 + 96_000);
        assert!(good.modified_ms > 1_700_000_000_000);

        let none = get("none");
        assert!(!none.has_sidecar);
        assert_eq!((none.sidecar, none.sidecar_error), (None, None));

        let bad = get("bad");
        assert!(bad.has_sidecar);
        assert_eq!(bad.sidecar, None);
        let err = bad.sidecar_error.unwrap();
        assert!(
            err.starts_with("invalid memo sidecar: recordings/bad.json: "),
            "{err}"
        );

        let future = get("future");
        assert_eq!(
            future.sidecar_error.as_deref(),
            Some("unsupported memo sidecar schema 2 (this app reads schema 1)")
        );

        let junk = get("junk");
        assert_eq!(
            (junk.frames, junk.sample_rate, junk.duration),
            (None, None, None)
        );
        assert_eq!(junk.size, 24);
    }

    #[test]
    fn listing_uses_exact_sidecar_entries_for_each_wav() {
        let (_tmp, p) = new_project();
        put(&p, "Named.wav", &mono(1));
        put(&p, "named.json", &sidecar().to_file_bytes());
        put(&p, "Pair.wav", &mono(1));
        put(&p, "Pair.json", &sidecar().to_file_bytes());
        let list = p.recordings().unwrap();
        assert_eq!(list.len(), 2);
        assert!(!list[0].has_sidecar);
        assert!(list[0].sidecar_error.is_none());
        assert!(list[1].has_sidecar);
        assert!(list[1].sidecar.is_some());
        for item in list {
            assert_eq!(p.recording_entry(&item.stem).unwrap(), item);
        }
    }

    #[test]
    fn one_entry_matches_the_listing() {
        let (_t, p) = new_project();
        put(&p, "good.wav", &mono(24_000));
        p.write_sidecar("good", &sidecar()).unwrap();
        put(&p, "bad.wav", &mono(10));
        put(&p, "bad.json", b"{ nope");
        put(&p, "none.wav", &mono(10));
        let list = p.recordings().unwrap();
        for stem in ["good", "bad", "none"] {
            let listed = list.iter().find(|e| e.stem == stem).unwrap();
            assert_eq!(&p.recording_entry(stem).unwrap(), listed, "{stem}");
        }
        assert!(matches!(
            p.recording_entry("gone"),
            Err(ProjectError::RecordingNotFound(ref s)) if s == "gone"
        ));
        assert!(matches!(
            p.recording_entry("x.y"),
            Err(ProjectError::InvalidRecordingName(_))
        ));
        fs::create_dir(rec(&p).join("dir.wav")).unwrap();
        assert!(matches!(
            p.recording_entry("dir"),
            Err(ProjectError::RecordingNotFound(_))
        ));
        test_symlink!(rec(&p).join("good.wav"), rec(&p).join("link.wav")).unwrap();
        assert!(matches!(
            p.recording_entry("link"),
            Err(ProjectError::PathOutsideProject(_))
        ));
    }

    #[test]
    fn one_entry_without_recordings_dir_is_not_found() {
        let (_t, p) = new_project();
        assert!(matches!(
            p.recording_entry("a"),
            Err(ProjectError::RecordingNotFound(_))
        ));
    }

    #[test]
    fn entry_serialises_camel_case() {
        let (_t, p) = new_project();
        put(&p, "m.wav", &mono(48));
        let value = serde_json::to_value(&p.recordings().unwrap()[0]).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut keys = keys;
        keys.sort();
        assert_eq!(
            keys,
            [
                "duration",
                "frames",
                "hasSidecar",
                "modifiedMs",
                "sampleRate",
                "sidecar",
                "sidecarError",
                "size",
                "stem"
            ]
        );
    }

    // --- WAV header ---

    fn header_of(bytes: &[u8]) -> Option<WavInfo> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.wav");
        fs::write(&path, bytes).unwrap();
        read_wav_header(&path)
    }

    #[test]
    fn header_plain_fmt_data_at_44() {
        let bytes = mono(3);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            header_of(&bytes),
            Some(WavInfo {
                frames: 3,
                sample_rate: 48_000,
                channels: 1,
                block_align: 4
            })
        );
    }

    #[test]
    fn header_extensible_fmt_data_at_68() {
        let bytes = wav(40, false, 2, 44_100, 8, None, &[0u8; 80]);
        assert_eq!(&bytes[60..64], b"data");
        assert_eq!(
            header_of(&bytes),
            Some(WavInfo {
                frames: 10,
                sample_rate: 44_100,
                channels: 2,
                block_align: 8
            })
        );
    }

    #[test]
    fn header_skips_a_list_chunk() {
        let bytes = wav(16, true, 1, 48_000, 4, None, &[0u8; 20]);
        assert_eq!(header_of(&bytes).unwrap().frames, 5);
    }

    #[test]
    fn header_clamps_the_data_size() {
        // A stale size (hound's last flush) is trusted when smaller than the file.
        assert_eq!(
            header_of(&wav(16, false, 1, 48_000, 4, Some(0), &[0u8; 40]))
                .unwrap()
                .frames,
            0
        );
        assert_eq!(
            header_of(&wav(16, false, 1, 48_000, 4, Some(8), &[0u8; 40]))
                .unwrap()
                .frames,
            2
        );
        // A size that overruns the file is clamped to it, and a torn frame dropped.
        assert_eq!(
            header_of(&wav(16, false, 1, 48_000, 4, Some(u32::MAX), &[0u8; 10]))
                .unwrap()
                .frames,
            2
        );
    }

    #[test]
    fn header_rejects_bad_files() {
        assert_eq!(header_of(b""), None);
        assert_eq!(header_of(b"RIFF\0\0\0\0WAVE"), None, "no chunks");
        assert_eq!(header_of(&mono(4)[..30]), None, "truncated fmt");
        let mut not_wave = mono(4);
        not_wave[8..12].copy_from_slice(b"AVI ");
        assert_eq!(header_of(&not_wave), None);
        // data before fmt
        let mut data_first = b"RIFF\0\0\0\0WAVEdata\x04\0\0\0\0\0\0\0".to_vec();
        data_first.extend_from_slice(&mono(1)[12..]);
        assert_eq!(header_of(&data_first), None);
        assert_eq!(
            header_of(&wav(16, false, 0, 48_000, 4, None, &[0; 8])),
            None
        );
        assert_eq!(header_of(&wav(16, false, 1, 0, 4, None, &[0; 8])), None);
        assert_eq!(
            header_of(&wav(16, false, 1, 48_000, 0, None, &[0; 8])),
            None
        );
        // more than 32 chunks before data
        let mut many = mono(0)[..36].to_vec();
        for _ in 0..40 {
            many.extend_from_slice(b"junk\0\0\0\0");
        }
        many.extend_from_slice(b"data\0\0\0\0");
        assert_eq!(header_of(&many), None);
    }

    // --- bytes and predicates ---

    #[test]
    fn read_bytes() {
        let (_t, p) = new_project();
        put(&p, "m.wav", &mono(5));
        assert_eq!(p.read_recording_bytes("m").unwrap(), mono(5));
        test_symlink!(rec(&p).join("m.wav"), rec(&p).join("s.wav")).unwrap();
        assert!(matches!(
            p.read_recording_bytes("s"),
            Err(ProjectError::PathOutsideProject(ref s)) if s == "recordings/s.wav"
        ));
        assert!(matches!(
            p.read_recording_bytes("a.b"),
            Err(ProjectError::InvalidRecordingName(_))
        ));
        assert!(matches!(
            p.read_recording_bytes("gone"),
            Err(ProjectError::RecordingNotFound(ref s)) if s == "gone"
        ));
        fs::remove_dir_all(rec(&p)).unwrap();
        assert!(matches!(
            p.read_recording_bytes("m"),
            Err(ProjectError::RecordingNotFound(_))
        ));
    }

    #[test]
    fn recording_file_names() {
        for yes in ["a.wav", "a.json", "a b.wav", "memo-01.json"] {
            assert!(is_recording_file_name(yes), "{yes}");
        }
        for no in [
            ".a.wav.partial",
            "a.WAV",
            "a.txt",
            "a .wav",
            ".wav",
            ".json",
            "a.wav.partial",
            ".a.wav",
            "a.b.wav",
        ] {
            assert!(!is_recording_file_name(no), "{no}");
        }
    }

    #[test]
    fn rfc3339() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(format_rfc3339_utc(2_147_483_648), "2038-01-19T03:14:08Z");
        assert_eq!(format_rfc3339_utc(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    // --- finish ---

    #[test]
    fn finish_publishes_wav_and_sidecar() {
        let (_t, p) = new_project();
        let partial = p.recording_partial_path("take").unwrap();
        fs::write(&partial, mono(7)).unwrap();
        let wav = p.finish_recording("take", &sidecar()).unwrap();
        assert_eq!(wav, p.root().join("recordings/take.wav"));
        assert_eq!(names(&p), ["take.json", "take.wav"]);
        assert_eq!(fs::read(&wav).unwrap(), mono(7));
        let text = fs::read_to_string(p.recording_json_path("take")).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"schema\": 1,"), "pretty: {text}");
        assert_eq!(p.read_sidecar("take").unwrap(), Some(sidecar()));
    }

    #[test]
    fn finish_refuses_an_existing_name_untouched() {
        let (_t, p) = new_project();
        put(&p, ".take.wav.partial", &mono(7));
        put(&p, "take.json", b"old");
        assert!(matches!(
            p.finish_recording("take", &sidecar()),
            Err(ProjectError::RecordingExists(ref s)) if s == "take"
        ));
        assert_eq!(names(&p), [".take.wav.partial", "take.json"]);
        fs::remove_file(rec(&p).join("take.json")).unwrap();
        put(&p, "take.wav", b"old");
        assert!(matches!(
            p.finish_recording("take", &sidecar()),
            Err(ProjectError::RecordingExists(_))
        ));
        assert_eq!(fs::read(rec(&p).join("take.wav")).unwrap(), b"old");
        assert_eq!(
            fs::read(rec(&p).join(".take.wav.partial")).unwrap(),
            mono(7)
        );
    }

    #[test]
    fn finish_checks_before_touching_the_disk() {
        let (_t, p) = new_project();
        assert!(matches!(
            p.finish_recording("take", &sidecar()),
            Err(ProjectError::RecordingNotFound(_))
        ));
        put(&p, ".take.wav.partial", &mono(7));
        let mut bad = sidecar();
        bad.tempo.bpm = 0.0;
        let err = p.finish_recording("take", &bad).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid memo sidecar: recordings/take.json: tempo.bpm must be a finite number above 0"
        );
        assert_eq!(names(&p), [".take.wav.partial"]);
    }

    #[test]
    fn finish_keeps_the_wav_when_the_sidecar_write_fails() {
        let (_t, p) = new_project();
        put(&p, ".take.wav.partial", &mono(7));
        let err = p
            .finish_recording_with(
                "take",
                &sidecar(),
                |a, b| fs::hard_link(a, b),
                |path, _| Err(ProjectError::io(path, io::Error::other("disk full"))),
            )
            .unwrap_err();
        assert!(err.to_string().ends_with("take.json: disk full"), "{err}");
        assert_eq!(names(&p), ["take.wav"]);
        let entry = &p.recordings().unwrap()[0];
        assert_eq!((entry.frames, entry.has_sidecar), (Some(7), false));
    }

    #[test]
    fn finish_without_hard_links() {
        let (_t, p) = new_project();
        put(&p, ".take.wav.partial", &mono(7));
        p.finish_recording_with("take", &sidecar(), no_links, |a, b| {
            atomic_write_bytes(a, b)
        })
        .unwrap();
        assert_eq!(names(&p), ["take.json", "take.wav"]);
    }

    // --- rename ---

    #[test]
    fn rename_moves_both_files() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        p.write_sidecar("a", &sidecar()).unwrap();
        assert_eq!(p.rename_recording("a", "b c").unwrap(), "b c");
        assert_eq!(names(&p), ["b c.json", "b c.wav"]);
        assert_eq!(p.read_sidecar("b c").unwrap(), Some(sidecar()));
    }

    #[test]
    fn rename_without_a_sidecar() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        p.rename_recording("a", "b").unwrap();
        assert_eq!(names(&p), ["b.wav"]);
    }

    #[test]
    fn rename_refuses_conflicts() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        put(&p, "a.json", b"x");
        put(&p, "w.wav", b"w");
        put(&p, "j.json", b"j");
        for target in ["a", "w", "j"] {
            assert!(
                matches!(p.rename_recording("a", target), Err(ProjectError::RecordingExists(ref s)) if s == target),
                "{target}"
            );
        }
        assert!(matches!(
            p.rename_recording("gone", "x"),
            Err(ProjectError::RecordingNotFound(ref s)) if s == "gone"
        ));
        assert!(matches!(
            p.rename_recording("a", "x.y"),
            Err(ProjectError::InvalidRecordingName(_))
        ));
        assert_eq!(names(&p), ["a.json", "a.wav", "j.json", "w.wav"]);
    }

    #[test]
    fn rename_refuses_the_stem_of_a_take_in_progress() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        let partial = p.recording_partial_path("take").unwrap();
        fs::write(&partial, mono(5)).unwrap();
        assert!(matches!(
            p.rename_recording("a", "take"),
            Err(ProjectError::RecordingExists(ref s)) if s == "take"
        ));
        assert_eq!(names(&p), [".take.wav.partial", "a.wav"]);
        p.finish_recording("take", &sidecar()).unwrap();
        assert_eq!(names(&p), ["a.wav", "take.json", "take.wav"]);
    }

    #[test]
    fn rename_case_only() {
        let (_t, p) = new_project();
        put(&p, "memo.wav", &mono(3));
        put(&p, "memo.json", b"x");
        p.rename_recording("memo", "Memo").unwrap();
        assert_eq!(names(&p), ["Memo.json", "Memo.wav"]);
    }

    #[test]
    fn rename_without_hard_links() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        put(&p, "a.json", b"x");
        p.rename_recording_with("a", "b", no_links, |path| fs::remove_file(path))
            .unwrap();
        assert_eq!(names(&p), ["b.json", "b.wav"]);
    }

    #[test]
    fn rename_leaves_one_name_when_a_removal_fails() {
        for failing in ["a.json", "a.wav"] {
            let (_t, p) = new_project();
            put(&p, "a.wav", &mono(3));
            put(&p, "a.json", b"x");
            let err = p
                .rename_recording_with(
                    "a",
                    "b",
                    |x, y| fs::hard_link(x, y),
                    |path| {
                        if path.ends_with(failing) {
                            Err(io::Error::other("busy"))
                        } else {
                            fs::remove_file(path)
                        }
                    },
                )
                .unwrap_err();
            assert!(err.to_string().ends_with("busy"), "{err}");
            assert_eq!(names(&p), ["a.json", "a.wav"], "{failing}");
            assert_eq!(fs::read(rec(&p).join("a.json")).unwrap(), b"x");
        }
    }

    #[test]
    fn rename_undoes_the_wav_when_the_sidecar_link_fails() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        put(&p, "a.json", b"x");
        let err = p
            .rename_recording_with(
                "a",
                "b",
                |x, y| {
                    if x.ends_with("a.json") {
                        Err(io::Error::new(io::ErrorKind::AlreadyExists, "raced"))
                    } else {
                        fs::hard_link(x, y)
                    }
                },
                |path| fs::remove_file(path),
            )
            .unwrap_err();
        assert!(matches!(err, ProjectError::RecordingExists(ref s) if s == "b"));
        assert_eq!(names(&p), ["a.json", "a.wav"]);
    }

    // --- delete ---

    #[test]
    fn delete_removes_both() {
        let (_t, p) = new_project();
        put(&p, "a.wav", &mono(3));
        put(&p, "a.json", b"x");
        put(&p, "b.wav", &mono(3));
        p.delete_recording("a").unwrap();
        p.delete_recording("b").unwrap();
        assert_eq!(names(&p), Vec::<String>::new());
        assert!(matches!(
            p.delete_recording("a"),
            Err(ProjectError::RecordingNotFound(ref s)) if s == "a"
        ));
        put(&p, "orphan.json", b"x");
        assert!(matches!(
            p.delete_recording("orphan"),
            Err(ProjectError::RecordingNotFound(_))
        ));
        assert_eq!(names(&p), ["orphan.json"]);
    }

    // --- sidecar ---

    #[test]
    fn sidecar_round_trip_keeps_big_and_signed_times() {
        let s = sidecar();
        let text = serde_json::to_string(&s).unwrap();
        assert!(
            text.contains(r#""bar1HostNs":"9007199254740993""#),
            "{text}"
        );
        assert!(text.contains(r#""offsetNs":"-12""#), "{text}");
        assert!(
            text.contains(r#""anchors":[[0,"1000"],[48000,"2000"]]"#),
            "{text}"
        );
        assert!(text.contains(r#""outputStamps":[[1.5,"1500"]]"#), "{text}");
        assert!(text.contains(r#""end":null"#), "{text}");
        assert_eq!(parse_sidecar(text.as_bytes(), "r").unwrap(), s);

        let mut nulls = sidecar();
        nulls.timing.clock_offset_ns = SidecarClockOffsets {
            start: None,
            end: None,
        };
        let text = serde_json::to_string(&nulls).unwrap();
        assert!(
            text.contains(r#""clockOffsetNs":{"start":null,"end":null}"#),
            "{text}"
        );
        assert_eq!(parse_sidecar(text.as_bytes(), "r").unwrap(), nulls);
    }

    #[test]
    fn sidecar_field_names_are_camel_case() {
        let v = sidecar_json(&sidecar());
        let keys = |v: &serde_json::Value| -> Vec<String> {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&v),
            [
                "app",
                "backing",
                "createdAt",
                "input",
                "output",
                "recovered",
                "schema",
                "tempo",
                "timing",
                "xruns"
            ]
        );
        assert_eq!(
            keys(&v["input"]),
            [
                "channel",
                "channels",
                "deviceId",
                "deviceName",
                "sampleRate"
            ]
        );
        assert_eq!(keys(&v["output"]), ["baseLatency", "outputLatency"]);
        assert_eq!(
            keys(&v["tempo"]),
            ["beatsPerBar", "bpm", "cps", "meter", "tempoOverride"]
        );
        assert_eq!(
            keys(&v["backing"]),
            [
                "countIn",
                "loopSection",
                "metronome",
                "song",
                "songPath",
                "startSection"
            ]
        );
        assert_eq!(
            keys(&v["timing"]),
            [
                "anchors",
                "bar1Frame",
                "bar1HostNs",
                "clockOffsetNs",
                "outputStamps"
            ]
        );
        assert_eq!(
            keys(&v["timing"]["clockOffsetNs"]["start"]),
            ["offsetNs", "rttNs"]
        );
    }

    #[test]
    fn host_ns_strings() {
        let host = |t: &str| serde_json::from_str::<HostNs>(t).map(|h| h.0).ok();
        assert_eq!(host(r#""0""#), Some(0));
        assert_eq!(host(r#""18446744073709551615""#), Some(u64::MAX));
        for bad in [
            r#""18446744073709551616""#,
            r#""-1""#,
            r#""+1""#,
            r#"" 1""#,
            r#""1 ""#,
            r#""""#,
            r#""1e3""#,
            "5",
            "null",
        ] {
            assert_eq!(host(bad), None, "{bad}");
        }
        let off = |t: &str| serde_json::from_str::<OffsetNs>(t).map(|h| h.0).ok();
        assert_eq!(off(r#""-12""#), Some(-12));
        assert_eq!(off(r#""7""#), Some(7));
        assert_eq!(off(r#""-9223372036854775808""#), Some(i64::MIN));
        for bad in [
            r#""-""#,
            r#""--1""#,
            r#""+1""#,
            r#""9223372036854775808""#,
            "-1",
        ] {
            assert_eq!(off(bad), None, "{bad}");
        }
    }

    type Edit = fn(&mut Sidecar);

    /// One edit per validation rule, with the message it gives.
    fn validation_cases() -> Vec<(Edit, &'static str)> {
        vec![
            (|s| s.schema = 2, "schema must be 1"),
            (|s| s.created_at.clear(), "createdAt must not be empty"),
            (
                |s| s.input.channels = 0,
                "input.channels must be at least 1",
            ),
            (
                |s| s.input.channel = 1,
                "input.channel must be below input.channels",
            ),
            (
                |s| s.input.sample_rate = 0,
                "input.sampleRate must be at least 1",
            ),
            (
                |s| s.output.base_latency = -0.1,
                "output.baseLatency must be a finite number at or above 0",
            ),
            (
                |s| s.output.base_latency = f64::NAN,
                "output.baseLatency must be a finite number at or above 0",
            ),
            (
                |s| s.output.output_latency = Some(f64::INFINITY),
                "output.outputLatency must be null or a finite number at or above 0",
            ),
            (
                |s| s.tempo.bpm = 0.0,
                "tempo.bpm must be a finite number above 0",
            ),
            (
                |s| s.tempo.cps = -1.0,
                "tempo.cps must be a finite number above 0",
            ),
            (
                |s| s.tempo.beats_per_bar = f64::NAN,
                "tempo.beatsPerBar must be a finite number above 0",
            ),
            (
                |s| s.tempo.meter = "4".into(),
                "tempo.meter must be N/D, for example 4/4",
            ),
            (
                |s| s.tempo.meter = "0/4".into(),
                "tempo.meter must be N/D, for example 4/4",
            ),
            (
                |s| s.tempo.meter = "4/04".into(),
                "tempo.meter must be N/D, for example 4/4",
            ),
            (
                |s| s.tempo.meter = "4/4/4".into(),
                "tempo.meter must be N/D, for example 4/4",
            ),
            (
                |s| s.tempo.meter = " 4/4".into(),
                "tempo.meter must be N/D, for example 4/4",
            ),
            (
                |s| {
                    s.backing.song = false;
                    s.backing.metronome = false;
                    s.timing.bar1_frame = None;
                },
                "backing.countIn requires backing.song or backing.metronome",
            ),
            (
                |s| s.backing.song_path = Some(String::new()),
                "backing.songPath must be null or a non-empty string",
            ),
            (
                |s| s.backing.start_section = Some(String::new()),
                "backing.startSection must be null or a non-empty string",
            ),
            (
                |s| s.backing.loop_section = Some(String::new()),
                "backing.loopSection must be null or a non-empty string",
            ),
            (
                |s| s.timing.bar1_frame = Some(-1.0),
                "timing.bar1Frame must be null or a finite number at or above 0",
            ),
            (
                |s| {
                    s.backing.song = false;
                    s.backing.metronome = false;
                    s.backing.count_in = false;
                },
                "timing.bar1Frame requires backing.song or backing.metronome",
            ),
            (
                |s| s.timing.bar1_host_ns = None,
                "timing.bar1Frame requires timing.bar1HostNs",
            ),
            (
                |s| s.timing.anchors = vec![(5, HostNs(1)), (5, HostNs(2))],
                "timing.anchors frames must be strictly increasing",
            ),
            (
                |s| s.timing.anchors = vec![(5, HostNs(2)), (6, HostNs(1))],
                "timing.anchors hostNs must not decrease",
            ),
            (
                |s| s.timing.output_stamps = vec![(-0.5, HostNs(1))],
                "timing.outputStamps contextTime must be a finite number at or above 0",
            ),
        ]
    }

    #[test]
    fn each_validation_rule_fails() {
        assert_eq!(sidecar().validate(), Ok(()));
        for (edit, message) in validation_cases() {
            let mut s = sidecar();
            edit(&mut s);
            assert_eq!(s.validate(), Err(message.to_string()));
        }
    }

    #[test]
    fn sanitize_makes_every_part_but_input_and_schema_valid() {
        let mut valid = sidecar();
        assert!(!valid.sanitize());
        assert_eq!(valid, sidecar());
        for (edit, message) in validation_cases() {
            let mut s = sidecar();
            edit(&mut s);
            let fixable = !(message.starts_with("schema") || message.starts_with("input."));
            assert_eq!(s.sanitize(), fixable, "{message}");
            if fixable {
                assert_eq!(s.validate(), Ok(()), "{message}");
            }
        }
        // `setcps(0)`: only the tempo is replaced.
        let mut s = sidecar();
        s.tempo.cps = 0.0;
        assert!(s.sanitize());
        assert_eq!(s.tempo, SidecarTempo::placeholder());
        assert_eq!((s.backing, s.timing), (sidecar().backing, sidecar().timing));
    }

    #[test]
    fn validation_accepts_the_fallback_values() {
        let mut s = sidecar();
        s.input.device_id.clear();
        s.input.device_name.clear();
        s.output = SidecarOutput {
            base_latency: 0.0,
            output_latency: None,
        };
        s.timing = SidecarTiming {
            bar1_frame: None,
            bar1_host_ns: None,
            clock_offset_ns: SidecarClockOffsets {
                start: None,
                end: None,
            },
            anchors: vec![(0, HostNs(5))],
            output_stamps: vec![],
        };
        assert_eq!(s.validate(), Ok(()));
        // Song and metronome both off, no count-in, tempo still saved.
        s.backing = SidecarBacking {
            song: false,
            metronome: false,
            count_in: false,
            song_path: None,
            start_section: None,
            loop_section: None,
        };
        s.timing.anchors.clear();
        assert_eq!(s.validate(), Ok(()));
        // A bar1HostNs without a frame (outside the take) is fine; equal host times too.
        s.backing.song = true;
        s.timing.bar1_host_ns = Some(HostNs(10));
        s.timing.anchors = vec![(0, HostNs(5)), (1, HostNs(5))];
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn schema_is_checked_first() {
        let unsupported = |text: &str| match parse_sidecar(text.as_bytes(), "recordings/m.json") {
            Err(ProjectError::UnsupportedSidecarSchema { found }) => Some(found),
            other => panic!("{text}: {other:?}"),
        };
        assert_eq!(
            unsupported(r#"{"schema": 2, "tempo": 5}"#).as_deref(),
            Some("2")
        );
        assert_eq!(unsupported(r#"{"schema": "1"}"#).as_deref(), Some("\"1\""));
        assert_eq!(unsupported(r#"{"schema": 1.5}"#).as_deref(), Some("1.5"));
        let err = ProjectError::UnsupportedSidecarSchema { found: "2".into() };
        assert_eq!(
            err.to_string(),
            "unsupported memo sidecar schema 2 (this app reads schema 1)"
        );
        for text in [r#"{"tempo": 5}"#, "[1]", "1"] {
            let err = parse_sidecar(text.as_bytes(), "recordings/m.json").unwrap_err();
            assert_eq!(
                err.to_string(),
                "invalid memo sidecar: recordings/m.json: missing schema",
                "{text}"
            );
        }
        let err = parse_sidecar(br#"{"schema": 1}"#, "recordings/m.json").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("invalid memo sidecar: recordings/m.json: missing field"),
            "{err}"
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let mut v = sidecar_json(&sidecar());
        v["future"] = serde_json::json!({ "x": 1 });
        v["tempo"]["swing"] = serde_json::json!(0.6);
        let text = serde_json::to_vec(&v).unwrap();
        assert_eq!(parse_sidecar(&text, "r").unwrap(), sidecar());
    }

    #[test]
    fn a_type_error_names_the_file() {
        let mut v = sidecar_json(&sidecar());
        v["timing"]["bar1HostNs"] = serde_json::json!(12);
        let err = parse_sidecar(&serde_json::to_vec(&v).unwrap(), "recordings/m.json").unwrap_err();
        assert!(matches!(err, ProjectError::InvalidSidecar(_)));
        assert!(
            err.to_string()
                .starts_with("invalid memo sidecar: recordings/m.json: invalid type"),
            "{err}"
        );
        let mut v = sidecar_json(&sidecar());
        v["input"]["channel"] = serde_json::json!(3);
        let err = parse_sidecar(&serde_json::to_vec(&v).unwrap(), "recordings/m.json").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid memo sidecar: recordings/m.json: input.channel must be below input.channels"
        );
    }

    #[test]
    fn read_and_write_sidecar() {
        let (_t, p) = new_project();
        assert_eq!(p.read_sidecar("m").unwrap(), None);
        assert!(matches!(
            p.write_sidecar("m", &sidecar()),
            Err(ProjectError::RecordingNotFound(_))
        ));
        put(&p, "m.wav", &mono(1));
        p.write_sidecar("m", &sidecar()).unwrap();
        let mut changed = sidecar();
        changed.xruns = 3;
        p.write_sidecar("m", &changed).unwrap();
        assert_eq!(p.read_sidecar("m").unwrap(), Some(changed));
        let mut bad = sidecar();
        bad.input.sample_rate = 0;
        assert!(matches!(
            p.write_sidecar("m", &bad),
            Err(ProjectError::InvalidSidecar(_))
        ));
        assert!(matches!(
            p.read_sidecar("m.n"),
            Err(ProjectError::InvalidRecordingName(_))
        ));
        fs::create_dir(rec(&p).join("d.json")).unwrap();
        assert_eq!(
            p.read_sidecar("d").unwrap_err().to_string(),
            "invalid memo sidecar: recordings/d.json: not a file"
        );
    }

    #[test]
    fn a_huge_sidecar_is_refused() {
        let (_t, p) = new_project();
        let mut big = br#"{"schema": 1, "pad": ""#.to_vec();
        big.resize(MAX_SIDECAR_BYTES as usize + 1, b' ');
        put(&p, "m.json", &big);
        assert_eq!(
            p.read_sidecar("m").unwrap_err().to_string(),
            "invalid memo sidecar: recordings/m.json: file too large"
        );
    }

    // --- repair ---

    #[test]
    fn repair_drops_a_torn_frame_and_fixes_sizes() {
        let (_t, p) = new_project();
        // hound's placeholder sizes (0), two whole frames and half a third.
        let partial = put_partial(
            &p,
            "take",
            &wav(16, false, 1, 48_000, 4, Some(0), &[7u8; 10]),
        );
        let mtime = mtime_secs(&fs::metadata(&partial).unwrap());
        let report = p.repair_partials().unwrap();
        assert_eq!(
            report,
            RepairReport {
                recovered: vec![RecoveredRecording {
                    stem: "take".into(),
                    frames: 2
                }],
                discarded: vec![],
                skipped: vec![],
            }
        );
        assert_eq!(names(&p), ["take.json", "take.wav"]);
        let bytes = fs::read(rec(&p).join("take.wav")).unwrap();
        assert_eq!(bytes.len(), 52);
        assert_eq!(&bytes[4..8], &44u32.to_le_bytes());
        assert_eq!(&bytes[40..44], &8u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[7u8; 8]);
        let entry = &p.recordings().unwrap()[0];
        assert_eq!(entry.frames, Some(2));
        let s = entry
            .sidecar
            .clone()
            .expect("the recovered sidecar validates");
        assert!(s.recovered);
        assert_eq!(s.input.sample_rate, 48_000);
        assert_eq!(s.tempo.meter, "4/4");
        assert_eq!(s.timing.bar1_frame, None);
        assert_eq!(s.app.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(s.created_at, format_rfc3339_utc(mtime));
    }

    #[test]
    fn repair_finds_data_after_an_extensible_fmt() {
        let (_t, p) = new_project();
        put_partial(&p, "x", &wav(40, false, 2, 44_100, 8, Some(0), &[1u8; 85]));
        let report = p.repair_partials().unwrap();
        assert_eq!(report.recovered[0].frames, 10);
        let bytes = fs::read(rec(&p).join("x.wav")).unwrap();
        assert_eq!(bytes.len(), 68 + 80);
        assert_eq!(&bytes[64..68], &80u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &(68u32 + 80 - 8).to_le_bytes());
        assert_eq!(read_wav_header(&rec(&p).join("x.wav")).unwrap().frames, 10);
    }

    #[test]
    fn repair_discards_partials_without_audio() {
        let (_t, p) = new_project();
        put_partial(&p, "empty", b"");
        put_partial(&p, "short", &mono(0)[..30]);
        put_partial(&p, "header", &mono(0));
        put_partial(&p, "torn", &wav(16, false, 1, 48_000, 4, Some(0), &[0; 3]));
        let report = p.repair_partials().unwrap();
        assert_eq!(
            report.discarded,
            [
                ".empty.wav.partial",
                ".header.wav.partial",
                ".short.wav.partial",
                ".torn.wav.partial"
            ]
        );
        assert_eq!((report.recovered.len(), report.skipped.len()), (0, 0));
        assert_eq!(names(&p), Vec::<String>::new());
    }

    #[test]
    fn repair_skips_unparseable_files_untouched() {
        let (_t, p) = new_project();
        let junk = vec![b'x'; 100];
        put_partial(&p, "junk", &junk);
        let mut no_fmt = b"RIFF\0\0\0\0WAVEdata\x40\0\0\0".to_vec();
        no_fmt.extend_from_slice(&[0u8; 64]);
        put_partial(&p, "nofmt", &no_fmt);
        let mut zero_align = wav(16, false, 1, 48_000, 0, None, &[0; 8]);
        zero_align.extend_from_slice(&[0; 40]);
        put_partial(&p, "zero", &zero_align);
        let report = p.repair_partials().unwrap();
        assert_eq!(
            report.skipped,
            [
                RepairSkip {
                    file: ".junk.wav.partial".into(),
                    reason: "not a RIFF/WAVE file".into()
                },
                RepairSkip {
                    file: ".nofmt.wav.partial".into(),
                    reason: "no fmt chunk before data".into()
                },
                RepairSkip {
                    file: ".zero.wav.partial".into(),
                    reason: "block align 0".into()
                },
            ]
        );
        assert_eq!(fs::read(rec(&p).join(".junk.wav.partial")).unwrap(), junk);
        assert_eq!(
            fs::read(rec(&p).join(".nofmt.wav.partial")).unwrap(),
            no_fmt
        );
    }

    #[test]
    fn repair_picks_a_recovered_name_when_taken() {
        let (_t, p) = new_project();
        put(&p, "take.wav", b"old");
        put_partial(&p, "take", &mono(2));
        put(&p, "b.json", b"old");
        put(&p, "b-recovered.wav", b"old");
        put_partial(&p, "b", &mono(3));
        let long = "z".repeat(64);
        put(&p, &format!("{long}.wav"), b"old");
        put_partial(&p, &long, &mono(1));
        let report = p.repair_partials().unwrap();
        let mut got: Vec<(String, u64)> = report
            .recovered
            .into_iter()
            .map(|r| (r.stem, r.frames))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("b-recovered-2".to_string(), 3),
                ("take-recovered".to_string(), 2),
                (format!("{}-recovered", "z".repeat(54)), 1),
            ]
        );
        assert_eq!(fs::read(rec(&p).join("take.wav")).unwrap(), b"old");
    }

    #[test]
    fn repair_skips_a_partial_in_use() {
        let (_t, p) = new_project();
        put(&p, ".live.wav.partial", &mono(4));
        let report = p.repair_partials().unwrap();
        assert_eq!(
            report.skipped,
            [RepairSkip {
                file: ".live.wav.partial".into(),
                reason: "in use".into()
            }]
        );
        assert_eq!(names(&p), [".live.wav.partial"]);
    }

    #[test]
    fn repair_ignores_other_files() {
        let (_t, p) = new_project();
        put(&p, "m.wav", &mono(4));
        put(&p, ".DS_Store", b"x");
        put(&p, ".bad name .wav.partial", &mono(4));
        let target = put(&p, "target", &mono(4));
        set_old(&target);
        test_symlink!(&target, rec(&p).join(".link.wav.partial")).unwrap();
        fs::create_dir(rec(&p).join(".dir.wav.partial")).unwrap();
        assert_eq!(p.repair_partials().unwrap(), RepairReport::default());
        // A second run over nothing to do is a no-op too.
        assert_eq!(p.repair_partials().unwrap(), RepairReport::default());
        fs::remove_dir_all(rec(&p)).unwrap();
        assert_eq!(p.repair_partials().unwrap(), RepairReport::default());
    }

    #[test]
    fn fixing_sizes_twice_changes_nothing() {
        // even data, odd data with a 3-byte frame, and 8-bit mono (block align 1, odd)
        for (align, len) in [(4u16, 10usize), (3, 10), (1, 5)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("p");
            fs::write(
                &path,
                wav(16, true, 1, 8_000, align, Some(0), &vec![9u8; len]),
            )
            .unwrap();
            let fix = || {
                let mut file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .unwrap();
                let len = file.metadata().unwrap().len();
                fix_partial_sizes(&mut file, len).unwrap()
            };
            let first = fix();
            let once = fs::read(&path).unwrap();
            assert_eq!(fix(), first);
            assert_eq!(fs::read(&path).unwrap(), once, "align {align}");
            let frames = (len / align as usize) as u64;
            assert_eq!(first, (frames, 8_000));
            let data_bytes = frames * u64::from(align);
            assert_eq!(once.len() as u64, 56 + data_bytes + (data_bytes & 1));
            assert_eq!(read_wav_header(&path).unwrap().frames, frames);
        }
    }

    // --- symlinks ---

    #[test]
    fn a_symlinked_recordings_folder_is_refused_everywhere() {
        let (tmp, p) = new_project();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::write(elsewhere.join("m.wav"), mono(2)).unwrap();
        fs::write(elsewhere.join(".n.wav.partial"), mono(2)).unwrap();
        fs::remove_dir_all(rec(&p)).unwrap();
        test_symlink!(&elsewhere, rec(&p)).unwrap();
        let outside = |r: Result<(), ProjectError>| {
            assert!(
                matches!(r, Err(ProjectError::PathOutsideProject(ref s)) if s.starts_with("recordings/")),
                "{r:?}"
            );
        };
        outside(p.recordings().map(drop));
        outside(p.default_recording_stem(None).map(drop));
        outside(p.recording_partial_path("x").map(drop));
        outside(p.read_recording_bytes("m").map(drop));
        outside(p.read_sidecar("m").map(drop));
        outside(p.write_sidecar("m", &sidecar()));
        outside(p.finish_recording("n", &sidecar()).map(drop));
        outside(p.rename_recording("m", "k").map(drop));
        outside(p.delete_recording("m"));
        outside(p.repair_partials().map(drop));
        let mut left: Vec<String> = fs::read_dir(&elsewhere)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, [".n.wav.partial", "m.wav"]);
    }

    #[test]
    fn a_recordings_file_is_an_io_error() {
        let (_t, p) = new_project();
        fs::remove_dir_all(rec(&p)).unwrap();
        fs::write(rec(&p), b"x").unwrap();
        assert!(matches!(p.recordings(), Err(ProjectError::Io { .. })));
        assert!(matches!(
            p.recording_partial_path("x"),
            Err(ProjectError::Io { .. })
        ));
    }

    #[test]
    fn symlinked_memo_files_are_refused() {
        let (tmp, p) = new_project();
        let target = tmp.path().join("t");
        fs::write(&target, mono(2)).unwrap();
        test_symlink!(&target, rec(&p).join("w.wav")).unwrap();
        put(&p, "j.wav", &mono(2));
        test_symlink!(&target, rec(&p).join("j.json")).unwrap();
        let outside = |r: Result<(), ProjectError>, name: &str| {
            assert!(
                matches!(r, Err(ProjectError::PathOutsideProject(ref s)) if s == &format!("recordings/{name}")),
                "{name}: {r:?}"
            );
        };
        outside(p.read_recording_bytes("w").map(drop), "w.wav");
        outside(p.write_sidecar("w", &sidecar()), "w.wav");
        outside(p.rename_recording("w", "k").map(drop), "w.wav");
        outside(p.delete_recording("w"), "w.wav");
        outside(p.read_sidecar("j").map(drop), "j.json");
        outside(p.write_sidecar("j", &sidecar()), "j.json");
        outside(p.rename_recording("j", "k").map(drop), "j.json");
        outside(p.delete_recording("j"), "j.json");
        // A symlinked partial is refused by finish.
        test_symlink!(&target, rec(&p).join(".f.wav.partial")).unwrap();
        outside(
            p.finish_recording("f", &sidecar()).map(drop),
            ".f.wav.partial",
        );
        // The listing keeps the real memo and reports its sidecar as unreadable.
        let list = p.recordings().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].stem, "j");
        assert!(!list[0].has_sidecar);
        assert!(list[0].sidecar_error.is_some());
        assert!(target.exists());
    }
}

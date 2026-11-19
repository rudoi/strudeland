//! The project folder: layout, `project.json`, songs and path-scoped file access.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::template::{MAIN_SONG, new_song};

/// The `project.json` schema this crate reads and writes.
pub const SCHEMA: u32 = 1;

/// The manifest file at the project root.
pub const MANIFEST_FILE: &str = "project.json";
/// The folder under the project root that holds the songs.
pub const SONGS_DIR: &str = "songs";
/// The extension every song file name ends with.
pub const SONG_EXT: &str = ".js";
pub(crate) const MAX_STEM_LEN: usize = 64;
/// Entries a create target may already contain and still count as empty.
const IGNORED_IN_EMPTY_TARGET: [&str; 2] = [".git", ".DS_Store"];

/// The contents of `project.json`. Unknown keys are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One song file in `songs/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongEntry {
    /// `songs/x.js`: relative to the project root, always forward slashes.
    pub path: String,
}

/// What the app shows for an open project: its root, name and songs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// The canonical project folder.
    pub root: String,
    pub name: String,
    pub songs: Vec<SongEntry>,
}

/// An open project folder.
#[derive(Debug)]
pub struct Project {
    root: PathBuf,
    manifest: Manifest,
}

/// How [`link_noclobber`] placed the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placed {
    /// A second name: the caller removes the old one.
    Linked,
    /// Renamed: the old name is gone.
    Moved,
}

/// Tries a hard link first, then an exclusive move on filesystems without hard links.
pub(crate) fn link_noclobber(
    from: &Path,
    to: &Path,
    link: &impl Fn(&Path, &Path) -> io::Result<()>,
    exists: impl FnOnce() -> ProjectError,
) -> Result<Placed, ProjectError> {
    let result = match link(from, to) {
        Ok(()) => return Ok(Placed::Linked),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => move_noreplace(from, to),
    };
    result.map(|()| Placed::Moved).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            exists()
        } else {
            ProjectError::io(from, e)
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(windows)]
pub(crate) fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
        if units.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        units.push(0);
        Ok(units)
    };
    let from = wide(from)?;
    let to = wide(to)?;
    // SAFETY: both buffers are live and NUL-terminated. Zero flags means never replace.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn move_noreplace(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Returns whether the target is another spelling of the same directory entry. Two distinct
/// directory entries for one inode can themselves differ only in case on case-sensitive volumes.
pub(crate) fn is_case_only_rename(old: &Path, new: &Path) -> io::Result<bool> {
    let (Some(old_name), Some(new_name), Some(dir)) =
        (old.file_name(), new.file_name(), new.parent())
    else {
        return Ok(false);
    };
    if !old_name.eq_ignore_ascii_case(new_name) || new_name == old_name {
        return Ok(false);
    }
    let new_id = match file_id::get_file_id(new) {
        Ok(id) => id,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if new_id != file_id::get_file_id(old)? {
        return Ok(false);
    }
    let mut has_old = false;
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        if name == new_name {
            return Ok(false);
        }
        has_old |= name == old_name;
    }
    Ok(has_old)
}

/// Filesystem lookup may find a different directory entry through case folding or an alias.
pub(crate) fn has_exact_name(path: &Path) -> Result<bool, ProjectError> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(false);
    };
    for entry in fs::read_dir(dir).map_err(|e| ProjectError::io(dir, e))? {
        if entry.map_err(|e| ProjectError::io(dir, e))?.file_name() == name {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Junctions, mount points and other Windows reparse points can redirect filesystem operations.
pub(crate) fn redirects(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

impl Project {
    /// Creates a new project in `dir`, which must not exist or must be empty (ignoring `.git` and
    /// `.DS_Store`). Writes `project.json`, `songs/main.js`, `.gitignore`, `instruments/`,
    /// `recordings/` and `renders/`.
    pub fn create(dir: &Path, name: &str) -> Result<Project, ProjectError> {
        if name.trim().is_empty() {
            return Err(ProjectError::InvalidName(
                "the name must not be empty".to_string(),
            ));
        }
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(|e| ProjectError::io(dir, e))?;
                    let file_name = entry.file_name();
                    let ignored = file_name
                        .to_str()
                        .is_some_and(|n| IGNORED_IN_EMPTY_TARGET.contains(&n));
                    if !ignored {
                        return Err(ProjectError::TargetNotEmpty(dir.to_path_buf()));
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(dir).map_err(|e| ProjectError::io(dir, e))?;
            }
            Err(e) => return Err(ProjectError::io(dir, e)),
        }
        let root = dir.canonicalize().map_err(|e| ProjectError::io(dir, e))?;

        let manifest = Manifest {
            schema: SCHEMA,
            name: name.to_string(),
            extra: serde_json::Map::new(),
        };
        let mut json =
            serde_json::to_string_pretty(&manifest).expect("a manifest always serialises to JSON");
        json.push('\n');
        write_new(&root.join(MANIFEST_FILE), &json)?;
        write_new(
            &root.join(".gitignore"),
            "renders/\n/recordings/*\n!/recordings/.gitkeep\n",
        )?;
        for sub in [SONGS_DIR, "instruments", "recordings", "renders"] {
            let path = root.join(sub);
            fs::create_dir(&path).map_err(|e| ProjectError::io(&path, e))?;
        }
        write_new(&root.join("instruments/.gitkeep"), "")?;
        write_new(&root.join("recordings/.gitkeep"), "")?;
        write_new(&root.join(SONGS_DIR).join("main.js"), MAIN_SONG)?;

        Ok(Project { root, manifest })
    }

    /// Opens the project in `dir`. The folder must contain a valid `project.json`.
    pub fn open(dir: &Path) -> Result<Project, ProjectError> {
        let root = dir.canonicalize().map_err(|e| ProjectError::io(dir, e))?;
        Project::load(root)
    }

    /// Finds the project a song file belongs to. The canonical path's parent directory must be
    /// named `songs`, the file must be a song file name, and the grandparent must hold a valid
    /// `project.json`. Returns the project and the song's stem. Never errors: an unreadable path
    /// or a broken project is simply "not a project".
    pub fn find(song_path: &Path) -> Option<(Project, String)> {
        let canonical = song_path.canonicalize().ok()?;
        let name = canonical.file_name()?.to_str()?;
        if !is_song_file_name(name) {
            return None;
        }
        let stem = name.strip_suffix(SONG_EXT)?.to_string();
        let songs = canonical.parent()?;
        if songs.file_name()? != SONGS_DIR {
            return None;
        }
        let project = Project::open(songs.parent()?).ok()?;
        Some((project, stem))
    }

    /// Finds the project `dir` is in: canonicalizes `dir`, then walks up from it to the nearest
    /// folder holding a regular `project.json` and opens that folder. The nearest manifest wins
    /// even when it is broken (then `None`). Never errors.
    pub fn find_from_dir(dir: &Path) -> Option<Project> {
        let start = dir.canonicalize().ok()?;
        let mut cur = Some(start.as_path());
        while let Some(dir) = cur {
            if fs::symlink_metadata(dir.join(MANIFEST_FILE)).is_ok_and(|m| m.is_file()) {
                return Project::open(dir).ok();
            }
            cur = dir.parent();
        }
        None
    }

    /// The output path `renders/<stem>[-<suffix>].<ext>` inside this project. `ext` is one of
    /// `wav`, `mp3`, `ogg`, `opus`; `suffix` (a section name, or a length tag such as `len16`) is
    /// slugged to `[A-Za-z0-9_-]` (each run of other characters becomes one `-`, the ends are
    /// trimmed). Creates nothing: [`Project::write_render`]
    /// creates `renders/` when it writes. The returned path does not exist yet unless a previous
    /// render wrote it.
    pub fn render_path(
        &self,
        stem: &str,
        suffix: Option<&str>,
        ext: &str,
    ) -> Result<PathBuf, ProjectError> {
        if !matches!(ext, "wav" | "mp3" | "ogg" | "opus") {
            return Err(ProjectError::InvalidRenderName(ext.to_string()));
        }
        if !is_new_stem(stem) {
            return Err(ProjectError::InvalidSongName(stem.to_string()));
        }
        let name = match suffix {
            None => format!("{stem}.{ext}"),
            Some(suffix) => {
                let slug = slugify(suffix);
                if slug.is_empty() {
                    return Err(ProjectError::InvalidRenderName(suffix.to_string()));
                }
                format!("{stem}-{slug}.{ext}")
            }
        };
        let dir = self.root.join(RENDERS_DIR);
        if fs::symlink_metadata(&dir).is_ok() {
            check_dir(&dir, RENDERS_DIR, &name)?;
        }
        let path = dir.join(&name);
        if fs::symlink_metadata(&path).is_ok_and(|m| redirects(&m)) {
            return Err(ProjectError::PathOutsideProject(format!("renders/{name}")));
        }
        Ok(path)
    }

    /// Writes `bytes` atomically to `path`, a file directly in this project's `renders/` (as
    /// [`Project::render_path`] returns it), creating `renders/` when it is missing. An existing
    /// file is replaced. A path elsewhere, or a symlinked `renders/` or file, is
    /// `PathOutsideProject`.
    pub fn write_render(&self, path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
        let dir = self.root.join(RENDERS_DIR);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let outside = || ProjectError::PathOutsideProject(path.display().to_string());
        #[cfg(windows)]
        if !windows_file_name(&name) {
            return Err(outside());
        }
        if name.is_empty() || path.parent() != Some(dir.as_path()) {
            return Err(outside());
        }
        if fs::symlink_metadata(&dir).is_ok_and(|m| redirects(&m)) {
            return Err(outside());
        }
        fs::create_dir_all(&dir).map_err(|e| ProjectError::io(&dir, e))?;
        check_dir(&dir, RENDERS_DIR, &name)?;
        if fs::symlink_metadata(path).is_ok_and(|m| redirects(&m)) {
            return Err(outside());
        }
        atomic_write_bytes(path, bytes)
    }

    /// Re-reads and re-validates `project.json` of this project's folder, returning a fresh
    /// `Project` with the same root. `self` is left as it was, so a caller can keep it when the
    /// manifest no longer parses.
    pub fn reload(&self) -> Result<Project, ProjectError> {
        Project::load(self.root.clone())
    }

    /// Reads the manifest of the canonical folder `root`.
    fn load(root: PathBuf) -> Result<Project, ProjectError> {
        let manifest_path = root.join(MANIFEST_FILE);
        if fs::symlink_metadata(&manifest_path).is_ok_and(|m| redirects(&m)) {
            return Err(ProjectError::PathOutsideProject(MANIFEST_FILE.into()));
        }
        let text = match fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ProjectError::NotAProject(root));
            }
            Err(e) => return Err(ProjectError::io(&manifest_path, e)),
        };
        let manifest = parse_manifest(&text)?;
        Ok(Project { root, manifest })
    }

    /// The canonical project folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The root, name and current songs (lists `songs/`).
    pub fn info(&self) -> Result<ProjectInfo, ProjectError> {
        Ok(ProjectInfo {
            root: self.root.to_string_lossy().into_owned(),
            name: self.manifest.name.clone(),
            songs: self.songs()?,
        })
    }

    /// Every `songs/*.js` regular file, sorted byte-wise by file name. Directories, symlinks,
    /// dotfiles and names that are not UTF-8 are skipped. A missing `songs/` gives an empty vec.
    pub fn songs(&self) -> Result<Vec<SongEntry>, ProjectError> {
        list_songs(&self.root)
    }

    /// Reads the song at `rel` (`songs/<name>.js`).
    pub fn read(&self, rel: &str) -> Result<String, ProjectError> {
        let path = self.resolve(rel)?;
        fs::read_to_string(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ProjectError::SongNotFound(rel.to_string())
            } else {
                ProjectError::io(&path, e)
            }
        })
    }

    /// Writes the song at `rel` atomically: a temp file in `songs/`, then a rename over the song.
    /// The song need not exist yet.
    pub fn write(&self, rel: &str, contents: &str) -> Result<(), ProjectError> {
        let path = self.resolve(rel)?;
        atomic_write(&path, contents)
    }

    /// Creates `songs/<stem>.js` from [`new_song`] and returns its relative path. Never overwrites.
    pub fn create_song(&self, stem: &str) -> Result<String, ProjectError> {
        if !is_new_stem(stem) {
            return Err(ProjectError::InvalidSongName(stem.to_string()));
        }
        let rel = format!("{SONGS_DIR}/{stem}{SONG_EXT}");
        let path = self.song_target(&rel, false)?;
        atomic_create_bytes(&path, new_song(stem).as_bytes()).map_err(|e| match e {
            ProjectError::Io { source, .. } if source.kind() == io::ErrorKind::AlreadyExists => {
                ProjectError::SongExists(rel.clone())
            }
            other => other,
        })?;
        Ok(rel)
    }

    /// Renames `old_rel` to `songs/<new_stem>.js` without ever replacing an existing song, and
    /// returns the new relative path.
    pub fn rename_song(&self, old_rel: &str, new_stem: &str) -> Result<String, ProjectError> {
        self.rename_song_with(
            old_rel,
            new_stem,
            |from, to| fs::hard_link(from, to),
            |path| fs::remove_file(path),
        )
    }

    /// [`Project::rename_song`] with the hard-link and remove steps injectable, so a test can
    /// simulate a filesystem without hard links or a failing removal.
    fn rename_song_with(
        &self,
        old_rel: &str,
        new_stem: &str,
        link: impl Fn(&Path, &Path) -> io::Result<()>,
        remove: impl Fn(&Path) -> io::Result<()>,
    ) -> Result<String, ProjectError> {
        if !is_new_stem(new_stem) {
            return Err(ProjectError::InvalidSongName(new_stem.to_string()));
        }
        // The old name may be any listed song, even one whose stem would not pass validation.
        let old = self.resolve(old_rel)?;
        let new_rel = format!("{SONGS_DIR}/{new_stem}{SONG_EXT}");
        let new = self.song_target(&new_rel, false)?;

        match fs::symlink_metadata(&old) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(ProjectError::SongNotFound(old_rel.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ProjectError::SongNotFound(old_rel.to_string()));
            }
            Err(e) => return Err(ProjectError::io(&old, e)),
        };
        if new_rel == old_rel {
            return Err(ProjectError::SongExists(new_rel));
        }

        if is_case_only_rename(&old, &new).map_err(|e| ProjectError::io(&old, e))? {
            fs::rename(&old, &new).map_err(|e| ProjectError::io(&old, e))?;
            return Ok(new_rel);
        }

        let exists = || ProjectError::SongExists(new_rel.clone());
        if link_noclobber(&old, &new, &link, exists)? == Placed::Linked
            && let Err(e) = remove(&old)
        {
            // Leave one name, not two. Best effort: the error reported is the first one.
            let _ = fs::remove_file(&new);
            return Err(ProjectError::io(&old, e));
        }
        Ok(new_rel)
    }

    /// Deletes a song file.
    pub fn delete_song(&self, rel: &str) -> Result<(), ProjectError> {
        let path = self.resolve(rel)?;
        fs::remove_file(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ProjectError::SongNotFound(rel.to_string())
            } else {
                ProjectError::io(&path, e)
            }
        })
    }

    /// Maps a project-relative song path to an absolute path, enforcing the scoping rule: `rel`
    /// must be exactly `songs/<name>.js` (forward slashes, non-empty name, not a dotfile), the
    /// canonical `songs/` must be `root/songs`, and an existing target must not be a symlink.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, ProjectError> {
        self.song_target(rel, true)
    }

    fn song_target(&self, rel: &str, exact: bool) -> Result<PathBuf, ProjectError> {
        let outside = || ProjectError::PathOutsideProject(rel.to_string());

        let mut parts = rel.split('/');
        let (Some(SONGS_DIR), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(outside());
        };
        if !is_song_file_name(name) {
            return Err(outside());
        }
        // Belt and braces: the split above already rules these out.
        let rel_path = Path::new(rel);
        if rel_path.is_absolute()
            || rel_path.components().count() != 2
            || !rel_path
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(outside());
        }

        let songs = self.root.join(SONGS_DIR);
        check_dir(&songs, SONGS_DIR, name)?;
        let target = songs.join(name);
        match fs::symlink_metadata(&target) {
            Ok(meta) if redirects(&meta) => Err(outside()),
            Ok(_) if exact && !has_exact_name(&target)? => {
                Err(ProjectError::SongNotFound(rel.into()))
            }
            Ok(_) => Ok(target),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(target),
            Err(e) => Err(ProjectError::io(&target, e)),
        }
    }
}

/// Lists regular song entries in the checked songs folder of a canonical project root.
pub fn list_songs(root: &Path) -> Result<Vec<SongEntry>, ProjectError> {
    let dir = root.join(SONGS_DIR);
    match check_dir(&dir, SONGS_DIR, "") {
        Ok(()) => {}
        Err(ProjectError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    }
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ProjectError::io(&dir, e)),
    };
    let mut songs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ProjectError::io(&dir, e))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_song_file_name(&name) {
            continue;
        }
        if !is_song_file(&entry.path()) {
            continue;
        }
        songs.push(SongEntry {
            path: format!("{SONGS_DIR}/{name}"),
        });
    }
    songs.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    Ok(songs)
}

/// Parses the schema before the name so an unsupported schema is reported first.
fn parse_manifest(text: &str) -> Result<Manifest, ProjectError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| ProjectError::InvalidManifest(e.to_string()))?;
    let Some(object) = value.as_object() else {
        return Err(ProjectError::InvalidManifest(
            "expected a JSON object".to_string(),
        ));
    };
    let Some(schema) = object.get("schema") else {
        return Err(ProjectError::InvalidManifest(
            "missing `schema`".to_string(),
        ));
    };
    if schema.as_u64() != Some(u64::from(SCHEMA)) {
        return Err(ProjectError::UnsupportedSchema {
            found: schema.to_string(),
        });
    }
    match object.get("name") {
        None => {
            return Err(ProjectError::InvalidManifest("missing `name`".to_string()));
        }
        Some(serde_json::Value::String(name)) if name.trim().is_empty() => {
            return Err(ProjectError::InvalidManifest(
                "`name` must not be empty".to_string(),
            ));
        }
        Some(serde_json::Value::String(_)) => {}
        Some(_) => {
            return Err(ProjectError::InvalidManifest(
                "`name` must be a string".to_string(),
            ));
        }
    }
    serde_json::from_value(value).map_err(|e| ProjectError::InvalidManifest(e.to_string()))
}

/// A song file name: `<name>.js` with a non-empty name that is not a dotfile.
pub fn is_song_file_name(name: &str) -> bool {
    if name.contains(['/', '\\', '\0']) {
        return false;
    }
    #[cfg(windows)]
    if !windows_file_name(name) {
        return false;
    }
    name.strip_suffix(SONG_EXT)
        .is_some_and(|stem| !stem.is_empty() && !stem.starts_with('.'))
}

/// If `rel` (relative to a project root) names a song, its `songs/<name>.js` form with forward
/// slashes. Only direct children of `songs/` whose name passes [`is_song_file_name`] count.
pub fn song_rel_path(rel: &Path) -> Option<String> {
    let mut parts = rel.components().map(|c| match c {
        Component::Normal(s) => s.to_str(),
        _ => None,
    });
    let (Some(Some(SONGS_DIR)), Some(Some(name)), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    is_song_file_name(name).then(|| format!("{SONGS_DIR}/{name}"))
}

/// Whether `path` is a song file as [`Project::songs`] lists it: a regular file, not a symlink
/// or directory. Any error reading its metadata counts as no.
pub fn is_song_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !redirects(&m))
}

/// An existing `<root>/<folder>` (`dir`) must be a real folder inside the project: a symlinked or
/// non-canonical folder is `PathOutsideProject("<folder>/<name>")`, a file in its place an `Io`
/// error (`NotADirectory`).
pub(crate) fn check_dir(dir: &Path, folder: &str, name: &str) -> Result<(), ProjectError> {
    let outside = || ProjectError::PathOutsideProject(format!("{folder}/{name}"));
    let meta = fs::symlink_metadata(dir).map_err(|e| ProjectError::io(dir, e))?;
    if redirects(&meta) {
        return Err(outside());
    }
    if !meta.is_dir() {
        let e = io::Error::new(io::ErrorKind::NotADirectory, "not a folder");
        return Err(ProjectError::io(dir, e));
    }
    let canonical = dir.canonicalize().map_err(|e| ProjectError::io(dir, e))?;
    if canonical != dir {
        return Err(outside());
    }
    Ok(())
}

/// `[A-Za-z0-9 _-]{1,64}` with no leading or trailing space.
pub(crate) fn is_valid_stem(stem: &str) -> bool {
    (!cfg!(windows) || !device_basename(stem))
        && (1..=MAX_STEM_LEN).contains(&stem.len())
        && !stem.starts_with(' ')
        && !stem.ends_with(' ')
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'-'))
}

/// Windows recognizes the basename as a device even when followed by an extension.
fn device_basename(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    base.strip_prefix("COM")
        .or_else(|| base.strip_prefix("LPT"))
        .is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

#[cfg(windows)]
fn windows_file_name(name: &str) -> bool {
    !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        && !device_basename(name)
}

pub(crate) fn is_new_stem(stem: &str) -> bool {
    is_valid_stem(stem) && !device_basename(stem)
}

/// Writes a file that must not exist yet.
fn write_new(path: &Path, contents: &str) -> Result<(), ProjectError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| ProjectError::io(path, e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| ProjectError::io(path, e))
}

const RENDERS_DIR: &str = "renders";

/// Each run of characters outside `[A-Za-z0-9_-]` becomes one `-`; leading and trailing `-` are
/// trimmed.
fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Bumped by every [`atomic_write`] so concurrent writes in this process never share a temp file.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `contents` to `.strudeland.tmp-<pid>-<n>` next to `path`, then renames it over `path`, so
/// a reader never sees a half-written file. `<n>` is unique per call, so concurrent writes of the
/// same file each get their own temp file and the last rename wins whole. The temp file is
/// removed if anything fails.
///
/// There is deliberately no `sync_all`: the rename alone makes the replacement atomic, and on
/// macOS a sync is `F_FULLFSYNC`, which costs tens to hundreds of milliseconds on a save the
/// editor awaits before evaluating. An app crash loses nothing (the data is already in the
/// kernel); only an OS crash or power loss can lose the most recent writes.
pub(crate) fn atomic_write(path: &Path, contents: &str) -> Result<(), ProjectError> {
    atomic_write_bytes(path, contents.as_bytes())
}

/// [`atomic_write`] for bytes. It does not create the parent directory.
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    write_and_publish(path, bytes, |from, to| fs::rename(from, to))
}

pub(crate) fn atomic_create_bytes(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    write_and_publish(path, bytes, move_noreplace)
}

fn write_and_publish(
    path: &Path,
    bytes: &[u8],
    publish: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), ProjectError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let (mut file, tmp) = loop {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".strudeland.tmp-{}-{n}", std::process::id()));
        match fs::File::create_new(&tmp) {
            Ok(file) => break (file, tmp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(ProjectError::io(path, e)),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        drop(file);
        publish(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        ProjectError::io(path, e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_symlink;
    use tempfile::TempDir;

    fn tempdir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn new_project() -> (TempDir, Project) {
        let tmp = tempdir();
        let project = Project::create(&tmp.path().join("p"), "p").expect("create");
        (tmp, project)
    }

    fn write_manifest(dir: &Path, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(MANIFEST_FILE), text).unwrap();
    }

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    // --- create ---

    #[test]
    fn create_writes_the_full_layout() {
        let tmp = tempdir();
        let dir = tmp.path().join("my-song");
        let project = Project::create(&dir, "my-song").unwrap();
        let root = project.root();

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("project.json")).unwrap()).unwrap();
        assert_eq!(
            manifest,
            serde_json::json!({ "schema": 1, "name": "my-song" })
        );
        assert_eq!(
            fs::read_to_string(root.join("songs/main.js")).unwrap(),
            MAIN_SONG
        );
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "renders/\n/recordings/*\n!/recordings/.gitkeep\n"
        );
        assert_eq!(fs::read(root.join("instruments/.gitkeep")).unwrap(), b"");
        assert_eq!(fs::read(root.join("recordings/.gitkeep")).unwrap(), b"");
        assert!(root.join("renders").is_dir());
        assert!(dir_names(&root.join("renders")).is_empty());
        assert_eq!(
            dir_names(root),
            [
                ".gitignore",
                "instruments",
                "project.json",
                "recordings",
                "renders",
                "songs"
            ]
        );
        assert_eq!(project.manifest().name, "my-song");
        assert_eq!(project.manifest().schema, SCHEMA);
    }

    #[test]
    fn open_preserves_an_existing_gitignore() {
        let tmp = tempdir();
        let project = Project::create(tmp.path(), "x").unwrap();
        let ignore = project.root().join(".gitignore");
        let existing_rules = "renders/\n/private-notes.md\n";
        fs::write(&ignore, existing_rules).unwrap();

        Project::open(project.root()).unwrap();

        assert_eq!(fs::read_to_string(ignore).unwrap(), existing_rules);
    }

    #[test]
    fn create_accepts_an_existing_empty_dir() {
        let tmp = tempdir();
        let project = Project::create(tmp.path(), "x").unwrap();
        assert!(project.root().join("project.json").is_file());
    }

    #[test]
    fn create_accepts_a_missing_dir_and_its_missing_parents() {
        let tmp = tempdir();
        let dir = tmp.path().join("a/b/c");
        Project::create(&dir, "c").unwrap();
        assert!(dir.join("songs/main.js").is_file());
    }

    #[test]
    fn create_ignores_git_and_ds_store() {
        let tmp = tempdir();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        fs::write(tmp.path().join(".DS_Store"), "x").unwrap();
        Project::create(tmp.path(), "x").unwrap();
        assert!(tmp.path().join(".git").is_dir());
        assert!(tmp.path().join("project.json").is_file());
    }

    #[test]
    fn create_rejects_a_non_empty_dir() {
        let tmp = tempdir();
        fs::write(tmp.path().join("notes.txt"), "hi").unwrap();
        let err = Project::create(tmp.path(), "x").unwrap_err();
        assert!(matches!(err, ProjectError::TargetNotEmpty(p) if p == tmp.path()));
        assert!(!tmp.path().join("project.json").exists());
    }

    #[test]
    fn create_rejects_an_existing_project() {
        let (_tmp, project) = new_project();
        let err = Project::create(project.root(), "again").unwrap_err();
        assert!(matches!(err, ProjectError::TargetNotEmpty(_)));
    }

    #[test]
    fn create_rejects_empty_and_whitespace_names() {
        let tmp = tempdir();
        for name in ["", "   ", "\t\n"] {
            let err = Project::create(&tmp.path().join("p"), name).unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidName(_)),
                "{name:?}: {err}"
            );
        }
        assert!(!tmp.path().join("p").exists());
    }

    #[test]
    fn create_fails_when_the_target_is_a_file() {
        let tmp = tempdir();
        let file = tmp.path().join("f");
        fs::write(&file, "x").unwrap();
        let err = Project::create(&file, "f").unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
    }

    // --- open ---

    #[test]
    fn open_round_trips_with_create() {
        let (_tmp, created) = new_project();
        let opened = Project::open(created.root()).unwrap();
        assert_eq!(opened.root(), created.root());
        assert_eq!(opened.manifest(), created.manifest());
    }

    #[test]
    fn open_rejects_a_folder_without_project_json() {
        let tmp = tempdir();
        let err = Project::open(tmp.path()).unwrap_err();
        assert!(matches!(err, ProjectError::NotAProject(_)), "{err}");
    }

    #[test]
    fn open_rejects_a_missing_folder() {
        let tmp = tempdir();
        let err = Project::open(&tmp.path().join("nope")).unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
    }

    #[test]
    fn open_rejects_invalid_manifests() {
        let cases = [
            "not json",
            "[1]",
            r#"{"schema":1}"#,
            r#"{"schema":1,"name":""}"#,
            r#"{"schema":1,"name":"  "}"#,
            r#"{"schema":1,"name":3}"#,
            r#"{"name":"x"}"#,
        ];
        for text in cases {
            let tmp = tempdir();
            write_manifest(tmp.path(), text);
            let err = Project::open(tmp.path()).unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidManifest(_)),
                "{text}: {err}"
            );
        }
    }

    #[test]
    fn open_rejects_unsupported_schemas() {
        let cases = [
            (r#"{"schema":2,"name":"x"}"#, "2"),
            (r#"{"schema":"1","name":"x"}"#, "\"1\""),
            (r#"{"schema":1.5,"name":"x"}"#, "1.5"),
            (r#"{"schema":0,"name":"x"}"#, "0"),
            (r#"{"schema":null,"name":"x"}"#, "null"),
            (r#"{"schema":2}"#, "2"),
        ];
        for (text, expected) in cases {
            let tmp = tempdir();
            write_manifest(tmp.path(), text);
            match Project::open(tmp.path()).unwrap_err() {
                ProjectError::UnsupportedSchema { found } => assert_eq!(found, expected, "{text}"),
                err => panic!("{text}: {err}"),
            }
        }
    }

    #[test]
    fn open_keeps_unknown_manifest_keys() {
        let tmp = tempdir();
        write_manifest(
            tmp.path(),
            r#"{"schema":1,"name":"x","songs":["b.js"],"future":{"a":true}}"#,
        );
        let project = Project::open(tmp.path()).unwrap();
        let extra = &project.manifest().extra;
        assert_eq!(extra.len(), 2);
        assert_eq!(extra["songs"], serde_json::json!(["b.js"]));
        assert_eq!(extra["future"], serde_json::json!({ "a": true }));
        let back = serde_json::to_value(project.manifest()).unwrap();
        assert_eq!(back["future"], serde_json::json!({ "a": true }));
        assert_eq!(back["schema"], 1);
    }

    #[test]
    fn open_tolerates_missing_optional_folders() {
        let (_tmp, project) = new_project();
        for sub in ["instruments", "recordings", "renders"] {
            fs::remove_dir_all(project.root().join(sub)).unwrap();
        }
        let opened = Project::open(project.root()).unwrap();
        assert_eq!(opened.songs().unwrap().len(), 1);
    }

    #[test]
    fn open_root_is_canonical() {
        let (tmp, project) = new_project();
        let dotted = tmp.path().join("p/../p/./");
        let opened = Project::open(&dotted).unwrap();
        assert_eq!(opened.root(), tmp.path().canonicalize().unwrap().join("p"));
        assert_eq!(opened.root(), project.root());

        let link = tmp.path().join("link");
        test_symlink!(project.root(), &link).unwrap();
        assert_eq!(Project::open(&link).unwrap().root(), project.root());
    }

    #[test]
    fn reload_picks_up_manifest_edits_and_keeps_the_root() {
        let (_tmp, project) = new_project();
        write_manifest(
            project.root(),
            r#"{"schema":1,"name":"renamed","future":1}"#,
        );
        let reloaded = project.reload().unwrap();
        assert_eq!(reloaded.root(), project.root());
        assert_eq!(reloaded.manifest().name, "renamed");
        assert_eq!(reloaded.manifest().extra["future"], 1);
        assert_eq!(project.manifest().name, "p");
    }

    #[test]
    fn reload_revalidates_the_manifest() {
        let (_tmp, project) = new_project();
        write_manifest(project.root(), r#"{"schema":1,"name":""}"#);
        assert!(matches!(
            project.reload().unwrap_err(),
            ProjectError::InvalidManifest(_)
        ));
        write_manifest(project.root(), r#"{"schema":2,"name":"x"}"#);
        assert!(matches!(
            project.reload().unwrap_err(),
            ProjectError::UnsupportedSchema { .. }
        ));
        fs::remove_file(project.root().join(MANIFEST_FILE)).unwrap();
        assert!(matches!(
            project.reload().unwrap_err(),
            ProjectError::NotAProject(_)
        ));
        assert_eq!(project.manifest().name, "p");
    }

    // --- songs ---

    #[test]
    fn songs_are_sorted_byte_wise() {
        let (_tmp, project) = new_project();
        let songs = project.root().join("songs");
        for name in ["b.js", "Z.js", "a b.js", "a.js", "_x.js", "10.js", "9.js"] {
            fs::write(songs.join(name), "").unwrap();
        }
        let paths: Vec<String> = project
            .songs()
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect();
        assert_eq!(
            paths,
            [
                "songs/10.js",
                "songs/9.js",
                "songs/Z.js",
                "songs/_x.js",
                "songs/a b.js",
                "songs/a.js",
                "songs/b.js",
                "songs/main.js",
            ]
        );
    }

    #[test]
    fn songs_skip_non_song_entries() {
        let (tmp, project) = new_project();
        let songs = project.root().join("songs");
        fs::create_dir(songs.join("dir.js")).unwrap();
        fs::write(songs.join("notes.txt"), "").unwrap();
        fs::write(songs.join("x.mjs"), "").unwrap();
        fs::write(songs.join(".js"), "").unwrap();
        fs::write(songs.join(".main.js.tmp-123"), "").unwrap();
        fs::write(songs.join(".hidden.js"), "").unwrap();
        let outside = tmp.path().join("outside.js");
        fs::write(&outside, "").unwrap();
        test_symlink!(&outside, songs.join("link.js")).unwrap();
        test_symlink!(songs.join("main.js"), songs.join("inner-link.js")).unwrap();
        fs::create_dir(songs.join("nested")).unwrap();
        fs::write(songs.join("nested/deep.js"), "").unwrap();

        let paths: Vec<String> = project
            .songs()
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect();
        assert_eq!(paths, ["songs/main.js"]);
    }

    #[test]
    fn songs_missing_folder_is_empty() {
        let (_tmp, project) = new_project();
        fs::remove_dir_all(project.root().join("songs")).unwrap();
        assert_eq!(project.songs().unwrap(), []);
    }

    #[test]
    fn songs_have_forward_slash_paths() {
        let (_tmp, project) = new_project();
        let songs = project.songs().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].path, "songs/main.js");
        assert!(!songs[0].path.contains('\\'));
    }

    #[test]
    fn song_entry_serialises_camel_case() {
        let entry = SongEntry {
            path: "songs/a.js".into(),
        };
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({ "path": "songs/a.js" })
        );
    }

    #[test]
    fn song_file_name_rule() {
        for name in ["a.js", "main.js", "a b.js", "_x.js", "a.b.js", "-.js"] {
            assert!(is_song_file_name(name), "{name:?}");
        }
        for name in [
            ".js",
            "..js",
            ".a.js",
            "a.js.bak",
            "a.mjs",
            "a",
            "",
            ".main.js.tmp-1",
        ] {
            assert!(!is_song_file_name(name), "{name:?}");
        }
    }

    #[test]
    fn song_rel_path_keeps_direct_song_children_only() {
        assert_eq!(
            song_rel_path(Path::new("songs/a.js")).as_deref(),
            Some("songs/a.js")
        );
        for rel in [
            "songs/..js",
            "songs/.js",
            "songs/.a.js",
            "songs/a/b.js",
            "songs",
            "a.js",
            "renders/a.js",
            "/songs/a.js",
            "./songs/a.js",
            "songs/../songs/a.js",
            "Songs/a.js",
        ] {
            assert_eq!(song_rel_path(Path::new(rel)), None, "{rel:?}");
        }
    }

    #[test]
    fn is_song_file_wants_a_regular_file() {
        let (tmp, project) = new_project();
        let songs = project.root().join("songs");
        assert!(is_song_file(&songs.join("main.js")));
        assert!(!is_song_file(&songs.join("missing.js")));
        fs::create_dir(songs.join("dir.js")).unwrap();
        assert!(!is_song_file(&songs.join("dir.js")));
        let outside = tmp.path().join("outside.js");
        fs::write(&outside, "").unwrap();
        test_symlink!(&outside, songs.join("link.js")).unwrap();
        assert!(!is_song_file(&songs.join("link.js")));
    }

    #[test]
    fn info_has_the_root_name_and_songs() {
        let (_tmp, project) = new_project();
        project.create_song("b").unwrap();
        let info = project.info().unwrap();
        assert_eq!(info.root, project.root().to_string_lossy());
        assert_eq!(info.name, "p");
        assert_eq!(info.songs, project.songs().unwrap());
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "root": project.root().to_string_lossy(),
                "name": "p",
                "songs": [{ "path": "songs/b.js" }, { "path": "songs/main.js" }],
            })
        );
    }

    // --- read / write ---

    #[test]
    fn read_write_round_trip() {
        let (_tmp, project) = new_project();
        assert_eq!(project.read("songs/main.js").unwrap(), MAIN_SONG);
        project.write("songs/main.js", "s(\"bd\")\n").unwrap();
        assert_eq!(project.read("songs/main.js").unwrap(), "s(\"bd\")\n");
    }

    #[test]
    fn write_overwrites_with_shorter_content() {
        let (_tmp, project) = new_project();
        project.write("songs/main.js", "x").unwrap();
        assert_eq!(
            fs::read_to_string(project.root().join("songs/main.js")).unwrap(),
            "x"
        );
    }

    #[test]
    fn write_creates_a_missing_song() {
        let (_tmp, project) = new_project();
        project.write("songs/new one.js", "hello").unwrap();
        assert_eq!(project.read("songs/new one.js").unwrap(), "hello");
        assert_eq!(project.songs().unwrap().len(), 2);
    }

    #[test]
    fn write_leaves_no_temp_file() {
        let (_tmp, project) = new_project();
        project.write("songs/main.js", "a").unwrap();
        project.write("songs/b.js", "b").unwrap();
        assert_eq!(
            dir_names(&project.root().join("songs")),
            ["b.js", "main.js"]
        );
    }

    #[test]
    fn write_fails_cleanly_when_the_target_is_a_directory() {
        let (_tmp, project) = new_project();
        let dir = project.root().join("songs/d.js");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("keep"), "").unwrap();
        let err = project.write("songs/d.js", "x").unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
        assert_eq!(
            dir_names(&project.root().join("songs")),
            ["d.js", "main.js"]
        );
    }

    #[test]
    fn read_missing_song_is_not_found() {
        let (_tmp, project) = new_project();
        let err = project.read("songs/nope.js").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongNotFound(ref p) if p == "songs/nope.js"),
            "{err}"
        );
    }

    #[test]
    fn read_and_write_go_through_resolve() {
        let (_tmp, project) = new_project();
        assert!(matches!(
            project.read("project.json").unwrap_err(),
            ProjectError::PathOutsideProject(_)
        ));
        assert!(matches!(
            project.write("../x.js", "x").unwrap_err(),
            ProjectError::PathOutsideProject(_)
        ));
    }

    // --- resolve ---

    #[test]
    fn resolve_accepts_song_paths() {
        let (_tmp, project) = new_project();
        assert_eq!(
            project.resolve("songs/main.js").unwrap(),
            project.root().join("songs/main.js")
        );
        assert_eq!(
            project.resolve("songs/not yet.js").unwrap(),
            project.root().join("songs/not yet.js")
        );
    }

    #[test]
    fn resolve_rejects_paths_outside_the_songs_folder() {
        let (_tmp, project) = new_project();
        let abs = project.root().join("songs/main.js");
        let abs = abs.to_str().unwrap();
        let cases = [
            abs,
            "/songs/main.js",
            "../songs/main.js",
            "songs/../main.js",
            "songs/..",
            "songs/../songs/main.js",
            "./songs/main.js",
            "songs/./main.js",
            "songs//main.js",
            "songs/a/b.js",
            "other/x.js",
            "x.js",
            "songs",
            "songs/",
            "songs/main.ts",
            "songs/main.js/",
            "songs/main",
            "songs/.js",
            "songs/.hidden.js",
            "songs/a\\b.js",
            "Songs/main.js",
            "",
        ];
        for rel in cases {
            let err = project.resolve(rel).unwrap_err();
            assert!(
                matches!(err, ProjectError::PathOutsideProject(ref p) if p == rel),
                "{rel:?}: {err}"
            );
        }
    }

    #[test]
    fn resolve_rejects_a_symlinked_song() {
        let (tmp, project) = new_project();
        let outside = tmp.path().join("secret.js");
        fs::write(&outside, "secret").unwrap();
        test_symlink!(&outside, project.root().join("songs/evil.js")).unwrap();
        assert!(matches!(
            project.resolve("songs/evil.js").unwrap_err(),
            ProjectError::PathOutsideProject(_)
        ));
        assert!(matches!(
            project.read("songs/evil.js").unwrap_err(),
            ProjectError::PathOutsideProject(_)
        ));
        assert!(project.write("songs/evil.js", "pwned").is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
    }

    #[test]
    fn resolve_rejects_a_dangling_symlinked_song() {
        let (tmp, project) = new_project();
        test_symlink!(
            tmp.path().join("missing.js"),
            project.root().join("songs/d.js"),
        )
        .unwrap();
        assert!(matches!(
            project.write("songs/d.js", "x").unwrap_err(),
            ProjectError::PathOutsideProject(_)
        ));
        assert!(!tmp.path().join("missing.js").exists());
    }

    #[test]
    fn resolve_rejects_a_songs_folder_symlinked_out_of_the_root() {
        let (tmp, project) = new_project();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::write(elsewhere.join("x.js"), "outside").unwrap();
        let songs = project.root().join("songs");
        fs::remove_dir_all(&songs).unwrap();
        test_symlink!(&elsewhere, &songs).unwrap();
        for rel in ["songs/x.js", "songs/new.js"] {
            assert!(matches!(
                project.resolve(rel).unwrap_err(),
                ProjectError::PathOutsideProject(_)
            ));
        }
        assert!(project.write("songs/new.js", "x").is_err());
        assert!(!elsewhere.join("new.js").exists());
    }

    #[test]
    fn resolve_with_missing_songs_folder_is_an_io_error() {
        let (_tmp, project) = new_project();
        fs::remove_dir_all(project.root().join("songs")).unwrap();
        let err = project.write("songs/main.js", "x").unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
    }

    // --- create_song ---

    #[test]
    fn create_song_accepts_valid_stems() {
        let (_tmp, project) = new_project();
        let long = "a".repeat(64);
        for stem in [
            "x",
            "Intro theme",
            "a-b_c",
            "A1 2-3_4",
            "-",
            "_",
            long.as_str(),
        ] {
            let rel = project.create_song(stem).unwrap();
            assert_eq!(rel, format!("songs/{stem}.js"));
            assert_eq!(project.read(&rel).unwrap(), new_song(stem));
        }
    }

    #[test]
    fn create_song_rejects_invalid_stems() {
        let (_tmp, project) = new_project();
        let too_long = "a".repeat(65);
        let cases = [
            "",
            too_long.as_str(),
            " lead",
            "trail ",
            " ",
            "a/b",
            "..",
            ".",
            "a.b",
            "../x",
            "café",
            "日本",
            "tab\t",
            "quote'",
            "back\\slash",
        ];
        for stem in cases {
            let err = project.create_song(stem).unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidSongName(ref s) if s == stem),
                "{stem:?}: {err}"
            );
        }
        assert_eq!(project.songs().unwrap().len(), 1);
    }

    #[test]
    fn create_song_twice_is_song_exists_and_does_not_clobber() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        project.write(&rel, "edited").unwrap();
        let err = project.create_song("intro").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongExists(ref p) if p == "songs/intro.js"),
            "{err}"
        );
        assert_eq!(project.read(&rel).unwrap(), "edited");
    }

    #[test]
    fn create_song_does_not_overwrite_main() {
        let (_tmp, project) = new_project();
        assert!(matches!(
            project.create_song("main").unwrap_err(),
            ProjectError::SongExists(_)
        ));
        assert_eq!(project.read("songs/main.js").unwrap(), MAIN_SONG);
    }

    #[test]
    fn create_song_shows_up_in_songs() {
        let (_tmp, project) = new_project();
        project.create_song("a").unwrap();
        let paths: Vec<String> = project
            .songs()
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect();
        assert_eq!(paths, ["songs/a.js", "songs/main.js"]);
    }

    // --- rename_song / delete_song ---

    fn song_paths(project: &Project) -> Vec<String> {
        project
            .songs()
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect()
    }

    #[test]
    fn rename_song_renames_and_keeps_contents() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        project.write(&rel, "edited").unwrap();
        let new = project.rename_song(&rel, "verse").unwrap();
        assert_eq!(new, "songs/verse.js");
        assert_eq!(project.read(&new).unwrap(), "edited");
        assert!(!project.root().join(&rel).exists());
        assert_eq!(song_paths(&project), ["songs/main.js", "songs/verse.js"]);
    }

    #[test]
    fn rename_song_onto_an_existing_name_is_song_exists() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        project.write(&rel, "mine").unwrap();
        let err = project.rename_song(&rel, "main").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongExists(ref p) if p == "songs/main.js"),
            "{err}"
        );
        assert_eq!(project.read("songs/main.js").unwrap(), MAIN_SONG);
        assert_eq!(project.read(&rel).unwrap(), "mine");
    }

    #[test]
    fn rename_song_to_the_same_path_is_song_exists() {
        let (_tmp, project) = new_project();
        let err = project.rename_song("songs/main.js", "main").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongExists(ref p) if p == "songs/main.js"),
            "{err}"
        );
        assert_eq!(project.read("songs/main.js").unwrap(), MAIN_SONG);
    }

    #[test]
    fn rename_song_rejects_invalid_stems() {
        let (_tmp, project) = new_project();
        let too_long = "a".repeat(65);
        for stem in ["a/b", "", too_long.as_str(), " lead", "a.b", ".."] {
            let err = project.rename_song("songs/main.js", stem).unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidSongName(ref s) if s == stem),
                "{stem:?}: {err}"
            );
        }
        assert_eq!(song_paths(&project), ["songs/main.js"]);
    }

    #[test]
    fn rename_song_of_a_missing_song_is_not_found() {
        let (_tmp, project) = new_project();
        let err = project.rename_song("songs/nope.js", "x").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongNotFound(ref p) if p == "songs/nope.js"),
            "{err}"
        );
    }

    #[test]
    fn rename_song_of_a_directory_is_not_found() {
        let (_tmp, project) = new_project();
        fs::create_dir(project.root().join("songs/dir.js")).unwrap();
        let err = project.rename_song("songs/dir.js", "x").unwrap_err();
        assert!(matches!(err, ProjectError::SongNotFound(_)), "{err}");
    }

    #[test]
    fn rename_song_rejects_bad_paths() {
        let (_tmp, project) = new_project();
        for rel in ["../x.js", "songs/a/b.js", "main.js", "songs/main.txt"] {
            let err = project.rename_song(rel, "x").unwrap_err();
            assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{rel}");
        }
    }

    #[test]
    fn rename_song_accepts_a_listed_song_with_an_invalid_own_stem() {
        let (_tmp, project) = new_project();
        fs::write(project.root().join("songs/a.b.js"), "dotted").unwrap();
        let new = project.rename_song("songs/a.b.js", "ab").unwrap();
        assert_eq!(new, "songs/ab.js");
        assert_eq!(project.read(&new).unwrap(), "dotted");
    }

    #[test]
    fn rename_song_case_only() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("Verse").unwrap();
        let new = project.rename_song(&rel, "verse").unwrap();
        assert_eq!(new, "songs/verse.js");
        assert_eq!(
            dir_names(&project.root().join("songs")),
            ["main.js", "verse.js"]
        );
        assert_eq!(project.read(&new).unwrap(), new_song("Verse"));
    }

    #[test]
    fn rename_song_falls_back_to_rename_when_links_are_unsupported() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let new = project
            .rename_song_with(&rel, "verse", unsupported, |p| fs::remove_file(p))
            .unwrap();
        assert_eq!(new, "songs/verse.js");
        assert_eq!(project.read(&new).unwrap(), new_song("intro"));
        assert_eq!(song_paths(&project), ["songs/main.js", "songs/verse.js"]);
    }

    #[test]
    fn rename_song_fallback_does_not_replace_an_existing_target() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let err = project
            .rename_song_with(&rel, "main", unsupported, |p| fs::remove_file(p))
            .unwrap_err();
        assert!(matches!(err, ProjectError::SongExists(_)), "{err}");
        assert_eq!(project.read("songs/main.js").unwrap(), MAIN_SONG);
        assert_eq!(project.read(&rel).unwrap(), new_song("intro"));
    }

    #[test]
    fn case_distinct_existing_entries_are_collisions_even_for_hard_links() {
        let (_tmp, project) = new_project();
        let old = project.create_song("Verse").unwrap();
        let source = project.root().join(&old);
        let target = project.root().join("songs/verse.js");
        if target.exists() {
            // The separate rename_song_case_only test covers case-insensitive volumes.
            assert!(matches!(
                project.read("songs/verse.js"),
                Err(ProjectError::SongNotFound(_))
            ));
            return;
        }
        fs::hard_link(&source, &target).unwrap();
        assert!(matches!(
            project.rename_song(&old, "verse"),
            Err(ProjectError::SongExists(_))
        ));
        assert_eq!(
            song_paths(&project),
            ["songs/Verse.js", "songs/main.js", "songs/verse.js"]
        );
    }

    #[test]
    fn exclusive_moves_with_competing_publishers_keep_both_payloads() {
        let tmp = tempdir();
        let dest = tmp.path().join("winner");
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ["one", "two"]
                .into_iter()
                .map(|name| {
                    let src = tmp.path().join(name);
                    fs::write(&src, name).unwrap();
                    let barrier = &barrier;
                    let dest = &dest;
                    scope.spawn(move || {
                        barrier.wait();
                        match move_noreplace(&src, dest) {
                            Ok(()) => {
                                assert!(!src.exists());
                                true
                            }
                            Err(e) => {
                                assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
                                assert_eq!(fs::read_to_string(src).unwrap(), name);
                                false
                            }
                        }
                    })
                })
                .collect();
            assert_eq!(
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap())
                    .filter(|won| *won)
                    .count(),
                1
            );
        });
        assert!(matches!(
            fs::read_to_string(dest).unwrap().as_str(),
            "one" | "two"
        ));
    }

    #[test]
    fn canonical_root_round_trips_in_paths_with_unicode_spaces_and_long_components() {
        let tmp = tempdir();
        let mut dir = tmp.path().join("作曲 session");
        for n in 0..5 {
            dir = dir.join(format!("{}-{n}", "a".repeat(48)));
        }
        let p = Project::create(&dir, "作曲").unwrap();
        let info = p.info().unwrap();
        let reopened = Project::open(Path::new(&info.root)).unwrap();
        assert_eq!(reopened.root(), p.root());
        fs::write(p.root().join("songs/été 1.js"), "content").unwrap();
        assert_eq!(reopened.read("songs/été 1.js").unwrap(), "content");
        assert!(
            reopened
                .songs()
                .unwrap()
                .iter()
                .any(|s| s.path == "songs/été 1.js")
        );
    }

    #[cfg(windows)]
    #[test]
    fn blocked_windows_replacement_keeps_the_file_and_removes_the_temp() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let tmp = tempdir();
        let path = tmp.path().join("out.wav");
        fs::write(&path, "old").unwrap();
        let other_reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        assert!(atomic_write_bytes(&path, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(dir_names(tmp.path()), ["out.wav"]);
        drop(other_reader);
        atomic_write_bytes(&path, b"new").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"new");
    }

    #[test]
    fn portable_destinations_reject_device_names() {
        let (_tmp, p) = new_project();
        for name in ["CON", "aux", "PRn", "NuL", "COM1", "lpt9"] {
            assert!(matches!(
                p.create_song(name),
                Err(ProjectError::InvalidSongName(_))
            ));
            assert!(matches!(
                p.rename_song("songs/main.js", name),
                Err(ProjectError::InvalidSongName(_))
            ));
            assert!(matches!(
                p.recording_partial_path(name),
                Err(ProjectError::InvalidRecordingName(_))
            ));
        }
        assert!(p.create_song("computer").is_ok());
        assert!(p.create_song("lpt10").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_reserved_files_can_be_read_and_renamed() {
        let (_tmp, p) = new_project();
        fs::write(p.root().join("songs/CON.js"), "old").unwrap();
        assert_eq!(p.read("songs/CON.js").unwrap(), "old");
        p.rename_song("songs/CON.js", "portable").unwrap();
        fs::write(p.root().join("recordings/AUX.wav"), "audio").unwrap();
        assert_eq!(p.recordings().unwrap()[0].stem, "AUX");
        assert_eq!(p.read_recording_bytes("AUX").unwrap(), b"audio");
        p.rename_recording("AUX", "portable").unwrap();
        p.delete_recording("portable").unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_stream_and_device_song_ids() {
        let (_tmp, p) = new_project();
        for id in [
            "songs/host:alternate.js",
            "songs/C:track.js",
            "songs/COM¹.js",
            "songs/AuX.other.js",
        ] {
            assert!(matches!(
                p.resolve(id),
                Err(ProjectError::PathOutsideProject(_))
            ));
        }
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_neither_listed_nor_written_through() {
        let (tmp, p) = new_project();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("external.js"), "protected").unwrap();
        fs::remove_dir_all(p.root().join("songs")).unwrap();
        // Unlike a file symlink, mklink /J needs no Developer Mode or administrator privilege.
        let status = std::process::Command::new("cmd.exe")
            .current_dir(tmp.path())
            .args(["/d", "/c", "mklink", "/J", r"p\songs", "outside"])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(matches!(
            p.songs(),
            Err(ProjectError::PathOutsideProject(_))
        ));
        assert!(matches!(
            p.write("songs/external.js", "new"),
            Err(ProjectError::PathOutsideProject(_))
        ));
        assert_eq!(
            fs::read_to_string(outside.join("external.js")).unwrap(),
            "protected"
        );
    }

    #[test]
    fn rename_song_onto_a_hard_link_of_itself_is_song_exists() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        let songs = project.root().join("songs");
        fs::hard_link(songs.join("intro.js"), songs.join("verse.js")).unwrap();
        let err = project.rename_song(&rel, "verse").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongExists(ref p) if p == "songs/verse.js"),
            "{err}"
        );
        assert_eq!(
            song_paths(&project),
            ["songs/intro.js", "songs/main.js", "songs/verse.js"]
        );
    }

    #[test]
    fn rename_song_removes_the_new_link_when_removing_the_old_name_fails() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        let denied = |_: &Path| Err(io::Error::from(io::ErrorKind::PermissionDenied));
        let err = project
            .rename_song_with(&rel, "verse", |a, b| fs::hard_link(a, b), denied)
            .unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
        assert_eq!(song_paths(&project), ["songs/intro.js", "songs/main.js"]);
        assert_eq!(project.read(&rel).unwrap(), new_song("intro"));
    }

    #[test]
    fn delete_song_removes_the_file() {
        let (_tmp, project) = new_project();
        let rel = project.create_song("intro").unwrap();
        project.delete_song(&rel).unwrap();
        assert!(!project.root().join(&rel).exists());
        assert_eq!(song_paths(&project), ["songs/main.js"]);
    }

    #[test]
    fn delete_song_of_a_missing_song_is_not_found() {
        let (_tmp, project) = new_project();
        let err = project.delete_song("songs/nope.js").unwrap_err();
        assert!(
            matches!(err, ProjectError::SongNotFound(ref p) if p == "songs/nope.js"),
            "{err}"
        );
    }

    #[test]
    fn delete_song_rejects_bad_paths_and_symlinks() {
        let (tmp, project) = new_project();
        let outside = tmp.path().join("outside.js");
        fs::write(&outside, "x").unwrap();
        test_symlink!(&outside, project.root().join("songs/link.js")).unwrap();
        for rel in ["songs/link.js", "../outside.js", "songs/a/b.js"] {
            let err = project.delete_song(rel).unwrap_err();
            assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{rel}");
        }
        assert!(outside.exists());
    }

    #[test]
    fn find_returns_the_project_and_stem() {
        let (_tmp, project) = new_project();
        let (found, stem) = Project::find(&project.root().join("songs/main.js")).unwrap();
        assert_eq!(found.root(), project.root());
        assert_eq!(stem, "main");
    }

    #[test]
    fn find_accepts_a_relative_path_with_dots() {
        let (_tmp, project) = new_project();
        let p = project.root().join("songs/../songs/./main.js");
        let (found, stem) = Project::find(&p).unwrap();
        assert_eq!((found.root(), stem.as_str()), (project.root(), "main"));
    }

    #[test]
    fn find_follows_a_symlinked_song_to_its_real_location() {
        let (tmp, project) = new_project();
        let link = tmp.path().join("link.js");
        test_symlink!(project.root().join("songs/main.js"), &link).unwrap();
        let (found, stem) = Project::find(&link).unwrap();
        assert_eq!((found.root(), stem.as_str()), (project.root(), "main"));
        // A symlink inside songs/ pointing out of the project resolves outside it.
        let outside = tmp.path().join("other/x.js");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "x").unwrap();
        test_symlink!(&outside, project.root().join("songs/out.js")).unwrap();
        assert!(Project::find(&project.root().join("songs/out.js")).is_none());
    }

    #[test]
    fn find_rejects_paths_that_are_not_songs_in_a_project() {
        let (tmp, project) = new_project();
        let root = project.root();
        fs::write(root.join("loose.js"), "x").unwrap();
        fs::create_dir_all(root.join("songs/nested")).unwrap();
        fs::write(root.join("songs/nested/deep.js"), "x").unwrap();
        fs::write(root.join("songs/notes.txt"), "x").unwrap();
        fs::write(root.join("songs/.hidden.js"), "x").unwrap();
        for p in [
            root.join("loose.js"),
            root.join("songs/nested/deep.js"),
            root.join("songs/notes.txt"),
            root.join("songs/.hidden.js"),
            root.join("songs/missing.js"),
            root.join("songs"),
            tmp.path().join("nowhere/songs/a.js"),
        ] {
            assert!(Project::find(&p).is_none(), "{}", p.display());
        }
    }

    #[test]
    fn find_rejects_a_missing_or_invalid_manifest() {
        let tmp = tempdir();
        let songs = tmp.path().join("songs");
        fs::create_dir_all(&songs).unwrap();
        fs::write(songs.join("a.js"), "x").unwrap();
        assert!(Project::find(&songs.join("a.js")).is_none(), "no manifest");
        for text in [
            "not json",
            "{\"schema\": 1}",
            "{\"schema\": 9, \"name\": \"n\"}",
        ] {
            fs::write(tmp.path().join(MANIFEST_FILE), text).unwrap();
            assert!(Project::find(&songs.join("a.js")).is_none(), "{text}");
        }
    }

    #[test]
    fn find_from_dir_at_the_root_and_below() {
        let (_tmp, project) = new_project();
        let root = project.root();
        fs::create_dir_all(root.join("songs/a/b")).unwrap();
        for dir in [
            root.to_path_buf(),
            root.join("songs"),
            root.join("songs/a/b"),
            root.join("recordings"),
        ] {
            let found = Project::find_from_dir(&dir).expect("found");
            assert_eq!(found.root(), root, "{}", dir.display());
        }
    }

    #[test]
    fn find_from_dir_gives_a_canonical_root() {
        let (tmp, project) = new_project();
        let dotted = project.root().join("songs/../recordings/.");
        assert_eq!(
            Project::find_from_dir(&dotted).unwrap().root(),
            project.root()
        );
        let link = tmp.path().join("link");
        test_symlink!(project.root().join("songs"), &link).unwrap();
        let found = Project::find_from_dir(&link).unwrap();
        assert_eq!(found.root(), project.root());
        assert!(found.root().is_absolute());
    }

    #[test]
    fn find_from_dir_outside_any_project() {
        let tmp = tempdir();
        fs::create_dir_all(tmp.path().join("x/y")).unwrap();
        assert!(Project::find_from_dir(&tmp.path().join("x/y")).is_none());
        assert!(Project::find_from_dir(&tmp.path().join("missing")).is_none());
    }

    #[test]
    fn find_from_dir_stops_at_the_nearest_manifest() {
        let (_tmp, project) = new_project();
        let inner = project.root().join("songs/inner");
        write_manifest(&inner, "not json");
        assert!(
            Project::find_from_dir(&inner).is_none(),
            "a broken one wins"
        );
        write_manifest(&inner, "{\"schema\": 1, \"name\": \"inner\"}");
        assert_eq!(
            Project::find_from_dir(&inner).unwrap().manifest().name,
            "inner"
        );
        // A directory named project.json is not a manifest.
        let nested = project.root().join("songs/other");
        fs::create_dir_all(nested.join(MANIFEST_FILE)).unwrap();
        assert_eq!(
            Project::find_from_dir(&nested).unwrap().root(),
            project.root()
        );
    }

    #[test]
    fn render_path_builds_the_file_name_and_creates_nothing() {
        let (_tmp, project) = new_project();
        fs::remove_dir(project.root().join("renders")).unwrap();
        let path = project.render_path("main", None, "wav").unwrap();
        assert_eq!(path, project.root().join("renders/main.wav"));
        assert!(!project.root().join("renders").exists());
        assert!(!path.exists());
        let path = project
            .render_path("my song", Some("verse 1"), "opus")
            .unwrap();
        assert_eq!(path, project.root().join("renders/my song-verse-1.opus"));
        for ext in ["wav", "mp3", "ogg", "opus"] {
            project.render_path("a", None, ext).unwrap();
        }
    }

    #[test]
    fn render_path_is_idempotent_and_keeps_existing_files() {
        let (_tmp, project) = new_project();
        let a = project.render_path("main", Some("a"), "wav").unwrap();
        fs::write(&a, "old").unwrap();
        let b = project.render_path("main", Some("a"), "wav").unwrap();
        assert_eq!(a, b);
        assert_eq!(fs::read_to_string(&b).unwrap(), "old");
    }

    #[test]
    fn render_path_slugs_the_section() {
        let (_tmp, project) = new_project();
        for (section, slug) in [
            ("verse", "verse"),
            ("Verse_2-b", "Verse_2-b"),
            ("  pre chorus  ", "pre-chorus"),
            ("a / b ** c", "a-b-c"),
            ("!!x!!", "x"),
            ("--x--", "x"),
            ("-a -b", "a--b"),
            ("\u{e9}t\u{e9}", "t"),
            ("a\u{e9}\u{e9}b", "a-b"),
            ("../../etc", "etc"),
        ] {
            let path = project.render_path("s", Some(section), "wav").unwrap();
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            assert_eq!(name, format!("s-{slug}.wav"), "section {section:?}");
        }
    }

    #[test]
    fn render_path_rejects_bad_names() {
        let (_tmp, project) = new_project();
        for section in ["", " ", "---", "\u{e9}", "///", "\u{1f3b5}"] {
            let err = project.render_path("s", Some(section), "wav").unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidRenderName(ref n) if n == section),
                "section {section:?}: {err}"
            );
        }
        for ext in ["", "flac", "WAV", ".wav", "wav/x"] {
            let err = project.render_path("s", None, ext).unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidRenderName(ref n) if n == ext),
                "ext {ext:?}: {err}"
            );
        }
        for stem in ["", " a", "a ", "a/b", "..", "a.b", "a\0b", &"x".repeat(65)] {
            let err = project.render_path(stem, None, "wav").unwrap_err();
            assert!(
                matches!(err, ProjectError::InvalidSongName(_)),
                "stem {stem:?}"
            );
        }
        assert!(project.render_path(&"x".repeat(64), None, "wav").is_ok());
    }

    #[test]
    fn write_render_creates_renders_and_replaces_the_file() {
        let (_tmp, project) = new_project();
        let renders = project.root().join("renders");
        fs::remove_dir(&renders).unwrap();
        let path = project.render_path("main", None, "wav").unwrap();
        project.write_render(&path, b"one").unwrap();
        assert!(renders.is_dir());
        assert_eq!(fs::read(&path).unwrap(), b"one");
        project.write_render(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(dir_names(&renders).len(), 1);
    }

    #[test]
    fn write_render_refuses_paths_outside_renders_and_symlinks() {
        let (tmp, project) = new_project();
        let root = project.root().to_path_buf();
        for path in [
            root.join("songs/a.wav"),
            root.join("renders/sub/a.wav"),
            root.join("renders"),
            tmp.path().join("a.wav"),
        ] {
            let err = project.write_render(&path, b"x").unwrap_err();
            assert!(
                matches!(err, ProjectError::PathOutsideProject(_)),
                "{path:?}: {err}"
            );
        }
        assert!(!root.join("songs/a.wav").exists());
        assert!(!root.join("renders/sub").exists());

        let outside = tmp.path().join("elsewhere");
        fs::create_dir(&outside).unwrap();
        let renders = root.join("renders");
        fs::remove_dir(&renders).unwrap();
        test_symlink!(&outside, &renders).unwrap();
        let err = project
            .write_render(&renders.join("main.wav"), b"x")
            .unwrap_err();
        assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{err}");
        assert!(dir_names(&outside).is_empty());
        #[cfg(unix)]
        fs::remove_file(&renders).unwrap();
        #[cfg(windows)]
        fs::remove_dir(&renders).unwrap();

        fs::create_dir(&renders).unwrap();
        let target = tmp.path().join("target.wav");
        fs::write(&target, "keep").unwrap();
        test_symlink!(&target, renders.join("main.wav")).unwrap();
        let err = project
            .write_render(&renders.join("main.wav"), b"x")
            .unwrap_err();
        assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{err}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn render_path_refuses_symlinks() {
        let (tmp, project) = new_project();
        let outside = tmp.path().join("elsewhere");
        fs::create_dir(&outside).unwrap();
        let renders = project.root().join("renders");
        fs::remove_dir(&renders).unwrap();
        test_symlink!(&outside, &renders).unwrap();
        let err = project.render_path("main", None, "wav").unwrap_err();
        assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{err}");
        #[cfg(unix)]
        fs::remove_file(&renders).unwrap();
        #[cfg(windows)]
        fs::remove_dir(&renders).unwrap();

        fs::create_dir(&renders).unwrap();
        let target = tmp.path().join("target.wav");
        fs::write(&target, "x").unwrap();
        test_symlink!(&target, renders.join("main.wav")).unwrap();
        let err = project.render_path("main", None, "wav").unwrap_err();
        assert!(matches!(err, ProjectError::PathOutsideProject(_)), "{err}");
        assert!(project.render_path("main", Some("a"), "wav").is_ok());
    }

    #[test]
    fn render_path_reports_renders_that_is_a_file() {
        let (_tmp, project) = new_project();
        let renders = project.root().join("renders");
        fs::remove_dir(&renders).unwrap();
        fs::write(&renders, "x").unwrap();
        let err = project.render_path("main", None, "wav").unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
        let err = project
            .write_render(&renders.join("main.wav"), b"x")
            .unwrap_err();
        assert!(matches!(err, ProjectError::Io { .. }), "{err}");
        assert_eq!(fs::read_to_string(&renders).unwrap(), "x");
    }

    #[test]
    fn atomic_write_bytes_writes_replaces_and_leaves_no_temp_file() {
        let tmp = tempdir();
        let path = tmp.path().join("out.bin");
        let bytes: Vec<u8> = (0..=255).collect();
        atomic_write_bytes(&path, &bytes).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        atomic_write_bytes(&path, b"").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"");
        atomic_write_bytes(&path, &[0xff, 0x00, 0xfe]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), [0xff, 0x00, 0xfe]);
        assert_eq!(dir_names(tmp.path()), ["out.bin"]);
    }

    #[test]
    fn atomic_write_bytes_failure_leaves_no_temp_file_and_no_target() {
        let tmp = tempdir();
        let missing = tmp.path().join("missing/x.bin");
        let err = atomic_write_bytes(&missing, b"x").unwrap_err();
        assert!(
            matches!(err, ProjectError::Io { path: ref p, .. } if *p == missing),
            "{err}"
        );
        assert!(dir_names(tmp.path()).is_empty());
        // The rename fails over a non-empty directory; the temp file must be removed.
        let target = tmp.path().join("dir");
        fs::create_dir_all(target.join("child")).unwrap();
        assert!(atomic_write_bytes(&target, b"x").is_err());
        assert_eq!(dir_names(tmp.path()), ["dir"]);
        assert!(target.join("child").is_dir());
    }

    #[test]
    fn atomic_write_reports_a_missing_parent() {
        let tmp = tempdir();
        let path = tmp.path().join("missing/x.js");
        let err = atomic_write(&path, "x").unwrap_err();
        assert!(
            matches!(err, ProjectError::Io { path: ref p, .. } if *p == path),
            "{err}"
        );
    }

    #[test]
    fn concurrent_writes_of_one_song_all_succeed_and_one_wins_whole() {
        let (_tmp, project) = new_project();
        let project = std::sync::Arc::new(project);
        // Large, distinct bodies so a shared temp file would show up as a mix or a truncation.
        let bodies: Vec<String> = (0..16)
            .map(|i| format!("// writer {i}\n").repeat(20_000))
            .collect();
        let handles: Vec<_> = bodies
            .iter()
            .map(|body| {
                let body = body.clone();
                let project = std::sync::Arc::clone(&project);
                std::thread::spawn(move || {
                    for _ in 0..4 {
                        project.write("songs/main.js", &body)?;
                    }
                    Ok::<(), ProjectError>(())
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let text = project.read("songs/main.js").unwrap();
        assert!(
            bodies.contains(&text),
            "final content is not one write in full"
        );
        assert_eq!(dir_names(&project.root().join("songs")), ["main.js"]);
    }
}

//! Watches the open project folder and reports song and manifest changes.
//!
//! A debounced `notify` watcher collects raw events; after each batch the pure [`classify_in`] looks
//! at which kept files exist now, so an atomic save (rename-over, seen as remove plus create)
//! arrives as `changed`. The app's own saves are reported too; the frontend ignores them by
//! comparing content. An event on the project root or any entry directly in it (other than
//! `project.json`) re-checks every known song and re-scans `songs/`: when the `songs` folder is
//! renamed, deleted or recreated the songs inside get no events of their own, and the debouncer
//! may report a rename only by its destination (`songs-old`), not by `songs`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, NoCache, new_debouncer_opt};
use serde::Serialize;
use strudeland_project::{
    MANIFEST_FILE, Project, RECORDINGS_DIR, is_recording_file_name, list_songs,
    recording_dir_entries, song_rel_path,
};

/// The Tauri event name carrying a [`ProjectChanged`] payload.
pub const CHANGED_EVENT: &str = "project://changed";

const DEBOUNCE: Duration = Duration::from_millis(150);

/// Song rel paths (`songs/<name>.js`), each list sorted.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SongChanges {
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// One debounced batch of project changes.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectChanged {
    /// The watched project's canonical root, spelt as `ProjectInfo::root`, so the frontend can
    /// drop batches from a previous project's watcher.
    pub root: String,
    pub songs: SongChanges,
    /// True iff `project.json` was touched in the batch.
    pub manifest: bool,
    /// True iff a memo `.wav` or `.json` under `recordings/` (or the folder itself) changed.
    pub recordings: bool,
}

enum Kept {
    Song(String),
    Manifest,
    /// The root or an entry directly in it, such as the `songs` folder or where it was renamed
    /// to: the songs need a full re-check.
    TopLevel,
    /// A memo file directly in `recordings/`.
    Recording,
    /// The `recordings` directory itself.
    RecordingFolder,
}

/// Native notifications can omit the verbatim drive/UNC prefix used by canonical Windows
/// roots. Compare components without resolving an event path that might already be gone.
fn relative_event<'a>(root: &Path, path: &'a Path) -> Option<&'a Path> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel);
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut root_parts = root.components();
        let mut path_parts = path.components();
        let (Some(Component::Prefix(r)), Some(Component::Prefix(p))) =
            (root_parts.next(), path_parts.next())
        else {
            return None;
        };
        let same_root = match (r.kind(), p.kind()) {
            (
                Prefix::VerbatimDisk(a) | Prefix::Disk(a),
                Prefix::VerbatimDisk(b) | Prefix::Disk(b),
            ) => a.eq_ignore_ascii_case(&b),
            (
                Prefix::VerbatimUNC(a, b) | Prefix::UNC(a, b),
                Prefix::VerbatimUNC(c, d) | Prefix::UNC(c, d),
            ) => a.eq_ignore_ascii_case(c) && b.eq_ignore_ascii_case(d),
            _ => false,
        };
        if same_root && root_parts.all(|r| Some(r) == path_parts.next()) {
            return Some(path_parts.as_path());
        }
    }
    None
}

/// Keeps `project.json`, the song paths of `root` (the crate's song rule), and the root and its
/// direct entries; drops the rest.
fn keep(root: &Path, path: &Path) -> Option<Kept> {
    let rel = relative_event(root, path)?;
    if rel == Path::new(MANIFEST_FILE) {
        return Some(Kept::Manifest);
    }
    if rel == Path::new(RECORDINGS_DIR) {
        return Some(Kept::RecordingFolder);
    }
    if rel.components().nth(1).is_none() {
        return Some(Kept::TopLevel);
    }
    let mut parts = rel.components();
    if let (Some(dir), Some(name), None) = (parts.next(), parts.next(), parts.next())
        && dir.as_os_str() == RECORDINGS_DIR
    {
        return name
            .as_os_str()
            .to_str()
            .filter(|n| is_recording_file_name(n))
            .map(|_| Kept::Recording);
    }
    song_rel_path(rel).map(Kept::Song)
}

/// The song rel paths in `root/songs` now, as [`Project::songs`] would list them, spelt exactly
/// as the folder lists them. A missing or unreadable folder has none.
fn scan_songs(root: &Path) -> BTreeSet<String> {
    list_songs(root)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.path)
        .collect()
}

/// Classifies one batch of raw event paths against the `known` song set, which it updates.
/// `root` must be canonical (event paths are). Returns `None` when there is nothing to report:
/// no kept path in the batch, or only songs that are neither known nor present.
///
/// A batch naming the root or an entry directly in it (such as `songs`) also re-checks every known song (reporting the
/// missing ones as removed) and re-scans `songs/` (reporting new ones as added). Songs found only
/// by that re-check are not reported as changed; only songs named in the batch are.
///
/// A song exists only when its exact name is in the `songs/` listing. On a case-insensitive
/// volume a lookup by path also finds `Verse.js` after it was renamed to `verse.js`, which would
/// keep the old spelling alive.
#[cfg(test)]
fn classify(
    root: &Path,
    known: &mut HashSet<String>,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Option<ProjectChanged> {
    classify_in(root, known, Touched::new(root, paths), scan_songs)
}

/// What one batch's kept paths touched.
#[derive(Default)]
struct Touched {
    /// The song paths named in the batch.
    named: BTreeSet<String>,
    /// The root or an entry directly in it: every song needs a re-check.
    rescan: bool,
    revalidate: bool,
    manifest: bool,
    recordings: bool,
}

impl Touched {
    #[cfg(test)]
    fn new(root: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Touched {
        let mut touched = Touched::default();
        for path in paths {
            touched.add(keep(root, &path), false);
        }
        touched
    }

    fn from_events(root: &Path, events: impl IntoIterator<Item = notify::Event>) -> Self {
        use notify::EventKind::{Create, Modify, Remove};
        use notify::event::ModifyKind::Name;
        let mut touched = Self::default();
        for event in events {
            if !affects_contents(event.kind) {
                continue;
            }
            let namespace_change = matches!(event.kind, Create(_) | Remove(_) | Modify(Name(_)));
            touched.rescan |= namespace_change;
            let folder_metadata = matches!(event.kind, Modify(_)) && !namespace_change;
            for path in event.paths {
                touched.add(keep(root, &path), folder_metadata);
            }
        }
        touched
    }

    fn add(&mut self, kept: Option<Kept>, folder_metadata: bool) {
        match kept {
            Some(Kept::Song(rel)) => {
                self.named.insert(rel);
            }
            Some(Kept::Manifest) => self.manifest = true,
            Some(Kept::TopLevel) => self.rescan = true,
            // Windows reports directory modifications for child writes, including ignored
            // partials and temporaries. Recheck the catalog before reporting a change.
            Some(Kept::RecordingFolder) if folder_metadata => self.rescan = true,
            Some(Kept::Recording | Kept::RecordingFolder) => self.recordings = true,
            None => {}
        }
    }

    fn full_rescan() -> Self {
        Self {
            rescan: true,
            revalidate: true,
            manifest: true,
            recordings: true,
            ..Self::default()
        }
    }

    /// Nothing kept: no song, top-level entry, manifest or memo.
    fn is_empty(&self) -> bool {
        self.named.is_empty() && !self.rescan && !self.reported_alone()
    }

    /// Whether the batch is reported even without a song change (the manifest or a memo).
    fn reported_alone(&self) -> bool {
        self.manifest || self.recordings
    }
}

impl SongChanges {
    fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

/// Classifies touched paths with the `songs/` listing supplied by `list`, which is called at most once.
fn classify_in(
    root: &Path,
    known: &mut HashSet<String>,
    touched: Touched,
    list: impl FnOnce(&Path) -> BTreeSet<String>,
) -> Option<ProjectChanged> {
    if touched.is_empty() {
        return None;
    }
    let named = &touched.named;
    let present = list(root);
    let mut songs: BTreeSet<String> = named.clone();
    if touched.rescan {
        songs.extend(known.iter().cloned());
        songs.extend(present.iter().cloned());
    }
    if songs.is_empty() && !touched.reported_alone() {
        return None;
    }
    // BTreeSet iteration is sorted, so each list comes out sorted.
    let mut changes = SongChanges::default();
    for rel in songs {
        match (present.contains(&rel), known.contains(&rel)) {
            (true, true) if touched.revalidate || named.contains(&rel) => changes.changed.push(rel),
            (true, true) => {}
            (true, false) => {
                known.insert(rel.clone());
                changes.added.push(rel);
            }
            (false, true) => {
                known.remove(&rel);
                changes.removed.push(rel);
            }
            (false, false) => {}
        }
    }
    if changes.is_empty() && !touched.reported_alone() {
        return None;
    }
    Some(ProjectChanged {
        root: root.to_string_lossy().into_owned(),
        songs: changes,
        manifest: touched.manifest,
        recordings: touched.recordings,
    })
}

// inotify also reports opens caused by our directory scans and the editor's own reloads.
// Only close-after-write can signal changed contents among the access events.
fn affects_contents(kind: notify::EventKind) -> bool {
    use notify::event::{AccessKind, AccessMode};
    match kind {
        notify::EventKind::Access(access) => access == AccessKind::Close(AccessMode::Write),
        _ => true,
    }
}

/// A one-sided move can name an unrelated destination while removing the manifest or a memo.
/// Keep metadata for just these entries so unrelated temp and render writes remain silent.
#[derive(PartialEq)]
struct Revision {
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

impl From<std::fs::Metadata> for Revision {
    fn from(meta: std::fs::Metadata) -> Self {
        Self {
            length: meta.len(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
        }
    }
}

#[derive(Default)]
struct Catalogs {
    manifest: Option<Revision>,
    recordings: Option<BTreeMap<String, Revision>>,
}

impl Catalogs {
    fn reconcile(&mut self, root: &Path, touched: &mut Touched) {
        if touched.rescan || touched.manifest {
            let now = std::fs::symlink_metadata(root.join(MANIFEST_FILE))
                .ok()
                .map(Revision::from);
            touched.manifest |= now != self.manifest;
            self.manifest = now;
        }
        if touched.rescan || touched.recordings {
            let now = recording_dir_entries(root).ok().flatten().map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        if !is_recording_file_name(&name) {
                            return None;
                        }
                        Some((name, Revision::from(entry.metadata().ok()?)))
                    })
                    .collect()
            });
            touched.recordings |= now != self.recordings;
            self.recordings = now;
        }
    }
}

/// A running watcher on one project. Dropping it stops watching; no event is emitted after the
/// drop returns.
pub struct Watcher {
    debouncer: Option<Debouncer<RecommendedWatcher, NoCache>>,
}

impl Watcher {
    /// Starts watching `project` recursively, calling `emit` once per classified batch from the
    /// watcher's thread.
    pub fn start(
        project: &Project,
        emit: impl Fn(ProjectChanged) + Send + 'static,
    ) -> Result<Watcher, String> {
        let root = project.root().to_path_buf();
        let mut known: HashSet<String> = project
            .songs()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| s.path)
            .collect();
        let mut catalogs = Catalogs::default();
        catalogs.reconcile(&root, &mut Touched::full_rescan());
        let handler_root = root.clone();
        let handler = move |result: DebounceEventResult| {
            // A lost batch can change contents without changing any filenames. Re-read every
            // known song and refresh both catalogs even when the backend supplied no paths.
            let mut touched = match result {
                Ok(events) if !events.iter().any(|e| e.need_rescan()) => {
                    Touched::from_events(&handler_root, events.into_iter().map(|e| e.event))
                }
                _ => Touched::full_rescan(),
            };
            catalogs.reconcile(&handler_root, &mut touched);
            if let Some(changed) = classify_in(&handler_root, &mut known, touched, scan_songs) {
                emit(changed);
            }
        };
        // FileIdMap follows directory links while populating its cache. The classifier
        // reconciles names from the checked project listing, including one-sided moves.
        let mut debouncer = new_debouncer_opt(
            DEBOUNCE,
            None,
            handler,
            NoCache::new(),
            notify::Config::default().with_follow_symlinks(false),
        )
        .map_err(|e| e.to_string())?;
        debouncer
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| e.to_string())?;
        Ok(Watcher {
            debouncer: Some(debouncer),
        })
    }

    /// A watcher that watches nothing, for tests that install a project without the slow
    /// FSEvents start and stop.
    #[cfg(test)]
    pub fn inert() -> Watcher {
        Watcher { debouncer: None }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // The debouncer's own drop only flags its thread to stop, which may still flush one
        // batch. `stop` joins the thread (at most one tick), so nothing is emitted afterwards.
        if let Some(debouncer) = self.debouncer.take() {
            debouncer.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;
    use tempfile::TempDir;

    // ---- classifier (no timing) ----

    struct Dir {
        _tmp: TempDir,
        root: PathBuf,
    }

    impl Dir {
        fn new() -> Dir {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().canonicalize().unwrap();
            fs::create_dir(root.join("songs")).unwrap();
            Dir { _tmp: tmp, root }
        }
        fn touch(&self, rel: &str) -> PathBuf {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "x").unwrap();
            path
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }
    }

    /// [`classify`] in `d`, checking the batch carries `d`'s root and then blanking it, so the
    /// expected values below need not spell it out.
    fn run(
        d: &Dir,
        known: &mut HashSet<String>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Option<ProjectChanged> {
        let mut changed = classify(&d.root, known, paths)?;
        assert_eq!(changed.root, d.root.to_string_lossy());
        changed.root = String::new();
        Some(changed)
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn songs(changed: &[&str], added: &[&str], removed: &[&str]) -> ProjectChanged {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        ProjectChanged {
            root: String::new(),
            songs: SongChanges {
                changed: v(changed),
                added: v(added),
                removed: v(removed),
            },
            manifest: false,
            recordings: false,
        }
    }

    #[test]
    fn existing_known_song_is_changed() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        let p = d.touch("songs/a.js");
        assert_eq!(
            run(&d, &mut known, [p]),
            Some(songs(&["songs/a.js"], &[], &[]))
        );
        assert_eq!(known, set(&["songs/a.js"]));
    }

    #[test]
    fn existing_unknown_song_is_added_and_joins_the_set() {
        let d = Dir::new();
        let mut known = HashSet::new();
        let p = d.touch("songs/new.js");
        assert_eq!(
            run(&d, &mut known, [p.clone()]),
            Some(songs(&[], &["songs/new.js"], &[]))
        );
        assert_eq!(known, set(&["songs/new.js"]));
        // The next event for it is a change.
        assert_eq!(
            run(&d, &mut known, [p]),
            Some(songs(&["songs/new.js"], &[], &[]))
        );
    }

    #[test]
    fn missing_known_song_is_removed_and_leaves_the_set() {
        let d = Dir::new();
        let mut known = set(&["songs/gone.js", "songs/keep.js"]);
        assert_eq!(
            run(&d, &mut known, [d.path("songs/gone.js")]),
            Some(songs(&[], &[], &["songs/gone.js"]))
        );
        assert_eq!(known, set(&["songs/keep.js"]));
    }

    #[test]
    fn missing_unknown_song_is_ignored() {
        let d = Dir::new();
        let mut known = HashSet::new();
        // A create-then-delete inside one batch: nothing to report.
        assert_eq!(run(&d, &mut known, [d.path("songs/ghost.js")]), None);
        assert!(known.is_empty());
    }

    #[test]
    fn manifest_sets_the_flag_whether_or_not_it_exists() {
        let d = Dir::new();
        let mut known = HashSet::new();
        let expected = ProjectChanged {
            manifest: true,
            ..Default::default()
        };
        assert_eq!(
            run(&d, &mut known, [d.path("project.json")]),
            Some(expected.clone())
        );
        let p = d.touch("project.json");
        assert_eq!(run(&d, &mut known, [p]), Some(expected));
    }

    #[test]
    fn ignored_paths_are_dropped_and_a_batch_of_only_them_is_none() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        d.touch("songs/a.js");
        // Top-level entries (`main.js`, `.DS_Store`) trigger a re-check, which finds nothing.
        let ignored = [
            d.touch("renders/a.js"),
            d.touch("renders/out.wav"),
            d.touch("recordings/.take.wav.tmp-1"),
            d.touch("recordings/.hidden.wav"),
            d.touch("recordings/take.wav.partial"),
            d.touch("recordings/take.txt"),
            d.touch("recordings/sub/take.wav"),
            d.touch("instruments/piano.js"),
            d.touch(".git/index"),
            d.touch(".git/songs/a.js"),
            d.touch("songs/.a.js.tmp-123"),
            d.touch("songs/.hidden.js"),
            d.touch("songs/notes.txt"),
            d.touch("songs/a.js.bak"),
            d.touch("songs/a/b.js"),
            d.touch("main.js"),
            d.touch(".DS_Store"),
            PathBuf::from("/elsewhere/songs/a.js"),
        ];
        assert_eq!(run(&d, &mut known, ignored), None);
        assert_eq!(run(&d, &mut known, []), None);
        assert_eq!(known, set(&["songs/a.js"]));
    }

    #[test]
    fn recording_files_and_the_folder_set_only_the_flag() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        d.touch("songs/a.js");
        let expected = ProjectChanged {
            recordings: true,
            ..Default::default()
        };
        for p in [
            d.touch("recordings/take.wav"),
            d.touch("recordings/take.json"),
            d.path("recordings/gone.wav"),
            d.path("recordings"),
        ] {
            assert_eq!(run(&d, &mut known, [p]), Some(expected.clone()));
        }
        // With a partial and a song in one batch, both are reported.
        let batch = [
            d.touch("recordings/a.wav.partial"),
            d.touch("recordings/a.wav"),
            d.touch("songs/a.js"),
        ];
        let mut both = songs(&["songs/a.js"], &[], &[]);
        both.recordings = true;
        assert_eq!(run(&d, &mut known, batch), Some(both));
        assert_eq!(known, set(&["songs/a.js"]));
    }

    #[test]
    fn a_songs_folder_or_root_event_with_nothing_changed_is_none() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        d.touch("songs/a.js");
        d.touch("songs/notes.txt");
        d.touch("songs/.hidden.js");
        let other = d.touch("renders/x.wav");
        let top = [
            d.path("songs"),
            d.root.clone(),
            d.path("renders"),
            d.touch("notes.txt"),
        ];
        for path in top {
            assert_eq!(run(&d, &mut known, [path, other.clone()]), None);
        }
        for batch in [vec![d.path("songs")], vec![d.root.clone()]] {
            assert_eq!(run(&d, &mut known, batch), None);
        }
        assert_eq!(known, set(&["songs/a.js"]));
    }

    #[test]
    fn a_songs_folder_event_removes_every_known_song_that_is_gone() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js", "songs/b.js"]);
        // The folder was renamed away: only the folder itself is in the batch.
        assert_eq!(
            run(&d, &mut known, [d.path("songs")]),
            Some(songs(&[], &[], &["songs/a.js", "songs/b.js"]))
        );
        assert!(known.is_empty());
    }

    #[test]
    fn a_rename_reported_only_by_its_destination_removes_the_songs() {
        // The debouncer reports `mv songs songs-old` as a create of `songs-old` alone.
        let d = Dir::new();
        fs::rename(d.path("songs"), d.path("songs-old")).unwrap();
        let mut known = set(&["songs/a.js"]);
        assert_eq!(
            run(&d, &mut known, [d.path("songs-old")]),
            Some(songs(&[], &[], &["songs/a.js"]))
        );
        assert!(known.is_empty());
    }

    #[test]
    fn a_root_event_removes_known_songs_when_songs_is_gone_entirely() {
        let d = Dir::new();
        fs::remove_dir(d.path("songs")).unwrap();
        let mut known = set(&["songs/a.js"]);
        assert_eq!(
            run(&d, &mut known, [d.root.clone()]),
            Some(songs(&[], &[], &["songs/a.js"]))
        );
        assert!(known.is_empty());
    }

    #[test]
    fn a_songs_folder_event_rescans_for_added_songs() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js", "songs/gone.js"]);
        d.touch("songs/a.js");
        d.touch("songs/back.js");
        d.touch("songs/notes.txt");
        fs::create_dir(d.path("songs/dir.js")).unwrap();
        assert_eq!(
            run(&d, &mut known, [d.path("songs")]),
            Some(songs(&[], &["songs/back.js"], &["songs/gone.js"]))
        );
        assert_eq!(known, set(&["songs/a.js", "songs/back.js"]));
    }

    #[test]
    fn a_song_named_in_a_rescan_batch_is_still_changed() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js", "songs/b.js"]);
        let a = d.touch("songs/a.js");
        d.touch("songs/b.js");
        assert_eq!(
            run(&d, &mut known, [d.path("songs"), a]),
            Some(songs(&["songs/a.js"], &[], &[]))
        );
    }

    #[test]
    fn nameless_song_files_are_not_songs() {
        // `.js` and `..js` have an empty or dotfile stem; `Project::songs` never lists them.
        let d = Dir::new();
        let mut known = HashSet::new();
        let batch = [d.touch("songs/.js"), d.touch("songs/..js")];
        assert_eq!(run(&d, &mut known, batch), None);
        assert!(known.is_empty());
    }

    #[test]
    fn a_kept_path_among_ignored_ones_is_reported() {
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        let batch = [
            d.touch("renders/x.wav"),
            d.touch("songs/a.js"),
            d.touch("songs/.a.js.tmp-1"),
        ];
        assert_eq!(
            run(&d, &mut known, batch),
            Some(songs(&["songs/a.js"], &[], &[]))
        );
    }

    #[test]
    fn duplicates_collapse_and_lists_are_sorted() {
        let d = Dir::new();
        let mut known = set(&["songs/b.js", "songs/a.js", "songs/y.js", "songs/x.js"]);
        let batch = [
            d.touch("songs/b.js"),
            d.touch("songs/a.js"),
            d.touch("songs/b.js"),
            d.touch("songs/d.js"),
            d.touch("songs/c.js"),
            d.touch("songs/c.js"),
            d.path("songs/y.js"),
            d.path("songs/x.js"),
            d.path("songs/y.js"),
            d.touch("project.json"),
            d.touch("project.json"),
        ];
        let mut expected = songs(
            &["songs/a.js", "songs/b.js"],
            &["songs/c.js", "songs/d.js"],
            &["songs/x.js", "songs/y.js"],
        );
        expected.manifest = true;
        assert_eq!(run(&d, &mut known, batch), Some(expected));
    }

    #[test]
    fn remove_then_create_of_an_existing_song_is_changed() {
        // Rename-over surfaces as a remove plus a create of the same path (and the temp file).
        let d = Dir::new();
        let mut known = set(&["songs/a.js"]);
        let song = d.touch("songs/a.js");
        let temp = d.path("songs/.a.js.tmp-42");
        assert_eq!(
            run(&d, &mut known, [temp, song.clone(), song]),
            Some(songs(&["songs/a.js"], &[], &[]))
        );
        assert_eq!(known, set(&["songs/a.js"]));
    }

    #[test]
    #[cfg(unix)]
    fn a_directory_or_symlink_named_like_a_song_does_not_exist_as_a_song() {
        let d = Dir::new();
        let mut known = set(&["songs/dir.js"]);
        fs::create_dir(d.path("songs/dir.js")).unwrap();
        let target = d.touch("elsewhere.js");
        std::os::unix::fs::symlink(&target, d.path("songs/link.js")).unwrap();
        assert_eq!(
            run(
                &d,
                &mut known,
                [d.path("songs/dir.js"), d.path("songs/link.js")]
            ),
            Some(songs(&[], &[], &["songs/dir.js"]))
        );
    }

    #[test]
    fn reads_do_not_start_another_watch_and_reload_cycle() {
        use notify::EventKind::Access;
        use notify::event::{AccessKind, AccessMode};
        assert!(!affects_contents(Access(AccessKind::Open(AccessMode::Any))));
        assert!(!affects_contents(Access(AccessKind::Read)));
        assert!(!affects_contents(Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(affects_contents(Access(AccessKind::Close(
            AccessMode::Write
        ))));
    }

    #[test]
    fn a_parent_directory_modification_only_reports_a_recording_catalog_change() {
        use notify::event::{CreateKind, ModifyKind};
        use notify::{Event, EventKind};

        let d = Dir::new();
        fs::create_dir(d.path(RECORDINGS_DIR)).unwrap();
        let mut catalogs = Catalogs::default();
        catalogs.reconcile(&d.root, &mut Touched::full_rescan());
        let parent_modified =
            || Event::new(EventKind::Modify(ModifyKind::Any)).add_path(d.path(RECORDINGS_DIR));
        let partial = d.touch("recordings/take.wav.partial");
        let mut touched = Touched::from_events(
            &d.root,
            [
                Event::new(EventKind::Create(CreateKind::Any)).add_path(partial),
                parent_modified(),
            ],
        );
        catalogs.reconcile(&d.root, &mut touched);
        let mut known = HashSet::new();
        assert!(classify_in(&d.root, &mut known, touched, scan_songs).is_none());

        d.touch("recordings/take.wav");
        let mut touched = Touched::from_events(&d.root, [parent_modified()]);
        catalogs.reconcile(&d.root, &mut touched);
        let changed = classify_in(&d.root, &mut known, touched, scan_songs).unwrap();
        assert!(changed.recordings);
        assert!(!changed.manifest && changed.songs.is_empty());
    }

    #[test]
    fn destination_only_moves_detect_removed_manifest_or_memo_without_reporting_unrelated_writes() {
        let d = Dir::new();
        let manifest = d.touch("project.json");
        let memo = d.touch("recordings/a.wav");
        let mut catalogs = Catalogs::default();
        catalogs.reconcile(&d.root, &mut Touched::full_rescan());
        let mut reconcile = |dest: &str| {
            let mut touched = Touched::new(&d.root, [d.path(dest)]);
            touched.rescan = true;
            catalogs.reconcile(&d.root, &mut touched);
            (touched.manifest, touched.recordings)
        };
        d.touch("recordings/.take.wav.partial");
        assert_eq!(reconcile("recordings/.take.wav.partial"), (false, false));
        fs::rename(manifest, d.path("old-manifest")).unwrap();
        assert_eq!(reconcile("old-manifest"), (true, false));
        fs::rename(memo, d.path("recordings/backup")).unwrap();
        assert_eq!(reconcile("recordings/backup"), (false, true));
        assert_eq!(reconcile("recordings/backup"), (false, false));
    }

    #[test]
    fn loss_of_events_revalidates_known_contents_and_both_catalogs() {
        let d = Dir::new();
        let mut known = set(&["songs/same.js", "songs/gone.js"]);
        d.touch("songs/same.js");
        d.touch("songs/new.js");
        let batch = classify_in(&d.root, &mut known, Touched::full_rescan(), scan_songs).unwrap();
        assert_eq!(batch.songs.changed, ["songs/same.js"]);
        assert_eq!(batch.songs.added, ["songs/new.js"]);
        assert_eq!(batch.songs.removed, ["songs/gone.js"]);
        assert!(batch.manifest && batch.recordings);
    }

    #[cfg(windows)]
    #[test]
    fn drive_and_unc_notifications_match_canonical_roots_even_after_removal() {
        for (root, event) in [
            (r"\\?\C:\work", r"C:\work\songs\gone.js"),
            (
                r"\\?\UNC\server\share\work",
                r"\\server\share\work\songs\gone.js",
            ),
        ] {
            assert!(
                matches!(keep(Path::new(root), Path::new(event)), Some(Kept::Song(id)) if id == "songs/gone.js")
            );
        }
    }

    #[test]
    fn a_case_only_rename_removes_the_old_spelling() {
        // A case-insensitive volume still finds `Verse.js` by path after the rename; only the
        // folder listing has the new spelling. The listing is injected so this holds on any FS.
        let d = Dir::new();
        let mut known = set(&["songs/Verse.js"]);
        let batch = [d.path("songs/Verse.js"), d.path("songs/verse.js")];
        let listing = || BTreeSet::from(["songs/verse.js".to_string()]);
        let changed = classify_in(&d.root, &mut known, Touched::new(&d.root, batch), |_| {
            listing()
        })
        .map(|mut c| {
            c.root = String::new();
            c
        });
        assert_eq!(
            changed,
            Some(songs(&[], &["songs/verse.js"], &["songs/Verse.js"]))
        );
        assert_eq!(known, set(&["songs/verse.js"]));
    }

    #[test]
    fn a_real_case_only_rename_removes_the_old_spelling() {
        let d = Dir::new();
        let mut known = set(&["songs/Verse.js"]);
        let old = d.touch("songs/Verse.js");
        let new = d.path("songs/verse.js");
        fs::rename(&old, &new).unwrap();
        assert_eq!(
            run(&d, &mut known, [old, new]),
            Some(songs(&[], &["songs/verse.js"], &["songs/Verse.js"]))
        );
        assert_eq!(known, set(&["songs/verse.js"]));
    }

    #[test]
    fn payload_serialises_to_the_ipc_shape() {
        let mut payload = songs(&["songs/a.js"], &["songs/b.js"], &["songs/c.js"]);
        payload.manifest = true;
        payload.recordings = true;
        payload.root = "/p/song".to_string();
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "root": "/p/song",
                "songs": {
                    "changed": ["songs/a.js"],
                    "added": ["songs/b.js"],
                    "removed": ["songs/c.js"],
                },
                "manifest": true,
                "recordings": true,
            })
        );
    }

    // ---- real watcher (FSEvents latency varies, so wait generously) ----

    /// How long to wait for an expected event.
    const EXPECT: Duration = Duration::from_secs(10);
    /// How long to wait to be confident no event is coming.
    const QUIET: Duration = Duration::from_millis(1200);

    struct Live {
        _tmp: TempDir,
        project: Project,
        rx: Receiver<ProjectChanged>,
        watcher: Option<Watcher>,
    }

    impl Live {
        fn new() -> Live {
            let tmp = tempfile::tempdir().unwrap();
            let project = Project::create(&tmp.path().join("p"), "live").unwrap();
            let (tx, rx) = channel();
            let watcher = Watcher::start(&project, move |c| {
                let _ = tx.send(c);
            })
            .unwrap();
            let live = Live {
                _tmp: tmp,
                project,
                rx,
                watcher: Some(watcher),
            };
            // FSEvents can deliver events from just before the stream started (the project
            // creation); let them pass so they do not leak into the first assertion.
            live.drain(Duration::from_millis(500));
            live
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.project.root().join(rel)
        }

        fn drain(&self, quiet: Duration) {
            while self.rx.recv_timeout(quiet).is_ok() {}
        }

        /// Merges batches until `done` holds for the merged result or the timeout passes. A
        /// single logical change may be split over several debounced batches.
        fn wait_for(&self, done: impl Fn(&ProjectChanged) -> bool) -> ProjectChanged {
            let deadline = Instant::now() + EXPECT;
            let mut merged = ProjectChanged::default();
            while !done(&merged) {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.rx.recv_timeout(left) {
                    Ok(c) => {
                        assert_eq!(c.root, self.project.info().unwrap().root, "{c:?}");
                        merged.root = c.root;
                        merged.songs.changed.extend(c.songs.changed);
                        merged.songs.added.extend(c.songs.added);
                        merged.songs.removed.extend(c.songs.removed);
                        merged.manifest |= c.manifest;
                        merged.recordings |= c.recordings;
                    }
                    Err(_) => panic!("timed out waiting for an event; merged so far: {merged:?}"),
                }
            }
            merged
        }

        fn assert_quiet(&self) {
            if let Ok(c) = self.rx.recv_timeout(QUIET) {
                panic!("unexpected event: {c:?}");
            }
        }
    }

    fn has_song(c: &ProjectChanged) -> bool {
        !c.songs.is_empty()
    }

    #[test]
    fn real_watcher_classifies_saves_creates_deletes_and_manifest_edits() {
        let live = Live::new();

        // Project::write is a temp-file rename-over.
        live.project.write("songs/main.js", "s(\"bd\")").unwrap();
        let c = live.wait_for(has_song);
        assert_eq!(c.songs, songs(&["songs/main.js"], &[], &[]).songs, "{c:?}");
        live.drain(QUIET);

        fs::write(live.path("songs/x.js"), "x").unwrap();
        let c = live.wait_for(has_song);
        assert_eq!(c.songs, songs(&[], &["songs/x.js"], &[]).songs, "{c:?}");
        live.drain(QUIET);

        fs::remove_file(live.path("songs/x.js")).unwrap();
        let c = live.wait_for(has_song);
        assert_eq!(c.songs, songs(&[], &[], &["songs/x.js"]).songs, "{c:?}");
        live.drain(QUIET);

        let manifest = live.path("project.json");
        let text = fs::read_to_string(&manifest).unwrap();
        fs::write(&manifest, text.replace("\"live\"", "\"renamed\"")).unwrap();
        let c = live.wait_for(|c| c.manifest);
        assert!(!has_song(&c), "{c:?}");
        live.drain(QUIET);

        fs::write(live.path("renders/out.wav"), "riff").unwrap();
        fs::write(live.path("recordings/.take.wav.tmp-1"), "riff").unwrap();
        fs::write(live.path("recordings/take.wav.partial"), "riff").unwrap();
        live.assert_quiet();

        fs::create_dir_all(live.path("recordings")).unwrap();
        fs::write(live.path("recordings/take.wav"), "riff").unwrap();
        let c = live.wait_for(|c| c.recordings);
        assert!(!has_song(&c) && !c.manifest, "{c:?}");
    }

    #[test]
    fn real_watcher_reports_a_manual_rename_over_as_changed() {
        let live = Live::new();
        let temp = live.path("songs/.main.js.swp");
        fs::write(&temp, "// from another editor").unwrap();
        fs::rename(&temp, live.path("songs/main.js")).unwrap();
        let c = live.wait_for(has_song);
        assert_eq!(c.songs, songs(&["songs/main.js"], &[], &[]).songs, "{c:?}");
        assert!(!c.manifest);
        assert_eq!(
            fs::read_to_string(live.path("songs/main.js")).unwrap(),
            "// from another editor"
        );
        live.drain(QUIET);
        live.assert_quiet();
    }

    #[test]
    fn real_watcher_reports_moving_the_songs_folder_away_and_back() {
        let live = Live::new();
        live.project.create_song("b").unwrap();
        live.wait_for(|c| c.songs.added == ["songs/b.js"]);
        live.drain(QUIET);

        let songs_dir = live.path("songs");
        let moved = live.path("songs-old");
        fs::rename(&songs_dir, &moved).unwrap();
        let c = live.wait_for(|c| c.songs.removed.len() == 2);
        let mut removed = c.songs.removed.clone();
        removed.sort();
        assert_eq!(removed, ["songs/b.js", "songs/main.js"], "{c:?}");
        assert!(c.songs.added.is_empty(), "{c:?}");
        live.drain(QUIET);

        fs::rename(&moved, &songs_dir).unwrap();
        let c = live.wait_for(|c| c.songs.added.len() == 2);
        let mut added = c.songs.added.clone();
        added.sort();
        assert_eq!(added, ["songs/b.js", "songs/main.js"], "{c:?}");
        assert!(c.songs.removed.is_empty(), "{c:?}");
    }

    #[test]
    fn dropping_the_watcher_stops_events() {
        let mut live = Live::new();
        // Prove the watcher is live first, so silence after the drop means something.
        live.project.write("songs/main.js", "1").unwrap();
        live.wait_for(has_song);
        live.drain(QUIET);

        drop(live.watcher.take());
        live.project.write("songs/main.js", "2").unwrap();
        fs::write(live.path("songs/new.js"), "x").unwrap();
        match live.rx.recv_timeout(QUIET) {
            // The sender lives in the handler, which the stopped debouncer thread dropped.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            other => panic!("expected the channel to close, got {other:?}"),
        }
    }
}

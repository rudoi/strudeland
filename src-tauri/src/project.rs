//! Project commands: the managed project state and the Tauri commands over `strudeland-project`.
//!
//! The logic lives in plain functions that take the state and the recents file path so it can be
//! tested without an `AppHandle`; the `#[tauri::command]` wrappers only resolve the path and build
//! the frontend's project watcher.
//!
//! The commands are `async` so Tauri runs them off the main thread, and the state lock is only
//! held to clone or swap the open project, never across file I/O.
//!
//! Because they run concurrently, the commands that act on the open project take the `root` the
//! frontend believes is open (as returned in [`ProjectInfo::root`]) and fail with
//! [`PROJECT_CHANGED`] if another project has been opened since, so a save in flight during a
//! switch can never land in the new project.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde::Serialize;
use strudeland_project::{
    PREFS_FILE, Prefs, Project, ProjectError, ProjectInfo, RecentProject, Recents, SongEntry,
};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::watcher::{self, Watcher};

const NO_PROJECT: &str = "no project is open";
/// The error when a command's `root` is not the open project's.
pub const PROJECT_CHANGED: &str = "the project changed";
const RECENTS_FILE: &str = "recent.json";

/// The project open in this window, if any. Dropping it stops its watcher.
pub struct OpenProject {
    pub project: Arc<Project>,
    #[expect(dead_code, reason = "held only so dropping the project stops watching")]
    pub watcher: Watcher,
}

/// What runs after each project is opened (the app's partial repair).
type OnOpen = Box<dyn Fn(&Arc<Project>) + Send + Sync>;

#[derive(Default)]
pub struct ProjectState {
    open: Mutex<Option<OpenProject>>,
    on_open: OnceLock<OnOpen>,
}

impl ProjectState {
    fn lock(&self) -> MutexGuard<'_, Option<OpenProject>> {
        // A panic while holding the lock leaves the data consistent, so recover from poison.
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets what runs after every open (create, open, last), once installed. It must not block:
    /// it runs before the open command returns. A second call is ignored.
    pub fn on_open(&self, f: impl Fn(&Arc<Project>) + Send + Sync + 'static) {
        let _ = self.on_open.set(Box::new(f));
    }

    /// The open project, shared so the caller can do I/O after the lock is released.
    fn current(&self) -> Result<Arc<Project>, String> {
        self.lock()
            .as_ref()
            .map(|open| Arc::clone(&open.project))
            .ok_or_else(|| NO_PROJECT.to_string())
    }

    /// The open project if its root, as [`ProjectInfo::root`] spells it, is `root`.
    pub(crate) fn current_for(&self, root: &str) -> Result<Arc<Project>, String> {
        let project = self.current()?;
        if project.root().to_string_lossy() != root {
            return Err(PROJECT_CHANGED.to_string());
        }
        Ok(project)
    }

    /// Installs `project` without a watcher or recents, and returns its root as
    /// [`ProjectInfo::root`] spells it.
    #[cfg(test)]
    pub(crate) fn open_unwatched(&self, project: Project) -> String {
        let root = project.root().to_string_lossy().into_owned();
        self.replace(Some(OpenProject {
            project: Arc::new(project),
            watcher: Watcher::inert(),
        }));
        root
    }

    /// Makes `open` the open project. The previous one is dropped after the lock is released,
    /// since stopping its watcher joins the watcher thread.
    fn replace(&self, open: Option<OpenProject>) {
        let previous = std::mem::replace(&mut *self.lock(), open);
        drop(previous);
    }
}

/// Starts a watcher on `project`, installs it as the open one (dropping any previous project and
/// its watcher), touches recents and runs the [`ProjectState::on_open`] hook. On error the open
/// project is left as it was.
fn install(
    state: &ProjectState,
    recents_file: &Path,
    project: Project,
    watch: impl FnOnce(&Project) -> Result<Watcher, String>,
) -> Result<ProjectInfo, String> {
    let result = project.info().map_err(|e| e.to_string())?;
    let watcher = watch(&project)?;
    Recents::touch_and_save(recents_file, project.root());
    let project = Arc::new(project);
    state.replace(Some(OpenProject {
        project: Arc::clone(&project),
        watcher,
    }));
    if let Some(on_open) = state.on_open.get() {
        on_open(&project);
    }
    Ok(result)
}

pub fn create_project(
    state: &ProjectState,
    recents_file: &Path,
    dir: &str,
    name: &str,
    watch: impl FnOnce(&Project) -> Result<Watcher, String>,
) -> Result<ProjectInfo, String> {
    let project = Project::create(Path::new(dir), name).map_err(|e| e.to_string())?;
    install(state, recents_file, project, watch)
}

pub fn open_project(
    state: &ProjectState,
    recents_file: &Path,
    dir: &str,
    watch: impl FnOnce(&Project) -> Result<Watcher, String>,
) -> Result<ProjectInfo, String> {
    let project = Project::open(Path::new(dir)).map_err(|e| e.to_string())?;
    install(state, recents_file, project, watch)
}

/// Drops the open project and its watcher. Only tests close a project; the app replaces it.
#[cfg(test)]
fn close_project(state: &ProjectState) {
    state.replace(None);
}

/// Prunes recents, then installs the first entry that opens and starts watching.
pub fn open_last(
    state: &ProjectState,
    recents_file: &Path,
    watch: impl Fn(&Project) -> Result<Watcher, String>,
) -> Result<Option<ProjectInfo>, String> {
    let recents = Recents::load_pruned(recents_file);
    Ok(recents
        .open_each()
        .find_map(|(_, project)| install(state, recents_file, project, &watch).ok()))
}

/// Re-reads the open project's `project.json` and returns its info, without restarting the
/// watcher or touching recents. On success the open project takes the new manifest; if the
/// manifest no longer parses the error is returned and the open project is left as it was.
/// Fails with [`PROJECT_CHANGED`] if `root` is not the open project.
pub fn reload_info(state: &ProjectState, root: &str) -> Result<ProjectInfo, String> {
    let current = state.current_for(root)?;
    let reloaded = current.reload().map_err(|e| e.to_string())?;
    let info = reloaded.info().map_err(|e| e.to_string())?;
    // Another command may have switched projects meanwhile; only update the one we reloaded.
    if let Some(open) = state.lock().as_mut()
        && Arc::ptr_eq(&open.project, &current)
    {
        open.project = Arc::new(reloaded);
    }
    Ok(info)
}

/// Runs `f` on the open project, if it is the one at `root` (see [`PROJECT_CHANGED`]).
fn with_project<T>(
    state: &ProjectState,
    root: &str,
    f: impl FnOnce(&Project) -> Result<T, strudeland_project::ProjectError>,
) -> Result<T, String> {
    let project = state.current_for(root)?;
    f(&project).map_err(|e| e.to_string())
}

pub fn list_songs(state: &ProjectState, root: &str) -> Result<Vec<SongEntry>, String> {
    with_project(state, root, |p| p.songs())
}

pub fn read_song(state: &ProjectState, root: &str, path: &str) -> Result<String, String> {
    with_project(state, root, |p| p.read(path))
}

pub fn save_song(
    state: &ProjectState,
    root: &str,
    path: &str,
    contents: &str,
) -> Result<(), String> {
    with_project(state, root, |p| p.write(path, contents))
}

/// The error of the song commands the frontend branches on: a machine-readable `kind` plus the
/// same message text the other commands return.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongError {
    pub kind: SongErrorKind,
    pub message: String,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SongErrorKind {
    SongExists,
    SongNotFound,
    InvalidName,
    ProjectChanged,
    Other,
}

impl From<ProjectError> for SongError {
    fn from(e: ProjectError) -> Self {
        let kind = match e {
            ProjectError::SongExists(_) => SongErrorKind::SongExists,
            ProjectError::SongNotFound(_) => SongErrorKind::SongNotFound,
            ProjectError::InvalidSongName(_) => SongErrorKind::InvalidName,
            _ => SongErrorKind::Other,
        };
        SongError {
            kind,
            message: e.to_string(),
        }
    }
}

/// `changed` for the [`PROJECT_CHANGED`] message, else `other`: the kind a structured command
/// error gives a plain [`with_project`]-style message.
pub(crate) fn changed_or<K>(message: &str, changed: K, other: K) -> K {
    if message == PROJECT_CHANGED {
        changed
    } else {
        other
    }
}

impl From<String> for SongError {
    fn from(message: String) -> Self {
        let kind = changed_or(
            &message,
            SongErrorKind::ProjectChanged,
            SongErrorKind::Other,
        );
        SongError { kind, message }
    }
}

/// Like [`with_project`], with a structured error.
fn with_song_project<T>(
    state: &ProjectState,
    root: &str,
    f: impl FnOnce(&Project) -> Result<T, ProjectError>,
) -> Result<T, SongError> {
    let project = state.current_for(root)?;
    Ok(f(&project)?)
}

pub fn new_song(state: &ProjectState, root: &str, stem: &str) -> Result<String, SongError> {
    with_song_project(state, root, |p| p.create_song(stem))
}

/// Renames a song and returns its new relative path.
pub fn rename_song(
    state: &ProjectState,
    root: &str,
    path: &str,
    stem: &str,
) -> Result<String, SongError> {
    with_song_project(state, root, |p| p.rename_song(path, stem))
}

pub fn delete_song(state: &ProjectState, root: &str, path: &str) -> Result<(), SongError> {
    with_song_project(state, root, |p| p.delete_song(path))
}

/// The canonical root of the project folder `dir`, spelled as `ProjectInfo::root` spells it.
/// Installs nothing, starts no watcher and touches no recents.
pub fn root_of(dir: &str) -> Result<String, String> {
    let project = Project::open(Path::new(dir)).map_err(|e| e.to_string())?;
    Ok(project.root().to_string_lossy().into_owned())
}

/// Starts a project watcher that emits each batch to the frontend.
fn app_watcher(app: &AppHandle) -> impl Fn(&Project) -> Result<Watcher, String> {
    move |project| {
        let app = app.clone();
        Watcher::start(project, move |changed| {
            let _ = app.emit(watcher::CHANGED_EVENT, changed);
        })
    }
}

fn app_data_file(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join(name))
        .map_err(|e| e.to_string())
}

fn recents_path(app: &AppHandle) -> Result<PathBuf, String> {
    app_data_file(app, RECENTS_FILE)
}

#[tauri::command]
pub async fn project_create(
    app: AppHandle,
    state: State<'_, ProjectState>,
    dir: String,
    name: String,
) -> Result<ProjectInfo, String> {
    create_project(&state, &recents_path(&app)?, &dir, &name, app_watcher(&app))
}

#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, ProjectState>,
    dir: String,
) -> Result<ProjectInfo, String> {
    open_project(&state, &recents_path(&app)?, &dir, app_watcher(&app))
}

#[tauri::command]
pub async fn project_info(
    state: State<'_, ProjectState>,
    root: String,
) -> Result<ProjectInfo, String> {
    reload_info(&state, &root)
}

#[tauri::command]
pub async fn project_last(
    app: AppHandle,
    state: State<'_, ProjectState>,
) -> Result<Option<ProjectInfo>, String> {
    open_last(&state, &recents_path(&app)?, app_watcher(&app))
}

#[tauri::command]
pub async fn recent_list(app: AppHandle) -> Result<Vec<RecentProject>, String> {
    Ok(Recents::list(&recents_path(&app)?))
}

#[tauri::command]
pub async fn prefs_get(app: AppHandle) -> Result<Prefs, String> {
    Ok(Prefs::load(&app_data_file(&app, PREFS_FILE)?))
}

/// Replaces the whole prefs object.
#[tauri::command]
pub async fn prefs_set(app: AppHandle, prefs: Prefs) -> Result<(), String> {
    prefs
        .save(&app_data_file(&app, PREFS_FILE)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn song_list(
    state: State<'_, ProjectState>,
    root: String,
) -> Result<Vec<SongEntry>, String> {
    list_songs(&state, &root)
}

#[tauri::command]
pub async fn song_read(
    state: State<'_, ProjectState>,
    root: String,
    path: String,
) -> Result<String, String> {
    read_song(&state, &root, &path)
}

#[tauri::command]
pub async fn song_save(
    state: State<'_, ProjectState>,
    root: String,
    path: String,
    contents: String,
) -> Result<(), String> {
    save_song(&state, &root, &path, &contents)
}

#[tauri::command]
pub async fn song_create(
    state: State<'_, ProjectState>,
    root: String,
    stem: String,
) -> Result<String, SongError> {
    new_song(&state, &root, &stem)
}

#[tauri::command]
pub async fn song_rename(
    state: State<'_, ProjectState>,
    root: String,
    path: String,
    stem: String,
) -> Result<String, SongError> {
    rename_song(&state, &root, &path, &stem)
}

#[tauri::command]
pub async fn song_delete(
    state: State<'_, ProjectState>,
    root: String,
    path: String,
) -> Result<(), SongError> {
    delete_song(&state, &root, &path)
}

#[tauri::command]
pub async fn project_root_of(dir: String) -> Result<String, String> {
    root_of(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watcher::ProjectChanged;
    use std::fs;
    use std::sync::mpsc::Sender;
    use tempfile::TempDir;

    fn unwatched(_: &Project) -> Result<Watcher, String> {
        Ok(Watcher::inert())
    }

    fn watch_to(tx: Sender<ProjectChanged>) -> impl FnOnce(&Project) -> Result<Watcher, String> {
        |project| {
            Watcher::start(project, move |changed| {
                let _ = tx.send(changed);
            })
        }
    }

    struct Env {
        tmp: TempDir,
        state: ProjectState,
    }

    impl Env {
        fn new() -> Env {
            Env {
                tmp: tempfile::tempdir().unwrap(),
                state: ProjectState::default(),
            }
        }
        fn recents(&self) -> PathBuf {
            self.tmp.path().join("data/recent.json")
        }
        fn dir(&self, name: &str) -> String {
            self.tmp.path().join(name).to_string_lossy().into_owned()
        }
        fn create(&self, name: &str) -> ProjectInfo {
            create_project(
                &self.state,
                &self.recents(),
                &self.dir(name),
                name,
                unwatched,
            )
            .unwrap()
        }
        /// The open project's root as `ProjectInfo::root` spells it.
        fn root(&self) -> String {
            self.state.current().unwrap().info().unwrap().root
        }
        fn recent_entries(&self) -> Vec<PathBuf> {
            Recents::load(&self.recents()).entries().to_vec()
        }
    }

    #[test]
    fn create_then_open_return_info_and_touch_recents() {
        let env = Env::new();
        let created = env.create("alpha");
        let root = Path::new(&env.dir("alpha")).canonicalize().unwrap();
        assert_eq!(created.name, "alpha");
        assert_eq!(created.root, root.to_string_lossy());
        assert_eq!(created.songs.len(), 1);
        assert_eq!(created.songs[0].path, "songs/main.js");
        assert_eq!(env.recent_entries(), std::slice::from_ref(&root));

        env.create("beta");
        close_project(&env.state);
        let opened =
            open_project(&env.state, &env.recents(), &env.dir("alpha"), unwatched).unwrap();
        assert_eq!(opened.root, created.root);
        assert_eq!(opened.songs, created.songs);
        assert_eq!(env.recent_entries()[0], root);
        assert_eq!(env.recent_entries().len(), 2);
    }

    #[test]
    fn every_open_path_runs_the_on_open_hook_once_installed() {
        let env = Env::new();
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = Arc::clone(&opened);
        env.state.on_open(move |project| {
            let root = project.root().to_string_lossy().into_owned();
            log.lock().unwrap().push(root);
        });
        let alpha = env.create("alpha");
        let err = open_project(&env.state, &env.recents(), &env.dir("nope"), unwatched);
        assert!(err.is_err());
        let beta = env.create("beta");
        open_project(&env.state, &env.recents(), &env.dir("alpha"), unwatched).unwrap();
        close_project(&env.state);
        let last = open_last(&env.state, &env.recents(), unwatched)
            .unwrap()
            .unwrap();
        assert_eq!(last.root, alpha.root);
        assert_eq!(
            *opened.lock().unwrap(),
            vec![
                alpha.root.clone(),
                beta.root,
                alpha.root.clone(),
                alpha.root
            ]
        );
    }

    #[test]
    fn create_into_a_non_empty_folder_errors_and_keeps_the_open_project() {
        let env = Env::new();
        let first = env.create("alpha");
        fs::create_dir(env.dir("busy")).unwrap();
        fs::write(Path::new(&env.dir("busy")).join("x"), "").unwrap();
        let err = create_project(
            &env.state,
            &env.recents(),
            &env.dir("busy"),
            "busy",
            unwatched,
        )
        .unwrap_err();
        assert!(err.contains("already contains files"), "{err}");
        assert_eq!(env.recent_entries().len(), 1);
        assert_eq!(list_songs(&env.state, &first.root).unwrap(), first.songs);
    }

    #[test]
    fn open_of_a_non_project_errors_without_touching_recents_or_the_open_project() {
        let env = Env::new();
        let err =
            open_project(&env.state, &env.recents(), &env.dir("nope"), unwatched).unwrap_err();
        assert!(!env.recents().exists(), "{err}");

        fs::create_dir(env.dir("plain")).unwrap();
        let err =
            open_project(&env.state, &env.recents(), &env.dir("plain"), unwatched).unwrap_err();
        assert!(err.contains("not a strudeland project"), "{err}");
        assert!(!env.recents().exists());

        let first = env.create("alpha");
        let before = env.recent_entries();
        let err =
            open_project(&env.state, &env.recents(), &env.dir("plain"), unwatched).unwrap_err();
        assert!(err.contains("not a strudeland project"), "{err}");
        assert_eq!(env.recent_entries(), before);
        assert_eq!(list_songs(&env.state, &first.root).unwrap(), first.songs);
    }

    #[test]
    fn watcher_start_failure_preserves_the_open_project_and_recents() {
        let env = Env::new();
        let first = env.create("alpha");
        let before = env.recent_entries();
        let error = create_project(&env.state, &env.recents(), &env.dir("beta"), "beta", |_| {
            Err("watch failed".to_owned())
        })
        .unwrap_err();
        assert_eq!(error, "watch failed");
        assert_eq!(env.root(), first.root);
        assert_eq!(env.recent_entries(), before);
    }

    #[test]
    fn opening_another_project_replaces_the_open_one() {
        let env = Env::new();
        env.create("alpha");
        let beta = env.create("beta");
        save_song(&env.state, &env.root(), "songs/main.js", "// beta").unwrap();
        assert_eq!(beta.name, "beta");
        let root = Path::new(&beta.root);
        assert_eq!(
            fs::read_to_string(root.join("songs/main.js")).unwrap(),
            "// beta"
        );
    }

    #[test]
    fn a_project_taken_from_the_state_outlives_a_switch() {
        let env = Env::new();
        env.create("alpha");
        save_song(&env.state, &env.root(), "songs/main.js", "// alpha").unwrap();
        let alpha = env.state.current().unwrap();
        env.create("beta");
        assert_eq!(alpha.read("songs/main.js").unwrap(), "// alpha");
        assert_eq!(env.state.current().unwrap().manifest().name, "beta");
    }

    #[test]
    fn project_info_rereads_the_manifest_and_keeps_watching() {
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let env = Env::new();
        let (tx, rx) = channel();
        let created = create_project(
            &env.state,
            &env.recents(),
            &env.dir("alpha"),
            "alpha",
            watch_to(tx),
        )
        .unwrap();
        let recents_before = fs::read_to_string(env.recents()).unwrap();
        let manifest = Path::new(&created.root).join("project.json");
        fs::write(&manifest, r#"{"schema":1,"name":"renamed"}"#).unwrap();
        new_song(&env.state, &env.root(), "b").unwrap();

        let info = reload_info(&env.state, &env.root()).unwrap();
        assert_eq!(info.root, created.root);
        assert_eq!(info.name, "renamed");
        let paths: Vec<_> = info.songs.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["songs/b.js", "songs/main.js"]);
        assert_eq!(env.state.current().unwrap().manifest().name, "renamed");
        assert_eq!(fs::read_to_string(env.recents()).unwrap(), recents_before);

        // The original watcher is still the one running: it still reports into our channel.
        while rx.recv_timeout(Duration::from_millis(1200)).is_ok() {}
        save_song(&env.state, &env.root(), "songs/main.js", "1").unwrap();
        let wait = Duration::from_secs(10);
        loop {
            let c = rx.recv_timeout(wait).expect("the watcher reports the save");
            assert_eq!(c.root, created.root, "batches carry the project's root");
            if c.songs.changed.iter().any(|p| p == "songs/main.js") {
                break;
            }
        }
    }

    #[test]
    fn project_info_with_a_broken_manifest_errors_and_keeps_the_open_project() {
        let env = Env::new();
        let created = env.create("alpha");
        let manifest = Path::new(&created.root).join("project.json");
        fs::write(&manifest, "{ nope").unwrap();
        let err = reload_info(&env.state, &env.root()).unwrap_err();
        assert!(err.contains("invalid project.json"), "{err}");
        assert_eq!(env.state.current().unwrap().manifest().name, "alpha");
        assert_eq!(
            list_songs(&env.state, &created.root).unwrap(),
            created.songs
        );

        fs::write(&manifest, r#"{"schema":9,"name":"x"}"#).unwrap();
        let err = reload_info(&env.state, &env.root()).unwrap_err();
        assert!(err.contains("unsupported project schema"), "{err}");
        assert_eq!(env.state.current().unwrap().manifest().name, "alpha");
    }

    #[test]
    fn song_commands_without_a_project_error() {
        let env = Env::new();
        let msg = "no project is open";
        assert_eq!(list_songs(&env.state, "/nowhere").unwrap_err(), msg);
        assert_eq!(
            read_song(&env.state, "/nowhere", "songs/main.js").unwrap_err(),
            msg
        );
        assert_eq!(
            save_song(&env.state, "/nowhere", "songs/main.js", "x").unwrap_err(),
            msg
        );
        let err = new_song(&env.state, "/nowhere", "x").unwrap_err();
        assert_eq!(err.message, msg);
        assert_eq!(err.kind, SongErrorKind::Other);
        assert_eq!(
            rename_song(&env.state, "/nowhere", "songs/a.js", "b")
                .unwrap_err()
                .kind,
            SongErrorKind::Other
        );
        assert_eq!(
            delete_song(&env.state, "/nowhere", "songs/a.js")
                .unwrap_err()
                .message,
            msg
        );
        assert_eq!(reload_info(&env.state, "/nowhere").unwrap_err(), msg);
    }

    #[test]
    fn commands_for_the_open_root_act_on_it() {
        let env = Env::new();
        let alpha = env.create("alpha");
        let root = alpha.root.as_str();
        save_song(&env.state, root, "songs/main.js", "// a").unwrap();
        assert_eq!(
            read_song(&env.state, root, "songs/main.js").unwrap(),
            "// a"
        );
        assert_eq!(new_song(&env.state, root, "b").unwrap(), "songs/b.js");
        assert_eq!(list_songs(&env.state, root).unwrap().len(), 2);
        assert_eq!(reload_info(&env.state, root).unwrap().root, alpha.root);
    }

    #[test]
    fn commands_for_a_stale_root_fail_and_touch_nothing() {
        let env = Env::new();
        let alpha = env.create("alpha");
        let beta = env.create("beta");
        let stale = alpha.root.as_str();
        let msg = "the project changed";
        assert_eq!(PROJECT_CHANGED, msg);
        assert_eq!(
            save_song(&env.state, stale, "songs/main.js", "// late").unwrap_err(),
            msg
        );
        assert_eq!(
            read_song(&env.state, stale, "songs/main.js").unwrap_err(),
            msg
        );
        let expected = SongError {
            kind: SongErrorKind::ProjectChanged,
            message: msg.to_string(),
        };
        assert_eq!(new_song(&env.state, stale, "late").unwrap_err(), expected);
        assert_eq!(
            rename_song(&env.state, stale, "songs/main.js", "late").unwrap_err(),
            expected
        );
        assert_eq!(
            delete_song(&env.state, stale, "songs/main.js").unwrap_err(),
            expected
        );
        assert_eq!(list_songs(&env.state, stale).unwrap_err(), msg);
        assert_eq!(reload_info(&env.state, stale).unwrap_err(), msg);
        // A root that only differs by spelling (not the canonical form) is also stale.
        let uncanonical = format!("{}/", beta.root);
        assert_eq!(list_songs(&env.state, &uncanonical).unwrap_err(), msg);

        for info in [&alpha, &beta] {
            let songs = Path::new(&info.root).join("songs");
            assert_ne!(
                fs::read_to_string(songs.join("main.js")).unwrap(),
                "// late"
            );
            assert!(!songs.join("late.js").exists());
        }
        assert_eq!(list_songs(&env.state, &beta.root).unwrap(), beta.songs);
    }

    #[test]
    fn song_save_and_read_round_trip() {
        let env = Env::new();
        env.create("alpha");
        save_song(&env.state, &env.root(), "songs/main.js", "s(\"bd\")").unwrap();
        assert_eq!(
            read_song(&env.state, &env.root(), "songs/main.js").unwrap(),
            "s(\"bd\")"
        );
    }

    #[test]
    fn song_paths_outside_songs_are_rejected() {
        let env = Env::new();
        env.create("alpha");
        for bad in ["../x.js", "songs/a/b.js", "/etc/passwd", "project.json"] {
            assert!(read_song(&env.state, &env.root(), bad).is_err(), "{bad}");
            assert!(
                save_song(&env.state, &env.root(), bad, "x").is_err(),
                "{bad}"
            );
        }
        assert!(!Path::new(&env.dir("x.js")).exists());
    }

    #[test]
    fn song_create_returns_the_rel_path_and_rejects_duplicates() {
        let env = Env::new();
        env.create("alpha");
        assert_eq!(
            new_song(&env.state, &env.root(), "jam").unwrap(),
            "songs/jam.js"
        );
        let err = new_song(&env.state, &env.root(), "jam").unwrap_err();
        assert!(err.message.contains("already exists"), "{}", err.message);
        assert_eq!(err.kind, SongErrorKind::SongExists);
        let err = new_song(&env.state, &env.root(), "bad/name").unwrap_err();
        assert_eq!(err.kind, SongErrorKind::InvalidName);
        let paths: Vec<_> = list_songs(&env.state, &env.root())
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect();
        assert_eq!(paths, ["songs/jam.js", "songs/main.js"]);
    }

    #[test]
    fn song_rename_and_delete_act_on_the_open_project() {
        let env = Env::new();
        env.create("alpha");
        let root = env.root();
        new_song(&env.state, &root, "jam").unwrap();
        assert_eq!(
            rename_song(&env.state, &root, "songs/jam.js", "groove").unwrap(),
            "songs/groove.js"
        );
        let paths = |env: &Env| -> Vec<String> {
            list_songs(&env.state, &env.root())
                .unwrap()
                .into_iter()
                .map(|s| s.path)
                .collect()
        };
        assert_eq!(paths(&env), ["songs/groove.js", "songs/main.js"]);
        delete_song(&env.state, &root, "songs/groove.js").unwrap();
        assert_eq!(paths(&env), ["songs/main.js"]);
    }

    #[test]
    fn song_error_kinds_follow_the_project_error() {
        let env = Env::new();
        env.create("alpha");
        let root = env.root();
        let kind = |e: SongError| e.kind;
        assert_eq!(
            kind(rename_song(&env.state, &root, "songs/main.js", "main").unwrap_err()),
            SongErrorKind::SongExists
        );
        let err = rename_song(&env.state, &root, "songs/nope.js", "x").unwrap_err();
        assert_eq!(err.kind, SongErrorKind::SongNotFound);
        assert_eq!(err.message, "song songs/nope.js not found");
        assert_eq!(
            kind(delete_song(&env.state, &root, "songs/nope.js").unwrap_err()),
            SongErrorKind::SongNotFound
        );
        assert_eq!(
            kind(rename_song(&env.state, &root, "songs/main.js", "a/b").unwrap_err()),
            SongErrorKind::InvalidName
        );
        // Path scoping and I/O failures are `other`.
        assert_eq!(
            kind(delete_song(&env.state, &root, "../x.js").unwrap_err()),
            SongErrorKind::Other
        );
    }

    #[test]
    fn song_error_serializes_as_kind_and_message() {
        let err = SongError {
            kind: SongErrorKind::SongExists,
            message: "song songs/x.js already exists".into(),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"kind":"songExists","message":"song songs/x.js already exists"}"#
        );
        for (kind, wire) in [
            (SongErrorKind::SongNotFound, "songNotFound"),
            (SongErrorKind::InvalidName, "invalidName"),
            (SongErrorKind::ProjectChanged, "projectChanged"),
            (SongErrorKind::Other, "other"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), wire);
        }
    }

    #[test]
    fn root_of_returns_the_canonical_root_and_installs_nothing() {
        let env = Env::new();
        let info = env.create("alpha");
        let canonical = info.root.clone();
        env.state.replace(None);
        assert_eq!(root_of(&env.dir("alpha")).unwrap(), canonical);
        assert_eq!(
            root_of(&format!("{}/../alpha", env.dir("alpha"))).unwrap(),
            canonical
        );
        let link = env.dir("link");
        std::os::unix::fs::symlink(env.dir("alpha"), &link).unwrap();
        assert_eq!(root_of(&link).unwrap(), canonical);
        assert!(env.state.current().is_err());
    }

    #[test]
    fn root_of_a_non_project_or_missing_folder_errors() {
        let env = Env::new();
        let plain = env.dir("plain");
        fs::create_dir(&plain).unwrap();
        let err = root_of(&plain).unwrap_err();
        assert!(err.contains("not a strudeland project"), "{err}");
        assert!(root_of(&env.dir("missing")).is_err());
        assert!(env.state.current().is_err());
        assert!(env.recent_entries().is_empty());
    }

    #[test]
    fn close_drops_the_project_and_leaves_recents() {
        let env = Env::new();
        env.create("alpha");
        let before = fs::read_to_string(env.recents()).unwrap();
        close_project(&env.state);
        assert_eq!(
            list_songs(&env.state, "/nowhere").unwrap_err(),
            "no project is open"
        );
        assert_eq!(fs::read_to_string(env.recents()).unwrap(), before);
        close_project(&env.state);
    }

    #[test]
    fn last_with_empty_recents_is_none() {
        let env = Env::new();
        assert_eq!(
            open_last(&env.state, &env.recents(), unwatched).unwrap(),
            None
        );
        assert!(list_songs(&env.state, "/nowhere").is_err());
    }

    #[test]
    fn last_skips_deleted_and_broken_projects_and_prunes() {
        let env = Env::new();
        let good = env.create("good");
        env.create("broken");
        env.create("gone");
        fs::write(Path::new(&env.dir("broken")).join("project.json"), "{ nope").unwrap();
        fs::remove_dir_all(env.dir("gone")).unwrap();
        close_project(&env.state);

        let opened = open_last(&env.state, &env.recents(), unwatched)
            .unwrap()
            .unwrap();
        assert_eq!(opened.root, good.root);
        assert_eq!(list_songs(&env.state, &env.root()).unwrap(), good.songs);
        // "gone" is pruned; "broken" still has a project.json so it stays, but "good" is first.
        let entries = env.recent_entries();
        assert_eq!(entries[0], Path::new(&good.root));
        assert!(entries.iter().all(|p| !p.ends_with("gone")));
    }

    #[test]
    fn last_is_none_when_nothing_opens() {
        let env = Env::new();
        env.create("broken");
        fs::write(Path::new(&env.dir("broken")).join("project.json"), "{ nope").unwrap();
        close_project(&env.state);
        assert_eq!(
            open_last(&env.state, &env.recents(), unwatched).unwrap(),
            None
        );
        assert!(list_songs(&env.state, "/nowhere").is_err());
    }

    #[test]
    fn open_starts_a_watcher_and_switching_or_closing_stops_it() {
        use std::sync::mpsc::{RecvTimeoutError, channel};
        use std::time::Duration;
        let wait = Duration::from_secs(10);
        let quiet = Duration::from_millis(1200);

        let env = Env::new();
        let (tx_a, rx_a) = channel();
        create_project(
            &env.state,
            &env.recents(),
            &env.dir("alpha"),
            "alpha",
            watch_to(tx_a),
        )
        .unwrap();
        save_song(&env.state, &env.root(), "songs/main.js", "1").unwrap();
        let c = rx_a
            .recv_timeout(wait)
            .expect("alpha's watcher reports the save");
        assert_eq!(c.songs.changed, ["songs/main.js"], "{c:?}");

        // Switching drops alpha's watcher (its sender goes with it).
        let (tx_b, rx_b) = channel();
        open_project(
            &env.state,
            &env.recents(),
            &env.dir("nope"),
            watch_to(tx_b.clone()),
        )
        .unwrap_err();
        create_project(
            &env.state,
            &env.recents(),
            &env.dir("beta"),
            "beta",
            watch_to(tx_b),
        )
        .unwrap();
        fs::write(Path::new(&env.dir("alpha")).join("songs/main.js"), "2").unwrap();
        loop {
            match rx_a.recv_timeout(quiet) {
                Err(RecvTimeoutError::Disconnected) => break,
                Ok(c) => assert_eq!(c.songs.changed, ["songs/main.js"], "late batch: {c:?}"),
                Err(RecvTimeoutError::Timeout) => panic!("alpha's watcher is still running"),
            }
        }

        save_song(&env.state, &env.root(), "songs/main.js", "3").unwrap();
        let c = rx_b
            .recv_timeout(wait)
            .expect("beta's watcher reports the save");
        assert_eq!(c.songs.changed, ["songs/main.js"], "{c:?}");

        close_project(&env.state);
        loop {
            match rx_b.recv_timeout(quiet) {
                Err(RecvTimeoutError::Disconnected) => break,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => panic!("beta's watcher is still running"),
            }
        }
    }

    #[test]
    fn capability_file_grants_exactly_the_expected_permissions() {
        let text = include_str!("../capabilities/default.json");
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let perms: Vec<&str> = json["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().or(p["identifier"].as_str()).unwrap())
            .collect();
        assert_eq!(
            perms,
            [
                "core:default",
                "opener:default",
                // Scoped to the microphone settings URL (checked in `recording`).
                "opener:allow-open-url",
                "dialog:allow-open",
                "dialog:allow-message",
                "core:window:allow-set-title",
                "core:window:allow-destroy",
            ]
        );
    }
}

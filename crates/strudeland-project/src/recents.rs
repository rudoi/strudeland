//! The recent projects list, one document of the app data dir: `recent.json`, stored as
//! `{ "schema": 1, "projects": ["/abs/path", ...] }`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_data::{self, AppData, Loaded};
use crate::error::ProjectError;
use crate::project::{MANIFEST_FILE, Project};

/// The recent projects file name inside the app data dir.
pub const RECENTS_FILE: &str = "recent.json";

/// How many recent projects are kept.
pub const MAX_RECENTS: usize = 10;

const RECENTS_SCHEMA: u32 = 1;

/// The document's fields besides `schema`. `projects` is required.
#[derive(Default, Serialize, Deserialize)]
struct RecentsDoc {
    projects: Vec<PathBuf>,
}

/// A recent project whose manifest still parses, as the app lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    /// The folder as stored in the list.
    pub path: String,
    pub name: String,
}

/// Recently opened project folders, most recent first, backed by a JSON file.
///
/// The helpers that write the list back ([`Recents::touch_and_save`], [`Recents::load_pruned`],
/// [`Recents::list`]) are best effort: a failed write is ignored, because the list is a
/// convenience and must never stop the user from opening their project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recents {
    file: PathBuf,
    entries: Vec<PathBuf>,
}

impl Recents {
    /// Loads the list from the app data dir. A missing, unreadable or corrupt file, or one with
    /// another schema, gives an empty list (with a problem for every case but a missing file);
    /// this never fails.
    pub fn load(app: &AppData) -> Loaded<Recents> {
        let file = app.file(RECENTS_FILE);
        let Loaded { value, problem } = app_data::load::<RecentsDoc>(&file, RECENTS_SCHEMA);
        let mut recents = Recents {
            file,
            entries: Vec::new(),
        };
        // Re-add in reverse so a hand-edited file is deduped and capped like `touch` would.
        for entry in value.projects.iter().rev() {
            recents.touch(entry);
        }
        Loaded {
            value: recents,
            problem,
        }
    }

    /// Most recent first.
    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    /// Moves `project_root` to the front (inserting it if new) and keeps at most [`MAX_RECENTS`].
    pub fn touch(&mut self, project_root: &Path) {
        self.entries.retain(|p| p != project_root);
        self.entries.insert(0, project_root.to_path_buf());
        self.entries.truncate(MAX_RECENTS);
    }

    /// Drops entries whose folder no longer contains `project.json`. Returns whether any entry
    /// was dropped.
    pub fn prune(&mut self) -> bool {
        let before = self.entries.len();
        self.entries.retain(|p| p.join(MANIFEST_FILE).is_file());
        self.entries.len() != before
    }

    /// Records `project_root` as the most recent project. Best effort. Returns the load's
    /// problem as a `Loaded<()>`, for the caller to [`Loaded::log`].
    pub fn touch_and_save(app: &AppData, project_root: &Path) -> Loaded<()> {
        let Loaded {
            value: mut recents,
            problem,
        } = Recents::load(app);
        recents.touch(project_root);
        recents.save_best_effort();
        Loaded { value: (), problem }
    }

    /// Loads the list, prunes it and, only if pruning dropped an entry, writes the pruned list
    /// back. Best effort.
    pub fn load_pruned(app: &AppData) -> Loaded<Recents> {
        let mut loaded = Recents::load(app);
        if loaded.value.prune() {
            loaded.value.save_best_effort();
        }
        loaded
    }

    /// Opens each entry in order, lazily, skipping those that no longer open (a broken or
    /// unsupported `project.json`, an unreadable folder). Yields the stored path with the project.
    pub fn open_each(&self) -> impl Iterator<Item = (&Path, Project)> + '_ {
        self.entries
            .iter()
            .filter_map(|path| Some((path.as_path(), Project::open(path).ok()?)))
    }

    /// The pruned list (see [`Recents::load_pruned`]) with each project's name, leaving out
    /// entries that no longer open.
    pub fn list(app: &AppData) -> Loaded<Vec<RecentProject>> {
        let Loaded { value, problem } = Recents::load_pruned(app);
        let value = value
            .open_each()
            .map(|(path, project)| RecentProject {
                path: path.to_string_lossy().into_owned(),
                name: project.manifest().name.clone(),
            })
            .collect();
        Loaded { value, problem }
    }

    /// The one place the best-effort policy lives: the list is a convenience, so a failed write
    /// is dropped.
    fn save_best_effort(&self) {
        let _ = self.save();
    }

    /// Writes the list atomically, creating the parent directory if needed.
    pub fn save(&self) -> Result<(), ProjectError> {
        let doc = RecentsDoc {
            projects: self.entries.clone(),
        };
        app_data::save(&self.file, RECENTS_SCHEMA, &doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_data::tempdir;
    use std::fs;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| PathBuf::from(format!("/p/{n}")))
            .collect()
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let file = tmp.path().join(RECENTS_FILE);
        for text in [r#"{"schema":1}"#, r#"{"schema":1,"projects":[1]}"#] {
            fs::write(&file, text).unwrap();
            let loaded = Recents::load(&app);
            assert!(loaded.value.entries().is_empty(), "{text:?}");
            assert!(loaded.problem.is_some(), "{text:?}");
        }
    }

    #[test]
    fn load_reads_entries_in_order_deduped_and_capped() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let file = tmp.path().join(RECENTS_FILE);
        let mut projects: Vec<String> = (0..12).map(|i| format!("/p/{i}")).collect();
        projects.insert(1, "/p/0".into());
        let json = serde_json::json!({ "schema": 1, "projects": projects });
        fs::write(&file, json.to_string()).unwrap();
        let expected: Vec<PathBuf> = (0..10).map(|i| PathBuf::from(format!("/p/{i}"))).collect();
        assert_eq!(Recents::load(&app).value.entries(), expected);
    }

    #[test]
    fn touch_moves_to_front_and_dedupes() {
        let tmp = tempdir();
        let mut recents = Recents::load(&AppData::at(tmp.path())).value;
        recents.touch(Path::new("/p/a"));
        recents.touch(Path::new("/p/b"));
        recents.touch(Path::new("/p/c"));
        assert_eq!(recents.entries(), paths(&["c", "b", "a"]));
        recents.touch(Path::new("/p/a"));
        assert_eq!(recents.entries(), paths(&["a", "c", "b"]));
        recents.touch(Path::new("/p/a"));
        assert_eq!(recents.entries(), paths(&["a", "c", "b"]));
    }

    #[test]
    fn touch_caps_at_max_recents() {
        let tmp = tempdir();
        let mut recents = Recents::load(&AppData::at(tmp.path())).value;
        for i in 0..(MAX_RECENTS + 3) {
            recents.touch(&PathBuf::from(format!("/p/{i}")));
        }
        assert_eq!(recents.entries().len(), MAX_RECENTS);
        assert_eq!(
            recents.entries()[0],
            PathBuf::from(format!("/p/{}", MAX_RECENTS + 2))
        );
        assert_eq!(recents.entries()[MAX_RECENTS - 1], PathBuf::from("/p/3"));
    }

    #[test]
    fn prune_drops_folders_without_project_json() {
        let tmp = tempdir();
        let live = tmp.path().join("live");
        let dead = tmp.path().join("dead");
        let gone = tmp.path().join("gone");
        let file_not_dir = tmp.path().join("weird");
        fs::create_dir(&live).unwrap();
        fs::write(live.join("project.json"), "{}").unwrap();
        fs::create_dir(&dead).unwrap();
        fs::create_dir_all(file_not_dir.join("project.json")).unwrap();
        let mut recents = Recents::load(&AppData::at(tmp.path())).value;
        for p in [&live, &dead, &gone, &file_not_dir] {
            recents.touch(p);
        }
        recents.prune();
        assert_eq!(recents.entries(), [live]);
    }

    #[test]
    fn saves_exactly_todays_bytes() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path().join("app/data"));
        let file = app.dir().join(RECENTS_FILE);
        let mut recents = Recents::load(&app).value;
        recents.touch(Path::new("/p/a"));
        recents.touch(Path::new("/p/b"));
        recents.save().unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"schema\": 1,\n  \"projects\": [\n    \"/p/b\",\n    \"/p/a\"\n  ]\n}\n"
        );
        assert_eq!(Recents::load(&app).value, recents);
    }

    /// Creates projects named `names` under `tmp` and touches them into `app` in order, so the
    /// last name is first. Returns their canonical roots in the same order as `names`.
    fn touched(tmp: &Path, app: &AppData, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| {
                let root = Project::create(&tmp.join(name), name)
                    .unwrap()
                    .root()
                    .to_path_buf();
                Recents::touch_and_save(app, &root);
                root
            })
            .collect()
    }

    #[test]
    fn touch_and_save_moves_to_front_and_persists() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path().join("data"));
        Recents::touch_and_save(&app, Path::new("/p/a"));
        Recents::touch_and_save(&app, Path::new("/p/b"));
        Recents::touch_and_save(&app, Path::new("/p/a"));
        assert_eq!(Recents::load(&app).value.entries(), paths(&["a", "b"]));
    }

    #[test]
    fn touch_and_save_ignores_a_failed_write() {
        let tmp = tempdir();
        let blocker = tmp.path().join("blocker");
        fs::write(&blocker, "").unwrap();
        Recents::touch_and_save(&AppData::at(&blocker), Path::new("/p/a"));
        assert_eq!(fs::read(&blocker).unwrap(), b"");
    }

    #[test]
    fn load_pruned_writes_the_pruned_list_back() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let roots = touched(tmp.path(), &app, &["a", "gone", "b"]);
        fs::remove_dir_all(&roots[1]).unwrap();
        let expected = [roots[2].clone(), roots[0].clone()];
        assert_eq!(Recents::load_pruned(&app).value.entries(), expected);
        assert_eq!(Recents::load(&app).value.entries(), expected);
    }

    #[test]
    fn load_pruned_leaves_the_file_alone_when_nothing_is_pruned() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let file = tmp.path().join(RECENTS_FILE);
        let roots = touched(tmp.path(), &app, &["a", "b"]);
        // Store the list in a form `save` never writes (compact JSON), so any rewrite shows.
        let compact = serde_json::json!({ "schema": 1, "projects": [&roots[1], &roots[0]] });
        let compact = compact.to_string();
        fs::write(&file, &compact).unwrap();
        assert_eq!(
            Recents::load_pruned(&app).value.entries(),
            [roots[1].clone(), roots[0].clone()]
        );
        assert_eq!(Recents::list(&app).value.len(), 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), compact);
    }

    #[test]
    fn open_each_skips_entries_that_do_not_open() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let roots = touched(tmp.path(), &app, &["a", "broken", "b"]);
        fs::write(roots[1].join(MANIFEST_FILE), "{ nope").unwrap();
        let recents = Recents::load_pruned(&app).value;
        // A broken manifest is still a file, so the entry survives pruning but does not open.
        assert_eq!(recents.entries().len(), 3);
        let opened: Vec<_> = recents
            .open_each()
            .map(|(path, project)| (path.to_path_buf(), project.manifest().name.clone()))
            .collect();
        assert_eq!(
            opened,
            [
                (roots[2].clone(), "b".to_string()),
                (roots[0].clone(), "a".to_string())
            ]
        );
    }

    #[test]
    fn list_returns_names_and_drops_dead_entries() {
        let tmp = tempdir();
        let app = AppData::at(tmp.path());
        let roots = touched(tmp.path(), &app, &["alpha", "broken", "gone", "beta"]);
        fs::write(roots[1].join(MANIFEST_FILE), "[]").unwrap();
        fs::remove_dir_all(&roots[2]).unwrap();

        let list = Recents::list(&app).value;
        assert_eq!(
            list,
            [
                RecentProject {
                    path: roots[3].to_string_lossy().into_owned(),
                    name: "beta".into(),
                },
                RecentProject {
                    path: roots[0].to_string_lossy().into_owned(),
                    name: "alpha".into(),
                },
            ]
        );
        assert_eq!(
            Recents::load(&app).value.entries().len(),
            3,
            "gone is pruned"
        );
        assert_eq!(
            Recents::list(&AppData::at(tmp.path().join("missing"))).value,
            []
        );
    }

    #[test]
    fn recent_project_serialises_camel_case() {
        let recent = RecentProject {
            path: "/p".into(),
            name: "n".into(),
        };
        assert_eq!(
            serde_json::to_value(&recent).unwrap(),
            serde_json::json!({ "path": "/p", "name": "n" })
        );
    }

    #[test]
    fn save_reports_an_unwritable_parent() {
        let tmp = tempdir();
        let blocker = tmp.path().join("blocker");
        fs::write(&blocker, "").unwrap();
        let recents = Recents::load(&AppData::at(&blocker)).value;
        assert!(matches!(
            recents.save().unwrap_err(),
            ProjectError::Io { .. }
        ));
    }
}

//! The encoders' and decoders' licences ship with the crate: each covered crate is in the
//! dependency tree, its licence file exists in the crate root with the expected text, the README
//! names every licence file, and every dependency is pinned exactly.

use std::collections::HashMap;
use std::path::PathBuf;

/// `(crate, licence file)`; every package whose name starts with `symphonia` maps to
/// `LICENSE-symphonia` (see `covered`).
const COVERED: &[(&str, &str)] = &[
    ("mp3lame-encoder", "LICENSE-lame"),
    ("mp3lame-sys", "LICENSE-lame"),
    ("vorbis_rs", "LICENSE-vorbis"),
    ("aotuv_lancer_vorbis_sys", "LICENSE-vorbis"),
    ("ogg_next_sys", "LICENSE-vorbis"),
    ("opusic-sys", "LICENSE-opus"),
    ("opus", "LICENSE-opus-rs"),
    ("ogg", "LICENSE-ogg-rs"),
    ("rubato", "LICENSE-rubato"),
    ("realfft", "LICENSE-rubato"),
    ("audioadapter", "LICENSE-rubato"),
    ("audioadapter-buffers", "LICENSE-rubato"),
    ("audioadapter-sample", "LICENSE-rubato"),
];

/// The text each licence file must contain.
const NEEDLES: &[(&str, &[&str])] = &[
    (
        "LICENSE-lame",
        &[
            "GNU LESSER GENERAL PUBLIC LICENSE",
            "Version 3, 29 June 2007",
            "GNU LIBRARY GENERAL PUBLIC LICENSE",
            "LAME",
        ],
    ),
    (
        "LICENSE-vorbis",
        &[
            "Aoyumi",
            "Xiph.org Foundation",
            "Redistribution and use in source and binary forms",
        ],
    ),
    (
        "LICENSE-opus",
        &[
            "Xiph.Org, Skype Limited",
            "Neither the name of Internet Society",
        ],
    ),
    ("LICENSE-opus-rs", &["Tad Hardesty", "Apache License"]),
    ("LICENSE-ogg-rs", &["est31", "Xiph.org Foundation"]),
    ("LICENSE-symphonia", &["Mozilla Public License Version 2.0"]),
    ("LICENSE-rubato", &["Henrik Enquist", "Apache License"]),
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn crate_file(name: &str) -> String {
    let path = crate_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The names of the packages reachable from `root` through `Cargo.lock`'s dependency lists.
fn lock_closure(lock: &str, root: &str) -> Vec<String> {
    // Each `[[package]]` block: its name and the first word of each `dependencies` entry
    // (`"name"` or `"name version"`).
    let mut deps: HashMap<String, Vec<String>> = Default::default();
    for block in lock.split("[[package]]").skip(1) {
        let name = block
            .lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .and_then(|l| l.strip_suffix('"'))
            .expect("package name")
            .to_string();
        let list = block
            .split_once("dependencies = [")
            .map(|(_, rest)| rest.split_once(']').map_or(rest, |(list, _)| list))
            .unwrap_or("");
        let names = list
            .split('"')
            .skip(1)
            .step_by(2)
            .map(|entry| entry.split(' ').next().unwrap_or(entry).to_string())
            .collect();
        deps.entry(name).or_default().extend::<Vec<String>>(names);
    }
    let mut seen = vec![root.to_string()];
    let mut next = 0;
    while next < seen.len() {
        for dep in deps.get(&seen[next]).cloned().unwrap_or_default() {
            if !seen.contains(&dep) {
                seen.push(dep);
            }
        }
        next += 1;
    }
    seen
}

fn render_closure() -> Vec<String> {
    let lock = std::fs::read_to_string(crate_dir().join("../../Cargo.lock")).unwrap();
    lock_closure(&lock, "strudeland-render")
}

/// The licence file of each covered package in `closure`, and the problems found: a table entry
/// whose crate is not in the closure, a missing file, a file without its needles.
fn check(closure: &[String], covered: &[(&str, &str)], dir: &std::path::Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut entries: Vec<(String, String)> = covered
        .iter()
        .map(|(c, f)| (c.to_string(), f.to_string()))
        .collect();
    let symphonia: Vec<&String> = closure
        .iter()
        .filter(|n| n.starts_with("symphonia"))
        .collect();
    if symphonia.is_empty() {
        problems.push("no symphonia crate in the closure".to_string());
    }
    entries.extend(
        symphonia
            .into_iter()
            .map(|n| (n.clone(), "LICENSE-symphonia".to_string())),
    );
    for (name, file) in &entries {
        if !closure.iter().any(|n| n == name) {
            problems.push(format!(
                "{name} is not a dependency (stale entry for {file})"
            ));
        }
        match std::fs::read_to_string(dir.join(file)) {
            Err(e) => problems.push(format!("{file} ({name}): {e}")),
            Ok(text) => {
                let needles = NEEDLES
                    .iter()
                    .find(|(f, _)| f == file)
                    .map(|(_, n)| *n)
                    .unwrap_or_default();
                if needles.is_empty() {
                    problems.push(format!("{file} has no needles"));
                }
                for needle in needles {
                    if !text.contains(needle) {
                        problems.push(format!("{file} lacks {needle:?}"));
                    }
                }
            }
        }
    }
    problems
}

#[test]
fn every_encoder_licence_is_shipped_and_current() {
    let closure = render_closure();
    let problems = check(&closure, COVERED, &crate_dir());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn every_encoder_dependency_has_an_entry() {
    // The C-backed and notice-requiring families: a new crate of one of them needs an entry.
    let closure = render_closure();
    let families = [
        "mp3lame",
        "vorbis",
        "ogg",
        "opus",
        "rubato",
        "realfft",
        "audioadapter",
    ];
    for name in &closure {
        if !name.starts_with("symphonia") && families.iter().any(|f| name.contains(f)) {
            assert!(
                COVERED.iter().any(|(c, _)| c == name),
                "{name} has no licence entry"
            );
        }
    }
}

#[test]
fn the_check_fails_on_a_missing_file_or_a_stale_entry() {
    let closure = render_closure();
    // A licence file renamed away.
    let dir = tempfile::tempdir().unwrap();
    for (file, _) in NEEDLES {
        std::fs::copy(crate_dir().join(file), dir.path().join(file)).unwrap();
    }
    assert!(check(&closure, COVERED, dir.path()).is_empty());
    std::fs::rename(
        dir.path().join("LICENSE-opus"),
        dir.path().join("LICENSE-opus.old"),
    )
    .unwrap();
    let problems = check(&closure, COVERED, dir.path());
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("LICENSE-opus (opusic-sys)")),
        "{problems:#?}"
    );
    // A file without its text.
    std::fs::write(dir.path().join("LICENSE-opus"), "nothing here").unwrap();
    let problems = check(&closure, COVERED, dir.path());
    assert!(
        problems
            .iter()
            .any(|p| p.contains("lacks \"Xiph.Org, Skype Limited\"")),
        "{problems:#?}"
    );
    // An entry for a crate that is not a dependency.
    let mut stale = COVERED.to_vec();
    stale.push(("not-a-crate", "LICENSE-lame"));
    let problems = check(&closure, &stale, &crate_dir());
    assert_eq!(
        problems,
        ["not-a-crate is not a dependency (stale entry for LICENSE-lame)"]
    );
}

#[test]
fn the_readme_names_every_licence_and_the_lgpl_position() {
    let readme = crate_file("README.md");
    let mut files: Vec<String> = std::fs::read_dir(crate_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("LICENSE-"))
        .collect();
    files.sort();
    assert_eq!(files.len(), NEEDLES.len(), "{files:?}");
    for file in &files {
        assert!(readme.contains(file), "README.md does not name {file}");
    }
    for needle in ["statically linked", "relink", "LGPL", "cmake"] {
        assert!(readme.contains(needle), "README.md lacks {needle:?}");
    }
}

#[test]
fn every_dependency_is_pinned_exactly() {
    let manifest = crate_file("Cargo.toml");
    let mut in_deps = false;
    let mut count = 0;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.ends_with("dependencies]");
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        count += 1;
        if line.contains("path = ") {
            continue;
        }
        assert!(
            line.contains("= \"=") || line.contains("version = \"="),
            "not pinned exactly: {line}"
        );
    }
    assert!(count > 10, "{count} dependency lines");
}

//! V8's licence and the notices of the libraries linked with it ship with the crate, and the
//! README names the file; V8 stays out of the engine crate's dependency tree.

use std::path::PathBuf;

fn crate_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_v8_licence_is_shipped_and_named() {
    let licence = crate_file("LICENSE-v8");
    for (needle, what) in [
        ("the Deno authors", "the v8 crate's MIT licence"),
        (
            "Copyright 2014, the V8 project authors",
            "V8's BSD-3 licence",
        ),
        ("Sun Microsystems", "the fdlibm notice"),
        ("UNICODE LICENSE V3", "ICU"),
        ("abseil-cpp", "Abseil"),
        ("The simdutf authors", "simdutf"),
        ("LLVM Exceptions to the Apache 2.0 License", "libc++"),
        ("highway", "Highway"),
        ("fast_float", "fast_float"),
        ("Boost Software License", "Dragonbox"),
        ("Copyright (c) 2017 Facebook Inc.", "FP16"),
        ("The Chromium Authors", "Chromium"),
    ] {
        assert!(
            licence.contains(needle),
            "LICENSE-v8 lacks {what} ({needle})"
        );
    }
    assert!(crate_file("README.md").contains("LICENSE-v8"));
}

#[test]
fn v8_is_the_only_runtime_and_pinned_exactly() {
    let manifest = crate_file("Cargo.toml");
    assert!(!manifest.contains("[features]"), "no runtime features");
    assert!(manifest.contains("v8 = \"="), "v8 is pinned exactly");
}

/// The names of the packages reachable from `root` through `Cargo.lock`'s dependency lists.
fn lock_closure(lock: &str, root: &str) -> Vec<String> {
    // Each `[[package]]` block: its name and the first word of each `dependencies` entry
    // (`"name"` or `"name version"`).
    let mut deps: std::collections::HashMap<String, Vec<String>> = Default::default();
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

#[test]
fn the_engine_crate_does_not_depend_on_the_host_or_v8() {
    let lock_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    let host = lock_closure(&lock, "strudeland-host");
    assert!(
        host.iter().any(|n| n == "v8"),
        "the closure walk finds v8: {host:?}"
    );
    assert!(host.iter().any(|n| n == "strudeland-engine"), "{host:?}");
    let engine = lock_closure(&lock, "strudeland-engine");
    assert!(engine.iter().any(|n| n == "rustfft"), "{engine:?}");
    for banned in ["strudeland-host", "v8"] {
        assert!(
            !engine.iter().any(|n| n == banned),
            "the engine reaches {banned}: {engine:?}"
        );
    }
}

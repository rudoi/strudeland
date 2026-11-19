//! Every source file that ports Blink names its sources and carries the Chromium licence header;
//! `LICENSE-chromium` exists; the README names the tag. The `jsmath` ports of V8, LLVM libc and
//! CORE-MATH carry their family's two-line header, and each family's licence file exists at the
//! crate root exactly when some file carries that header.

use std::path::{Path, PathBuf};

const TAG: &str = "156.0.8078.4";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `text` mentions Blink: `third_party/blink` or a Blink C++ file name (`*.cc`, `*.h`).
fn mentions_blink(text: &str) -> bool {
    text.contains("third_party/blink")
        || text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            .any(|w| {
                let w = w.trim_end_matches('.');
                (w.ends_with(".cc") || w.ends_with(".h")) && w.len() > 4
            })
}

fn has_header(text: &str) -> bool {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    let second = lines.next().unwrap_or("");
    first.starts_with(&format!("// Port of Blink (Chromium {TAG}) "))
        && first.ends_with('.')
        && second == "// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium."
}

#[test]
fn ported_files_carry_the_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(!files.is_empty());
    let mut ported = 0;
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        if mentions_blink(&text) {
            ported += 1;
            assert!(
                has_header(&text),
                "{} mentions Blink but lacks the licence header",
                f.display()
            );
        }
    }
    assert!(
        ported >= 1,
        "render.rs ports offline_audio_context.cc and audio_utilities.cc"
    );
}

#[test]
fn worklet_files_carry_or_avoid_the_blink_header() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let worklet = std::fs::read_to_string(src.join("worklet.rs")).unwrap();
    assert!(has_header(&worklet), "worklet.rs ports Blink");
    let first = worklet.lines().next().unwrap();
    for f in [
        "modules/webaudio/audio_worklet_handler.cc",
        "modules/webaudio/audio_worklet_processor.cc",
        "modules/webaudio/audio_worklet_global_scope.cc",
        "modules/webaudio/base_audio_context.cc",
    ] {
        assert!(first.contains(f), "worklet.rs header names {f}");
    }
    // The processors port superdough's JS, not Blink: no Blink file named anywhere.
    for f in [
        "shaper.rs",
        "transient.rs",
        "ladder.rs",
        "djf.rs",
        "vowel.rs",
    ] {
        let text = std::fs::read_to_string(src.join(f)).unwrap();
        assert!(!mentions_blink(&text), "{f} names a Blink file");
        assert!(
            family_of(&text).unwrap().is_none(),
            "{f} has no port header"
        );
    }
}

#[test]
fn the_compressor_carries_its_blink_header() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let text = std::fs::read_to_string(src.join("compressor.rs")).unwrap();
    assert!(has_header(&text), "compressor.rs ports Blink");
    let first = text.lines().next().unwrap();
    for f in [
        "platform/audio/dynamics_compressor.cc",
        "modules/webaudio/dynamics_compressor_handler.cc",
        "modules/webaudio/dynamics_compressor_node.cc",
        "platform/audio/audio_utilities.cc",
    ] {
        assert!(first.contains(f), "compressor.rs header names {f}");
    }
}

#[test]
fn the_rule_detects_blink_mentions() {
    assert!(mentions_blink("see oscillator_handler.cc."));
    assert!(mentions_blink("platform/audio/biquad.cc"));
    assert!(mentions_blink("third_party/blink/renderer"));
    assert!(!mentions_blink("superdough.mjs:521 and lib.rs"));
    assert!(has_header(&format!(
        "// Port of Blink (Chromium {TAG}) platform/audio/biquad.cc.\n// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.\n"
    )));
    assert!(!has_header("// Port of Blink\n"));
}

#[test]
fn licence_file_and_readme() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let licence = std::fs::read_to_string(root.join("LICENSE-chromium")).unwrap();
    assert!(licence.contains("The Chromium Authors"));
    assert!(licence.contains("Google Inc."));
    assert!(licence.contains("Redistribution and use in source and binary forms"));
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains(TAG), "the README names the Blink tag");
}

/// A non-Blink source family: the first-line prefix, the exact second line and the licence file.
struct Family {
    prefix: &'static str,
    second: &'static str,
    licence: &'static str,
    /// Text the licence file must contain.
    marker: &'static str,
}

const FAMILIES: [Family; 3] = [
    Family {
        prefix: "// Port of V8 (Chromium 156.0.8078.4, V8 b976f35) ",
        second: "// Copyright the V8 project authors. BSD-3-Clause; see LICENSE-v8.",
        licence: "LICENSE-v8",
        marker: "Sun Microsystems",
    },
    Family {
        prefix: "// Port of LLVM libc (llvm-project ",
        second: "// Apache-2.0 WITH LLVM-exception; see LICENSE-llvm.",
        licence: "LICENSE-llvm",
        marker: "LLVM Exceptions",
    },
    Family {
        prefix: "// Port of CORE-MATH (",
        second: "// MIT; see LICENSE-core-math.",
        licence: "LICENSE-core-math",
        marker: "CORE-MATH",
    },
];

/// The family whose header `text` starts with, if any; `Err` when the first line names a family
/// but the header is malformed.
fn family_of(text: &str) -> Result<Option<&'static Family>, String> {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    let second = lines.next().unwrap_or("");
    for f in &FAMILIES {
        if first.starts_with(f.prefix) {
            if !first.ends_with('.') {
                return Err(format!("first line does not end in '.': {first:?}"));
            }
            if second != f.second {
                return Err(format!("second line {second:?}, expected {:?}", f.second));
            }
            return Ok(Some(f));
        }
    }
    if first.starts_with("// Port of ") && !first.starts_with("// Port of Blink ") {
        return Err(format!("unknown port header {first:?}"));
    }
    Ok(None)
}

#[test]
fn ported_math_files_and_licence_files_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut used = [false; 3];
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        match family_of(&text) {
            Err(e) => panic!("{}: {e}", f.display()),
            Ok(Some(fam)) => {
                let i = FAMILIES
                    .iter()
                    .position(|x| x.licence == fam.licence)
                    .unwrap();
                used[i] = true;
                assert!(
                    root.join(fam.licence).exists(),
                    "{} has the {} header but {} is missing",
                    f.display(),
                    fam.licence,
                    fam.licence
                );
            }
            Ok(None) => {}
        }
    }
    for (fam, used) in FAMILIES.iter().zip(used) {
        let path = root.join(fam.licence);
        if path.exists() {
            assert!(
                used,
                "{} exists but no source file carries its header",
                fam.licence
            );
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains(fam.marker),
                "{} lacks {:?}",
                fam.licence,
                fam.marker
            );
        }
    }
}

#[test]
fn the_family_rule_reads_headers() {
    let ok = "// Port of CORE-MATH (abc) src/binary64/exp/exp.c, cr_exp.\n// MIT; see LICENSE-core-math.\n";
    assert_eq!(
        family_of(ok).unwrap().map(|f| f.licence),
        Some("LICENSE-core-math")
    );
    let v8 = "// Port of V8 (Chromium 156.0.8078.4, V8 b976f35) src/base/ieee754, cosh.\n// Copyright the V8 project authors. BSD-3-Clause; see LICENSE-v8.\n";
    assert_eq!(
        family_of(v8).unwrap().map(|f| f.licence),
        Some("LICENSE-v8")
    );
    let llvm = "// Port of LLVM libc (llvm-project abc) src/__support/math/pow, pow.\n// Apache-2.0 WITH LLVM-exception; see LICENSE-llvm.\n";
    assert_eq!(
        family_of(llvm).unwrap().map(|f| f.licence),
        Some("LICENSE-llvm")
    );
    assert!(family_of("// Port of CORE-MATH (abc) exp.c, cr_exp.\n// MIT.\n").is_err());
    assert!(
        family_of("// Port of CORE-MATH (abc) exp.c, cr_exp\n// MIT; see LICENSE-core-math.\n")
            .is_err()
    );
    assert!(family_of("// Port of glibc (x) e_exp.c.\n// LGPL\n").is_err());
    assert!(family_of("//! A module.\n").unwrap().is_none());
    assert!(
        family_of(&format!("// Port of Blink (Chromium {TAG}) a.cc.\n// x\n"))
            .unwrap()
            .is_none()
    );
}

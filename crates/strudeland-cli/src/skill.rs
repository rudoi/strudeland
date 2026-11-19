//! The Claude Code skill embedded in the binary, and how `skill install` writes it.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The skill text, exactly as shipped.
pub const SKILL: &str = include_str!("../skill/SKILL.md");

/// What `install` did to the target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    Updated,
    Unchanged,
}

impl Outcome {
    /// The word shared by the human and JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Installed => "installed",
            Outcome::Updated => "updated",
            Outcome::Unchanged => "unchanged",
        }
    }
}

/// Where the skill goes below a Claude config folder (`~/.claude`, or a project's `.claude`).
pub const REL_PATH: &str = "skills/strudeland/SKILL.md";

/// The user's Claude Code config folder: `CLAUDE_CONFIG_DIR` if set, else `<home>/.claude`.
/// Empty values count as unset.
pub fn claude_dir(config_dir: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty()).map(PathBuf::from);
    set(config_dir).or_else(|| set(home).map(|h| h.join(".claude")))
}

/// Writes [`SKILL`] to `path` unless it is already there, via a temp file and a rename. A
/// symlinked `path` is written through, so the link survives.
pub fn install(path: &Path) -> io::Result<Outcome> {
    let resolved;
    let path = if path.is_symlink() {
        resolved = std::fs::canonicalize(path)?;
        resolved.as_path()
    } else {
        path
    };
    let outcome = match std::fs::read(path) {
        Ok(existing) if existing == SKILL.as_bytes() => return Ok(Outcome::Unchanged),
        Ok(_) => Outcome::Updated,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Outcome::Installed,
        Err(e) => return Err(e),
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // Per-process name, so two installs racing on the same folder don't share a temp file.
    let tmp = dir.join(format!(".SKILL.md.tmp-{}", std::process::id()));
    std::fs::write(&tmp, SKILL)
        .and_then(|()| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `key: value` pairs of the leading `---` block, or `None` if there is none.
    fn frontmatter(text: &str) -> Option<Vec<(String, String)>> {
        let rest = text.strip_prefix("---\n")?;
        let (block, _) = rest.split_once("\n---")?;
        block
            .lines()
            .map(|l| {
                let (k, v) = l.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect()
    }

    #[test]
    fn install_fresh_unchanged_updated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/SKILL.md");
        assert_eq!(install(&path).unwrap(), Outcome::Installed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
        assert_eq!(install(&path).unwrap(), Outcome::Unchanged);
        std::fs::write(&path, "old").unwrap();
        assert_eq!(install(&path).unwrap(), Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "no temp file left behind");
    }

    #[test]
    fn install_replaces_a_non_utf8_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert_eq!(install(&path).unwrap(), Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL);
    }

    #[test]
    #[cfg(unix)]
    fn install_writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles.md");
        std::fs::write(&real, "old").unwrap();
        let link = dir.path().join("SKILL.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(install(&link).unwrap(), Outcome::Updated);
        assert!(link.is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), SKILL);
        assert_eq!(install(&link).unwrap(), Outcome::Unchanged);
    }

    #[test]
    fn claude_dir_prefers_the_config_dir_env() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(claude_dir(None, os("/h")), Some("/h/.claude".into()));
        assert_eq!(claude_dir(os("/c"), os("/h")), Some("/c".into()));
        assert_eq!(claude_dir(os(""), os("/h")), Some("/h/.claude".into()));
        assert_eq!(claude_dir(None, os("")), None);
        assert_eq!(claude_dir(None, None), None);
    }

    #[test]
    fn install_reports_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("f");
        std::fs::write(&blocker, "x").unwrap();
        assert!(install(&blocker.join("SKILL.md")).is_err());
    }

    #[test]
    fn frontmatter_parses_the_shipped_skill() {
        let fm = frontmatter(SKILL).unwrap();
        let get = |k: &str| fm.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("name"), Some("strudeland"));
        assert!(get("description").is_some_and(|d| !d.is_empty() && d.contains("project.json")));
        assert!(get("allowed-tools").is_some_and(|t| t.contains("Bash(strudeland:*)")));
    }

    /// Every subcommand path (`code get`) and long flag (`--expect-file`) of `cmd`, below `path`.
    fn names(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
        for arg in cmd.get_arguments() {
            if let Some(long) = arg.get_long()
                && long != "help"
                && long != "version"
            {
                out.push(format!("--{long}"));
            }
        }
        for sub in cmd.get_subcommands() {
            let p = if path.is_empty() {
                sub.get_name().to_string()
            } else {
                format!("{path} {}", sub.get_name())
            };
            out.push(p.clone());
            names(sub, &p, out);
        }
    }

    #[test]
    fn skill_mentions_every_command_and_flag() {
        use clap::CommandFactory;
        let mut all = Vec::new();
        names(&crate::cli::Cli::command(), "", &mut all);
        assert!(all.iter().any(|n| n == "code set"), "{all:?}");
        assert!(all.iter().any(|n| n == "--expect-file"), "{all:?}");
        // Subcommands as `strudeland <path>`; flags as whole words, so `--eval` is not satisfied by
        // `--evaluate` and `stop` not by prose.
        let mentioned = |n: &str| {
            if n.starts_with("--") {
                SKILL.match_indices(n).any(|(i, _)| {
                    let next = SKILL[i + n.len()..].chars().next();
                    !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
                })
            } else {
                SKILL.contains(&format!("strudeland {n}"))
            }
        };
        let missing: Vec<_> = all.iter().filter(|n| !mentioned(n)).collect();
        assert!(missing.is_empty(), "SKILL.md does not mention {missing:?}");
        assert!(SKILL.contains("STRUDELAND_SOCKET"));
    }

    #[test]
    fn skill_does_not_mention_the_headless_host() {
        // The headless host is a separate binary, not a `strudeland` subcommand.
        assert!(!SKILL.contains("strudeland-host"));
    }

    #[test]
    fn skill_does_not_mention_the_dev_binaries() {
        assert!(!SKILL.contains("strudeland-render"));
    }

    #[test]
    fn skill_says_what_render_reads_and_refuses() {
        for needle in [
            "on disk",
            "first at bar N",
            "-23 LUFS",
            "-1 dBTP",
            "strudeland render",
        ] {
            assert!(SKILL.contains(needle), "SKILL.md lacks {needle:?}");
        }
    }

    #[test]
    fn frontmatter_rejects_missing_or_malformed() {
        assert_eq!(frontmatter("no block"), None);
        assert_eq!(frontmatter("---\nname: x"), None);
        assert_eq!(frontmatter("---\nnot a pair\n---\n"), None);
        assert_eq!(
            frontmatter("---\na: b: c\n---\nbody"),
            Some(vec![("a".into(), "b: c".into())])
        );
    }
}

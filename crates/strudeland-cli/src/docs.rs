//! The embedded Strudel function docs, plus the app's own `song` and `meta` entries.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DocsFile {
    pub schema: u32,
    pub source: DocsSource,
    pub entries: Vec<DocEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DocsSource {
    /// Upstream Strudel repository URL.
    pub repo: String,
    /// The vendored upstream commit the docs were extracted from.
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEntry {
    pub name: String,
    pub synonyms: Vec<String>,
    pub description: String,
    pub params: Vec<DocParam>,
    pub examples: Vec<String>,
    pub superdirt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocParam {
    pub name: String,
    pub types: Vec<String>,
    pub description: String,
}

const DOCS_JSON: &str = include_str!("../strudel-docs.json");

/// The extracted docs file, parsed once.
fn docs_file() -> &'static DocsFile {
    static FILE: OnceLock<DocsFile> = OnceLock::new();
    FILE.get_or_init(|| serde_json::from_str(DOCS_JSON).expect("embedded strudel-docs.json parses"))
}

/// All entries (extracted plus app), sorted by name.
pub fn entries() -> &'static [DocEntry] {
    static ALL: OnceLock<Vec<DocEntry>> = OnceLock::new();
    ALL.get_or_init(|| merge(docs_file().entries.clone(), app_entries()))
}

fn param(name: &str, types: &[&str], description: &str) -> DocParam {
    DocParam {
        name: name.into(),
        types: types.iter().map(|t| (*t).into()).collect(),
        description: description.into(),
    }
}

/// Entries for what the app adds to Strudel's scope.
fn app_entries() -> Vec<DocEntry> {
    vec![
        DocEntry {
            name: "song".into(),
            synonyms: vec![],
            description: "Play a song's sections in order and loop the loop section. Put it on a `$:` line. Tempo comes from meta.bpm and meta.meter; one cycle is one bar.".into(),
            params: vec![
                param("meta", &["object"], "the song header: the `meta` constant"),
                param(
                    "parts",
                    &["object"],
                    "one pattern per section name from meta.sections",
                ),
            ],
            examples: vec![
                "$: song(meta, {\n  intro: note(\"c3 e3 g3 c4\"),\n  body: note(\"<c2 g1>*4\").s(\"sawtooth\"),\n})".into(),
            ],
            superdirt: false,
        },
        DocEntry {
            name: "meta".into(),
            synonyms: vec![],
            description: "The song header, a plain `const meta = { ... }` object literal (never `export const`). Fields: title, bpm (quarter notes per minute), meter ('N/D' as a single-quoted string; one cycle is one bar), seed (optional integer from -2147483648 to 2147483647 that fixes the song's randomness; a .seed() in the code wins), sections (a list of { name, bars, loop? } with at most one loop section), loops (optional passes of the loop section when rendered, default 1). Strings the app reads must be single-quoted.".into(),
            params: vec![],
            examples: vec![
                "const meta = {\n  title: 'main',\n  bpm: 120,\n  meter: '4/4',\n  sections: [\n    { name: 'intro', bars: 2 },\n    { name: 'body', bars: 4, loop: true },\n  ],\n  loops: 2,\n}".into(),
            ],
            superdirt: false,
        },
    ]
}

/// Merges app entries into the extracted ones (an extracted entry of the same name wins) and sorts
/// by name.
fn merge(extracted: Vec<DocEntry>, app: Vec<DocEntry>) -> Vec<DocEntry> {
    let mut all = extracted;
    for a in app {
        if !all.iter().any(|e| e.name == a.name) {
            all.push(a);
        }
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// Exact name, then exact synonym.
pub fn lookup<'a>(entries: &'a [DocEntry], name: &str) -> Option<&'a DocEntry> {
    entries.iter().find(|e| e.name == name).or_else(|| {
        entries
            .iter()
            .find(|e| e.synonyms.iter().any(|s| s == name))
    })
}

fn matches_name(e: &DocEntry, term: &str, test: impl Fn(&str, &str) -> bool) -> bool {
    test(&e.name.to_lowercase(), term) || e.synonyms.iter().any(|s| test(&s.to_lowercase(), term))
}

/// Up to 5 entry names for a miss: those with a name or synonym starting with `term`, then those
/// containing it (case-insensitive, alphabetical within each group).
pub fn suggest(entries: &[DocEntry], term: &str) -> Vec<String> {
    let term = term.to_lowercase();
    if term.is_empty() {
        return vec![];
    }
    let starts = |h: &str, t: &str| h.starts_with(t);
    let contains = |h: &str, t: &str| h.contains(t);
    let mut out: Vec<String> = Vec::new();
    for test in [&starts as &dyn Fn(&str, &str) -> bool, &contains] {
        let mut group: Vec<&str> = entries
            .iter()
            .filter(|e| matches_name(e, &term, test))
            .map(|e| e.name.as_str())
            .filter(|n| !out.iter().any(|o| o == n))
            .collect();
        group.sort_unstable();
        out.extend(group.into_iter().map(String::from));
    }
    out.truncate(5);
    out
}

/// Entries whose name or synonym contains `term` (alphabetical), then those matching only in the
/// description (alphabetical). Case-insensitive.
pub fn search<'a>(entries: &'a [DocEntry], term: &str) -> Vec<&'a DocEntry> {
    let term = term.to_lowercase();
    let contains = |h: &str, t: &str| h.contains(t);
    let mut named: Vec<&DocEntry> = entries
        .iter()
        .filter(|e| matches_name(e, &term, contains))
        .collect();
    let mut described: Vec<&DocEntry> = entries
        .iter()
        .filter(|e| {
            !matches_name(e, &term, contains) && e.description.to_lowercase().contains(&term)
        })
        .collect();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    described.sort_by(|a, b| a.name.cmp(&b.name));
    named.extend(described);
    named
}

/// All names, alphabetical.
pub fn list(entries: &[DocEntry]) -> Vec<&str> {
    let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, synonyms: &[&str], description: &str) -> DocEntry {
        DocEntry {
            name: name.into(),
            synonyms: synonyms.iter().map(|s| (*s).into()).collect(),
            description: description.into(),
            params: vec![],
            examples: vec![],
            superdirt: false,
        }
    }

    fn names(v: &[&DocEntry]) -> Vec<String> {
        v.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn embedded_file_matches_the_pin() {
        let pins: serde_json::Value =
            serde_json::from_str(include_str!("../../../strudel-pins.json")).unwrap();
        let file = docs_file();
        assert_eq!(file.schema, 2);
        assert_eq!(
            file.source.commit,
            pins["strudel"]["commit"].as_str().unwrap()
        );
        assert_eq!(file.source.repo, pins["strudel"]["repo"].as_str().unwrap());
        assert!(file.entries.len() > 100);
    }

    #[test]
    fn exact_and_synonym_hits() {
        let all = entries();
        assert_eq!(lookup(all, "fast").unwrap().name, "fast");
        assert_eq!(lookup(all, "sound").unwrap().name, "s");
        assert_eq!(lookup(all, "cutoff").unwrap().name, "lpf");
        assert!(lookup(all, "Fast").is_none());
        assert!(lookup(all, "nope").is_none());
    }

    #[test]
    fn exact_name_beats_a_synonym() {
        let all = vec![e("a", &["b"], ""), e("b", &[], "")];
        assert_eq!(lookup(&all, "b").unwrap().name, "b");
    }

    #[test]
    fn app_entries_are_merged_and_sorted() {
        let all = entries();
        assert!(lookup(all, "song").is_some());
        assert!(lookup(all, "meta").is_some());
        assert!(all.windows(2).all(|w| w[0].name <= w[1].name));
    }

    #[test]
    fn extracted_entry_overrides_app_entry() {
        let merged = merge(
            vec![e("song", &[], "extracted"), e("z", &[], "")],
            vec![e("song", &[], "app"), e("meta", &[], "app")],
        );
        let by: Vec<_> = merged.iter().map(|x| (&*x.name, &*x.description)).collect();
        assert_eq!(by, vec![("meta", "app"), ("song", "extracted"), ("z", "")]);
    }

    #[test]
    fn suggestions_prefix_before_contains_max_five() {
        let all = vec![
            e("fastGap", &[], ""),
            e("fast", &["density"], ""),
            e("slowfast", &[], ""),
            e("fastcat", &[], ""),
            e("fa1", &[], ""),
            e("fa2", &[], ""),
            e("fa3", &[], ""),
            e("unrelated", &[], ""),
        ];
        assert_eq!(
            suggest(&all, "FAST"),
            vec!["fast", "fastGap", "fastcat", "slowfast"]
        );
        assert_eq!(suggest(&all, "fa").len(), 5);
        assert_eq!(
            suggest(&all, "fa"),
            vec!["fa1", "fa2", "fa3", "fast", "fastGap"]
        );
    }

    #[test]
    fn suggestions_dedupe_and_use_synonyms() {
        let all = vec![e("fast", &["fastish", "xfast"], ""), e("other", &[], "")];
        assert_eq!(suggest(&all, "fas"), vec!["fast"]);
        assert_eq!(suggest(&all, "DENS"), Vec::<String>::new());
        let all = vec![e("fast", &["density"], "")];
        assert_eq!(suggest(&all, "dens"), vec!["fast"]);
    }

    #[test]
    fn fats_has_no_suggestions() {
        assert!(suggest(entries(), "fats").is_empty());
        assert!(suggest(entries(), "").is_empty());
    }

    #[test]
    fn search_orders_name_matches_first() {
        let all = vec![
            e("aaa", &[], "about a filter"),
            e("lpf", &["cutoff"], "low pass"),
            e("filter", &[], "x"),
            e("bbb", &[], "FILTER again"),
            e("hpf", &["highpassfilter"], ""),
        ];
        assert_eq!(
            names(&search(&all, "Filter")),
            vec!["filter", "hpf", "aaa", "bbb"]
        );
        assert!(search(&all, "zzz").is_empty());
    }

    #[test]
    fn list_is_alphabetical() {
        let all = vec![e("b", &[], ""), e("a", &[], ""), e("C", &[], "")];
        assert_eq!(list(&all), vec!["C", "a", "b"]);
    }
}

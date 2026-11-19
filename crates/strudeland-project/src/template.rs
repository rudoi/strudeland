//! File templates written by [`crate::Project::create`] and [`crate::Project::create_song`].

/// The song written to `songs/main.js` when a project is created.
pub const MAIN_SONG: &str = r#"const meta = {
  title: 'main',
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
};

// strudeland demo: synth-only, no samples, works offline.
// Cmd-Enter evaluates and plays. Cmd-. stops.
// The tempo comes from meta. song() plays the sections in order and loops the one marked loop.

$: s("sine*4").note("c2").penv(24).pdecay(0.05).decay(0.25).sustain(0).gain(0.9) // kick

$: s("white*8").decay(0.04).sustain(0).hpf(7000).gain(0.3).pan(sine.range(0.4, 0.6)) // hat

$: song(meta, {
  intro: note("<[c3,eb3,g3] [ab2,c3,eb3]>").s("supersaw").attack(0.1).release(0.8).room(0.5).gain(0.25), // pad
  body: stack(
    note("<c2 c2 eb2 g1>*4").s("sawtooth").lpf(slider(600, 100, 2000)).decay(0.2).sustain(0.2).gain(0.5), // bass
    note("<[c3,eb3,g3] [ab2,c3,eb3] [bb2,d3,f3] [g2,b2,d3]>").s("supersaw").attack(0.1).release(0.8).room(0.5).gain(0.25), // pad
  ),
  tail: note("[c3,eb3,g3]").s("supersaw").attack(0.05).release(1.5).room(0.6).gain(0.25), // last chord
})
"#;

/// The contents of a new song named `stem`.
///
/// `stem` is inserted verbatim. [`crate::Project::create_song`] only accepts stems from
/// `[A-Za-z0-9 _-]`, which contain no quote or backslash, so no escaping is needed.
pub fn new_song(stem: &str) -> String {
    format!(
        "const meta = {{\n  title: '{stem}',\n  bpm: 120,\n}};\n\n$: s(\"sine*4\").note(\"c2\")\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal copy of the header rule: the first line starts `const meta = {` and the header
    /// runs to the matching close brace. Returns the text between the braces.
    fn header(code: &str) -> Option<&str> {
        let prefix = "const meta = {";
        if !code.starts_with(prefix) {
            return None;
        }
        let open = prefix.len() - 1;
        let mut depth = 0usize;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&code[open + 1..open + i]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Finds `key: value` in a flat header body and returns the trimmed value text.
    fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
        body.split(',').find_map(|part| {
            let (k, v) = part.split_once(':')?;
            (k.trim() == key).then(|| v.trim())
        })
    }

    #[test]
    fn main_song_has_a_sectioned_header() {
        let body = header(MAIN_SONG).expect("header");
        assert!(body.contains("title: 'main'"));
        assert!(body.contains("bpm: 120"));
        assert!(body.contains("meter: '4/4'"));
        assert!(body.contains("{ name: 'intro', bars: 2 }"));
        assert!(body.contains("{ name: 'body', bars: 4, loop: true }"));
        assert!(body.contains("{ name: 'tail', bars: 2 }"));
    }

    #[test]
    fn main_song_builds_on_song_and_plain_lines() {
        assert!(MAIN_SONG.ends_with('\n'));
        assert!(!MAIN_SONG.ends_with("\n\n"));
        assert_eq!(MAIN_SONG.lines().filter(|l| l.starts_with("$:")).count(), 3);
        assert!(MAIN_SONG.contains("$: song(meta, {"));
        assert!(!MAIN_SONG.contains("setcpm"));
        assert!(MAIN_SONG.contains("// strudeland demo: synth-only, no samples, works offline."));
    }

    #[test]
    fn new_song_has_a_header_with_the_stem_as_title() {
        let code = new_song("x y");
        let body = header(&code).expect("header");
        assert_eq!(field(body, "title"), Some("'x y'"));
        assert_eq!(field(body, "bpm"), Some("120"));
        assert!(!code.contains("setcpm"));
        assert!(code.contains("$: s(\"sine*4\").note(\"c2\")"));
    }

    #[test]
    fn new_song_ends_with_a_single_newline() {
        let code = new_song("a");
        assert!(code.ends_with('\n'));
        assert!(!code.ends_with("\n\n"));
    }

    #[test]
    fn header_extractor_rejects_code_without_a_header() {
        assert_eq!(header("setcpm(30)\n"), None);
        assert_eq!(header("// c\nconst meta = { a: 1 };"), None);
        assert_eq!(header("const meta = { a: {"), None);
    }
}

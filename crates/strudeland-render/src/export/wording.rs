//! The desktop dialog's wording of a length request the song cannot satisfy. The CLI
//! prints the planner's own message instead (it names flags).

use crate::LengthProblem;

/// The dialog text for `problem`: lowercase, flag-free, with the remedy the dialog offers.
pub fn dialog_text(problem: &LengthProblem) -> String {
    match problem {
        LengthProblem::NoSections | LengthProblem::SectionWithoutSections => {
            "this song has no sections; choose a length in bars".into()
        }
        LengthProblem::UnknownSection { name, sections } => format!(
            "this song has no section '{name}' (sections: {}); choose another section",
            sections.join(", ")
        ),
        LengthProblem::LengthPastEnd { bars, clip_bars } => format!(
            "{} is past the end of the song ({}); choose {} or fewer",
            bar_count(*bars),
            bar_count(*clip_bars),
            bar_count(*clip_bars)
        ),
        LengthProblem::ClipPastLimit {
            clip_bars, limit, ..
        } => format!(
            "the song's clip is {clip_bars} bars, past the render limit of {limit} bars; \
             choose a length in bars"
        ),
        LengthProblem::SectionPastLimit {
            name,
            to_bar,
            limit,
        } => {
            format!("section '{name}' ends at bar {to_bar}, past the render limit of {limit} bars")
        }
        LengthProblem::LoopPastLimit { end, limit } => {
            format!("the loop ends at bar {end}, past the render limit of {limit} bars")
        }
        LengthProblem::PresetWithoutLoop => {
            "loop export needs a meta.loop range or a section with loop: true".into()
        }
        LengthProblem::PresetWithLength => "loop exports use the marked loop section".into(),
        LengthProblem::LoopsWithoutLoop => {
            "the loop count needs a meta.loop range or a section with loop: true".into()
        }
        LengthProblem::LoopsPastLimit {
            loops,
            clip_bars,
            limit,
        } => format!(
            "{loops} loop passes make the clip {clip_bars} bars, past the render limit of \
             {limit} bars"
        ),
        LengthProblem::LoopsInvalid { message } => message.clone(),
    }
}

/// `n bars`, or `1 bar`.
fn bar_count(n: u64) -> String {
    if n == 1 {
        "1 bar".into()
    } else {
        format!("{n} bars")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_problem_has_its_dialog_text() {
        let cases = [
            (
                LengthProblem::PresetWithLength,
                "loop exports use the marked loop section",
            ),
            (
                LengthProblem::LoopsWithoutLoop,
                "the loop count needs a meta.loop range or a section with loop: true",
            ),
            (
                LengthProblem::LoopsInvalid {
                    message: "x".into(),
                },
                "x",
            ),
            (
                LengthProblem::NoSections,
                "this song has no sections; choose a length in bars",
            ),
            (
                LengthProblem::LoopsPastLimit {
                    loops: 1000,
                    clip_bars: 10000002,
                    limit: 100000,
                },
                "1000 loop passes make the clip 10000002 bars, past the render limit of 100000 bars",
            ),
            (
                LengthProblem::ClipPastLimit {
                    clip_bars: 110000,
                    limit: 100000,
                    has_loop: true,
                },
                "the song's clip is 110000 bars, past the render limit of 100000 bars; choose a length in bars",
            ),
            (
                LengthProblem::SectionWithoutSections,
                "this song has no sections; choose a length in bars",
            ),
            (
                LengthProblem::UnknownSection {
                    name: "drop".into(),
                    sections: vec!["intro".into(), "body".into(), "tail".into()],
                },
                "this song has no section 'drop' (sections: intro, body, tail); choose another section",
            ),
            (
                LengthProblem::SectionPastLimit {
                    name: "s10".into(),
                    to_bar: 110000,
                    limit: 100000,
                },
                "section 's10' ends at bar 110000, past the render limit of 100000 bars",
            ),
            (
                LengthProblem::LengthPastEnd {
                    bars: 9,
                    clip_bars: 8,
                },
                "9 bars is past the end of the song (8 bars); choose 8 bars or fewer",
            ),
            (
                LengthProblem::LengthPastEnd {
                    bars: 2,
                    clip_bars: 1,
                },
                "2 bars is past the end of the song (1 bar); choose 1 bar or fewer",
            ),
            (
                LengthProblem::PresetWithoutLoop,
                "loop export needs a meta.loop range or a section with loop: true",
            ),
            (
                LengthProblem::LoopPastLimit {
                    end: 110000,
                    limit: 100000,
                },
                "the loop ends at bar 110000, past the render limit of 100000 bars",
            ),
        ];
        for (problem, want) in &cases {
            let text = dialog_text(problem);
            assert_eq!(text, *want, "{problem:?}");
            assert!(!text.contains("--"), "{problem:?}: {text}");
            let first = text.chars().next().unwrap();
            if !matches!(problem, LengthProblem::LoopsInvalid { .. }) {
                assert!(!first.is_uppercase(), "{problem:?}: {text}");
            }
        }
    }
}

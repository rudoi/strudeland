//! Planning through the headless host, and the refusal of material the engine cannot play.

use std::collections::BTreeMap;
use std::time::Instant;

use strudeland_engine::{EngineJob, UnsupportedSignature, unsupported_signatures};
use strudeland_host::{ExportPreset, Host, PlannedRender, PlannedSection, RenderPlanRequest};

use crate::{RenderError, RenderOptions, Unsupported};

/// Plans every chunk of the render in one host call, on a host that is dropped (with its
/// isolate's memory) before this returns. Returns the plan and the milliseconds it took.
pub(crate) fn plan(
    code: &str,
    opts: &RenderOptions,
    preset: Option<ExportPreset>,
) -> Result<(PlannedRender, f64), RenderError> {
    let started = Instant::now();
    let request = RenderPlanRequest {
        instruments: opts.instruments.clone(),
        length: opts.length.clone(),
        preset,
        include_events: opts.include_events.then_some(true),
        chunk: None,
        timeout_sec: opts.plan_timeout.map(|d| d.as_secs_f64()),
    };
    let planned = {
        let host = Host::new()?;
        host.plan_render(code, &request)?
    };
    Ok((planned, started.elapsed().as_secs_f64() * 1000.0))
}

/// Takes each chunk's job text and decodes it for the workers, releasing the text as it goes.
/// Refuses unsupported signatures before either pass renders.
pub(crate) fn take_jobs(planned: &mut PlannedRender) -> Result<Vec<EngineJob>, RenderError> {
    let repeat = planned
        .export
        .as_mut()
        .and_then(|export| export.repeat.as_mut());
    let mut jobs =
        Vec::with_capacity(planned.chunks.len() + repeat.as_ref().map_or(0, |p| p.chunks.len()));
    let mut signatures = Vec::new();
    for (i, chunk) in planned
        .chunks
        .iter_mut()
        .chain(repeat.into_iter().flat_map(|p| &mut p.chunks))
        .enumerate()
    {
        let job = EngineJob::from_json(&std::mem::take(&mut chunk.json))
            .map_err(|e| RenderError::Render(format!("chunk {i}: job: {e}")))?;
        signatures.extend(unsupported_signatures(&job));
        jobs.push(job);
    }
    refuse(&planned.sections, signatures)?;
    Ok(jobs)
}

/// `Err(Unsupported)` when there is any signature: deduplicated by `(s, keys)` with the minimum
/// `first_bar`, each named with the section the plan places its bar in
/// (`PlannedRender::sections`), sorted by bar, then sound.
pub(crate) fn refuse(
    sections: &[PlannedSection],
    signatures: impl IntoIterator<Item = UnsupportedSignature>,
) -> Result<(), RenderError> {
    let mut first: BTreeMap<(String, Vec<String>), i64> = BTreeMap::new();
    for sig in signatures {
        first
            .entry((sig.s, sig.keys))
            .and_modify(|bar| *bar = (*bar).min(sig.first_bar))
            .or_insert(sig.first_bar);
    }
    if first.is_empty() {
        return Ok(());
    }
    let mut list: Vec<Unsupported> = first
        .into_iter()
        .map(|((s, keys), layout_bar)| Unsupported {
            section: section_name(sections, layout_bar).map(str::to_string),
            s,
            keys,
            layout_bar,
        })
        .collect();
    list.sort();
    Err(RenderError::Unsupported(list))
}

/// The section of the run in `sections` (sorted, disjoint) that contains layout bar `bar`;
/// `None` for a bar in no run, a negative bar included.
fn section_name(sections: &[PlannedSection], bar: i64) -> Option<&str> {
    let i = sections.partition_point(|s| i64::from(s.to_bar) <= bar);
    sections
        .get(i)
        .filter(|s| i64::from(s.from_bar) <= bar)
        .map(|s| s.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(s: &str, keys: &[&str], first_bar: i64) -> UnsupportedSignature {
        UnsupportedSignature {
            s: s.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            first_bar,
        }
    }

    fn run(from_bar: u32, to_bar: u32, name: &str) -> PlannedSection {
        PlannedSection {
            from_bar,
            to_bar,
            name: name.into(),
        }
    }

    fn refused(sections: &[PlannedSection], sigs: Vec<UnsupportedSignature>) -> Vec<Unsupported> {
        match refuse(sections, sigs) {
            Err(RenderError::Unsupported(list)) => list,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn section_name_looks_bars_up_in_the_runs() {
        let runs = [run(0, 2, "intro"), run(2, 10, "body"), run(12, 14, "tail")];
        assert_eq!(section_name(&runs, -1), None);
        assert_eq!(section_name(&runs, 0), Some("intro"));
        assert_eq!(section_name(&runs, 1), Some("intro"));
        assert_eq!(section_name(&runs, 2), Some("body"));
        assert_eq!(section_name(&runs, 9), Some("body"));
        assert_eq!(section_name(&runs, 10), None);
        assert_eq!(section_name(&runs, 11), None);
        assert_eq!(section_name(&runs, 12), Some("tail"));
        assert_eq!(section_name(&runs, 13), Some("tail"));
        assert_eq!(section_name(&runs, 14), None);
        assert_eq!(section_name(&runs, i64::MAX), None);
        for bar in [i64::MIN, -1, 0, 1, 13, 14, i64::MAX] {
            assert_eq!(section_name(&[], bar), None, "bar {bar}");
        }
    }

    #[test]
    fn nothing_unsupported_is_not_refused() {
        assert!(refuse(&[], vec![]).is_ok());
    }

    #[test]
    fn signatures_dedupe_by_sound_and_keys_with_the_minimum_bar() {
        // Chunk order is not bar order: a later chunk's lookback can hold an earlier bar.
        let list = refused(
            &[],
            vec![
                sig("kick", &[], 8),
                sig("hat", &["speed"], 4),
                sig("kick", &[], 3),
                sig("hat", &[], 6),
                sig("kick", &[], 5),
                sig("hat", &["speed"], 1),
                sig("clap", &[], 3),
            ],
        );
        let got: Vec<(&str, Vec<String>, i64)> = list
            .iter()
            .map(|u| (u.s.as_str(), u.keys.clone(), u.layout_bar))
            .collect();
        assert_eq!(
            got,
            vec![
                ("hat", vec!["speed".to_string()], 1),
                ("clap", vec![], 3),
                ("kick", vec![], 3),
                ("hat", vec![], 6),
            ]
        );
        assert!(list.iter().all(|u| u.section.is_none()));
        assert_eq!(
            RenderError::Unsupported(list).to_string(),
            "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=hat keys=speed first at bar 2\n  s=clap first at bar 4\n  s=kick first at bar 4\n  s=hat first at bar 7"
        );
    }

    #[test]
    fn the_clip_layout_names_the_clip_section() {
        // `song-linear.js`'s clip runs: intro 2, body (the loop section, twice) 8, tail 2.
        let list = refused(
            &[run(0, 2, "intro"), run(2, 10, "body"), run(10, 12, "tail")],
            vec![
                sig("zzfx", &[], 10),
                sig("kick", &[], 0),
                sig("hat", &[], 7),
            ],
        );
        assert_eq!(
            RenderError::Unsupported(list).to_string(),
            "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=kick first at bar 1 (section 'intro')\n  s=hat first at bar 8 (section 'body')\n  s=zzfx first at bar 11 (section 'tail')"
        );
    }

    #[test]
    fn the_linear_layout_names_the_song_section() {
        // `song-linear.js`'s linear runs (intro 2, body 4, tail 2): the song's linear layout.
        // This tests the lookup and the text; a real `--section body` plan's runs stop at bar 6,
        // so a bar-6 signature cannot come from one.
        let list = refused(
            &[run(0, 2, "intro"), run(2, 6, "body"), run(6, 8, "tail")],
            vec![sig("zzfx", &[], 1), sig("kick", &["speed"], 6)],
        );
        assert_eq!(list[0].section.as_deref(), Some("intro"));
        assert_eq!(list[1].section.as_deref(), Some("tail"));
        assert_eq!(
            RenderError::Unsupported(list).to_string(),
            "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=zzfx first at bar 2 (section 'intro')\n  s=kick keys=speed first at bar 7 (section 'tail')"
        );
    }
}

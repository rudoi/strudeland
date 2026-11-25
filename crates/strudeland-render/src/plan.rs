//! Planning through the headless host, and the refusal of material the engine cannot play.

use std::collections::BTreeMap;
use std::time::Instant;

use strudeland_engine::EngineJob;
use strudeland_engine::controls::{UnsupportedSignature, unsupported_signatures};
use strudeland_host::{
    ExportPreset, Host, Layout, PlannedRender, PlannedWindow, RenderPlanRequest, Structure,
};

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
    refuse(&planned.window, &planned.structure, signatures)?;
    Ok(jobs)
}

/// `Err(Unsupported)` when there is any signature: deduplicated by `(s, keys)` with the minimum
/// `first_bar`, each named with the section its bar plays in, sorted by bar, then sound.
pub(crate) fn refuse(
    window: &PlannedWindow,
    structure: &Structure,
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
            section: section_at(window.layout, structure, layout_bar).map(str::to_string),
            s,
            keys,
            layout_bar,
        })
        .collect();
    list.sort();
    Err(RenderError::Unsupported(list))
}

/// The section playing in layout bar `bar`.
fn section_at(layout: Layout, structure: &Structure, bar: i64) -> Option<&str> {
    match layout {
        Layout::Clip => section_of_clip_bar(structure, bar),
        Layout::Linear => section_of_song_bar(structure, bar),
    }
}

/// The section containing song bar `bar` (the linear timeline); `None` outside every section.
pub(crate) fn section_of_song_bar(structure: &Structure, bar: i64) -> Option<&str> {
    structure
        .sections
        .iter()
        .find(|s| {
            let start = i64::from(s.start);
            bar >= start && bar < start + i64::from(s.bars)
        })
        .map(|s| s.name.as_str())
}

/// The section playing in clip bar `bar`: the intro (bars before `intro_end`) maps linearly, the
/// `loops * loop_bars` bars after it are the loop section, and later bars map to the sections
/// after the loop section. Without a loop section, the linear mapping.
pub(crate) fn section_of_clip_bar(structure: &Structure, bar: i64) -> Option<&str> {
    let (start, end) = if let Some(range) = structure.loop_range {
        (i64::from(range.start), i64::from(range.end))
    } else if let Some(section) = structure.loop_index.and_then(|i| structure.sections.get(i)) {
        let start = i64::from(section.start);
        (start, start + i64::from(section.bars))
    } else {
        return section_of_song_bar(structure, bar);
    };
    let period = end - start;
    if period <= 0 || bar < start {
        return section_of_song_bar(structure, bar);
    }
    let elapsed = bar - start;
    // Divide before comparing to avoid multiplying arbitrary plan lengths.
    let authored = if elapsed / period < i64::from(structure.loops) {
        start + elapsed % period
    } else {
        end.checked_add(elapsed - period * i64::from(structure.loops))?
    };
    section_of_song_bar(structure, authored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strudeland_host::{Meter, Section};

    fn section(name: &str, start: u32, bars: u32, is_loop: bool) -> Section {
        Section {
            name: name.into(),
            start,
            bars,
            is_loop,
        }
    }

    /// `song-linear.js`: intro 2, body 4 (loop), tail 2; `loops: 2`; the clip is 12 bars.
    fn song_linear() -> Structure {
        Structure {
            bpm: 120.0,
            meter: Meter { num: 4, den: 4 },
            beats_per_bar: 4.0,
            cps: 0.5,
            sections: vec![
                section("intro", 0, 2, false),
                section("body", 2, 4, true),
                section("tail", 6, 2, false),
            ],
            loop_index: Some(1),
            loop_range: None,
            meter_changes: None,
            loops: 2,
            total_bars: 8,
            intro_end: 2,
            end_of_song: None,
            clip_bars: 12,
            clip_seconds: 24.0,
        }
    }

    fn no_sections() -> Structure {
        Structure {
            sections: vec![],
            loop_index: None,
            loops: 1,
            total_bars: 0,
            intro_end: 0,
            end_of_song: Some(0),
            clip_bars: 0,
            clip_seconds: 0.0,
            ..song_linear()
        }
    }

    #[test]
    fn clip_bars_map_through_the_loop() {
        let s = song_linear();
        let names: Vec<Option<&str>> = (-1..14).map(|b| section_of_clip_bar(&s, b)).collect();
        assert_eq!(
            names,
            vec![
                None,
                Some("intro"),
                Some("intro"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("tail"),
                Some("tail"),
                None,
                None,
            ]
        );
        // Three loops push the tail to clip bars 14 and 15.
        let three = Structure {
            loops: 3,
            clip_bars: 16,
            ..song_linear()
        };
        assert_eq!(section_of_clip_bar(&three, 13), Some("body"));
        assert_eq!(section_of_clip_bar(&three, 14), Some("tail"));
        assert_eq!(section_of_clip_bar(&three, 15), Some("tail"));
        assert_eq!(section_of_clip_bar(&three, 16), None);
    }

    #[test]
    fn clip_diagnostics_follow_each_section_inside_a_repeated_range() {
        let mut s = song_linear();
        s.loop_range = Some(strudeland_host::LoopRange {
            first: 0,
            last: 1,
            start: 0,
            end: 6,
        });
        assert_eq!(section_of_clip_bar(&s, 6), Some("intro"));
        assert_eq!(section_of_clip_bar(&s, 8), Some("body"));
        assert_eq!(section_of_clip_bar(&s, 12), Some("tail"));
    }

    #[test]
    fn song_bars_map_linearly() {
        let s = song_linear();
        let names: Vec<Option<&str>> = (-1..9).map(|b| section_of_song_bar(&s, b)).collect();
        assert_eq!(
            names,
            vec![
                None,
                Some("intro"),
                Some("intro"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("body"),
                Some("tail"),
                Some("tail"),
                None,
            ]
        );
        // Without a loop section the clip mapping is the linear one.
        let no_loop = Structure {
            sections: vec![section("a", 0, 3, false), section("b", 3, 1, false)],
            loop_index: None,
            loops: 1,
            total_bars: 4,
            intro_end: 4,
            end_of_song: Some(4),
            clip_bars: 4,
            ..song_linear()
        };
        for b in -1..6 {
            assert_eq!(
                section_of_clip_bar(&no_loop, b),
                section_of_song_bar(&no_loop, b),
                "bar {b}"
            );
        }
        assert_eq!(section_of_song_bar(&no_sections(), 0), None);
        assert_eq!(section_of_clip_bar(&no_sections(), 0), None);
    }

    fn sig(s: &str, keys: &[&str], first_bar: i64) -> UnsupportedSignature {
        UnsupportedSignature {
            s: s.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            first_bar,
        }
    }

    fn window(from_bar: u32, to_bar: u32, layout: Layout) -> PlannedWindow {
        PlannedWindow {
            from_bar,
            to_bar,
            layout,
        }
    }

    fn refused(
        window: &PlannedWindow,
        structure: &Structure,
        sigs: Vec<UnsupportedSignature>,
    ) -> Vec<Unsupported> {
        match refuse(window, structure, sigs) {
            Err(RenderError::Unsupported(list)) => list,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn nothing_unsupported_is_not_refused() {
        assert!(refuse(&window(0, 4, Layout::Linear), &no_sections(), vec![]).is_ok());
    }

    #[test]
    fn signatures_dedupe_by_sound_and_keys_with_the_minimum_bar() {
        // Chunk order is not bar order: a later chunk's lookback can hold an earlier bar.
        let list = refused(
            &window(0, 12, Layout::Linear),
            &no_sections(),
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
        let list = refused(
            &window(0, 12, Layout::Clip),
            &song_linear(),
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
        // A `--section body` render ([2, 6) linear) whose lookback reaches into the intro.
        let list = refused(
            &window(2, 6, Layout::Linear),
            &song_linear(),
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

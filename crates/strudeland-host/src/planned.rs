//! The output of [`Host::plan_render`](crate::Host::plan_render): the serde mirror of
//! `PlannedRender` in `src/lib/render/job-plan.ts`, which the bundle writes as one ASCII JSON text
//! in `globalThis.__strudelandRender`. No `deny_unknown_fields`, so a newer bundle field does not
//! break an older host.

use serde::{Deserialize, Serialize};

use crate::Layout;

/// Every chunk of a render, planned in one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedRender {
    /// The resolved window and layout.
    pub window: PlannedWindow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<PlannedExport>,
    pub cps: f64,
    /// Musical content length, excluding the lookback and any export tail.
    pub music_frames: u64,
    /// The song's structure (with a `loops` override applied).
    pub structure: Structure,
    /// The longest lookback across the planned passes, in seconds.
    pub lookback_sec: f64,
    /// Captured and selection errors of every chunk, deduplicated.
    pub errors: Vec<PlanError>,
    /// The opening window's schedule for an export, or the standard schedule over `[0, toBar)`.
    /// A `RenderEvent[]` returned only when the request set `includeEvents`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<serde_json::Value>,
    /// The chunks in order; their windows are contiguous and cover `window`.
    pub chunks: Vec<PlannedChunk>,
}

/// Musical loop boundary and post-roll for the opening or one-shot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedExport {
    /// The loop's first bar; present together with `loop_start_frame`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_start_bar: Option<u32>,
    /// Loop start relative to the opening's first content frame; present together with
    /// `loop_start_bar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_start_frame: Option<u64>,
    pub tail_frames: u64,
    /// The isolated loop plan when the opening also contains intro bars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<PlannedRepeat>,
}

/// An isolated body using the same authored bars as the opening's loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedRepeat {
    /// The body's musical length, excluding the lookback and its tail.
    pub music_frames: u64,
    pub tail_frames: u64,
    pub chunks: Vec<PlannedChunk>,
}

/// The window a length request resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedWindow {
    pub from_bar: u32,
    pub to_bar: u32,
    pub layout: Layout,
}

/// One chunk's engine job text and its window. Standard jobs match the render page's plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedChunk {
    pub json: String,
    pub plan: ChunkWindow,
}

/// A chunk's `WindowPlan` plus its content window and the exact `lookbackSec` its request carries
/// (`lookback_sec_used` is not always equal to it in floating point).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkWindow {
    pub c0: u32,
    pub c1: u32,
    pub lookback_sec_used: f64,
    pub window_start_frame: u64,
    /// Difference of the rounded absolute edges of this chunk's musical content.
    pub content_frames: u64,
    /// Frames after the content, present exactly when the chunk renders overlap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap_frames: Option<u64>,
    pub buffer_start_sec: f64,
    pub frames: u64,
    pub from_bar: u32,
    pub to_bar: u32,
    pub lookback_sec: f64,
}

/// A planner error (`RenderError` in `contract.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanError {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<i64>,
}

/// A song's structure (`SongStructure` in `structure.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    pub bpm: f64,
    pub meter: Meter,
    pub beats_per_bar: f64,
    pub cps: f64,
    pub sections: Vec<Section>,
    pub loop_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_range: Option<LoopRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_changes: Option<Vec<MeterChange>>,
    pub loops: u32,
    pub total_bars: u32,
    pub intro_end: u32,
    pub end_of_song: Option<u32>,
    pub clip_bars: u32,
    pub clip_seconds: f64,
}

/// Inclusive section indices and half-open authored bar range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRange {
    pub first: usize,
    pub last: usize,
    pub start: u32,
    pub end: u32,
}

/// A meter change at a zero-based authored bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterChange {
    pub bar: u32,
    pub meter: Meter,
}

/// A meter (`'6/8'` is `{ num: 6, den: 8 }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meter {
    pub num: u32,
    pub den: u32,
}

/// One section of a song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    /// The song bar where it starts.
    pub start: u32,
    pub bars: u32,
    #[serde(rename = "loop")]
    pub is_loop: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANNED: &str = r#"{"window":{"fromBar":0,"toBar":12,"layout":"clip"},"cps":0.5,"structure":{"bpm":120,"meter":{"num":4,"den":4},"beatsPerBar":4,"cps":0.5,"sections":[{"name":"a","start":0,"bars":4,"loop":false},{"name":"b","start":4,"bars":4,"loop":true}],"loopIndex":1,"loops":2,"totalBars":8,"introEnd":4,"endOfSong":null,"clipBars":12,"clipSeconds":24},"lookbackSec":0.30000000000000004,"errors":[{"kind":"hap","message":"m","bar":3},{"kind":"other","message":"n"}],"musicFrames":1058400,"chunks":[{"json":"{\"schema\":2}","plan":{"c0":0,"c1":12,"lookbackSecUsed":0,"windowStartFrame":0,"contentFrames":1058400,"bufferStartSec":0,"frames":1058400,"fromBar":0,"toBar":12,"lookbackSec":0.30000000000000004}}],"future":1}"#;

    #[test]
    fn the_bundle_shape_parses_exactly() {
        let p: PlannedRender = serde_json::from_str(PLANNED).unwrap();
        assert_eq!(
            p.window,
            PlannedWindow {
                from_bar: 0,
                to_bar: 12,
                layout: Layout::Clip
            }
        );
        assert_eq!(
            p.lookback_sec.to_bits(),
            0.300_000_000_000_000_04_f64.to_bits()
        );
        assert!(p.structure.sections[1].is_loop);
        assert_eq!(p.structure.loop_index, Some(1));
        assert_eq!(p.structure.end_of_song, None);
        assert_eq!(p.structure.meter, Meter { num: 4, den: 4 });
        assert_eq!(p.errors[0].bar, Some(3));
        assert_eq!(p.errors[1].bar, None);
        assert_eq!(p.events, None);
        assert_eq!(p.chunks[0].json, r#"{"schema":2}"#);
        assert_eq!(p.chunks[0].plan.frames, 1_058_400);
        assert_eq!(p.music_frames, 1_058_400);
        assert_eq!(p.chunks[0].plan.content_frames, 1_058_400);
        assert_eq!(p.chunks[0].plan.overlap_frames, None);
        // Re-serialised: camelCase, `loop`, no `events` and no `bar` when absent, unknown
        // bundle fields dropped.
        let text = serde_json::to_string(&p).unwrap();
        assert!(
            text.starts_with(r#"{"window":{"fromBar":0,"toBar":12,"layout":"clip"},"cps":0.5,"#)
        );
        assert!(text.contains(r#""loop":true"#), "{text}");
        assert!(text.contains(r#"{"kind":"other","message":"n"}"#), "{text}");
        assert!(
            !text.contains("events") && !text.contains("future"),
            "{text}"
        );
        assert!(text.contains(r#""musicFrames":1058400"#), "{text}");
        assert!(text.contains(r#""contentFrames":1058400"#), "{text}");
        assert!(!text.contains("overlapFrames"), "{text}");
        assert_eq!(serde_json::from_str::<PlannedRender>(&text).unwrap(), p);
    }

    #[test]
    fn events_round_trip_when_present() {
        let with = PLANNED.replace(
            r#""chunks":"#,
            r#""events":[{"s":"sine","onset":0}],"chunks":"#,
        );
        let p: PlannedRender = serde_json::from_str(&with).unwrap();
        assert_eq!(
            p.events,
            Some(serde_json::json!([{ "s": "sine", "onset": 0 }]))
        );
        assert!(
            serde_json::to_string(&p)
                .unwrap()
                .contains(r#""events":[{"#)
        );
    }

    #[test]
    fn overlap_frames_round_trip_when_present() {
        let with = PLANNED.replace(
            r#""contentFrames":1058400,"#,
            r#""contentFrames":1058400,"overlapFrames":88200,"#,
        );
        let p: PlannedRender = serde_json::from_str(&with).unwrap();
        assert_eq!(p.chunks[0].plan.overlap_frames, Some(88_200));
        assert!(
            serde_json::to_string(&p)
                .unwrap()
                .contains(r#""overlapFrames":88200"#)
        );
    }

    #[test]
    fn a_wrong_shape_is_rejected() {
        for (from, to) in [
            (r#""frames":1058400"#, r#""frames":-1"#),
            (r#""frames":1058400"#, r#""frames":1.5"#),
            (r#""cps":0.5,"structure""#, r#""cps":"0.5","structure""#),
            (r#""layout":"clip""#, r#""layout":"song""#),
            (r#""chunks":["#, r#""chunkz":["#),
            (r#""musicFrames":1058400,"#, ""),
            (r#""contentFrames":1058400,"#, ""),
        ] {
            let text = PLANNED.replacen(from, to, 1);
            assert_ne!(text, PLANNED, "{from}");
            assert!(
                serde_json::from_str::<PlannedRender>(&text).is_err(),
                "{to}"
            );
        }
    }
}

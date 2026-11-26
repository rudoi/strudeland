//! A take's sidecar and its publish: the webview's fields or the timing-null fallback, written
//! through `strudeland-project`.

use std::sync::Arc;

use strudeland_project::{
    HostNs, Project, ProjectError, RecordingEntry, SIDECAR_SCHEMA, Sidecar, SidecarApp,
    SidecarClockOffsets, SidecarInput, SidecarOutput, SidecarTiming,
};

use super::{EndedTake, TakeError, TakeStart, TakeStop};
use crate::{Anchor, ArmedInfo, TakeResult, frame_at};

/// What a take needs to write its sidecar, from the start on.
pub(super) struct TakeMeta {
    pub(super) project: Arc<Project>,
    pub(super) stem: String,
    pub(super) start: TakeStart,
    pub(super) input: ArmedInfo,
    pub(super) created_at: String,
    /// The sidecar's `app.version`, from [`super::TakesConfig`].
    pub(super) app_version: String,
}

/// The fractional sample index of bar 1's downbeat: `frame_at` over the take's anchors, `None`
/// when there is no bar-1 time, no anchor, or the result falls outside `[0, frames]` (the clocks
/// disagree; logged).
pub fn bar1_frame(
    anchors: &[Anchor],
    sample_rate: u32,
    frames: u64,
    bar1_host_ns: Option<u64>,
) -> Option<f64> {
    let host_ns = bar1_host_ns?;
    let frame = frame_at(anchors, host_ns, sample_rate)?;
    if (0.0..=frames as f64).contains(&frame) {
        Some(frame)
    } else {
        eprintln!("recording: bar 1 at frame {frame} is outside the take's {frames} frames");
        None
    }
}

/// The sidecar of a take: the webview's `fields`, or with `None` the fallback (start fields,
/// null timing; any part that would fail validation replaced by [`Sidecar::sanitize`], so the
/// fallback always validates).
fn build_sidecar(meta: &TakeMeta, result: &TakeResult, fields: Option<&TakeStop>) -> Sidecar {
    let input = &meta.input;
    let mut sidecar = Sidecar {
        schema: SIDECAR_SCHEMA,
        created_at: meta.created_at.clone(),
        app: SidecarApp {
            version: meta.app_version.clone(),
        },
        input: SidecarInput {
            device_id: input.device_id.clone(),
            device_name: input.device_name.clone(),
            channel: u32::from(input.channel),
            channels: u32::from(input.channels),
            sample_rate: input.sample_rate,
        },
        output: SidecarOutput {
            base_latency: 0.0,
            output_latency: None,
        },
        tempo: meta.start.tempo.clone(),
        backing: meta.start.backing.clone(),
        timing: SidecarTiming {
            bar1_frame: None,
            bar1_host_ns: None,
            clock_offset_ns: SidecarClockOffsets {
                start: None,
                end: None,
            },
            anchors: result
                .anchors
                .iter()
                .map(|a| (a.frame, HostNs(a.host_ns)))
                .collect(),
            output_stamps: Vec::new(),
        },
        xruns: result.xruns,
        recovered: false,
    };
    if let Some(f) = fields {
        let host_ns = f.bar1_host_ns.map(|h| h.0);
        sidecar.output = f.output.clone();
        sidecar.tempo = f.tempo.clone();
        sidecar.backing = f.backing.clone();
        sidecar.timing.bar1_frame =
            bar1_frame(&result.anchors, input.sample_rate, result.frames, host_ns);
        sidecar.timing.bar1_host_ns = f.bar1_host_ns;
        sidecar.timing.clock_offset_ns = f.clock_offset_ns.clone();
        sidecar.timing.output_stamps = f.output_stamps.clone();
    } else if sidecar.sanitize() {
        // The start fields were bad (a `setcps(0)` tempo, say): the audio is kept anyway.
        eprintln!(
            "recording: {} had invalid start fields; the fallback sidecar replaces them",
            meta.stem
        );
    }
    sidecar
}

/// A memo file error as the take's error.
pub(super) fn memo(e: ProjectError) -> TakeError {
    TakeError::Memo(Arc::new(e))
}

/// What [`write_sidecar_or_fallback`] does when the webview's fields fail validation.
#[derive(Clone, Copy)]
enum OnInvalid {
    /// Write the fallback sidecar instead.
    WriteFallback,
    /// Keep the sidecar already on disk.
    KeepExisting,
}

/// Writes the take's sidecar (from `fields`, or the fallback with `None`) with `write`. Webview
/// fields that fail validation are logged and handled as `on_invalid` says, so the audio is
/// kept.
fn write_sidecar_or_fallback(
    meta: &TakeMeta,
    result: &TakeResult,
    fields: Option<&TakeStop>,
    on_invalid: OnInvalid,
    write: impl Fn(&Sidecar) -> Result<(), ProjectError>,
) -> Result<(), TakeError> {
    match write(&build_sidecar(meta, result, fields)) {
        Err(ProjectError::InvalidSidecar(message)) if fields.is_some() => match on_invalid {
            OnInvalid::WriteFallback => {
                eprintln!("recording: {message}; writing the fallback sidecar");
                write(&build_sidecar(meta, result, None)).map_err(memo)
            }
            OnInvalid::KeepExisting => {
                eprintln!("recording: {message}; keeping the fallback sidecar");
                Ok(())
            }
        },
        other => other.map_err(memo),
    }
}

/// The listed entry of `stem`.
fn entry(project: &Project, stem: &str) -> Result<RecordingEntry, TakeError> {
    project.recording_entry(stem).map_err(memo)
}

/// Publishes a finished take's partial as `<stem>.wav` with its sidecar. Webview fields that
/// fail validation fall back to the timing-null sidecar (logged), so the audio is kept. On an
/// error the partial is still there, so the publish can be retried.
fn publish(
    meta: &TakeMeta,
    result: &TakeResult,
    fields: Option<&TakeStop>,
) -> Result<(), TakeError> {
    write_sidecar_or_fallback(meta, result, fields, OnInvalid::WriteFallback, |sidecar| {
        meta.project.finish_recording(&meta.stem, sidecar).map(drop)
    })
}

/// Publishes a take Rust ended or stopped with the fallback sidecar, logging a failure; whether
/// it was published.
pub(super) fn publish_fallback(meta: &TakeMeta, result: &TakeResult) -> bool {
    match publish(meta, result, None) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("recording: could not finalize {}: {e}", meta.stem);
            false
        }
    }
}

/// The webview's stop after Rust finalized the take: replaces the fallback sidecar, or
/// publishes the take when that has not happened yet (marking it published).
pub(super) fn complete_ended(
    ended: &mut EndedTake,
    fields: Option<&TakeStop>,
) -> Result<RecordingEntry, TakeError> {
    let meta = &ended.meta;
    if !ended.published {
        publish(meta, &ended.result, fields)?;
        ended.published = true;
    } else if fields.is_some() {
        write_sidecar_or_fallback(
            meta,
            &ended.result,
            fields,
            OnInvalid::KeepExisting,
            |sidecar| meta.project.write_sidecar(&meta.stem, sidecar),
        )?;
    }
    entry(&meta.project, &meta.stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1_000;
    const T0: u64 = 1_000_000_000;

    fn a(frame: u64, host_ns: u64) -> Anchor {
        Anchor { frame, host_ns }
    }

    #[test]
    fn bar1_frame_cases() {
        let anchors = [a(0, T0), a(1_000, T0 + 1_000_000_000)];
        assert_eq!(bar1_frame(&anchors, RATE, 2_000, None), None);
        assert_eq!(bar1_frame(&[], RATE, 2_000, Some(T0)), None);
        // Before the first frame.
        assert_eq!(
            bar1_frame(&anchors, RATE, 2_000, Some(T0 - 1_000_000)),
            None
        );
        // Beyond the take's frames.
        assert_eq!(
            bar1_frame(&anchors, RATE, 1_500, Some(T0 + 1_600_000_000)),
            None
        );
        // Interior, both ends of the range.
        assert_eq!(
            bar1_frame(&anchors, RATE, 2_000, Some(T0 + 250_500_000)),
            Some(250.5)
        );
        assert_eq!(bar1_frame(&anchors, RATE, 2_000, Some(T0)), Some(0.0));
        assert_eq!(
            bar1_frame(&anchors, RATE, 1_500, Some(T0 + 1_500_000_000)),
            Some(1_500.0)
        );
    }

    #[test]
    fn bar1_frame_above_two_to_the_53() {
        let base = (1u64 << 60) + 7;
        let anchors = [a(0, base), a(48_000, base + 1_000_000_000)];
        assert_eq!(
            bar1_frame(&anchors, 48_000, 96_000, Some(base + 500_000_000)),
            Some(24_000.0)
        );
        assert_eq!(
            bar1_frame(&anchors, 48_000, 96_000, Some(base + 1)),
            Some(48_000.0 / 1e9)
        );
    }
}

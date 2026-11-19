//! The streaming engine against the offline one: a refs fixture job's haps replayed through
//! `LiveEngine` (IRs built synchronously), each pause's haps scheduled when the render head
//! reaches that pause's frame, give the offline render bit for bit.

mod common;

use strudeland_engine::live::{LiveEngine, LiveEvent, LiveHap, LiveOptions};
use strudeland_engine::{EngineJob, QUANTUM, render};

const SR: f64 = 44100.0;
const Q: u64 = QUANTUM as u64;

/// `RoundUpToMultiple(frame, 128)`.
fn round_up_128(frame: u64) -> u64 {
    frame.div_ceil(Q) * Q
}

/// The frame an offline suspend at `suspend_sec` happens at: `trunc(suspend_sec * 44100)`
/// rounded up to the quantum.
fn pause_frame(suspend_sec: f64) -> u64 {
    round_up_128((suspend_sec * SR).trunc() as u64)
}

/// Renders quanta until the render head is at `frame`, appending the interleaved output.
fn render_until(e: &mut LiveEngine, frame: u64, out: &mut Vec<f32>) {
    let mut buf = [0.0f32; 2 * QUANTUM];
    while e.now_frame() < frame {
        e.render_quantum(&mut buf);
        out.extend_from_slice(&buf);
    }
    assert_eq!(e.now_frame(), frame, "pause frames are quantum multiples");
}

fn live_render(job: &EngineJob) -> (Vec<f32>, LiveEngine) {
    let mut e = LiveEngine::with_options(job.seed, LiveOptions { sync_ir: true });
    for (k, value) in job.values.iter().enumerate() {
        e.add_value(k as u32, value.clone(), job.orbits.get(k).cloned());
    }
    let mut out = Vec::new();
    for pause in &job.pauses {
        render_until(&mut e, pause_frame(pause.suspend_sec), &mut out);
        for hap in &pause.haps {
            e.schedule(LiveHap {
                frame: (hap.t * SR).floor().max(0.0) as u64,
                t_sec: hap.t,
                duration_sec: hap.duration,
                cycle: hap.cycle,
                value: hap.v,
                cps: job.cps,
            });
        }
    }
    render_until(&mut e, round_up_128(job.frames), &mut out);
    (out, e)
}

fn check(file: &str) {
    let job = common::load_job(file);
    assert!(
        job.orbits.len() <= job.values.len(),
        "{file}: more orbits than values"
    );
    assert_eq!(job.channels.count, 2, "{file}: the live engine is stereo");
    let offline = render(&job).unwrap_or_else(|e| panic!("{file}: {e}"));
    let (live, mut e) = live_render(&job);
    let frames = job.frames as usize;
    let bits = |x: &[f32]| x.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    for (c, channel) in offline.channels.iter().enumerate() {
        let live_c: Vec<f32> = live
            .iter()
            .skip(c)
            .step_by(2)
            .take(frames)
            .copied()
            .collect();
        assert_eq!(live_c.len(), frames, "{file}");
        if let Some(i) = (0..frames).find(|&i| live_c[i].to_bits() != channel[i].to_bits()) {
            panic!(
                "{file}: channel {c} differs first at frame {i}: live {} offline {}",
                live_c[i], channel[i]
            );
        }
        assert_eq!(bits(&live_c), bits(channel), "{file}: channel {c}");
    }
    assert!(
        channel_has_sound(&offline.channels[0]),
        "{file}: the fixture sounds"
    );
    let errors: Vec<String> = e
        .take_events()
        .into_iter()
        .map(|ev| match ev {
            LiveEvent::Error { message } => message,
            other => panic!("{file}: {other:?}"),
        })
        .collect();
    let offline_errors: Vec<&str> = offline.errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(errors, offline_errors, "{file}: the same render errors");
    let n = e.counters();
    assert_eq!(n.skipped_haps, 0, "{file}");
    assert_eq!(n.late_haps, 0, "{file}");
    let haps: usize = job.pauses.iter().map(|p| p.haps.len()).sum();
    assert_eq!(n.haps, haps as u64, "{file}");
}

fn channel_has_sound(c: &[f32]) -> bool {
    c.iter().any(|x| *x != 0.0)
}

#[test]
fn fx_room() {
    check("fixture-fx-room--mix-0.job.json");
}

#[test]
fn fx_bus() {
    check("fixture-fx-bus--mix-0.job.json");
}

#[test]
fn duck() {
    check("fixture-duck--mix-0.job.json");
}

#[test]
fn bass_line() {
    check("fixture-bass-line--mix-0.job.json");
}

#[test]
fn fx_delay() {
    check("fixture-fx-delay--mix-0.job.json");
}

#[test]
fn pulse_with_width_modulation() {
    check("pulse-pwm.job.json");
}

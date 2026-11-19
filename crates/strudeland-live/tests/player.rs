//! The `Player` through the fake output: anchors, batches, stats, scope, levels, the limiter,
//! device changes, stop.
//!
//! The fake's clock is virtual, so anchor arithmetic is exact; `Pace::RealTime` keeps the ring
//! fed. Timing waits are generous (the machine running the tests may be loaded).

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde_json::json;
use strudeland_engine::live::{LiveEngine, LiveHap};
use strudeland_engine::{OrbitSpec, QUANTUM, Value};
use strudeland_live::testing::{FAKE_EPOCH_NS, FakeHandle, FakeOutput, Pace, QuantumResampler};
use strudeland_live::{
    Batch, Event, HapEntry, LevelFrame, LiveError, Output, OutputError, OutputInfo, Player,
    RenderCallback, Stats, ValueEntry,
};

const WAIT: Duration = Duration::from_secs(5);
/// The master stage's lookahead delay in engine frames: `round(0.0015 * 44100)`.
const MASTER_DELAY: f64 = 66.0;
/// The limiter's ceiling, -1 dBFS.
const CEILING: f32 = 0.891_251;

fn info(rate: u32) -> OutputInfo {
    OutputInfo {
        sample_rate: rate,
        buffer_frames: None,
    }
}

fn start(rate: u32, pace: Pace) -> (Player, Receiver<Event>, FakeHandle) {
    let (out, handle) = FakeOutput::new(info(rate), pace);
    let (player, rx) = Player::start(Box::new(out), 0).expect("player starts");
    (player, rx, handle)
}

fn orbit1() -> Option<OrbitSpec> {
    Some(OrbitSpec {
        orbit: 1.0,
        channels: vec![0, 1],
    })
}

fn sine(freq: f64) -> serde_json::Value {
    json!({ "s": "sine", "freq": freq, "gain": 0.5 })
}

/// A batch adding value `index` (with orbit 1 when it is the first) and one hap of it.
fn one_hap(index: u32, value: serde_json::Value, t: f64, duration: f64) -> Batch {
    Batch {
        values: vec![ValueEntry(
            index,
            value,
            if index == 0 { orbit1() } else { None },
        )],
        haps: vec![HapEntry(t, duration, t, index)],
        cps: 1.0,
    }
}

/// The next event matching `pred` within `timeout`, with the events skipped on the way.
fn wait_for(
    rx: &Receiver<Event>,
    timeout: Duration,
    mut pred: impl FnMut(&Event) -> bool,
) -> (Option<Event>, Vec<Event>) {
    let end = Instant::now() + timeout;
    let mut skipped = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) if pred(&e) => return (Some(e), skipped),
            Ok(e) => skipped.push(e),
            Err(_) => break,
        }
    }
    (None, skipped)
}

fn collect_for(rx: &Receiver<Event>, d: Duration) -> Vec<Event> {
    wait_for(rx, d, |_| false).1
}

fn anchors(events: &[Event]) -> Vec<(u64, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Anchor { frame, host_ns } => Some((*frame, *host_ns)),
            _ => None,
        })
        .collect()
}

fn is_anchor(e: &Event) -> bool {
    matches!(e, Event::Anchor { .. })
}

/// A `Stats` event that satisfies `pred`.
fn wait_stats(rx: &Receiver<Event>, mut pred: impl FnMut(&Stats) -> bool) -> (Stats, Vec<Event>) {
    let (e, skipped) = wait_for(rx, WAIT, |e| matches!(e, Event::Stats(s) if pred(s)));
    match e {
        Some(Event::Stats(s)) => (s, skipped),
        _ => panic!("no matching stats; events {skipped:?}"),
    }
}

/// The engine time (seconds) heard now, from an anchor and the fake's clock.
fn heard_now(anchor: (u64, u64), handle: &FakeHandle) -> f64 {
    anchor.0 as f64 / 44100.0 + (handle.host_now_ns() - anchor.1) as f64 / 1e9
}

/// The fake's pulled frame index of a host time (exact: the fake's clock is frames / rate).
fn pulled_frame(host_ns: u64, rate: u32) -> usize {
    ((host_ns - FAKE_EPOCH_NS) as f64 * f64::from(rate) / 1e9).round() as usize
}

fn left(samples: &[f32]) -> Vec<f32> {
    samples.chunks(2).map(|f| f[0]).collect()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

/// The mean spacing of rising zero crossings in the loud middle of `x`.
fn crossing_spacing(x: &[f32]) -> f64 {
    let first = x.iter().position(|s| s.abs() > 0.05).expect("a loud part");
    let body = &x[first + 4000..first + 12000];
    let c: Vec<usize> = (1..body.len())
        .filter(|&i| body[i - 1] < 0.0 && body[i] >= 0.0)
        .collect();
    assert!(c.len() > 20, "{} crossings", c.len());
    (c[c.len() - 1] - c[0]) as f64 / (c.len() - 1) as f64
}

#[test]
fn the_first_event_is_an_anchor_at_frame_0() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    let first = rx.recv_timeout(WAIT).expect("an event");
    assert!(
        matches!(first, Event::Anchor { frame: 0, host_ns } if host_ns >= FAKE_EPOCH_NS),
        "{first:?}"
    );
    player.stop();
}

#[test]
fn a_failing_output_fails_start() {
    let (out, handle) = FakeOutput::new(info(44100), Pace::RealTime);
    handle.fail_starts(true);
    match Player::start(Box::new(out), 0) {
        Err(LiveError::Output(_)) => {}
        Err(e) => panic!("wrong error {e}"),
        Ok(_) => panic!("started"),
    }
}

#[test]
fn about_four_anchors_a_second() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    let events = collect_for(&rx, Duration::from_millis(1200));
    player.stop();
    let a = anchors(&events);
    assert!((3..=5).contains(&a.len()), "{a:?}");
    assert!(a.windows(2).all(|w| w[1].0 > w[0].0 && w[1].1 > w[0].1));
}

#[test]
fn an_underrun_is_counted_and_anchored() {
    let (player, rx, _h) = start(44100, Pace::AsFastAsPossible);
    let (stats, before) = wait_stats(&rx, |_| true);
    player.stop();
    assert!(stats.underruns > 0, "{stats:?}");
    assert_eq!(stats.render_headroom_ms, 0.0);
    // the first anchor, then one after each underrun
    assert!(anchors(&before).len() >= 2);
}

#[test]
fn resampled_anchors_are_net_of_the_resampler_delay() {
    let (player, rx, _h) = start(48000, Pace::RealTime);
    let events = collect_for(&rx, Duration::from_millis(700));
    player.stop();
    let a = anchors(&events);
    assert!(a.len() >= 2, "{a:?}");
    assert_eq!(a[0].0, 0, "the delay clamps the first anchor at 0");
    // the resampler's delay and the master stage's lookahead (66 frames)
    let delay = QuantumResampler::new(48000).unwrap().delay_engine_frames() + MASTER_DELAY;
    for w in a.windows(2) {
        // frames consumed between two anchors: their host times on the fake's clock
        let consumed = (pulled_frame(w[1].1, 48000) - pulled_frame(a[0].1, 48000)) as f64;
        let expected = (consumed * 44100.0 / 48000.0 - delay).round() as u64;
        assert_eq!(w[1].0, expected);
    }
}

#[test]
fn a_hap_is_heard_at_its_pitch_through_the_resampler() {
    let (player, rx, h) = start(48000, Pace::RealTime);
    let (a, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame, host_ns }) = a else {
        panic!("no anchor")
    };
    let t = heard_now((frame, host_ns), &h) + 0.2;
    player.schedule(one_hap(0, sine(440.0), t, 0.6));
    let (s, _) = wait_stats(&rx, |s| s.haps == 1);
    assert_eq!(s.late_haps, 0);
    std::thread::sleep(Duration::from_millis(900));
    player.stop();
    let x = left(&h.pulled());
    assert!(peak(&x) > 0.01);
    let spacing = crossing_spacing(&x);
    let want = 48000.0 / 440.0;
    assert!((spacing - want).abs() / want < 0.005, "spacing {spacing}");
}

/// Renders `value` at `t` on a fresh engine: the samples a 44.1 kHz stream should carry.
fn reference(value: serde_json::Value, t: f64, duration: f64, frames: usize) -> Vec<f32> {
    let mut e = LiveEngine::new(0);
    let Ok(Value::Obj(m)) = Value::from_json(value) else {
        panic!("map")
    };
    e.add_value(0, m, orbit1());
    e.schedule(LiveHap {
        frame: (t * 44100.0).floor() as u64,
        t_sec: t,
        duration_sec: duration,
        cycle: t,
        value: 0,
        cps: 1.0,
    });
    let mut out = Vec::new();
    let mut q = [0.0f32; 2 * QUANTUM];
    while out.len() < frames * 2 {
        e.render_quantum(&mut q);
        out.extend_from_slice(&q);
    }
    out
}

#[test]
fn a_device_change_to_44100_drops_the_resampler() {
    let (player, rx, h) = start(48000, Pace::RealTime);
    let (first, _) = wait_for(&rx, WAIT, is_anchor);
    assert!(first.is_some());
    std::thread::sleep(Duration::from_millis(300));
    let changed_at = h.host_now_ns();
    h.change_device(info(44100));
    // let the new stream run, then read its latest anchor
    let mut events = collect_for(&rx, Duration::from_millis(400));
    let after: Vec<(u64, u64)> = anchors(&events)
        .into_iter()
        .filter(|a| a.1 > changed_at)
        .collect();
    assert!(!after.is_empty(), "{events:?}");
    let last = *after.last().unwrap();
    let t = heard_now(last, &h) + 0.2;
    let value = sine(330.0);
    player.schedule(one_hap(0, value.clone(), t, 0.3));
    events.extend(collect_for(&rx, Duration::from_millis(800)));
    player.stop();
    let mut all = vec![first.unwrap()];
    all.extend(events);
    let a = anchors(&all);
    assert!(a.windows(2).all(|w| w[1].0 >= w[0].0), "monotonic {a:?}");
    // at 44.1 kHz the ring carries the engine's samples: onset-aligned, bit for bit
    let got = h.pulled();
    let want = reference(value, t, 0.3, ((t + 0.4) * 44100.0) as usize);
    let onset = |s: &[f32]| s.iter().position(|&v| v != 0.0).expect("sound");
    let (g, w) = (onset(&got), onset(&want));
    let n = 2 * 8820;
    assert!(got.len() >= g + n && want.len() >= w + n);
    assert_eq!(&got[g..g + n], &want[w..w + n]);
}

#[test]
fn a_device_change_to_96000_resamples_at_the_new_rate() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    wait_for(&rx, WAIT, is_anchor).0.expect("anchor");
    let changed_at = h.host_now_ns();
    h.change_device(info(96000));
    let (a, _) = wait_for(
        &rx,
        WAIT,
        |e| matches!(e, Event::Anchor { host_ns, .. } if *host_ns > changed_at),
    );
    let Some(Event::Anchor { frame, host_ns }) = a else {
        panic!("no anchor after the change")
    };
    std::thread::sleep(Duration::from_millis(100));
    let t = heard_now((frame, host_ns), &h) + 0.2;
    player.schedule(one_hap(0, sine(440.0), t, 0.6));
    std::thread::sleep(Duration::from_millis(1000));
    player.stop();
    let x = left(&h.pulled());
    let spacing = crossing_spacing(&x);
    let want = 96000.0 / 440.0;
    assert!((spacing - want).abs() / want < 0.005, "spacing {spacing}");
}

#[test]
fn a_reopened_streams_first_anchor_is_on_the_same_line_as_the_next() {
    // the new stream starts at an engine frame past 0, so its first anchor (consumed 0) is
    // not clamped: it reports the frame heard, `base - MASTER_DELAY`, like every later one
    let (player, rx, h) = start(44100, Pace::RealTime);
    wait_for(&rx, WAIT, is_anchor).0.expect("anchor");
    std::thread::sleep(Duration::from_millis(300));
    let changed_at = h.host_now_ns();
    h.change_device(info(44100));
    let after = |e: &Event| matches!(e, Event::Anchor { host_ns, .. } if *host_ns > changed_at);
    let (a0, _) = wait_for(&rx, WAIT, after);
    let (a1, _) = wait_for(&rx, WAIT, after);
    player.stop();
    let (
        Some(Event::Anchor {
            frame: f0,
            host_ns: h0,
        }),
        Some(Event::Anchor {
            frame: f1,
            host_ns: h1,
        }),
    ) = (a0, a1)
    else {
        panic!("no two anchors after the change")
    };
    assert!(f0 > 0, "the first anchor of the new stream is past frame 0");
    // at 44.1 kHz no resampler: the engine frames between two anchors are the device frames
    let pulled = (pulled_frame(h1, 44100) - pulled_frame(h0, 44100)) as u64;
    assert_eq!(f1 - f0, pulled, "anchors {f0} then {f1}");
}

#[test]
fn a_failed_reopen_is_reported_once_and_retried() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    let (first, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame: f0, .. }) = first else {
        panic!("no anchor")
    };
    h.fail_starts(true);
    h.change_device(info(44100));
    let (err, _) = wait_for(&rx, WAIT, |e| matches!(e, Event::Error { .. }));
    assert!(err.is_some());
    // the retries fail silently
    let quiet = collect_for(&rx, Duration::from_millis(1500));
    assert!(!quiet.iter().any(|e| matches!(e, Event::Error { .. })));
    h.fail_starts(false);
    let (back, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame, .. }) = back else {
        panic!("no anchor after the retry")
    };
    assert!(frame >= f0);
    player.stop();
}

#[test]
fn values_sent_while_the_output_is_down_are_kept() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    wait_for(&rx, WAIT, is_anchor).0.expect("no anchor");
    h.fail_starts(true);
    h.change_device(info(44100));
    wait_for(&rx, WAIT, |e| matches!(e, Event::Error { .. }))
        .0
        .expect("no reopen error");
    // while down: the value's only crossing (its hap is dropped)
    player.schedule(one_hap(0, sine(440.0), 0.0, 0.1));
    std::thread::sleep(Duration::from_millis(100));
    h.fail_starts(false);
    let (back, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame, host_ns }) = back else {
        panic!("no anchor after the retry")
    };
    // back up: a hap naming the value sent while down
    let t = heard_now((frame, host_ns), &h) + 0.2;
    player.schedule(Batch {
        values: vec![],
        haps: vec![HapEntry(t, 0.1, t, 0)],
        cps: 1.0,
    });
    let (s, events) = wait_stats(&rx, |s| s.batches == 2 && s.haps == 1);
    player.stop();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Error { .. })),
        "{events:?}"
    );
    assert_eq!(s.haps, 1);
}

#[test]
fn a_batch_plays() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    let (a, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame, host_ns }) = a else {
        panic!("no anchor")
    };
    let t = heard_now((frame, host_ns), &h) + 0.2;
    player.schedule(one_hap(0, sine(440.0), t, 0.3));
    std::thread::sleep(Duration::from_millis(700));
    player.stop();
    let pulled = h.pulled();
    let at = 2 * (pulled_frame(host_ns, 44100) + (t * 44100.0) as usize - frame as usize);
    assert!(peak(&pulled[..at]) == 0.0, "silent before the hap");
    assert!(peak(&pulled[at..]) > 0.01);
}

#[test]
fn a_late_hap_is_counted() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    std::thread::sleep(Duration::from_millis(300));
    player.schedule(one_hap(0, sine(440.0), 0.0, 0.1));
    let (s, _) = wait_stats(&rx, |s| s.haps == 1);
    player.stop();
    assert_eq!(s.late_haps, 1);
}

#[test]
fn an_unsupported_value_warns_once_and_is_skipped() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    player.schedule(Batch {
        values: vec![ValueEntry(0, json!({ "s": "z_noise" }), orbit1())],
        haps: vec![HapEntry(0.3, 0.1, 0.3, 0), HapEntry(0.4, 0.1, 0.4, 0)],
        cps: 1.0,
    });
    let (s, events) = wait_stats(&rx, |s| s.batches == 1);
    player.stop();
    let warnings: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::Warning { .. }))
        .collect();
    assert_eq!(warnings.len(), 1, "{events:?}");
    let Event::Warning {
        signature,
        first_cycle,
        ..
    } = warnings[0]
    else {
        unreachable!()
    };
    assert_eq!(signature, "z_noise");
    assert_eq!(*first_cycle, 0.3);
    assert_eq!(s.skipped_haps, 2);
    assert_eq!(s.haps, 0);
}

#[test]
fn a_malformed_value_is_an_error_and_play_goes_on() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    player.schedule(Batch {
        values: vec![
            ValueEntry(0, json!(5), orbit1()),
            ValueEntry(1, sine(440.0), orbit1()),
        ],
        haps: vec![HapEntry(0.3, 0.1, 0.3, 0), HapEntry(0.3, 0.2, 0.3, 1)],
        cps: 1.0,
    });
    let (err, _) = wait_for(&rx, WAIT, |e| matches!(e, Event::Error { .. }));
    let Some(Event::Error { message }) = err else {
        panic!("no error")
    };
    assert!(message.contains("value 0"), "{message}");
    // the good value (index 1) still plays and nothing else is reported
    let (s, events) = wait_stats(&rx, |s| s.haps == 1);
    assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
    assert_eq!(s.batches, 1);
    let pulls = h.pulls();
    wait_for(&rx, WAIT, is_anchor).0.expect("still running");
    assert!(h.pulls() > pulls);
    player.stop();
    assert!(peak(&h.pulled()) > 0.01);
}

#[test]
fn a_hap_naming_an_unknown_value_is_an_error() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    player.schedule(Batch {
        values: vec![],
        haps: vec![HapEntry(0.3, 0.1, 0.3, 42), HapEntry(0.4, 0.1, 0.4, 42)],
        cps: 1.0,
    });
    let (err, _) = wait_for(&rx, WAIT, |e| matches!(e, Event::Error { .. }));
    let Some(Event::Error { message }) = err else {
        panic!("no error")
    };
    assert!(message.contains("42"), "{message}");
    let (s, events) = wait_stats(&rx, |s| s.batches == 1);
    assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
    assert_eq!(s.haps, 0);
    player.stop();
}

#[test]
fn stats_arrive_every_second() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    player.schedule(Batch {
        values: vec![ValueEntry(0, sine(220.0), orbit1())],
        haps: vec![
            HapEntry(0.5, 0.1, 0.5, 0),
            HapEntry(0.6, 0.1, 0.6, 0),
            HapEntry(0.7, 0.1, 0.7, 0),
        ],
        cps: 1.0,
    });
    let (e, _) = wait_for(
        &rx,
        Duration::from_millis(2500),
        |e| matches!(e, Event::Stats(s) if s.batches == 1),
    );
    player.stop();
    let Some(Event::Stats(s)) = e else {
        panic!("no stats in 2.5 s")
    };
    assert_eq!(s.haps, 3);
    assert!(s.batch_decode_us.max > 0, "{s:?}");
    assert!(s.batch_decode_us.p50 <= s.batch_decode_us.max);
    assert!(s.render_headroom_ms > 0.0, "{s:?}");
}

#[test]
fn the_scope_shows_a_playing_hap() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    player.schedule(one_hap(0, sine(440.0), 0.3, 1.0));
    let (e, _) = wait_for(
        &rx,
        WAIT,
        |e| matches!(e, Event::Scope { samples } if peak(samples) > 0.01),
    );
    player.stop();
    let Some(Event::Scope { samples }) = e else {
        panic!("no loud scope")
    };
    assert_eq!(samples.len(), 1024);
}

#[test]
fn stop_fades_to_silence_quickly() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    player.schedule(one_hap(0, sine(440.0), 0.2, 5.0));
    wait_for(
        &rx,
        WAIT,
        |e| matches!(e, Event::Scope { samples } if peak(samples) > 0.01),
    )
    .0
    .expect("playing");
    // the scope shows what was rendered; let it reach the output
    std::thread::sleep(Duration::from_millis(250));
    let started = Instant::now();
    player.stop();
    assert!(started.elapsed() < Duration::from_millis(500));
    let pulled = h.pulled();
    let block = 2 * 512;
    assert!(pulled.len() >= 4 * block);
    let last = &pulled[pulled.len() - block..];
    assert!(last.iter().all(|&s| s == 0.0), "the final block is silent");
    assert!(
        peak(&pulled[pulled.len() - 8 * block..]) > 0.01,
        "it was playing"
    );
    // nothing is pulled after stop
    let n = h.pulls();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(h.pulls(), n);
}

#[test]
fn drop_without_stop_does_not_hang_and_a_restart_begins_at_0() {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (player, _rx, _h) = start(44100, Pace::RealTime);
        player.schedule(one_hap(0, sine(440.0), 0.1, 1.0));
        std::thread::sleep(Duration::from_millis(200));
        drop(player);
        let _ = done_tx.send(());
    });
    done_rx.recv_timeout(WAIT).expect("drop returned");
    let (player, rx, _h) = start(44100, Pace::RealTime);
    let (a, _) = wait_for(&rx, WAIT, is_anchor);
    assert!(matches!(a, Some(Event::Anchor { frame: 0, .. })), "{a:?}");
    player.stop();
}

#[test]
fn set_seed_changes_the_audio_of_later_haps() {
    let noise = json!({ "s": "white", "gain": 0.5 });
    let run = |reseed: bool| {
        let (player, rx, h) = start(44100, Pace::RealTime);
        let (a, _) = wait_for(&rx, WAIT, is_anchor);
        let Some(Event::Anchor { frame: 0, host_ns }) = a else {
            panic!("no first anchor")
        };
        player.schedule(one_hap(0, noise.clone(), 0.3, 0.1));
        wait_stats(&rx, |s| s.haps == 1);
        if reseed {
            player.set_seed(1234);
        }
        player.schedule(Batch {
            values: vec![],
            haps: vec![HapEntry(1.6, 0.1, 0.3, 0)],
            cps: 1.0,
        });
        std::thread::sleep(Duration::from_millis(1000));
        player.stop();
        let pulled = h.pulled();
        let base = pulled_frame(host_ns, 44100);
        let seg = |t: f64| {
            let i = 2 * (base + (t * 44100.0) as usize);
            pulled[i..i + 2 * 4410].to_vec()
        };
        (seg(0.3), seg(1.6))
    };
    let (a1, a2) = run(false);
    let (b1, b2) = run(true);
    assert!(peak(&a1) > 0.01 && peak(&a2) > 0.01 && peak(&b2) > 0.01);
    assert_eq!(a1, b1, "same seed, same noise");
    assert_eq!(a2.len(), b2.len());
    assert_ne!(a2, b2, "a new seed, new noise");
}

#[test]
fn a_dropped_event_receiver_does_not_stop_play() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    drop(rx);
    player.schedule(one_hap(0, sine(440.0), 0.2, 0.2));
    std::thread::sleep(Duration::from_millis(1300)); // past a stats event
    let pulls = h.pulls();
    std::thread::sleep(Duration::from_millis(100));
    assert!(h.pulls() > pulls);
    let started = Instant::now();
    player.stop();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(peak(&h.pulled()) > 0.01);
}

fn levels(events: &[Event]) -> Vec<LevelFrame> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Levels(f) => Some(*f),
            _ => None,
        })
        .collect()
}

fn loud() -> serde_json::Value {
    json!({ "s": "sine", "freq": 220.0, "gain": 1.0, "postgain": 4.0 })
}

#[test]
fn an_impulse_at_frame_f_is_heard_at_the_anchor_that_reports_f() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    // the second anchor: the first is clamped at 0 (the delays put it before frame 0)
    let (first, _) = wait_for(&rx, WAIT, is_anchor);
    assert!(
        matches!(first, Some(Event::Anchor { frame: 0, .. })),
        "{first:?}"
    );
    let (a, _) = wait_for(&rx, WAIT, is_anchor);
    let Some(Event::Anchor { frame, host_ns }) = a else {
        panic!("no second anchor")
    };
    assert!(frame > 0);
    let t = heard_now((frame, host_ns), &h) + 0.2;
    let value = sine(440.0);
    player.schedule(one_hap(0, value.clone(), t, 0.2));
    std::thread::sleep(Duration::from_millis(700));
    player.stop();
    let onset = |s: &[f32]| s.iter().position(|&v| v != 0.0).expect("sound") / 2;
    // the engine frame of the first sound, and where the output carried it
    let sounded = onset(&reference(value, t, 0.2, ((t + 0.3) * 44100.0) as usize)) as u64;
    let heard = onset(&h.pulled()) as u64;
    let at_anchor = pulled_frame(host_ns, 44100) as u64;
    assert_eq!(heard, at_anchor + (sounded - frame));
}

/// An output that starts and never calls its callback: no anchors, the ring stays primed.
struct SilentOutput;

impl Output for SilentOutput {
    fn start(&mut self, render: RenderCallback) -> Result<OutputInfo, OutputError> {
        drop(render);
        Ok(info(44100))
    }
    fn stop(&mut self) {}
}

#[test]
fn levels_arrive_without_an_anchor() {
    let (player, rx) = Player::start(Box::new(SilentOutput), 0).expect("player starts");
    let events = collect_for(&rx, Duration::from_millis(400));
    player.stop();
    assert!(!events.iter().any(is_anchor), "{events:?}");
    // the priming renders once; with nothing rendered since, no further frame is sent
    let l = levels(&events);
    assert_eq!(l.len(), 1, "{events:?}");
    assert_eq!(l[0], LevelFrame::default(), "priming renders silence");
}

#[test]
fn levels_cover_every_rendered_sample() {
    let (player, rx, _h) = start(44100, Pace::RealTime);
    let (t, duration) = (0.3, 0.3);
    player.schedule(one_hap(0, loud(), t, duration));
    let events = collect_for(&rx, Duration::from_millis(1500));
    player.stop();
    let want = reference(loud(), t, duration, ((t + 1.0) * 44100.0) as usize);
    let count = |ch: usize| {
        want.iter()
            .skip(ch)
            .step_by(2)
            .filter(|s| s.abs() >= 1.0)
            .count() as u64
    };
    let (want_l, want_r) = (count(0), count(1));
    assert!(want_l > 100, "the hap clips: {want_l}");
    let l = levels(&events);
    assert!(l.len() > 20, "about 30 a second: {}", l.len());
    let sum = |ch: usize| l.iter().map(|f| u64::from(f.overs[ch])).sum::<u64>();
    assert_eq!((sum(0), sum(1)), (want_l, want_r));
    assert!(l.iter().all(|f| f.limit_db == 0.0), "the limiter is off");
    let peak = l.iter().fold(0.0f32, |p, f| p.max(f.peak[0]));
    assert_eq!(peak, peak_of(&want, 0));
}

fn peak_of(samples: &[f32], ch: usize) -> f32 {
    samples
        .iter()
        .skip(ch)
        .step_by(2)
        .fold(0.0f32, |m, &s| m.max(s.abs()))
}

#[test]
fn set_limiter_holds_a_loud_hap_to_the_ceiling() {
    let (player, rx, h) = start(44100, Pace::RealTime);
    player.set_limiter(true);
    player.schedule(one_hap(0, loud(), 0.3, 0.3));
    let events = collect_for(&rx, Duration::from_millis(900));
    player.stop();
    let out = peak(&h.pulled());
    assert!(out <= CEILING, "output peak {out}");
    assert!(out > 0.85, "output peak {out}");
    // levels are measured before the limiter, which reports its gain
    let l = levels(&events);
    assert!(l.iter().any(|f| f.peak[0] > 1.0 && f.overs[0] > 0));
    let deepest = l.iter().fold(0.0f32, |m, f| m.min(f.limit_db));
    assert!(deepest < -1.0, "limit {deepest} dB");
}

//! The writer thread: drains the rings, writes the WAV, flushes once a second, enforces the
//! length cap and emits the level events.

use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hound::{WavSpec, WavWriter};

use crate::anchors::Anchor;
use crate::error::RecordError;
use crate::levels::{Clock, LEVEL_INTERVAL, LevelAccumulator};
use crate::pipeline::Rings;
use crate::recorder::{Ended, Event, Msg};

/// The writer's poll interval.
pub const TICK: Duration = Duration::from_millis(5);
/// How often the WAV header is rewritten and the buffer flushed.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// The file's write buffer.
pub const WRITE_BUFFER: usize = 65_536;
/// The longest a close waits for the ring to drain.
const CLOSE_DRAIN: Duration = Duration::from_millis(100);

/// The take length cap in frames: two hours, or 1e9 frames (the u32 WAV data-size limit for
/// 4-byte samples) above 138 888 Hz.
pub fn max_frames(sample_rate: u32) -> u64 {
    (7_200 * u64::from(sample_rate)).min(1_000_000_000)
}

/// The mono 32-bit float spec of a take.
pub fn wav_spec(sample_rate: u32) -> WavSpec {
    WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    }
}

/// A finished take, as the writer saw it.
#[derive(Debug)]
pub(crate) struct TakeData {
    pub frames: u64,
    pub anchors: Vec<Anchor>,
    pub ended: Ended,
}

struct OpenTake<W: Write + Seek> {
    id: u64,
    wav: WavWriter<W>,
    written: u64,
    max_frames: u64,
    anchors: Vec<Anchor>,
    last_flush: Duration,
    first_anchor: Option<SyncSender<Anchor>>,
    at_cap: bool,
    path: PathBuf,
}

/// The writer's state, driven by [`WriterCore::tick`] with an explicit time so tests control
/// the flush and level cadence.
pub(crate) struct WriterCore<W: Write + Seek> {
    rings: Rings,
    events: Sender<Event>,
    levels: LevelAccumulator,
    last_level: Duration,
    take: Option<OpenTake<W>>,
    /// A take that ended by itself (cap, write error), kept for the next close.
    ended: Option<TakeData>,
}

impl<W: Write + Seek> WriterCore<W> {
    pub fn new(rings: Rings, events: Sender<Event>, now: Duration) -> WriterCore<W> {
        WriterCore {
            rings,
            events,
            levels: LevelAccumulator::default(),
            last_level: now,
            take: None,
            ended: None,
        }
    }

    /// Starts writing take `id` to `wav`. Audio and anchors still in the rings are older than
    /// the take and are dropped.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        id: u64,
        wav: WavWriter<W>,
        path: PathBuf,
        max_frames: u64,
        first_anchor: SyncSender<Anchor>,
        now: Duration,
    ) {
        let n = self.rings.audio.slots();
        if let Ok(chunk) = self.rings.audio.read_chunk(n) {
            chunk.commit_all();
        }
        while self.rings.anchors.pop().is_ok() {}
        self.ended = None;
        self.take = Some(OpenTake {
            id,
            wav,
            written: 0,
            max_frames,
            anchors: Vec::new(),
            last_flush: now,
            first_anchor: Some(first_anchor),
            at_cap: false,
            path,
        });
    }

    /// One poll: drain audio, anchors and levels, emit a level, flush, check the cap. Returns
    /// the take's id when it ended by itself in this tick.
    pub fn tick(&mut self, now: Duration) -> Option<u64> {
        let mut ended = self.drain_audio().err().map(Ended::WriteError);
        self.drain_anchors();
        self.drain_levels(now);
        if ended.is_none()
            && let Some(take) = &mut self.take
        {
            if now.saturating_sub(take.last_flush) >= FLUSH_INTERVAL {
                take.last_flush = now;
                if let Err(e) = take.wav.flush() {
                    ended = Some(Ended::WriteError(e.to_string()));
                }
            }
            if ended.is_none() && take.at_cap {
                ended = Some(Ended::Cap);
            }
        }
        let ended = ended?;
        let id = self.take.as_ref()?.id;
        self.end(ended);
        Some(id)
    }

    /// Writes what is in the audio ring (dropped when no take is open). Returns the frames
    /// taken off the ring.
    fn drain_audio(&mut self) -> Result<usize, String> {
        let n = self.rings.audio.slots();
        if n == 0 {
            return Ok(0);
        }
        let Ok(chunk) = self.rings.audio.read_chunk(n) else {
            return Ok(0);
        };
        if let Some(take) = self.take.as_mut().filter(|t| !t.at_cap) {
            let (a, b) = chunk.as_slices();
            let room = usize::try_from(take.max_frames - take.written).unwrap_or(usize::MAX);
            for &x in a.iter().chain(b).take(room) {
                let x = if x.is_finite() { x } else { 0.0 };
                take.wav.write_sample(x).map_err(|e| e.to_string())?;
                take.written += 1;
            }
            if take.written >= take.max_frames {
                take.at_cap = true;
            }
        }
        chunk.commit_all();
        Ok(n)
    }

    fn drain_anchors(&mut self) {
        while let Ok(anchor) = self.rings.anchors.pop() {
            let Some(take) = &mut self.take else {
                continue;
            };
            if anchor.frame >= take.max_frames {
                continue;
            }
            if let Some(first) = take.first_anchor.take() {
                let _ = first.try_send(anchor);
            }
            take.anchors.push(anchor);
        }
    }

    fn drain_levels(&mut self, now: Duration) {
        while let Ok(block) = self.rings.levels.pop() {
            self.levels.add(&block);
        }
        if now.saturating_sub(self.last_level) >= LEVEL_INTERVAL {
            self.last_level = now;
            if let Some(level) = self.levels.take() {
                let _ = self.events.send(Event::Level(level));
            }
        }
    }

    /// Finalizes the open take and keeps its data for the next close.
    fn end(&mut self, ended: Ended) {
        let Some(take) = self.take.take() else {
            return;
        };
        let finalized = take.wav.finalize();
        let ended = match (ended, finalized) {
            (Ended::User | Ended::Cap, Err(e)) => Ended::WriteError(e.to_string()),
            (ended, _) => ended,
        };
        self.ended = Some(TakeData {
            frames: take.written,
            anchors: take.anchors,
            ended,
        });
    }

    /// Ends the take at the user's stop: writes what is in the ring (until it is empty twice in
    /// a row, at most 100 ms), finalizes, and returns the data. For a take that already ended
    /// by itself, returns that data once.
    pub fn close(&mut self) -> Option<TakeData> {
        if self.take.is_some() {
            let deadline = Instant::now() + CLOSE_DRAIN;
            let mut empty = 0;
            let mut ended = Ended::User;
            while empty < 2 {
                match self.drain_audio() {
                    Ok(0) => empty += 1,
                    Ok(_) => empty = 0,
                    Err(e) => {
                        ended = Ended::WriteError(e);
                        break;
                    }
                }
                self.drain_anchors();
                if empty < 2 {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            if matches!(ended, Ended::User) && self.take.as_ref().is_some_and(|t| t.at_cap) {
                ended = Ended::Cap;
            }
            self.end(ended);
        }
        self.ended.take()
    }

    /// Drops the open take without finalizing; returns its path for the caller to remove.
    pub fn abort(&mut self) -> Option<PathBuf> {
        self.ended = None;
        self.take.take().map(|t| t.path)
    }
}

/// Messages to the writer thread.
pub(crate) enum WriterMsg {
    Open {
        id: u64,
        file: File,
        path: PathBuf,
        sample_rate: u32,
        max_frames: u64,
        first_anchor: SyncSender<Anchor>,
        reply: SyncSender<Result<(), RecordError>>,
    },
    Close(SyncSender<Option<TakeData>>),
    /// Drops the open take and removes its file.
    Abort(SyncSender<()>),
    Quit,
}

pub(crate) struct WriterHandle {
    pub tx: Sender<WriterMsg>,
    pub join: JoinHandle<()>,
}

impl WriterHandle {
    /// Sends `msg` built around a reply channel and waits for the reply.
    pub fn call<T>(&self, msg: impl FnOnce(SyncSender<T>) -> WriterMsg) -> Option<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx.send(msg(tx)).ok()?;
        rx.recv().ok()
    }

    pub fn quit(self) {
        let _ = self.tx.send(WriterMsg::Quit);
        let _ = self.join.join();
    }
}

/// Spawns the `record-writer` thread over `rings`.
pub(crate) fn spawn_writer(
    rings: Rings,
    events: Sender<Event>,
    control: Sender<Msg>,
    clock: Box<dyn Clock>,
) -> std::io::Result<WriterHandle> {
    let (tx, rx) = mpsc::channel();
    let join = std::thread::Builder::new()
        .name("record-writer".to_string())
        .spawn(move || run(rings, events, control, clock, rx))?;
    Ok(WriterHandle { tx, join })
}

fn run(
    rings: Rings,
    events: Sender<Event>,
    control: Sender<Msg>,
    clock: Box<dyn Clock>,
    rx: Receiver<WriterMsg>,
) {
    let mut core: WriterCore<BufWriter<File>> = WriterCore::new(rings, events, clock.now());
    loop {
        match rx.recv_timeout(TICK) {
            Ok(WriterMsg::Open {
                id,
                file,
                path,
                sample_rate,
                max_frames,
                first_anchor,
                reply,
            }) => {
                let buf = BufWriter::with_capacity(WRITE_BUFFER, file);
                match WavWriter::new(buf, wav_spec(sample_rate)) {
                    Ok(wav) => {
                        core.open(id, wav, path, max_frames, first_anchor, clock.now());
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e.into()));
                    }
                }
            }
            Ok(WriterMsg::Close(reply)) => {
                let _ = reply.send(core.close());
            }
            Ok(WriterMsg::Abort(reply)) => {
                if let Some(path) = core.abort() {
                    let _ = std::fs::remove_file(path);
                }
                let _ = reply.send(());
            }
            Ok(WriterMsg::Quit) | Err(RecvTimeoutError::Disconnected) => {
                core.close();
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(take_id) = core.tick(clock.now()) {
            let _ = control.send(Msg::WriterEnded { take_id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::levels::Level;
    use crate::pipeline::{Capture, Shared};
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    const RATE: u32 = 1_000;
    const HEADER_LEN: usize = 68;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    struct Rig {
        capture: Capture,
        shared: Arc<Shared>,
        events: Receiver<Event>,
        first: Receiver<Anchor>,
    }

    fn rig<W: Write + Seek>(wav: Option<WavWriter<W>>, max: u64) -> (WriterCore<W>, Rig) {
        let shared = Arc::new(Shared::default());
        let (capture, rings) = Capture::new(RATE, shared.clone());
        let (ev_tx, events) = mpsc::channel();
        let mut core = WriterCore::new(rings, ev_tx, Duration::ZERO);
        let (first_tx, first) = mpsc::sync_channel(1);
        if let Some(wav) = wav {
            core.open(
                1,
                wav,
                PathBuf::from("x.wav"),
                max,
                first_tx,
                Duration::ZERO,
            );
            shared.take_gen.fetch_add(1, Ordering::AcqRel);
            shared.recording.store(true, Ordering::Release);
        }
        (
            core,
            Rig {
                capture,
                shared,
                events,
                first,
            },
        )
    }

    fn levels(events: &Receiver<Event>) -> Vec<Level> {
        events
            .try_iter()
            .filter_map(|e| match e {
                Event::Level(l) => Some(l),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_cap_formula() {
        assert_eq!(max_frames(48_000), 345_600_000);
        assert_eq!(max_frames(138_888), 999_993_600);
        assert_eq!(max_frames(138_889), 1_000_000_000);
        assert_eq!(max_frames(192_000), 1_000_000_000);
        assert_eq!(max_frames(0), 0);
        // The cap keeps the RIFF size (data + 60) inside hound's u32.
        assert!(max_frames(u32::MAX) * 4 + 60 <= u64::from(u32::MAX));
    }

    #[test]
    fn levels_emit_every_33_ms_of_the_clock() {
        let (mut core, mut rig) = rig::<Cursor<Vec<u8>>>(None, 0);
        rig.capture.push_block(&[0.5; 10], 0);
        core.tick(ms(32));
        assert_eq!(levels(&rig.events), vec![]);
        rig.capture.push_block(&[-1.0; 10], 0);
        core.tick(ms(33));
        assert_eq!(
            levels(&rig.events),
            vec![Level {
                peak: 1.0,
                rms: (0.625f64).sqrt() as f32
            }]
        );
        // A window with no blocks emits nothing; the next window starts at 33 ms.
        core.tick(ms(66));
        assert_eq!(levels(&rig.events), vec![]);
        rig.capture.push_block(&[0.25; 4], 0);
        core.tick(ms(98));
        assert_eq!(levels(&rig.events), vec![]);
        core.tick(ms(99));
        assert_eq!(
            levels(&rig.events),
            vec![Level {
                peak: 0.25,
                rms: 0.25
            }]
        );
    }

    #[test]
    fn idle_audio_is_dropped_and_levels_still_flow() {
        let (mut core, mut rig) = rig::<Cursor<Vec<u8>>>(None, 0);
        rig.shared.recording.store(true, Ordering::Release);
        rig.capture.push_block(&[0.5; 100], 0);
        assert_eq!(core.tick(ms(40)), None);
        assert_eq!(rig.capture.free_audio_slots(), rig.capture.audio_capacity());
        assert_eq!(levels(&rig.events).len(), 1);
        assert!(core.close().is_none());
    }

    #[test]
    fn writes_samples_anchors_and_replaces_non_finite_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        let wav = WavWriter::create(&path, wav_spec(RATE)).unwrap();
        let (mut core, mut rig) = rig(Some(wav), 1_000_000);
        rig.capture.push_block(
            &[0.25, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5],
            1_000,
        );
        assert_eq!(core.tick(ms(5)), None);
        assert_eq!(
            rig.first.try_recv(),
            Ok(Anchor {
                frame: 0,
                host_ns: 1_000
            })
        );
        let data = core.close().unwrap();
        assert_eq!(data.frames, 5);
        assert_eq!(data.ended, Ended::User);
        assert_eq!(
            data.anchors,
            vec![Anchor {
                frame: 0,
                host_ns: 1_000
            }]
        );
        // Closed: a second close has nothing.
        assert!(core.close().is_none());
        let samples: Vec<f32> = hound::WavReader::open(&path)
            .unwrap()
            .into_samples()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples, vec![0.25, 0.0, 0.0, 0.0, -0.5]);
    }

    #[test]
    fn a_copy_between_flushes_is_a_valid_wav_up_to_the_last_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        let file = File::create_new(&path).unwrap();
        let wav =
            WavWriter::new(BufWriter::with_capacity(WRITE_BUFFER, file), wav_spec(RATE)).unwrap();
        let (mut core, mut rig) = rig(Some(wav), max_frames(RATE));
        let ramp: Vec<f32> = (0..1_500).map(|i| i as f32 / 2_000.0).collect();
        rig.capture.push_block(&ramp[..500], 0);
        core.tick(ms(500));
        rig.capture.push_block(&ramp[500..1_000], 0);
        core.tick(ms(1_000)); // flush: 1000 frames
        rig.capture.push_block(&ramp[1_000..], 0);
        core.tick(ms(1_500)); // no flush
        let copy = dir.path().join("copy.wav");
        std::fs::copy(&path, &copy).unwrap();
        let bytes = std::fs::read(&copy).unwrap();
        // 32-bit samples get hound's WAVE_FORMAT_EXTENSIBLE header: 68 bytes, the data size at 64.
        assert_eq!(&bytes[64 - 4..64], b"data");
        let data_size = u32::from_le_bytes(bytes[64..68].try_into().unwrap());
        assert_eq!(bytes.len(), HEADER_LEN + 4_000);
        assert_eq!(data_size, 4_000);
        let reader = hound::WavReader::open(&copy).unwrap();
        assert_eq!(reader.len(), 1_000);
        let samples: Vec<f32> = reader.into_samples().map(Result::unwrap).collect();
        assert_eq!(samples, ramp[..1_000]);

        let data = core.close().unwrap();
        assert_eq!(data.frames, 1_500);
        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec, wav_spec(RATE));
        assert_eq!(&std::fs::read(&path).unwrap()[..4], b"RIFF");
        let samples: Vec<f32> = reader.into_samples().map(Result::unwrap).collect();
        assert_eq!(samples, ramp);
    }

    #[test]
    fn the_cap_stops_at_exactly_max_frames_mid_block() {
        let wav = WavWriter::new(Cursor::new(Vec::new()), wav_spec(RATE)).unwrap();
        let (mut core, mut rig) = rig(Some(wav), 250);
        rig.capture.push_block(&[0.1; 100], 10);
        rig.capture.push_block(&[0.1; 100], 20);
        assert_eq!(core.tick(ms(5)), None);
        rig.capture.push_block(&[0.1; 100], 30);
        rig.capture.push_block(&[0.1; 100], 40);
        assert_eq!(core.tick(ms(10)), Some(1));
        // Anything after the cap is dropped; the stored data is returned once.
        rig.capture.push_block(&[0.1; 100], 50);
        assert_eq!(core.tick(ms(15)), None);
        let data = core.close().unwrap();
        assert_eq!(data.frames, 250);
        assert_eq!(data.ended, Ended::Cap);
        assert!(core.close().is_none());
    }

    #[test]
    fn anchors_past_the_cap_are_dropped() {
        let wav = WavWriter::new(Cursor::new(Vec::new()), wav_spec(1)).unwrap();
        let shared = Arc::new(Shared::default());
        let (mut capture, rings) = Capture::new(1, shared.clone());
        let (ev_tx, _events) = mpsc::channel();
        let mut core = WriterCore::new(rings, ev_tx, Duration::ZERO);
        let (first_tx, _first) = mpsc::sync_channel(1);
        core.open(7, wav, PathBuf::from("x"), 2, first_tx, Duration::ZERO);
        shared.take_gen.fetch_add(1, Ordering::AcqRel);
        shared.recording.store(true, Ordering::Release);
        // At 1 Hz every one-frame block is anchored.
        for t in 0..3 {
            capture.push_block(&[0.0], t);
        }
        assert_eq!(core.tick(ms(1)), Some(7));
        let data = core.close().unwrap();
        assert_eq!(data.frames, 2);
        assert_eq!(
            data.anchors.iter().map(|a| a.frame).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    /// A sink whose writes fail after `ok` bytes.
    struct Failing {
        inner: Cursor<Vec<u8>>,
        ok: usize,
    }

    impl Write for Failing {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.inner.get_ref().len() + buf.len() > self.ok {
                return Err(std::io::Error::other("disk full"));
            }
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Seek for Failing {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    #[test]
    fn a_write_error_ends_the_take() {
        let sink = Failing {
            inner: Cursor::new(Vec::new()),
            ok: HEADER_LEN + 8,
        };
        let wav = WavWriter::new(sink, wav_spec(RATE)).unwrap();
        let (mut core, mut rig) = rig(Some(wav), 1_000);
        rig.capture.push_block(&[0.1; 3], 0);
        assert_eq!(core.tick(ms(5)), Some(1));
        let data = core.close().unwrap();
        assert_eq!(data.frames, 2);
        assert_eq!(data.ended, Ended::WriteError("disk full".to_string()));
    }

    #[test]
    fn abort_drops_the_take_and_returns_its_path() {
        let wav = WavWriter::new(Cursor::new(Vec::new()), wav_spec(RATE)).unwrap();
        let (mut core, _rig) = rig(Some(wav), 1_000);
        assert_eq!(core.abort(), Some(PathBuf::from("x.wav")));
        assert_eq!(core.abort(), None);
        assert!(core.close().is_none());
    }

    #[test]
    fn open_drops_audio_left_from_before_the_take() {
        let (mut core, mut rig) = rig::<Cursor<Vec<u8>>>(None, 0);
        rig.shared.take_gen.fetch_add(1, Ordering::AcqRel);
        rig.shared.recording.store(true, Ordering::Release);
        rig.capture.push_block(&[0.9; 10], 1);
        let wav = WavWriter::new(Cursor::new(Vec::new()), wav_spec(RATE)).unwrap();
        let (first_tx, _first) = mpsc::sync_channel(1);
        core.open(2, wav, PathBuf::from("y"), 100, first_tx, Duration::ZERO);
        let data = core.close().unwrap();
        assert_eq!(data.frames, 0);
        assert!(data.anchors.is_empty());
    }
}

//! The input level: per-block sums from the callback, windows of them on the writer thread.

use std::time::Duration;

/// A window's level, linear amplitude: 0.0 silence, 1.0 full scale (not clamped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub peak: f32,
    pub rms: f32,
}

/// One callback block's level sums.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LevelBlock {
    pub peak: f32,
    pub sumsq: f32,
    pub frames: u32,
}

impl LevelBlock {
    /// The sums of one block of the chosen channel.
    pub fn of(frames: &[f32]) -> LevelBlock {
        let mut peak = 0.0f32;
        let mut sumsq = 0.0f32;
        for &x in frames {
            peak = peak.max(x.abs());
            sumsq += x * x;
        }
        LevelBlock {
            peak,
            sumsq,
            frames: u32::try_from(frames.len()).unwrap_or(u32::MAX),
        }
    }
}

/// How often the writer emits a level event.
pub const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

/// Sums the blocks since the last emit.
#[derive(Debug, Default)]
pub struct LevelAccumulator {
    peak: f32,
    sumsq: f64,
    frames: u64,
}

impl LevelAccumulator {
    pub fn add(&mut self, block: &LevelBlock) {
        self.peak = self.peak.max(block.peak);
        self.sumsq += f64::from(block.sumsq);
        self.frames += u64::from(block.frames);
    }

    /// The window's level, resetting the sums; `None` when no frames arrived since the last call.
    pub fn take(&mut self) -> Option<Level> {
        let acc = std::mem::take(self);
        if acc.frames == 0 {
            return None;
        }
        Some(Level {
            peak: acc.peak,
            rms: (acc.sumsq / acc.frames as f64).sqrt() as f32,
        })
    }
}

/// The writer's time source: monotonic time since some origin. The fake in tests is set by hand.
pub trait Clock: Send + 'static {
    fn now(&self) -> Duration;
}

/// The real clock, from an `Instant` taken at construction.
pub struct SystemClock(std::time::Instant);

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock(std::time::Instant::now())
    }
}

impl Default for SystemClock {
    fn default() -> SystemClock {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_wave_has_equal_peak_and_rms() {
        let block: Vec<f32> = (0..100)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let mut acc = LevelAccumulator::default();
        acc.add(&LevelBlock::of(&block));
        assert_eq!(
            acc.take(),
            Some(Level {
                peak: 0.5,
                rms: 0.5
            })
        );
    }

    #[test]
    fn silence_is_zero() {
        let mut acc = LevelAccumulator::default();
        acc.add(&LevelBlock::of(&[0.0; 64]));
        assert_eq!(
            acc.take(),
            Some(Level {
                peak: 0.0,
                rms: 0.0
            })
        );
    }

    #[test]
    fn blocks_combine_weighted_by_frames() {
        // 100 frames at 1.0 and 300 at 0.0: rms = sqrt(100 / 400) = 0.5, peak 1.0.
        let mut acc = LevelAccumulator::default();
        acc.add(&LevelBlock::of(&[1.0; 100]));
        acc.add(&LevelBlock::of(&[0.0; 300]));
        assert_eq!(
            acc.take(),
            Some(Level {
                peak: 1.0,
                rms: 0.5
            })
        );
    }

    #[test]
    fn no_blocks_emit_nothing_and_take_resets() {
        let mut acc = LevelAccumulator::default();
        assert_eq!(acc.take(), None);
        acc.add(&LevelBlock::of(&[0.25; 8]));
        assert!(acc.take().is_some());
        assert_eq!(acc.take(), None);
        // An empty block adds no frames.
        acc.add(&LevelBlock::of(&[]));
        assert_eq!(acc.take(), None);
    }

    #[test]
    fn peak_is_the_absolute_maximum_and_not_clamped() {
        let block = LevelBlock::of(&[0.1, -1.5, 0.7]);
        assert_eq!(block.peak, 1.5);
        assert_eq!(block.frames, 3);
    }

    #[test]
    fn the_level_interval_is_33_ms() {
        assert_eq!(LEVEL_INTERVAL, Duration::from_millis(33));
    }
}

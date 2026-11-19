//! Clock anchors (frame index to host time) and their interpolation.

/// One callback block's first sample: its index in the WAV (`frame`, 0 is the first frame
/// written) and its capture time on the mach host clock (`host_ns`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub frame: u64,
    pub host_ns: u64,
}

const NS_PER_SEC: i128 = 1_000_000_000;

/// The fractional frame index at which host time `host_ns` falls in a take with `anchors`
/// (strictly increasing in both fields). Piecewise linear between the two anchors that bracket
/// `host_ns`; before the first anchor, after the last, or with a single anchor, extrapolated from
/// the nearest anchor at the nominal rate. `None` without anchors. The result is not
/// range-checked.
pub fn frame_at(anchors: &[Anchor], host_ns: u64, sample_rate: u32) -> Option<f64> {
    let first = anchors.first()?;
    let last = anchors.last()?;
    let t = i128::from(host_ns);
    let nominal = |a: &Anchor| {
        let dt = t - i128::from(a.host_ns);
        exact(a.frame, dt * i128::from(sample_rate), NS_PER_SEC)
    };
    if host_ns <= first.host_ns {
        return Some(nominal(first));
    }
    if host_ns >= last.host_ns {
        return Some(nominal(last));
    }
    // The first anchor strictly after `host_ns`; the one before it is at or before it.
    let upper = anchors.partition_point(|a| a.host_ns <= host_ns);
    let (a, b) = (&anchors[upper - 1], &anchors[upper]);
    let dt = t - i128::from(a.host_ns);
    let dframe = i128::from(b.frame) - i128::from(a.frame);
    let dhost = i128::from(b.host_ns) - i128::from(a.host_ns);
    Some(exact(a.frame, dt * dframe, dhost))
}

/// `base + num / den` with the integer part kept exact before the conversion to `f64`.
fn exact(base: u64, num: i128, den: i128) -> f64 {
    let whole = i128::from(base) + num / den;
    let rest = num % den;
    whole as f64 + rest as f64 / den as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(frame: u64, host_ns: u64) -> Anchor {
        Anchor { frame, host_ns }
    }

    #[test]
    fn no_anchors_is_none() {
        assert_eq!(frame_at(&[], 5, 48_000), None);
    }

    #[test]
    fn exact_at_each_anchor() {
        let anchors = [
            a(0, 1_000),
            a(48_010, 1_000_001_000),
            a(96_000, 2_000_000_000),
        ];
        assert_eq!(frame_at(&anchors, 1_000, 48_000), Some(0.0));
        assert_eq!(frame_at(&anchors, 1_000_001_000, 48_000), Some(48_010.0));
        assert_eq!(frame_at(&anchors, 2_000_000_000, 48_000), Some(96_000.0));
    }

    #[test]
    fn linear_between_two_anchors_uses_their_slope() {
        // 1000 frames over 1e9 ns, not the nominal 48 000.
        let anchors = [a(0, 0), a(1_000, 1_000_000_000)];
        assert_eq!(frame_at(&anchors, 250_000_000, 48_000), Some(250.0));
        assert_eq!(
            frame_at(&anchors, 500_000_001, 48_000),
            Some(500.0 + 1_000.0 / 1e9)
        );
    }

    #[test]
    fn extrapolates_before_the_first_at_the_nominal_rate() {
        let anchors = [a(48_000, 1_000_000_000), a(48_100, 2_000_000_000)];
        assert_eq!(frame_at(&anchors, 500_000_000, 48_000), Some(24_000.0));
        // Before frame 0: negative, not clamped.
        assert_eq!(frame_at(&anchors, 0, 48_000), Some(0.0));
        let early = [a(0, 1_000_000_000)];
        assert_eq!(frame_at(&early, 999_000_000, 48_000), Some(-48.0));
    }

    #[test]
    fn extrapolates_after_the_last_at_the_nominal_rate() {
        // The end segment's slope is 100 frames per second; the nominal rate is used instead.
        let anchors = [a(0, 0), a(100, 1_000_000_000)];
        assert_eq!(frame_at(&anchors, 1_500_000_000, 48_000), Some(24_100.0));
    }

    #[test]
    fn a_single_anchor_uses_the_nominal_rate() {
        let anchors = [a(10, 1_000)];
        assert_eq!(
            frame_at(&anchors, 1_000 + 1_000_000_000, 44_100),
            Some(44_110.0)
        );
        assert_eq!(
            frame_at(&anchors, 1_000 + 1, 44_100),
            Some(10.0 + 44_100.0 / 1e9)
        );
    }

    #[test]
    fn huge_host_times_keep_sub_sample_precision() {
        let base = 1_000_000_000_000_000_000u64; // 1e18 ns
        let anchors = [a(0, base), a(48_000, base + 1_000_000_000)];
        assert_eq!(
            frame_at(&anchors, base + 500_000_000, 48_000),
            Some(24_000.0)
        );
        // 1/48 000 s past the first anchor is exactly frame 1, but 20 833.33… ns is not an
        // integer: 20 833 ns gives frame 0.999984.
        assert_eq!(frame_at(&anchors, base + 20_833, 48_000), Some(0.999984));
        assert_eq!(
            frame_at(&anchors, u64::MAX, 48_000).map(f64::is_finite),
            Some(true)
        );
    }

    #[test]
    fn every_anchor_of_a_jittered_list_maps_to_its_frame() {
        // A deterministic jitter (LCG) of up to ±0.5 ms around 1 s steps at 1e18 ns.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut anchors = Vec::new();
        let mut host = 1_000_000_000_000_000_000u64;
        let mut frame = 0u64;
        for _ in 0..500 {
            anchors.push(a(frame, host));
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let jitter = (state >> 33) % 1_000_000;
            host += 999_500_000 + jitter;
            frame += 48_000 + (state >> 60);
        }
        for (k, anchor) in anchors.iter().enumerate() {
            assert_eq!(
                frame_at(&anchors, anchor.host_ns, 48_000),
                Some(anchor.frame as f64),
                "anchor {k}"
            );
        }
    }
}

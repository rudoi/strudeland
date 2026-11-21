//! Every tuning constant of the crate, each with its source. "provisional (#70)" values were
//! chosen for synthetic audio and are retuned on real memos and nowhere else.

/// The analysis sample rate in Hz: enough for a voice's fundamental, cheap for pYIN (proposal).
pub const ANALYSIS_RATE: u32 = 16_000;
/// The longest memo `Span::Whole` reads, in seconds (proposal: long-memo cap).
pub const MAX_WHOLE_SECONDS: f64 = 600.0;

/// The pitch frame length in samples at the analysis rate: 64 ms (proposal).
pub const FRAME: usize = 1024;
/// The hop in samples at the analysis rate: 10 ms (proposal).
pub const HOP: usize = 160;
/// The lowest tracked fundamental in Hz (proposal).
pub const FMIN: f64 = 60.0;
/// The highest tracked fundamental in Hz (proposal).
pub const FMAX: f64 = 1100.0;
/// The width of a pitch bin in cents (proposal).
pub const BIN_CENTS: f64 = 20.0;
/// The number of pitch bins: `ceil(1200 * log2(FMAX / FMIN) / BIN_CENTS)` (proposal).
pub const N_BINS: usize = 252;
/// The beta prior's first parameter over the YIN thresholds (librosa's default).
pub const BETA_A: f64 = 2.0;
/// The beta prior's second parameter over the YIN thresholds (librosa's default).
pub const BETA_B: f64 = 18.0;
/// The number of YIN thresholds the prior is sampled at (librosa's default).
pub const N_THRESHOLDS: usize = 100;
/// The fastest pitch movement the HMM allows, in octaves per second (proposal).
pub const MAX_TRANSITION_OCT_PER_SEC: f64 = 35.0;
/// The bins a pitch may move per frame, either way, under the transition limit:
/// `MAX_TRANSITION_OCT_PER_SEC * 1200 * HOP / ANALYSIS_RATE / BIN_CENTS` = 21 (proposal).
pub const BAND_BINS: usize =
    (MAX_TRANSITION_OCT_PER_SEC * 1200.0 * HOP as f64 / ANALYSIS_RATE as f64 / BIN_CENTS) as usize;
/// The probability of switching between voiced and unvoiced per frame (proposal).
pub const VOICING_SWITCH: f64 = 0.01;
/// The weight of the global CMND minimum for thresholds no trough falls below (Mauch and Dixon
/// 2014's `p_a`, librosa's `no_trough_prob`).
pub const NO_TROUGH_PROB: f64 = 0.01;
/// Provisional minimum probability for a pitched frame to support a note and count toward its
/// confidence.
pub const MIN_VOICED_PROB_EMIT: f32 = 0.5;

/// The attack of each voiced run that tuning and the note pitch skip, in milliseconds (proposal).
pub const TUNING_SKIP_MS: u32 = 30;
/// A frame is stable when the smoothed pitch moves less than this many semitones (proposal).
pub const TUNING_STABLE_ST: f32 = 0.3;
/// The window over which that movement is measured, in milliseconds (proposal).
pub const TUNING_STABLE_MS: u32 = 50;
/// Tuning compares the pitch's centred median over this window with 12-TET, so a vibrato cycle
/// (5 Hz and faster) is centred first; provisional (#70).
pub const TUNING_CENTRE_MS: u32 = 200;
/// A tuning offset beyond this many cents makes the nearest semitone ambiguous (proposal).
pub const TUNING_WARN_CENTS: f32 = 40.0;
/// The median filter length over the MIDI track: 50 ms; provisional (#70).
pub const MEDIAN_FRAMES: usize = 5;

/// Unvoiced gaps shorter than this are bridged, in milliseconds (proposal).
pub const GAP_BRIDGE_MS: u32 = 40;
/// A pitch change is a note boundary beyond this many semitones (proposal).
pub const PITCH_CHANGE_ST: f32 = 0.6;
/// A pitch change counts when held at least this long, in milliseconds (proposal).
pub const PITCH_CHANGE_MS: u32 = 50;
/// A pitch-change boundary moves back to an onset peak at most this many milliseconds before
/// it (the change is seen only after it has held, later than its attack); provisional (#70).
pub const PITCH_SNAP_MS: u32 = 80;
/// A piece shorter than this that glides into the next note is merged into it (an attack glide
/// or a portamento cut off by a pitch change or an onset), in milliseconds; provisional (#70).
pub const GLIDE_MAX_MS: u32 = 100;
/// A glide passes through: its median lies at least this fraction of its travel in from either
/// end; provisional (#70).
pub const GLIDE_INSIDE: f32 = 0.1;
/// Adjacent pieces split by a pitch change whose pitches lie less than this many semitones apart
/// are one drifting note when the boundary is not re-articulated; provisional (#70).
pub const DRIFT_MERGE_ST: f32 = 1.0;
/// The shorter of those pieces is under this many milliseconds; provisional (#70).
pub const DRIFT_MAX_MS: u32 = 200;
/// An onset peak at least this strong within a frame of the boundary is an attack, so those
/// pieces are two notes; above a hummed wobble's peaks (at most 0.57); provisional (#70).
pub const DRIFT_ONSET_VETO: f32 = 0.6;
/// Notes shorter than this merge or drop, in milliseconds (proposal).
pub const MIN_NOTE_MS: u32 = 60;

/// The onset STFT window in samples (32 ms); provisional (#70).
pub const ONSET_WIN: usize = 512;
/// The onset STFT hop in samples, the pitch hop (provisional (#70)).
pub const ONSET_HOP: usize = HOP;
/// The band the spectral flux covers, in Hz, over bins of `log(1 + 100 * mag)`; provisional (#70).
pub const ONSET_BAND_HZ: (f32, f32) = (80.0, 4000.0);
/// The flux's magnitudes are compressed as `log(1 + ONSET_LOG_GAIN * mag)`; provisional (#70).
pub const ONSET_LOG_GAIN: f64 = 100.0;
/// Each flux bin is compared with the previous frame's maximum over this many bins either side
/// (the SuperFlux maximum filter, Böck and Widmer, DAFx 2013); provisional (#70).
pub const ONSET_MAX_FILTER_BINS: usize = 1;
/// The flux is scaled by the energy this many ms after the frame over the energy as long before
/// (capped at 1), so a decay's splatter is not an attack; provisional (#70).
pub const ONSET_DECAY_LOOK_MS: u32 = 20;
/// The RMS rise's weight in the onset strength; provisional (#70).
pub const ONSET_RMS_WEIGHT: f32 = 0.5;
/// Flux and RMS rise are each divided by this percentile of their values; provisional (#70).
pub const ONSET_NORM_PERCENTILE: f64 = 0.95;
/// After that division each is clamped to this before they are summed; provisional (#70).
pub const ONSET_NORM_CLAMP: f64 = 1.5;
/// The divisor is at least this fraction of the component's maximum, so a sparse component (a
/// percentile at the noise level) does not saturate its noise; provisional (#70).
pub const ONSET_NORM_FLOOR: f64 = 0.2;
/// The adaptive threshold's window, each side, in milliseconds; provisional (#70).
pub const ONSET_THRESH_WINDOW_MS: u32 = 250;
/// The adaptive threshold's offset on the 0..1 strength scale; provisional (#70).
pub const ONSET_THRESH_DELTA: f32 = 0.12;
/// A peak must be the maximum within this many milliseconds each side; provisional (#70).
pub const ONSET_MIN_DISTANCE_MS: u32 = 50;
/// An onset splits a stable voiced region only at or above this strength; provisional (#70).
pub const ONSET_SPLIT_MIN: f32 = 0.25;
/// An onset splits a held pitch only at a re-articulation: the lowest `REARTICULATION_WIN` RMS within
/// `REARTICULATION_LOOK_MS` before it (to one frame after) is at most this fraction of the
/// louder level on the quieter side (the highest RMS within `REARTICULATION_LEVEL_MS` before and
/// after that valley, inside the note), so a hummed note's loudness wobble does not re-attack
/// it; provisional (#70).
pub const REARTICULATION_DIP: f32 = 0.77;
/// The window of the RMS the re-articulation dip is measured on, in samples (20 ms: two periods
/// of a low hummed voice, short enough that the brief dip of a re-sung note is not smoothed
/// away); provisional (#70).
pub const REARTICULATION_WIN: usize = 320;
/// How far before an onset the re-articulation valley is looked for, in milliseconds;
/// provisional (#70).
pub const REARTICULATION_LOOK_MS: u32 = 60;
/// The window each side of the valley the note's level is taken over, in milliseconds;
/// provisional (#70).
pub const REARTICULATION_LEVEL_MS: u32 = 30;
/// A slow re-articulation (a re-sung note that fades out and in over a longer time): the dip
/// over `REARTICULATION_SLOW_MS` each side is at most this; provisional (#70).
pub const REARTICULATION_SLOW_DIP: f32 = 0.5;
/// The window each side of the valley the slow dip's level is taken over, in milliseconds;
/// provisional (#70).
pub const REARTICULATION_SLOW_MS: u32 = 150;

/// The phase search window around each grid step, in milliseconds (proposal).
pub const PHASE_WINDOW_MS: u32 = 25;
/// The candidate phase spacing over one beat, in milliseconds; provisional (#70).
pub const PHASE_STEP_MS: u32 = 5;
/// The slowest inferred tempo in bpm (proposal).
pub const TEMPO_MIN: f64 = 60.0;
/// The fastest inferred tempo in bpm (proposal).
pub const TEMPO_MAX: f64 = 180.0;
/// How far an inferred tempo may differ from the sidecar's and be kept: 4% (proposal).
pub const TEMPO_AGREE: f64 = 0.04;
/// Downbeats (every `beats_per_bar` beats) count this much on top of the beats when phases are
/// scored, so the bar's structure settles a phase the beats alone leave open; provisional (#70).
pub const PHASE_DOWNBEAT_WEIGHT: f64 = 3.0;
/// At most this many candidate phases over one bar; a very long bar spaces them wider than
/// `PHASE_STEP_MS` (a guard, not a tuning value).
pub const PHASE_MAX_CANDIDATES: usize = 20_000;
/// The fewest onsets tempo and phase inference needs (proposal).
pub const MIN_ONSETS: usize = 4;
/// An onset is strong at or above this fraction of the largest peak; provisional (#70).
pub const STRONG_ONSET: f32 = 0.5;
/// The onset strength is smoothed by a Gaussian of this standard deviation, in milliseconds,
/// before its autocorrelation, so timing jitter does not split an ACF peak; provisional (#70).
pub const TEMPO_SMOOTH_MS: u32 = 20;
/// A beat lag scores its ACF plus, for each `(fraction, weight)`, the weight times the ACF at
/// that fraction of the lag (the sixteenth pulses under the beat), so a melody on a sixteenth grid
/// locks onto four sixteenths rather than three or six; provisional (#70).
pub const TEMPO_SUBDIV_WEIGHTS: [(f64, f64); 3] = [(0.25, 0.5), (0.5, 0.5), (0.75, 0.5)];
/// The coarse beat lag is refined at these multiples of itself (an ACF peak many beats out
/// pins the period more finely than one beat does); provisional (#70).
pub const TEMPO_REFINE_BEATS: [u32; 2] = [4, 16];
/// The centre of the log-normal tempo prior the beat score is weighted by, in bpm (librosa's
/// `beat.tempo` `start_bpm`); provisional (#70).
pub const TEMPO_PRIOR_BPM: f64 = 120.0;
/// The tempo prior's standard deviation in octaves (librosa's `beat.tempo` `std_bpm`);
/// provisional (#70).
pub const TEMPO_PRIOR_OCTAVES: f64 = 0.7;

/// Notation renders at most this many bars; notes beyond it give no notation and a warning (a
/// guard against nonsense offsets, not a tuning value: ten minutes at 400 bpm in 1/4 is 4000).
pub const MAX_NOTATION_BARS: u32 = 100_000;

/// The fewest notes key detection needs (proposal).
pub const KEY_MIN_NOTES: usize = 6;
/// A note attacked on a downbeat adds this many mean note weights to the key histogram (metre as
/// a key cue, Temperley 2001); provisional (#70).
pub const KEY_DOWNBEAT_BONUS: f64 = 1.0;
/// The final note adds this many mean note weights to the key histogram (melodies end on their
/// tonic); provisional (#70).
pub const KEY_FINAL_BONUS: f64 = 6.0;
/// The smallest correlation margin between the best two keys; provisional (#70).
pub const KEY_MARGIN_MIN: f32 = 0.05;

/// A note more than this many cents from its semitone (after tuning) names the neighbouring
/// semitone it leans toward as its `alternative`; provisional (#70).
pub const AMBIGUOUS_CENTS: f32 = 30.0;
/// Notes below this confidence are flagged `?` in the human table; provisional (#70).
pub const LOW_CONFIDENCE: f32 = 0.5;
/// Confidence is multiplied by `1 - min(|snapErrorMs|, SNAP_PENALTY_MS) / 100`; provisional (#70).
pub const SNAP_PENALTY_MS: f32 = 50.0;
/// The fewest gaps between onsets the rhythm fit of a grid division needs; provisional
/// (#70).
pub const GRID_FIT_MIN_PAIRS: usize = 8;
/// A `--grid` division misfits the rhythm when its rhythm fit is at least this (chance is 0.25:
/// the gaps between onsets fall between its steps); only then is another suggested; provisional
/// (#70).
pub const GRID_MISFIT: f64 = 0.32;
/// Another `--grid` division is suggested when its rhythm fit is at most this fraction of the
/// requested division's (and below chance, 0.25); provisional (#70).
pub const GRID_SUGGEST_RATIO: f64 = 0.75;
/// A shift is suggested when it brings the timing spread to at most this fraction of the
/// unshifted one; provisional (#70).
pub const SHIFT_SUGGEST_RATIO: f64 = 0.7;
/// A shift is suggested only when the notes' phases within the step agree at least this much
/// (the length of their confidence-weighted mean on the circle, 0 to 1); provisional (#70).
pub const SHIFT_MIN_CONSISTENCY: f64 = 0.6;
/// The fewest notes a shift suggestion needs; provisional (#70).
pub const SHIFT_MIN_NOTES: usize = 8;
/// A suggested shift is at least this many milliseconds; provisional (#70).
pub const SHIFT_MIN_MS: f64 = 20.0;
/// A timing spread above this many milliseconds warns; provisional (#70).
pub const TIMING_SPREAD_WARN_MS: f32 = 60.0;

/// The voiced component's exponent in the confidence's weighted geometric mean; provisional
/// (#70).
pub const CONF_W_VOICED: f32 = 0.4;
/// The stability component's exponent; provisional (#70).
pub const CONF_W_STABILITY: f32 = 0.3;
/// The fit component's exponent; provisional (#70).
pub const CONF_W_FIT: f32 = 0.3;
/// `stability = 1 - min(MAD / STABILITY_FULL_ST, 1)`, the MAD in semitones; provisional (#70).
pub const STABILITY_FULL_ST: f32 = 0.5;

/// The compute budget in seconds per minute of audio, in release (proposal).
pub const PERF_BUDGET_SECONDS_PER_MINUTE: f64 = 1.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_bins_is_252() {
        let derived = (1200.0 * (FMAX / FMIN).log2() / BIN_CENTS).ceil() as usize;
        assert_eq!(derived, 252);
        assert_eq!(N_BINS, 252);
    }

    #[test]
    fn band_is_21_bins() {
        // 35 octaves per second over a 10 ms hop is 0.35 octave, 420 cents, 21 bins of 20 cents
        let cents_per_frame =
            MAX_TRANSITION_OCT_PER_SEC * 1200.0 * HOP as f64 / f64::from(ANALYSIS_RATE);
        assert_eq!(cents_per_frame, 420.0);
        assert_eq!(BAND_BINS, 21);
        assert_eq!(BAND_BINS, (cents_per_frame / BIN_CENTS) as usize);
        const { assert!(2 * BAND_BINS + 1 < N_BINS) };
    }

    #[test]
    fn hop_is_10_ms_at_analysis_rate() {
        assert_eq!(HOP as u32 * 1000 / ANALYSIS_RATE, 10);
        assert_eq!(HOP as f64 / f64::from(ANALYSIS_RATE), 0.01);
        assert_eq!(ONSET_HOP, HOP);
    }

    #[test]
    fn confidence_weights_sum_to_1() {
        let sum = CONF_W_VOICED + CONF_W_STABILITY + CONF_W_FIT;
        assert!((sum - 1.0).abs() < 1e-6, "weights sum to {sum}");
    }
}

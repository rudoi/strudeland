/**
 * Every render gate constant, in one place. Tune only by editing here. Start values come from the
 * earlier render harness and its calibration. A `calibrated: see …` line names the probe evidence
 * file and field that set the value (plans/0008-render-harness/evidence/calibration.md). Re-run
 * the probes after a Playwright bump or a Strudel sync. Pure.
 */

// ---- lookback and chunks

// lookback.ts:20 (K_MINUS_60_DB: roomsize is the -60 dB point)
// calibrated: no probe sets it (start value kept)
export const LOOKBACK_K = 1.0;
// lookback.ts:86 (margin after the additive sum)
// calibrated: no probe sets it (start value kept)
export const LOOKBACK_MARGIN_SEC = 0.5;
// lookback.ts:63 (feedback 1.0 would never decay)
// calibrated: no probe sets it (start value kept)
export const DELAY_FEEDBACK_CEIL = 0.98;
// spec and earlier render harness's chunkBars default
// calibrated: no probe sets it (start value kept)
export const CHUNK_MIN_BARS = 8;
// lookback.ts:156 (chunk 1.5x the lookback bounds the overhead)
// calibrated: no probe sets it (start value kept)
export const CHUNK_LOOKBACK_RATIO = 1.5;
// planChunks.ts:56 (bars either side a seam may move)
// calibrated: no probe sets it (start value kept)
export const SEAM_SEARCH_BARS = 2;
// pipeline.ts:232 (seamBars default 1)
// calibrated: no probe sets it (start value kept)
export const SEAM_OVERLAP_BARS = 1;

// ---- seam gate

// oracle.ts:71-73 (Hz; differs from dsp.ts BANDS on purpose)
// calibrated: no probe sets it (start value kept)
export const SEAM_BANDS: readonly (readonly [name: string, loHz: number, hiHz: number])[] = [
  ['sub', 20, 130],
  ['mid', 130, 2000],
  ['high', 2000, 16000],
];
// oracle.ts:83
// calibrated: no probe sets it (start value kept)
export const SEAM_WINDOW_SEC = 0.1;
// oracle.ts:84 (linear RMS)
// calibrated: no probe sets it (start value kept)
export const SEAM_FLOOR = 1e-5;
// pipeline.ts:26; twin-render-null.json (worst p95 0.317 x 1.5)
// was 0.48
// calibrated: see plans/0008-render-harness/evidence/seam-stats.json decision.SEAM_TOLERANCE_DB (1.25 x pooled sub-band max 2.48)
export const SEAM_TOLERANCE_DB = 3.1;
// pipeline.ts:33; oracle-calibration.md (clean 0.01-0.42, broken 43-78)
// unchanged
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json seam.choice (T 3: 0 false alarms, recall 1)
export const SEAM_FAIL_DB = 3;

// ---- twin gate

// measure-twin-null.probe.ts:17
// calibrated: no probe sets it (start value kept)
export const TWIN_WINDOW_SEC = 0.1;
// measure-twin-null.probe.ts:96
// calibrated: no probe sets it (start value kept)
export const TWIN_FLOOR = 1e-7;
// twin-render-null.json; start = SEAM_TOLERANCE_DB
// was 0.35 (0008 calibration)
// calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.TWIN_P95_DB (1.25 x worst p95 9e-7, rounded up to at least one 0.05 step)
export const TWIN_P95_DB = 0.05;
// pipeline.ts:35-36 (twins differed by up to 1.9 dB in earlier calibration) + headroom
// was 6.2 (0008 calibration)
// calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.TWIN_MAX_DB (1.25 x worst band max 0.003, rounded up to at least one 0.1 step)
export const TWIN_MAX_DB = 0.1;
// twin-render-null.json (sampleResidualDb -15.38 and -23.75 on two earlier fixtures) + headroom
// was -18 (0008 calibration)
// set: a deliberate floor, not the probe's decision.TWIN_RESIDUAL_DB (-77 = worst -80.28 + 3). The
// worst mix-mode result depends on load (alone -139.8 dB), so 3 dB of headroom would make
// render:check flaky. -60 still fails every voice-level difference the 0008
// probe saw (-21 to -48). Evidence: plans/0047-deterministic-voices/evidence/calibration.md
export const TWIN_RESIDUAL_DB = -60;

// ---- stems gate

// stems.ts:156
// calibrated: no probe sets it (start value kept)
export const STEMS_MAX_ATTACK_SEC = 0.05;
// stems.ts:157
// calibrated: no probe sets it (start value kept)
export const STEMS_MIN_GAIN = 0.05;
// stems.ts:169
// calibrated: no probe sets it (start value kept)
export const STEMS_PRE_SEC = 0.01;
// stems.ts:169, pipeline.ts:20
// calibrated: no probe sets it (start value kept)
export const STEMS_EDGE_SEC = 0.04;
// pipeline.ts:310 (hopSec: 0.005)
// calibrated: no probe sets it (start value kept)
export const STEMS_HOP_SEC = 0.005;
// pipeline.ts:21; oracle-calibration.md, stems-v2-calibration.json
// was 12
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json stems.choice (8: 0 false alarms, recall 1; 6 has 1)
export const STEMS_MISS_DB = 8;
// stems.ts:15 default; spec
// calibrated: no probe sets it (start value kept)
export const STEMS_MAX_PAIRS = 16;

// ---- bass gate

// oracle.ts:125,134
// calibrated: no probe sets it (start value kept)
export const BASS_MAX_HZ = 130;
// oracle.ts:125,134
// calibrated: no probe sets it (start value kept)
export const BASS_MIN_HZ = 20;
// oracle.ts:132
// calibrated: no probe sets it (start value kept)
export const BASS_MAX_ATTACK_SEC = 0.3;
// oracle.ts:126
// calibrated: no probe sets it (start value kept)
export const BASS_MIN_GAIN = 0.05;
// oracle.ts:147
// calibrated: no probe sets it (start value kept)
export const BASS_WINDOW_SEC = 0.12;
// oracle.ts:156
// calibrated: no probe sets it (start value kept)
export const BASS_PAD_SEC = 0.05;
// oracle.ts:158
// calibrated: no probe sets it (start value kept)
export const BASS_BAND_RATIO = 1.06;
// oracle.ts:160
// calibrated: no probe sets it (start value kept)
export const BASS_GAIN_FLOOR = 1e-3;
// oracle.ts:168; oracle-calibration.md (41% recall, 2 clean false alarms)
// was 12
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json bass.choice (16: 0 false alarms in 1374 notes, recall 1)
export const BASS_MISS_DB = 16;
// new: start value, uncalibrated
// unchanged
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json bass.rule.chosen.clusterMin
export const BASS_CLUSTER_MIN = 3;
// new: start value, uncalibrated
// unchanged
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json bass.rule.chosen.clusterSpanBars
export const BASS_CLUSTER_SPAN_BARS = 4;
// new: start value, uncalibrated (earlier false alarms sit at -12.6 to -13.6 dB,
// oracle-calibration.md)
// was 15
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json bass.rule.chosen.clusterDeepDb (swept from BASS_MISS_DB 16)
export const BASS_CLUSTER_DEEP_DB = 16;
// new: start value, uncalibrated
// unchanged
// calibrated: see plans/0008-render-harness/evidence/mutation-sweep.json bass.rule.chosen.fractionMax
export const BASS_FRACTION_MAX = 0.2;
// new: start value, uncalibrated
// calibrated: no probe sets it (start value kept)
export const BASS_FRACTION_MIN_NOTES = 10;
// new: start value, uncalibrated
// calibrated: no probe sets it (start value kept)
export const BASS_MIN_NOTES = 5;

// ---- mix gate

// verify.ts:77
// calibrated: no probe sets it (start value kept)
export const OPEN_TOL_SEC = 0.01;
// verify.ts:99
// calibrated: no probe sets it (start value kept)
export const OPEN_LATE_TOL_SEC = 0.25;
// verify.ts:31
// calibrated: no probe sets it (start value kept)
export const OPEN_THRESHOLD_DB = -40;
// verify.ts:48 (x threshold)
// calibrated: no probe sets it (start value kept)
export const OPEN_PERCUSSIVE_HEADROOM = 10;
// analyze.ts:114
// calibrated: no probe sets it (start value kept)
export const CLICK_WINDOW_SEC = 0.003;
// verify.ts:88
// calibrated: no probe sets it (start value kept)
export const CLICK_MAX_RATIO = 30;
// verify.ts:94-95
// calibrated: no probe sets it (start value kept)
export const CLICK_NEIGHBOR_FACTOR = 2;
// verify.ts:94-95
// calibrated: no probe sets it (start value kept)
export const CLICK_NEIGHBOR_BARS = 2;
// verify.ts:105 (clipFraction counts |x| > 1.0)
// calibrated: no probe sets it (start value kept)
export const CLIP_LIMIT = 1.0;
// spec (exact frame count)
// calibrated: no probe sets it (start value kept)
export const FRAME_TOL = 0;
// spec; the twin probe sets it from evidence (a boolean, not a number)
// was true
// calibrated: see plans/0047-deterministic-voices/evidence/twin-null-final.json decision.TWIN_BITEXACT_HOLDS (12 of 23 non-stochastic fixtures differ, all at
// float level: worst residual -80 dB, band max 0.003 dB)
export const TWIN_BITEXACT_HOLDS = false;

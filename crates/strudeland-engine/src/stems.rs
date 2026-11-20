//! Optional offline taps on the ordinary stereo graph. The catalog spans every chunk
//! (including lookback), while voice and orbit indices belong to a single job.

use std::collections::BTreeSet;

use crate::controls::{js_to_string, prepare_value};
use crate::job::{EngineJob, JobError, Value, orbit_number};
use crate::js::number_to_string;
use crate::orbit::{Orbit, sum_stereo};
use crate::panner::{Stereo, pan_to_target_value};
use crate::render::{Engine, Slot, zeroed};
use crate::voice::{VoiceOutput, sound_name};
use crate::{QUANTUM, Rendered};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StemGrouping {
    Orbit,
    Voice,
    Instrument,
}

/// Orbit strings are the engine's canonical JS numeric keys. The typed variants keep arbitrary
/// user labels distinct from an unlabeled part and from shared effects.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StemId {
    Orbit(String),
    Voice(String),
    Unlabeled,
    Instrument(String),
    Effects(String),
}

#[derive(Debug, Clone)]
pub struct StemLayout {
    grouping: StemGrouping,
    ids: Vec<StemId>,
}

/// Visit each scheduled value once. The intern table may contain values no hap plays.
fn scheduled_groups(
    job: &EngineJob,
    grouping: StemGrouping,
) -> impl Iterator<Item = (usize, StemId, String)> + '_ {
    let mut seen = vec![false; job.values.len()];
    job.pauses
        .iter()
        .flat_map(|p| &p.haps)
        .filter_map(move |h| {
            let index = h.v as usize;
            let value = job.values.get(index)?;
            if std::mem::replace(&mut seen[index], true) {
                return None;
            }
            let value = prepare_value(value, h.duration)?;
            let orbit = number_to_string(orbit_number(&value));
            let id = match grouping {
                StemGrouping::Orbit => StemId::Orbit(orbit.clone()),
                StemGrouping::Voice => value
                    .get("label")
                    .filter(|v| !matches!(v, Value::Null))
                    .map(js_to_string)
                    .filter(|s| !s.is_empty())
                    .map_or(StemId::Unlabeled, StemId::Voice),
                StemGrouping::Instrument => StemId::Instrument(sound_name(value.get("s"))),
            };
            Some((index, id, orbit))
        })
}

impl StemLayout {
    /// Deterministic output order, independent of chunk order and worker count.
    pub fn for_jobs(jobs: &[EngineJob], grouping: StemGrouping) -> Self {
        let mut ids = BTreeSet::new();
        for job in jobs {
            for (_, id, orbit) in scheduled_groups(job, grouping) {
                ids.insert(id);
                if grouping != StemGrouping::Orbit {
                    ids.insert(StemId::Effects(orbit));
                }
            }
        }
        Self {
            grouping,
            ids: ids.into_iter().collect(),
        }
    }

    pub fn ids(&self) -> &[StemId] {
        &self.ids
    }
}

/// Render the regular mix, then one stereo pair per catalog entry. A chunk retains silent
/// channels for groups elsewhere in the render. Callers budget the complete layout before
/// dispatching jobs to workers.
pub fn render_with_stems(job: &EngineJob, layout: &StemLayout) -> Result<Rendered, JobError> {
    let (result, mix_overflowed) = Engine::with_stems(job, layout)?.run_observing_overflow();
    if mix_overflowed {
        return Err(JobError::new("non-finite stem reference mix"));
    }
    if result.channels[2..]
        .iter()
        .flatten()
        .any(|x| !x.is_finite())
    {
        return Err(JobError::new("non-finite stem output"));
    }
    Ok(result)
}

struct DirectSum {
    samples: Stereo,
    silent: [bool; 2],
}

impl Default for DirectSum {
    fn default() -> Self {
        Self {
            samples: [[0.0; QUANTUM]; 2],
            silent: [true; 2],
        }
    }
}

/// An exact signed sum of finite f32 terms in units of the smallest f32 subnormal (2^-149).
/// A term contributes at most 24 significand bits shifted left by at most 63 inside a bin.
/// Each of the at most 2^32 scheduled value indices can contribute two routes and two terms
/// per destination, so even all their contributions to one bin fit comfortably in an i128.
#[derive(Clone, Copy, Default)]
struct Residual([i128; 4]);

impl Residual {
    fn add(&mut self, sample: f32) {
        if !sample.is_finite() {
            // The owning mix or direct stem already records the export failure.
            return;
        }
        let bits = sample.to_bits();
        let exponent = (bits >> 23) & 0xff;
        let fraction = bits & 0x007f_ffff;
        let significand = i128::from(if exponent == 0 {
            fraction
        } else {
            fraction | (1 << 23)
        });
        let shift = exponent.saturating_sub(1);
        let magnitude = significand << (shift % 64);
        self.0[(shift / 64) as usize] += if bits >> 31 == 0 {
            magnitude
        } else {
            -magnitude
        };
    }

    fn sample(mut self) -> f32 {
        // Carry before rounding. Balanced digits keep each lower bin below half a unit of
        // the next bin, so conversion cannot lose a small residual to cancellation.
        const RADIX: i128 = 1 << 64;
        for i in 0..self.0.len() - 1 {
            let carry = (self.0[i] + RADIX / 2) >> 64;
            self.0[i] -= carry << 64;
            self.0[i + 1] += carry;
        }
        let units = self
            .0
            .iter()
            .rev()
            .fold(0.0, |sum, &bin| sum * RADIX as f64 + bin as f64);
        (units * f64::from(f32::from_bits(1))) as f32
    }
}

pub(crate) struct StemCapture {
    pub(crate) channels: Vec<Vec<f32>>,
    /// Orbit stem (orbit mode) or shared-effects stem (voice and instrument modes).
    orbit_stem: Vec<Option<usize>>,
    value_stem: Vec<Option<usize>>,
    /// The current quantum's (voice index, stem index) in connection order for each orbit.
    voices: Vec<Vec<(usize, usize)>>,
    /// Reused within each orbit. Only groups that actually sounded are visited and cleared.
    direct: Vec<DirectSum>,
    active: Vec<usize>,
    /// Ordinary merger output immediately before the next orbit, for rounding attribution.
    previous_mix: Stereo,
}

impl StemCapture {
    pub(crate) fn new(job: &EngineJob, layout: &StemLayout) -> Result<Self, JobError> {
        let groups_direct = layout.grouping != StemGrouping::Orbit;
        let mut value_stem = if groups_direct {
            vec![None; job.values.len()]
        } else {
            Vec::new()
        };
        for (index, id, orbit) in scheduled_groups(job, layout.grouping) {
            let required = |id: &StemId| {
                layout
                    .ids
                    .binary_search(id)
                    .map_err(|_| JobError::new(format!("missing stem in render layout: {id:?}")))
            };
            let group = required(&id)?;
            if groups_direct {
                value_stem[index] = Some(group);
                required(&StemId::Effects(orbit))?;
            }
        }
        let orbit_stem = job
            .orbits
            .iter()
            .map(|o| {
                let key = number_to_string(o.orbit);
                let id = if groups_direct {
                    StemId::Effects(key)
                } else {
                    StemId::Orbit(key)
                };
                layout.ids.binary_search(&id).ok()
            })
            .collect();
        let frames = usize::try_from(job.frames).map_err(|_| JobError::new("frames: too large"))?;
        let count = layout
            .ids
            .len()
            .checked_mul(2)
            .ok_or_else(|| JobError::new("too many stems"))?;
        let channels = (0..count)
            .map(|_| zeroed(frames))
            .collect::<Result<_, _>>()?;
        let direct = if groups_direct {
            (0..layout.ids.len())
                .map(|_| DirectSum::default())
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            channels,
            orbit_stem,
            value_stem,
            direct,
            voices: if groups_direct {
                vec![Vec::new(); job.orbits.len()]
            } else {
                Vec::new()
            },
            active: Vec::new(),
            previous_mix: [[0.0; QUANTUM]; 2],
        })
    }

    pub(crate) fn groups_direct(&self) -> bool {
        !self.direct.is_empty()
    }

    pub(crate) fn begin(&mut self, slots: &[Slot]) {
        self.previous_mix = [[0.0; QUANTUM]; 2];
        if !self.groups_direct() {
            return;
        }
        for voices in &mut self.voices {
            voices.clear();
        }
        for (i, slot) in slots.iter().enumerate() {
            if let Some(group) = self.value_stem[slot.value] {
                self.voices[slot.orbit].push((i, group));
            }
        }
    }

    /// Tap the same splitter outputs and merger connections as the mix, including custom
    /// channel mappings. The final partial quantum is truncated for every output alike.
    pub(crate) fn record(
        &mut self,
        start: usize,
        k: usize,
        orbit: &Orbit,
        outputs: &[VoiceOutput],
        gains: Option<&[f32; QUANTUM]>,
        mix: &[[f32; QUANTUM]],
    ) {
        let Some(orbit_stem) = self.orbit_stem[k] else {
            return;
        };
        let Some(gains) = gains else {
            self.record_orbit(start, orbit_stem, orbit, mix);
            return;
        };

        for &(i, group) in &self.voices[k] {
            let input = outputs[i].to_orbit();
            if input.silent {
                continue;
            }
            let sum = &mut self.direct[group];
            if sum.silent[0] {
                self.active.push(group);
            }
            sum_stereo(&mut sum.samples, &mut sum.silent, input);
        }
        // Start with what this orbit actually added to the stereo mixer. Subtract what each
        // direct group actually adds to its exported stem. Keeping the individual terms
        // retains mixer rounding even when much louder signals cancel across orbits.
        let mut residual = [[Residual::default(); QUANTUM]; 2];
        for ((delta, old), new) in residual.iter_mut().zip(&mut self.previous_mix).zip(mix) {
            for ((d, a), b) in delta.iter_mut().zip(&mut *old).zip(new) {
                d.add(*b);
                d.add(-*a);
            }
            *old = *new;
        }
        for group in self.active.drain(..) {
            let sum = &mut self.direct[group];
            for ch in &mut sum.samples {
                for (x, gain) in ch.iter_mut().zip(gains) {
                    *x *= gain;
                }
            }
            let mut post = [[0.0; QUANTUM]; 2];
            pan_to_target_value(&sum.samples, 0.0, &mut post);
            add_direct(
                &mut self.channels,
                group,
                start,
                orbit,
                &post,
                &mut residual,
            );
            sum.silent = [true; 2]; // sum_stereo copies the first non-silent connection.
        }
        // Residuals are already in destination channel order.
        for (ch, values) in residual.iter().enumerate() {
            for (d, x) in self.channels[2 * orbit_stem + ch][start..]
                .iter_mut()
                .zip(values)
            {
                *d += x.sample();
            }
        }
    }

    fn record_orbit(&mut self, start: usize, group: usize, orbit: &Orbit, mix: &[[f32; QUANTUM]]) {
        let (finished, _) = orbit.output();
        for (ch, current) in mix.iter().enumerate() {
            let previous = std::mem::replace(&mut self.previous_mix[ch], *current);
            let mut inputs = orbit.routes().iter().filter(|&&(_, merge)| merge == ch);
            let Some(&(split, _)) = inputs.next() else {
                continue;
            };
            let routed = if let Some(&(other, _)) = inputs.next() {
                std::array::from_fn(|i| {
                    fold_orbit(
                        finished[split][i],
                        finished[other][i],
                        previous[i],
                        current[i],
                    )
                })
            } else {
                finished[split]
            };
            for (dst, x) in self.channels[2 * group + ch][start..]
                .iter_mut()
                .zip(routed)
            {
                *dst += x;
            }
        }
    }
}

/// Two splitter outputs routed to the same channel. Preserve the orbit's own signal unless
/// cancellation between its two sides exposes rounding that the exported stems cannot explain.
fn fold_orbit(left: f32, right: f32, before: f32, after: f32) -> f32 {
    let folded = left + right;
    if left.is_sign_positive() == right.is_sign_positive() {
        return folded;
    }
    let previous = f64::from(before);
    let current = f64::from(after);
    let contribution = f64::from(folded);
    let delta = current - previous;
    // A float mix of the existing signal and folded stem can round at this scale. Using the
    // separately routed sides as the scale would hide cancellation of a much quieter orbit.
    let tolerance = f64::from(f32::EPSILON) * (previous.abs() + current.abs() + contribution.abs());
    if (delta - contribution).abs() > tolerance {
        delta as f32
    } else {
        folded
    }
}

/// Add this quantum through the orbit's splitter/merger connections. Zipping a 128-frame
/// source with the remaining destination also truncates the last quantum.
fn add_direct(
    channels: &mut [Vec<f32>],
    group: usize,
    start: usize,
    orbit: &Orbit,
    source: &Stereo,
    residual: &mut [[Residual; QUANTUM]; 2],
) {
    for &(split, merge) in orbit.routes() {
        let dst = &mut channels[2 * group + merge][start..];
        for ((d, x), r) in dst.iter_mut().zip(&source[split]).zip(&mut residual[merge]) {
            let old = *d;
            *d += x;
            r.add(-*d);
            r.add(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Residual;

    #[test]
    fn residual_recovers_subnormals_after_cancellation_at_every_bin_boundary() {
        let tiny = f32::from_bits(1);
        for sign in [1.0, -1.0] {
            let mut sum = Residual::default();
            sum.add(sign * tiny);
            // These normals have significand shifts 64, 128 and 192 in subnormal units.
            // Their half values fall into the preceding bin, forcing cross-bin carries.
            let scales = [2.0f32.powi(-62), 4.0, 2.0f32.powi(66), f32::MAX];
            for &x in &scales {
                sum.add(sign * x);
            }
            for &x in &scales {
                let half = -sign * (x * 0.5);
                sum.add(half);
                sum.add(half);
            }
            assert_eq!(sum.sample(), sign * tiny);
        }
    }
}

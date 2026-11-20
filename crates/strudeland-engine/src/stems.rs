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
        let mut value_stem = vec![None; job.values.len()];
        for (index, id, orbit) in scheduled_groups(job, layout.grouping) {
            let required = |id: &StemId| {
                layout
                    .ids
                    .binary_search(id)
                    .map_err(|_| JobError::new(format!("missing stem in render layout: {id:?}")))
            };
            value_stem[index] = Some(required(&id)?);
            if groups_direct {
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
            voices: vec![Vec::new(); job.orbits.len()],
            active: Vec::new(),
            previous_mix: [[0.0; QUANTUM]; 2],
        })
    }

    pub(crate) fn groups_direct(&self) -> bool {
        !self.direct.is_empty()
    }

    pub(crate) fn begin(&mut self, slots: &[Slot]) {
        if !self.groups_direct() {
            return;
        }
        self.previous_mix = [[0.0; QUANTUM]; 2];
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
        let (finished, _) = orbit.output();
        let Some(gains) = gains else {
            add_routed(&mut self.channels, orbit_stem, start, orbit, finished, None);
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
        // direct group actually adds to its exported stem. Measuring both differences in f64
        // retains mixer rounding in the effects stem, even when the same label later cancels
        // across orbits and exposes a much quieter part.
        let mut residual = [[0.0; QUANTUM]; 2];
        for ((delta, old), new) in residual.iter_mut().zip(&mut self.previous_mix).zip(mix) {
            for ((d, a), b) in delta.iter_mut().zip(&mut *old).zip(new) {
                *d = f64::from(*b) - f64::from(*a);
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
            add_routed(
                &mut self.channels,
                group,
                start,
                orbit,
                &post,
                Some(&mut residual),
            );
            sum.silent = [true; 2]; // sum_stereo copies the first non-silent connection.
        }
        // Residuals are already in destination channel order.
        for (ch, values) in residual.iter().enumerate() {
            for (d, x) in self.channels[2 * orbit_stem + ch][start..]
                .iter_mut()
                .zip(values)
            {
                *d += *x as f32;
            }
        }
    }
}

/// Add this quantum through the orbit's splitter/merger connections. Zipping a 128-frame
/// source with the remaining destination also truncates the last quantum.
fn add_routed(
    channels: &mut [Vec<f32>],
    group: usize,
    start: usize,
    orbit: &Orbit,
    source: &Stereo,
    mut residual: Option<&mut [[f64; QUANTUM]; 2]>,
) {
    for &(split, merge) in orbit.routes() {
        let dst = &mut channels[2 * group + merge][start..];
        if let Some(remaining) = residual.as_deref_mut() {
            for ((d, x), r) in dst
                .iter_mut()
                .zip(&source[split])
                .zip(&mut remaining[merge])
            {
                let old = *d;
                *d += x;
                *r -= f64::from(*d) - f64::from(old);
            }
        } else {
            for (d, x) in dst.iter_mut().zip(&source[split]) {
                *d += x;
            }
        }
    }
}

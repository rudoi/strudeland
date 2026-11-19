//! Real-time scheduling for the engine thread and the fake output's pull thread.
//!
//! On macOS a thread asks for the Mach time-constraint policy (`THREAD_TIME_CONSTRAINT_POLICY`),
//! the policy CoreAudio's I/O threads run under: every `period` the thread needs about
//! `computation` of CPU, delivered within `constraint` of the period's start. The kernel schedules
//! such a thread ahead of every timeshare thread, so a loaded machine cannot starve it, and keeps
//! it preemptible. A thread that runs past its computation is preempted and, if it keeps doing
//! so, demoted, so callers keep each burst of work inside the computation (the engine thread
//! renders in slices and blocks between them).
//!
//! Best effort: if the policy is refused the thread falls back to the user-interactive QoS class,
//! and the refusal is counted ([`realtime_refusals`]); nothing is logged.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static REFUSALS: AtomicU64 = AtomicU64::new(0);

/// How many threads were refused the time-constraint policy (and run at the QoS class instead).
/// Always 0 off macOS, where no policy is asked for.
pub fn realtime_refusals() -> u64 {
    REFUSALS.load(Ordering::Relaxed)
}

/// The scheduling the calling thread got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The time-constraint policy.
    RealTime,
    /// The policy was refused (or is not available): the user-interactive QoS class, if any.
    Fallback,
}

/// A time-constraint request. `computation <= constraint <= period` (checked by [`promote`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeConstraint {
    pub period: Duration,
    pub computation: Duration,
    pub constraint: Duration,
}

impl TimeConstraint {
    /// `period`, with `computation` the given fraction of it (`num / den`) and the constraint the
    /// whole period.
    pub const fn fraction_of(period: Duration, num: u32, den: u32) -> TimeConstraint {
        let ns = period.as_nanos() as u64;
        TimeConstraint {
            period,
            computation: Duration::from_nanos(ns * num as u64 / den as u64),
            constraint: period,
        }
    }

    fn is_valid(&self) -> bool {
        !self.computation.is_zero()
            && self.computation <= self.constraint
            && self.constraint <= self.period
    }
}

/// Puts the calling thread under the time-constraint policy, falling back to the
/// user-interactive QoS class (counted in [`realtime_refusals`]).
pub fn promote(tc: TimeConstraint) -> Priority {
    if tc.is_valid() && imp::time_constraint(tc) {
        return Priority::RealTime;
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    imp::qos_user_interactive();
    Priority::Fallback
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::TimeConstraint;
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::mach_init::mach_thread_self;
    use mach2::mach_port::mach_port_deallocate;
    use mach2::mach_time::{mach_timebase_info, mach_timebase_info_data_t};
    use mach2::thread_policy::{
        THREAD_TIME_CONSTRAINT_POLICY, THREAD_TIME_CONSTRAINT_POLICY_COUNT, thread_policy_set,
        thread_policy_t, thread_time_constraint_policy_data_t,
    };
    use mach2::traps::mach_task_self;
    use std::time::Duration;

    /// Nanoseconds to Mach absolute time units, saturating at `u32::MAX`; `None` without a
    /// timebase.
    fn to_abs(d: Duration, tb: mach_timebase_info_data_t) -> Option<u32> {
        if tb.numer == 0 || tb.denom == 0 {
            return None;
        }
        let abs = d.as_nanos() * u128::from(tb.denom) / u128::from(tb.numer);
        Some(abs.min(u128::from(u32::MAX)) as u32)
    }

    pub(super) fn time_constraint(tc: TimeConstraint) -> bool {
        let mut tb = mach_timebase_info_data_t::default();
        // SAFETY: writes the timebase into a valid struct.
        if unsafe { mach_timebase_info(&mut tb) } != KERN_SUCCESS {
            return false;
        }
        let (Some(period), Some(computation), Some(constraint)) = (
            to_abs(tc.period, tb),
            to_abs(tc.computation, tb),
            to_abs(tc.constraint, tb),
        ) else {
            return false;
        };
        let mut policy = thread_time_constraint_policy_data_t {
            period,
            computation,
            constraint,
            preemptible: 1,
        };
        // SAFETY: `mach_thread_self` returns a send right to the calling thread, released below;
        // the policy struct and its count match the flavor.
        unsafe {
            let thread = mach_thread_self();
            let status = thread_policy_set(
                thread,
                THREAD_TIME_CONSTRAINT_POLICY,
                (&mut policy as *mut thread_time_constraint_policy_data_t).cast::<i32>()
                    as thread_policy_t,
                THREAD_TIME_CONSTRAINT_POLICY_COUNT,
            );
            let _ = mach_port_deallocate(mach_task_self(), thread);
            status == KERN_SUCCESS
        }
    }

    pub(super) fn qos_user_interactive() {
        // <pthread/qos.h>: QOS_CLASS_USER_INTERACTIVE.
        const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: plain libSystem call on the current thread with a valid class and priority 0.
        let _ = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
    }

    #[cfg(test)]
    pub(super) fn current_policy() -> Option<thread_time_constraint_policy_data_t> {
        use mach2::boolean::boolean_t;
        use mach2::thread_policy::thread_policy_get;
        let mut policy = thread_time_constraint_policy_data_t {
            period: 0,
            computation: 0,
            constraint: 0,
            preemptible: 0,
        };
        let mut count = THREAD_TIME_CONSTRAINT_POLICY_COUNT;
        let mut get_default: boolean_t = 0;
        // SAFETY: as in `time_constraint`; the out-parameters are valid.
        unsafe {
            let thread = mach_thread_self();
            let status = thread_policy_get(
                thread,
                THREAD_TIME_CONSTRAINT_POLICY,
                (&mut policy as *mut thread_time_constraint_policy_data_t).cast::<i32>()
                    as thread_policy_t,
                &mut count,
                &mut get_default,
            );
            let _ = mach_port_deallocate(mach_task_self(), thread);
            (status == KERN_SUCCESS && get_default == 0).then_some(policy)
        }
    }

    #[cfg(test)]
    pub(super) fn abs_of(d: Duration) -> u32 {
        let mut tb = mach_timebase_info_data_t::default();
        // SAFETY: as above.
        unsafe { mach_timebase_info(&mut tb) };
        to_abs(d, tb).expect("timebase")
    }
}

#[cfg(not(target_vendor = "apple"))]
mod imp {
    use super::TimeConstraint;

    pub(super) fn time_constraint(_tc: TimeConstraint) -> bool {
        false
    }

    pub(super) fn qos_user_interactive() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_of_splits_the_period() {
        let tc = TimeConstraint::fraction_of(Duration::from_millis(10), 1, 2);
        assert_eq!(tc.computation, Duration::from_millis(5));
        assert_eq!(tc.constraint, Duration::from_millis(10));
        assert!(tc.is_valid());
    }

    #[test]
    fn an_invalid_request_falls_back_and_is_counted() {
        let before = realtime_refusals();
        let got = std::thread::spawn(|| {
            promote(TimeConstraint {
                period: Duration::from_millis(1),
                computation: Duration::from_millis(2),
                constraint: Duration::from_millis(2),
            })
        })
        .join()
        .unwrap();
        assert_eq!(got, Priority::Fallback);
        assert!(realtime_refusals() > before);
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn the_policy_is_set_on_the_calling_thread() {
        let tc = TimeConstraint::fraction_of(Duration::from_micros(11_610), 1, 2);
        let (got, policy) = std::thread::spawn(move || (promote(tc), imp::current_policy()))
            .join()
            .unwrap();
        assert_eq!(got, Priority::RealTime);
        let policy = policy.expect("a time-constraint policy");
        assert_eq!(policy.period, imp::abs_of(tc.period));
        assert_eq!(policy.computation, imp::abs_of(tc.computation));
        assert_eq!(policy.constraint, imp::abs_of(tc.constraint));
        assert_eq!(policy.preemptible, 1);
    }
}
